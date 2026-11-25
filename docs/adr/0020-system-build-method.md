# ADR 0020: Building the base system: Linux From Scratch, scripted in the container

- Status: proposed
- Date: 2026-11-25
- Lot: S-10a

## Context

The base system is built from source, with glibc, systemd and pacman (ADR 0002), for x86_64 in
UEFI only (ADR 0001). Track S starts with the toolchain: a cross compiler that produces programs
for the new system without depending on the libraries of the build machine. The build must run
in the build container (`distro/builder`, lot Q-03), without root, and be redone from scratch by
one command. One author maintains it: the method has to be documented and widely used, so that a
failure can be compared with what others get.

## Decision

1. **Linux From Scratch, systemd edition, version 13.1** (1 September 2026, the latest stable
   one on 25 November 2026) is the reference. Its package versions are taken as a set, because
   the book has built and tested them together. Its commands are followed, one script per
   section, in `distro/bootstrap/steps/`, and each deviation is written in the script.
2. **Target triplet `x86_64-nomos-linux-gnu`.** The vendor field only has to differ from the
   host's for the cross compilation to be real; it holds the placeholder of the OS name and
   changes with it.
3. **Sources**: a manifest, `distro/bootstrap/sources.txt`, gives name, version, SHA-256 and an
   https URL from the upstream project for each archive. The checksum is checked before every
   use. The SHA-256 values were taken from archives whose MD5 matched the LFS 13.1 book and,
   where the project signs its releases, whose signature was checked.
4. **Output**: the future root of the system (`$LFS` of the book) is
   `distro/bootstrap/output/sysroot`, ignored by git, seen as `/repo/distro/bootstrap/output/sysroot`
   in the container. This path is written into the cross tools, so it is the same on every
   machine.
5. **Resuming**: a step that succeeded leaves a stamp with its fingerprint (its script, the build
   environment, the checksums of its sources). The next run skips it. A changed step is rebuilt
   with every step after it.
6. **Reproducible inputs, not yet reproducible bits**: same container, same sources, an empty
   environment (`env -i`, as the `lfs` user of the book), the POSIX locale and fixed paths. Two
   builds are not compared byte for byte.

## Consequences

- `just toolchain` builds chapters 5 and 6 of the book: cross binutils and gcc, kernel headers,
  glibc, libstdc++, then the temporary tools. `just test-toolchain` checks the scripts in a few
  seconds, in the CI.
- The book has no SHA-256: each new version of LFS means downloading the new archives, checking
  them against its MD5 list and the upstream signatures, then writing the new checksums.
- Not done yet, left to the next lots:
  - chapter 7, the chroot: it needs root or user namespaces in the container, a choice of its own
    (S-10b);
  - chapter 8, the final system with systemd, and the image that boots it (S-10c, S-11);
  - the test suites of the packages, the 32-bit libraries (S-16), the pacman packages (S-12);
  - the full build in the CI, about an hour of compiling (see `docs/system-build.md`).

## What would make us reconsider

- LFS stops following glibc, gcc or systemd closely enough, or drops the systemd edition.
- The pacman recipes of S-12 make the chapter 6 tools useless, in favour of a bootstrap from
  packages built by makepkg.
- Bit-for-bit reproducible builds become a requirement, for instance for a signed public image.
