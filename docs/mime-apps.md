# mime-apps

`desktop/mime-apps` answers three questions for the desktop: which applications are
installed, what type a file is, and which application opens a type by default. Evidence uses
it for "Ouvrir avec" and for opening files; the "Default applications" page of Control Room
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
mime_apps::set_default(&catalog.dirs, &mime, &app.id)?;        // remember a choice
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
- `DesktopEntry::command(files)`: splits `Exec` with its quoting rules and expands the field
  codes (`%f %F %u %U %i %c %k %%`; deprecated codes are dropped).
- `set_default(dirs, mime, app)`: in `$XDG_CONFIG_HOME/mimeapps.list`, puts `app` first for
  `mime` under `[Default Applications]` and `[Added Associations]`, keeps the other
  applications after it, and leaves every other section, entry, comment and blank line as it
  was. The file is written beside the old one, flushed, then renamed over it: another program
  reading it sees the old file or the new one, never half of one. The file's permissions are
  kept.

## Order of the files

Reading merges, most important first: for `$XDG_CONFIG_HOME`, then each `$XDG_CONFIG_DIRS`,
then `$XDG_DATA_HOME/applications` and each `$XDG_DATA_DIRS/applications`, the
desktop-specific `<desktop>-mimeapps.list` before `mimeapps.list`. A desktop id found in a more
important data directory hides the same id further down, even when that first entry is
`Hidden=true` (how a user removes a system application).

## Limits

- Types come from file names (`globs2`, case-insensitive unless the pattern says `cs`);
  content sniffing (`magic`) is not done. An unknown name is `application/octet-stream`.
- The `mimeinfo.cache` files are not read: applications are scanned directly, which also
  covers user entries that no cache lists.
- `TryExec`, `OnlyShowIn` and `NotShowIn` are not checked yet.
- Icons are not part of this crate: Evidence looks them up in hicolor and pixmaps
  (`desktop/evidence/src/icons.rs`).

## Tests

`cargo test -p mime-apps`, on temporary XDG directories only: desktop entries (localised
names, hidden entries, user entries hiding system ones), `Exec` expansion with quotes and
field codes, type detection (patterns, case, longest pattern), parent types, merging of
several `mimeapps.list` files, writing a default that keeps everything else, creating a missing
file, and handlers ordered with the default first.
