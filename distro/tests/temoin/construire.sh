#!/usr/bin/env bash
# Construit l'image témoin du banc de boot : un disque GPT avec une partition EFI qui contient
# un noyau Linux minimal, démarrable par UEFI, et son initramfs intégré d'un seul binaire.
# Tourne dans le conteneur de build. Appelé par `just temoin`. Mode d'emploi : docs/tests-boot.md.
#
#   construire.sh [ok | muet]
#
#   ok     (par défaut) /init écrit BOOT-OK : sortie/temoin.img
#   muet   /init écrit une autre ligne : sortie/temoin-muet.img, pour tester l'échec du banc
#
# Variable reconnue : TEMOIN_JOBS, nombre de compilations en parallèle (nproc par défaut).
set -euo pipefail

# Noyau stable de kernel.org, épinglé. Pour monter de version, change les deux lignes ensemble :
# l'empreinte vient de sha256sums.asc du dossier de la version, dont on vérifie la signature.
noyau_version=7.3.6
noyau_sha256=8183d475a76a0faffa92d6cd500af421f589dfd0300a0f1060bb875e11d45723
noyau_url="https://cdn.kernel.org/pub/linux/kernel/v${noyau_version%%.*}.x/linux-$noyau_version.tar.xz"

ici="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
sortie="$(cd "$ici/.." && pwd)/sortie"
variante="${1:-ok}"
jobs="${TEMOIN_JOBS:-$(nproc)}"

case "$variante" in
    ok) image="$sortie/temoin.img" ; options_init=() ;;
    muet) image="$sortie/temoin-muet.img" ; options_init=(-DTEMOIN_MUET) ;;
    *) printf 'usage : construire.sh [ok | muet]\n' >&2; exit 2 ;;
esac
[[ -n "${CACHE_SOURCES:-}" ]] || { printf 'temoin : CACHE_SOURCES vide, lance par just temoin\n' >&2; exit 2; }

etape() { printf 'temoin : %s\n' "$*"; }

# 1. Sources du noyau, dans le cache des sources, vérifiées à chaque construction.
archive="$CACHE_SOURCES/linux-$noyau_version.tar.xz"
if [[ ! -f "$archive" ]]; then
    etape "téléchargement de $noyau_url"
    curl -fL --retry 3 -o "$archive.partiel" "$noyau_url"
    mv "$archive.partiel" "$archive"
fi
if ! printf '%s  %s\n' "$noyau_sha256" "$archive" | sha256sum --check --quiet --strict; then
    rm -f "$archive"
    printf 'temoin : empreinte fausse, %s supprimée. Relance pour la retélécharger.\n' "$archive" >&2
    exit 1
fi

src="$sortie/linux-$noyau_version"
if [[ ! -f "$src/.extrait" ]]; then
    etape "extraction de linux-$noyau_version"
    rm -rf "$src"
    mkdir -p "$sortie"
    tar -xJf "$archive" -C "$sortie"
    touch "$src/.extrait"
fi

# 2. Initramfs : /init et les deux consoles. La liste au format de gen_init_cpio laisse le
# noyau créer les nœuds de périphérique, sans droits root.
travail="$sortie/initramfs-$variante"
mkdir -p "$travail"
etape "compilation de /init ($variante)"
gcc -static -Os -s -Wall -Wextra -Werror "${options_init[@]}" -o "$travail/init" "$ici/init.c"
cat >"$travail/liste" <<EOF
dir /dev 0755 0 0
nod /dev/console 0600 0 0 c 5 1
nod /dev/tty0 0620 0 0 c 4 0
file /init $travail/init 0755 0 0
EOF

# 3. Configuration : tinyconfig, plus noyau.config, plus le chemin de l'initramfs.
construction="$sortie/noyau"
mkdir -p "$construction"
fragment="$travail/noyau.config"
cat "$ici/noyau.config" >"$fragment"
printf 'CONFIG_INITRAMFS_SOURCE="%s"\n' "$travail/liste" >>"$fragment"
etape "configuration du noyau"
make -s -C "$src" O="$construction" tinyconfig >/dev/null
"$src/scripts/kconfig/merge_config.sh" -m -O "$construction" "$construction/.config" "$fragment" >/dev/null
make -s -C "$src" O="$construction" olddefconfig >/dev/null
manquantes=0
while IFS= read -r option; do
    if ! grep -qxF "$option" "$construction/.config"; then
        printf 'temoin : option refusée par la configuration du noyau : %s\n' "$option" >&2
        manquantes=1
    fi
done < <(grep '^CONFIG_' "$fragment")
[[ "$manquantes" -eq 0 ]] || exit 1

etape "compilation du noyau, $jobs en parallèle"
make -s -C "$src" O="$construction" -j"$jobs" bzImage
noyau="$construction/arch/x86/boot/bzImage"

# 4. Disque : table GPT, une partition EFI de 62 Mio en FAT32, le noyau à l'emplacement que le
# micrologiciel lance par défaut. mtools écrit la FAT sans montage ; identifiants fixes, pour
# que deux constructions donnent le même disque à la date des fichiers près.
etape "écriture de $image"
esp="$travail/esp.img"
rm -f "$esp" "$image"
mformat -i "$esp" -C -T $((62 * 2048)) -h 64 -s 32 -F -N 4e4f4d4f -v TEMOIN ::
mmd -i "$esp" ::/EFI ::/EFI/BOOT
mcopy -i "$esp" "$noyau" ::/EFI/BOOT/BOOTX64.EFI
truncate -s 64M "$image"
sfdisk --quiet "$image" <<EOF
label: gpt
label-id: 4E4F4D4F-0000-4000-8000-000000000000
start=2048, size=$((62 * 2048)), type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B, uuid=4E4F4D4F-0000-4000-8000-000000000001, name="EFI"
EOF
dd if="$esp" of="$image" bs=1M seek=1 conv=notrunc,sparse status=none
rm -f "$esp"
etape "prêt : $image (noyau $noyau_version, $(stat -c %s "$noyau") octets)"
