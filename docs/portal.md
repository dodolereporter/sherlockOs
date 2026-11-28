# Portal backend: screen sharing (lot D-28d)

Discord (Vesktop), Chrome (Meet), Firefox and Teams share a screen or a window through
`xdg-desktop-portal`. In the SherlockOS session its `ScreenCast` portal goes to our own backend,
`sherlock-portal` (crate `desktop/portal`, ADR 0028): it asks the user what to share in our
picker, captures it from the compositor and pushes the frames into PipeWire, whose node the
frontend hands to the application.

```
application ──D-Bus──▶ xdg-desktop-portal ──D-Bus──▶ sherlock-portal ──spawns──▶ sherlock-portal --pick
     ▲                                                   │  ▲                      (the picker)
     └──────────── PipeWire node ◀── frames ─────────────┘  └── shell socket: ext-image-copy-capture
```

## PipeWire (lot D-28d1)

The crate `desktop/portal` starts with its PipeWire part: `pipewire.rs`, a video source loaded at
run time, and `pod.rs`, the SPA pods of the format and of the buffers, written by hand. The
capture client, the D-Bus backend, the picker and the bar's indicator follow in lots D-28d2 to
D-28d4.

- **PipeWire.** A `Video/Source` node that drives its graph (`PW_STREAM_FLAG_DRIVER`), in its own
  thread loop. It offers raw video in `BGRx` (blue first, as `xrgb8888`), at the source's size,
  a variable rate up to 60 frames per second; the buffers are 2 to 8 in shared memory that
  PipeWire allocates and maps (`MemFd` or `MemPtr`). A frame is dropped when the application
  holds every buffer. The library is loaded with `dlopen` (ADR 0028, amended): nothing to build
  against, no headers, no `bindgen`; without PipeWire, `Start` fails and nothing else does.

Tests: `pod.rs`, the format laid out word by word as SPA writes it, and the buffers padded to 8
bytes. PipeWire itself is not run in CI (no daemon there).
