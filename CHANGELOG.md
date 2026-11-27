# Changelog

SherlockOS has no release yet. This page sums up each milestone; the detail is in the merged
pull requests and in [ROADMAP.md](ROADMAP.md).

## Unreleased: J3, real session (in progress)

- Real session from a display manager: DRM, libseat, the session's own D-Bus.
- Secure lock screen on `ext-session-lock-v1` and PAM, with Sherlock in 3D and a rescue mode.
- Control Room, the settings: appearance, touchpad, sound, lock, default applications.
- Applications view, launcher and applications view kept resident.
- Global menu in the bar; media keys with an elastic on-screen display; keyboard backlight.
- Dock: several windows per application, fullscreen windows cover the bar and the dock.
- Clipboard between Wayland and X11; XWayland on by default, polkit agent, keyring, file portal.
- `sherlock-3d`: the mascot rendered from its glTF model.
- In review: login screen on greetd, sleep and wake, blur behind glass, screen capture for
  sharing, notification area, screenshot v2.

## J2: shell (reached)

- Bar with control centre, calendar, notification centre, Wi-Fi and Bluetooth.
- Dock with magnification, bounce, pinning by drag, smart hiding.
- Launcher with fuzzy search, notifications daemon.
- Evidence, the file manager; Mission Control; touchpad gestures; genie effect; window
  animations.
- Design system `sherlock-ui`; dark and light themes switched live; icons, wallpapers and cursor
  theme; GTK and Qt themes that follow the desktop.
- The name SherlockOS.

## J1: nested compositor (reached)

- Wayland compositor on Smithay: windows, keyboard, mouse, moving and resizing, snapping.
- Rounded corners, shadows, spring animations; layer-shell, several screens, fractional scaling.
- XWayland; GPU rendering of the shell; linux-dmabuf.

## J0: foundations (reached)

- One repository, Rust CI (format, clippy, tests, language check) and boot CI in QEMU.
- Build container of the distribution and QEMU boot bench.
- iced chosen as the toolkit (ADR 0014); all development in English (ADR 0017).
- `sherlockd`, the agent's daemon, driving Claude Code with the user's subscription.
