# ADR 0029: application menus as Wayland popups

- Status: proposed
- Date: 2026-11-27
- Lot: E-02

## Context

The author wants the menus of our applications to leave the window, as on macOS. That covers the
right-click menu, its submenus, the drop-down lists, the path bar's completion list (E-04) and
the tooltips. Today Evidence draws them inside its own window. A menu opened near an edge is
pushed back inside, and a submenu flips to the left. In a small window a menu that needs more
room than the window has is cut.

The right tool is `xdg_popup` placed with `xdg_positioner`. The compositor places the popup
against the screen, not the window. It flips it (`flip_x`, `flip_y`) and slides it (`slide_x`,
`slide_y`) at the edges of the output. Our compositor already handles popups: the bar uses
them for its panels and menus (D-20, D-36).

What constrains the choice:

- Evidence runs on `iced::application`, that is iced 0.14 on winit 0.30. winit makes the
  window's `xdg_toplevel` but offers no way to make an `xdg_popup` of it.
- The shell programs (bar, dock, launcher, notifications, lock) run on `iced_exwlshell` 0.21.4,
  already in the workspace. Its `daemon` can make an `xdg_toplevel` window
  (`ExWlShellCustomAction::NewBaseWindow`: app id, client-side decorations) and popups whose
  parent is that window (`NewPopUp`, with anchor, gravity, `constraint_adjustment` flip and
  slide, and the grab serial of the click). Started with `StartMode::Background`, it opens no
  layer surface: the window is the program's only surface. It also turns iced's `window::drag`,
  `toggle_maximize` and `minimize` into `xdg_toplevel.move`, `set_maximized` and
  `set_minimized`, which Evidence's own title bar needs.
- winit could lend its `wl_surface` through `raw-window-handle`, and a popup could be made
  beside it with `wayland-client`. That popup would then need its own iced renderer, surface
  and event loop, written by us: a second toolkit inside the program.

## Decision

Our applications that need menus outside their window run on `iced_exwlshell`. They use an
`xdg_toplevel` window made with `NewBaseWindow`, and one `xdg_popup` per menu, submenu, list or
tooltip, made with `NewPopUp` and parented to the window, or to the menu for a submenu. The
anchor is the clicked point or the opening control. The constraint adjustment is
`FlipX | FlipY | SlideX | SlideY`. The submenu anchors to its entry's row, so the compositor
flips it to the left at the screen's edge.

Evidence moves first, in three stacked lots of about 600 lines each:

1. **E-02a**: Evidence runs on `iced_exwlshell` with an `xdg_toplevel`. The menus stay drawn
   inside the window. The window, its title bar (move, maximize, minimize, close), its size
   limits, its app id and the keyboard behave exactly as today. The headless scenarios keep
   driving `App::update` and rendering `App::view` for the main window.
2. **E-02b**: the right-click menu and its submenus become popups. The opening and closing
   animation (`MenuMotion`) plays inside the popup, and the popup closes once it has folded
   back, as the bar's menus do. The grab gives the menu the keyboard. A click outside
   (`popup_done`) closes it.
3. **E-02c**: the path bar's completion list, the drop-down lists and the tooltips become
   popups. These are made without a grab, so the field keeps the keyboard.

The window keeps drawing a menu inside itself when the compositor has no `xdg_wm_base` popups,
which does not happen on a Wayland desktop. Under X11 (a developer's machine), Evidence keeps
running on `iced::application` behind a cargo feature, with the menus drawn inside.

## Consequences

- Menus, submenus and lists are never cut at the window's edge, whatever its size. Near the
  screen's edge, the compositor places them.
- The code that keeps a menu inside the window goes away: `menu::submenu_at`, and the clamp
  in `open_menu`. So do the limits it imposed on the window's smallest size (`MIN_WINDOW` was
  sized so that the menu and the chooser fit).
- Each popup is a window of the daemon: `view(id)` draws the main window or one menu. The
  tests render each one apart, so they need a small change of their harness.
- One more program depends on `iced_exwlshell`, which we already follow for six others.
- Control Room and Inquiry can follow the same path later. This ADR does not move them.

## What would change our mind

- If `iced_exwlshell`'s `xdg_toplevel` window turns out less complete than winit's, a spike in
  E-02a will show it: resizing from the edges, the IME, drag and drop (E-03 needs it) or the
  fractional scale. Then the popups would be made beside winit's surface instead (see Context).
  That costs more, but it leaves the main window on winit.
- A release of iced or winit that supports `xdg_popup` would make
  the move unnecessary: Evidence would stay on `iced::application` and use that API.
