# CI Rust

The CI fails any pull request that does not pass `just verify`. It runs on GitHub Actions,
described in `.github/workflows/ci-rust.yml`, in two jobs side by side since lot Q-18b.

## When it runs

| Event | What happens |
| --- | --- |
| Pull request, opened or updated | One run. A new commit cancels the one in progress |
| Commit on `main` | One run, carried to the end, which also saves the cache |

## What it checks

Two jobs run side by side, each on its machine:

- `checks` runs `just checks`: `fmt-check`, `check-language`, `lint`, then `check-tokens`,
  stopping at the first one that fails. The two checks that compile nothing come first, so they fail in seconds.
- `verify` runs `just test-ci`: the tests through `cargo-nextest`, then the doc tests.

| Recipe | Command | Fails if |
| --- | --- | --- |
| `just fmt-check` | `cargo fmt --all -- --check` | A file does not follow `rustfmt.toml` |
| `just check-language` | `distro/tests/check-language.py --self-test`, then `distro/tests/check-language.py` | One of the checker's own cases fails, or French is left in development text (ADR 0017) |
| `just lint` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Clippy or rustc emits a warning, or a lint of `[workspace.lints]` is broken |
| `just check-tokens` | `cargo check -p sherlock-ui --no-default-features --lib` | sherlock-ui does not build without its `iced` feature, as the compositor and the configuration crates take it: `lint` builds every feature and does not see it (lot Q-53) |
| `just test-ci` | `cargo nextest run --workspace --all-features --profile ci`, then `cargo test --workspace --all-features --doc` | A test fails or does not compile |

Locally, `just verify` runs `just checks` then `just test`, the same tests through `cargo test`:
nothing more to install. The CI has no command list of its own: it runs the recipes of the
`justfile`.

**Why nextest** (lot Q-18b). `cargo test` runs the test binaries one after the other, each with
its tests side by side: the whole was the sum of the binaries, about three minutes once D-25d
optimized the software renderer, and one test of a single binary (the lock screen's captures)
held a minute alone. `cargo-nextest` runs every test in a process of its own, the tests of all
binaries side by side on the four processors of the machine. Its settings are in
`.config/nextest.toml`, profile `ci`: every failure reported, a test slower than a minute named,
and the tests that measure time against a fake service (the media keys, the session lock and
logind, the frame cycle, the watchers of `system-services`) running alone, so that the drawing
tests beside them do not slow those services down. A test that keeps a global of the process
(the color theme of the captures) is safe there, each process having its own; under `cargo test`
those tests still take a lock, as before.

The image build and the boot in QEMU, required by rule 3 of `CLAUDE.md`, run in a second workflow,
`.github/workflows/ci-boot.yml`, job `test-boot`, described in `docs/boot-tests.md`. It adds two
official actions, `actions/cache` and `actions/upload-artifact`, pinned by digest like the others.
A pull request is green only if `verify` and `test-boot` both are.

## How it sets up

- **Rust.** `rustup toolchain install`, without an argument, installs the version and the
  components written in `rust-toolchain.toml`. No version is written in the workflow: changing
  `rust-toolchain.toml` is enough.
- **System libraries.** `libxkbcommon-dev`, `libwayland-dev`, `libudev-dev`, `libinput-dev`,
  `libseat-dev`, `libgbm-dev` and `libdrm-dev`, installed by `apt-get` on every run, because the
  CI machine is new every time. The compositor (smithay) links to libxkbcommon, and to udev,
  libinput, libseat and GBM for the real session (lot D-14); its integration test loads
  libwayland-client. `xwayland` provides the `Xwayland` binary,
  which the XWayland integration test (`tests/xwayland.rs`) starts, and `libegl1` the
  `libEGL.so.1` without which Xwayland stops at startup: libepoxy loads it at run time, and the
  `xwayland` package does not pull it in. `libegl-mesa0` is Mesa's EGL, whose software device
  (llvmpipe) draws the genie of the compositor without a GPU (`tests/genie.rs`, lot D-19): the
  test fails on the CI without it. `dbus-daemon` runs the private session buses of the
  notification daemon's tests (`desktop/notifications/tests/dbus.rs`). `mesa-vulkan-drivers`
  gives Mesa's software Vulkan driver (lavapipe), on which `sherlock-3d` renders the mascot off
  screen (lot D-30). Any system library added here is also added to the prerequisites of the
  local machine.
- **Mascot.** `just mascot` downloads the 3D model of Sherlock (`sherlock.glb`, 1.4 MiB) from
  the public repository `dodolereporter/sherlock`, at the revision and SHA-256 pinned in the
  `justfile`: it is not in this repository (ADR 0024). `just verify` then runs with
  `SHERLOCK_3D_REQUIRE=1`, so the tests of `sherlock-3d` fail instead of skipping when the
  model or the Vulkan driver is missing.
- **Python.** `check-language` runs with the `python3` of the image (3.12 on `ubuntu-24.04`) and
  only uses its standard library. The workflow prints its version before `just verify`.
- **just and cargo-nextest.** Installed by `taiki-e/install-action`, which checks their
  release files: just 1.59.0 in both jobs, cargo-nextest 0.9.151 in `verify`, pinned in the
  workflow.
- **Cargo cache.** `Swatinem/rust-cache` keeps the Cargo registry and the `target` folder. Its key
  contains the rustc version and `Cargo.lock`: a change to either starts from a fresh cache. Only
  runs on `main` save the cache. Pull requests read it without overwriting it. Each job keeps a
  cache of its own: clippy's for `checks`, the tests' build for `verify`.
- **Third-party actions.** Each one is pinned by the full digest of its commit, with the version in
  a comment. To upgrade one, replace the digest and the comment together.
- **Machine.** `ubuntu-24.04`, named explicitly rather than `ubuntu-latest`, so that an image change
  does not break the CI by surprise.
- **Disk.** The `target` folder of the whole workspace no longer fits on the runner's disk with
  full debug information (lot D-14 brought the real session's dependencies: `ld` died with a bus
  error, the next run with `No space left on device`). The CI builds with
  `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, which keeps file and line in backtraces, and
  `CARGO_INCREMENTAL=0`.
- **Maximum duration.** 30 minutes per job (15 for the single job before Q-24). Beyond that, the
  run is stopped and counts as a failure.

## Running it locally

Before opening a pull request:

```sh
just verify
```

This is exactly what the CI runs, with the same Rust version, read from `rust-toolchain.toml`.
Prerequisites of the local machine: `rustup`, `just`, `python3`, a C compiler (`gcc` and `libc6-dev` on
Debian), the compositor libraries (`libxkbcommon-dev`, `libwayland-dev`, `libudev-dev`,
`libinput-dev`, `libseat-dev`, `libgbm-dev` and `libdrm-dev` on Debian), the `Xwayland` server
(`xwayland` package on Debian), Mesa's EGL (`libegl-mesa0`, without which the genie test only
says it has nothing to draw with), `dbus-daemon` (package of the same name) and, for the
mascot's off-screen render on a machine without an AMD chip, Mesa's Vulkan drivers
(`mesa-vulkan-drivers`). The tests of `sherlock-3d` need `just mascot` first; without the model
they say so and pass.

To run a single step:

```sh
just fmt-check       # format only
just check-language  # French left in development text
just fmt             # fixes the format
just lint            # clippy only
just checks          # the above and check-tokens: the CI's `checks` job
just test            # tests only, through cargo test
just test-ci         # tests as the CI runs them (needs cargo-nextest)
```

`just check-language` runs `distro/tests/check-language.py`, which reports French left in
development text (ADR 0017), after its own test cases (`--self-test`). It skips Rust string
literals, interface labels quoted « … » in docs and Rust comments, and data files. A French name
kept on purpose in a script or configuration file is announced by a comment containing
`check-language: allow-next-line` on the line above. It is part of `just verify`, so the CI fails
a pull request that leaves French in development text. To check only some paths:
`just check-language docs/ci.md justfile`.

## Reading a failure

In the pull request, the `CI Rust / checks`, `CI Rust / verify` or `CI boot / test-boot` check
turns red. The
"Details" link opens the log: the last recipe started by `just` is the one that failed, and its
output gives the file and line at fault. Fix it, run the same recipe locally (`just verify`, or
`just test-boot` and `just test-bench`), push. For `test-boot`, the `boot-screenshots` artifact
keeps the screenshot and the serial log.

Never disable a lint or a test to get the CI to pass.

## Making the check blocking

The workflow marks the pull request as failed, but it does not remove the merge button. For GitHub
to refuse the merge, the `verify` check must be declared required on `main`, by a branch protection
rule or a ruleset ("Require status checks to pass").

As of 24 November 2026, this setting is not available on this repository: it is private, and
GitHub reserves branch protection of private repositories to the Pro plan. The API answers
"Upgrade to GitHub Pro or make this repository public to enable this feature". Meanwhile, rule 1
of `CLAUDE.md` holds by discipline: never merge a pull request unless `CI Rust / checks`,
`CI Rust / verify` and `CI boot / test-boot` are all green.
