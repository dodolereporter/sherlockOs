#!/usr/bin/env bash
# Builds the cross toolchain and the temporary tools (Linux From Scratch, chapters 5 and 6).
# Runs in the build container, never as root. Called by `just toolchain`. How it works:
# docs/system-build.md.
#
#   build.sh [--dry-run] [STEP]   builds every step, or up to STEP (number or name, "glibc")
#   build.sh clean                deletes the output folder; the source cache stays
#
# A step that succeeded leaves a stamp holding its fingerprint: its script, env.sh and the
# checksums of its sources. It is skipped while the fingerprint matches. The first step whose
# fingerprint changed is rebuilt, and every step after it.
#
# Environment variables:
#   TOOLCHAIN_OUTPUT     output folder, distro/bootstrap/output by default (ignored by git)
#   TOOLCHAIN_JOBS       parallel compile jobs, 4 by default
#   TOOLCHAIN_SIMULATE   1: runs the driver without building nor reading sources (tests)
#   TOOLCHAIN_SIMULATE_FAIL  with TOOLCHAIN_SIMULATE, the step that fails (tests)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# The steps run in an empty environment, like the lfs user of the book: nothing of the host
# leaks into the build. Only the variables below cross.
if [[ -z "${BOOTSTRAP_CLEAN_ENV:-}" ]]; then
    exec env -i BOOTSTRAP_CLEAN_ENV=1 HOME="$HOME" TERM="${TERM:-dumb}" \
        SOURCE_CACHE="${SOURCE_CACHE:-}" \
        TOOLCHAIN_OUTPUT="${TOOLCHAIN_OUTPUT:-$here/output}" \
        TOOLCHAIN_JOBS="${TOOLCHAIN_JOBS:-4}" \
        TOOLCHAIN_MANIFEST="${TOOLCHAIN_MANIFEST:-$here/sources.txt}" \
        TOOLCHAIN_SIMULATE="${TOOLCHAIN_SIMULATE:-0}" \
        TOOLCHAIN_SIMULATE_FAIL="${TOOLCHAIN_SIMULATE_FAIL:-}" \
        /usr/bin/bash "$0" "$@"
fi
source "$here/lib.sh"
source "$here/env.sh"

[[ "$(id -u)" != 0 ]] || fail "refusing to run as root: a wrong LFS would overwrite the system"
[[ "$TOOLCHAIN_JOBS" =~ ^[1-9][0-9]*$ ]] || fail "TOOLCHAIN_JOBS must be a positive number"

output="$TOOLCHAIN_OUTPUT"
dry_run=0
target=""
for argument in "$@"; do
    case "$argument" in
        --dry-run) dry_run=1 ;;
        clean)
            # Files of the sysroot may be read-only. Running it again finishes an interrupted run.
            if [[ -d "$output" ]]; then
                chmod -R u+w "$output"
                rm -rf "$output"
            fi
            printf 'toolchain: %s deleted\n' "$output"
            exit 0 ;;
        -*) fail "unknown option $argument" ;;
        *) target="$argument" ;;
    esac
done

steps=("$here"/steps/[0-9][0-9]-*.sh)
step_id() { basename "$1" .sh; }

if [[ -n "$target" ]]; then
    found=""
    for file in "${steps[@]}"; do
        id="$(step_id "$file")"
        if [[ "$id" == "$target" || "${id#*-}" == "$target" || "${id%%-*}" == "$target" ]]; then
            found="$id"
        fi
    done
    [[ -n "$found" ]] || fail "no step named $target"
    target="$found"
fi

# Fingerprint of a step: what it runs and what it reads.
fingerprint() {
    {
        cat "$1" "$here/env.sh"
        for name in $(step_sources "$1"); do
            entry "$name"
        done
    } | sha256sum | cut -d' ' -f1
}

run_step() {
    local file="$1" id="$2" log="$output/logs/$2.log" work="$output/work/$2" name start status seconds
    rm -rf "$work"
    mkdir -p "$work" "$output/logs"
    if [[ "$TOOLCHAIN_SIMULATE" == 1 ]]; then
        [[ "$id" != "$TOOLCHAIN_SIMULATE_FAIL" ]] || { printf 'toolchain: %s failed (simulated)\n' "$id" >&2; exit 1; }
    else
        for name in $(step_sources "$file"); do
            verify "$name" || fail "$id: $name missing from the cache or wrong SHA-256, run fetch.sh"
        done
        start="$(date +%s)"
        # set +e around the subshell, not "if !": a condition would turn errexit off inside it.
        set +e
        (set -e; cd "$work"; set -x; source "$file") >"$log" 2>&1
        status=$?
        set -e
        seconds=$(($(date +%s) - start))
        if [[ "$status" != 0 ]]; then
            tail -n 30 "$log" >&2
            fail "$id failed after $seconds s, log: $log, sources left in $work"
        fi
        printf '%s %s\n' "$id" "$seconds" >>"$output/logs/durations"
        printf 'toolchain: %s took %s s\n' "$id" "$seconds"
    fi
    rm -rf "$work"
}

if [[ "$dry_run" == 0 ]]; then
    [[ "$TOOLCHAIN_SIMULATE" == 1 ]] || "$here/fetch.sh"
    mkdir -p "$output/stamps" "$LFS"
fi

rebuild=0
built=0
for file in "${steps[@]}"; do
    id="$(step_id "$file")"
    stamp="$output/stamps/$id"
    sum="$(fingerprint "$file")"
    if [[ "$rebuild" == 0 && -f "$stamp" && "$(cat "$stamp")" == "$sum" ]]; then
        printf 'toolchain: %s done, skipped\n' "$id"
    elif [[ "$dry_run" == 1 ]]; then
        rebuild=1
        printf 'toolchain: %s would be built\n' "$id"
    else
        # From here on every stamp is stale: an interrupted run starts again at this step.
        rebuild=1
        rm -f "$stamp"
        for later in "${steps[@]}"; do
            if [[ "$(step_id "$later")" > "$id" ]]; then
                rm -f "$output/stamps/$(step_id "$later")"
            fi
        done
        printf 'toolchain: %s building, log in %s\n' "$id" "$output/logs/$id.log"
        run_step "$file" "$id"
        printf '%s\n' "$sum" >"$stamp"
        built=$((built + 1))
        printf 'toolchain: %s built\n' "$id"
    fi
    [[ "$id" != "$target" ]] || break
done

if [[ "$dry_run" == 0 ]]; then
    printf 'toolchain: %s steps built, output %s (%s)\n' "$built" "$output" "$(du -sh "$output" | cut -f1)"
fi
