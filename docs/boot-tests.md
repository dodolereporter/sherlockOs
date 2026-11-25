# QEMU boot bench

Every lot of the system track must prove that its image boots. The bench starts a disk image in
QEMU, without a screen, reads the serial port and tells whether the `BOOT-OK` line came out in
less than 90 seconds. It also takes a screenshot of the virtual screen. Everything runs in the
build container (see `docs/build.md`): the host only provides Docker and `just`.

## Commands

| Command | What it does |
| --- | --- |
| `just test-boot` | Builds the witness image, then boots it. Succeeds if `BOOT-OK` comes out |
| `just test-boot <image>` | Boots the given image. Only the default witness image, `distro/tests/output/witness.img`, is rebuilt first |
| `just witness` | Builds the witness image only: `distro/tests/output/witness.img` |
| `just witness silent` | Builds the variant that does not write `BOOT-OK`: `witness-silent.img` |
| `just test-bench` | Checks that the bench fails cleanly (see "Tests of the bench") |

The image given to `just test-boot` must be inside the repository, for instance under
`distro/tests/output/`: only the repository is mounted in the container.

## What `distro/tests/boot.sh` does

`boot.sh <image>` runs in the container. It:

1. reads the format of the image (`raw`, `qcow2`…) with `qemu-img info`;
2. starts `qemu-system-x86_64` as a `q35` machine, with the OVMF UEFI firmware (the target is UEFI,
   ADR 0001), the image as a virtio disk, a `std` graphics card without a window
   (`-display none`) and the serial port written to a file;
3. watches the serial port every half second, until the `BOOT-OK` line (read by
   `distro/tests/marker.py`, see "Reading the BOOT-OK line"), QEMU stopping or the end of the
   timeout;
4. takes a screenshot over QMP (`screendump`, as PNG), then stops QEMU.

| Result | Code | Output |
| --- | --- | --- |
| A line that is exactly `BOOT-OK`, kernel messages inside it aside, comes out before the timeout | 0 | Boot duration, path of the log |
| No `BOOT-OK` at the end of the timeout | 1 | `FAILED`, then the last 50 lines of the serial port |
| QEMU stops before (crash, reboot) | 1 | The same, with the exit code of QEMU |
| Missing image, invalid timeout, missing tool | 2 | The problem, without starting QEMU |
| `marker.py` cannot read the log | 2 | The problem, then QEMU is stopped |

A few choices:

- **`-snapshot`**: what the booted system writes to its disk is thrown away. The tested image does
  not change.
- **`-no-reboot`**: a kernel that crashes and reboots stops QEMU, instead of looping until the
  timeout.
- **KVM** when `/dev/kvm` can be opened, emulation (`-accel tcg`) otherwise. The bench says which.
- **UEFI variables**: copied to a temporary folder on every run, never kept.
- The `\r` line endings of the serial port and the escape sequences of OVMF are removed from the
  50 lines shown. The full log keeps everything.

| Variable | Default | Role |
| --- | --- | --- |
| `BOOT_TIMEOUT` | `90` | Seconds given to the boot |
| `BOOT_OUTPUT` | `distro/tests/output/boot-<image name>` | Folder of the log and of the screenshot |

Example: `BOOT_TIMEOUT=30 just test-boot distro/tests/output/witness.img`.

Every run writes into its output folder:

| File | Content |
| --- | --- |
| `serial.log` | The whole serial port: OVMF, kernel, `/init` |
| `screenshot.png` | The virtual screen at the time of the verdict, pass or fail |

An image that wants to pass the bench writes `BOOT-OK`, alone on its line, on the first serial port
(`ttyS0`, 115200 baud) once booted. The real image (lots S-11 and S-15) will have to do it. To keep
kernel messages out of the line, it first lowers the console log level (`dmesg -n 1`), and it keeps
the kernel timestamps on (`CONFIG_PRINTK_TIME`), which let the bench recognise a message that still
gets in.

## Reading the BOOT-OK line

The image writes `BOOT-OK` from user space, through the tty layer, while the kernel writes its
messages straight to the same serial port, sometimes late. A kernel message can therefore land in
the middle of the line. On 25 November 2026, the CI read `BOOT-OK[    0.207789] Freeing unused
kernel image...`, four more kernel lines, then the line ending of `BOOT-OK`, and failed although
the witness had booted.

`distro/tests/marker.py` reads the log for `boot.sh`. A kernel message is written whole, from its
timestamp (`[    0.207789] `, followed or not by the caller, `[    T1] `) to its line ending: every
such message is removed from the log, wherever it starts, then `BOOT-OK` is searched alone on a
complete line. Nothing else is removed, so these never pass:

- `BOOT-OK` inside a kernel message, for instance on the kernel command line;
- `BOOT-OK` with anything before or after it on its line, `BOOT-OKAY` for instance;
- a `BOOT-OK` line not finished yet (no line ending);
- `BOOT-OK` cut by a line without a kernel timestamp: systemd status, firmware.

`python3 distro/tests/marker.py --self-test` runs these cases, the 25 November log included.
`just test-bench` runs it.

## The witness image

It proves that the bench works, as long as the real image does not exist. Files in
`distro/tests/witness/`, built by `build.sh`, in the container:

| Item | Choice |
| --- | --- |
| Kernel | Linux 7.3.6, stable on kernel.org as of 25 November 2026, pinned with its SHA-256 checksum |
| Configuration | `make tinyconfig`, plus the options of `kernel.config`: ELF, initramfs, UEFI, serial port, console on the UEFI framebuffer |
| Initramfs | Built into the kernel. A single binary, `/init`, compiled statically from `init.c` |
| Disk | 64 MiB, GPT table, one EFI partition in FAT32, the kernel in `\EFI\BOOT\BOOTX64.EFI` |
| Kernel size | About 1.6 MB, initramfs included |

`/init` lowers the console log level, like `dmesg -n 1`, so that the kernel prints nothing but
emergencies from then on, messages already waiting included. Then it writes `BOOT-OK` on the
serial port and on the screen, and waits. The silent variant writes
another line instead. The firmware starts the kernel directly, without a boot loader: the kernel is
also a UEFI application (`CONFIG_EFI_STUB`), with its built-in command line.

`build.sh` checks that every option of `kernel.config` ends up in the final `.config`, and stops
otherwise: an option that `make olddefconfig` silently dropped would break the boot.

The kernel archive is downloaded once into the source cache (`SOURCE_CACHE`, see `docs/build.md`)
and its checksum is checked on every build. A wrong archive is deleted. Everything else goes to
`distro/tests/output/`, ignored by git: extracted sources (about 1.8 GB), build folder, images,
logs, screenshots. To erase everything, delete that folder.

The first build extracts the sources and compiles the kernel, then only the changed steps are
redone. `WITNESS_JOBS` sets the number of parallel compile jobs, every processor by default, for
instance `WITNESS_JOBS=4 just test-boot`.

### Moving to a newer kernel

1. Read the version on kernel.org and download `sha256sums.asc` from the `v<major>.x` folder.
2. Check its signature, made by the "Kernel.org checksum autosigner" key
   (`B8868C80BA62A1FFFAF5FDA9632D3A06589DA6B1`), for instance
   `gpg --auto-key-locate clear,wkd --locate-keys autosigner@kernel.org` then
   `gpg --verify sha256sums.asc`.
3. Copy the version and the checksum of the `.tar.xz` at the top of `build.sh`, then run
   `just test-boot` and `just test-bench`.

## Tests of the bench

`just test-boot` is the passing case. `just test-bench` builds the silent variant and checks, with
`distro/tests/test-bench.sh`, that:

- the cases of `marker.py --self-test` pass;
- the bench returns 1 after the 90 seconds, with the timeout message;
- it prints at most 50 lines of the serial port, including the line written by `/init` instead of
  `BOOT-OK`;
- the PNG screenshot is taken despite the failure;
- a missing image returns 2 without starting QEMU.

This test therefore lasts a little more than 90 seconds.

## In the CI

The workflow `.github/workflows/ci-boot.yml`, job `test-boot`, runs on every pull request and on
`main`, like the Rust CI (`docs/ci.md`). It:

1. makes `/dev/kvm` openable by the user of the machine, so that QEMU benefits from it;
2. restores the build image from the cache, or builds it;
3. runs `just test-builder`, `just test-boot` then `just test-bench`;
4. uploads the screenshots and serial logs in the `boot-screenshots` artifact, even on failure.

**Cache of the build image.** Building the image takes a few minutes and depends on
`snapshot.debian.org`. The image is therefore kept in the GitHub Actions cache, exported by
`docker save` and compressed with zstd. The key contains the digest of the `Dockerfile` and the uid
of the machine, the two things that make the fingerprint of `builder.sh`: a restored image is not
rebuilt. Only runs on `main` save the cache. Pull requests read it, and a pull request that
changes the `Dockerfile` rebuilds the image.

The kernel archive is not cached: it comes from the kernel.org CDN on every run, and its checksum
is checked.

To read a failure: the log of the `just test-boot` step shows the last 50 lines of the serial
port, and the `boot-screenshots` artifact gives the screenshot and the full log.
