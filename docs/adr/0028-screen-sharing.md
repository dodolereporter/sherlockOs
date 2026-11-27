# ADR 0028: Screen sharing through a capture protocol of the compositor and a portal of our own

- Status: proposed
- Date: 2026-11-27
- Lot: D-28b

## Context

Discord (Vesktop), Chrome (Meet) and Teams must be able to share a screen or a window. Under
Wayland they all go through `xdg-desktop-portal`: its `ScreenCast` interface asks a backend
which source to share, and the backend pushes the frames into a PipeWire stream that the
application reads. The session already routes the portals through `sherlockos-portals.conf`
(lot D-35b): GTK's backend for everything, no screen capture, since Smithay 0.7 offers no
capture protocol. The author wants:

- a whole screen or one window, chosen in a picker of our style (glass, thumbnails, animated in
  and out, French, « tu »);
- a visible indicator while something is shared, as on macOS, with « Arrêter le partage »;
- nothing shared while the session is locked, the locked content never exposed;
- 30 to 60 frames per second at 1080p without the desktop stuttering, the cursor included or
  given as metadata.

Two ways were open:

- **(a) `xdg-desktop-portal-wlr`.** The compositor implements `wlr-screencopy`, or
  `ext-image-copy-capture-v1` with its capture sources, and the wlroots portal backend does the
  rest. Debian 13 packages version 0.7.1 only: it speaks `wlr-screencopy` alone, so screens
  only, no window (window capture through `ext-foreign-toplevel-image-capture-source-v1` came in
  0.8, which Debian 13 does not have), and its picker is an external menu command given the
  screen names (`slurp`, `wofi`). It knows nothing of the lock nor of an indicator.
- **(b) A portal backend of our own**, in Rust, that implements
  `org.freedesktop.impl.portal.ScreenCast`, shows our picker and pushes into PipeWire.

Either way the compositor needs a capture protocol: the frames come from it.

## Decision

1. **The compositor implements the standard capture protocols**, written against
   `wayland-protocols` 0.32 (Smithay 0.7 has no helper for them):
   `ext-image-copy-capture-v1`, with `ext-output-image-capture-source-v1` for a screen and,
   with `ext-foreign-toplevel-list-v1`, `ext-foreign-toplevel-image-capture-source-v1` for a
   window. Frames go into the client's buffers: shared memory first (`xrgb8888`, `argb8888`),
   dmabuf next.
2. **Only the portal sees these globals**: a capture protocol shows every pixel of the
   session. They are offered on the shell socket only, like `sherlock_shell_v1` (ADR 0021), so
   an ordinary client, a Flatpak application or XWayland cannot capture anything; and on that
   socket only to the program `sherlock-portal`, recognised when it connects by the executable
   of the process at the other end (`SO_PEERCRED`, `/proc/<pid>/exe`), not to the bar or the
   dock. Applications go through the portal, which asks the user. (Amended in lot D-28c: a
   program of the same user can still reach any socket of the session; this check keeps the
   capture out of the shell programs and out of reach by mistake, it is not a sandbox.)
3. **The portal backend is ours**: `sherlock-portal`, a Rust program on the session bus
   (`zbus`), client of the shell socket, that implements `ScreenCast` (and later
   `Screenshot`), shows the picker (iced, the shell's glass) and pushes the frames into
   PipeWire (`pipewire-rs`). `sherlockos-portals.conf` gives it `ScreenCast`.
   (Amended in lot D-28d: PipeWire is loaded at run time with `dlopen`, through the few calls
   a video source needs, rather than built against with `pipewire-rs`: no
   `libpipewire-0.3-dev` nor `libclang-dev` on the machine or in the CI image, and the portal
   still starts on a machine without PipeWire.)
4. **The compositor holds the policy**, whoever the client is: it knows every capture session,
   so it tells the bar that something is shared (the dot and « Arrêter le partage » go
   through `sherlock_shell_v1`), it stops a session on request (`stopped`), and while the
   session is locked it never draws a captured source: every frame is black.

## Consequences

- Window sharing works on Debian 13, with our picker, and the indicator and the lock hold
  even for a capture client that is not our portal.
- Being standard, the protocol lets a later `xdg-desktop-portal-wlr` 0.8 or OBS's capture plug
  in through the shell socket, should we want them.
- PipeWire is only needed at run time (`libpipewire-0.3.so.0`, there wherever PipeWire runs):
  the layouts of the few structures used are written in `desktop/portal/src/pipewire.rs`,
  stable since PipeWire 0.3.
- The capture draws the source again offscreen and reads it back for each frame, in the
  compositor's thread. That costs a render and a copy per frame. Dmabuf buffers, filled by the
  GPU without the copy, are the next step for 1080p at 60 frames per second.
- The work goes in lots of about 600 lines: D-28b the capture of a screen (shared memory,
  lock, shell socket); D-28c the capture of a window (foreign toplevel list) and the cursor;
  D-28d the portal backend and PipeWire; D-28e the picker, the indicator and « Arrêter le
  partage ».

## What would change our mind

- Debian packaging an `xdg-desktop-portal-wlr` that captures windows and accepts a picker
  program: its PipeWire part could then replace ours, the picker and the policy staying.
- Smithay shipping these protocols: our implementation would give way to its helpers.
- The offscreen render costing frames to the desktop at 1080p: capture from the frame the
  screen just showed instead of drawing it again.
