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

# Revision of the public repository dodolereporter/sherlock that holds the mascot's model, and the SHA-256 of its sherlock.glb (ADR 0024)
mascot_revision := "5af31dc452b52db7046a761667106e24d41dfa13"
mascot_sha256 := "a50ec91a1801557333d94920f47c78537a854d5c1659431915b0f2f849ac9229"

# Downloads the mascot's 3D model at the pinned revision and checks its SHA-256; it is not in this repository
mascot output="":
    #!/usr/bin/env bash
    set -euo pipefail
    output={{ quote(output) }}
    if [[ -z "$output" ]]; then
        output="${XDG_DATA_HOME:-$HOME/.local/share}/sherlockos/mascot/sherlock.glb"
    fi
    if [[ -f "$output" ]] && echo "{{ mascot_sha256 }}  $output" | sha256sum --check --status; then
        echo "already there: $output"
        exit 0
    fi
    mkdir -p "$(dirname "$output")"
    partial=$(mktemp "$output.XXXXXX")
    trap 'rm -f "$partial"' EXIT
    curl --fail --silent --show-error --location --retry 3 --output "$partial" \
        "https://raw.githubusercontent.com/dodolereporter/sherlock/{{ mascot_revision }}/sherlock.glb"
    echo "{{ mascot_sha256 }}  $partial" | sha256sum --check --quiet
    chmod 644 "$partial"
    mv "$partial" "$output"
    echo "downloaded: $output"

# Generates the cursor themes sherlockos and sherlockos-light from the SVGs of assets/cursors
cursors output="target/cursors":
    cargo run --release -p cursor-theme --features render -- build assets/cursors {{ quote(output) }}

# Draws the board of every cursor, dark and light, for docs/cursor.md
cursor-board output="docs/cursor/board.png":
    cargo run --release -p cursor-theme --features render -- board assets/cursors {{ quote(output) }}

# Builds the session (compositor, bar, dock) and the cursor themes, then prints the sudo commands that install them for the login screen, without running them
install-session: cursors
    cargo build --release -p compositor -p bar -p dock -p launcher -p apps -p notifications -p evidence -p lock
    desktop/compositor/session/install.sh "$PWD"

# Builds the compositor and the login screen, then prints the sudo commands that make it the display manager (greetd, ADR 0026) and those that go back to SDDM, without running them
install-greeter:
    cargo build --release -p compositor -p greeter
    desktop/greeter/install.sh "$PWD"

# Installs Evidence for the user, without root: its binary in ~/.local/bin, its desktop entry, which the dock pins and the launcher lists, and its icon
install-evidence:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p evidence
    exe="$HOME/.local/bin/evidence"
    install -D -m 755 "${CARGO_TARGET_DIR:-target}/release/evidence" "$exe"
    # Exec quoted as the desktop entry specification wants: \ " ` $ escaped inside the quotes,
    # then every backslash doubled by the string escape, and % doubled.
    quoted=$(printf '%s' "$exe" | sed -e 's/[\\"`$]/\\&/g' -e 's/\\/\\\\/g' -e 's/%/%%/g')
    entry=$(while IFS= read -r line; do
        case "$line" in
            "Exec=evidence "*) printf 'Exec="%s" %s\n' "$quoted" "${line#Exec=evidence }" ;;
            *) printf '%s\n' "$line" ;;
        esac
    done < desktop/evidence/evidence.desktop)
    printf '%s\n' "$entry" | install -D -m 644 /dev/stdin "${XDG_DATA_HOME:-$HOME/.local/share}/applications/evidence.desktop"
    # Its icon in the user's hicolor theme, at each size of desktop/sherlock-ui/icons/hicolor.
    theme=desktop/sherlock-ui/icons/hicolor
    for file in "$theme"/*/apps/evidence.*; do
        install -D -m 644 "$file" "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/${file#"$theme"/}"
    done

# Installs Control Room for the user, without root: its binary in ~/.local/bin, its desktop entry and its icon
install-control-room:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p control-room
    exe="$HOME/.local/bin/control-room"
    install -D -m 755 "${CARGO_TARGET_DIR:-target}/release/control-room" "$exe"
    # Exec quoted as the desktop entry specification wants, as for Evidence.
    quoted=$(printf '%s' "$exe" | sed -e 's/[\\"`$]/\\&/g' -e 's/\\/\\\\/g' -e 's/%/%%/g')
    sed "s|^Exec=control-room$|Exec=\"$quoted\"|" desktop/control-room/control-room.desktop \
        | install -D -m 644 /dev/stdin "${XDG_DATA_HOME:-$HOME/.local/share}/applications/control-room.desktop"
    theme=desktop/sherlock-ui/icons/hicolor
    for file in "$theme"/*/apps/control-room.*; do
        install -D -m 644 "$file" "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/${file#"$theme"/}"
    done

# Writes the GTK, Qt and KDE themes under a folder laid out like a home folder, to look at them; overwrites nothing unless `force` is `--force`
themes output="target/themes" force="":
    cargo run -p toolkit-theme -- generate {{ force }} {{ quote(output) }}

# Installs the colors of the desktop for GTK, Qt and KDE applications, without root: every file it replaces is kept as <name>.bak-<date>
install-themes:
    cargo build --release -p toolkit-theme
    "${CARGO_TARGET_DIR:-target}/release/toolkit-theme" install

# Takes away what install-themes installed and puts the backups back
uninstall-themes:
    cargo build --release -p toolkit-theme
    "${CARGO_TARGET_DIR:-target}/release/toolkit-theme" uninstall

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
