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

## Pieces

| File | What it does |
| --- | --- |
| `screencast.rs` | `org.freedesktop.impl.portal.ScreenCast` version 5 and its sessions (`org.freedesktop.impl.portal.Session`); `org.sherlockos.ScreenShare1` for the bar |
| `picker.rs` | The request written to the picker and its answer, bounded and checked |
| `capture.rs` | The capture client on the compositor's shell socket: screens, windows, frames |
| `pipewire.rs` | A PipeWire video source, `libpipewire-0.3.so.0` loaded at run time |
| `pod.rs` | The SPA pods of the format and of the buffers, written by hand |

## The calls

- **`CreateSession`** puts a `Session` object at the path the frontend gives; `Close` on it,
  or the application leaving (the frontend closes the session then), stops its shares.
- **`SelectSources`** keeps what the application asks for: screens (`MONITOR`) and windows
  (`WINDOW`) only, there is no virtual screen; one source or several (`multiple`); the cursor
  hidden or drawn into the frames (`cursor_mode`: `HIDDEN` or `EMBEDDED`; `METADATA` is not
  offered and counts as drawn). `persist_mode` and `restore_data` are not kept: the picker asks
  each time.
- **`Start`** lists the screens and the windows from the compositor, starts the picker and
  waits for the user, off the bus's thread. Cancelled: answer 1. Chosen: one PipeWire stream per
  source, each answered with its node, `source_type` and `size`.

## Capture and PipeWire

- **Access.** The compositor offers the capture protocols to the program `sherlock-portal` alone
  (ADR 0028, `docs/compositor.md`, "Screen capture"); the picker is the same program.
- **Frames.** Each share has a thread and a connection of its own: a capture session on its
  source (`paint_cursors` if the cursor is wanted), a `xrgb8888` buffer in a memfd, then one
  frame after another, each copied into a PipeWire buffer. The compositor gives a frame when the
  source shows a new picture, at most once per vertical blank: a still screen sends nothing. The
  thread waits 100 ms at most at a time, so a stop is seen at once. A source that changes size
  gives new constraints: a new buffer, and the format is offered again to PipeWire. A source
  gone (`stopped`), or PipeWire that stops reading, ends the share; within a second the session
  ends (`reap`), the frontend tells the application, and the bar's indicator goes.
- **PipeWire.** A `Video/Source` node that drives its graph (`PW_STREAM_FLAG_DRIVER`), in its own
  thread loop. It offers raw video in `BGRx` (blue first, as `xrgb8888`), at the source's size,
  a variable rate up to 60 frames per second; the buffers are 2 to 8 in shared memory that
  PipeWire allocates and maps (`MemFd` or `MemPtr`). A frame is dropped when the application
  holds every buffer. The library is loaded with `dlopen` (ADR 0028, amended): nothing to build
  against, no headers, no `bindgen`; without PipeWire, `Start` fails and nothing else does.
- **Bounds.** Sizes beyond 16384 pixels a side are refused; the picker's answer is read up to
  4 KiB and must name sources offered; the request's fields are cut at 512 characters, without
  tabs nor line ends.

## The picker and the bar (lot D-28d4)

`sherlock-portal --pick` reads the request on its standard input and writes the numbers of the
sources chosen on its standard output, nothing to cancel (`picker.rs`). Its window comes with lot
D-28d4, as do the bar's indicator, which follows `org.sherlockos.ScreenShare1` (signal
`Changed(as)`, methods `Sharing` and `StopAll`, at `/org/sherlockos/ScreenShare`), and the files
that make `xdg-desktop-portal` use this backend. Until then the backend is not registered, and
`--pick` cancels every request.

## Tests

- `pod.rs`: the format laid out word by word as SPA writes it; the buffers padded to 8 bytes.
- `picker.rs`: a request reads back without tabs nor line ends; the answer chooses among what
  is offered, one unless several are allowed.
- `screencast.rs`: the options keep what can be shared; a stream says its kind and size.
- Not tested in CI: PipeWire itself (no daemon there) and the bus with `xdg-desktop-portal`;
  see the pull requests for the trial on the machine.
