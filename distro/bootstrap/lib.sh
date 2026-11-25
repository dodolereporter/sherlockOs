# Functions shared by the bootstrap scripts. Sourced, never run.
# How the build works: docs/system-build.md.

bootstrap_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
manifest="${TOOLCHAIN_MANIFEST:-$bootstrap_dir/sources.txt}"

fail() {
    printf 'bootstrap: %s\n' "$*" >&2
    exit 1
}

# Entries of the manifest, one "name version sha256 url" line each, without comments.
manifest_entries() {
    grep -v -E '^[[:space:]]*(#|$)' "$manifest"
}

# entry NAME: the manifest line of NAME.
entry() {
    local line
    line="$(manifest_entries | awk -v name="$1" '$1 == name')"
    [[ -n "$line" ]] || fail "$1 is not in the manifest $manifest"
    printf '%s\n' "$line"
}

version() { entry "$1" | awk '{print $2}'; }
checksum() { entry "$1" | awk '{print $3}'; }

# source_path NAME: path of the archive of NAME in the source cache.
source_path() {
    [[ -n "${SOURCE_CACHE:-}" ]] || fail "SOURCE_CACHE is empty, run it through just"
    printf '%s/%s\n' "$SOURCE_CACHE" "$(entry "$1" | awk '{print $4}' | xargs basename)"
}

# verify NAME: true when the archive of NAME is in the cache with the checksum of the manifest.
verify() {
    local path
    path="$(source_path "$1")"
    [[ -f "$path" ]] && printf '%s  %s\n' "$(checksum "$1")" "$path" | sha256sum --check --quiet --strict >/dev/null 2>&1
}

# check_manifest: fails on a malformed manifest. Four fields, unique names and file names,
# a SHA-256 in lowercase hexadecimal, an https URL.
check_manifest() {
    local name sum url extra errors=0
    declare -A names=() files=()
    while read -r name _ sum url extra; do
        if [[ -z "$url" || -n "$extra" ]]; then
            printf 'manifest: %s: expected "name version sha256 url"\n' "$name" >&2; errors=1; continue
        fi
        [[ "$sum" =~ ^[0-9a-f]{64}$ ]] || { printf 'manifest: %s: bad SHA-256 %s\n' "$name" "$sum" >&2; errors=1; }
        [[ "$url" =~ ^https://[^/]+/.+[^/]$ ]] || { printf 'manifest: %s: not an https file URL: %s\n' "$name" "$url" >&2; errors=1; }
        [[ -z "${names[$name]:-}" ]] || { printf 'manifest: %s: duplicate name\n' "$name" >&2; errors=1; }
        [[ -z "${files[${url##*/}]:-}" ]] || { printf 'manifest: %s: duplicate file %s\n' "$name" "${url##*/}" >&2; errors=1; }
        names[$name]=1
        files[${url##*/}]=1
    done < <(manifest_entries)
    [[ "${#names[@]}" -gt 0 ]] || { printf 'manifest: no entry in %s\n' "$manifest" >&2; errors=1; }
    return "$errors"
}

# step_sources FILE: the names listed on the "# Sources:" line of a step script.
step_sources() {
    sed -n 's/^# Sources: *//p' "$1"
}

# Helpers for the step scripts, which run in their own empty work folder.

# unpack NAME [DIR]: extracts the archive of NAME here, renamed DIR when given.
unpack() {
    local archive top
    archive="$(source_path "$1")"
    top="$(basename "$archive")"
    top="${top%.tar.*}"
    tar -xf "$archive"
    if [[ -n "${2:-}" ]]; then
        mv "$top" "$2"
    fi
}

# enter NAME: extracts the archive of NAME and goes into its folder.
enter() {
    local top
    top="$(basename "$(source_path "$1")")"
    unpack "$1"
    cd "${top%.tar.*}"
}
