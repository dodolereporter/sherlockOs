#!/usr/bin/env bash
# Builds the image of the build container and runs commands in it.
# Called by `just builder` and `just in-builder`. How to use it: docs/build.md.
#
#   builder.sh build           builds the image
#   builder.sh run [command]   runs the command in the container, bash by default
#
# Environment variables:
#   SOURCE_CACHE    folder of the source cache on the host,
#                   ${XDG_CACHE_HOME:-$HOME/.cache}/sherlockos/sources by default
#   BUILDER_IMAGE   name of the image, sherlockos-builder:local by default
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
image="${BUILDER_IMAGE:-sherlockos-builder:local}"
cache="${SOURCE_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/sherlockos/sources}"

# The container user takes the uid and gid of the host, so that the files it writes in the
# repository belong to the host user. A host running as root (some CI) gets 1000: the container
# never runs as root.
uid="$(id -u)"
gid="$(id -g)"
if [[ "$uid" == 0 ]]; then
    uid=1000
    gid=1000
fi

# Fingerprint of what defines the image. It is set as a label on the image, and `run` rebuilds
# the image when it no longer matches (Dockerfile changed, other user).
fingerprint="$(cat "$here/Dockerfile" <(printf 'uid=%s gid=%s\n' "$uid" "$gid") | sha256sum | cut -d' ' -f1)"

fail() {
    printf 'builder: %s\n' "$*" >&2
    exit 1
}

build() {
    docker build \
        --build-arg "BUILDER_UID=$uid" \
        --build-arg "BUILDER_GID=$gid" \
        --label "sherlockos.builder.fingerprint=$fingerprint" \
        --tag "$image" \
        "$here"
}

run() {
    local current
    current="$(docker image inspect --format '{{ index .Config.Labels "sherlockos.builder.fingerprint" }}' "$image" 2>/dev/null || true)"
    if [[ "$current" != "$fingerprint" ]]; then
        printf 'builder: image missing or outdated, building %s\n' "$image" >&2
        build >&2
    fi

    # Checked before creating the folder, so that nothing is left in the repository.
    cache="$(realpath -m "$cache")"
    case "$cache/" in
        "$(realpath "$repo")/"*) fail "the source cache ($cache) must be outside the repository" ;;
    esac
    mkdir -p "$cache"

    local options=(
        --rm --init --interactive
        --name "sherlockos-builder-$$"
        --volume "$repo:/repo"
        --volume "$cache:/cache/sources"
        --workdir /repo
    )
    # A terminal only when there is one, so that calls from a script work.
    if [[ -t 0 && -t 1 ]]; then
        options+=(--tty)
    fi
    # KVM speeds QEMU up when the host allows it. Without it, QEMU emulates, more slowly.
    if [[ -r /dev/kvm && -w /dev/kvm ]]; then
        options+=(--device /dev/kvm --group-add "$(stat -c %g /dev/kvm)")
    fi

    if [[ $# -eq 0 ]]; then
        set -- bash
    fi
    exec docker run "${options[@]}" "$image" "$@"
}

command -v docker >/dev/null || fail "docker not found, see docs/build.md"

case "${1:-}" in
    build) build ;;
    run) shift; run "$@" ;;
    *) fail "usage: builder.sh build | run [command]" ;;
esac
