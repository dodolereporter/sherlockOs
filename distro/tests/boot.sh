#!/usr/bin/env bash
# Banc de boot : démarre une image disque UEFI dans QEMU, sans écran, et dit si elle a booté.
# Tourne dans le conteneur de build, où sont QEMU et OVMF. Mode d'emploi : docs/tests-boot.md.
#
#   boot.sh <image>
#
# Réussit (code 0) si une ligne BOOT-OK sort sur le port série avant le délai. Échoue (code 1)
# sinon, et affiche les 50 dernières lignes du port série. Code 2 : mauvais usage ou outil absent.
# Dans les deux cas, une capture de la sortie graphique est prise par QMP tant que QEMU tourne.
#
# Variables reconnues :
#   BOOT_DELAI    secondes accordées au boot, 90 par défaut
#   BOOT_SORTIE   dossier du journal série et de la capture,
#                 par défaut distro/tests/sortie/boot-<nom de l'image>
set -euo pipefail

ici="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
marqueur="BOOT-OK"
delai="${BOOT_DELAI:-90}"
ovmf_code=/usr/share/OVMF/OVMF_CODE_4M.fd
ovmf_vars=/usr/share/OVMF/OVMF_VARS_4M.fd

usage() {
    printf 'boot : %s\n' "$*" >&2
    exit 2
}

[[ $# -eq 1 ]] || usage "usage : boot.sh <image>"
image="$1"
[[ -f "$image" ]] || usage "image introuvable : $image"
[[ "$delai" =~ ^[1-9][0-9]*$ ]] || usage "BOOT_DELAI doit être un nombre de secondes : $delai"
for outil in qemu-system-x86_64 qemu-img python3; do
    command -v "$outil" >/dev/null || usage "$outil est introuvable, lance le banc par just test-boot"
done
[[ -r "$ovmf_code" && -r "$ovmf_vars" ]] || usage "micrologiciel OVMF introuvable dans /usr/share/OVMF"

nom="$(basename "$image")"
sortie="${BOOT_SORTIE:-$ici/sortie/boot-${nom%.*}}"
mkdir -p "$sortie"
journal="$sortie/serie.log"
capture="$sortie/capture.png"
rm -f "$journal" "$capture"
: >"$journal"

# Le format est lu plutôt que deviné par QEMU, qui refuse d'écrire sur une image raw devinée.
format="$(qemu-img info --output=json "$image" | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["format"])')"

temp="$(mktemp -d)"
qemu_pid=""
nettoyer() {
    if [[ -n "$qemu_pid" ]] && kill -0 "$qemu_pid" 2>/dev/null; then
        python3 -I "$ici/qmp.py" "$temp/qmp.sock" quit >/dev/null 2>&1 || kill "$qemu_pid" 2>/dev/null || true
        wait "$qemu_pid" 2>/dev/null || true
    fi
    rm -rf "$temp"
}
trap nettoyer EXIT

# Les variables UEFI doivent être modifiables : on travaille sur une copie jetable.
cp "$ovmf_vars" "$temp/vars.fd"

if [[ -r /dev/kvm && -w /dev/kvm ]]; then
    acceleration=(-accel kvm -cpu host)
    printf 'boot : KVM disponible\n'
else
    acceleration=(-accel tcg -cpu max)
    printf 'boot : pas de KVM, QEMU émule le processeur (plus lent)\n'
fi

# -snapshot : les écritures du système démarré ne touchent pas l'image testée.
# -no-reboot : un redémarrage ou un plantage arrête QEMU au lieu de boucler jusqu'au délai.
qemu-system-x86_64 \
    -nodefaults -machine q35 "${acceleration[@]}" -smp 2 -m 1024 \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$ovmf_code" \
    -drive "if=pflash,format=raw,unit=1,file=$temp/vars.fd" \
    -drive "if=virtio,format=$format,file=$image" -snapshot \
    -vga std -display none \
    -chardev "file,id=serie,path=$journal" -serial chardev:serie \
    -qmp "unix:$temp/qmp.sock,server=on,wait=off" \
    -no-reboot &
qemu_pid=$!
printf 'boot : %s démarre, %s s accordées pour la ligne %s\n' "$nom" "$delai" "$marqueur"

# Le port série écrit des fins de ligne \r\n : la ligne vaut BOOT-OK, suivi ou non de \r.
resultat=""
debut=$SECONDS
while [[ -z "$resultat" ]]; do
    if grep -qE "^${marqueur}"$'\r?$' "$journal"; then
        resultat=reussi
    elif ! kill -0 "$qemu_pid" 2>/dev/null; then
        code_qemu=0
        wait "$qemu_pid" || code_qemu=$?
        qemu_pid=""
        # QEMU a pu écrire la ligne juste avant de s'arrêter : on relit une dernière fois.
        if grep -qE "^${marqueur}"$'\r?$' "$journal"; then
            resultat=reussi
        else
            resultat="QEMU s'est arrêté (code $code_qemu) sans écrire $marqueur"
        fi
    elif ((SECONDS - debut >= delai)); then
        resultat="pas de ligne $marqueur après $delai s"
    else
        sleep 0.5
    fi
done
duree=$((SECONDS - debut))

if [[ -n "$qemu_pid" ]]; then
    if python3 -I "$ici/qmp.py" "$temp/qmp.sock" screendump "$capture"; then
        printf 'boot : capture de l'\''écran dans %s\n' "$capture"
    else
        printf 'boot : la capture de l'\''écran a échoué\n' >&2
    fi
fi

if [[ "$resultat" == reussi ]]; then
    printf 'boot : réussi, %s vu après %s s. Journal série : %s\n' "$marqueur" "$duree" "$journal"
    exit 0
fi

printf 'boot : ÉCHEC, %s\n' "$resultat"
printf 'boot : 50 dernières lignes du port série (%s) :\n' "$journal"
# Sans le \r du port série ni les séquences d'échappement du micrologiciel, qui brouillent le terminal.
tail -n 50 "$journal" | tr -d '\r' | sed -E $'s/\x1b\[[0-9;=?]*[A-Za-z]//g'
exit 1
