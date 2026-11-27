# Autostart (lot D-44)

The applications that start with the session: Discord minimized in the tray, a sync client, a
password manager. `sherlockos-autostart` (crate `desktop/autostart`) reads the `.desktop` files of
the autostart folders once, starts what it should, and stops once the last one is started. The
session script starts it with the shell (`desktop/compositor/session/sherlockos-session`);
`SHERLOCKOS_AUTOSTART=0` leaves it out.

It follows the Desktop Application Autostart Specification.

## Where the entries are

- `$XDG_CONFIG_HOME/autostart` (`~/.config/autostart`), the user's, first;
- then each `$XDG_CONFIG_DIRS/autostart` (`/etc/xdg/autostart`), in order.

A file name seen once hides the same name further down. A user turns a system entry off with a
file of the same name holding `Hidden=true`, which is what the settings of KDE and GNOME write.

## What is left out

| Key or reason | Rule |
| --- | --- |
| `Hidden=true` | Left out, and hides the system file of the same name |
| `OnlyShowIn` | Started only if it names one of `XDG_CURRENT_DESKTOP` (`SherlockOS` in the session). An entry for `KDE` or `GNOME` alone is not ours |
| `NotShowIn` | Left out if it names `SherlockOS` |
| `TryExec` | Left out when the program is not installed |
| `X-GNOME-Autostart-enabled=false` | Left out |
| `X-KDE-autostart-condition=file:group:key:default` | The boolean `key` of `[group]` in the configuration file `file` (the user's first, then `XDG_CONFIG_DIRS`), `default` when it is not set. Left out when false |
| `Terminal=true` | Left out: the session has no terminal to give it |
| Not an application, no `Exec` or no `Name` | Left out |
| `EXCLUDED` | Left out: the session already does it (below) |

The log of the session says why each entry is left out: `autostart: left out id=… skip=…`.

## What the session already does

`EXCLUDED` in `desktop/autostart/src/lib.rs` names them, by file name (without `.desktop`) or by
program. Most entries of KDE and GNOME already say `OnlyShowIn=KDE` or `GNOME`: this list is for
those that do not, or not always.

| Entries | Why |
| --- | --- |
| `polkit-kde-authentication-agent-1`, `org.kde.polkit-kde-authentication-agent-1`, `polkit-gnome-authentication-agent-1`, `polkit-mate-authentication-agent-1`, `lxpolkit` | The session starts the first polkit agent installed (lot D-35b): two agents fight over the requests |
| `gnome-keyring-secrets`, `gnome-keyring-daemon`, `kwalletd5`, `kwalletd6`, `org.kde.kwalletd5`, `org.kde.kwalletd6`, `pam_kwallet_init` | The session starts the keyring itself, on its own bus |
| `nm-applet`, `nm-tray`, `blueman`, `blueman-applet` | The bar shows the network and Bluetooth |
| `xembedsniproxy`, `org.kde.plasmashell`, `org.kde.kded6` | Parts of the Plasma session |

## How it starts them

- The command is the `Exec` line, split by its quoting rules and its field codes expanded with
  no file (`%U`, `%f`… vanish, `%c` is the name, `%k` the file): `mime_apps`, the same code as
  Evidence and the launcher. Nothing goes through a shell. `Path` is the working directory.
- `X-GNOME-Autostart-Delay` (seconds) delays it, from the start of `sherlockos-autostart`, at
  most 300 s. The entries start the soonest first.
- The child gets the environment of the session (`WAYLAND_DISPLAY`, `DISPLAY` with XWayland),
  with `sherlock_ui::gpu::for_child`, and no standard input. It is not watched: an application
  that stops is not started again.

## Tests

- `desktop/autostart/src/lib.rs`: the user hiding a system entry, `OnlyShowIn` and `NotShowIn`,
  `X-GNOME-Autostart-enabled`, `TryExec`, `Terminal`, the KDE condition, the exclusions, the
  delays and their bound. Temporary folders only.
- `desktop/autostart/tests/launch.rs`: the program run under `env -i` on temporary folders: the
  entries start a recording script, without a shell, the delayed one a second later; a `KDE`
  entry and a hidden one do not start.
- `desktop/compositor/tests/session.rs`: the session script starts it, and leaves it out with
  `SHERLOCKOS_AUTOSTART=0`.

## Left for later

- A page of Control Room to turn entries on and off (it would write `Hidden=true` in
  `~/.config/autostart`).
- `X-GNOME-AutoRestart`, and the session's own way of restarting what stops.
