# Building the system from source

The base system is built from source, in the build container (`docs/build.md`), following
Linux From Scratch 13.1, systemd edition. The method and its limits are decided in
`docs/adr/0020-system-build-method.md`. This page covers the first part, lot S-10a: the cross
toolchain and the temporary tools, chapters 5 and 6 of the book.

## Commands

| Command | What it does |
| --- | --- |
| `just toolchain` | Downloads the missing sources, builds every step not built yet, then checks the toolchain |
| `just toolchain <step>` | Builds up to that step only, given by number or name: `just toolchain glibc`, `just toolchain 04` |
| `just toolchain-clean` | Deletes the output folder. The downloaded sources stay in the cache |
| `just test-toolchain` | Quick test of the scripts, without network nor compiling (see "Tests") |

`TOOLCHAIN_JOBS` sets the parallel compile jobs, 4 by default:
`TOOLCHAIN_JOBS=8 just toolchain`.

## What is built

| Step | LFS 13.1 | Result |
| --- | --- | --- |
| `00-layout` | 4.2 | Folders of the future root, `/bin`, `/lib` and `/sbin` linked to `/usr` |
| `01-binutils-pass1` | 5.2 | Cross assembler and linker, in `tools/` |
| `02-gcc-pass1` | 5.3 | Cross gcc, C and C++, in `tools/`, with GMP, MPFR and MPC built in |
| `03-linux-headers` | 5.4 | Kernel API headers, for glibc |
| `04-glibc` | 5.5 | The C library of the target |
| `05-libstdcxx` | 5.6 | The C++ library of the target |
| `06-m4` to `20-xz` | 6.2 to 6.16 | M4, ncurses, bash, coreutils, diffutils, file, findutils, gawk, grep, gzip, make, patch, sed, tar, xz |
| `21-binutils-pass2` | 6.17 | binutils for the target |
| `22-gcc-pass2` | 6.18 | gcc for the target |

The target triplet is `x86_64-nomos-linux-gnu`: the cross tools are
`tools/bin/x86_64-nomos-linux-gnu-gcc` and so on. The programs of chapter 6 are built for the
target and installed in their final place: they will only run once the system is entered as its
own root, in chapter 7 (lot S-10b).

## Files

| File | Role |
| --- | --- |
| `distro/bootstrap/sources.txt` | Manifest: name, version, SHA-256 and https URL of every source |
| `distro/bootstrap/fetch.sh` | Downloads what is missing into the source cache and checks every SHA-256 |
| `distro/bootstrap/build.sh` | Runs the steps in order, skips those already built, writes the logs |
| `distro/bootstrap/env.sh` | Environment of the steps, that of the `lfs` user of the book |
| `distro/bootstrap/lib.sh` | Shared functions: manifest, cache, `enter` and `unpack` for the steps |
| `distro/bootstrap/steps/NN-name.sh` | One step, one section of the book. Its `# Sources:` line lists the sources it reads |
| `distro/bootstrap/check.sh` | Final check of the toolchain |
| `distro/bootstrap/test.sh` | Quick test |

## Output

Everything goes to `distro/bootstrap/output/`, ignored by git:

| Folder | Content |
| --- | --- |
| `sysroot/` | The future root of the system, `$LFS` of the book. The cross tools are in `sysroot/tools/` |
| `logs/<step>.log` | Full log of each step, commands included (`set -x`) |
| `logs/durations` | Seconds taken by each step built |
| `stamps/<step>` | Fingerprint of each step built |
| `work/<step>/` | Sources of the step being built. Deleted when it succeeds, kept when it fails |

The cross tools record the path of the sysroot, `/repo/distro/bootstrap/output/sysroot` in the
container. Do not move the folder: rebuild instead.

## Resuming

A step that succeeds writes its stamp: the SHA-256 of its script, of `env.sh` and of the
manifest lines of its sources. `just toolchain` skips every step whose stamp matches. So:

- a second run builds nothing and only runs the check;
- after a failure, or `Ctrl-C`, the next run starts again at the step that did not finish;
- changing a step, the environment or the version of a source rebuilds that step and every step
  after it, since they were built with it. For a clean result after such a change, run
  `just toolchain-clean` first: files installed by the old version stay in the sysroot otherwise.

A failed step prints the end of its log, and leaves its sources in `work/`.

## Sources

Every archive is downloaded once into the source cache (`SOURCE_CACHE`, see `docs/build.md`),
over https only, redirects included. Its SHA-256 is checked after download and again before each
step that uses it. A wrong archive is deleted and the build stops.

The versions are those of LFS 13.1, systemd edition, tested together by the book. The URLs point
to the upstream projects: `ftp.gnu.org` for GNU packages, `cdn.kernel.org`, the sites of ncurses
and file, the releases of xz on GitHub, and the patches of the book on `linuxfromscratch.org`.

### Moving to a new version of LFS

1. Read the new book (`stable-systemd`) and its `md5sums` file on linuxfromscratch.org.
2. Change the versions and URLs in `sources.txt`, and the commands of the changed sections in
   `steps/`.
3. Download each new archive, check its MD5 against the book and, when the project signs its
   releases, its signature (`.sig` of GNU, `sha256sums.asc` of kernel.org, `.asc` of file, `.sig`
   of xz). Then write its SHA-256 in the manifest.
4. Run `just test-toolchain`, `just toolchain-clean` then `just toolchain`.

## Check

`check.sh` runs at the end of every `just toolchain`. It compiles the `dummy.c` of the book
(`int main(){}`) with the cross gcc and checks that:

- the program is an x86-64 ELF 64-bit file, whose interpreter is `/lib64/ld-linux-x86-64.so.2`;
- the start files, the headers, `libc.so.6` and the dynamic loader come from the sysroot, and the
  linker only searches the sysroot;
- the program, then the cross compiled bash, run through the new dynamic loader
  (`sysroot/usr/lib/ld-linux-x86-64.so.2 --library-path sysroot/usr/lib`);
- the target gcc of pass 2 and its `cc` link are installed.

## Tests

`just test-toolchain` runs `distro/bootstrap/test.sh` in the container, in a few seconds, without
network. It checks:

- the manifest: four fields, SHA-256 in lowercase hexadecimal, https URLs, no duplicate, and
  that malformed manifests are refused;
- that each source a step uses is declared on its `# Sources:` line and is in the manifest;
- the syntax of every script (`bash -n`), and `shellcheck` when the container has it;
- `fetch.sh --check` on a fake cache: right archive accepted, wrong archive refused and deleted,
  missing archive reported;
- resuming, with the driver in simulation (`TOOLCHAIN_SIMULATE=1`: stamps and order are real,
  nothing is compiled): a second run builds nothing, a changed step is rebuilt with those after
  it, a failed step stops the run and the next run starts again there, a target step, a dry run,
  `clean`.

The CI runs `just test-toolchain` in the `test-boot` job, and `shellcheck` on the scripts.

## Duration and size

MEASUREMENTS

## In the CI, later

The full build does not run in the CI: MEASUREMENTS_CI
