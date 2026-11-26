# icon-theme

`desktop/icon-theme` finds the file of an icon from its name (the `Icon` key of a desktop
entry), as the freedesktop Icon Theme specification describes it. It is meant for every
program of the desktop that shows application icons: the launcher uses it since D-22b; the
dock (`desktop/dock/src/icons.rs`) and Evidence (`desktop/evidence/src/icons.rs`) still carry
their own lookup and are meant to move onto it in a later batch. It depends on the standard
library only.

## API

```rust
let data: Vec<PathBuf> = dirs.data().cloned().collect();       // $XDG_DATA_HOME first
let theme = icon_theme::user_theme(&dirs.config_home);          // GTK's, else Plasma's
let lookup = icon_theme::Lookup::new(&data, Some(&home), theme.as_deref())
    .context("Applications");                                   // optional
let file = lookup.find("kitty", 30, 2);                         // Option<PathBuf>
```

- `Lookup::new(data_dirs, home, theme)` reads the `index.theme` of the chosen theme, of the
  themes it inherits from (depth first, in the order of `Inherits`), and of `hicolor`, which
  always comes last. A theme that is not installed is skipped; a loop of `Inherits` ends. A
  theme name or a `Directories` entry holding `..` or starting with `/` is refused: the lookup
  never leaves the base folders.
- `context(name)` keeps only the folders of that `Context`: an application icon is then looked
  for in some forty folders instead of the eight hundred of `hicolor`.
- `find(name, size, scale)` returns the file. An absolute name is taken as it is when the file
  exists; a name written with its extension (`kitty.png`) is looked up without it; a name with a
  `/` in it finds nothing.
- `user_theme(config_home)`: `gtk-icon-theme-name` of `gtk-3.0/settings.ini` (then `gtk-4.0`),
  which Plasma writes too, else `Theme` in the `[Icons]` group of `kdeglobals`.

## Lookup

As `FindIcon` in the specification, without the cache file:

1. In each theme of the chain, `LookupIcon`: the first folder whose size matches exactly
   (`DirectoryMatchesSize`: the same `Scale`, the size within `Fixed`, `Scalable` or
   `Threshold` bounds) wins; else the folder at the smallest `DirectorySizeDistance`, in device
   pixels (size times scale), the first one written between equals. `ScaledDirectories` are
   read with `Directories`.
2. Then the base folders themselves (`$HOME/.icons`, `<data dir>/icons`) and every
   `<data dir>/pixmaps`.

Base folders: `$HOME/.icons`, then `icons` in each data directory. In each folder, `png` then
`svg`. XPM is left out: nothing in the desktop draws it.

The theme indexes are parsed once in `Lookup::new`; the icon files are looked for with one
`stat` each at every `find`. A program that shows the same icon often caches the answer (the
launcher does, by icon name).

## Tests

`cargo test -p icon-theme`, on a fake theme written in a temporary folder: the chosen theme
before its parent before `hicolor` before `pixmaps`, a loop of `Inherits`, an exact size before
a closer one, the `@2` folder at scale 2, a scalable folder's range, the closest fixed size, a
context narrowing the folders, theme names and folders climbing out refused, absolute names, names with an extension, `$HOME/.icons` first, a
missing theme, and the user's theme from GTK then from Plasma.

## Limits

- No `icon-theme.cache`: the files are looked for directly.
- No XPM.
- The dock and Evidence still have their own copy (see above).
