set shell := ["bash", "-euo", "pipefail", "-c"]

# Lists the recipes
default:
    @just --list

# Format, language, clippy and tests: what the CI requires
verify: fmt-check check-language lint test

# Checks the format without changing anything
fmt-check:
    cargo fmt --all -- --check

# Applies the format
fmt:
    cargo fmt --all

# Clippy on the whole workspace, warnings refused
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Runs every test of the workspace
test:
    cargo test --workspace --all-features

# Builds the whole workspace
build:
    cargo build --workspace

# Generates the wallpaper variants (PNG at 1080p, 1440p and 2160p, dithered) from their originals
wallpapers output="target/wallpapers":
    cargo run --release -p desktop-config --features variants --bin wallpaper-variants -- assets/wallpapers {{ quote(output) }}

# Installs Evidence for the user, without root: its binary in ~/.local/bin and its desktop entry, which the dock pins and the launcher lists
install-evidence:
    cargo build --release -p evidence
    install -D -m 755 target/release/evidence "$HOME/.local/bin/evidence"
    sed "s|^Exec=evidence |Exec=$HOME/.local/bin/evidence |" desktop/evidence/evidence.desktop \
        | install -D -m 644 /dev/stdin "${XDG_DATA_HOME:-$HOME/.local/share}/applications/evidence.desktop"

# Reports French left in development text (ADR 0017), after the checker's own test cases
check-language *paths:
    python3 distro/tests/check-language.py --self-test
    python3 distro/tests/check-language.py {{ paths }}

# Builds the image of the build container of the distribution
builder:
    distro/builder/builder.sh build

# Runs a command in the build container, repository mounted on /repo (bash without a command)
[positional-arguments]
in-builder *command:
    @distro/builder/builder.sh run "$@"

# Tests the build container: tools, user, source cache, exit codes
test-builder:
    JUST='{{ just_executable() }}' distro/builder/test.sh quick

# Builds the image twice without cache and compares the package versions (slow)
test-builder-repro:
    distro/builder/test.sh repro

# Parallel compile jobs for the witness kernel: every processor, unless WITNESS_JOBS is set
kernel_jobs := env("WITNESS_JOBS", num_cpus())

# Builds the witness image of the boot bench in the container: ok (writes BOOT-OK) or silent
witness variant="ok":
    distro/builder/builder.sh run env WITNESS_JOBS={{ kernel_jobs }} distro/tests/witness/build.sh {{ quote(variant) }}

# Boots a UEFI disk image in QEMU and waits for BOOT-OK (without argument: the witness image, built first)
test-boot image="distro/tests/output/witness.img":
    if [[ {{ quote(image) }} == distro/tests/output/witness.img ]]; then {{ quote(just_executable()) }} witness ok; fi
    distro/builder/builder.sh run env BOOT_TIMEOUT="${BOOT_TIMEOUT:-90}" BOOT_OUTPUT="${BOOT_OUTPUT:-}" distro/tests/boot.sh {{ quote(image) }}

# Checks that the bench fails cleanly: initramfs without BOOT-OK, missing image
test-bench:
    {{ quote(just_executable()) }} witness silent
    distro/builder/builder.sh run distro/tests/test-bench.sh
