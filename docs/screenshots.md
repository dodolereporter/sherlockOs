# Screenshots

Lots D-35c and D-35e. The compositor takes screenshots itself, whatever window has the keyboard.

| Keys | What is taken |
| --- | --- |
| Print Screen | The screen under the pointer, as it shows, without the cursor |
| Super + Shift + S, or Shift + Print Screen | Opens the screenshot tool (below) |
| Alt + Print Screen | The window that has the keyboard, as at rest: its title bar, border and round corners included, the corners transparent |

Print Screen is taken in the real session only: nested (`--nested`), the host session keeps it,
and the tool opens with Alt + Shift + S, the nested gesture key.

## The screenshot tool (lot D-35e)

A toolbar of glass at the bottom of the screen under the pointer, above the dock, rises and
fades in; it goes the other way on Escape, the right button or « Annuler ».

| Group | Buttons |
| --- | --- |
| What | « Zone », « Fenêtre », « Écran entier » (Tab goes to the next one) |
| « Minuteur » | « Non », « 3 s », « 10 s » |
| « Options » | A menu above it: « Copier seulement » or « Enregistrer et copier »; « Enregistrer dans » « Captures d'écran », « Bureau » or « Documents » (disabled when the image is only copied); « Montrer le curseur » |
| | « Annuler », « Capturer » (disabled until there is something to take) |

- **« Zone ».** The cursor is a cross. The screen is dimmed around the area (the scrim color at 60 %).
  Press and drag draws it, with its size in pixels of the screen beside the pointer; once let go
  it keeps eight handles to adjust it, and its inside moves it. Space held while drawing moves
  the area being drawn. Dragged past the edge of the screen, it stretches less and less, as a
  rubber band, and springs back inside once let go. Enter or « Capturer » takes it.
- **« Fenêtre ».** The window under the pointer is outlined in the accent color; a click takes it.
- **« Écran entier ».** The screen is outlined; a click or Enter takes the screen under the pointer.
- **Timer.** With « 3 s » or « 10 s », the tool goes and a countdown shows in the middle of the
  screen; the image is taken at its end, without the countdown.
- The image always goes to the clipboard. « Copier seulement » writes no file and notifies
  « Capture d'écran copiée ». The choices stay while the session lasts.
- Hover and press of a button come in the time of a hover; the chosen button of a group has
  the soft accent; a disabled one has the disabled color and takes no click.
- Every measure follows the screen: the toolbar keeps its size from 1920 logical pixels up (a
  little larger on a wider screen, at most 1.25 times), and shrinks to fit a narrower one.

The PNG goes into the `Captures d'écran` folder of the pictures folder: `XDG_PICTURES_DIR` of
`~/.config/user-dirs.dirs`, or `~/Images` when it is not set. Its name carries the local date
and time, `Capture d'écran du 2026-11-26 à 18.40.12.png`, with ` (2)`, ` (3)`… when that second
already has one. The image then goes into the clipboard as `image/png`, and a notification
says « Capture d'écran enregistrée », with the file name.

No client gets the pointer or the keyboard while the tool is open. No board draws it: the
measures of `sherlock_ui::screenshot_tool` are choices of the lot, to review with the design.

## How

| File | Role |
| --- | --- |
| `desktop/compositor/src/capture.rs` | The keys, the area, the drawing offscreen and the reading back, the worker that writes and notifies, the clipboard |
| `desktop/compositor/src/screenshot_tool.rs` | The tool: its keys, pointer and drawing, the timer |
| `desktop/sherlock-ui/src/screenshot.rs` | The folder and file names, the notification texts |
| `desktop/sherlock-ui/src/screenshot_tool.rs` | The tool's choices, texts, measures and geometry: toolbar and menu layout, handles, rubber band, thumbnail place |
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
  Screen gives up with Escape, then takes a 200 × 150 area drawn with the pointer, on Enter.
- `cargo test -p sherlock-ui --no-default-features --lib screenshot_tool`: the toolbar laid out
  from 640 × 480 to 3840 × 2160, its buttons, the menu and its disabled folders, the handles,
  the turn over of an area, the rubber band, the thumbnail's place.

## Manual test

In the real session: each of the three keys, the file in `~/Images/Captures d'écran`, the
notification, a paste into an application that takes images (`wl-paste -t image/png > a.png`).
Super + Shift + S: each mode, the handles, Space while drawing, a drag past the edge, the timer,
« Copier seulement » and the other folders.
