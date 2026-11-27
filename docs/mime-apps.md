# mime-apps

`desktop/mime-apps` answers three questions for the desktop: which applications are
installed, what type a file is, and which application opens a type by default. Evidence uses
it for « Ouvrir avec » and for opening files; the "Default applications" page of Control Room
(lot D-26) will reuse it. It depends on the standard library only, reads and writes plain
files, and never runs `xdg-mime` or talks to D-Bus.

It follows three freedesktop specifications:

| Specification | What it reads or writes |
| --- | --- |
| Desktop Entry 1.5 | `applications/**/*.desktop` in every XDG data directory |
| Shared MIME-info 0.21 | `mime/globs2` and `mime/subclasses` in every XDG data directory |
| MIME Applications Associations 1.0.1 | every `mimeapps.list`, and the user's one for writing |

## API

```rust
use mime_apps::{Catalog, Dirs};

let dirs = Dirs::from_env(&home);                 // XDG_* variables, or their defaults
let catalog = Catalog::load(dirs, &desktops, &locales); // read once, off the interface thread
let mime = catalog.mime.type_of(path, is_dir);   // "text/rust", "inode/directory"…
let apps = catalog.handlers(&mime);              // who opens it, the default first
let default = catalog.default_for(&mime);
let (program, args) = default?.command(&[path.to_path_buf()])?; // Exec expanded, no shell
let (program, args) = catalog.command(app, &files)?;          // in a terminal if needed
mime_apps::set_default(&catalog.dirs, &desktops, &mime, &app.id)?; // remember a choice
```

- `Dirs` holds the XDG directories (`config_home`, `config_dirs`, `data_home`, `data_dirs`).
  Tests build one on a temporary folder.
- `Catalog::load(dirs, desktops, locales)`: `desktops` are the lower-case names of
  `XDG_CURRENT_DESKTOP` (`kde`), for `kde-mimeapps.list`; `locales` the names to try for
  application names, most specific first (`fr_FR`, `fr`).
- `Catalog::handlers(mime)`: the associations' defaults for the type and its parents, then the
  added associations, then the applications declaring the type, then those declaring a parent
  (`text/rust` derives from `text/plain`, every `text/*` does). Removed associations are left
  out. No duplicates, by desktop id or by name and command.
- `Catalog::choosable()`: every application not hidden from menus, by name, for a "choose an
  application" list.
- `DesktopEntry` holds the id, the file, `Name`, `GenericName` and `Keywords` (all three
  localized), `Exec`, `Path` (`working_dir`), `Icon`, `MimeType`, `NoDisplay`, `Terminal` and
  `TryExec`. String values are unescaped (`\s`, `\n`…); in lists (`Keywords`, `MimeType`),
  `\;` is a semicolon inside an item. The launcher (`docs/launcher.md`) searches the name,
  generic name and keywords.
- `DesktopEntry::command(files)`: splits `Exec` with its quoting rules and expands the field
  codes (`%f %F %u %U %i %c %k %%`; deprecated codes are dropped).
- `Catalog::command(app, files)`: the same, run inside a terminal when the entry says
  `Terminal=true`. The terminal is `$TERMINAL` when it names an installed program, else the
  first installed of `x-terminal-emulator`, `konsole`, `gnome-terminal` (with `--`), `kitty`,
  `alacritty`, `foot`, `xterm` (with `-e`). Without any terminal, `Terminal=true` entries are
  left out of the catalog, so nothing that needs one can be picked. `Catalog::load_with_terminal`
  takes the terminal as an argument instead of looking it up (tests, a future setting).
- `set_default(dirs, desktops, mime, app)`: in `$XDG_CONFIG_HOME/mimeapps.list`, puts `app`
  first for `mime` under `[Default Applications]` and `[Added Associations]`, keeps the other
  applications after it, and takes `app` out of `[Removed Associations]` for that type (it would
  cancel the choice). Every other section, entry, comment and blank line stays as it was.
  `<desktop>-mimeapps.list` files of the current desktops (`kde-mimeapps.list` under Plasma)
  are read before `mimeapps.list`: when one already has a default for the type, that entry is
  updated the same way, otherwise it would hide the choice; a desktop file without an entry for
  the type is not touched, and none is created.
- Writing: the file is written beside the old one, flushed, then renamed over it, so another
  program reading it sees the old file or the new one, never half of one. Its permissions are
  kept. When `mimeapps.list` is a symbolic link (dotfiles kept in a repository), the file it
  points to is written, in its own folder, and the link stays. A mime type or an application id
  with a line break, `;` or `=` is refused: it would add entries nobody chose.

## Settings files of the desktop

`mime_apps::file` is the one reader and writer of every settings file of the desktop (Q-41):
`mimeapps.list`, `dock.conf`, `appearance.conf`, the `key = value` files of `desktop-config`,
`notifications.conf`, the qt5ct and qt6ct files of `toolkit-theme`, and the themes'
`index.theme` (read only).

- `write_atomically(path, text)` and `write_bytes_atomically(path, bytes, new_mode)`: a
  temporary file of its own in the folder of the file the links name
  (`.<name>.<pid>.<count>.tmp`, created with `O_EXCL`), with the permissions of the file it
  replaces from the start and its owner when the process may give it, flushed, renamed over the
  old file; then the folder is flushed. A folder that refuses the flush (some network file
  systems) leaves the write done, not failed. A new file gets `new_mode` through the umask.
  Temporary files left by a write that was stopped (their process is gone) are removed before
  the next write of the same file.
- `read_bounded(path, MAX_SETTINGS_SIZE)`: a regular file only, once its links are followed,
  1 MiB at most, in UTF-8. A link to `/dev/zero` or a huge file is refused instead of taking all
  the memory of the compositor, the bar or the dock. A missing file stays `NotFound`.
- `breaks_line(value)`: every writer refuses a value with a line break, which would end its
  line and add others (`desktop-config` keeps the value of the file for that key).

## Order of the files

Reading merges, most important first: for `$XDG_CONFIG_HOME`, then each `$XDG_CONFIG_DIRS`,
then `$XDG_DATA_HOME/applications` and each `$XDG_DATA_DIRS/applications`, the
desktop-specific `<desktop>-mimeapps.list` before `mimeapps.list`. A desktop id found in a more
important data directory hides the same id further down, even when that first entry is
`Hidden=true` (how a user removes a system application). An entry whose `TryExec` program
does not exist is left out. Folders under `applications/` are entered once each, by their real
path: a loop of symbolic links lists nothing twice.

## Limits

- Types come from file names (`globs2`, case-insensitive unless the pattern says `cs`);
  content sniffing (`magic`) is not done. An unknown name is `application/octet-stream`.
- The `mimeinfo.cache` files are not read: applications are scanned directly, which also
  covers user entries that no cache lists.
- `OnlyShowIn` and `NotShowIn` are not checked yet.
- A field code glued inside an argument (`--file=%f`) goes through a lossy string: a file name
  that is not UTF-8 is altered there (a lone `%f` keeps its bytes).
- Patterns support `*`, `?` and plain `[abc]` sets, not ranges or negation; the shared MIME
  database hardly uses them.
- Icons are not part of this crate: Evidence looks them up in hicolor and pixmaps
  (`desktop/evidence/src/icons.rs`).

## Bounded reads

Every file this crate reads comes from outside the program. `file::read_bounded` reads a text
file only when it is a regular file once its links are followed, no larger than the bound given,
and in UTF-8; anything else is an error (`NotFound` for an absent file, as with
`read_to_string`). Desktop entries are bounded to 256 KiB (`entry::read_entry`), settings files
to 64 KiB (`file::MAX_SETTINGS_SIZE`). The dock (its entries, `dock.conf`, `mimeapps.list`) and
`icon-theme` (`index.theme`, 512 KiB) use the same function. `entry::desktop_files` lists the
`.desktop` files of an `applications` folder with their ids, sub-folders included, for the dock.

## Tests

`cargo test -p mime-apps`, on temporary XDG directories only: desktop entries (localised
names, hidden entries, user entries hiding system ones), `Exec` expansion with quotes and
field codes, type detection (patterns, case, longest pattern), parent types, merging of
several `mimeapps.list` files, writing a default that keeps everything else, creating a missing
file, handlers ordered with the default first, a `kde-mimeapps.list` default updated and a
removed association lifted, bounded reads refusing a link to `/dev/zero`, a large file and
non UTF-8 text, a linked `mimeapps.list` written through its link, `TryExec` and
link loops, and `Terminal=true` entries run in a terminal.
