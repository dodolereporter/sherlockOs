#!/usr/bin/env bash
# Boot bench: starts a UEFI disk image in QEMU, without a screen, and tells whether it booted.
# Runs in the build container, where QEMU and OVMF are. How to use it: docs/boot-tests.md.
#
#   boot.sh <image>
#
# Succeeds (code 0) if a BOOT-OK line comes out on the serial port before the timeout. Fails
# (code 1) otherwise, and prints the last 50 lines of the serial port. Code 2: wrong usage or
# missing tool. In both cases, a screenshot of the graphical output is taken over QMP while
# QEMU runs.
#
# Environment variables:
#   BOOT_TIMEOUT   seconds given to the boot, 90 by default
#   BOOT_OUTPUT    folder of the serial log and of the screenshot,
#                  distro/tests/output/boot-<image name> by default
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
marker="BOOT-OK"
timeout="${BOOT_TIMEOUT:-90}"
ovmf_code=/usr/share/OVMF/OVMF_CODE_4M.fd
ovmf_vars=/usr/share/OVMF/OVMF_VARS_4M.fd

usage() {
    printf 'boot: %s\n' "$*" >&2
    exit 2
}

[[ $# -eq 1 ]] || usage "usage: boot.sh <image>"
image="$1"
[[ -f "$image" ]] || usage "image not found: $image"
[[ "$timeout" =~ ^[1-9][0-9]*$ ]] || usage "BOOT_TIMEOUT must be a number of seconds: $timeout"
for tool in qemu-system-x86_64 qemu-img python3; do
    command -v "$tool" >/dev/null || usage "$tool not found, run the bench with just test-boot"
done
[[ -r "$ovmf_code" && -r "$ovmf_vars" ]] || usage "OVMF firmware not found in /usr/share/OVMF"

name="$(basename "$image")"
output="${BOOT_OUTPUT:-$here/output/boot-${name%.*}}"
mkdir -p "$output"
log="$output/serial.log"
screenshot="$output/screenshot.png"
rm -f "$log" "$screenshot"
: >"$log"

# The format is read rather than guessed by QEMU, which refuses to write to a guessed raw image.
format="$(qemu-img info --output=json "$image" | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["format"])')"

temp="$(mktemp -d)"
qemu_pid=""
cleanup() {
    if [[ -n "$qemu_pid" ]] && kill -0 "$qemu_pid" 2>/dev/null; then
        python3 -I "$here/qmp.py" "$temp/qmp.sock" quit >/dev/null 2>&1 || kill "$qemu_pid" 2>/dev/null || true
        wait "$qemu_pid" 2>/dev/null || true
    fi
    rm -rf "$temp"
}
trap cleanup EXIT

# The UEFI variables must be writable: work on a throwaway copy.
cp "$ovmf_vars" "$temp/vars.fd"

if [[ -r /dev/kvm && -w /dev/kvm ]]; then
    acceleration=(-accel kvm -cpu host)
    printf 'boot: KVM available\n'
else
    acceleration=(-accel tcg -cpu max)
    printf 'boot: no KVM, QEMU emulates the processor (slower)\n'
fi

# -snapshot: the writes of the booted system do not touch the tested image.
# -no-reboot: a reboot or a crash stops QEMU instead of looping until the timeout.
qemu-system-x86_64 \
    -nodefaults -machine q35 "${acceleration[@]}" -smp 2 -m 1024 \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$ovmf_code" \
    -drive "if=pflash,format=raw,unit=1,file=$temp/vars.fd" \
    -drive "if=virtio,format=$format,file=$image" -snapshot \
    -vga std -display none \
    -chardev "file,id=serial,path=$log" -serial chardev:serial \
    -qmp "unix:$temp/qmp.sock,server=on,wait=off" \
    -no-reboot &
qemu_pid=$!
printf 'boot: %s starts, %s s given for the %s line\n' "$name" "$timeout" "$marker"

# The serial port writes \r\n line endings: the line is BOOT-OK, followed or not by \r.
result=""
start=$SECONDS
while [[ -z "$result" ]]; do
    if grep -qE "^${marker}"$'\r?$' "$log"; then
        result=passed
    elif ! kill -0 "$qemu_pid" 2>/dev/null; then
        qemu_code=0
        wait "$qemu_pid" || qemu_code=$?
        qemu_pid=""
        # QEMU may have written the line just before stopping: read it one last time.
        if grep -qE "^${marker}"$'\r?$' "$log"; then
            result=passed
        else
            result="QEMU stopped (code $qemu_code) without writing $marker"
        fi
    elif ((SECONDS - start >= timeout)); then
        result="no $marker line after $timeout s"
    else
        sleep 0.5
    fi
done
duration=$((SECONDS - start))

if [[ -n "$qemu_pid" ]]; then
    if python3 -I "$here/qmp.py" "$temp/qmp.sock" screendump "$screenshot"; then
        printf 'boot: screenshot in %s\n' "$screenshot"
    else
        printf 'boot: the screenshot failed\n' >&2
    fi
fi

if [[ "$result" == passed ]]; then
    printf 'boot: passed, %s seen after %s s. Serial log: %s\n' "$marker" "$duration" "$log"
    exit 0
fi

printf 'boot: FAILED, %s\n' "$result"
printf 'boot: last 50 lines of the serial port (%s):\n' "$log"
# Without the \r of the serial port nor the escape sequences of the firmware, which garble the
# terminal.
tail -n 50 "$log" | tr -d '\r' | sed -E $'s/\x1b\[[0-9;=?]*[A-Za-z]//g'
exit 1
