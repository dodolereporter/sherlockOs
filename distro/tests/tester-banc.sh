#!/usr/bin/env bash
# Vérifie que le banc de boot échoue proprement. Tourne dans le conteneur de build, après
# `just temoin muet`. Appelé par `just tester-banc`. Mode d'emploi : docs/tests-boot.md.
#
# Le cas qui réussit est `just test-boot` lui-même : il n'est pas refait ici.
set -euo pipefail

ici="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
sortie="$ici/sortie"

echecs=0
reussi() { printf 'ok      %s\n' "$1"; }
echoue() {
    printf 'ÉCHEC   %s\n' "$1"
    if [[ -n "${2:-}" ]]; then printf '        %s\n' "$2"; fi
    echecs=$((echecs + 1))
}
executer() {
    code=0
    sortie_banc="$("$@" 2>&1)" || code=$?
}

# 1. Un initramfs qui n'écrit pas BOOT-OK : le banc attend le délai entier, rend 1 et montre
# la fin du port série, où doit figurer la ligne que /init a écrite à la place.
image="$sortie/temoin-muet.img"
dossier="$sortie/boot-temoin-muet"
if [[ ! -f "$image" ]]; then
    echoue "image muette présente" "$image manque, lance just tester-banc"
else
    debut=$SECONDS
    executer "$ici/boot.sh" "$image"
    duree=$((SECONDS - debut))
    printf '%s\n' "$sortie_banc" | sed 's/^/        | /'
    lignes="$(printf '%s\n' "$sortie_banc" | sed -n '/50 dernières lignes/,$p' | tail -n +2 | wc -l)"
    if [[ "$code" -eq 1 ]]; then reussi "initramfs muet : code 1"; else echoue "initramfs muet : code 1" "code $code"; fi
    if [[ "$sortie_banc" == *"pas de ligne BOOT-OK après ${BOOT_DELAI:-90} s"* && "$duree" -ge "${BOOT_DELAI:-90}" ]]; then
        reussi "initramfs muet : échec au bout du délai, $duree s"
    else
        echoue "initramfs muet : échec au bout du délai" "$duree s, message attendu absent"
    fi
    if [[ "$lignes" -ge 1 && "$lignes" -le 50 && "$sortie_banc" == *"temoin : initramfs muet"* ]]; then
        reussi "initramfs muet : $lignes dernières lignes du port série affichées, ligne de /init comprise"
    else
        echoue "initramfs muet : fin du port série affichée" "$lignes lignes, ligne de /init absente"
    fi
    if [[ "$(head -c 8 "$dossier/capture.png" 2>/dev/null | od -An -tx1 | tr -d ' \n')" == 89504e470d0a1a0a ]]; then
        reussi "initramfs muet : capture PNG prise malgré l'échec"
    else
        echoue "initramfs muet : capture PNG prise malgré l'échec" "$dossier/capture.png absente ou pas en PNG"
    fi
fi

# 2. Une image absente : erreur d'usage, code 2, sans lancer QEMU.
executer "$ici/boot.sh" "$sortie/absente.img"
if [[ "$code" -eq 2 && "$sortie_banc" == *"image introuvable"* ]]; then
    reussi "image absente : code 2"
else
    echoue "image absente : code 2" "code $code : $sortie_banc"
fi

if [[ "$echecs" -gt 0 ]]; then
    printf '%s vérification(s) en échec\n' "$echecs"
    exit 1
fi
printf 'tout est bon\n'
