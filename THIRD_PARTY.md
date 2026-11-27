# Third-party works

The code of SherlockOS is under GPL-3.0-or-later (`LICENSE`, `NOTICE`). The name, the logo and
the mascot are reserved (`TRADEMARKS.md`). This page lists what comes from elsewhere: the works
stored in this repository, the Rust crates built into the programs, and the programs that
SherlockOS starts without shipping them. Each keeps its own license.

## Stored in this repository

| Work | Files | License | Notes |
| --- | --- | --- | --- |
| Inter, by The Inter Project Authors | `desktop/sherlock-ui/fonts/InterVariable.ttf` | SIL Open Font License 1.1 | Text: `desktop/sherlock-ui/fonts/Inter-OFL.txt`. Built into the programs with `include_bytes!` |
| JetBrains Mono, by The JetBrains Mono Project Authors | `desktop/sherlock-ui/fonts/JetBrainsMono-Regular.ttf` | SIL Open Font License 1.1 | Text: `desktop/sherlock-ui/fonts/JetBrainsMono-OFL.txt`. Built in |
| Space Grotesk, by The Space Grotesk Project Authors | `desktop/sherlock-ui/fonts/SpaceGrotesk-Variable.ttf` | SIL Open Font License 1.1 | Text: `desktop/sherlock-ui/fonts/SpaceGrotesk-OFL.txt`. Built in |

The OFL allows bundling the fonts with GPL software, as long as the fonts are not sold on their
own and keep their license text, which sits beside each of them.

Everything else was made for this project:

| Work | Files | Status |
| --- | --- | --- |
| Application icons (Evidence, Control Room, Inquiry, Lab, Games, Installer) | `desktop/sherlock-ui/icons/` | Drawn for SherlockOS, under the license of the code, except the Sherlock icon (reserved, `TRADEMARKS.md`) |
| Cursors | `assets/cursors/` | Drawn for SherlockOS after the project's own boards, under the license of the code |
| Test icons and desktop entries | `desktop/*/fixtures/` | Made up for the tests, under the license of the code |
| Logo, mascot pictures, wallpapers, lock pictures | see `TRADEMARKS.md` | Reserved, not under the license of the code |
| Design boards | `docs/design/` | The project's visual reference. The pictures of the mascot in them are reserved |
| Shaders | `desktop/compositor/src/shaders/`, `desktop/sherlock-3d/src/*.wgsl` | Written for SherlockOS, under the license of the code |

Not stored here: Sherlock's 3D model (`sherlock.glb`). `just mascot` downloads it from the
repository `dodolereporter/sherlock` at a pinned revision and checks its SHA-256 (ADR 0024). It
is reserved like the rest of the mascot.

## Rust crates

The programs are built from about 480 crates of crates.io, pinned in `Cargo.lock`. All of them
are under permissive licenses compatible with GPL-3.0-or-later:

| License (as declared) | Crates |
| --- | --- |
| MIT, Apache-2.0, or either of them | almost all |
| Zlib, BSD-2-Clause, BSD-3-Clause, ISC, BSL-1.0, CC0-1.0, Unlicense, 0BSD, Unicode-3.0 (alone or as an option) | a few dozen |
| Apache-2.0 OR GPL-2.0-only | `self_cell`: used under Apache-2.0 |
| MIT OR Apache-2.0 OR LGPL-2.1-or-later | `r-efi`: used under MIT or Apache-2.0 |

The main ones:

| Crate | Role | License |
| --- | --- | --- |
| `smithay` 0.7 | Wayland compositor library | MIT |
| `iced` 0.14, `iced_exwlshell` | Toolkit of the shell and the applications, layer-shell | MIT |
| `wgpu`, `naga` | GPU rendering | MIT OR Apache-2.0 |
| `tiny-skia` | Software rendering | BSD-3-Clause |
| `cosmic-text`, `rustybuzz`, `swash`, `fontdb` | Text layout and font rasterization | MIT, or MIT OR Apache-2.0 |
| `resvg`, `usvg`, `image`, `png`, `zune-jpeg` | Pictures and icons | MIT OR Apache-2.0 (and Zlib) |
| `gltf`, `glam` | Sherlock's 3D model | MIT OR Apache-2.0 |
| `zbus` | D-Bus | MIT |
| `x11rb` | XWayland window manager, X11 clipboard | MIT OR Apache-2.0 |
| `calloop`, `drm`, `gbm`, `input`, `libseat`, `xkbcommon`, `wayland-protocols` | Real session: event loop, KMS, input, seats, keymaps | MIT |
| `tokio`, `tracing`, `serde`, `serde_json` | Runtime, logs, data | MIT, or MIT OR Apache-2.0 |

Apache-2.0 is compatible with version 3 of the GPL (not with version 2), which is one more
reason the project is under GPL-3.0-or-later. To list every crate with its license:

```sh
cargo metadata --format-version 1 --locked \
  | python3 -c 'import json,sys; [print(p["name"], p["version"], p["license"]) for p in json.load(sys.stdin)["packages"] if p["source"]]'
```

Every new dependency is justified in its pull request (`CLAUDE.md`, rule 6), license included.

## System libraries and programs

Linked dynamically or started as separate programs, never copied into this repository:

| Component | Role | License |
| --- | --- | --- |
| libwayland, libxkbcommon, libinput, libseat, libdrm, Mesa (EGL, GBM, Vulkan) | Display, input, GPU | MIT |
| libudev (systemd) | Devices | LGPL-2.1-or-later |
| Linux-PAM | Lock screen authentication | BSD-3-Clause or GPL (dual) |
| Xwayland | X11 applications | MIT |
| greetd (lots D-24, in review) | Login screen daemon, spoken to over its socket | GPL-3.0 |
| gnome-keyring, polkit agents, xdg-desktop-portal | Session services, when installed | Their own free licenses |
| Claude Code | The model behind Sherlock: started by `sherlockd` with the user's own installation and subscription | Proprietary, Anthropic's terms. Never shipped nor modified by SherlockOS |

The packages of the future system image (`distro/`) are built from upstream sources and keep
their licenses.

## Prototypes

`prototypes/` holds throwaway programs outside the main workspace. `prototypes/toolkit-slint`
depends on Slint, which offers the GPL-3.0 among its licenses: compatible, and not part of any
SherlockOS program.
