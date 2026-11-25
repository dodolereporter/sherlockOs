#!/usr/bin/env bash
# Tests of the build container. Called by `just test-builder` and `just test-builder-repro`.
#
#   test.sh quick   tools, user, /bin/sh, cache refused inside the repository, exit codes
#   test.sh repro   two builds without cache install the same package versions
#
# Environment variable: JUST, the just binary used to go through the recipes (just by default).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
builder="$here/builder.sh"
just_bin="${JUST:-just}"

failures=0
pass() { printf 'ok      %s\n' "$1"; }
fail() {
    printf 'FAILED  %s\n' "$1"
    if [[ -n "${2:-}" ]]; then printf '        %s\n' "$2"; fi
    failures=$((failures + 1))
}

# Runs a command and stores its output and exit code in $output and $code, without stopping
# the script.
capture() {
    code=0
    output="$("$@" 2>&1)" || code=$?
}

# Checks that a SOURCE_CACHE inside the repository is refused and that nothing is created.
# The third argument is the folder that must not appear, empty if it already exists.
check_cache_refused() {
    local label="$1" value="$2" path="$3"
    capture env SOURCE_CACHE="$value" "$builder" run true
    if [[ "$code" -eq 0 ]]; then
        fail "$label" "accepted, code 0"
    elif [[ "$output" != *"must be outside the repository"* ]]; then
        fail "$label" "refused for another reason: $output"
    elif [[ -n "$path" && -e "$path" ]]; then
        fail "$label" "refused, but $path was created"
    else
        pass "$label"
    fi
    # If the refusal went wrong, remove the empty folder created by mistake in the repository.
    if [[ -n "$path" && -d "$path" ]]; then rmdir "$path" 2>/dev/null || true; fi
}

quick() {
    # The current folder matters for the relative path: start from the root, like just.
    cd "$repo"

    # The first call builds the image if it is missing, before any check.
    capture "$builder" run true
    if [[ "$code" -ne 0 ]]; then
        printf '%s\n' "$output"
        fail "the container starts" "code $code"
        return
    fi
    pass "the container starts"

    capture "$builder" run gcc --version
    if [[ "$code" -eq 0 && "$output" == gcc* ]]; then
        pass "gcc answers: ${output%%$'\n'*}"
    else
        fail "gcc answers" "code $code: ${output%%$'\n'*}"
    fi

    capture "$builder" run qemu-system-x86_64 --version
    if [[ "$code" -eq 0 && "$output" == "QEMU emulator version"* ]]; then
        pass "qemu-system-x86_64 answers: ${output%%$'\n'*}"
    else
        fail "qemu-system-x86_64 answers" "code $code: ${output%%$'\n'*}"
    fi

    capture "$builder" run id -u
    if [[ "$code" -eq 0 && "$output" =~ ^[0-9]+$ && "$output" != 0 ]]; then
        pass "the user is not root: uid $output"
    else
        fail "the user is not root" "code $code: $output"
    fi

    capture "$builder" run readlink -f /bin/sh
    if [[ "$code" -eq 0 && "$output" == */bash ]]; then
        pass "/bin/sh points to bash: $output"
    else
        fail "/bin/sh points to bash" "code $code: $output"
    fi

    # Unique names, so as to never hit an existing folder of the repository.
    local name=".test-cache-$$"
    check_cache_refused "cache inside the repository refused, absolute path" \
        "$repo/distro/builder/$name-absolute" "$repo/distro/builder/$name-absolute"
    check_cache_refused "cache inside the repository refused, relative path" \
        "distro/builder/$name-relative" "$repo/distro/builder/$name-relative"
    check_cache_refused "cache inside the repository refused, repository root" \
        "." ""
    check_cache_refused "cache inside the repository refused, path with .." \
        "/tmp/../$repo/$name-detour" "$repo/$name-detour"

    capture "$builder" run false
    if [[ "$code" -eq 1 ]]; then pass "false returns 1"; else fail "false returns 1" "code $code"; fi

    capture "$builder" run bash -c 'exit 7'
    if [[ "$code" -eq 7 ]]; then pass "exit 7 returns 7"; else fail "exit 7 returns 7" "code $code"; fi

    # The same codes through the recipe, the one people actually type.
    if command -v "$just_bin" >/dev/null; then
        capture "$just_bin" --justfile "$repo/justfile" --working-directory "$repo" in-builder false
        if [[ "$code" -eq 1 ]]; then
            pass "just in-builder false returns 1"
        else
            fail "just in-builder false returns 1" "code $code"
        fi
        capture "$just_bin" --justfile "$repo/justfile" --working-directory "$repo" in-builder bash -c 'exit 7'
        if [[ "$code" -eq 7 ]]; then
            pass "just in-builder bash -c 'exit 7' returns 7"
        else
            fail "just in-builder bash -c 'exit 7' returns 7" "code $code"
        fi
    else
        fail "just is available to test the recipes" "$just_bin not found"
    fi

    # Arguments arrive intact, spaces and apostrophes included.
    capture "$builder" run printf '%s|' 'a b' "c'd" ''
    if [[ "$code" -eq 0 && "$output" == "a b|c'd||" ]]; then
        pass "arguments pass intact"
    else
        fail "arguments pass intact" "code $code: $output"
    fi
}

# Builds the image with the legacy Docker builder, without cache. Unlike BuildKit, it keeps no
# build cache: removing the image frees everything it used.
# The installed versions do not depend on the builder, only on the Dockerfile.
build_once() {
    DOCKER_BUILDKIT=0 docker build --quiet --no-cache --force-rm --tag "$1" "$here" >/dev/null
}

# Global variables: the cleanup runs when the script exits, after repro has finished.
tag="sherlockos-builder:repro-$$"
folder=""
base=""
base_present=1

clean_repro() {
    docker image rm --force "$tag" >/dev/null 2>&1 || true
    # The base image is removed only if this test downloaded it.
    if [[ -n "$base" && "$base_present" -eq 0 ]]; then
        docker image rm "$base" >/dev/null 2>&1 || true
    fi
    if [[ -n "$folder" ]]; then rm -rf "$folder"; fi
}

repro() {
    base="$(sed -n 's/^FROM[[:space:]]\{1,\}\([^[:space:]]*\).*/\1/p' "$here/Dockerfile" | head -n 1)"
    docker image inspect "$base" >/dev/null 2>&1 || base_present=0
    folder="$(mktemp -d)"
    # The test image, the downloaded base image and the lists go away whatever happens.
    trap clean_repro EXIT

    local n
    for n in 1 2; do
        printf 'build %s without cache…\n' "$n"
        build_once "$tag"
        docker run --rm --name "sherlockos-builder-repro-$$" "$tag" \
            dpkg-query -W -f '${Package}=${Version}\n' >"$folder/packages-$n.txt"
        docker image rm --force "$tag" >/dev/null
    done

    local total
    total="$(wc -l <"$folder/packages-1.txt")"
    if [[ "$total" -gt 0 ]] && diff -u "$folder/packages-1.txt" "$folder/packages-2.txt"; then
        pass "two builds without cache: the same $total packages at the same versions"
    else
        fail "two builds without cache install the same versions" "differences above"
    fi
}

command -v docker >/dev/null || { printf 'docker not found\n' >&2; exit 1; }

case "${1:-}" in
    quick) quick ;;
    repro) repro ;;
    *) printf 'usage: test.sh quick | repro\n' >&2; exit 2 ;;
esac

if [[ "$failures" -gt 0 ]]; then
    printf '%s check(s) failed\n' "$failures"
    exit 1
fi
printf 'all good\n'
