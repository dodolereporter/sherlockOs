#!/usr/bin/env bash
# Construit l'image du conteneur de build et lance des commandes dedans.
# Appelé par `just builder` et `just dans-builder`. Mode d'emploi : docs/build.md.
#
#   builder.sh construire          construit l'image
#   builder.sh lancer [commande]   lance la commande dans le conteneur, bash par défaut
#
# Variables reconnues :
#   CACHE_SOURCES   dossier du cache des sources sur le poste,
#                   par défaut ${XDG_CACHE_HOME:-$HOME/.cache}/sherlockos/sources
#   BUILDER_IMAGE   nom de l'image, par défaut sherlockos-builder:local
set -euo pipefail

ici="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
depot="$(cd "$ici/../.." && pwd)"
image="${BUILDER_IMAGE:-sherlockos-builder:local}"
cache="${CACHE_SOURCES:-${XDG_CACHE_HOME:-$HOME/.cache}/sherlockos/sources}"

# L'utilisateur du conteneur reprend l'uid et le gid du poste, pour que les fichiers écrits
# dans le dépôt lui appartiennent. Un poste qui tourne en root (certaines CI) prend 1000 :
# le conteneur ne tourne jamais en root.
uid="$(id -u)"
gid="$(id -g)"
if [[ "$uid" == 0 ]]; then
    uid=1000
    gid=1000
fi

# Empreinte de ce qui définit l'image. Elle est posée en étiquette sur l'image, et `lancer`
# reconstruit l'image quand elle ne correspond plus (Dockerfile modifié, autre utilisateur).
empreinte="$(cat "$ici/Dockerfile" <(printf 'uid=%s gid=%s\n' "$uid" "$gid") | sha256sum | cut -d' ' -f1)"

erreur() {
    printf 'builder : %s\n' "$*" >&2
    exit 1
}

construire() {
    docker build \
        --build-arg "BUILDER_UID=$uid" \
        --build-arg "BUILDER_GID=$gid" \
        --label "sherlockos.builder.empreinte=$empreinte" \
        --tag "$image" \
        "$ici"
}

lancer() {
    local actuelle
    actuelle="$(docker image inspect --format '{{ index .Config.Labels "sherlockos.builder.empreinte" }}' "$image" 2>/dev/null || true)"
    if [[ "$actuelle" != "$empreinte" ]]; then
        printf 'builder : image absente ou périmée, construction de %s\n' "$image" >&2
        construire >&2
    fi

    # Vérifié avant de créer le dossier, pour ne rien laisser dans le dépôt.
    cache="$(realpath -m "$cache")"
    case "$cache/" in
        "$(realpath "$depot")/"*) erreur "le cache des sources ($cache) doit être hors du dépôt" ;;
    esac
    mkdir -p "$cache"

    local options=(
        --rm --init --interactive
        --name "sherlockos-builder-$$"
        --volume "$depot:/depot"
        --volume "$cache:/cache/sources"
        --workdir /depot
    )
    # Un terminal seulement quand il y en a un, pour que les appels depuis un script marchent.
    if [[ -t 0 && -t 1 ]]; then
        options+=(--tty)
    fi
    # KVM accélère QEMU quand le poste le permet. Sans lui, QEMU émule, plus lentement.
    if [[ -r /dev/kvm && -w /dev/kvm ]]; then
        options+=(--device /dev/kvm --group-add "$(stat -c %g /dev/kvm)")
    fi

    if [[ $# -eq 0 ]]; then
        set -- bash
    fi
    exec docker run "${options[@]}" "$image" "$@"
}

command -v docker >/dev/null || erreur "docker est introuvable, voir docs/build.md"

case "${1:-}" in
    construire) construire ;;
    lancer) shift; lancer "$@" ;;
    *) erreur "usage : builder.sh construire | lancer [commande]" ;;
esac
