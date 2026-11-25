#!/usr/bin/env bash
# Checks that the boot bench fails cleanly. Runs in the build container, after
# `just witness silent`. Called by `just test-bench`. How to use it: docs/boot-tests.md.
#
# The passing case is `just test-boot` itself: it is not repeated here.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
output="$here/output"

failures=0
pass() { printf 'ok      %s\n' "$1"; }
fail() {
    printf 'FAILED  %s\n' "$1"
    if [[ -n "${2:-}" ]]; then printf '        %s\n' "$2"; fi
    failures=$((failures + 1))
}
capture() {
    code=0
    bench_output="$("$@" 2>&1)" || code=$?
}

# 1. An initramfs that does not write BOOT-OK: the bench waits for the whole timeout, returns 1
# and shows the end of the serial port, where the line written by /init instead must appear.
image="$output/witness-silent.img"
folder="$output/boot-witness-silent"
if [[ ! -f "$image" ]]; then
    fail "silent image present" "$image is missing, run just test-bench"
else
    start=$SECONDS
    capture "$here/boot.sh" "$image"
    duration=$((SECONDS - start))
    printf '%s\n' "$bench_output" | sed 's/^/        | /'
    lines="$(printf '%s\n' "$bench_output" | sed -n '/last 50 lines/,$p' | tail -n +2 | wc -l)"
    if [[ "$code" -eq 1 ]]; then pass "silent initramfs: code 1"; else fail "silent initramfs: code 1" "code $code"; fi
    if [[ "$bench_output" == *"no BOOT-OK line after ${BOOT_TIMEOUT:-90} s"* && "$duration" -ge "${BOOT_TIMEOUT:-90}" ]]; then
        pass "silent initramfs: fails at the end of the timeout, $duration s"
    else
        fail "silent initramfs: fails at the end of the timeout" "$duration s, expected message missing"
    fi
    if [[ "$lines" -ge 1 && "$lines" -le 50 && "$bench_output" == *"witness: silent initramfs"* ]]; then
        pass "silent initramfs: last $lines lines of the serial port shown, /init line included"
    else
        fail "silent initramfs: end of the serial port shown" "$lines lines, /init line missing"
    fi
    if [[ "$(head -c 8 "$folder/screenshot.png" 2>/dev/null | od -An -tx1 | tr -d ' \n')" == 89504e470d0a1a0a ]]; then
        pass "silent initramfs: PNG screenshot taken despite the failure"
    else
        fail "silent initramfs: PNG screenshot taken despite the failure" "$folder/screenshot.png missing or not a PNG"
    fi
fi

# 2. A missing image: usage error, code 2, without starting QEMU.
capture "$here/boot.sh" "$output/missing.img"
if [[ "$code" -eq 2 && "$bench_output" == *"image not found"* ]]; then
    pass "missing image: code 2"
else
    fail "missing image: code 2" "code $code: $bench_output"
fi

if [[ "$failures" -gt 0 ]]; then
    printf '%s check(s) failed\n' "$failures"
    exit 1
fi
printf 'all good\n'
