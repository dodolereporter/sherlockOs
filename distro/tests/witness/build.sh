#!/usr/bin/env bash
# Builds the witness image of the boot bench: a GPT disk with an EFI partition holding a minimal
# Linux kernel, bootable by UEFI, and its built-in initramfs made of a single binary.
# Runs in the build container. Called by `just witness`. How to use it: docs/boot-tests.md.
#
#   build.sh [ok | silent]
#
#   ok       (default) /init writes BOOT-OK: output/witness.img
#   silent   /init writes another line: output/witness-silent.img, to test the bench failing
#
# Environment variable: WITNESS_JOBS, number of parallel compile jobs (nproc by default).
set -euo pipefail

# Stable kernel from kernel.org, pinned. To move to a newer version, change both lines together:
# the checksum comes from sha256sums.asc in the version folder, whose signature is checked.
kernel_version=7.3.6
kernel_sha256=8183d475a76a0faffa92d6cd500af421f589dfd0300a0f1060bb875e11d45723
kernel_url="https://cdn.kernel.org/pub/linux/kernel/v${kernel_version%%.*}.x/linux-$kernel_version.tar.xz"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
output="$(cd "$here/.." && pwd)/output"
variant="${1:-ok}"
jobs="${WITNESS_JOBS:-$(nproc)}"

case "$variant" in
    ok) image="$output/witness.img" ; init_options=() ;;
    silent) image="$output/witness-silent.img" ; init_options=(-DWITNESS_SILENT) ;;
    *) printf 'usage: build.sh [ok | silent]\n' >&2; exit 2 ;;
esac
[[ -n "${SOURCE_CACHE:-}" ]] || { printf 'witness: SOURCE_CACHE empty, run it with just witness\n' >&2; exit 2; }

step() { printf 'witness: %s\n' "$*"; }

# 1. Kernel sources, in the source cache, checked on every build.
archive="$SOURCE_CACHE/linux-$kernel_version.tar.xz"
if [[ ! -f "$archive" ]]; then
    step "downloading $kernel_url"
    curl -fL --retry 3 -o "$archive.partial" "$kernel_url"
    mv "$archive.partial" "$archive"
fi
if ! printf '%s  %s\n' "$kernel_sha256" "$archive" | sha256sum --check --quiet --strict; then
    rm -f "$archive"
    printf 'witness: wrong checksum, %s deleted. Run again to download it again.\n' "$archive" >&2
    exit 1
fi

src="$output/linux-$kernel_version"
if [[ ! -f "$src/.extracted" ]]; then
    step "extracting linux-$kernel_version"
    rm -rf "$src"
    mkdir -p "$output"
    tar -xJf "$archive" -C "$output"
    touch "$src/.extracted"
fi

# 2. Initramfs: /init and the two consoles. The list in gen_init_cpio format lets the kernel
# create the device nodes, without root rights.
work="$output/initramfs-$variant"
mkdir -p "$work"
step "compiling /init ($variant)"
gcc -static -Os -s -Wall -Wextra -Werror "${init_options[@]}" -o "$work/init" "$here/init.c"
cat >"$work/list" <<EOF
dir /dev 0755 0 0
nod /dev/console 0600 0 0 c 5 1
nod /dev/tty0 0620 0 0 c 4 0
file /init $work/init 0755 0 0
EOF

# 3. Configuration: tinyconfig, plus kernel.config, plus the path of the initramfs.
build="$output/kernel"
mkdir -p "$build"
fragment="$work/kernel.config"
cat "$here/kernel.config" >"$fragment"
printf 'CONFIG_INITRAMFS_SOURCE="%s"\n' "$work/list" >>"$fragment"
step "configuring the kernel"
make -s -C "$src" O="$build" tinyconfig >/dev/null
"$src/scripts/kconfig/merge_config.sh" -m -O "$build" "$build/.config" "$fragment" >/dev/null
make -s -C "$src" O="$build" olddefconfig >/dev/null
missing=0
while IFS= read -r option; do
    if ! grep -qxF "$option" "$build/.config"; then
        printf 'witness: option refused by the kernel configuration: %s\n' "$option" >&2
        missing=1
    fi
done < <(grep '^CONFIG_' "$fragment")
[[ "$missing" -eq 0 ]] || exit 1

step "compiling the kernel, $jobs jobs in parallel"
make -s -C "$src" O="$build" -j"$jobs" bzImage
kernel="$build/arch/x86/boot/bzImage"

# 4. Disk: GPT table, one 62 MiB EFI partition in FAT32, the kernel at the path the firmware
# boots by default. mtools writes the FAT without mounting; fixed identifiers, so that two
# builds give the same disk except for file dates.
step "writing $image"
esp="$work/esp.img"
rm -f "$esp" "$image"
mformat -i "$esp" -C -T $((62 * 2048)) -h 64 -s 32 -F -N 4e4f4d4f -v WITNESS ::
mmd -i "$esp" ::/EFI ::/EFI/BOOT
mcopy -i "$esp" "$kernel" ::/EFI/BOOT/BOOTX64.EFI
truncate -s 64M "$image"
sfdisk --quiet "$image" <<EOF
label: gpt
label-id: 4E4F4D4F-0000-4000-8000-000000000000
start=2048, size=$((62 * 2048)), type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B, uuid=4E4F4D4F-0000-4000-8000-000000000001, name="EFI"
EOF
dd if="$esp" of="$image" bs=1M seek=1 conv=notrunc,sparse status=none
rm -f "$esp"
step "ready: $image (kernel $kernel_version, $(stat -c %s "$kernel") bytes)"
