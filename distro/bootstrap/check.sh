#!/usr/bin/env bash
# Checks the built toolchain: the cross gcc compiles and links the dummy.c of Linux From Scratch
# for the target, against the sysroot, with the right program interpreter. Then the test program
# and the cross compiled bash run through the new dynamic loader.
# Runs in the build container, after build.sh. Called by `just toolchain`.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export TOOLCHAIN_OUTPUT="${TOOLCHAIN_OUTPUT:-$here/output}" TOOLCHAIN_JOBS=1
source "$here/lib.sh"
source "$here/env.sh"

loader=/lib64/ld-linux-x86-64.so.2
errors=0
check() {
    local label="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        printf 'ok      %s\n' "$label"
    else
        printf 'FAILED  %s\n' "$label"
        errors=1
    fi
}

[[ -x "$LFS/tools/bin/$LFS_TGT-gcc" ]] || fail "no toolchain in $LFS, run just toolchain"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"

echo 'int main(){}' >dummy.c
"$LFS_TGT-gcc" dummy.c -v -Wl,--verbose >dummy.log 2>&1
"$LFS_TGT-readelf" -h -l a.out >readelf.txt

check "dummy.c compiled and linked by $LFS_TGT-gcc" test -f a.out
check "ELF 64 bits for x86-64" grep -qE 'Class: +ELF64' readelf.txt
check "machine x86-64" grep -qE 'Machine: +Advanced Micro Devices X86-64' readelf.txt
check "program interpreter $loader" grep -qF "[Requesting program interpreter: $loader]" readelf.txt
check "start files from the sysroot" test "$(grep -cE "^$LFS/lib.*/S?crt[1in]\.o succeeded" dummy.log)" -eq 3
check "headers from the sysroot" grep -qx " $LFS/usr/include" dummy.log
check "libc of the sysroot" grep -qF "attempt to open $LFS/usr/lib/libc.so.6 succeeded" dummy.log
check "loader of the sysroot" grep -qF "found ld-linux-x86-64.so.2 at $LFS/usr/lib/ld-linux-x86-64.so.2" dummy.log
grep -o 'SEARCH_DIR("[^"]*")' dummy.log >search.txt || true
check "linker searches only the sysroot" bash -c '[[ -s search.txt ]] && ! grep -v "^SEARCH_DIR(\"=" search.txt'
check "$LFS$loader points to the new glibc" test "$(readlink -f "$LFS$loader")" = "$(readlink -f "$LFS/usr/lib/ld-linux-x86-64.so.2")"

# The kernel of the container is new enough for the new glibc (5.10 or later).
run() { "$LFS/usr/lib/ld-linux-x86-64.so.2" --library-path "$LFS/usr/lib" "$@"; }
check "the test program runs with the new loader" run ./a.out
check "the cross compiled bash runs" test "$(run "$LFS/usr/bin/bash" -c 'echo $((6 * 7))')" = 42
check "the target gcc of pass 2 is installed" test -x "$LFS/usr/bin/gcc" -a -L "$LFS/usr/bin/cc"

[[ "$errors" == 0 ]] || fail "the toolchain check failed"
printf 'toolchain: check passed, %s\n' "$("$LFS_TGT-gcc" --version | head -n 1)"
