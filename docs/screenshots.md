# Screenshots

Lot D-35c. In the real session, the compositor takes screenshots itself, with the Print Screen
key, whatever window has the keyboard.

| Keys | What is taken |
| --- | --- |
| Print Screen | The screen under the pointer, as it shows, without the cursor |
| Shift + Print Screen | An area drawn with the mouse: press the left button, drag, release. Escape or the right button gives up |
| Alt + Print Screen | The window that has the keyboard, as at rest: its title bar, border and round corners included, the corners transparent |

The PNG goes into the `Captures d'écran` folder of the pictures folder: `XDG_PICTURES_DIR` of
`~/.config/user-dirs.dirs`, or `~/Images` when it is not set. Its name carries the local date
and time, `Capture d'écran du 2026-11-26 à 18.40.12.png`, with ` (2)`, ` (3)`… when that second
already has one. The image then goes into the clipboard as `image/png`, and a notification
says « Capture d'écran enregistrée », with the file name.

While an area is chosen, the screen is dimmed (the scrim color at 60 %), the cursor is a cross,
and the rectangle drawn shows like the snap preview: the active entry color inside, a 2 px
border in the accent color. No client gets the pointer meanwhile. No board draws it.

Nested mode (`--nested`) leaves the key to the host session.

## How

| File | Role |
| --- | --- |
| `desktop/compositor/src/capture.rs` | The keys, the area, the drawing offscreen and the reading back, the worker that writes and notifies, the clipboard |
| `desktop/sherlock-ui/src/screenshot.rs` | The folder and file names, the notification texts, the look of the area |
| `desktop/system-services/src/notify.rs` | `org.freedesktop.Notifications.Notify` on the session bus |

- **Drawn by the compositor.** The backend gives a `Capturer`, as it gives the `Snapshotter`
  of the closing windows (lot D-34): it has the renderer. A screen is drawn with the scene of a
  frame (`Scene::screens`, with damage identities of its own), filled with the desktop color,
  into a texture of its size in pixels; a window with `window_anim::snapshot`. The texture is
  read back (`copy_framebuffer`), rows put from the top, alpha made straight. An area is the
  screen of its start, cut to the rectangle. This happens between two frames: the loop runs one
  source at a time.
- **Leaving the console gives up the area.** The real session forgets the keys and the area
  being chosen when the console is left (`forget_intercepted_keys`, which `pause` calls): back
  on it, the screen is not dimmed any more.
- **Never in the loop.** Encoding the PNG, writing it and sending the notification run on a
  thread of their own (`screenshots`). The PNG comes back to the loop, which makes it the
  selection of the seat. A client that pastes it gets it written by the loop, into its pipe made
  non-blocking, as the pipe takes it: no thread per paste, and at most 8 pastes under way
  (`MAX_PASTES`); past them a paste is refused, so that a client that asks without reading
  holds nothing more.
- **Clipboard.** The selection the compositor sets carries its bytes (`SelectionUserData`);
  the one it sets for XWayland carries none, and only that one is cleared when the X11 client
  lets it go. X11 applications do not see the screenshot in the clipboard yet.

## Tests

- `cargo test -p compositor capture`: the keys, the pictures folder read from `user-dirs.dirs`,
  the crop, a PNG written under a new name and read back, a paste larger than a pipe written as
  it is read and the bound on pastes, the area given up when the console is left.
- `cargo test -p compositor --test screenshots`: on Mesa's software renderer, Print Screen
  writes an 800 × 600 PNG of the desktop into a folder of the test's own, notifies a fake
  notification server on a private bus and puts the same bytes in the clipboard; Shift + Print
  Screen gives up with Escape, then takes a 200 × 150 area drawn with the pointer.

## Manual test

In the real session: each of the three keys, the file in `~/Images/Captures d'écran`, the
notification, a paste into an application that takes images (`wl-paste -t image/png > a.png`).
