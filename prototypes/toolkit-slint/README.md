# Prototype D-00: the bar in Slint

Throwaway prototype of lot D-00, twin of `prototypes/toolkit-iced`: the same 36 px bar anchored
at the top of the screen by layer-shell, the same clock, the same three workspaces, the same
« Fenêtre » menu of five entries with the « Déplacer vers le bureau » submenu, the same opening
animation.

Slint cannot create a layer-shell surface. The prototype therefore plugs Slint into Wayland
through its public `slint::platform` API:

- `ui/bar.slint` describes the bar, the menu and the submenu, with every design value in the
  `Theme` global;
- `src/wayland.rs` creates the layer-shell surface and the `xdg_popup`s with
  smithay-client-toolkit, forwards the pointer to Slint and copies Slint's software rendering
  into `wl_shm` buffers;
- `src/main.rs` creates the components and connects their callbacks to the Wayland loop.

Limits of this bridge, which a lasting shell would have to fill: scale fixed at 1 (no
`wp_fractional_scale`), no keyboard, no custom cursor, software rendering only, no
accessibility wired.

## Running

In a Wayland session whose compositor supports `zwlr_layer_shell_v1` (KDE Plasma, sway,
Hyprland, niri...):

```sh
cd prototypes/toolkit-slint
cargo run --release
```

Without a Wayland session, a KWin nested in a window is enough:

```sh
kwin_wayland --x11-display :0 --socket wayland-prototype --width 1440 --height 900 &
WAYLAND_DISPLAY=wayland-prototype cargo run --release
```

With `PROTOTYPE_AUTO=1`, the bar follows the same scenario as the iced prototype: 4 s idle, then
ten openings of the menu, then it quits.

## Measuring

The measurement script is shared with the iced prototype:

```sh
cargo build --release
python3 ../toolkit-iced/measure.py --runs 5 target/release/toolkit-slint
```

## Dependencies

| Crate | Version | Why |
| --- | --- | --- |
| `slint` | 1.19.1 | The toolkit under evaluation, without a backend: only the software renderer and the system fonts |
| `slint-build` | 1.19.1 | Compiles `ui/bar.slint` to Rust at build time |
| `smithay-client-toolkit` | 0.21.1 | Layer-shell surface, popups, pointer and shared buffers, which Slint does not do itself |
| `chrono` | 0.4.42 | Local time of the clock |
