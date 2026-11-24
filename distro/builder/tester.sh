#!/usr/bin/env bash
# Tests du conteneur de build. Appelé par `just tester-builder` et `just tester-builder-repro`.
#
#   tester.sh rapide   outils, utilisateur, /bin/sh, refus du cache dans le dépôt, codes de sortie
#   tester.sh repro    deux constructions sans cache installent les mêmes versions de paquets
#
# Variable reconnue : JUST, le binaire just à utiliser pour passer par les recettes (just par défaut).
set -euo pipefail

ici="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
depot="$(cd "$ici/../.." && pwd)"
builder="$ici/builder.sh"
just_bin="${JUST:-just}"

echecs=0
reussi() { printf 'ok      %s\n' "$1"; }
echoue() {
    printf 'ÉCHEC   %s\n' "$1"
    if [[ -n "${2:-}" ]]; then printf '        %s\n' "$2"; fi
    echecs=$((echecs + 1))
}

# Lance une commande et range sa sortie et son code dans $sortie et $code, sans arrêter le script.
executer() {
    code=0
    sortie="$("$@" 2>&1)" || code=$?
}

# Vérifie qu'un CACHE_SOURCES placé dans le dépôt est refusé et que rien n'est créé.
# Le troisième argument est le dossier qui ne doit pas apparaître, vide s'il existe déjà.
verifier_refus_cache() {
    local libelle="$1" valeur="$2" chemin="$3"
    executer env CACHE_SOURCES="$valeur" "$builder" lancer true
    if [[ "$code" -eq 0 ]]; then
        echoue "$libelle" "accepté, code 0"
    elif [[ "$sortie" != *"doit être hors du dépôt"* ]]; then
        echoue "$libelle" "refusé pour une autre raison : $sortie"
    elif [[ -n "$chemin" && -e "$chemin" ]]; then
        echoue "$libelle" "refusé, mais $chemin a été créé"
    else
        reussi "$libelle"
    fi
    # Si le refus a raté, on retire le dossier vide créé par erreur dans le dépôt.
    if [[ -n "$chemin" && -d "$chemin" ]]; then rmdir "$chemin" 2>/dev/null || true; fi
}

rapide() {
    # Le dossier courant compte pour le chemin relatif : on se place à la racine, comme just.
    cd "$depot"

    # Le premier appel construit l'image si elle manque, avant toute vérification.
    executer "$builder" lancer true
    if [[ "$code" -ne 0 ]]; then
        printf '%s\n' "$sortie"
        echoue "le conteneur démarre" "code $code"
        return
    fi
    reussi "le conteneur démarre"

    executer "$builder" lancer gcc --version
    if [[ "$code" -eq 0 && "$sortie" == gcc* ]]; then
        reussi "gcc répond : ${sortie%%$'\n'*}"
    else
        echoue "gcc répond" "code $code : ${sortie%%$'\n'*}"
    fi

    executer "$builder" lancer qemu-system-x86_64 --version
    if [[ "$code" -eq 0 && "$sortie" == "QEMU emulator version"* ]]; then
        reussi "qemu-system-x86_64 répond : ${sortie%%$'\n'*}"
    else
        echoue "qemu-system-x86_64 répond" "code $code : ${sortie%%$'\n'*}"
    fi

    executer "$builder" lancer id -u
    if [[ "$code" -eq 0 && "$sortie" =~ ^[0-9]+$ && "$sortie" != 0 ]]; then
        reussi "l'utilisateur n'est pas root : uid $sortie"
    else
        echoue "l'utilisateur n'est pas root" "code $code : $sortie"
    fi

    executer "$builder" lancer readlink -f /bin/sh
    if [[ "$code" -eq 0 && "$sortie" == */bash ]]; then
        reussi "/bin/sh pointe vers bash : $sortie"
    else
        echoue "/bin/sh pointe vers bash" "code $code : $sortie"
    fi

    # Noms uniques, pour ne jamais tomber sur un dossier existant du dépôt.
    local nom=".essai-cache-$$"
    verifier_refus_cache "cache dans le dépôt refusé, chemin absolu" \
        "$depot/distro/builder/$nom-absolu" "$depot/distro/builder/$nom-absolu"
    verifier_refus_cache "cache dans le dépôt refusé, chemin relatif" \
        "distro/builder/$nom-relatif" "$depot/distro/builder/$nom-relatif"
    verifier_refus_cache "cache dans le dépôt refusé, racine du dépôt" \
        "." ""
    verifier_refus_cache "cache dans le dépôt refusé, chemin avec .." \
        "/tmp/../$depot/$nom-detour" "$depot/$nom-detour"

    executer "$builder" lancer false
    if [[ "$code" -eq 1 ]]; then reussi "false rend 1"; else echoue "false rend 1" "code $code"; fi

    executer "$builder" lancer bash -c 'exit 7'
    if [[ "$code" -eq 7 ]]; then reussi "exit 7 rend 7"; else echoue "exit 7 rend 7" "code $code"; fi

    # Les mêmes codes au travers de la recette, celle qu'on tape vraiment.
    if command -v "$just_bin" >/dev/null; then
        executer "$just_bin" --justfile "$depot/justfile" --working-directory "$depot" dans-builder false
        if [[ "$code" -eq 1 ]]; then
            reussi "just dans-builder false rend 1"
        else
            echoue "just dans-builder false rend 1" "code $code"
        fi
        executer "$just_bin" --justfile "$depot/justfile" --working-directory "$depot" dans-builder bash -c 'exit 7'
        if [[ "$code" -eq 7 ]]; then
            reussi "just dans-builder bash -c 'exit 7' rend 7"
        else
            echoue "just dans-builder bash -c 'exit 7' rend 7" "code $code"
        fi
    else
        echoue "just est disponible pour tester les recettes" "$just_bin introuvable"
    fi

    # Les arguments arrivent intacts, espaces et apostrophes compris.
    executer "$builder" lancer printf '%s|' 'a b' "c'd" ''
    if [[ "$code" -eq 0 && "$sortie" == "a b|c'd||" ]]; then
        reussi "les arguments passent intacts"
    else
        echoue "les arguments passent intacts" "code $code : $sortie"
    fi
}

# Construit l'image avec le builder historique de Docker, sans cache. Contrairement à BuildKit,
# il ne garde pas de cache de construction : supprimer l'image libère tout ce qu'elle a occupé.
# Les versions installées ne dépendent pas du builder, seulement du Dockerfile.
construire_essai() {
    DOCKER_BUILDKIT=0 docker build --quiet --no-cache --force-rm --tag "$1" "$ici" >/dev/null
}

# Variables globales : le nettoyage tourne à la sortie du script, après la fin de repro.
etiquette="sherlockos-builder:repro-$$"
dossier=""
base=""
base_presente=1

nettoyer_repro() {
    docker image rm --force "$etiquette" >/dev/null 2>&1 || true
    # L'image de base n'est retirée que si ce test l'a téléchargée.
    if [[ -n "$base" && "$base_presente" -eq 0 ]]; then
        docker image rm "$base" >/dev/null 2>&1 || true
    fi
    if [[ -n "$dossier" ]]; then rm -rf "$dossier"; fi
}

repro() {
    base="$(sed -n 's/^FROM[[:space:]]\{1,\}\([^[:space:]]*\).*/\1/p' "$ici/Dockerfile" | head -n 1)"
    docker image inspect "$base" >/dev/null 2>&1 || base_presente=0
    dossier="$(mktemp -d)"
    # L'image d'essai, l'image de base téléchargée et les listes partent quoi qu'il arrive.
    trap nettoyer_repro EXIT

    local n
    for n in 1 2; do
        printf 'construction %s sans cache…\n' "$n"
        construire_essai "$etiquette"
        docker run --rm --name "sherlockos-builder-repro-$$" "$etiquette" \
            dpkg-query -W -f '${Package}=${Version}\n' >"$dossier/paquets-$n.txt"
        docker image rm --force "$etiquette" >/dev/null
    done

    local total
    total="$(wc -l <"$dossier/paquets-1.txt")"
    if [[ "$total" -gt 0 ]] && diff -u "$dossier/paquets-1.txt" "$dossier/paquets-2.txt"; then
        reussi "deux constructions sans cache : mêmes $total paquets aux mêmes versions"
    else
        echoue "deux constructions sans cache installent les mêmes versions" "différences ci-dessus"
    fi
}

command -v docker >/dev/null || { printf 'docker est introuvable\n' >&2; exit 1; }

case "${1:-}" in
    rapide) rapide ;;
    repro) repro ;;
    *) printf 'usage : tester.sh rapide | repro\n' >&2; exit 2 ;;
esac

if [[ "$echecs" -gt 0 ]]; then
    printf '%s vérification(s) en échec\n' "$echecs"
    exit 1
fi
printf 'tout est bon\n'
