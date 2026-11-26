# Cursors

The cursor theme of the desktop (lot D-07): its drawings, the generator that turns them into
Xcursor themes, and the way the compositor shows them in the real session.

The drawings follow the author's cursor boards, `docs/design/cursors-dark.html` and
`docs/design/cursors-light.html` (renderings: `cursors-dark.png`, `cursors-light.png`). The
boards make the reference: base size 24 px, sharp up to 3×, an outline that detaches the
cursor from any background, short animations with a bounce.

![Every cursor, dark theme on the dark desktop, light theme on the light desktop](cursor/board.png)

`docs/cursor/board.png` is generated: `just cursor-board`. Each row is a shape, at 24, 48 and
72 px, then its 24 px image enlarged five times.

## The shapes

Seventeen drawings, in `assets/cursors/`. The CSS names that `wp_cursor_shape_v1` gives (versions
1 and 2) and the legacy X11 names are aliases: links to the file of the drawing.

| Drawing | Name on the board | Hotspot | Also answers to |
| --- | --- | --- | --- |
| `default` | « Pointeur » | tip (5, 2.5) | `context-menu`, `help`, `alias`, `dnd-ask`, `left_ptr`, `arrow`… |
| `pointer` | « Lien » | finger tip (11.2, 2.6) | `hand2`, `hand1`, `hand`, `pointing_hand` |
| `text` | « Texte » | middle | `vertical-text`, `xterm`, `ibeam` |
| `wait` | « Attente » | center | `watch` |
| `progress` | « En arrière-plan » | tip | `left_ptr_watch`, `half-busy` |
| `ew-resize` | « Largeur » | middle | `e-resize`, `w-resize`, `col-resize`, `h_double_arrow`… |
| `ns-resize` | « Hauteur » | middle | `n-resize`, `s-resize`, `row-resize`, `v_double_arrow`… |
| `nwse-resize` | « Diagonale » | middle | `nw-resize`, `se-resize`, `bd_double_arrow`… |
| `nesw-resize` | « Diagonale », mirrored | middle | `ne-resize`, `sw-resize`, `fd_double_arrow`… |
| `move` | « Déplacer » | middle | `all-resize`, `all-scroll`, `fleur`, `size_all` |
| `grabbing` | « Saisi » | palm (12.8, 14) | `grab`, `openhand`, `closedhand` |
| `not-allowed` | « Interdit » | middle | `no-drop`, `forbidden`, `crossed_circle` |
| `crosshair` | « Précision » | middle | `cell`, `cross`, `tcross`, `plus` |
| `zoom-in` | « Zoom » | lens (10.5, 10.5) | |
| `zoom-out` | « Zoom », with a minus | lens | |
| `copy` | « Copier » | tip | `dnd-copy` |
| `sherlock` | « Sherlock » | tip | |

The boards draw fifteen of them. `nesw-resize` is the diagonal of the board turned the other
way, `zoom-out` is its magnifier with a minus. The CSS names without a drawing take the closest
one: a column edge moves as a window edge, a cell is aimed at as a point, `grab` shows the
closed hand. The full list is `SHAPES` in `desktop/cursor-theme/src/lib.rs`, and a test checks
that every CSS name has a shape.

`sherlock` is the arrow with a magnifier badge, for Super + click: Sherlock explains what is
under the pointer. The theme holds the shape; the action itself is reserved and not plugged in
yet.

## The drawings

One SVG per drawing, `assets/cursors/<name>.svg`, copied from the boards: on a grid of 24 units
(one unit is one pixel at 24 px), the paths of the boards unchanged. Each file says what the
generator needs:

- `data-hotspot="x y"` on the root: the point of the cursor, in grid units.
- A `<style id="theme">` block, with the colors of the dark theme. The parts that change with
  the theme carry its classes: `body` (inside), `edge` (outline), `ink` (lines of the line
  drawings), `track` (ring under the green arc). The green of the arc and the badges keep
  their own colors, as in both boards.
- For the waiting cursors, a group `id="spin"` with `data-center="x y"`: the part that turns.

Replacing a drawing is copying its SVG over the old one, then `just cursors`. A test checks
that every file has these marks and that its style block is the dark theme as the generator
writes it.

The colors come from `sherlock_ui::cursor`: cream with the darkest blue outline on the dark
desktop, night blue with a white outline on the light one. The timings of the motion live there
too.

## The generator

`desktop/cursor-theme` is a library (shapes, names, the Xcursor writer) and, with its `render`
feature, the `cursor-theme` program, which renders the SVGs with resvg (the version iced
already pulls).

| Command | What it does |
| --- | --- |
| `just cursors` | Writes `target/cursors/nom-os/` (dark desktop) and `target/cursors/nom-os-light/` |
| `just cursor-board` | Draws `docs/cursor/board.png` |
| `just install-session` | Also runs `just cursors`, then prints the commands that copy both themes into `/usr/local/share/icons` |

Each theme has an `index.theme` (inheriting `default`), with the name a cursor setting shows:
« SherlockOS » and « SherlockOS clair » (`sherlock_ui::cursor`). In `cursors/`, it has one
Xcursor file per drawing and a link per alias. A file holds one image per nominal size: 24, 32, 48, 64 and 96
(scales 1 to 4). An image is a quarter larger than its size (30 px at 24): room for the outline
that passes the grid and for the badges that pass it at the bottom right, as in the boards. The
grid starts two units in, rounded up to a whole pixel, so the drawing stays on its pixels; the
generator refuses a drawing that touches the edge of its image. The hotspot is the pixel under
the point. The waiting cursors hold, per size, a turn of 30 images of 33 ms: a turn per second.

To try the themes without installing them: `XCURSOR_PATH=$PWD/target/cursors`.

## In the compositor

`desktop/compositor/src/cursor.rs`, real session only: in nested mode the host window keeps
its own cursor.

- **Theme.** The session (`sherlockos-session`) exports `XCURSOR_THEME`: `nom-os`, or
  `nom-os-light` when the appearance file (`docs/appearance.md`) gives the light theme at
  startup, automatic mode included. It exports `XCURSOR_SIZE` (24 unless already set) and
  `XCURSOR_PATH`, with `cursors` beside the programs' folder first (`target/cursors` when the
  session runs from the repository with `NOM_OS_BIN=target/release`), then the usual folders
  (`~/.local/share/icons`, `~/.icons`, `/usr/local/share/icons`, `/usr/share/icons`). The
  compositor takes the house theme that `XCURSOR_THEME` names, so XWayland, GTK and Qt, when
  they draw their cursor themselves, draw the same one: two themes never mix. Started
  otherwise, the compositor picks the house theme from the appearance file itself. Without the
  house theme installed, `XCURSOR_THEME`, else the system's `default`. A shape missing from the theme shows the
  arrow. Following a theme change live comes with the theme setting of the compositor (lot
  D-05c), as for the wallpaper.
- **Size.** `XCURSOR_SIZE` (24 by default) times the screen's scale. The image of the nearest
  size above is shown pixel for pixel: at 1.25, 30 px are asked for and the image of 32 is
  shown, never a stretched 24.
- **Waiting.** `wait` and `progress` turn, one image per 33 ms. A timer wakes the screen when
  the next image is due: nothing is drawn between two images.
- **Change of shape.** A new shape fades in from 30 % and 80 % of its size to full, in 120 ms,
  on the boards' curve (`cubic-bezier(0.2, 0.9, 0.3, 1.35)`, a light bounce past the end).
- **Shake to find.** Four changes of direction within 700 ms, each move longer than 6 logical
  pixels, make the cursor 2.6 times larger in 240 ms. It stays so until a second after the last
  shake, then comes back in 240 ms, with a bounce. While it is larger, the image of the largest
  size is drawn scaled down, sharp at its peak.

While a transition runs, the screen draws at each vertical blank; once it is over, nothing is
drawn until something changes (see "Frames of the real session" in `docs/compositor.md`). A
still cursor goes on the screen's cursor plane when it fits; a scaled or fading one is
composited.

Not done in this lot: **show the clicks** (the accessibility option of the boards: the pointer
sinks and a green wave leaves the clicked point) needs an option and a drawn ring, and comes
next; Super + click for Sherlock is only a shape for now.

## Tests

| Test | What it checks |
| --- | --- |
| `cursor-theme`, unit | Every name given once; every CSS name of the protocol has a shape; every drawing has its marks and its hotspot inside its image; the dark style block of the files; the light theme changes only the style block; the spinning group turns around its center; the Xcursor writer, read back by the parser the compositor uses |
| `cursor-theme`, `tests/theme.rs` | Runs the generator: in both themes, every shape at every size, its hotspot, something drawn under it, 30 images for the waiting cursors, every alias a link to its drawing |
| `compositor`, `cursor.rs` | The theme the session names wins; every shape a client names finds the file of its drawing; the nearest size above; fallback on the user's theme and on the arrow; the turn of the waiting cursor and the time of its next image; the fade of a new shape; shake to find, held then back to rest; nothing asked at rest |
| `sherlock-ui`, `cursor.rs` | The bounce curve; the contrast of body and outline |

On the machine, in the real session, after `just install-session` and the printed commands:

1. Move over the desktop: the cream arrow (night blue on the light theme), sharp. At scale 1.25,
   sharp too.
2. Over a link in a browser, the hand; over text, the I-beam; at a window edge, the double arrow.
   Each change fades in, with a light bounce.
3. Launch a program that takes time to start: the arrow with the turning ring, smooth, a turn per
   second. `--measure` shows about 30 frames per second meanwhile, none once it stops.
4. Shake the mouse left and right: the cursor grows, stays a second, then comes back with a
   bounce. At rest, no frame is drawn.
