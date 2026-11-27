# ADR 0027: Blur behind glass, asked with ext-background-effect-v1, drawn by the compositor

- Status: proposed
- Date: 2026-11-27
- Lot: D-37

## Context

Everything translucent on the desktop is glass: the dock, the bar, the panels, the menus, the
on-screen display, the notification bubbles, later the glass windows of our applications. Over
a busy background (text of a terminal), the glass alone does not keep its own text readable. The
author wants a light blur behind it, a moderate radius and a little saturation, as the
"vibrancy" of macOS. Three questions:

- **Who asks, and how.** Only the client knows where its glass is: the dock's surface is taller
  than its bar (magnified icons, label), the bar's surface holds a shadow. Two Wayland protocols
  exist: `ext_background_effect_v1` (wayland-protocols, staging) and KDE's `org_kde_kwin_blur`.
  The toolkit of our shell, iced_exwlshell 0.21.4, already binds the first one on every surface
  it makes, and sends the region its widgets declare (`BlurRegions`). Qt applications (Konsole,
  Kate) ask with the second one.
- **How to blur.** The compositor draws with GLES (Smithay 0.7) on the AMD chip (Renoir), at 60
  frames per second, and must not draw anything while nothing changes (lot P-01).
- **Where it cannot.** The software renderer (an old PC, a virtual machine), or a GPU without
  GLES 3.

## Decision

**Protocol.** The compositor offers `ext_background_effect_v1` (version 1, the `blur`
capability). The region a surface gives, in its own coordinates and clipped to its size, is
double-buffered: it applies at the surface's commit. A region is made of rectangles only: our
components send their rounded shape as a band per row in the corners (`sherlock_ui::blur::
rounded_region`); the glass drawn over it smooths the edge. KDE's `org_kde_kwin_blur` follows
in lot D-37b, for Qt applications, into the same state.

**Drawing.** Behind each surface with a region, the compositor puts a blur element:

- When something behind it changed in its area, it is damaged whole: the damage tracker redraws
  what is behind, then the element copies that area of the frame at half its size
  (`glBlitFramebuffer`), and runs a dual Kawase filter, `PASSES` halvings then as many doublings,
  into textures of its own. It then draws the result over its region, with `SATURATION`.
- Otherwise it keeps its textures and draws them again where the frame is redrawn: nothing is
  computed while the screen is at rest, and nothing is drawn when nothing changes.
- "Something behind changed" is read from the elements behind it: one that appears, goes, moves,
  fades, or whose damage since the last frame reaches the area.

**Without blur.** On the software renderer (`llvmpipe`, `softpipe`), without GLES 3, or after a
GL error, the element draws a veil of the desktop color at `VEIL_OPACITY` instead: the glass
reads denser, as the "Reduce transparency" setting of macOS does. While the session is locked,
no blur is drawn.

**Tokens.** The number of passes, the offset of the samples, the saturation and the veil live in
`sherlock_ui::blur`. The glass components of sherlock-ui declare their region by themselves.

## Consequences

- Any client can get the blur, ours without a line of protocol code.
- A blur behind an animation (a window passing under the dock, the genie) is computed again at
  each frame of it: a few passes on textures of a quarter of the area or less.
- The blur samples only inside its area: near its edges it is a little lighter than in the
  middle, which the glass hides.
- The "Reduce transparency" setting of Control Room, and the switch to the veil when a frame
  with blur takes too long on the GPU (it needs timer queries, which Smithay's GL bindings do not
  load), are left to lot D-37b.

## What would change the decision

- A protocol for rounded regions, or a radius given by the client: the corners would be smoothed
  by the compositor.
- Measurements on the reference machine showing the blur costs more than a millisecond per
  frame at rest or in motion.
- Smithay offering a blur of its own, or render passes between elements.
