# appearance-config

`desktop/appearance-config` reads the appearance settings file and follows it live. Every
program of the desktop uses it to pick its theme; the settings application (Control Room, D-26)
will write the file. Decision: ADR 0019.

## The file

`$XDG_CONFIG_HOME/appearance/appearance.conf` (`~/.config/appearance/appearance.conf` by
default), a key file like the desktop entries:

```text
[Appearance]
Theme=auto
AutoLightFrom=07:00
AutoDarkFrom=19:00
```

| Key | Values | Default |
| --- | --- | --- |
| `Theme` | `dark`, `light`, `auto` | `dark` |
| `AutoLightFrom` | Local time `HH:MM`, from which `auto` is light | `07:00` |
| `AutoDarkFrom` | Local time `HH:MM`, from which `auto` is dark | `19:00` |
| `ReduceMotion` | `true`, `false`: what only decorates a change jumps to its end (lot D-26p) | `false` |
| `ReduceTransparency` | `true`, `false`: every glass opaque | `false` |
| `IncreaseContrast` | `true`, `false`: opaque glass, firmer borders, darker secondary texts | `false` |
| `Effects` | `full`, `reduced` (the light mode), `none` | `full` |

The last four are Control Room's Accessibility page. `follow` applies them to the process
(`Accessibility::apply`: sherlock-ui's `motion` and `color` read them) each time the file
changes them, sends `Event::Accessibility`, then the theme again, so that the program redraws;
`follow_themes` does the same for the programs that only want the themes.

Every key is optional; an absent file gives the defaults. A value that cannot be read gives
its default and a `Problem` for the caller to log. Two equal hours are refused: both go back to
their defaults. `AutoLightFrom` may come after `AutoDarkFrom`: the light span then holds
midnight (light from 22:00 to 06:00, for instance).

## Light values that replace the table

The light palette follows `docs/design/THEME-CLAIR.md`, except for five text colors. With the
values of the table, they stayed below 4.5 to 1 on some backgrounds of the light theme; on the
author's decision (D-05a), these values replace those of the table. Each is the table color
darkened toward the night blue `#14233B` just enough to reach 4.5 on its worst background.

| Role | Table | Kept | Worst background, before then after |
| --- | --- | --- | --- |
| Tertiary text | `#5C6B82` | `#4F5E75` | Lightest desktop disc `#C8D8E9`: 3.73, then 4.53 |
| Line numbers, ghost text | `#7A889D` | `#657388` | Code area `#F6F8FB`: 3.38, then 4.53 |
| Green as text or icon | `#2E7D4F` | `#2C764D` | Success badge on the title bar: 4.15, then 4.54 |
| Amber as text | `#9A6412` | `#936114` | Warning badge on the title bar: 4.26, then 4.52 |
| Salmon as text | `#B23A2A` | `#92352D` | Destructive button hovered, on the lightest disc: 3.60, then 4.59 |

The ring of the color logo in the top bar takes the icon green of the table, `#2E7D4F`, in the
light theme (4.65 to 1 on the bar, against 1.41 for the fill green). The sherlock-ui test checks
the table value of each row and these replacements (`OVERRIDES` in `color.rs`).

The wallpaper follows the theme (author's decision): in the light theme, the default becomes
the light version of « Premier indice ». D-04 holds the pictures and pairs each dark wallpaper
with its light one; the compositor changes to the pendant when the theme changes, with a
crossfade (lot D-05c, "Theme" in `docs/compositor.md`).

## API

| Item | Role |
| --- | --- |
| `AppearanceConfig::parse`, `load` | The settings and the problems of a text, of a file |
| `AppearanceConfig::theme_at(now)` | The theme at the time of day `now`, and the time of day of the next switch (`None` for `dark` and `light`) |
| `TimeOfDay::at(epoch)` | Local time of day of an instant, from `localtime_r` |
| `TimeOfDay::next_after(epoch)` | The next instant with that local time, from `mktime`: right across a change of daylight saving time |
| `epoch_now` | Seconds since 1970, from the real-time clock |
| `follow(path, on_event)` | Calls `on_event` with the theme, then at every change; `Event::Problem` and `Event::Error` report what went wrong |
| `follow_themes(path, on_theme)` | `follow` for a program that only wants the themes: problems and errors go to the log (`tracing`) |
| `theme_now(path)` | The theme of the file now, read once: what a program makes active before its first frame |
| `local(epoch)` | Broken-down local time (`struct tm`) of an instant, the time zone read again first; the top bar's clock uses it |
| `path` | Path of the file |

`follow` runs one thread, which waits with `poll` on two descriptors:

- **inotify**, on the directory of the file, so that a file replaced by a rename is seen. A
  read is taken in full: a deleted directory sends the deletion of its file and its own end
  together. When the directory is deleted or moved (`IN_DELETE_SELF`, `IN_MOVE_SELF`,
  `IN_IGNORED`), or events were lost (`IN_Q_OVERFLOW`), the watch starts again. While the
  directory is missing, the closest existing ancestor is watched, as many levels up as needed,
  and the file reads as absent: the default theme.
- **a timerfd on `CLOCK_REALTIME`**, armed on the absolute instant of the next switch
  (`TFD_TIMER_ABSTIME`). It counts real time: a switch that passed during a suspend fires on
  wake-up. With `TFD_TIMER_CANCEL_ON_SET`, a change of the clock (`timedatectl`, NTP at start)
  wakes the thread with `ECANCELED`. Either way, the thread reads the time again and arms the
  next switch. The time zone is read again each time (glibc's `mktime` runs `tzset`).

No polling. The thread stops the next time it reports something after `on_event` returned
`false`.

A program wires it like the top bar, Evidence, the dock and the compositor (`theme.rs`, through
a calloop channel) do: `color::set_active(theme_now(..))`
before its first frame, so that a light session never starts on a dark frame, then a
subscription whose stream receives the themes of `follow_themes`, given to a `color::ThemeFade` in `update`, which fades
the palette shown over `motion::THEME_FADE` (see "Following the theme" in
`docs/sherlock-ui.md`). The sherlock-ui gallery switches at once with `color::set_active`.

## Limits

- Fixed hours only: sunrise and sunset would need a position (ADR 0019).
- A change of time zone alone does not cancel the timer: it is seen at the next wake-up (edit,
  switch or change of the clock).
- The crate does not write the file yet: Control Room will need it, with the unknown keys kept,
  as `dock-config` does. A shared `config-watch` crate will take the inotify part of this crate
  and of the dock, in a lot of its own.
- Moving an ancestor of the folder is not seen, for instance `mv ~/.config ~/.config.old` then
  a new `~/.config/appearance/`: the folder itself is neither deleted nor moved, so the kernel
  sends no `IN_MOVE_SELF`, and the watch follows the old inode. The program sees the new file at
  its next start. The shared `config-watch` crate may treat it.
- A switch time inside the hour that the clocks skip or repeat comes late, on those two nights
  only: in Paris, 02:30 does not exist on the last Sunday of March and `mktime` places it at
  03:30 (up to 59 minutes late for 02:59); it exists twice on the last Sunday of October and
  `mktime` takes the second one (an hour late). Times outside 02:00 to 02:59 are not touched.
- When the watch cannot be set up (`EMFILE` past `max_user_instances`, `ENOSPC`, `EACCES`), it is
  tried again only at the next wake-up of the thread. In the `dark` and `light` modes the timer
  is disarmed, so the file is not followed again until the clock changes: the program logs the
  error, and its next start reads the file.

## Tests

`cargo test -p appearance-config`, without sleeping:

- reading, defaults and problems; `+7:00` and other signs refused;
- the automatic mode at its bounds (06:59:59, 07:00, 18:59:59, 19:00), across midnight, and a
  light night at its bounds (21:59:59, 22:00, 05:59:59, 06:00);
- the next switch as an instant of the local clock, and across daylight saving time in a child
  process with `TZ=Europe/Paris`: on 28 March 2026 at 20:00, 07:00 comes 10 hours later; on
  24 October 2026, 12 hours later. Run straight by `--include-ignored` without that zone, the
  child test says it is skipped and passes;
- the timer: an instant already passed (after a suspend) fires at once, a disarmed timer does
  not. `ECANCELED` needs a change of the system clock, so it is not tested;
- inotify: every event of a read counts (a deleted directory, a moved one, lost events); a file
  created two missing levels down, replaced by a rename, its folder deleted then made again,
  moved away then made again: the theme follows each time.
