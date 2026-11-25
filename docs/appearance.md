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
with its light one; D-05c links the active theme to the wallpaper.

## API

| Item | Role |
| --- | --- |
| `AppearanceConfig::parse`, `load` | The settings and the problems of a text, of a file |
| `AppearanceConfig::theme_at(now)` | The theme at `now`, and how long until the automatic mode switches (`None` for `dark` and `light`) |
| `TimeOfDay::now` | Local time, from `date`: the standard library does not know the time zone |
| `follow(path, on_event)` | Calls `on_event` with the theme, then at every change; `Event::Problem` and `Event::Error` report what went wrong |
| `path` | Path of the file |

`follow` runs two threads. One waits on inotify, on the directory of the file (or on its parent
until the directory exists), so that a file replaced by a rename is seen. The other reloads the
file when told, then sleeps until the next switch of the automatic mode or the next edit. No
polling. Both stop once `on_event` returns `false`.

A program wires it like the sherlock-ui gallery does (`examples/gallery/view.rs`): a
subscription whose stream receives the themes, and `color::set_active(theme)` in `update`.

## Limits

- The timer counts monotonic time: after a suspend that runs past a switch time, the theme
  changes at the next edit or the next switch. A change of time zone or of daylight saving time
  is seen at the next wake-up as well.
- Fixed hours only: sunrise and sunset would need a position (ADR 0019).
- The crate does not write the file yet: Control Room will need it, with the unknown keys kept,
  as `dock-config` does.

## Tests

`cargo test -p appearance-config`: reading, defaults and problems; the automatic mode at its
bounds (06:59:59, 07:00, 18:59:59, 19:00) and across midnight, with a light span that holds
midnight; a file created in a directory that did not exist, then replaced by a rename, changes
the theme live.
