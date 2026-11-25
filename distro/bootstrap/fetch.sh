#!/usr/bin/env bash
# Downloads the sources of the manifest into the source cache and checks their SHA-256.
# Runs in the build container. Called by build.sh. How it works: docs/system-build.md.
#
#   fetch.sh           downloads what is missing, then checks every archive
#   fetch.sh --check   checks only, without network: a missing archive is an error
#
# An archive whose checksum is wrong is deleted, so that the next run downloads it again.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

check_only=0
case "${1:-}" in
    --check) check_only=1 ;;
    "") ;;
    *) fail "usage: fetch.sh [--check]" ;;
esac

check_manifest || fail "malformed manifest, see above"
mkdir -p "$SOURCE_CACHE"

errors=0
while read -r name _ _ url; do
    path="$(source_path "$name")"
    if [[ ! -f "$path" ]]; then
        if [[ "$check_only" == 1 ]]; then
            printf 'fetch: %s missing: %s\n' "$name" "$path" >&2
            errors=1
            continue
        fi
        # ftp.gnu.org sometimes refuses connections: ftpmirror.gnu.org, the redirector of the GNU
        # project, then sends to a mirror. The checksum below protects from a wrong mirror.
        urls=("$url")
        [[ "$url" != https://ftp.gnu.org/gnu/* ]] || urls+=("https://ftpmirror.gnu.org/${url#https://ftp.gnu.org/gnu/}")
        for try in "${urls[@]}"; do
            printf 'fetch: downloading %s\n' "$try"
            # https only, redirects included. A download slower than 1 kB/s for a minute is retried.
            if curl -fsSL --proto '=https' --proto-redir '=https' --retry 3 --connect-timeout 30 \
                --speed-limit 1024 --speed-time 60 -o "$path.partial" "$try"; then
                mv "$path.partial" "$path"
                break
            fi
            rm -f "$path.partial"
        done
        [[ -f "$path" ]] || { printf 'fetch: %s could not be downloaded\n' "$name" >&2; errors=1; continue; }
    fi
    if ! verify "$name"; then
        rm -f "$path"
        printf 'fetch: wrong SHA-256 for %s, %s deleted\n' "$name" "$path" >&2
        errors=1
    fi
done < <(manifest_entries)

[[ "$errors" == 0 ]] || fail "some sources are missing or wrong, see above"
printf 'fetch: %s sources checked in %s\n' "$(manifest_entries | wc -l)" "$SOURCE_CACHE"
