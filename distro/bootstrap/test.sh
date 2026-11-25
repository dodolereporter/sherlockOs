#!/usr/bin/env bash
# Quick test of the bootstrap scripts, without network and without compiling anything.
# Runs in the build container, in a few seconds. Called by `just test-toolchain`, and by the CI.
# Prints one "ok" or "FAILED" line per check.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/lib.sh"
tmp="$(mktemp -d)"
trap 'chmod -R u+w "$tmp"; rm -rf "$tmp"' EXIT
failures=0

check() {
    local label="$1"
    shift
    if "$@" >"$tmp/last.log" 2>&1; then
        printf 'ok      %s\n' "$label"
    else
        printf 'FAILED  %s\n' "$label"
        sed 's/^/        /' "$tmp/last.log"
        failures=1
    fi
}
fails() { ! "$@"; }

# 1. The manifest: four fields, SHA-256 in hexadecimal, https URLs, no duplicate.
check "manifest well formed" check_manifest
bad_manifest() {
    printf '%s\n' "$1" >"$tmp/bad.txt"
    TOOLCHAIN_MANIFEST="$tmp/bad.txt" bash -c "source '$here/lib.sh'; check_manifest"
}
sum="$(printf demo | sha256sum | cut -d' ' -f1)"
check "manifest with an http URL refused" fails bad_manifest "demo 1 $sum http://example.org/demo.txt"
check "manifest with a short checksum refused" fails bad_manifest "demo 1 ${sum:0:40} https://example.org/demo.txt"
check "manifest with an uppercase checksum refused" fails bad_manifest "demo 1 ${sum^^} https://example.org/demo.txt"
check "manifest with a duplicate name refused" fails bad_manifest "demo 1 $sum https://example.org/a.txt
demo 2 $sum https://example.org/b.txt"

# 2. Every source a step uses is declared on its "# Sources:" line and is in the manifest.
steps=("$here"/steps/[0-9][0-9]-*.sh)
step_names() {
    local file="$1" name declared
    declared=" $(step_sources "$file") "
    for name in $(step_sources "$file"); do entry "$name" >/dev/null; done
    for name in $(grep -oE '\b(enter|unpack|source_path|version) [a-z0-9-]+' "$file" | cut -d' ' -f2); do
        [[ "$declared" == *" $name "* ]] || { echo "$name used but not declared"; return 1; }
    done
}
for file in "${steps[@]}"; do
    check "sources of $(basename "$file") declared" step_names "$file"
done

# 3. Syntax of every script. shellcheck goes further when the container has it.
for file in "$here"/*.sh "${steps[@]}"; do
    check "syntax of ${file#"$here"/}" bash -n "$file"
done
if command -v shellcheck >/dev/null; then
    check "shellcheck" shellcheck -s bash -S warning "$here"/*.sh "${steps[@]}"
else
    printf 'skipped shellcheck: not in the container (the CI runs it on the runner)\n'
fi

# 4. fetch.sh --check: right file accepted, wrong file refused and deleted, missing file
# reported without network.
cache="$tmp/cache"
mkdir -p "$cache"
printf 'demo 1 %s https://example.invalid/demo.txt\n' "$sum" >"$tmp/demo.txt"
fetch_check() { SOURCE_CACHE="$cache" TOOLCHAIN_MANIFEST="$tmp/demo.txt" "$here/fetch.sh" --check; }
printf demo >"$cache/demo.txt"
check "fetch accepts the right checksum" fetch_check
printf other >"$cache/demo.txt"
check "fetch refuses a wrong checksum" fails fetch_check
check "fetch deletes the wrong archive" test ! -e "$cache/demo.txt"
check "fetch --check reports a missing archive" fails fetch_check

# 5. Resuming, with the driver in simulation: nothing is built, stamps and order are real.
count=${#steps[@]}
step_at() { basename "${steps[$1]}" .sh; }
out="$tmp/output"
simulate() { TOOLCHAIN_OUTPUT="$out" TOOLCHAIN_SIMULATE=1 "$here/build.sh" "$@" >"$tmp/run.log" 2>&1; }
expect() {
    local built skipped would
    built="$(grep -cE '^toolchain: [^ ]+ built$' "$tmp/run.log" || true)"
    skipped="$(grep -c ' skipped$' "$tmp/run.log" || true)"
    would="$(grep -c ' would be built$' "$tmp/run.log" || true)"
    [[ "$built $skipped $would" == "$1 $2 $3" ]] || { echo "built $built, skipped $skipped, would be built $would; expected $1, $2, $3"; cat "$tmp/run.log"; return 1; }
}
simulate
check "first run builds every step" expect "$count" 0 0
simulate
check "second run builds nothing" expect 0 "$count" 0
echo changed >"$out/stamps/$(step_at 2)"
simulate
check "changed step rebuilt, with every step after it" expect $((count - 2)) 2 0
rm "$out/stamps/$(step_at 4)"
simulate_failing() { TOOLCHAIN_SIMULATE_FAIL="$(step_at 5)" simulate; }
check "failing step stops the run" fails simulate_failing
check "steps before the failure kept" test -f "$out/stamps/$(step_at 4)" -a ! -f "$out/stamps/$(step_at 5)"
simulate
check "next run resumes at the failed step" expect $((count - 5)) 5 0
simulate clean
check "clean deletes the output" test ! -e "$out"
simulate "$(step_at 2 | cut -d- -f1)"
check "a target step stops the run after it" expect 3 0 0
simulate --dry-run
check "dry run builds nothing" expect 0 3 $((count - 3))
check "dry run leaves the stamps" test ! -e "$out/stamps/$(step_at 3)"
check "an unknown step is refused" fails simulate no-such-step

[[ "$failures" == 0 ]] || { printf 'toolchain test: FAILED\n'; exit 1; }
printf 'toolchain test: every check passed\n'
