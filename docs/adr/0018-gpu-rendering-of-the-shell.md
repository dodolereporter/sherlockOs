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

The iced programs of the shell (bar, Evidence, and the dock when it lands) render with wgpu on
Vulkan, through Mesa's driver for the AMD chip and that driver only. Each program decides at the
very start of `main`, before any thread exists, with `sherlock_ui::gpu::Choice`:

1. It reads `/sys/class/drm/card*/device` (vendor, kernel driver, `boot_vga`). These are values
   kept by the kernel: reading them never wakes a card. (`lspci` does: it reads the PCI
   configuration space. It woke the NVIDIA card during this lot.)
2. If an AMD chip is bound to `amdgpu` and a manifest of Mesa's Vulkan driver (`radeon_icd*.json`)
   exists in the loader's folders, it sets `VK_DRIVER_FILES` to those manifests only,
   `WGPU_BACKEND=vulkan`, `WGPU_POWER_PREF=low` and `ICED_BACKEND=wgpu,tiny-skia`. The Vulkan
   loader never opens NVIDIA's driver, nor nouveau's (NVK), nor the software one.
3. Otherwise (no GPU, CI, NVIDIA alone, an Intel chip not tried yet) it sets
   `ICED_BACKEND=tiny-skia`: wgpu is never started.
4. If `ICED_BACKEND` is already set, the user chose: nothing is changed.

If wgpu still finds no adapter that can present to the compositor, iced falls back to tiny-skia
by itself (`ICED_BACKEND=wgpu,tiny-skia`). The commands a program starts (applications opened by
Evidence, the bar's monitors) get the environment the program was started with
(`gpu::for_child`), so that a game launched from the shell is not bound to the AMD chip.

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
measurement already showed the NVIDIA card asleep with it (appendix of ADR 0014). The OpenGL path
was to be measured side by side in this lot; that measurement is still to do (see below).

## Consequences

- `sherlock-ui` enables iced's `wgpu` feature: the binaries grow by the size of wgpu and naga
  (to be measured), and `iced_renderer` is declared to tell, when drawing, which renderer iced
  picked (`surface::gpu_drawn`).
- Tests stay in software on every machine: `.cargo/config.toml` sets `ICED_TEST_BACKEND=tiny-skia`
  (without it, iced_test tries wgpu with the most powerful adapter, which wakes the NVIDIA card).
- A surface drawn by the GPU has alpha: the floating bar shows its rounded corners and its glass.
  In software it stays opaque, as before.
- **Our compositor cannot show GPU clients yet.** It only offers `wl_shm` buffers: no
  `zwp_linux_dmabuf_v1`, no `wl_drm`. Mesa's Vulkan driver needs one of them to present on
  Wayland, so under our compositor the shell is expected to fall back to tiny-skia, opaque. The
  compositor must offer `linux-dmabuf` (Smithay's `DmabufState`) for the GPU path, and for every
  GPU application and game anyway. That is a compositor lot; this lot does not touch the
  compositor.
- Only AMD chips are tried. An Intel chip (`i915`, `xe`, manifest `intel_icd*.json`) would take
  the same path, once tested on a machine.
- The bar's menu surface grows by the shadow (18 left, 32 top, 54 right, 76 bottom) on the GPU.
  Since xdg_positioner keeps the anchor rectangle inside the parent, the popup is centred below a
  point of the bar instead of hanging from the Sherlock button; a click on the shadow closes the
  menu, like a click elsewhere.

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
| Bar: start to first frame, median of 5 | 137.6 ms (136 to 147) | to measure |
| Bar: RSS / PSS at rest | 17.6 / 14.2 MiB | to measure |
| Bar: CPU and frames at rest | 0.0 %, 0 frames/s | to measure |
| Evidence: start to first frame, median of 5 | 169.3 ms (163 to 192) | to measure |
| Evidence: RSS / PSS at rest, 5,000-file folder | 38.0 / 31.5 MiB | to measure |
| Evidence maximised: folder of 5,000 files opened | 56 to 63 ms | to measure |
| Evidence maximised: scroll, 3 runs | 46.2, 41.4, 36.2 frames/s, p95 28 to 34 ms | to measure |
| Evidence maximised: animated selection, 3 runs | 59.3 to 59.7 frames/s, max interval 26 to 34 ms | to measure |
| AMD package power during the Evidence measure | 28 to 30 W | to measure |
| NVIDIA card | `suspended` before, during, after | to measure |

The compositor gives a frame every 16.7 ms: "over 16.7 ms" counts the jitter of that pace, and a
missed frame shows as an interval of about 33 ms.

During one series of Evidence (before), the NVIDIA card was seen `resuming` then `active`. That
binary compiles no GPU renderer and loads no graphics driver; the five runs repeated right after
saw it `suspended` throughout. The wake came from another process of the machine.

## What would make us reconsider

- The NVIDIA card leaves `suspended` while a shell program starts, renders or stops on the GPU.
- The GPU path costs more memory or power at rest than it saves while animating, for the bar
  (which barely animates) in particular: the bar could then stay in software, once softbuffer
  offers buffers with alpha.
- Mesa's Vulkan driver is missing on a target machine with an AMD chip, or the OpenGL path
  proves as safe and lighter.
- iced lets a program create its wgpu instance and adapter itself: the environment variables
  would no longer be needed.
