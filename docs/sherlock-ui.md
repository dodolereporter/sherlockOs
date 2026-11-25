# sherlock-ui

`desktop/sherlock-ui` is the visual library of the desktop: the only place where colors, sizes,
fonts and interface texts are written. Lot D-01 fills it from the Visual identity section of
`docs/contexte.md` and from the design boards, which are authoritative for the appearance. The
toolkit is iced 0.14 (ADR 0014), rendered on the AMD chip or in software (ADR 0018).

## Two layers

| Layer | Modules | Depends on |
| --- | --- | --- |
| Tokens | `color`, `metrics`, `motion`, `strings`, `typography`, `icon`, `logo`, `window`, `gpu` | Nothing |
| iced | `theme`, `component`, `focus` | iced 0.14, behind the `iced` feature |

The gallery is not in the library: it lives in `examples/gallery/` and its tests include it.
The shell programs compile neither it nor its sample texts.

The `iced` feature is on by default, because every shell program wants it. The compositor,
which only reads tokens, turns it off with `default-features = false` and does not compile
iced. iced is compiled with both renderers, wgpu and tiny-skia: see Renderer.

## Renderer (D-02)

A program chooses its renderer first thing in `main`, before any thread exists (ADR 0018):

```rust
let renderer = sherlock_ui::gpu::Choice::detect();
// SAFETY: first statement of `main`, no other thread exists yet.
unsafe { renderer.apply() };
```

`Choice::detect` reads `/sys/class/drm` (never `lspci`, which wakes a sleeping card) and the
Vulkan loader's folders. With an AMD chip bound to `amdgpu` and Mesa's Vulkan manifest
(`radeon_icd*.json`), it renders with wgpu on Vulkan, the loader seeing that manifest only
(`VK_DRIVER_FILES`): the NVIDIA card is never asked anything. Otherwise it renders with
tiny-skia. `ICED_BACKEND` set by the user wins (`ICED_BACKEND=tiny-skia bar` forces software).
If wgpu finds no adapter that can present, iced falls back to tiny-skia by itself.

A command the program starts goes through `gpu::for_child(&mut command)`, which gives it back
the environment the program was started with.

`surface::gpu_drawn()` tells which renderer drew the last `surface::Shadowed`: a program that
sizes a surface for a blurred shadow (the bar's menu) learns it from its first frame.

Tests always render with tiny-skia: `.cargo/config.toml` sets `ICED_TEST_BACKEND=tiny-skia`.
`ICED_TEST_BACKEND=wgpu cargo test` tries the GPU, on the most powerful adapter: not on the
reference laptop, it wakes the NVIDIA card.

## Tokens

All tokens are named by their role, in English like the rest of the code.

- **`color`**: backgrounds (`DESKTOP`, `DESKTOP_DISCS`, `SYSTEM_SCREEN`, `DEEPEST`, `WINDOW`,
  `TITLE_BAR`, `SIDEBAR`, `CARD`, `TERMINAL_BLOCK`), glass (`GLASS`, `MENU_GLASS`,
  `DOCK_GLASS`), lines and veils (`BORDER`, `MENU_BORDER`, `CARD_BORDER`, `DOCK_BORDER`,
  `SECONDARY_BORDER`, `DIVIDER`, `FIELD`, `HOVER`, `PRESSED`, `DISABLED`, `TOGGLE_OFF`,
  `SHADOW`, `MENU_SHADOW`), texts (`TEXT_PRIMARY`, `TEXT_SECONDARY`, `TEXT_TERTIARY`,
  `TEXT_LINE_NUMBER`, `TEXT_ON_ACCENT`, `TEXT_ON_CREAM`), accents (`ACCENT` and its hover and
  pressed shades, the four blues, `CREAM`, `AMBER`, `SALMON`), states (`SELECTED_ROW`,
  `ACTIVE_ENTRY`, `FOCUS_RING`). `Color` can lay itself over a background (`over`, sRGB blending
  like the browser and tiny-skia) and compute a WCAG contrast (`contrast`).
- **`metrics`**: top bar, window, panel, menu, button, toggle, field, card, focus ring, icons,
  dock, spacings by role (`GAP_TIGHT`, `GAP_ICON`, `GAP_INLINE`, `GAP_SECTION`, `GAP_ROW`,
  `PAGE_PADDING`) and durations. The ranges of the design (button from 28 to 36, radius from 8
  to 10, icon stroke from 1.6 to 1.7…) are checked at compile time.
- **`typography`**: families, sizes, weights, and the three font files to load.
- **`strings`** (added by D-20): shared interface texts. Abbreviated days and months, short
  date (« mar. 24 nov. »), time, percentage with the narrow no-break space, workspace label
  (« 1 · Code ») and the Sherlock menu entries (`strings::top_bar`).
- **Top bar** (D-20): `color::TOP_BAR_GLASS` and `TOP_BAR_BORDER` (Main board, slightly
  lighter than `GLASS` and `BORDER`), `CHIP` and `CHIP_HOVER` for the workspace chips;
  `metrics::TOP_BAR_*`, `SHERLOCK_*`, `CHIP_*`, `STATUS_*`, `MENU_AVATAR`,
  `MENU_HEADER_PADDING`; `color::TOP_BAR_BUTTON_OPEN` (veil of an open bar button); icons `NetworkOff`, `SoundMuted`, `Battery`, `BatteryCharging`;
  `color::TRANSPARENT` for the background of a shell surface, `metrics::EVENT_COALESCE_MS`
  (150 ms) and `EVENT_COALESCE_MAX_MS` (500 ms) for merging bursts of system events.
- **`window`**: grab band and snapping of windows, set by D-17 for the compositor.
- **`metrics::DESKTOP_DISC_GEOMETRY`**: position and diameter of the three wallpaper discs on
  the reference screen (Main board), added by D-11.

## Motion

The `motion` module, added by D-11, holds the physics of the animations: pure computation,
without iced, that the compositor already uses and that the shell will take up. Time is a
`Duration` counted from an origin chosen by the caller: an animation is computed at any
instant, with no state that advances frame by frame.

| Type | Role |
| --- | --- |
| `Spring` | Damped spring of mass 1: stiffness (`stiffness`) and damping (`damping`). `damping_ratio` gives the damping ratio. `solve(offset, velocity, t)` returns the offset and the velocity after `t` seconds, by the exact solution (underdamped, critical or overdamped) |
| `Animation` | A value driven by a `Spring` from `from` to `to`. `value(now)`, `target()`, `is_done(now, precision)`. `retarget(now, to)` changes target while keeping value and velocity: nothing jumps |
| `Fade` | Fade over a fixed duration, brisk start and gentle arrival (cubic). `value(now)`, `is_done(now)`. `retarget(now, to, duration)` restarts from the current value, for the share of the run that is left |

| Token | Value | Use |
| --- | --- | --- |
| `SNAP_SPRING` | stiffness 500, damping 33.5 (ratio 0.75, overshoot of 3 %) | Snap, maximize, restore of a window |
| `OPEN_SPRING` | stiffness 450, damping 34 (ratio 0.80, overshoot of 2 %) | Scale of a window as it opens |
| `PREVIEW_SPRING` | stiffness 700, damping 42 (ratio 0.79) | Snap preview |
| `OPEN_SCALE` | 0.92 | Scale of a window at the start of its opening, and of the preview that appears |
| `FADE` | 150 ms, the top of `metrics::HOVER_MS` | Fade of opening, appearing and clearing |
| `FRAME_RATE` | 60 | Frame rate to hold, animations included (`docs/contexte.md`) |
| `GEOMETRY_PRECISION` | 0.5 px | Offset below which an animated geometry is settled |
| `SCALE_PRECISION` | 0.001 | Offset below which an animated scale is settled |

`docs/contexte.md` only says « ressorts » and « léger dépassement »: the stiffnesses and dampings
are choices of D-11, to review with the design. The `motion` tests hold them: overshoot
between 0.5 and 4 %, 90 % of the run in less than 150 ms, settled within 1 % in less than
300 ms, fade within the range of a hover. Their use in the compositor is described in
`docs/compositor.md`, section Animations.

## Fonts

Inter (variable version), Space Grotesk (variable version) and JetBrains Mono Regular are
embedded in `desktop/sherlock-ui/fonts/`, each with its OFL 1.1 license. They come from their
authors' repositories: Inter 4.1 (`rsms/inter`), Space Grotesk (`google/fonts`, file
`SpaceGrotesk[wght].ttf`), JetBrains Mono 2.304. The variable versions avoid one file per
weight; cosmic-text, the text engine of iced, applies the requested `wght` axis. The internal
name of variable Inter is "Inter Variable": that is the one in `typography::UI_FAMILY`. A
program loads them at start-up: `typography::FONTS` passed to `application(..).font(..)`.

## Theme and components

`theme::theme()` builds the iced theme on the tokens. Each component has its style function
by state in `theme`, and its constructor in `component`:

| Component | Constructor | Design board |
| --- | --- | --- |
| Primary, secondary button | `primary_button`, `secondary_button` | Reglages, Panneaux |
| Icon-only button | `icon_button(icon, label, message)` | Title bar of Reglages |
| Toggle 44 by 26 | `toggle(value, message)` | Reglages |
| Text field | `text_field` | Search of Reglages |
| Menu entry, shortcut, submenu, check | `menu_item(label, Trailing, message)`, `open` | Menus |
| Menu, separator | `menu`, `menu_separator` | Menus |
| Setting row, line between two rows | `setting_row(label, help, control)`, `divider` | Reglages |
| Section label | `section_label` | Panneaux, Main |
| Card, glass panel | `card`, `glass_panel` | Reglages, Panneaux |
| Texts | `body`, `description`, `title`, `icon` | All |
| Workspace chip (D-20) | `chip(label, active, message)` | Main |
| Sherlock button, logo (D-20) | `sherlock_button(open, message)`, `logo`, `logo_square` | Main, Menus |
| Top bar indicator (D-20) | `status(icon, value, tint)` | Main |
| Menu header (D-20) | `menu_header(title, subtitle)` | Menus |

The design boards are the files of `docs/design/planches/`, named here without their
`.dc.html` extension.

A component without a message is disabled. Clickable components return a `Control`, which
becomes an `Element` and accepts `status(..)` and `focus_ring()` to force a state (gallery).

### Keyboard

iced 0.14 can only focus its text fields. `focus::Focusable` wraps the other components: it
registers with iced's focus operations, triggers the action on Enter or Space, and draws a
2 px light blue ring 2 px away from the edge. A key held down activates only once. A
component that becomes disabled loses the focus. A click clears the ring, like
`:focus-visible` in a browser. The program moves the focus around:

```rust
fn subscription(&self) -> Subscription<Message> {
    keyboard::listen().filter_map(focus::tab).map(Message::Focus)
}
// in update: Message::Focus(direction) => return focus::advance(direction),
```

### Icons

An icon is an SVG path on the 24 grid (`icon::Icon::path`). The shared frame (1.7 stroke,
round caps and joins) comes from the tokens; `component::icon` draws it at the wanted size and
color. To add one: a variant of `Icon`, its path, a place in `Icon::ALL`. The test checks that
every path stays in the grid. A clickable icon on its own goes through `icon_button`, whose
label is mandatory.

### Adding a component

1. Missing values first go into tokens, in `color` or `metrics`, named by role.
2. Its style function by state goes into `theme`, without a literal color or size.
3. Its constructor goes into `component`. If it is clickable, it returns a `Control` to get the
   keyboard focus and the ring.
4. Every text it lays on a background enters the contrast test of `theme`.
5. It takes a row in the table of states of the gallery (`examples/gallery/view.rs`).

## Logo

The logo has neither text nor name. It is redrawn by hand on a 512 grid, after the
AI-generated board the author supplied, without automatic tracing:

| Shape | Value |
| --- | --- |
| Background square | 512, radius 96 |
| Ring | Center (256, 256), radius 140, width 36 |
| Tail | Arc of radius 80 and width 28, round caps, from (340, 368) on the ring to the dot |
| Dot | Center (308, 256), radius 22 |
| Version without square | Same drawing, framed on `96 96 320 320` |

The files are in `desktop/sherlock-ui/logo/`: `logo.svg` (green ring, cream dot, night blue
square), `logo-cream.svg` (cream on night blue), `logo-night.svg` (night blue on cream), each
also as `-transparent` (without the square) and, with the square, in the optical versions
`-32` and `-16`, and their PNGs at 16, 32, 64, 256 and 512 in `logo/png/`. The night blue is
`DESKTOP`, the green `ACCENT`, the cream `CREAM`; a test checks it. The SVGs and the PNGs are
written with `python3 export.py` from that folder (librsvg, already on the machine).

Deliberate gaps with the board: the tail of the board tapers to a point; here it keeps a
constant width, more legible when small. The dot has the same place in the three versions.

### Optical versions for 32 and 16 px

Drawn as is at 16 px, the ring of the version with a square is only one pixel and the tail
becomes a smudge. The logo with a square thus has two optical versions, written by the same
`export.py` (`logo-32.svg`, `logo-16.svg` and their cream and night equivalents): same
construction, symbol enlarged within the square, thicker ring, shorter and thicker tail.

| Shape | 32 px (1 px = 16 units) | 16 px (1 px = 32 units) |
| --- | --- | --- |
| Ring | Radius 175, width 50 (3 px) | Radius 180, width 56 (1.75 px) |
| Tail | From (361, 396), arc of radius 100, width 36 | From (364, 400), arc of radius 110, width 40 |
| Dot | Center (300, 248), radius 36 | Center (300, 252), radius 38 |
| Margin between the ring and the edge of the square | 31 units, 2 px | 20 units, 0.6 px |

The start of the tail stays on the ring, 53° below the horizontal (3-4-5 triangle); a test
checks it in every version. The 16 and 32 px PNGs with a square are taken from the optical
versions. In the code, `logo::Logo::square(size)` picks the right version. The version without
a square already fills its frame and reads at 16 px: it has no optical version.

Legibility board at 16 px, enlarged eight times without smoothing: the three versions drawn as
is, then the three optical versions.

![Legibility of the logo at 16 px](../desktop/sherlock-ui/logo/png/legibility-16.png)

The source board is not in the repository: it is AI-generated, irregular, weighs 760 KiB and
has no use left once the symbol is redrawn. Its measurements are in the table above.

## Gallery

```sh
cargo run -p sherlock-ui --example gallery
```

It shows each component in its five states (normal, hover, pressed, focus, disabled), on the
desktop background then on a glass panel: buttons, icon-only button, toggle on and off, menu
entry in its menu, empty and filled field. Then come the « Fenêtre » menu of the Menus board
and its submenu, two cards of the Reglages board, live buttons, and the logo in its three
versions, with a square then without, at 96, 64, 32 and 16 px (optical versions at 32 and 16).
Tab and Shift+Tab move the focus, Enter or Space activates.

The only sizes written in the gallery are those of its own layout, grouped in the `layout`
module of `examples/gallery/view.rs`: width of the table cells, names column, gap between
columns, display sizes of the logo. They are not values of the design.

Procedure to compare with the design boards, on the author's machine (Plasma X11):

1. Run the gallery. The window is 1440 by 900, like the design boards.
2. Open `docs/design/planches/Menus.dc.html` in an editor: the « Menu Fenêtre » menu reads in
   the inline styles. In the gallery, click « Déplacer vers le bureau ». Compare: menu 256
   wide, padding 6, entries of 30 and radius 7, shortcuts in `#A9B4C6` at 12 px, separators,
   submenu of 190 lined up with its entry and set 4 from the menu (`SUBMENU_GAP`), green check
   on « 1 · Code ».
3. Open `docs/design/planches/Reglages.dc.html` the same way. Compare the cards (`#152238`,
   radius 14, padding 16 and 18), the setting rows (13 px label weight 500, 12 px help in
   `#C5CEDB`, 48 high, `rgba(255,255,255,0.07)` line between two rows), the toggles (44 by 26,
   green, dark knob of 20), the secondary button 34 high, and the search field
   (`rgba(255,255,255,0.07)`, radius 9).
4. A pixel measurement is done on the snapshot of the test (below), at scale 2.

## Tests

`just verify` runs everything, without a screen:

- **Contrast** (`theme`): every text of every component, in each of its states, laid on every
  background where it can appear (desktop and its three discs, glass and menu laid on each,
  window, title bar, sidebar, card, system screen, code area, terminal block), must reach 4.5
  to 1. The computation reads the real styles, lays the translucent backgrounds and compares.
  Menu entries are checked inside menus. On top of that come the code texts (line numbers,
  blue keywords, green, amber, salmon) on the code backgrounds and the window, the texts on a
  selected row (`SELECTED_ROW`) in a window, a sidebar and a card, and on the active entry of a
  navigation (`ACTIVE_ENTRY`) on the sidebar. All pass without changing a token.
- **Token consistency**: design ranges checked at compile time, shades of the green, logo
  limited to the theme colors, icons inside their grid.
- **Gallery** (`tests/gallery.rs`): snapshot of the gallery render, submenu open, into
  `target/tmp/gallery-tiny-skia.png`, 2,400 high to hold everything; a click triggers its
  action; Tab walks the components skipping the disabled ones, Enter or Space activates them,
  a key held down activates only once, a disabled component loses the focus.

The snapshot is not compared with a reference image: it is written at each test run to be
looked at. A pixel-exact comparison would fail the CI at the slightest rendering change.

## Known gaps with the design boards

- **Blur**: the glass is only a translucent tint. The blur is the compositor's job.
- **Letter spacing**: iced 0.14 has none. Section labels are in uppercase without the 0.06 em
  of the board.
- **Menu selection**: the Menus board lays the open entry at 0.30 opacity, `docs/contexte.md` says
  0.26. At 0.30, a shortcut in tertiary grey on that row drops to 4.46, against 4.81 at 0.26:
  the token keeps 0.26. A test shows it.
- **Text field**: the placeholder is in secondary grey, not tertiary: the tertiary drops to
  4.25 on the field laid over the lightest disc. The board does not show the search icon in
  the field: iced 0.14 only accepts a font icon in `text_input`.
- **States missing from the boards**: hover, pressed, focus and disabled are drawn nowhere.
  Choices made: hover of the green lightened by 18 %, pressed darkened by 14 %; white veil of 7
  and 12 % on the elements without background; light blue focus ring `#8FC1E8`; disabled
  without background, tertiary text, `DIVIDER` border. The toggle that is off has no board:
  white track at 18 %, light grey knob.
- **Border of the secondary button**: 0.20 in the Reglages board, 0.18 in `docs/contexte.md` and in
  the « Panneaux » and « Fichiers » boards. The token keeps 0.18: it is the value of the reference text
  and of most boards.
- **Stroke of the menu icons**: the Menus board draws the chevron at 2 and the check at 2.4.
  The token keeps 1.7 for every icon: `docs/contexte.md` sets a single range, from 1.6 to 1.7, and a
  shared stroke keeps the icons consistent. At 14 px, the difference is a third of a pixel.
- **Animations**: iced changes state at once. The springs and the fade exist in `motion`
  (D-11), but the iced components do not use them yet: the hovers of 110 to 150 ms are not
  done.
- **Accessibility**: iced 0.14 exposes nothing to screen readers. The label of an icon-only
  button is mandatory in the API and shows as a tooltip on hover, until AccessKit (ADR 0014).
- **Text rendering**: cosmic-text antialiases in greyscale, without subpixel antialiasing; the
  600 weight comes out slightly bolder than in a browser (appendix of ADR 0014).

## Additions of D-27 (Evidence)

- **Tokens.** `metrics`: application window (`REFERENCE_WINDOW`, title bar paddings, app badge,
  window controls), toolbar, breadcrumb, branch chip, search field, status bar, sidebar and
  navigation entries, preview panel, code block, lists (`LIST_*`, column widths, overscan,
  slide), badges, dialog, context menu and submenu widths (236 and 216, « Fichiers » board).
  `color`: `TEXT_CODE`, `INSET`, `CONTROLS`, `SCRIM`, `AMBER_SOFT`, `ACCENT_SOFT`,
  `NEUTRAL_SOFT`, `ACCENT_BORDER`. `typography`: `CODE_SMALL_SIZE`, `COLUMN_HEADER_SIZE`,
  `BADGE_SIZE`, `UI_CHAR_WIDTH` (to cut names without shaping them).
- **Icons.** Back, sort arrows, minimise, maximise, home, desktop, document, download, folder,
  image, trash, cube (from the board); system disk (board); internal disk, USB key, external
  disk, card, optical disc, eject, lock (drawn for Evidence on the same grid and stroke).
  `component::icon` now builds each SVG handle once.
- **Texts.** New module `texts`: French interface texts and their formatting (`short_date`,
  `full_date`, `size`, `items`), and `texts::evidence` for the file manager.
- **Components.** `danger` (salmon menu entry), `danger_button`, `primary_block` and
  `secondary_block` (full width, centred label), `toolbar_button`, `search_field` (magnifier
  inside), `nav_item` and `nav_section`, `badge` (with `theme::Tone`), `branch_chip` (named
  apart from the bar's workspace `chip`), `window_controls`, `crumb`, `column_header`,
  `code_block` with `CodeRole` colouring.
  `theme::window_theme()` gives an application window whose body is the renderer's clear colour.
- **Animations.** `animated::Hoverable` fades a hover veil in and out over `motion::FADE` and
  requests its own frames while it runs. Every control without a resting background (menu
  entries, icon, secondary, window and navigation buttons) now uses it: its hover is animated.
  (`animated::Scaled`, which drew menus opening at a scale, was removed: see below.)
- **Shadows** (revised by D-02, see below). Menus and floating panels carry no iced shadow: iced 0.14's software renderer
  paints a blurred quad shadow as a separate image, outside the redrawn area's clip and
  without the current transformation, which broke the context menu of Evidence.
  `surface::Shadowed` draws a crisp shadow instead, from `metrics::MENU_SHADOW_LAYERS` and
  `PANEL_SHADOW_LAYERS` (steps of offset, spread and opacity of `color::SHADOW_BASE`).
  `component::floating_menu` (a menu over a window, such as a context menu) and
  `component::floating_panel` (a glass panel of a given width: dialogs) use it;
  `component::menu`, which a surface of its own fills (the bar's menus), has no shadow. A test checks that no surface style has an iced shadow.
- **Tests.** The contrast test covers the new components, badges on a selected row, the code
  text, the destructive button (its pressed state keeps the hover veil: the lighter veil would
  drop the salmon to 4.39:1 on the lightest desktop disc). The gallery shows the destructive
  button, the toolbar button, the destructive menu entry and the navigation entries, and its
  snapshot is now 2,400 pixels high.

## Additions of D-02 (GPU rendering)

- **`gpu`**: the choice of the renderer, see Renderer. Tested without a GPU: the reference
  laptop picks RADV's manifests only, no AMD chip or no Mesa Vulkan driver gives software, a
  user's `ICED_BACKEND` wins, chips are read from a fake `/sys/class/drm`, children get the
  environment back.
- **Shadows.** `surface::Shadowed` takes an `Elevation` (`Flat`, `Menu`, `Panel`). On the GPU it
  draws the design board's blurred shadow (`color::MENU_SHADOW` or `SHADOW`,
  `metrics::MENU_SHADOW_OFFSET` and `_BLUR`, `SHADOW_OFFSET` and `_BLUR`); in software it keeps
  the crisp steps of D-27. `Elevation::margins` gives the room the blurred shadow takes around a
  surface. `Flat` draws nothing and only records the renderer.
- **Gallery test**: its renderer is created headless with tiny-skia named explicitly.

## What the lot does not do

- Animated components: the springs and the fade are in `motion` since D-11, to be wired to the
  hovers and the transitions of the components.
- The menu that opens as an `xdg_popup`: the gallery lays the submenu next to the menu, like
  the board.
- (D-20) layer-shell and the real placement of a menu live in the bar: see `docs/bar.md`,
  which also records what was learnt about `iced_exwlshell` 0.21. The bar components (`chip`,
  `sherlock_button`, `status`, `menu_header`) are not in the gallery: the bar has its own
  headless screenshot (`desktop/bar/tests/render.rs`).
