# Prototype D-00: the bar in iced

Throwaway prototype of lot D-00. It reproduces the top bar of the board
`docs/design/planches/Main.dc.html`: 36 px high, anchored at the top of the screen by
layer-shell, with a clock, three workspaces of which one is active, and a « Fenêtre » button
that opens a menu of five entries. The « Déplacer vers le bureau » entry opens a submenu. The
menu and the submenu are real nested `xdg_popup`s, attached to the surface of the bar.

The twin prototype in Slint is in `prototypes/toolkit-slint`. The comparison and the decision
are in `docs/adr/0014-toolkit-du-shell.md` and its appendix of measurements (they cite the
former folder name, « essais/ »).

## Running

In a Wayland session whose compositor supports `zwlr_layer_shell_v1` (KDE Plasma, sway,
Hyprland, niri...):

```sh
cd prototypes/toolkit-iced
cargo run --release
```

On a laptop with two graphics cards like the reference machine, iced picks the most powerful
card by default, hence the NVIDIA one. To stay on the AMD chip:

```sh
# Software rendering, without a graphics card
ICED_BACKEND=tiny-skia cargo run --release
# Or wgpu rendering limited to the AMD Vulkan driver
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json WGPU_POWER_PREF=low cargo run --release
```

Without a Wayland session, a KWin nested in a window is enough:

```sh
kwin_wayland --x11-display :0 --socket wayland-prototype --width 1440 --height 900 &
WAYLAND_DISPLAY=wayland-prototype ICED_BACKEND=tiny-skia cargo run --release
```

## Automatic mode

With `PROTOTYPE_AUTO=1`, the bar stays idle for 4 s, then opens the menu ten times: the submenu
opens 750 ms after the menu, everything closes at 1.25 s, and the program quits at the end.
This mode is used for measurements.

## Measuring

`measure.py` measures a binary from the outside, from the `WAYLAND_DEBUG` log: time to the first
frame, CPU and memory when idle, frame rate of the menu opening animation. The same script
measures the Slint prototype.

```sh
cargo build --release
python3 measure.py --runs 5 target/release/toolkit-iced
```

## Dependencies

| Crate | Version | Why |
| --- | --- | --- |
| `iced` | 0.14.0 | The toolkit under evaluation. Features: wgpu and tiny-skia rendering, Wayland, "web" colors to blend transparencies like the HTML board |
| `iced_layershell` | 0.19.1 | Layer-shell surface and popups for iced. Last version under this name, built on iced 0.14 |
| `chrono` | 0.4.42 | Local time of the clock |

`Cargo.lock` pins `winit-core` and `winit-common` at 0.31.0-beta.2: `iced_exdevtools` 0.19.1 does
not compile with beta.4.
