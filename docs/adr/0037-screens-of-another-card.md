# ADR 0037: The screens of another card, drawn on the chip that renders

- Status: proposed
- Date: 2026-11-28
- Lot: D-16

## Context

The reference laptop wires its HDMI port to the NVIDIA card; the AMD chip drives the internal
screen and renders the desktop (ADR 0006, ADR 0018). Until now the compositor never opened the
NVIDIA card, so that it sleeps outside games: the HDMI screen stayed dark, while the author needs
it to project in class. The first freezes of the machine came with wakes of the NVIDIA card: it
must be awake only while a screen is plugged into it.

## Decision

**Opened for a screen, closed without one.** A card other than the one that renders is opened
through the seat when `/sys` says a screen is plugged into it (the status the kernel keeps,
updated by the card's hotplug interrupt: reading it never wakes the card), and closed when its
last screen goes, its descriptor given back to the seat so that logind lets go of its own. A card
that cannot drive its screen is closed and given up until the screen is unplugged.

**Drawn on the chip that renders, shown by the other card.** The other card only scans out:
the frames of its screens are allocated (linear) and drawn on the AMD chip by the same renderer
as every screen, then imported into the other card (PRIME, `gbm_bo_import` through the other
card's GBM device). No EGL context, no renderer, no client buffer on the NVIDIA card. Smithay's
`MultiRenderer` (a renderer per card, with copies between them) was not taken: it would open the
NVIDIA card's EGL, and change the type of the renderer every part of the compositor draws with.

**A way back without reinstalling**: the file `~/.config/sherlockos/no-secondary-cards`, or
`SHERLOCKOS_SECONDARY_CARDS=0`, and no other card is ever opened.

## Consequences

- The HDMI screen of the reference laptop works when the NVIDIA card's GBM backend imports
  linear buffers of the AMD chip; otherwise it is given up after three frames, and the card
  sleeps again.
- The NVIDIA card is awake as long as a screen is plugged into it, even unused.
- Each frame of that screen crosses PCIe from the AMD chip's memory: fine for slides and video,
  not for games, which render on the NVIDIA card anyway.
- No cursor plane on that screen: the cursor is drawn into each frame.

## What would make us reconsider

- The machine freezes when the card wakes or sleeps: the way back stays, and the cause is looked
  for in the driver's runtime power management first.
- The import fails on the NVIDIA driver: a copy through the processor into the card's own
  buffers (dumb buffers), or the `MultiRenderer`.
