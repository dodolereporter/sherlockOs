# Launcher

`desktop/launcher` is the desktop's application launcher: a search field that finds the
installed applications and starts them from the keyboard. It is a separate program started on
demand, shown above everything with layer-shell (ADR 0005), written with iced 0.14 and
`iced_exwlshell` 0.21.4 like the bar (`docs/bar.md`) and the dock (`docs/dock.md`). Batch
D-22a builds its core from the launcher of `Panneaux.dc.html`; files, settings, the calculator
and « Demander à Sherlock » come in D-22b. The crate, the binary and the layer-shell namespace
are all `launcher`. Its texts stay in French, from `sherlock-ui`.

![The launcher just opened](../desktop/launcher/screenshots/launcher-empty.png)

![Results for « te »](../desktop/launcher/screenshots/launcher-results.png)

![Nothing found](../desktop/launcher/screenshots/launcher-nothing.png)

These come from the headless rendering test (`tests/launcher.rs`), at scale 2, from the
fixtures in `desktop/launcher/fixtures`.

## What it shows

| Element | Board | Launcher |
| --- | --- | --- |
| Panel | 600 wide, 150 below the screen top, padding 8, radius 20, glass `rgba(16,26,44,0.88)`, border `rgba(255,255,255,0.12)`, shadow `0 32px 80px` | Same, centered across the screen; blur and shadow are the compositor's job, as for the dock |
| Search row | 52 high, green magnifier of 20, query at 18, « Échap » key cap, line `rgba(255,255,255,0.08)` under it | Same; the board's hidden label « Rechercher ou lancer » is the placeholder |
| Section label | « Applications », 11 px semibold capitals, tertiary text, padding 10 10 4 | Same; letter spacing is a known gap of iced 0.14 (`docs/sherlock-ui.md`) |
| Result | 46 high, radius 10, tile of 30 at radius 8, name in medium weight, a 12 px line under it | Same; the line is the entry's `GenericName`; the selected row takes the selection blue and the « Entrée » key cap |
| Icon | A line icon on the tile | The application's initial on the tile, see Open questions |
| Nothing found | Not on the board | « Aucun résultat » in secondary text, one row high |

The light theme takes the light board's values: white glass at 0.88, night blue veils at the
same opacity. The launcher reads the theme of the appearance file once at start
(`appearance-config`, ADR 0019): it lives a few seconds and does not follow changes.

## Architecture

| Module | Role |
| --- | --- |
| `main.rs` | Renderer choice, logs, single instance, then `run` |
| `lib.rs` | Layer-shell surface, iced settings, theme, locales, sources, startup |
| `app.rs` | State (`Launcher`), messages, keys, update, view |
| `source.rs` | The `Source` trait, `Hit`, `Action`, `Command`, sections |
| `apps.rs` | The installed applications, a `Source` |
| `search.rs` | Folding (case, accents), scoring a term, `Haystack` |
| `launch.rs` | Starting a program without a shell |
| `instance.rs` | The lock file that keeps one launcher at a time |

Every color, size and text comes from `sherlock_ui::launcher` (one new file and a `pub mod`
line): the geometry, `panel_height`, the launcher's own colors for each theme (`Colors::DARK`,
`Colors::LIGHT`), the styles `panel`, `divider`, `key`, `tile`, `row`, `query`, and the texts.
The rest comes from the palette of the active theme (`color::palette()`).

### Data: `mime-apps`, not the dock's reader

Both read desktop entries. `mime-apps` (D-27) is the one meant to be shared, the dock plans to
move onto it (`docs/dock.md`), and it already did more of what the launcher needs: folders
under `applications/` (`kde/org.kde.konsole.desktop`), localized names for the locales of
`LC_ALL`, `LC_MESSAGES` or `LANG`, `Hidden` entries masking the same id further down, `TryExec`,
and `Terminal=true` entries run in a terminal (left out when there is none). It lacked
`GenericName`, `Keywords` and `Path`: this batch adds them to its `DesktopEntry` (localized like
the name), and nothing else changes there. The launcher leaves out `NoDisplay` entries.

### Surface

Anchored to the top only, so the compositor centers it, with a top margin of 150 and an
exclusive zone of -1: the panel sits at the board's place whatever the bar reserves. Layer
`Overlay`, above the bar, the dock and the windows. The keyboard is `Exclusive`: the launcher
gets every key as soon as it maps, and the search field takes the focus at once. The surface is
exactly the panel: 600 wide, as high as `launcher::panel_height(sections, rows)`, and it is
resized (`LayoutChange`) when the number of results changes.

### Search

Every key typed in the field searches again: the first one already filters. An empty query shows
no result. A query is folded (lower case, accents removed: « É » is « e », « œ » is « oe ») and
split into terms; every term must be found. Each term is scored where it fits best:

| Where | Score |
| --- | --- |
| The whole name | 1000 |
| Start of the name | 900 |
| Start of a word of the name | 800 |
| Inside the name | 600 |
| The letters in order in the name (fuzzy) | 400, less 10 per letter skipped, 100 at least, plus 50 when the first starts a word |
| `GenericName` | The same, 100 less, only from the start of a word (800 and up) |
| `Keywords` | The same, 200 less, only from the start of a word |

Scattered letters only count in the name: in the longer generic names and keywords they matched
almost anything. A result's score is the sum over the terms. The order is stable and total:
score, then folded name, then desktop id. The files' order never matters (tested by reading
them backwards). At most 8 results are shown (`launcher::MAX_RESULTS`), which keeps the panel
clear of the dock on the 900 high reference screen. Searching 500 applications takes well under
50 ms (tested, in a debug build).

### Keyboard

| Key | Action |
| --- | --- |
| Letters | Typed in the field, results filtered |
| Down, Up | Next, previous result; they stop at both ends |
| Enter | Starts the selected result, then the launcher closes |
| Escape | Closes the launcher |

The first result is selected after every key. The keys are read with `event::listen_with`,
whatever the field did with them: iced's text field takes Escape for itself. A click on a result
starts it too.

« ki » then Enter starts Kitty: three keys after opening. The test types the two letters into
the field through the headless simulator, then sends Enter. Another test checks that every
application of the fixtures is the first result for two letters of its name.

### Starting a program

As in the dock: the `Exec` line is split and expanded by `mime-apps` (no files given, so file
field codes disappear), run without a shell, with the environment the launcher started with
minus its renderer choice (`sherlock_ui::gpu::for_child`, ADR 0018), in its own process group,
with no standard streams, in the entry's `Path` when it has one. `Terminal=true` entries run
in the terminal `mime-apps` picks (`$TERMINAL`, then the usual ones). The launcher closes
right after; if the start fails, it stays open and logs the error.

### One launcher at a time

The launcher is started on demand: by a keyboard shortcut of the compositor later, by hand for
now. Started again while open, it closes the open one: pressing the shortcut twice toggles it.

- The first launcher opens `$XDG_RUNTIME_DIR/launcher.lock`, takes an exclusive `flock` on it
  without waiting, and writes its process id in it. It keeps the file open while it runs.
- A second one cannot take the lock: it reads the process id, sends it `SIGTERM`, and stops
  without showing anything. Only a plain id above 1 and other than its own is signalled.
- The kernel drops the lock when its holder ends, crash included: a stale file never blocks
  the next start. The file is opened close-on-exec, so started programs never inherit the lock.
- Without `XDG_RUNTIME_DIR`, or when the file cannot be opened, the launcher runs without the
  lock and logs a warning.

## Running it

```sh
cargo run -p launcher                 # in a Wayland session with layer-shell
RUST_LOG=launcher=debug cargo run -p launcher
```

## Limits

- No application icons yet: the initial of the name sits on the tile. The icon lookup lives in
  the dock (`desktop/dock/src/icons.rs`) and in Evidence (`desktop/evidence/src/icons.rs`); see
  Open questions.
- `OnlyShowIn` and `NotShowIn` are not checked (a limit of `mime-apps`).
- A click outside the panel does nothing: the surface is only the panel, and with the keyboard
  `Exclusive` the launcher keeps it until Escape or a start. To revisit with the compositor's
  shortcut.
- The `Comment` key is not read: the line under a name is its `GenericName`.
- No real session was run for this batch: the author's machine rules forbade opening a window.
  The checks of the next section are still to do.

## Checks in a session

1. `cargo run -p launcher`: the panel appears 150 below the top, centered, above everything,
   and typing goes into the field without a click.
2. Type « ki », Enter: Kitty starts, the launcher closes.
3. Start it twice: the second start closes the first.
4. Escape closes it. `kill -9` it, start it again: it opens (the lock was released).

## Tests

`cargo test -p launcher`, on the fixtures only:

- applications: `NoDisplay`, `Hidden` and missing `TryExec` left out, `Terminal=true` only with
  a terminal, localized names, a subfolder;
- search: folding, the order of the tiers, the name before the generic name and the keywords,
  every term required, a stable order whatever the file order, the first key filtering, 500
  applications under 50 ms;
- keyboard: « ki » typed into the field then Enter starts Kitty and closes the launcher; every
  fixture reachable with two letters; arrows stop at the ends; a new query selects the first
  result; Escape closes; a click starts its result in its `Path`;
- start: no shell (a `;` in an argument stays in the file name), the working directory, a
  missing program refused;
- layout: the panel height for each state, the texts drawn, the surface's place, and three
  screenshots in `target/tmp`;
- single instance: the lock taken, our own id never signalled, a sleeping child closed by
  `SIGTERM`, the lock free again after, odd file contents signalling nobody.

`cargo test -p sherlock-ui launcher`: contrast of the texts on the panel and on the selected
row in both themes, and the panel height.

## Open questions

- **Icons.** Showing the applications' icons needs the theme lookup the dock and Evidence each
  have. Rather than a third copy, move one into a shared crate (or into `mime-apps`) in a batch
  of its own, then the launcher, the dock and Evidence use it.
- **Locales.** `launcher::locales` repeats the function Evidence keeps private; it could move to
  `mime-apps` with that same batch.
