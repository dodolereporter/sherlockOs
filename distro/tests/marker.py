#!/usr/bin/env python3
"""Tells whether a serial log holds the boot marker, alone on its line.

The booted system writes the marker from user space, through the tty layer, while the kernel
writes its messages straight to the same serial port. A kernel message can therefore land in the
middle of the marker line. On 25 November 2026 the CI read `BOOT-OK[    0.207789] Freeing unused
kernel image...`, four more kernel lines, then the line ending of `BOOT-OK`, and failed although
the image had booted.

A kernel message is written whole, from its timestamp to its line ending. Every such message is
removed from the log, wherever it starts, then the marker is searched alone on a complete line.
Nothing else is removed, so these never count: the marker inside a kernel message, the marker
with something before or after it on its line, a marker line not finished yet, and a marker cut
by a line that has no kernel timestamp (systemd status, firmware).

This relies on kernel timestamps on the console (CONFIG_PRINTK_TIME, on in the witness kernel and
in most distribution kernels). Without them, only a marker that nothing cut is seen.

Usage: marker.py MARKER LOG    code 0 if the marker line is in LOG, 1 if not, 2 on wrong usage
       marker.py --self-test   runs the cases below, code 0 if they all pass
"""

import re
import sys

# `[    0.207789] ` and, with CONFIG_PRINTK_CALLER, `[    0.207789][    T1] `, up to the line
# ending. The serial console writes `\r\n`, and the `\r` may come before the line ending of the
# marker, which the kernel message then follows.
KERNEL_MESSAGE = re.compile(rb"\[ *\d+\.\d{6}\](?:\[ *[CT]\d+\])? [^\r\n]*\r*\n")


def has_marker(marker, log):
    """True if `log` (bytes) holds `marker` alone on a complete line, kernel messages aside."""
    text = KERNEL_MESSAGE.sub(b"", log)
    line = re.compile(rb"^" + re.escape(marker) + rb"\r*\n", re.MULTILINE)
    return line.search(text) is not None


# (case, serial log, marker seen). The 25 November 2026 log, cut short, is the second case.
CASES = [
    ("clean line", b"[    0.210239] Run /init as init process\r\nBOOT-OK\r\n", True),
    (
        "kernel lines before the line ending (CI, 25 November 2026)",
        b"[    0.201830] sched_clock: Marking stable\r\n"
        b"BOOT-OK[    0.207789] Freeing unused kernel image (initmem) memory: 920K\r\n"
        b"[    0.208272] Write protecting the kernel read-only data: 6144k\r\n"
        b"[    0.210239] Run /init as init process\r\n"
        b"\r\n"
        b"[    0.510472] input: ImExPS/2 Generic Explorer Mouse\r\n",
        True,
    ),
    ("kernel line inside the marker", b"BOOT[    1.000000] random: crng init done\r\n-OK\r\n", True),
    ("kernel line between \\r and \\n", b"BOOT-OK\r[    1.000000] tsc: Refined TSC\r\n\n", True),
    (
        "kernel line with the caller",
        b"BOOT-[    0.207789][    T1] Run /init as init process\r\nOK\r\n",
        True,
    ),
    ("line ending without \\r", b"BOOT-OK\n", True),
    ("other line only", b"witness: silent initramfs, no marker\r\n", False),
    ("empty log", b"", False),
    ("marker inside a kernel message", b"[    0.000000] Command line: BOOT-OK\r\n", False),
    ("kernel message that is the marker", b"[    0.000000] BOOT-OK\r\n", False),
    ("text before the marker", b"xBOOT-OK\r\n", False),
    ("text after the marker", b"BOOT-OKAY\r\n", False),
    ("text after a kernel line", b"BOOT-OK[    0.207789] Run /init\r\nAY\r\n", False),
    ("line not finished", b"[    0.210239] Run /init as init process\r\nBOOT-OK", False),
    ("cut by a line without a timestamp", b"BOOT-\r\nOK\r\n", False),
    ("cut by a systemd status", b"BOOT-[  OK  ] Started journald.\r\nOK\r\n", False),
    ("cut by a wrong timestamp", b"BOOT-[0.2] x\r\nOK\r\n", False),
]


def self_test():
    failures = 0
    for name, log, expected in CASES:
        if has_marker(b"BOOT-OK", log) != expected:
            print(f"self-test: FAILED {name}: expected {expected}")
            failures += 1
    print(f"self-test: {len(CASES) - failures}/{len(CASES)} cases pass")
    return 1 if failures else 0


def main():
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if len(sys.argv) != 3 or not sys.argv[1]:
        print("usage: marker.py MARKER LOG | --self-test", file=sys.stderr)
        return 2
    try:
        with open(sys.argv[2], "rb") as file:
            log = file.read()
    except OSError as error:
        print(f"marker: {error}", file=sys.stderr)
        return 2
    return 0 if has_marker(sys.argv[1].encode(), log) else 1


if __name__ == "__main__":
    sys.exit(main())
