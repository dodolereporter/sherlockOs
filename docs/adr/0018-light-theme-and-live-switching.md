# ADR 0018: Light theme, automatic mode and live switching

- Status: proposed
- Date: 2026-11-25
- Lot: D-05

## Context

The author supplied a light theme (`docs/design/THEME-CLAIR.md` and the sixteen boards of
`docs/design/planches-clair/`). Both themes share every size and spacing: only colors change.
The author wants to switch by hand or with the time of day, and every program of the desktop
(compositor, bar, dock, Evidence, and the applications to come) must follow without restarting.
Until now, `sherlock-ui` held one color per role, as constants.

## Decision

- **Model.** `sherlock_ui::color::Palette` holds every color of a theme, one field per role;
  `Palette::DARK` and `Palette::LIGHT` hold the values of the two tables. `color::Theme` is
  `Dark` or `Light`. No component tests the theme: it reads the palette.
- **Active theme.** Each program has one active theme (`color::set_active`, `color::palette`).
  The iced styles read the palette of the iced theme they are given (`theme::palette`), and
  `theme::theme()` builds that iced theme from the active one: a program that changes the
  active theme redraws, and everything follows.
- **Setting.** A key file, `$XDG_CONFIG_HOME/appearance/appearance.conf`, group `[Appearance]`:
  `Theme=dark|light|auto`, `AutoLightFrom=07:00`, `AutoDarkFrom=19:00`. Read by the
  `appearance-config` crate (standard library and the tokens of `sherlock-ui` only), written
  later by Control Room (D-26).
- **Automatic mode.** Fixed hours of the local time, from the `date` command. No position data:
  sunrise and sunset are not used.
- **Propagation.** Each program follows the file itself (`appearance_config::follow`): inotify
  on its directory for edits, and a timer that sleeps until the next switch of the automatic
  mode. No polling, no daemon, no bus: the file is the only shared state.

## Consequences

- Each program adds a subscription and a call to `color::set_active` (D-05b for the bar and
  Evidence, D-05c for the compositor and the dock).
- The module constants of `color` are the dark values, kept until every program reads the
  palette; D-05c removes them.
- Every program reads the file and the clock by itself: two programs may switch a few
  milliseconds apart.
- The timer counts monotonic time: after a suspend that runs past a switch time, the theme
  changes only at the next event (edit of the file, or the following switch).

## What would change our mind

- A settings daemon or a D-Bus portal (`org.freedesktop.appearance.color-scheme`) becomes
  needed, for instance for GTK and Qt applications: the file would then feed it.
- The author wants sunrise and sunset: that needs a position, hence a privacy decision.
- Switches missed after a suspend turn out to matter: listen to logind's `PrepareForSleep`.
