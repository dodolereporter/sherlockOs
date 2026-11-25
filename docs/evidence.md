# Evidence

Evidence is the file manager of the desktop (lot D-27). It browses folders, copies, moves,
renames, sends to the trash and restores, previews files, shows the git state of each file,
and lists disks, USB keys and cards. The planche `docs/design/planches/Fichiers.dc.html` is the
reference for its look. Its interface texts are French and live in `sherlock_ui::texts::evidence`.

```sh
cargo run --release -p evidence            # opens the home folder
cargo run --release -p evidence -- ~/Projets   # opens a folder
```

Use `--release`: in a debug build, iced and tiny-skia are not optimised and the software
renderer is several times slower (see Performance).

## Architecture

| Module | Role | iced |
| --- | --- | --- |
| `listing` | Reads a folder, kind of each entry, labels of each row (date, size, kind), sort by column | no |
| `ops` | Copy, move, rename, duplicate, new folder. Never overwrites; typed errors with French messages | no |
| `trash` | freedesktop Trash 1.0 in `$XDG_DATA_HOME/Trash`: put, list, restore, empty | no |
| `git` | Runs `git status --porcelain=v2 -z --branch --ignored` and reads it | no |
| `preview` | First lines of a text file with light highlighting, image, or information | no |
| `devices` | Reads UDisks2 objects, chooses and names volumes, `DeviceSource` trait | no |
| `udisks` | The real `DeviceSource`: UDisks2 on the system bus through zbus, and its watcher | no |
| `places` | Favourites from `user-dirs.dirs` | no |
| `clock` | Local time (`localtime_r`) | no |
| `probe` | `--measure`: smoothness measurement | no |
| `app` | State, messages, keyboard, commands, animations | yes |
| `view` | The window, after the planche | yes |
| `scenarios` | End-to-end tests driven by the keyboard, headless capture | tests |

Nothing slow runs in `update` or `view`. Reading a folder (with one `stat` per entry and the
item count of each subfolder), the git state, the preview, the trash listing, copies, moves,
emptying the trash and every device action are tasks on iced's thread pool. A folder read
carries a generation number, so a slow read overtaken by a newer navigation is dropped.

The list is virtualised: only the visible rows and four more above and below are in the
widget tree, between two spacers. Row labels are computed once when the folder is read, icons
are built once (`sherlock_ui::component::icon` caches the SVG handles).

## What it does

- Sidebar: Favourites (home, Desktop, Documents, Downloads, Pictures, from xdg-user-dirs),
  Devices, System (system disk, trash). The favourite that contains the current folder is
  active; Home only for the home folder itself.
- Toolbar: back, forward, breadcrumb (folded with "…" beyond five segments), git branch,
  search in the current folder.
- List: name with an icon per kind, modified date, size (item count for folders), kind, git
  state (modified, new, ignored). Click a column header to sort, again to reverse. Folders
  stay first. Hidden files with Ctrl+H.
- Preview: header with tile, size and git state; first lines of a text file in JetBrains Mono
  with keywords and comments coloured (Dockerfile, Rust, TOML, shell, Python); PNG and JPEG
  images; date, kind and place for the rest. Buttons: Open, Copy path (Restore in the trash).
- Context menu as on the planche, with the "Open with" submenu: default application, terminal
  in this folder. On the background: new folder, paste, hidden files. In the trash: restore,
  empty. On a device: open or mount, unmount, eject.
- Trash: deleting moves to the freedesktop trash (shared with Plasma, GNOME, `gio trash`).
  The trash view lists the original name, deletion date and origin. Emptying asks first.
- Operations: copy, cut, paste, duplicate, rename in place, new folder (named right away).
  Pasting a copy over a taken name makes "name (copie).ext"; a move or rename onto a taken
  name is refused with a message. Copying a folder into itself is refused.
- Devices: internal partitions mounted by the user, USB keys, external disks, memory cards,
  optical discs, from UDisks2. Name: volume label, else UDisks' name hint, else model and size,
  else "Volume de 32 Go". Icon by kind. Free space when mounted. A click mounts (through
  `Filesystem.Mount`, so polkit asks when needed) then opens. Unmount and eject in the menu,
  eject button beside removable media. A medium plugged in or pulled out shows up or goes away
  at once (UDisks2 signals watched on a thread). Hidden: what UDisks marks `HintIgnore`, what it
  marks `HintSystem` unless mounted outside system places, anything mounted at `/`, `/boot`,
  `/efi`, `/usr`, `/var` or `/home`, and blocks without a drive (loop devices, zram). LUKS
  volumes show locked.

## Keyboard

Everything works from the keyboard. Arrow keys and shortcuts act on the list unless a text
field has the focus.

| Keys | Action |
| --- | --- |
| Up, Down, Home, End | Move the selection |
| Enter | Open (folder: enter it; file: `xdg-open`) |
| Backspace, Alt+Up | Parent folder, with the folder you came from selected |
| Alt+Left, Alt+Right | Back, forward |
| Ctrl+C, Ctrl+X, Ctrl+V | Copy, cut, paste |
| Ctrl+D | Duplicate |
| Ctrl+Shift+C | Copy the path |
| F2 | Rename (Enter confirms; Escape leaves the field, a second Escape cancels) |
| Ctrl+Shift+N | New folder |
| Delete | Move to the trash |
| Ctrl+H | Show or hide hidden files |
| Ctrl+A | Select all |
| Ctrl+F | Search |
| F5, Ctrl+R | Reload |
| Space | Show or hide the preview |
| Menu key, Shift+F10 | Context menu; arrows, Right for the submenu, Enter, Escape |
| Tab, Shift+Tab | Move the focus through buttons and fields |
| Escape | Clear the search and the selection |

Ctrl+click adds a row to the selection.

## Animations

Built on `sherlock_ui::motion`. The application subscribes to window frames only while one of
its animations runs (or during `--measure`), never at rest.

| What | How |
| --- | --- |
| Hover of rows, menu entries, sidebar entries, icon and secondary buttons | 150 ms fade of the hover veil (`sherlock_ui::animated::Hoverable`, which requests its own frames) |
| Selection | The highlight glides from row to row on `PREVIEW_SPRING` |
| Opening a folder | The rows rise 8 px and fade in over 150 ms |
| Context menu | Opens from its corner, scale 0.92 to 1 on `OPEN_SPRING` |
| Preview | Fades in over 150 ms when its content arrives |

iced 0.14 has no group opacity. A fade is a veil of the background colour laid over the
content and fading out. The context menu cannot use that trick (it floats over the list), so
it only scales; it does not fade. Nothing blinks.

## Performance

`evidence --measure <folder> [--maximized]` opens the folder, scrolls the list continuously
for 3 s, then moves the selection one row every 9 frames for 3 s, and logs each phase
(`RUST_LOG=evidence=info`). Frames are counted from iced's frame events: an interval longer
than 16.7 ms is a frame that misses 60 per second. Reference laptop, Plasma X11, 144 Hz
screen, release build, a folder of 5,000 files.

| Step | Open | Scroll | Selection animation |
| --- | --- | --- | --- |
| Before (first version) | 258 ms | 6.3 fps, p95 170 ms | 24 fps, p95 47 ms |
| Background reading, virtual list, cached labels | 46 ms | 12 fps, p95 96 ms | 108 fps, p95 49 ms |
| No clipped container per row | 49 ms | 12 fps | 110 fps |
| Window body drawn as the renderer's clear colour | 49 ms | 95 fps, p95 11 ms | 267 fps, p95 10 ms |
| Evidence, 1300 × 676 | 61 ms | 101 fps, p95 10.5 ms, 5 late frames | 285 fps, p95 9.9 ms, none late |
| Evidence, maximised (1920 × 1080 screen) | 49 ms | 50 fps, p95 21.8 ms, 148 late of 152 | 159 fps, p95 23 ms, 32 late |

What the profile showed (stack samples with gdb during the scroll): almost all the time is in
tiny-skia. iced's software renderer redraws, for every damaged rectangle of a frame and every
layer, every quad of that layer, through a clip mask as large as the window. A quad as large
as the window (the window body) was therefore filled again many times per frame; it is now
the clear colour of the renderer (`sherlock_ui::theme::window_theme`). Each clipped
container or extra stack layer also costs a full-window mask: rows do not clip (long names
are cut with "…" from an estimate of the width), and the code block does not clip (lines are
cut to the panel width).

Limit of the engine: in a maximised window, scrolling stays around 50 frames per second
(about 20 ms per frame), below the 60 required, with the software renderer imposed by
ADR 0014. The remaining cost is tiny-skia's: the sidebar and preview backgrounds and the title
bar are still quads, redrawn for every damaged rectangle. This lot does not change the engine.
The question of a GPU renderer (wgpu on the AMD chip) is for the author, as a revision of
ADR 0014.

`cargo test --release -p evidence frame_cost -- --ignored` writes the headless cost of a frame
(view, layout, render) to `target/tmp/frame_cost.txt`.

## Comparing with the planche

1. Render the planche once: replace `/_blob/5a8a…` with `docs/design/images/sherlock-tete.png`,
   drop the `support.js` line, then
   `chromium --headless --disable-gpu --hide-scrollbars --window-size=1440,900 --screenshot=planche.png Fichiers.html`.
   Crop the window: `(70, 64)` to `(1370, 740)`, 1300 × 676.
2. `cargo test -p evidence window_renders_headless` (see `scenarios.rs`) writes
   `target/tmp/evidence-tiny-skia.png`, at scale 2: the same folder as the planche (a git repo
   with a modified `src`, a new `Dockerfile`, an ignored `target`), Dockerfile selected, menu
   and submenu open where the planche has them. Scale it down to 1300 × 676.
3. Put the two side by side. The pair of this lot is in `docs/evidence/`.

On the author's laptop: `cargo run --release -p evidence -- <a git project>`, then compare
with the planche at 1440 × 900 (the window is 1300 × 676, as on the planche).

## Gaps with the planche

- No grid or column views: list only (the lot's size went to smoothness and devices).
- No "Ask Sherlock" button, Sherlock bubble, "Build the image" or "Run in a sandbox"
  buttons, no Dev section (Docker volumes, sandboxes): out of this lot's scope.
- "Open with" offers the default application and a terminal in the folder; "Other
  application" is shown disabled. The planche's "Editor" and "Sandbox" entries are not there.
- The open submenu entry uses the selection token at 0.26 (the planche has 0.30; see
  `docs/sherlock-ui.md`).
- A selected row's git badge sits on the darkest background: the planche's veil would fall
  under 4.5:1 contrast on the selection.
- The search field has no clear button. Column header letter spacing is not supported by iced.
- The sort arrow next to the sorted column is an addition.
- The planche has no Devices section: its entries use the sidebar's style, with an eject
  button in the style of sherlock-ui's icon buttons.
- The context menu scales in but cannot fade (see Animations).

## Limits

- No folder watching: the list reloads after each operation, with F5 or Ctrl+R.
- The clipboard is internal: copy and cut between Evidence and another application are not
  shared (the path is copied to the system clipboard with Ctrl+Shift+C).
- Copy progress is a message in the status bar, without a percentage; a copy cannot be
  cancelled.
- The trash handles the home trash only: a file on another disk is refused with a message.
- Encrypted volumes show locked; unlocking them is not done yet.
- Internal partitions that UDisks marks `HintSystem` show only once mounted elsewhere than the
  system's places. On the reference laptop (EFI, root, swap), the Devices section is empty until
  a key or a card is plugged in.
- Long names are cut from an estimated character width, not measured text.
- Image preview decodes PNG and JPEG only, on the interface thread at draw time.
- Accessibility: iced 0.14 exposes nothing to screen readers (ADR 0014).

## Tests

`just verifier` runs them without a screen, on temporary folders. The trash used by the tests is
in the temporary folder, never the user's; no test calls the real UDisks.

- `ops`: copy (with links), duplicate and name conflicts, folder into itself, move and refused
  move, rename and invalid names, new folder.
- `trash`: put and restore (folder recreated), two items with the same name, refused restore,
  empty, path encoding.
- `git`: porcelain v2 parsing (renames, spaces, root prefix, whole ignored folder) and a real
  repository.
- `listing`: sort by each column, natural order, kinds.
- `preview`, `places`, `clock`, `devices` (filtering, naming, kinds, free space and errors
  from UDisks2 objects recorded with `busctl --json` on the reference laptop, plus a USB key,
  an SD card and a LUKS disk in the same format).
- `scenarios`: keyboard navigation, copy/cut/paste and conflicts, rename and new folder, trash
  with restore and confirmed emptying, context menu from the keyboard, devices through a fake
  source (mount and open, menu, eject, locked, busy), and a headless capture of the window.

## Manual procedure

On the author's laptop, Plasma X11:

1. `cargo run --release -p evidence -- <a git project>`. Check the list, the git badges, the
   branch, the preview of a text file and of an image.
2. Hover rows and buttons (150 ms fade), move with the arrows (the highlight glides), open a
   folder (rows rise and fade in), right-click (the menu scales in), hover "Ouvrir avec".
3. In a scratch folder: copy, cut, paste, Ctrl+D, F2, Ctrl+Shift+N, Delete, then the trash:
   restore, then empty (the question appears; Escape cancels).
4. Plug in a USB key: it appears under Appareils. Click it: Plasma's polkit agent may ask for
   the password, then the key opens. Right-click: Démonter, Éjecter. Click the eject button:
   the key disappears from the list and the status bar says it can be removed. With a file of
   the key open in another program, Démonter shows the "busy" message.
5. `RUST_LOG=evidence=info cargo run --release -p evidence -- --measure <folder of 5,000
   files> --maximized` and read the three `measure:` lines.
