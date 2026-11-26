# Media keys

Lot D-35a. In the real session, the compositor takes the keys of the keyboard that change the
sound, the brightness and what a media player plays, whatever window has the keyboard, and shows
what changed in a small pill at the bottom of the screen (the on-screen display).

| Key | What it does | Repeats while held |
| --- | --- | --- |
| `XF86AudioRaiseVolume`, `XF86AudioLowerVolume` | Volume of the default output, by steps of 5 %, from 0 to 100 %; also unmutes it | Yes |
| `XF86AudioMute` | Mutes or unmutes the default output | No |
| `XF86AudioMicMute` | Mutes or unmutes the default microphone | No |
| `XF86MonBrightnessUp`, `XF86MonBrightnessDown` | Brightness of the screen, by steps of 5 %, from 1 to 100 % | Yes |
| `XF86AudioPlay`, `XF86AudioPause`, `XF86AudioNext`, `XF86AudioPrev` | Play or pause, pause, next and previous track of a media player (MPRIS) | No |

A step goes to the next multiple of 5: 52 % goes up to 55 % and down to 50 %. A volume set above
100 % elsewhere is kept by the up key and lowered by the down key. A held key repeats with the
keyboard's delay and pace (200 ms, then 25 times a second, `KEY_REPEAT_DELAY` and
`KEY_REPEAT_RATE` in `src/lib.rs`), until its release, another key, or a console switch.

Nested mode (`--nested`) leaves these keys to the host session, which takes them anyway (KWin
under Plasma): the compositor never changes the sound of the session it runs in.

## How

| File | Role |
| --- | --- |
| `desktop/system-services` | The services, shared with the quick settings of the bar: `audio` (`pactl`, `wpctl`), `backlight` (sysfs read, logind write), `mpris`, and their errors (`Problem`) |
| `desktop/compositor/src/media_keys.rs` | The keys: which keysym is which, the worker thread, the repeat |
| `desktop/compositor/src/osd.rs` | The on-screen display: its state, its springs, its drawing |
| `desktop/sherlock-ui/src/osd.rs` | Its sizes, colors, springs and texts; the icons `Microphone`, `MicrophoneMuted` and `Brightness` |

- **Sound.** `pactl` when it is in `PATH`, else `wpctl` (WirePlumber), the same commands the bar
  uses: PipeWire has no D-Bus interface. Each call is killed after 3 seconds
  (`audio::TIMEOUT`), and the key then does nothing but log a warning.
- **Brightness.** The backlight is read from `/sys/class/backlight` (the firmware one first, as
  the bar does), and written by logind's `Session.SetBrightness` on the system bus, which lets
  the user of the active session change it without root. Nothing is ever written into sysfs.
- **Players.** The command goes, on the session bus, to the MPRIS player that plays, else to one
  that is paused, else to the first one by name. No pill for them.
- **Never in the loop.** The commands and the bus calls run on a thread of their own
  (`media-keys`), one key after the other, in the order pressed. What changed comes back to the
  event loop by a calloop channel and shows in the pill. A slow `pactl` or a bus that does not
  answer never holds a frame back.
- **Nothing piles up.** A held key only repeats when the worker has done the keys before it (a
  flag set from a key sent until the worker is idle again); a press always goes. The steps of
  the volume, or of the brightness, waiting together are merged: their sum is set at once. A
  key held 4 s against a `pactl` that hangs leaves at most one action after its release
  (`tests/media_keys.rs`), instead of a hundred.

## The on-screen display

A pill of menu glass, 232 by 48, round ends, 124 px above the bottom edge of the screen (above
the dock), centered on the screen under the pointer, in front of everything. On its left the
icon of what changed (the sound, muted or not; the microphone; a sun for the brightness), then a
gauge in the accent color, grey when the output is muted. The microphone has no gauge: the pill
says « Micro activé » or « Micro coupé ».

It comes on the spring of a window that opens (opacity, and a rise of 10 px), its gauge follows
the level on a quicker spring, and it goes the same way 1.5 s after the last key
(`osd::VISIBLE`). Another key of the same kind keeps the pill and moves its gauge; a key of
another kind replaces it.

**Compositor overlay, not a layer-shell client.** The compositor already sees the keys; drawing
the pill itself costs one texture and no protocol. A client would need the keys forwarded to it
through the shell protocol, and the pill would vanish whenever that client restarts. The pill is
drawn on the processor (tiny-skia, the icons through resvg) into one texture, made again only
when what it shows changes; at rest it costs nothing (see "Frames of the real session" in
`docs/compositor.md`): the 1.5 s wait is a timer, not frames.

No board draws it: its measures come from the panel and dock tokens, to review with the design.

## Tests

- `cargo test -p system-services`: parsing of `pactl` and `wpctl`, the steps, the choice of a
  player.
- `cargo test -p compositor --test media_keys`: the keys through `State::keyboard_key`, against
  a fake `pactl` that keeps its volume in a file, and a private bus (its own `dbus-daemon`)
  with a fake logind writing a fake sysfs folder and a fake player. Volume up, mute, mute of the
  microphone, a held brightness key that repeats then stops at its release, the play key. Never
  the machine's sound, brightness or buses.
- `cargo test -p compositor osd`: the pill draws its icon and its level.

## Manual test

In the real session (`docs/compositor.md`, "Real session"): each key of the table, held for the
volume and the brightness; the pill on the screen under the pointer; the bar's quick settings
show the same level once their panel opens again.
