# ADR 0025: display configuration through wlr-output-management

- Status: accepted (the author approved it on 2026-11-27)
- Date: 2026-11-27
- Lot: D-26a3 (decision), D-26b (work)

## Context

Control Room needs a « Écrans » page: the list of screens, resolution, refresh rate, scale
(100 to 200 %), main screen, arrangement, and « Dupliquer » or « Étendre », applied live and
kept across restarts. Super+P (D-39) needs the same operations and remembers a choice per screen.

What the compositor does today (main, after D-25a):

- each screen takes its preferred mode (`udev.rs`, `add_head`); nothing changes it afterwards;
- the scale comes from the command line (`options.scale`), the same for every new screen;
- screens are laid side by side in the order they were plugged in (`output.rs`,
  `place_screens`); there is no main screen and no mirroring;
- no file describes the screens, and no protocol lets a client read or change them.

Smithay 0.7 has no output-management module. `wayland-protocols-wlr` 0.3.12, already in the
lock file, has the bindings of `wlr-output-management-unstable-v1`.

Two ways were weighed:

1. **A file read again live**, `$XDG_CONFIG_HOME/sherlockos/displays.conf`, like the touchpad and
   the wallpaper. Simple to write, but Control Room still cannot list the screens and their
   modes (`wl_output` only gives the current mode), cannot test a configuration before
   applying it, and nothing else on Linux speaks it.
2. **`wlr-output-management-unstable-v1`** in the compositor. It lists every head with its
   modes (size and refresh), position, scale, transform and whether it is on; a client sends a
   whole configuration, tests it or applies it, and is told whether it succeeded. `wlr-randr`,
   `kanshi`, `wdisplays` and `nwg-displays` already speak it.

## Decision

The compositor implements `wlr-output-management-unstable-v1` (lot D-26b): heads, modes, test
and apply, mode changes on the DRM side (`DrmCompositor::use_mode`), per-screen scale, position,
on or off, and mirroring of a screen onto another as a SherlockOS extension of the arrangement
(a mirrored head is placed at the position of the one it copies). It saves each applied
configuration in `$XDG_CONFIG_HOME/sherlockos/displays.conf`, keyed by the screen's make, model
and serial number (EDID), and applies the saved one when that screen comes back. The main screen
is the one at the origin; the bar and the notifications follow it.

Control Room's « Écrans » page is a client of that protocol (lot D-26a3, after D-26b), with a
confirmation that reverts after 15 seconds without an answer, as on macOS.

## Consequences

- The page shows what the compositor really drives, and a configuration the hardware refuses is
  refused before anything changes.
- Other tools work too: `wlr-randr` to debug, `kanshi` for profiles.
- D-39 (Super+P) uses the same code in the compositor and the same file.
- The compositor gains the protocol (about 600 lines) and the mode change on DRM; on the
  reference laptop the HDMI port is wired to the NVIDIA card (D-16), so mirroring and extending
  onto it wait for D-16.
- Control Room gains a Wayland client connection of its own (`wayland-client`,
  `wayland-protocols-wlr`), besides the one of its window.

## What would change our mind

- A standard protocol for output configuration agreed upon in `wayland-protocols` (the
  `ext-` namespace): we would follow it.
- If the DRM mode change proves unreliable on the reference laptop, the file alone (option 1)
  with a restart of the session could be a first step.
