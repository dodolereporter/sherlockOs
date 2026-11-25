# appearance-config

`desktop/appearance-config` reads the appearance settings file and follows it live. Every
program of the desktop uses it to pick its theme; the settings application (Control Room, D-26)
will write the file. Decision: ADR 0018.

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
- Fixed hours only: sunrise and sunset would need a position (ADR 0018).
- The crate does not write the file yet: Control Room will need it, with the unknown keys kept,
  as `dock-config` does.

## Tests

`cargo test -p appearance-config`: reading, defaults and problems; the automatic mode at its
bounds (06:59:59, 07:00, 18:59:59, 19:00) and across midnight, with a light span that holds
midnight; a file created in a directory that did not exist, then replaced by a rename, changes
the theme live.
