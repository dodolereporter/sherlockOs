# Contributing to SherlockOS

Thanks for your interest. SherlockOS is a passion project with a high bar for finish: a small,
well-tested change is worth more than a large one. Read this page, then `CLAUDE.md` (the rules
of the repository, written for every contributor, human or not) and `docs/contexte.md` (the
project's context, in French).

By taking part, you agree to follow the [code of conduct](CODE_OF_CONDUCT.md).

## Before you start

- **Look at the [roadmap](ROADMAP.md).** Work is cut into lots with an ID (`D-21`, `E-03`,
  `Q-30`…). Pick a planned lot, or open an issue to propose one, and say you are on it so that
  two people do not do the same thing.
- **Architecture decisions go through an ADR.** Copy `docs/adr/0000-modele.md` under the next
  number, in English, and open it as its own pull request. A decision applies only once the
  author has approved it. The frozen decisions (ADRs 0001 to 0013) are not discussed inside a
  lot.
- **Bugs and ideas**: use the issue templates. **Vulnerabilities**: never in an issue, see
  [SECURITY.md](SECURITY.md).

## Build

Prerequisites (Debian or Ubuntu names):

```sh
sudo apt install gcc libc6-dev python3 \
  libxkbcommon-dev libwayland-dev libudev-dev libinput-dev libseat-dev libgbm-dev libdrm-dev \
  xwayland libegl1 libegl-mesa0 dbus-daemon mesa-vulkan-drivers
```

Then `rustup` and [`just`](https://github.com/casey/just). The Rust version comes from
`rust-toolchain.toml` and installs itself; do not change it (it is copied into `rust-version` of
the root `Cargo.toml`, both change together). `just mascot` fetches Sherlock's 3D model, which
the mascot's tests need.

| Command | What it does |
| --- | --- |
| `just verify` | Everything the CI checks: `fmt-check`, `check-language`, `lint`, `test` |
| `just fmt` | Applies the format |
| `just check-language` | Reports French left in development text |
| `just lint` | Clippy on the whole workspace, warnings refused |
| `cargo test -p <crate> <filter>` | The tests of your crate, faster than the whole suite |

See the [README](README.md#try-it-in-a-window) to run the desktop nested in a window.

## Rules of the repository

From `CLAUDE.md`, which is the reference:

- **English for development** (ADR 0017): identifiers, file and folder names, comments, docs,
  commits and pull requests. **The interface stays in French**, addressing the user as "tu":
  never translate interface text. `just check-language` must pass; it ignores string literals
  and French quotes « … » in docs. ADRs 0000 to 0016, `docs/design/`, `docs/plan.md` and
  `docs/contexte.md` stay in French.
- **Lints** are defined once in `[workspace.lints]` of the root `Cargo.toml`, and every crate
  takes them with `[lints] workspace = true`. Never disable one to get the CI to pass.
- **No `unwrap`, `expect`, `println!`, `eprintln!` or `dbg!` outside tests.** Every `unsafe`
  block has a `// SAFETY:` comment, every public `unsafe` function a `# Safety` section.
- Typed errors in libraries, errors with context in binaries, logs with `tracing`.
- **No color, size or interface text hard-coded outside `desktop/sherlock-ui`.**
- **Every external input is bounded and validated**: files, D-Bus, Wayland requests, network.
  Nothing blocking on the interface thread.
- **Every new dependency is justified in the pull request**, with a pinned version and its
  license (it must be compatible with GPL-3.0-or-later; see [THIRD_PARTY.md](THIRD_PARTY.md)).
- **Every lot ships its tests and its doc page** in `docs/`.
- No personal data in code, fixtures or screenshots: no real names, user names, network names,
  serial numbers or e-mail addresses. Use made-up ones.

### The finish of a Mac

Any change to the interface is judged on how it feels, after macOS and iOS:

- animations on the way in **and** on the way out, with the durations of `sherlock_ui::motion`;
- hover, pressed and disabled states;
- rubber-band at the limits;
- "…" rather than clipped text;
- the right cursor;
- a layout that adapts to every screen size and scale: small laptop, 1920 × 1080, 4K, scales
  1.25, 1.5 and 2. Nothing computed for a single screen.

**Screenshots at two sizes at least, one of them 1920 × 1080**, light and dark themes when
both apply, are part of the pull request. Take them from your own windows only.

## Pull requests

- **One lot, one branch, one pull request.** Branch `lot/<ID>` from an up-to-date `main`.
- **About 600 changed lines at most**, generated code excluded. Split a larger lot into stacked
  pull requests (`D-21c1`, `D-21c2`…).
- **Commits**: `<ID>: <what changes>`, in English, in the present tense. Example:
  `D-10: add window moving`. No AI signature: no `Co-Authored-By` line for a tool, no
  "Generated with" mention.
- **The CI must be green**: both `verify` (CI Rust) and `test-boot` (CI boot). There is no
  branch protection: a pull request is never merged with a red CI.
- Fill in the template: the lot, what changes, the tests, the screenshots, and what was **not**
  tried for real (on hardware, in a real session).
- Changes that touch the kernel, the driver, the lock, the login or Sherlock's consents are
  merged only after a test on real hardware.
- In doubt, stop and ask your question in the pull request rather than guessing.

## License of contributions

There is no contributor license agreement and no sign-off required. **By opening a pull
request, you agree that your contribution is published under the GNU General Public License,
version 3 or later**, like the rest of the code, and that you have the right to submit it. A
`Signed-off-by` line (`git commit -s`) is welcome but not needed.

Contributions to the reserved identity (name, logo, mascot, wallpapers): see
[TRADEMARKS.md](TRADEMARKS.md).
