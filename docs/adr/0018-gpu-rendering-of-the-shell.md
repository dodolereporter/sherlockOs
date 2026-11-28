# ADR 0018: GPU rendering of the shell on the integrated chip

- Status: proposed
- Date: 2026-11-25
- Lot: D-02

## Context

ADR 0014 chose iced 0.14 for the shell and imposed software rendering (tiny-skia) on its
surfaces, because iced's GPU renderer (wgpu), left alone, asked every Vulkan driver for its
adapters and woke the NVIDIA card of the reference laptop, or crashed in a nested KWin. Two facts
came since:

- Evidence maximised scrolls at about 50 frames per second on a 1920 × 1080 screen, and the
  profile puts almost all the time in tiny-skia (`docs/evidence.md`, Performance).
- tiny-skia presents through softbuffer 0.4.8, whose Wayland backend only creates `XRGB8888`
  buffers: every iced surface is opaque. The rounded corners and the translucent glass of the
  bar come out black (`docs/compositor.md`, layer-shell toolkit).

What constrains the choice: the NVIDIA card must stay asleep outside games (`runtime_status`
`suspended`); nothing is written for this laptop in particular, the OS must start on a machine
without an AMD chip; iced 0.14 only lets a program choose its renderer and its adapter through
environment variables (`ICED_BACKEND`, `WGPU_BACKEND`, `WGPU_POWER_PREF`, and the Vulkan loader's
own variables), read when the first surface is created.

## Decision

The iced programs of the shell (bar, Evidence, dock) render with wgpu on Vulkan,
through Mesa's driver for the AMD chip that drives the screen, and that driver only. Every iced
program of the workspace (the dock and the gallery included) decides at the very start of
`main`, before any thread exists, with `sherlock_ui::gpu::Choice`; a test of sherlock-ui fails
if a program of the workspace that draws with iced does not:

1. It reads `/sys/class/drm/card*/device` (vendor, kernel driver, `boot_vga`). These are values
   kept by the kernel: reading them never wakes a card. (`lspci` does: it reads the PCI
   configuration space. It woke the NVIDIA card during this lot.)
2. If a single chip is bound to `amdgpu`, an AMD chip that drives the screen (`boot_vga`), and a
   manifest of Mesa's Vulkan driver (`radeon_icd*.json`, 32-bit ones left out by name) exists in
   the loader's folders, it sets `VK_DRIVER_FILES` to those manifests only,
   `WGPU_BACKEND=vulkan`, `WGPU_POWER_PREF=low` and `ICED_BACKEND=wgpu,tiny-skia`. The Vulkan
   loader never opens NVIDIA's driver, nor nouveau's (NVK), nor the software one. It also sets
   Mesa's `vk_wsi_force_bgra8_unorm_first=true`: iced takes the first format the driver lists
   without sRGB, and on Wayland Mesa lists the 10-bit ones first (`AB30`, 2 bits of alpha), which
   would turn the bar's 0.78 glass into 0.67. With it, the buffers are `AR24` (8-bit alpha).
   And it unsets what would bring another GPU back in: `VK_ADD_DRIVER_FILES` (drivers added to
   the list), `__NV_PRIME_RENDER_OFFLOAD` (which enables NVIDIA's implicit layer) and
   `DRI_PRIME`.
3. Otherwise it sets `ICED_BACKEND=tiny-skia`, and wgpu is never started: no GPU (CI), NVIDIA
   alone, an Intel chip not tried yet, and a dedicated AMD card, which would be woken up and
   kept awake like the NVIDIA card: an Intel laptop with a Radeon card (the AMD chip is not
   `boot_vga`), an AMD APU with a Radeon card (two `amdgpu` chips: RADV would open both).
4. If the user set `ICED_BACKEND` without `wgpu`, nothing is changed. With `wgpu`, the user's
   value is kept and the Vulkan restriction of point 2 still applies; where the GPU cannot be
   narrowed down that way, nothing is changed, and wgpu may then wake a dedicated card.

If wgpu still finds no adapter that can present to the compositor, iced falls back to tiny-skia
by itself (`ICED_BACKEND=wgpu,tiny-skia`). The commands a program starts (applications opened by
Evidence, the bar's monitors) get the environment the program was started with
(`gpu::for_child`), unset variables included, so that a game launched from the shell is not
bound to the AMD chip.

Only wgpu's Vulkan backend is compiled (iced's `wgpu-bare` feature and a direct `wgpu`
dependency with `vulkan` alone): no OpenGL backend, so no EGL vendor library can be loaded,
even by a program that would forget the choice.

Shadows follow the renderer: on the GPU, menus and panels get the blurred shadow of the design
boards (`0 22px 54px`, `0 28px 70px`); in software they keep the crisp stepped shadow of D-27,
because tiny-skia draws blurred shadows outside the clip of the redrawn area.

ADR 0014 stays valid for the choice of iced and of `iced_exwlshell`. Only its rendering rule
("software rendering until a measurement shows the GPU is needed") is replaced by this one.

### Why Vulkan restricted by manifest, and not OpenGL

wgpu's OpenGL backend goes through EGL. On Debian, libglvnd loads every EGL vendor library and
NVIDIA's comes first (`10_nvidia.json`); keeping it out takes `__EGL_VENDOR_LIBRARY_FILENAMES`,
a libglvnd variable that other distributions may not honour, and Mesa's EGL then still has to
pick the right render node. The Vulkan loader, on the other hand, opens exactly the manifests it
is given: the restriction holds in one standard variable, on any distribution, and the D-00
measurement already showed the NVIDIA card asleep with it (appendix of ADR 0014). Measured side
by side on the bar (3 runs each), OpenGL restricted to Mesa's EGL also kept the NVIDIA card
asleep, but started in 234 to 353 ms against 190 to 205 ms, with 104 to 110 MiB resident against
74.6 MiB. Vulkan is both the safer and the lighter path.

Disabling the Vulkan loader's implicit layers (`VK_LOADER_LAYERS_DISABLE=~implicit~`) changed
nothing (74.4 against 74.6 MiB), and no NVIDIA library is mapped in either case. NVIDIA's layer
(`VK_LAYER_NV_optimus`, `nvidia_layers.json`) stays off because its manifest only enables it
when `__NV_PRIME_RENDER_OFFLOAD=1` is set (`prime-run` sets it), not because its driver is
hidden: with that variable it would load `libGLX_nvidia.so.0`. Hence `apply` unsets
`__NV_PRIME_RENDER_OFFLOAD`, rather than disabling every implicit layer.

## Consequences

- `sherlock-ui` enables iced's `wgpu-bare` feature and depends on `wgpu =27.0.1` (the version
  iced pulls) with `std`, `wgsl` and `vulkan`. Release binaries grow from 15.2 to 22.4 MiB
  (bar) and from 21.0 to 28.1 MiB (Evidence); with iced's full `wgpu` feature (Vulkan
  and OpenGL) they were 23.1 and 28.9 MiB. `iced_renderer` is declared to tell, when drawing,
  which renderer iced picked (`surface::gpu_drawn`).
- Tests stay in software on every machine: `.cargo/config.toml` sets `ICED_TEST_BACKEND=tiny-skia`
  (without it, iced_test tries wgpu with the most powerful adapter, which wakes the NVIDIA card).
- A surface drawn by the GPU has alpha: the floating bar shows its rounded corners and its glass.
  In software it stays opaque, as before.
- **Our compositor cannot show GPU clients yet.** It only offers `wl_shm` buffers: no
  `zwp_linux_dmabuf_v1`, no `wl_drm`. Mesa's Vulkan driver needs one of them to present on
  Wayland, so under our compositor the shell falls back to tiny-skia, opaque. Checked with our
  compositor nested in the headless KWin: the bar chose Vulkan, found no adapter able to
  present, and drew `wl_shm` XRGB8888 buffers with tiny-skia, without an error. The compositor
  must offer `linux-dmabuf` (Smithay's `DmabufState`) for the GPU path, and for every GPU
  application and game anyway. **Until it does, the gain of this ADR (60 frames per second in
  Evidence, a translucent bar) only holds under Plasma (KWin).** That is a compositor lot; this
  lot does not touch the compositor.
- Only AMD chips are tried. An Intel chip (`i915`, `xe`, manifest `intel_icd*.json`) would take
  the same path: a follow-up, once tested on a machine.
- The dock (lot D-21) makes the choice, so it never wakes the NVIDIA card, and its launcher
  gives the environment back to the applications. Its transparency and shadows come in a small
  follow-up lot.
- The bar's menu surface grows by the shadow (18 left, 32 top, 54 right, 76 bottom) on the GPU.
  Since xdg_positioner keeps the anchor rectangle inside the parent, the popup is centred below a
  point of the bar instead of hanging from the Sherlock button. Its input region is the menu
  alone: the transparent room for the shadow lies over the bar (from y 20 on the screen), whose
  buttons stay reachable through it. Before the bar's first frame the renderer is unknown and the
  menu takes the software margins: its blurred shadow would then be cut (unlikely: the menu
  opens from the drawn bar).
- The GPU menu has been seen only as a headless render, not opened in a real compositor: it needs
  a try by the author under Plasma before this ADR is accepted.

## Measurements

Method: a headless KWin 6.3 (`kwin_wayland --virtual`, 1920 × 1080, 60 Hz) in its own D-Bus
session and XDG folders, run inside `bwrap` with a `/dev/dri` that only holds the AMD render node
(the virtual backend takes the first render node it finds, which is NVIDIA's on this laptop), with
Mesa's EGL only. Nothing appears on the author's screen. Release builds. The NVIDIA status is read
every 100 ms before, during and after each run; power is the AMD package power (`amdgpu` hwmon
`PPT`, read-only, 1 W steps). `upower` gives no discharge rate on AC, and the CPU powercap
(`intel-rapl:0/energy_uj`) is readable by root only. Other builds ran on the machine at the same
time: power figures are indicative.

| Measure | Before: tiny-skia | After: wgpu, AMD |
| --- | --- | --- |
| Bar: start to first frame, median of 5 | 137.6 ms (136 to 147) | 200.3 ms (190 to 205; 1,110 ms on the very first run, shader cache empty) |
| Bar: RSS / PSS at rest | 17.6 / 14.2 MiB | 74.6 / 37.7 MiB |
| Bar: CPU and frames at rest | 0.0 %, 0 frames/s | 0.0 %, 0 frames/s |
| Evidence: start to first frame, median of 5 | 169.3 ms (163 to 192) | 221.5 ms |
| Evidence: RSS / PSS at rest, 5,000-file folder | 38.0 / 31.5 MiB | 88.4 / 51.6 MiB |
| Evidence maximised: folder of 5,000 files opened | 56 to 63 ms | 63 to 81 ms |
| Evidence maximised: scroll, 3 runs | 46.2, 41.4, 36.2 frames/s, p95 28 to 34 ms, max 50 to 52 ms | 60.0, 60.0, 60.1 frames/s, p95 17.1 to 17.4 ms, max 17.6 to 18.1 ms |
| Evidence maximised: animated selection, 3 runs | 59.3 to 59.7 frames/s, max 26 to 34 ms | 60.0 frames/s, max 17.6 to 18.0 ms |
| Evidence measure (6 s): CPU of the process, 3 interleaved pairs | 59 to 63 % of a core | 20 % of a core |
| Evidence measure: AMD package power, same pairs | 32.8 to 36.6 W | 24.2 to 32.9 W |
| NVIDIA card | `suspended` before, during, after | `suspended` before, during, after; no NVIDIA device or library held by the process |

Evidence maximised now holds 60 frames per second: no interval reaches two frames. The GPU
costs about 50 MiB of resident memory per program, a part of it shared (PSS grows less than
RSS), and 60 ms more at start. The package power is not conclusive: other builds ran on the
machine (load about 5), and the sensor has 1 W steps. The CPU time of the process, which other
load does not distort, falls by two thirds while it draws 60 frames instead of about 40.

The "after" column was measured with iced's full `wgpu` feature (Vulkan and OpenGL compiled,
Vulkan used); the Vulkan-only build that ships renders the same way (checked: RADV, `AR24`
buffers, NVIDIA asleep).

The compositor gives a frame every 16.7 ms: "over 16.7 ms" counts the jitter of that pace, and a
missed frame shows as an interval of about 33 ms.

During one series of Evidence (before), the NVIDIA card was seen `resuming` then `active`. That
binary compiles no GPU renderer and loads no graphics driver; the five runs repeated right after
saw it `suspended` throughout. The wake came from another process of the machine.

## Amendment (lot UI-09)

Without an AMD chip (an Intel laptop, an NVIDIA desktop, no GPU), the shell rendered with
tiny-skia: black corners around the bar, the bubbles' glass mixed with black (a dull gray in
the light theme, « 12 % » of a low battery at 3.1 to 1), and no animation of the panels and
menus. Proposed change, to approve with the rest of this ADR:

- The chip that drives the screen (`boot_vga`, or the only chip) is used when its kernel driver
  has a Vulkan driver the shell trusts, restricted to that driver's manifests exactly as for
  AMD: `i915`/`xe` with ANV and HASVK, NVIDIA's own driver with its manifest (that chip drives
  the screen, so it is awake anyway). nouveau (NVK) and virtual GPUs are not used. A second
  chip of the same family keeps the choice in software, as for two AMD chips.
- Otherwise, Vulkan on the processor (lavapipe), when installed, rather than tiny-skia: alpha,
  rounded corners and animations, at the cost of the processor time of a software rasterizer
  while something moves. tiny-skia stays the last resort, with an opaque glass.

The NVIDIA card of a hybrid laptop stays asleep: it never drives the screen there, and the
loader is never given its manifest.

## What would make us reconsider

- The NVIDIA card leaves `suspended` while a shell program starts, renders or stops on the GPU.
- The GPU path costs more memory or power at rest than it saves while animating, for the bar
  (which barely animates) in particular: the bar could then stay in software, once softbuffer
  offers buffers with alpha.
- Mesa's Vulkan driver is missing on a target machine with an AMD chip, or the OpenGL path
  proves as safe and lighter.
- iced lets a program create its wgpu instance and adapter itself: the environment variables
  would no longer be needed.
