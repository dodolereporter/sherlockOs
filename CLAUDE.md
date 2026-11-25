First of all, read `docs/contexte.md` in full. Then read this file, then the brief of your lot.

# NOM_OS

A Linux OS built from source, with a desktop in Rust and an AI agent named Sherlock.
The plan, milestones and lots are in `docs/plan.md`. Architecture decisions are in `docs/adr/`.

## Language

All development is in English (ADR 0017): identifiers, names of folders, crates, binaries, files
and recipes, command-line options, environment variables, code comments, technical documentation,
commit messages and pull requests. This rule replaces the "Langue" section of `docs/contexte.md`.

The user interface stays in French, addressing the user as "tu": never translate interface text
(the strings of `sherlock_ui`, messages shown to the user, what Sherlock says, `.desktop` files).

ADRs 0000 to 0016 stay in French for the record, and so does `docs/design/`. `docs/plan.md` and
`docs/contexte.md` are still in French and may cite old names: the names in this file are the
current ones. Briefs may be written in French; what you produce follows this rule.

`just check-language` reports French left in comments, file names and identifiers.

## Layout

| Folder | Content |
| --- | --- |
| `distro/` | Build container, package recipes, image factory, boot tests |
| `desktop/compositor/` | Wayland compositor, on Smithay (binary) |
| `desktop/sherlock-ui/` | Colors, radii, typography, components (library) |
| `desktop/<program>/` | One crate per shell program and per application, to come |
| `sherlock/sherlockd/` | Daemon of the Sherlock agent (binary) |
| `sherlock/<server>/` | Consent broker and one crate per MCP server, to come |
| `docs/plan.md` | Plan, milestones, lots |
| `docs/contexte.md` | Everything an agent must know about the project |
| `docs/adr/` | Decision records, starting from `docs/adr/0000-modele.md` |
| `docs/design/` | Boards and pictures of Sherlock. Visual reference, do not modify it |
| `prototypes/` | Throwaway prototypes, outside the main workspace |

At the root: the workspace `Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`,
`.editorconfig`, `justfile`.

## Stability rules

1. The main branch is always green. Nobody pushes to it directly.
2. One lot, one branch, one pull request. About 600 changed lines at most, generated code excluded.
3. CI is mandatory: format, clippy without warnings, tests, image build, boot in QEMU.
4. Every lot ships its tests and its doc page.
5. No `unsafe` without a comment that justifies it. No `unwrap` outside tests.
6. Every new dependency is justified in the pull request, with a pinned version.
7. Every architecture decision goes through an ADR in `docs/adr/`.
8. An agent in doubt stops and asks its question in the pull request. It does not make things up.
9. Nothing that touches the kernel, the driver or Sherlock's consents is merged without your test
   on the machine.
10. Three agents in parallel at most, on different folders.
11. No AI signature. No `Co-Authored-By` line, no "Generated with" mention, no model or agent name,
    neither in commits nor in pull requests. The author is the git account of the repository.

## Branches and commits

- One branch per lot: `lot/<ID>`, for instance `lot/D-10`. Started from an up-to-date main branch.
- A single pull request per lot, with the report asked for by the brief.
- Commit message: `<ID>: <what changes>`, in the present tense, in English.
  Example: `D-10: add window moving`.

## Verification

Before opening a pull request, `just verify` must pass. The CI runs the same command on every pull
request and on the main branch (see `docs/ci.md`). The repository has no branch protection: never
merge a pull request whose CI is not green.

| Command | What it does |
| --- | --- |
| `just verify` | `fmt-check`, then `lint`, then `test` |
| `just fmt-check` | `cargo fmt --all -- --check` |
| `just fmt` | Applies the format |
| `just lint` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| `just test` | `cargo test --workspace --all-features` |
| `just build` | `cargo build --workspace` |
| `just check-language` | Reports French left in development text (ADR 0017) |

Build container: `just builder`, `just in-builder <command>`, `just test-builder`,
`just test-builder-repro` (see `docs/build.md`). Boot bench: `just test-boot [image]`,
`just witness`, `just test-bench` (see `docs/boot-tests.md`). The CI also runs the bench: `verify`
and `test-boot` must both be green.

Prerequisites of the machine: `rustup`, `just`, a C compiler used as linker (`gcc` and `libc6-dev`
on Debian) and the compositor libraries (`libxkbcommon-dev` and `libwayland-dev` on Debian). The
Rust version is read from `rust-toolchain.toml` and installs itself on the first `cargo`. Do not
change it: it is also copied into `rust-version` of the root `Cargo.toml`, and both change
together.

## Rust code

- Lints are defined once in `[workspace.lints]` of the root `Cargo.toml`. Every crate takes them
  with `[lints] workspace = true`. Never disable them to get the CI to pass.
- Refused: `unwrap`, `expect`, `println!`, `eprintln!`, `dbg!` outside tests; any `unsafe` block
  without a `// SAFETY:` comment; any public `unsafe` function without a `# Safety` section.
- Typed errors in libraries, errors with context in binaries. Logging with `tracing`.
- Identifiers, comments and documentation in English. No accents in file and folder names.
- No color, size or interface text hard-coded outside `desktop/sherlock-ui`.
- No name for the OS: write `NOM_OS` wherever one is needed.

## Decisions

The frozen decisions of the plan each have their ADR, from `0001` to `0013`. To propose a new one,
copy `docs/adr/0000-modele.md` under the next number, and write it in English. An ADR is applied
only once the author has approved it.
