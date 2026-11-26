# Media keys

Lots D-35a and D-35d. In the real session, the compositor takes the keys of the keyboard that
change the sound, the brightness of the screen and of the keyboard, and what a media player plays, whatever window has the keyboard, and shows
what changed in a small pill at the bottom of the screen (the on-screen display).

| Key | What it does | Repeats while held |
| --- | --- | --- |
| `XF86AudioRaiseVolume`, `XF86AudioLowerVolume` | Volume of the default output, by steps of 5 %, from 0 to 100 %; also unmutes it | Yes |
| `XF86AudioMute` | Mutes or unmutes the default output | No |
| `XF86AudioMicMute` | Mutes or unmutes the default microphone | No |
| `XF86MonBrightnessUp`, `XF86MonBrightnessDown` | Brightness of the screen, by steps of 5 %, from 1 to 100 % | Yes |
| `XF86KbdBrightnessUp`, `XF86KbdBrightnessDown` | Keyboard backlight, by the levels of the machine, at most 10 from off to full | Yes |
| `XF86KbdLightOnOff` | Turns the keyboard backlight off, or back on where it was (half its levels the first time) | No |
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
| `desktop/system-services` | The services, shared with the quick settings of the bar: `audio` (`pactl`, `wpctl`), `backlight` (sysfs read, logind write), `keyboard` (UPower, else logind), `mpris`, and their errors (`Problem`) |
| `desktop/compositor/src/media_keys.rs` | The keys: which keysym is which, the worker thread, the repeat |
| `desktop/compositor/src/osd.rs` | The on-screen display: its state, its springs, its drawing |
| `desktop/sherlock-ui/src/osd.rs` | Its sizes, colors, springs and texts; the icons `Microphone`, `MicrophoneMuted`, `Brightness` and `KeyboardBrightness` |

- **Sound.** `pactl` when it is in `PATH`, else `wpctl` (WirePlumber), the same commands the bar
  uses: PipeWire has no D-Bus interface. Each call is killed after 3 seconds
  (`audio::TIMEOUT`), and the key then does nothing but log a warning.
- **Brightness.** The backlight is read from `/sys/class/backlight` (the firmware one first, as
  the bar does), and written by logind's `Session.SetBrightness` on the system bus, which lets
  the user of the active session change it without root. Nothing is ever written into sysfs.
- **Keyboard backlight.** UPower's `org.freedesktop.UPower.KbdBacklight` (`GetMaxBrightness`,
  `GetBrightness`, `SetBrightness`) on the system bus. Without UPower, the LED whose name ends in
  `::kbd_backlight` is read from `/sys/class/leds` and set by logind's
  `SetBrightness("leds", name, value)`. The steps follow the machine: a backlight of 3 levels
  steps by one, one of 255 by tenths (`keyboard::LEVELS`). The calls share the 5 s limit of the
  bus (`BUS_TIMEOUT`).
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

A pill of menu glass with round ends, centered on the screen under the pointer, above the dock,
in front of everything. On its left the icon of what changed (the sound, muted or not; the
microphone; a sun for the screen; a lit keyboard), then a gauge in the accent color, grey when
the output is muted. The microphone has no gauge: the pill says « Micro activé » or « Micro
coupé ».

**Its size and place follow the screen** (`osd::fit`, `osd::bottom`). On a screen 1920 logical
pixels wide it measures 232 by 48; elsewhere the pill and its content take the same share of
the width, between 200 and 300 logical pixels, so a small laptop keeps a readable pill and a
wide screen does not get a tiny one. Its bottom sits 40 pixels (at its scale) above the dock:
above what the dock, or anything anchored at the bottom, reserves when it reserves more than
the dock of the default size, and never higher than 40 % of the screen. Everything is in
logical pixels, so a 4K screen at scale 2 shows the pill of a 1920 screen, drawn sharp.

| Screen | Scale | Pill (logical) |
| --- | --- | --- |
| 1366 × 768 | 1 | 200 × 41 |
| 1920 × 1080 | 1 | 232 × 48 |
| 2560 × 1600 | 1.5 | 206 × 43 (1707 logical wide) |
| 3840 × 2160 | 2 | 232 × 48 |

**Its motion**, as on macOS and iOS:

- It comes by growing from 0.9 to 1 around its center while it fades in, on the curve and the
  250 ms of a window that opens (`osd::ENTER`), and it goes the other way, 180 ms, 1.5 s after the
  last key (`osd::VISIBLE`, `osd::LEAVE`).
- Its gauge slides to the new level on a spring (`osd::GAUGE_SPRING`), never jumps. Another key
  of the same kind keeps the pill, even while it goes; a key of another kind replaces it.
- A key that insists at the end, « plus » at the top or « moins » at the bottom, stretches the
  pill towards its side by about 5 % of its length (`osd::STRETCH`), a little thinner, the
  gauge pulled with it; it is let go after 120 ms and comes back on a spring, a little past its
  rest. Each press aims at the same stretch again from where the pill is: a held key keeps it
  stretched, never longer (`osd::STRETCH_SPRING`, `osd::STRETCH_HOLD`).

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
  with a fake logind writing a fake sysfs folder, a fake UPower and a fake player. Volume up, mute, mute of the
  microphone, a held brightness key that repeats then stops at its release, the play key. Never
  the machine's sound, brightness or buses.
- `cargo test -p compositor --lib osd`: the pill draws its icon and its level; it stretches at
  the top and at the bottom, towards the key; presses that follow each other do not add up; the
  gauge slides; it grows in and shrinks out; its place and size on several screens; no frame is
  asked for at rest.
- `cargo test -p compositor --test media_keys`: also the keyboard backlight through a fake
  UPower, and through logind without it; a volume key that insists at an end stretches the pill;
  once the pill is gone, nothing is drawn.
- `cargo test -p compositor --test osd`: the boards, in `target/tmp`: `osd-stretch.png`,
  `osd-gauge.png`, `osd-enter-leave.png`, and `osd-screen-*.png` for 1366 × 768, 1920 × 1080,
  2560 × 1600 at 1.5 and 3840 × 2160 at 2.

## Manual test

In the real session (`docs/compositor.md`, "Real session"): each key of the table, held for the
volume and the brightnesses; held again at 100 % and at 0 %, the pill stretches then comes back;
the pill on the screen under the pointer, on each screen; the bar's quick settings
show the same level once their panel opens again.
