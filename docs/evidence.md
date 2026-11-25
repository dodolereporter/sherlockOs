# Evidence

Evidence is the file manager of the desktop (lot D-27). It browses folders, copies, moves,
renames, sends to the trash and restores, previews files, shows the git state of each file,
and lists disks, USB keys and cards. The design board `docs/design/planches/Fichiers.dc.html` is the
reference for its look. Its interface texts are French and live in `sherlock_ui::texts::evidence`.

```sh
cargo run --release -p evidence            # opens the home folder
cargo run --release -p evidence -- ~/Projets   # opens a folder
```

Use `--release`: in a debug build, iced and tiny-skia are not optimised and the software
renderer is several times slower (see Performance). Evidence logs the renderer it chose at start
(ADR 0018); `ICED_BACKEND=tiny-skia` forces software rendering.

## Architecture

| Module | Role | iced |
| --- | --- | --- |
| `listing` | Reads a folder, kind of each entry (folder, file, link, special), labels of each row (date, size, kind), sort by column | no |
| `ops` | Copy, move, rename, duplicate, new folder, on raw (`OsStr`) names. Never overwrites, even in a race; typed errors with French messages | no |
| `trash` | freedesktop Trash 1.0 in `$XDG_DATA_HOME/Trash`: put, list, restore, remove | no |
| `git` | Runs `git status --porcelain=v2` with a locked-down configuration and reads it | no |
| `preview` | First lines of a text file with light highlighting, decoded image, or information | no |
| `devices` | Reads UDisks2 objects, chooses and names volumes, `DeviceSource` trait | no |
| `udisks` | The real `DeviceSource`: UDisks2 on the system bus through zbus, and its watcher | no |
| `icons` | Application icons (hicolor, pixmaps, absolute paths), decoded off the interface thread | no |
| `places` | Favourites from `user-dirs.dirs` | no |
| `clock` | Local time (`localtime_r`, `mktime`) | no |
| `probe` | `--measure`: smoothness measurement | no |
| `app` | State, messages, keyboard, commands, animations | yes |
| `view` | The window, after the design board | yes |
| `scenarios` | End-to-end tests driven by the keyboard, headless capture | tests |

Installed applications, file types and default applications come from the workspace crate
`mime-apps` (`docs/mime-apps.md`), which Control Room will reuse.

No file system access runs in `update` or `view`. Reading a folder (with one `stat` per
entry and the item count of each subfolder), checking which favourites exist, the git
state, the preview (text read and image decoding), the trash listing, copies, moves,
renames, new folders, restoring, emptying the trash, launching programs, reading the
applications and their icons, writing `mimeapps.list` and every device action are tasks on
iced's thread pool. A folder read carries a generation number, so a slow read overtaken by a
newer navigation is dropped. `view` only reads what those tasks cached.

The list is virtualised: only the visible rows and four more above and below are in the
widget tree, between two spacers. Row labels are computed once when the folder is read, icons
are built once (`sherlock_ui::component::icon` caches the SVG handles).

## Safety

- **Special files.** A named pipe, a socket or a device is listed as « Fichier spécial » and
  never opened: the preview opens regular files only, checks the type again on the open
  descriptor, opens with `O_NONBLOCK`, and gives up after 2 s on its own thread. A copy
  recreates named pipes (`mkfifo`) without reading them, and leaves sockets and devices aside
  with a message.
- **git.** Listing a folder must not run code, even from a repository found on a USB key or
  in an archive. Evidence runs git with `GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`,
  none of the caller's `GIT_*` variables, `--no-optional-locks`, and forced settings that win
  over the repository's `.git/config` and the files it includes: `core.fsmonitor=false`,
  `core.untrackedCache=false`, `core.hooksPath=/dev/null`, no pager, no ask-pass, no external
  diff. It first reads (without running anything) the filter drivers the repository declares,
  then empties each one (`filter.<name>.clean=`, `.smudge=`, `.process=`, `.required=false`),
  which git treats as no filter. Submodules, each with its own configuration, are not entered
  (`--ignore-submodules=all`). A test builds a trapped repository (fsmonitor, clean and process
  filters, an included file) and checks that no command ran while the state is still read.
  Asking the user to trust each repository was the alternative; it adds a question for every
  project folder, for no more safety once the configuration is locked down.
- **No overwrite, even in a race.** Copies create files with `create_new`; renames and moves
  use `renameat2(RENAME_NOREPLACE)` (a plain check then rename on filesystems without it).
- **Failures clean up.** A copy, or a move between disks, that fails removes what it created
  at the destination; the source is untouched. Modification times are kept from one disk to
  another.
- **Names are bytes.** Copy, move and conflict names keep the exact bytes of a name that is not
  UTF-8; only the display is lossy.

## What it does

- Sidebar: Favourites (home, Desktop, Documents, Downloads, Projets when `~/Projets` exists,
  Pictures, from xdg-user-dirs), Devices, System (system disk, trash). The favourite that
  contains the current folder is active; Home only for the home folder itself.
- Toolbar: back, forward, breadcrumb (folded with "…" beyond five segments), git branch,
  search in the current folder.
- List: name with an icon per kind, modified date, size (item count for folders), kind, git
  state (modified, new, ignored). Click a column header to sort, again to reverse. Folders
  stay first. Hidden files with Ctrl+H.
- Preview: header with tile, size and git state; first lines of a text file in JetBrains Mono
  with keywords and comments coloured (Dockerfile, Rust, TOML, shell, Python); PNG and JPEG
  images; date, kind and place for the rest. Buttons: Open, Copy path (Restore in the trash).
- Context menu as on the design board. « Ouvrir avec » lists the installed applications that open the
  file's type (from their `.desktop` files, with their icon, the default first and checked),
  then « Terminal, dans ce dossier », « Autre application… » (a chooser of every application, with
  a search), and the switch "Toujours ouvrir ce type ainsi". With the switch on, the chosen
  application becomes the default in `~/.config/mimeapps.list` (and in `kde-mimeapps.list`
  when that file already names a default for the type); opening a file then uses the default
  application of its type (xdg-open when there is none). The switch applies to one choice
  only: it goes back off after that choice, and when the menu or the chooser closes, so an
  "open once with" never changes the defaults shared with the desktop. `Terminal=true`
  applications open in the terminal (see `docs/mime-apps.md`). On the background: new folder,
  paste, hidden files. In the trash: restore, empty. On a device: open or mount, unmount, eject.
- Trash: deleting moves to the freedesktop trash (shared with Plasma, GNOME, `gio trash`).
  The trash view lists the original name, the deletion date read from the record, and the
  origin. Emptying asks first and deletes only what was listed when the question was asked.
  Long names are cut in the record's name, never refused; no empty record is left behind.
- Operations: copy, cut, paste, duplicate, rename in place, new folder (named right away).
  Pasting a copy over a taken name makes "name (copie).ext"; a move or rename onto a taken
  name is refused with a message. Copying a folder into itself is refused.
- Devices: internal data partitions (mounted or not, even when UDisks marks them
  `HintSystem`), USB keys, external disks, memory cards, optical discs, from UDisks2. Name:
  volume label, else UDisks' name hint, else model and size, else "Volume de 32 Go". Icon by
  kind. Free space when mounted. A click mounts (through `Filesystem.Mount`, so polkit asks
  when needed) then opens. Unmount and eject in the menu, eject button beside removable media.
  A medium plugged in or pulled out shows up or goes away at once (UDisks2 signals watched on a
  thread). Hidden: what UDisks marks `HintIgnore`; anything mounted at `/`, `/boot`, `/efi`,
  `/usr`, `/var` or `/home`; unmounted EFI, extended boot, BIOS boot, Microsoft reserved,
  Windows recovery and swap partitions (by partition type) and blocks without a mountable
  filesystem; blocks without a drive (loop devices, zram). LUKS volumes show locked. Eject is
  refused when no drive can be found for the volume.

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
| Context menu | Appears at once, with no animation (see below) |
| Preview | Fades in over 150 ms when its content arrives |

iced 0.14 has no group opacity. A fade is a veil of the background colour laid over the
content and fading out. The context menu cannot use that trick (it floats over the list).

It used to open from its corner on a spring (`Scaled`, a transformation). Review 3 showed it
broken in a real window: a black rounded block with a greenish halo hid every entry but the
first, and cut the submenu. The cause is iced 0.14's software renderer: a quad's blurred
shadow is drawn as a separate image, computed in screen pixels and painted without the
current transformation and without the clip mask of the area being redrawn. Under the scale
transformation, the menu's shadow (`0 22px 54px`) came out as a large opaque block drawn over
the menu. Checked in an isolated Xephyr: with the scale removed and the blurred shadow kept,
the menu draws correctly. Two changes, for safety beyond this case:

- the menu has no scale animation any more (and `Scaled` is gone from sherlock-ui);
- menus and floating panels no longer use iced shadows at all: `sherlock_ui::surface::Shadowed`
  draws a crisp, light shadow as a few plain offset rounded rectangles (ordinary quads,
  clipped and transformed like everything else), close to the design board's.
  Since D-02 this crisp shadow is only the software one: on the GPU, `Shadowed` draws the
  design board's blurred shadow (ADR 0018).

Nothing blinks.

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
ADR 0014: ADR 0018 (lot D-02) renders Evidence with wgpu on the AMD chip, and keeps tiny-skia
when there is none. Its measurements are in the ADR.

`cargo test --release -p evidence frame_cost -- --ignored` writes the headless cost of a frame
(view, layout, render) to `target/tmp/frame_cost.txt`.

## Comparing with the design board

1. Render the design board once, from a copy of `docs/design/planches/Fichiers.dc.html` saved as
   `board.html`: replace `/_blob/5a8a…` with `docs/design/images/sherlock-tete.png`, drop the
   `support.js` line, then
   `chromium --headless --disable-gpu --hide-scrollbars --window-size=1440,900 --screenshot=board.png board.html`.
   Crop the window: `(70, 64)` to `(1370, 740)`, 1300 × 676.
2. `cargo test -p evidence window_renders_headless` (see `scenarios.rs`) writes
   `target/tmp/evidence-tiny-skia.png`, at scale 2: the same folder as the design board (a git repo
   with a modified `src`, a new `Dockerfile`, an ignored `target`), Dockerfile selected, menu
   and submenu open where the design board has them. Scale it down to 1300 × 676.
3. Put the two side by side. The pair of this lot is in `docs/evidence/`.

The context menu was also checked in a real window, in an isolated Xephyr (`Xephyr :7 -screen
1300x676`), Evidence started with `DISPLAY=:7`, temporary `HOME`/XDG directories and neutral
fake applications, the pointer driven by XTest on `:7` only: `docs/evidence/menu-xephyr.png`
(menu open), `open-with-xephyr.png` (submenu open) and `open-with-xephyr-2s.png` (the same
two seconds later, pointer still).

On the author's laptop: `cargo run --release -p evidence -- <a git project>`, then compare
with the design board at 1440 × 900 (the window is 1300 × 676, as on the design board).

## Gaps with the design board

- No grid or column views: list only.
- No "Ask Sherlock" button, Sherlock bubble, "Build the image" or "Run in a sandbox"
  buttons, no Dev section (Docker volumes, sandboxes): out of this lot's scope.
- « Ouvrir avec » is wider than the design board's submenu (280 instead of 216): application names
  and the "always" switch. The design board's « Éditeur » entry is the list of installed
  applications; its « Bac à sable » entry is out of scope.
- The open submenu entry uses the selection token at 0.26 (the design board has 0.30; see
  `docs/sherlock-ui.md`).
- A selected row's git badge sits on the darkest background: the design board's veil would fall
  under 4.5:1 contrast on the selection.
- The search field has no clear button. Column header letter spacing is not supported by iced.
- The sort arrow next to the sorted column is an addition, and so are Copier and Couper in
  the context menu.
- The design board has no Devices section: its entries use the sidebar's style, with an eject
  button in the style of sherlock-ui's icon buttons.
- The context menu scales in but cannot fade (see Animations).

## Limits

- No folder watching: the list reloads after each operation, with F5 or Ctrl+R.
- The clipboard is internal: copy and cut between Evidence and another application are not
  shared (the path is copied to the system clipboard with Ctrl+Shift+C).
- Copy progress is a message in the status bar, without a percentage; a copy cannot be
  cancelled.
- The trash handles the home trash only: a file on another disk is refused with a message.
  `directorysizes` is not maintained yet (the specification makes it optional).
- Encrypted volumes show locked; unlocking them is not done yet.
- File types come from names (`globs2`); content sniffing (`magic`) is not done.
- Application icons are looked up in hicolor and pixmaps only, not in the current icon theme.
  Applications with `Terminal=true` are started directly, not in a terminal.
- Long names are cut from an estimated character width, not measured text.
- Accessibility: iced 0.14 exposes nothing to screen readers (ADR 0014).
- Left for later, from the second review: `type_of` (a walk over the `globs2` patterns, about
  0.3 ms in a debug build) still runs in `update` when the « Ouvrir avec » submenu opens; moving a
  folder across disks drops the sockets and devices it contains (with the "left aside"
  message); a dark trace of the menu shadow was seen under the menu in Xephyr.

## Tests

`just verify` runs them without a screen, on temporary folders. The trash, the XDG
directories and `mimeapps.list` used by the tests are in the temporary folder, never the
user's; no test calls the real UDisks.

- `ops`: copy (with links), named pipes recreated and never read, names that are not UTF-8,
  modification times kept, a failed copy that leaves nothing, no-replace rename, duplicate and
  name conflicts, folder into itself, move and refused move, rename and invalid names, new
  folder.
- `trash`: put and restore (folder recreated), two items with the same name, refused restore,
  emptying only the listed items, long names, orphan items, the deletion date from the record,
  path encoding.
- `git`: porcelain v2 parsing (renames, spaces, root prefix, whole ignored folder), a real
  repository, and a trapped repository that must run nothing.
- `listing`, `preview` (named pipe never opened, image decoded, broken image), `places`,
  `clock`, `icons`, `devices` (filtering, internal data partitions, naming, kinds, free space
  and errors from UDisks2 objects recorded with `busctl --json`, made anonymous, plus a USB key,
  an SD card and a LUKS disk in the same format).
- `scenarios`: keyboard navigation, copy/cut/paste and conflicts, rename and new folder, trash
  with restore and confirmed emptying, context menu from the keyboard, devices through a fake
  source, « Ouvrir avec » with fake applications and a default written to a temporary
  `mimeapps.list`, and a headless capture of the window.
- `sherlock_ui::animated`: the hover fade, and a reused row that starts from rest;
  `sherlock_ui::component`: no hover on a disabled control.
- `mime-apps`: see `docs/mime-apps.md`.

## Manual procedure

On the author's laptop, Plasma X11:

1. `cargo run --release -p evidence -- <a git project>`. Check the list, the git badges, the
   branch, the preview of a text file and of an image.
2. Hover rows and buttons (150 ms fade), move with the arrows (the highlight glides), open a
   folder (rows rise and fade in), right-click (the menu scales in), hover « Ouvrir avec ».
3. In a scratch folder: copy, cut, paste, Ctrl+D, F2, Ctrl+Shift+N, Delete, then the trash:
   restore, then empty (the question appears; Escape cancels).
4. Right-click a text file, hover « Ouvrir avec »: the editors installed on the laptop are
   listed with their icon. Turn on "Toujours ouvrir ce type ainsi", pick one: it opens, and
   `~/.config/mimeapps.list` gains it under `[Default Applications]` (back the file up first;
   the other lines stay as they were). Enter on another file of the same type opens it with
   that application. « Autre application… » lists every application, with a search.
5. Plug in a USB key: it appears under Appareils. Click it: Plasma's polkit agent may ask for
   the password, then the key opens. Right-click: « Démonter », « Éjecter ». Click the eject button:
   the key disappears from the list and the status bar says it can be removed. With a file of
   the key open in another program, « Démonter » shows the "busy" message.
6. `RUST_LOG=evidence=info cargo run --release -p evidence -- --measure <folder of 5,000
   files> --maximized` and read the three `measure:` lines.
