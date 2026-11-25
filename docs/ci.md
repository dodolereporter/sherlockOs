# CI Rust

The CI fails any pull request that does not pass `just verify`. It runs on GitHub Actions,
described in `.github/workflows/ci-rust.yml`.

## When it runs

| Event | What happens |
| --- | --- |
| Pull request, opened or updated | One run. A new commit cancels the one in progress |
| Commit on `main` | One run, carried to the end, which also saves the cache |

## What it checks

A single job, `verify`, runs `just verify`. It chains three recipes of the `justfile` and stops at
the first one that fails.

| Recipe | Command | Fails if |
| --- | --- | --- |
| `just fmt-check` | `cargo fmt --all -- --check` | A file does not follow `rustfmt.toml` |
| `just lint` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Clippy or rustc emits a warning, or a lint of `[workspace.lints]` is broken |
| `just test` | `cargo test --workspace --all-features` | A test fails or does not compile |

The CI has no command list of its own: it runs the recipes of the `justfile`. To change what it
checks, change `just verify`, and the local machine follows.

The image build and the boot in QEMU, required by rule 3 of `CLAUDE.md`, run in a second workflow,
`.github/workflows/ci-boot.yml`, job `test-boot`, described in `docs/boot-tests.md`. It adds two
official actions, `actions/cache` and `actions/upload-artifact`, pinned by digest like the others.
A pull request is green only if `verify` and `test-boot` both are.

## How it sets up

- **Rust.** `rustup toolchain install`, without an argument, installs the version and the
  components written in `rust-toolchain.toml`. No version is written in the workflow: changing
  `rust-toolchain.toml` is enough.
- **System libraries.** `libxkbcommon-dev` and `libwayland-dev`, installed by `apt-get` on every
  run, because the CI machine is new every time. The compositor (smithay) links to libxkbcommon,
  and its integration test loads libwayland-client. `xwayland` provides the `Xwayland` binary,
  which the XWayland integration test (`tests/xwayland.rs`) starts, and `libegl1` the
  `libEGL.so.1` without which Xwayland stops at startup: libepoxy loads it at run time, and the
  `xwayland` package does not pull it in. Any system library added here is also added to the
  prerequisites of the local machine.
- **just.** Installed by `taiki-e/install-action`, version 1.59.0, pinned in the workflow.
- **Cargo cache.** `Swatinem/rust-cache` keeps the Cargo registry and the `target` folder. Its key
  contains the rustc version and `Cargo.lock`: a change to either starts from a fresh cache. Only
  runs on `main` save the cache. Pull requests read it without overwriting it.
- **Third-party actions.** Each one is pinned by the full digest of its commit, with the version in
  a comment. To upgrade one, replace the digest and the comment together.
- **Machine.** `ubuntu-24.04`, named explicitly rather than `ubuntu-latest`, so that an image change
  does not break the CI by surprise.
- **Maximum duration.** 15 minutes. Beyond that, the run is stopped and counts as a failure.

## Running it locally

Before opening a pull request:

```sh
just verify
```

This is exactly what the CI runs, with the same Rust version, read from `rust-toolchain.toml`.
Prerequisites of the local machine: `rustup`, `just`, a C compiler (`gcc` and `libc6-dev` on
Debian), the compositor libraries (`libxkbcommon-dev` and `libwayland-dev` on Debian) and the
`Xwayland` server (`xwayland` package on Debian).

To run a single step:

```sh
just fmt-check   # format only
just fmt         # fixes the format
just lint        # clippy only
just test        # tests only
```

`just check-language` runs `distro/tests/check-language.py`, which reports French left in
development text (ADR 0017), after its own test cases (`--self-test`). It skips Rust string
literals, interface labels quoted « … » in docs and Rust comments, and data files. A French name
kept on purpose in a script or configuration file is announced by a comment containing
`check-language: allow-next-line` on the line above. It is not part of `just verify` yet.

## Reading a failure

In the pull request, the `CI Rust / verify` or `CI boot / test-boot` check turns red. The
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
of `CLAUDE.md` holds by discipline: never merge a pull request unless `CI Rust / verify` and
`CI boot / test-boot` are both green.
