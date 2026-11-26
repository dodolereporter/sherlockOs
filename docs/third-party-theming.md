# Third-party theming

`desktop/toolkit-theme` gives the colors of the desktop to the applications that draw their own
interface: GTK 3, GTK 4, Qt 5, Qt 6 and KDE. Every file is generated from the palettes of
sherlock-ui (`Palette::DARK` and `Palette::LIGHT` in `desktop/sherlock-ui/src/color.rs`), in a
dark and a light variant. The NOM_OS session follows the active theme of `appearance.conf`
(see `docs/appearance.md`). Plasma sees none of it.

```text
just themes [folder]     # writes every file under target/themes, laid out like a home folder
just themes folder --force  # the same, over files already there (refused without it)
just install-themes      # installs them for the user, without root
just uninstall-themes    # takes them away and puts the backups back
```

## What is installed

| File | Read by |
| --- | --- |
| `~/.local/share/themes/NOM_OS-{dark,light}/gtk-3.0/gtk.css` | GTK 3, when the session sets `gtk-theme` |
| `~/.local/share/themes/NOM_OS-{dark,light}/gtk-4.0/gtk.css` | GTK 4 without libadwaita, the same way |
| `~/.local/share/color-schemes/NOMOS{Dark,Light}.colors` | KDE: listed among the color schemes, applied nowhere |
| `~/.local/share/nom-os/themes/qt5ct/{dark,light}.conf` | qt5ct, palette of 21 roles |
| `~/.local/share/nom-os/themes/qt6ct/{dark,light}.conf` | qt6ct, palette of 22 roles (`Accent` included) |
| `~/.config/qt5ct/qt5ct.conf`, `~/.config/qt6ct/qt6ct.conf` | qt5ct and qt6ct: the palette, the Fusion style |
| `~/.local/share/nom-os/themes/libadwaita/{dark,light}.css` | Nothing by default (see "libadwaita") |
| `~/.local/share/nom-os/themes/dconf-profile` | dconf, in the NOM_OS session only |
| `~/.local/share/nom-os/themes/toolkit-theme` | The session, which runs it to follow the theme |
| `~/.local/share/nom-os/themes/manifest` | `uninstall`: the installed files, their backups, and the folders created for them |

The GTK theme imports a base theme, defines the named colors, then paints the main surfaces
with them: the window, the header bar, views and lists, the sidebar, fields and buttons, the
suggested and destructive actions, the selection, switches and checks, popovers and menus.
The base theme is adw-gtk3 when it is installed (`~/.local/share/themes`, `~/.themes`, then
`/usr/share/themes`), and otherwise the Adwaita theme built into GTK 3 (`Default` for GTK 4).
The built-in themes write their colors as literals, which is why the surfaces are painted
again. adw-gtk3 reads the libadwaita named colors itself.

The files under `~/.config/gtk-3.0` and `~/.config/gtk-4.0`, which Plasma writes, are not
touched. Neither are `kdeglobals` or `~/.config/dconf/user`.

## Colors

| Role | GTK name | Qt role | KDE | Palette |
| --- | --- | --- | --- | --- |
| Window | `window_bg_color`, `view_bg_color` | `Base` | `View` background | `window` |
| Side panel | `sidebar_bg_color` | `Window` | `Window` background | `sidebar` |
| Title bar | `headerbar_bg_color` | | `Header`, `[WM]` | `title_bar` |
| Card, field, button | `card_bg_color`, `dialog_bg_color` | `Button`, `AlternateBase` | `Button` background | `card` |
| Text | `*_fg_color` | `WindowText`, `Text`, `ButtonText` | `ForegroundNormal` | `text_primary` |
| Text of disabled elements | `insensitive_fg_color` | `Text` (disabled group) | `ForegroundInactive` | `text_tertiary` |
| Green | `accent_bg_color`, `accent_color` | `Accent` | `ForegroundPositive` | `accent`, `accent_text` |
| Selection | `theme_selected_bg_color` | `Highlight` | `Selection` | `selected_row` over `window` |
| Popover, menu, tooltip | `popover_bg_color` | `ToolTipBase` | `Tooltip` | `menu_glass` over `window` |
| Links | `link_color` | `Link`, `LinkVisited` | `ForegroundLink` | `blue_light` |
| Error, warning | `destructive_*`, `error_*`, `warning_*` | | `ForegroundNegative`, `ForegroundNeutral` | `salmon`, `amber` |

The toolkits have no glass, and Qt and KDE take no opacity: a veil of the palette is laid on
the window first (`flat`). The bevel roles of Qt (`Light`, `Midlight`, `Mid`, `Dark`,
`Shadow`) are the chip, border and shadow veils, laid the same way.

## The session

When `dconf-profile` and the program are installed, `sherlockos-session` exports:

| Variable | Why |
| --- | --- |
| `DCONF_PROFILE` | The GTK settings of the session live in a dconf database of their own, `nom_os`, written first; the user's database, `user`, is read after it and never written. The GTK theme and the color scheme set in the session never reach Plasma |
| `ADW_DISABLE_PORTAL=1` | Without it, libadwaita asks the portal for the color scheme, and `xdg-desktop-portal` answers with Plasma's (its KDE backend is the fallback for an unknown desktop). With it, libadwaita reads the session's dconf database |
| `QT_QPA_PLATFORMTHEME=qt6ct` | Qt 6 takes its palette and style from qt6ct |

The session then runs `toolkit-theme follow`, like the bar and the dock (restarted when it
stops). It sets the theme of the appearance file at start, then at every change:

- `gtk-theme` becomes `NOM_OS-dark` or `NOM_OS-light`, and `color-scheme` becomes `prefer-dark`
  or `default`, with `gsettings`. The program refuses to call `gsettings` when `DCONF_PROFILE`
  is not the installed profile: run by hand under Plasma, it never writes the user's database.
- the `color_scheme_path` line of `qt5ct.conf` and `qt6ct.conf` names the palette of the theme.
  The file is replaced through a rename, and every other line is kept. A settings file that
  `install` did not write is left alone.

`toolkit-theme apply [dark|light]` does the same once.

### Switching between dark and light

| Applications | Switch |
| --- | --- |
| GTK 3 | Live: GTK reloads its theme when `gtk-theme` changes |
| GTK 4 without libadwaita | Live, the same way |
| libadwaita | Live between its own dark and light styles (`color-scheme`) |
| Qt through qt6ct | qt6ct watches its settings folder and reads the palette again a few seconds later |
| KDE applications | Their palette follows qt6ct; the colors they read through `KColorScheme` come from `kdeglobals` (see "Limits") |
| Firefox, Zen | GTK widgets follow the GTK theme; the dark mode of web pages is to be checked |

Checked without a screen: the GTK 3 and GTK 4 themes as rendered, and the live switch of
libadwaita through the session's database. Not checked: the reload of GTK under Wayland when
`gtk-theme` changes, which Broadway cannot show, and that of qt6ct, which is not installed on
the machine. Both are to be checked in the session.

## libadwaita

libadwaita applications (GNOME applications, Nautilus) load their own stylesheet. The only way
to change their colors is `~/.config/gtk-4.0/gtk.css`, which Plasma writes too, and which
applies under Plasma as well. `GTK_THEME` is no answer either: libadwaita then drops its own
stylesheet. So in the session, libadwaita applications keep the colors of Adwaita and follow
dark and light, live.

`libadwaita/{dark,light}.css` hold our colors, as `@define-color` and as the CSS variables of
libadwaita 1.6 and later (`--window-bg-color`…). Imported at the end of
`~/.config/gtk-4.0/gtk.css`, they recolor libadwaita (checked without a screen with libadwaita
1.7), but in every session, Plasma included, and in a single variant: GTK 4.18 ignores
`@media (prefers-color-scheme)` in that file and does not reload it while an application runs.
That choice is the author's.

## What is not covered

- **Electron** applications (VS Code, Discord…) draw with Chromium. Some take the GTK theme
  for their title bar or follow the portal's color scheme, which in our session is Plasma's.
- Applications that impose their own theme: Steam, JetBrains IDEs, Blender, LibreOffice
  partly, and Flatpak applications, which read neither `~/.local/share/themes` nor our dconf
  database.
- **Kvantum**: a Kvantum theme is SVG artwork plus a configuration file, more than a palette.
  It is a follow-up.
- The terminal colors of Konsole: a Konsole `.colorscheme` is a follow-up.
- Icons, cursors and fonts stay those of the user's database.
- Qt 5 applications: the session points `QT_QPA_PLATFORMTHEME` at qt6ct, and Qt 5 looks for a
  plugin of that name. They take the qt5ct settings only when started with
  `QT_QPA_PLATFORMTHEME=qt5ct`.

## Limits

- KDE applications read part of their colors through `KColorScheme`, from `kdeglobals`, and so
  from Plasma's scheme. Most KDE applications can be given `NOMOSDark` from their own color
  scheme menu, but that choice is kept in their own settings file, under Plasma too.
- The `nom_os` dconf database also holds every GTK setting changed while in the session (the
  preferences of Nautilus, the size of a window, for instance): they do not reach Plasma, and
  Plasma's later changes show in the session only where the session never set the key.
- An application started by D-Bus activation (`DBusActivatable=true` in its `.desktop` file)
  is started by the bus, which does not have `DCONF_PROFILE`: the session does not import its
  environment into the bus. Such an application reads and writes the user's database, `user`,
  and does not follow the theme of the session.
- An adw-gtk3 installed after `just install-themes` is used after running it again.
- `/etc/dconf/profile/user`, when it exists, may name system databases: the session's profile
  does not read them.

## Going back

`just uninstall-themes`. Each installed file is deleted, and the file it had replaced, kept as
`<name>.bak-<date>`, takes its place again. A symbolic link is kept as it is, even one that
leads nowhere (dotfiles managed with stow, for instance), and comes back the same. A file of ours that was changed since (the qt6ct
settings edited with qt6ct, for instance) is not deleted: it is kept as
`<name>.bak-<date>` too. The folders that installing created go once empty, and only those.
The manifest is written after each file: when the removal stops on an error, running it again
goes on where it stopped. Installing writes it before each file, so that a backup is always
listed, whatever stops the installation. Without the profile and
the program, the session exports nothing and runs no follower. The session's dconf database,
`~/.config/dconf/nom_os`, is left in place: delete it to forget the GTK settings of the session.

`just install-themes` run again rewrites only what changed and keeps the first backups.

## Packages

To install on Debian for everything to apply: `qt6ct`, `qt5ct` (Qt 5 applications), and
`adw-gtk3` for GTK 3 (the package or the theme from its project page). Without them, Qt keeps
its default look, and GTK 3 is painted over its built-in Adwaita. Kvantum is not needed.

## Follow-ups

- Our own portal, with `org.freedesktop.appearance color-scheme`: libadwaita, Firefox and
  Electron will read the theme there, and `ADW_DISABLE_PORTAL` can go.
- Kvantum, a Konsole color scheme, `kdeglobals` scoped to the session.
- GTK 4.20 reads `@media (prefers-color-scheme)` in `gtk.css`: a single file could then hold
  both variants of the libadwaita colors.

## Tests

`cargo test -p toolkit-theme`:

- every stylesheet passes a CSS reader (imports, `@define-color`, rules and declarations, no
  named color used before it is defined) and defines exactly the named colors, with the values
  of the palettes; the libadwaita variables are the same colors;
- the qt5ct and qt6ct palettes have 21 and 22 colors per group, in `#ffRRGGBB`, with the window,
  base, button and text roles of the palettes;
- the KDE schemes have every group, twelve `r,g,b` keys each, and the palette's values;
- switching the qt6ct palette keeps the other lines;
- in a temporary home: installing backs up the user's qt6ct settings and leaves the GTK files
  of Plasma alone; removing gives the home back exactly as it was; installing twice keeps the
  first backup; a file changed since is kept on reinstall and on removal; a taken backup name
  gets a number; a symbolic link that leads nowhere is kept and comes back; a removal stopped
  halfway and run again puts every file back in place; the folders created go, those that
  were there stay; the GTK settings are left alone outside the session;
- `generate` refuses a folder where a file is already there, unless `force` is given.

The captures in `desktop/toolkit-theme/screenshots/` were taken without a screen, in a
temporary home on a private D-Bus: GTK 3 and GTK 4 through Broadway with the generated themes,
libadwaita with the opt-in colors, Qt 6 with `QT_QPA_PLATFORM=offscreen`, once with the qt6ct
palette read as qt6ct reads it (Fusion), once with the KDE scheme as `kdeglobals` through the
Plasma platform theme.
