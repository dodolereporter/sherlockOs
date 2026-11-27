//! Keyboard and mouse: click to focus, move and resize gestures, snapping, shortcuts and cursor
//! shape.
//!
//! A gesture comes from the gesture key (see [`gesture_modifier_held`]) with a button, from a
//! press in the band around a window, or from a client request (`xdg_toplevel.move` or
//! `.resize`, its title bar or its edges).

use sherlock_ui::window as ui;
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, Switch, SwitchState,
        SwitchToggleEvent,
    },
    desktop::{Window, WindowSurfaceType},
    input::{
        keyboard::{FilterResult, Keycode, Keysym, ModifiersState},
        pointer::{
            AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, GrabStartData, MotionEvent,
        },
    },
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState,
        wayland_server::{Resource, protocol::wl_surface::WlSurface},
    },
    utils::{Logical, Physical, Point, Rectangle, SERIAL_COUNTER, Size},
    wayland::{
        compositor::with_states,
        shell::{wlr_layer::Layer, xdg::SurfaceCachedState},
    },
};

use crate::{
    State, capture,
    decorations::{self, Control},
    handlers::{Arrangement, is_fullscreen, is_maximized},
    is_override_redirect,
    layout::{self, Edges, Resize, Zone},
    media_keys::MediaKey,
    window_data,
};

// Button codes, taken from linux/input-event-codes.h.
pub(crate) const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Gesture key of the compositor. Super in a real session. In nested mode, the host session
/// keeps Super for itself (under Plasma: KWin window actions, launcher), so it is Alt, as niri
/// does.
pub(crate) fn gesture_modifier_held(nested: bool, modifiers: &ModifiersState) -> bool {
    if nested {
        modifiers.alt
    } else {
        modifiers.logo
    }
}

/// The gesture key is the only one held: Alt + Shift + arrow, for example, stays with the client.
fn gesture_modifier_alone(nested: bool, m: &ModifiersState) -> bool {
    let others = [m.ctrl, m.shift, if nested { m.logo } else { m.alt }];
    gesture_modifier_held(nested, m) && !others.contains(&true)
}

/// Where a gesture comes from, for the log.
#[derive(Debug, Clone, Copy)]
pub(crate) enum GestureSource {
    /// Gesture key and mouse button.
    Modifier,
    /// Press in the grab band around the window.
    Border,
    /// Client request: `xdg_toplevel.move` or `xdg_toplevel.resize`.
    Client,
    /// Drag of the title bar the compositor draws (lot D-08).
    TitleBar,
}

/// Gesture in progress: the compositor moves or resizes a window.
pub(crate) struct Gesture {
    window: Window,
    source: GestureSource,
    /// Button that started the gesture: only its release ends it.
    button: u32,
    start: Point<f64, Logical>,
    initial: Rectangle<i32, Logical>,
    /// `None` for a move, the pulled edges for a resize.
    edges: Option<Edges>,
    last_size: Size<i32, Logical>,
    /// The pointer has moved enough to move the window. A mere click does not shift it.
    moved: bool,
    /// Floating place of the window at the start of the move: the one it will return to if it
    /// gets snapped. A zero size lets the client choose.
    floating: Rectangle<i32, Logical>,
    /// Zone where the moved window will be snapped if dropped now, and its screen.
    zone: Option<(Zone, Output)>,
}

/// What is under the pointer.
pub(crate) enum Hit {
    /// A client surface, with the position of its origin.
    Surface(Window, WlSurface, Point<f64, Logical>),
    /// The grab band around a window, and the edges it pulls.
    Border(Window, Edges),
    /// The title bar the compositor draws above a window, or one of its buttons.
    Decoration(Window, Control),
    /// An anchored surface (or one of its menus or subsurfaces): its main surface, the surface
    /// hit and the position of its origin.
    Layer(WlSurface, WlSurface, Point<f64, Logical>),
}

/// Compositor shortcut: gesture key and an arrow or a digit, or a key of the session.
#[derive(Debug, Clone, Copy)]
enum Shortcut {
    /// 1 to 9: goes to that workspace (its index, from 0).
    Workspace(usize),
    /// Shift + 1 to 9: sends the window that has the keyboard to that workspace.
    SendToWorkspace(usize),
    /// Ctrl + Left or Right arrow: goes to the workspace on that side.
    NeighborWorkspace(isize),
    /// Up arrow: maximizes, or restores the previous place.
    ToggleMaximized,
    /// Left or right arrow: half of the screen.
    Snap(Zone),
    /// Down arrow: restores the previous place.
    Restore,
    /// Tab: opens or closes Mission Control (lot D-33).
    ToggleOverview,
    /// Space: starts the launcher, or closes it (lot C-01).
    Launcher,
    /// Super + V: the clipboard history (lot D-42).
    ClipboardHistory,
    /// F4 alone: opens or closes the applications view (lot D-41), as on macOS.
    Applications,
    /// Escape while Mission Control is open: closes it.
    CloseOverview,
    /// Ctrl + Alt + F1 to F12, in a real session: switches to that console.
    SwitchVt(i32),
    /// Ctrl + Alt + Backspace, in a real session and unlocked: stops the compositor, the way
    /// out when nothing else answers. Never while locked (lot D-25j).
    Quit,
    /// A media key, in a real session, with its xkb code (lot D-35).
    Media(MediaKey, Keycode),
    /// Print Screen, with Shift or Alt, in a real session (lot D-35c).
    Screenshot(capture::Kind),
    /// Super + Shift + S (Alt + Shift + S nested): the screenshot tool (lot D-35e).
    ScreenshotTool,
    /// A key while the screenshot tool is open: every key stays with it.
    ToolKey(Keysym),
    /// L: locks the session (lot D-25a).
    Lock,
    /// Enter on the rescue screen of the lock: the lock programs are tried again.
    RetryLock,
}

impl Shortcut {
    /// Keys of the session, which never reach a client. xkb turns Ctrl + Alt + Fn into
    /// `XF86Switch_VT_n`.
    fn for_session(nested: bool, m: &ModifiersState, sym: Keysym) -> Option<Self> {
        let vt = sym.raw().wrapping_sub(Keysym::XF86_Switch_VT_1.raw());
        match sym {
            _ if nested => None,
            Keysym::BackSpace | Keysym::Terminate_Server if m.ctrl && m.alt => Some(Self::Quit),
            _ if vt < 12 => Some(Self::SwitchVt(vt as i32 + 1)),
            _ => None,
        }
    }

    /// Workspace keys: the gesture key with 1 to 9, Shift + 1 to 9, Ctrl + Left or Right.
    /// The digits are the keys of the top row, whatever the layout: on AZERTY they give
    /// `&`, `é`, `"`… without Shift.
    fn for_workspaces(nested: bool, m: &ModifiersState, key: Keycode, sym: Keysym) -> Option<Self> {
        let other = if nested { m.logo } else { m.alt };
        if !gesture_modifier_held(nested, m) || other {
            return None;
        }
        // xkb codes of the keys 1 to 9 of the top row: evdev KEY_1 (2) to KEY_9 (10), plus 8.
        let digit = (10..=18)
            .contains(&key.raw())
            .then(|| key.raw() as usize - 10);
        match (m.ctrl, m.shift, digit, sym) {
            (false, false, Some(index), _) => Some(Self::Workspace(index)),
            (false, true, Some(index), _) => Some(Self::SendToWorkspace(index)),
            (true, false, None, Keysym::Left) => Some(Self::NeighborWorkspace(-1)),
            (true, false, None, Keysym::Right) => Some(Self::NeighborWorkspace(1)),
            _ => None,
        }
    }

    /// F4 without any modifier: the applications view (lot D-41).
    fn for_applications(m: &ModifiersState, sym: Keysym) -> Option<Self> {
        let alone = !m.ctrl && !m.alt && !m.shift && !m.logo;
        (sym == Keysym::F4 && alone).then_some(Self::Applications)
    }

    fn for_key(sym: Keysym) -> Option<Self> {
        match sym {
            Keysym::Up => Some(Self::ToggleMaximized),
            Keysym::Left => Some(Self::Snap(Zone::Left)),
            Keysym::Right => Some(Self::Snap(Zone::Right)),
            Keysym::Down => Some(Self::Restore),
            Keysym::Tab => Some(Self::ToggleOverview),
            Keysym::space => Some(Self::Launcher),
            Keysym::v => Some(Self::ClipboardHistory),
            Keysym::l => Some(Self::Lock),
            _ => None,
        }
    }

    /// The only keys the compositor takes while the session is locked: the console switch,
    /// the volume and brightness keys, and Enter on the rescue screen. Every other key goes to
    /// the lock surface, or nowhere: Ctrl + Alt + Backspace too, which would close the session
    /// of anyone at the machine, and give back the console it was started from (lot D-25j).
    fn while_locked(state: &State, m: &ModifiersState, key: Keycode, sym: Keysym) -> Option<Self> {
        let media =
            MediaKey::of(sym).filter(|media| media.acts_while_locked() && state.takes_media_keys());
        let retry = matches!(sym, Keysym::Return | Keysym::KP_Enter) && state.lock_rescue_shown();
        Self::for_session(state.nested, m, sym)
            .filter(|shortcut| !matches!(shortcut, Self::Quit))
            .or(media.map(|media| Self::Media(media, key)))
            .or(retry.then_some(Self::RetryLock))
    }
}

/// Takes the window out of maximized or of its zone. Returns its previous place if it had one
/// (of zero size if it had no content), `None` if it was floating.
fn leave_arrangement(window: &Window) -> Option<Rectangle<i32, Logical>> {
    let data = window_data(window);
    let was_zoned = data.zone.take().is_some();
    let mut arrangement = Arrangement::of(window);
    let was_maximized = std::mem::take(&mut arrangement.maximized);
    arrangement.apply(window);
    let restore = data.restore.take();
    (was_zoned || was_maximized).then(|| restore.unwrap_or_default())
}

/// Minimum and maximum sizes the client accepts, zero if it gives none.
fn size_limits(window: &Window) -> (Size<i32, Logical>, Size<i32, Logical>) {
    if let Some(surface) = window.x11_surface() {
        return (
            surface.min_size().unwrap_or_default(),
            surface.max_size().unwrap_or_default(),
        );
    }
    let Some(toplevel) = window.toplevel() else {
        return Default::default();
    };
    with_states(toplevel.wl_surface(), |states| {
        let mut cached = states.cached_state.get::<SurfaceCachedState>();
        (cached.current().min_size, cached.current().max_size)
    })
}

/// Cursor shape to pull these edges.
fn resize_cursor(edges: Edges) -> CursorIcon {
    match (edges.top, edges.bottom, edges.left, edges.right) {
        (true, _, true, _) => CursorIcon::NwResize,
        (true, _, _, true) => CursorIcon::NeResize,
        (_, true, true, _) => CursorIcon::SwResize,
        (_, true, _, true) => CursorIcon::SeResize,
        (true, ..) => CursorIcon::NResize,
        (_, true, ..) => CursorIcon::SResize,
        (_, _, true, _) => CursorIcon::WResize,
        _ => CursorIcon::EResize,
    }
}

impl State {
    /// Starts a gesture on `window`, from the pointer position `start` and the press of
    /// `button`. `edges` is `None` for a move. A fullscreen window does not move.
    pub(crate) fn start_gesture(
        &mut self,
        window: Window,
        start: Point<f64, Logical>,
        button: u32,
        edges: Option<Edges>,
        source: GestureSource,
    ) {
        if is_fullscreen(&window) {
            tracing::debug!(?source, "gesture refused: the window is fullscreen");
            return;
        }
        // The window follows the pointer without delay: a running animation stops, and the
        // return to the previous size of a snapped window being pulled out is not animated.
        window_data(&window).motion.borrow_mut().follow();
        if let Some(edges) = edges {
            // When resized, it leaves maximized or its zone and starts from its current size,
            // which goes to the client with the first motion.
            leave_arrangement(&window);
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
            let data = window_data(&window);
            if let Some(initial) = self.space.element_geometry(&window) {
                // Realigning on the opposite edge holds for the whole gesture.
                data.resize.set(Some(Resize {
                    edges,
                    initial,
                    done: false,
                }));
            }
        }
        let Some(initial) = self.space.element_geometry(&window) else {
            return;
        };
        tracing::debug!(
            ?source,
            ?edges,
            ?initial,
            "gesture start (empty edges: move)"
        );
        self.gesture = Some(Gesture {
            window,
            source,
            button,
            start,
            initial,
            edges,
            last_size: initial.size,
            moved: false,
            floating: initial,
            zone: None,
        });
    }

    fn update_gesture(&mut self, pointer: Point<f64, Logical>) {
        if let Some(mut gesture) = self.gesture.take() {
            self.drive_gesture(&mut gesture, pointer);
            self.gesture = Some(gesture);
        }
    }

    fn drive_gesture(&mut self, gesture: &mut Gesture, pointer: Point<f64, Logical>) {
        let Some(edges) = gesture.edges else {
            let delta = pointer - gesture.start;
            let threshold = f64::from(ui::DRAG_THRESHOLD);
            if !gesture.moved && delta.x.hypot(delta.y) < threshold {
                return;
            }
            if !gesture.moved {
                gesture.moved = true;
                self.detach_for_move(gesture);
            }
            let loc = layout::moved(gesture.initial.loc, gesture.start, pointer);
            let loc = self.below_top_bar(&gesture.window, loc, pointer);
            self.move_window(&gesture.window, loc, true);
            // The screen under the pointer: its edges snap, its work area receives the window.
            gesture.zone = self.space.output_under(pointer).next().and_then(|output| {
                let area = self.space.output_geometry(output)?;
                let zone = Zone::under(area, pointer, ui::SNAP_EDGE, ui::SNAP_CORNER)?;
                Some((zone, output.clone()))
            });
            return;
        };
        let (min, max) = size_limits(&gesture.window);
        let mut delta = pointer - gesture.start;
        // The top edge stops under the top bar, like a move.
        if edges.top
            && let Some(top) = self.top_limit(&gesture.window, pointer)
        {
            delta.y = delta.y.max(f64::from(top - gesture.initial.loc.y).min(0.0));
        }
        gesture.last_size = layout::resized(gesture.initial, edges, delta, min, max);
        match gesture.window.toplevel() {
            Some(toplevel) => {
                toplevel.with_pending_state(|s| {
                    s.states.set(XdgState::Resizing);
                    s.size = Some(gesture.last_size);
                });
                toplevel.send_pending_configure();
            }
            None => self.resize_x11(&gesture.window, gesture.last_size),
        }
    }

    /// An X11 window takes its size as soon as it is given: it is placed right away, the edge
    /// opposite the pulled one staying in place.
    fn resize_x11(&mut self, window: &Window, size: Size<i32, Logical>) {
        let data = window_data(window);
        let Some(resize) = data.resize.get() else {
            return;
        };
        let (loc, next) = resize.on_commit(size);
        data.resize.set(next);
        self.request_size(window, Some(size));
        self.move_window(window, loc, false);
    }

    /// First real motion of a move: a maximized or snapped window gets its previous size back
    /// under the pointer. If it had none (maximized before its first content), the client
    /// chooses.
    fn detach_for_move(&mut self, gesture: &mut Gesture) {
        let Some(restore) = leave_arrangement(&gesture.window) else {
            return;
        };
        let size = (!restore.size.is_empty()).then_some(restore.size);
        self.request_size(&gesture.window, size);
        if let Some(size) = size {
            // The pointer keeps its place in the whole window, title bar included.
            let inset = decorations::inset(&gesture.window);
            let frame = decorations::frame(&gesture.window, gesture.initial);
            let whole = Size::from((size.w, size.h + inset));
            let loc = layout::unsnapped(frame, whole, gesture.start) + Point::from((0, inset));
            let loc = self.below_top_bar(&gesture.window, loc, gesture.start);
            gesture.initial = Rectangle::new(loc, size);
            self.move_window(&gesture.window, loc, true);
        }
        gesture.floating = Rectangle::new(gesture.initial.loc, restore.size);
        if let Some(toplevel) = gesture.window.toplevel() {
            toplevel.send_pending_configure();
        }
    }

    /// The client leaves the maximized state itself (`unset_maximized`, or `_NET_WM_STATE` for
    /// X11). Some clients do so when their title bar is pulled, before or after asking for the
    /// move: while the window is being dragged, it comes off under the pointer at its previous
    /// size, as a move takes it out, instead of going back to its place from before it was
    /// maximized (lot D-18d). Dragged means a move of this window (the client's or the gesture
    /// key's) or a button held on it, with the pointer past the drag threshold from the press:
    /// a double click on a title bar that the client draws (GTK 4 unmaximizes at the second
    /// press) restores the window to its place, as any other unmaximize does.
    pub(crate) fn client_unmaximize(&mut self, window: &Window) {
        if self.dragged(window) {
            match self.gesture.take() {
                Some(mut gesture) => {
                    self.detach_for_move(&mut gesture);
                    self.gesture = Some(gesture);
                }
                None => self.detach_held(window),
            }
            return;
        }
        self.rearrange(window, |a, zone| {
            a.maximized = false;
            zone.set(None);
        });
        // A move that has not shifted the window yet goes on from its restored place.
        if let Some(gesture) = self.gesture.as_mut().filter(|g| g.window == *window)
            && let Some(rect) = self.space.element_geometry(window)
        {
            gesture.initial = rect;
        }
    }

    /// `window` is being dragged: a move of it, or a button held on it, and the pointer past
    /// the drag threshold from where the button was pressed.
    fn dragged(&self, window: &Window) -> bool {
        let Some(pointer) = self.seat.get_pointer() else {
            return false;
        };
        let start = match &self.gesture {
            Some(gesture) if gesture.window == *window && gesture.edges.is_none() => {
                Some(gesture.start)
            }
            Some(_) => None,
            None => self.held_from(window).map(|start| start.location),
        };
        start.is_some_and(|start| {
            let delta = pointer.current_location() - start;
            delta.x.hypot(delta.y) >= f64::from(ui::DRAG_THRESHOLD)
        })
    }

    /// The press that holds a button on `window`, if one does.
    fn held_from(&self, window: &Window) -> Option<GrabStartData<State>> {
        let pointer = self.seat.get_pointer().filter(|p| p.is_grabbed())?;
        let start = pointer.grab_start_data()?;
        let focus = start.focus.as_ref().map(|(focus, _)| focus);
        let own = smithay::wayland::seat::WaylandFocus::wl_surface(window);
        (focus.is_some() && focus == own.as_deref()).then_some(start)
    }

    /// A window held by a button and dragged, before the client asks for the move: it comes
    /// off under the point where the button was pressed, at its previous size.
    fn detach_held(&mut self, window: &Window) {
        let Some(start) = self.held_from(window) else {
            return;
        };
        let current = self.space.element_geometry(window);
        let Some(restore) = leave_arrangement(window) else {
            return;
        };
        window_data(window).motion.borrow_mut().follow();
        let size = (!restore.size.is_empty()).then_some(restore.size);
        self.request_size(window, size);
        if let (Some(size), Some(current)) = (size, current) {
            let loc = layout::unsnapped(current, size, start.location);
            self.move_window(window, loc, false);
        }
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_pending_configure();
        }
    }

    fn finish_gesture(&mut self, gesture: Gesture) {
        let Gesture {
            window,
            source,
            button: gesture_button,
            initial,
            edges,
            last_size,
            moved,
            floating,
            zone,
            ..
        } = gesture;
        tracing::debug!(?source, ?edges, size = ?last_size, ?zone, "gesture end");
        if let (GestureSource::Modifier, None, BTN_LEFT, false) =
            (source, edges, gesture_button, moved)
        {
            // Super + click: Sherlock will explain what is under the pointer. The action is
            // reserved: the shape shows it, the agent is not plugged in yet.
            tracing::info!("gesture key + click: for Sherlock, not plugged in yet");
        }
        let Some(edges) = edges else {
            if let Some((zone, output)) = zone {
                // Dropped against an edge: snapped. It will return to its place from before the
                // drag.
                window_data(&window).restore.set(Some(floating));
                self.snap(&window, Some(zone), Some(output));
            } else if let Some(rect) = self.space.element_geometry(&window) {
                // Dropped: its title bar stays reachable, whatever moved it.
                let pointer = self.seat.get_pointer().map(|p| p.current_location());
                let at = pointer.unwrap_or_else(|| rect.loc.to_f64());
                let loc = self.below_top_bar(&window, rect.loc, at);
                if loc != rect.loc {
                    self.move_window(&window, loc, false);
                }
            }
            return;
        };
        // The position is realigned on the client's next commit, the last of the gesture.
        window_data(&window).resize.set(Some(Resize {
            edges,
            initial,
            done: true,
        }));
        match window.toplevel() {
            Some(toplevel) => {
                toplevel.with_pending_state(|s| {
                    s.states.unset(XdgState::Resizing);
                    s.size = Some(last_size);
                });
                toplevel.send_pending_configure();
            }
            None => self.resize_x11(&window, last_size),
        }
    }

    /// Forgets the shortcut keys waiting for their release: the host window has lost the
    /// keyboard, and these releases will not come.
    /// The area of a screenshot being chosen goes too (lot D-35c): the real session calls it
    /// when the console is left.
    pub fn forget_intercepted_keys(&mut self) {
        self.intercepted_keys.clear();
        self.cancel_area();
        self.stop_media_repeat(None);
    }

    /// `loc`, the place of the content of `window`, moved down if needed so that the top of the
    /// window, its title bar when the compositor draws one, stays below the anchored surfaces
    /// at the top of the screen under `at`: as on macOS, a window never goes under the top bar,
    /// where it could no longer be grabbed (lot C-01).
    pub(crate) fn below_top_bar(
        &self,
        window: &Window,
        loc: Point<i32, Logical>,
        at: Point<f64, Logical>,
    ) -> Point<i32, Logical> {
        match self.top_limit(window, at) {
            Some(top) if loc.y < top => (loc.x, top).into(),
            _ => loc,
        }
    }

    /// Highest place of the content of `window` on the screen under `at` (or its own): the top
    /// of its work area, plus its title bar. `None` for menus and fullscreen windows, and when
    /// another screen lies just above: the window may go there.
    pub(crate) fn top_limit(&self, window: &Window, at: Point<f64, Logical>) -> Option<i32> {
        if is_override_redirect(window) || is_fullscreen(window) {
            return None;
        }
        let output = self
            .space
            .output_under(at)
            .next()
            .cloned()
            .or_else(|| self.window_output(window))?;
        let screen = self.space.output_geometry(&output)?;
        let x = at.x.clamp(
            f64::from(screen.loc.x),
            f64::from(screen.loc.x + screen.size.w - 1),
        );
        let above = Point::from((x, f64::from(screen.loc.y) - 1.0));
        if self.space.output_under(above).next().is_some() {
            return None;
        }
        Some(self.work_area(&output)?.loc.y + decorations::inset(window))
    }

    /// A move or resize gesture is in progress.
    pub fn gesture_active(&self) -> bool {
        self.gesture.is_some()
    }

    /// Window being moved or resized.
    pub(crate) fn gesture_window(&self) -> Option<Window> {
        self.gesture.as_ref().map(|g| g.window.clone())
    }

    /// Rectangle where the moved window will be snapped if dropped now, for the preview.
    pub fn snap_preview(&self) -> Option<Rectangle<i32, Logical>> {
        let (zone, output) = self.gesture.as_ref()?.zone.as_ref()?;
        Some(zone.geometry(self.work_area(output)?, self.snap_gap))
    }

    /// Cursor shape, `None` to hide it. During a gesture and over the grab band, the compositor
    /// chooses it. Over a client, the client does, if it names it (cursor shape protocol); if it
    /// draws its cursor itself, the arrow.
    pub fn cursor_icon(&self) -> Option<CursorIcon> {
        if self.picking_area() {
            return self.tool_cursor();
        }
        if self.overview_open() {
            return Some(CursorIcon::Default);
        }
        if let Some(gesture) = &self.gesture {
            // A window taken by its title bar is held in the closed hand; moved with the
            // gesture key, the four arrows of the boards (« Super + glisser sur une fenêtre »).
            let moving = match gesture.source {
                GestureSource::Modifier => CursorIcon::Move,
                _ => CursorIcon::Grabbing,
            };
            return Some(gesture.edges.map_or(moving, resize_cursor));
        }
        let location = self.seat.get_pointer()?.current_location();
        // Locked: the shape the lock surface under the pointer names, else the arrow. Never
        // the edges of the windows hidden behind it (lot D-25a).
        if self.is_locked() {
            return match (self.lock_hit(location), &self.cursor_status) {
                (Some(_), CursorImageStatus::Named(icon)) => Some(*icon),
                (Some(_), CursorImageStatus::Hidden) => None,
                _ => Some(CursorIcon::Default),
            };
        }
        match (self.hit(location), &self.cursor_status) {
            (Some(Hit::Border(_, edges)), _) => Some(resize_cursor(edges)),
            (Some(Hit::Surface(..) | Hit::Layer(..)), CursorImageStatus::Named(icon)) => {
                Some(*icon)
            }
            (Some(Hit::Surface(..) | Hit::Layer(..)), CursorImageStatus::Hidden) => None,
            _ => Some(CursorIcon::Default),
        }
    }

    /// The shape the real session draws: `cursor_icon`, or the Sherlock cursor while the
    /// gesture key and the left button are held on a window that has not moved yet. Released
    /// there, it is a Super + click, which asks Sherlock about what is under the pointer
    /// (cursor boards); dragged, it becomes the move of the window.
    pub fn cursor_shape(&self) -> Option<crate::cursor::Shape> {
        let sherlock = self.gesture.as_ref().is_some_and(|g| {
            matches!(g.source, GestureSource::Modifier)
                && g.edges.is_none()
                && g.button == BTN_LEFT
                && !g.moved
        });
        match sherlock {
            true => Some(crate::cursor::Shape::Sherlock),
            false => self.cursor_icon().map(crate::cursor::Shape::Icon),
        }
    }

    /// What is under the pointer at `pos`. The band of a window comes before the windows it
    /// covers, not before those above it. Overlay and top anchored
    /// surfaces come before the windows, bottom and background ones after. A fullscreen
    /// window covers the top layer, unless its edge brought it back (lot D-21e).
    pub(crate) fn hit(&self, pos: Point<f64, Logical>) -> Option<Hit> {
        self.layer_hit(pos, &[Layer::Overlay])
            .or_else(|| self.top_hit(pos, true))
            .or_else(|| self.window_hit(pos))
            .or_else(|| self.top_hit(pos, false))
            .or_else(|| self.layer_hit(pos, &[Layer::Bottom, Layer::Background]))
    }

    /// What a press with the gesture key reaches at `pos`: the window under the top bar or
    /// the dock themselves, the way back for a window lost there. The overlay layer (lock
    /// screen), and the panels and menus of the top layer (the popups of the bar), keep their
    /// presses.
    fn gesture_hit(&self, pos: Point<f64, Logical>) -> Option<Hit> {
        if let Some(hit) = self.layer_hit(pos, &[Layer::Overlay]) {
            return Some(hit);
        }
        match self.top_hit(pos, true) {
            Some(Hit::Layer(main, surface, origin)) if main != surface => {
                Some(Hit::Layer(main, surface, origin))
            }
            top => self.window_hit(pos).or(top).or_else(|| self.hit(pos)),
        }
    }

    fn window_hit(&self, pos: Point<f64, Logical>) -> Option<Hit> {
        // Hidden windows (other workspaces, minimized) are skipped, which the space does not do,
        // and so are those of a minimize effect, whose drawing is not their place (lot D-19).
        let now = self.now();
        // A window without content (an X11 window before its first buffer) is not drawn: it
        // takes no click either.
        let shown = |w: &&Window| {
            self.is_visible(w)
                && self.minimize_progress(w, now).is_none()
                && (crate::has_buffer(w) || is_override_redirect(w))
        };
        for window in self.space.elements().rev().filter(shown) {
            let loc = self
                .space
                .element_location(window)
                .map(|l| l - window.geometry().loc);
            let under = loc.and_then(|loc| {
                let surface = window.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL);
                surface.map(|(surface, origin)| (surface, origin + loc))
            });
            if let Some((surface, origin)) = under {
                return Some(Hit::Surface(window.clone(), surface, origin.to_f64()));
            }
            if let Some(control) = self.decoration_at(window, pos) {
                return Some(Hit::Decoration(window.clone(), control));
            }
            // The band surrounds the whole window, its title bar included.
            let edges = self
                .space
                .element_geometry(window)
                .map(|r| decorations::frame(window, r))
                .filter(|r| {
                    !r.size.is_empty() && !is_fullscreen(window) && !is_override_redirect(window)
                })
                .and_then(|r| layout::border_edges(r, pos, ui::RESIZE_BAND, ui::RESIZE_CORNER));
            if let Some(edges) = edges {
                return Some(Hit::Border(window.clone(), edges));
            }
        }
        None
    }

    /// The key types a character, whatever the layout: a key of the main block or of the
    /// keypad, without Ctrl, Alt nor the logo key (lot D-25g).
    fn key_types(&self, key: Keycode) -> bool {
        let held = self
            .seat
            .get_keyboard()
            .map(|k| k.modifier_state())
            .is_some_and(|m| m.ctrl || m.alt || m.logo);
        // evdev codes, plus 8: 1 to =, Q to ], A to `, \ to /, space, the key left of Z, the
        // keypad's digits and signs.
        let typing = [
            (2, 13),
            (16, 27),
            (30, 41),
            (43, 53),
            (57, 57),
            (86, 86),
            (71, 83),
        ];
        let code = key.raw().wrapping_sub(8);
        !held && typing.iter().any(|(a, b)| (*a..=*b).contains(&code))
    }

    /// A key (xkb code) is pressed or released. Entry point of every backend, and of the
    /// tests.
    pub fn keyboard_key(&mut self, key: Keycode, key_state: KeyState, time: u32) {
        self.queue_redraw();
        self.note_input();
        // Locked: the lock program must answer, and show what is typed (lots D-25f, D-25g).
        if key_state == KeyState::Pressed {
            self.lock_input(self.key_types(key));
        }
        // A media key held repeats until its release, or until another key (lot D-35).
        self.stop_media_repeat((key_state == KeyState::Released).then_some(key));
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        let shortcut = keyboard.input(self, key, key_state, serial, time, |state, mods, sym| {
            let sym = sym.modified_sym();
            // The gesture key pressed and released alone toggles Mission Control (lot D-33).
            let other = if state.nested { mods.logo } else { mods.alt };
            let alone = !mods.ctrl && !mods.shift && !other;
            let gesture_key = crate::mission_control::is_gesture_key(state.nested, sym);
            let pressed = key_state == KeyState::Pressed;
            if !state.is_locked() {
                state.overview.key(gesture_key, pressed, alone);
            }
            if key_state == KeyState::Released {
                // Space let go while the area moved: it is drawn again (lot D-35e).
                if state.screenshot_tool_open() && sym == Keysym::space {
                    state.tool_key(sym, false);
                }
                let len = state.intercepted_keys.len();
                state.intercepted_keys.retain(|k| *k != key);
                return if state.intercepted_keys.len() < len {
                    FilterResult::Intercept(None)
                } else {
                    FilterResult::Forward
                };
            }
            // A forwarded press cancels an interception whose release got lost.
            state.intercepted_keys.retain(|k| *k != key);
            if state.is_locked() {
                let Some(shortcut) = Shortcut::while_locked(state, mods, key, sym) else {
                    return FilterResult::Forward;
                };
                state.intercepted_keys.push(key);
                return FilterResult::Intercept(Some(shortcut));
            }
            if state.screenshot_tool_open() {
                state.intercepted_keys.push(key);
                return FilterResult::Intercept(Some(Shortcut::ToolKey(sym)));
            }
            if state.overview_open() && sym == Keysym::Escape {
                state.intercepted_keys.push(key);
                return FilterResult::Intercept(Some(Shortcut::CloseOverview));
            }
            let media = MediaKey::of(sym).filter(|_| state.takes_media_keys());
            // Nested, Print Screen stays with the host session.
            let screenshot =
                capture::Kind::of(sym, mods).filter(|_| state.takes_screenshots() && !state.nested);
            let tool = (matches!(sym, Keysym::s | Keysym::S)
                && mods.shift
                && !mods.ctrl
                && gesture_modifier_held(state.nested, mods)
                && state.takes_screenshots())
            .then_some(Shortcut::ScreenshotTool);
            let shortcut = Shortcut::for_session(state.nested, mods, sym)
                .or_else(|| Shortcut::for_workspaces(state.nested, mods, key, sym))
                .or(media.map(|media| Shortcut::Media(media, key)))
                .or(screenshot.map(Shortcut::Screenshot))
                .or(tool)
                .or_else(|| Shortcut::for_applications(mods, sym));
            if let Some(shortcut) = shortcut {
                state.intercepted_keys.push(key);
                return FilterResult::Intercept(Some(shortcut));
            }
            match Shortcut::for_key(sym) {
                Some(shortcut) if gesture_modifier_alone(state.nested, mods) => {
                    state.intercepted_keys.push(key);
                    FilterResult::Intercept(Some(shortcut))
                }
                _ => FilterResult::Forward,
            }
        });
        if let Some(Some(shortcut)) = shortcut {
            self.run_shortcut(shortcut);
        }
        if self.overview.take_toggle() && !self.is_locked() {
            self.toggle_overview();
        }
    }

    fn run_shortcut(&mut self, shortcut: Shortcut) {
        match shortcut {
            Shortcut::SwitchVt(vt) => {
                tracing::info!(vt, "switching console");
                if let Some(switch) = self.switch_vt.as_mut() {
                    switch(vt);
                }
                return;
            }
            Shortcut::Quit if self.is_locked() => {
                tracing::warn!("Ctrl + Alt + Backspace while locked: refused");
                return;
            }
            Shortcut::Quit => {
                tracing::info!("Ctrl + Alt + Backspace: the compositor stops");
                self.loop_signal.stop();
                return;
            }
            Shortcut::Workspace(index) => {
                if let Some(id) = self.workspaces().id_at(index).map(str::to_owned) {
                    self.activate_workspace(&id);
                }
                return;
            }
            Shortcut::NeighborWorkspace(steps) => {
                self.activate_workspace_by(steps);
                return;
            }
            Shortcut::ToggleOverview => {
                self.toggle_overview();
                return;
            }
            Shortcut::CloseOverview => {
                self.close_overview();
                return;
            }
            Shortcut::Launcher => {
                self.start_launcher();
                return;
            }
            Shortcut::Applications => {
                self.start_applications();
                return;
            }
            Shortcut::ClipboardHistory => {
                self.start_clipboard_panel();
                return;
            }
            Shortcut::Media(media, code) => {
                self.press_media_key(code, media);
                return;
            }
            Shortcut::Screenshot(kind) => {
                self.screenshot(kind);
                return;
            }
            Shortcut::ScreenshotTool => {
                self.open_screenshot_tool();
                return;
            }
            Shortcut::ToolKey(sym) => {
                self.tool_key(sym, true);
                return;
            }
            Shortcut::Lock => {
                tracing::info!("Super + L: the session locks");
                self.lock_session();
                return;
            }
            Shortcut::RetryLock => {
                self.retry_lock_program();
                return;
            }
            Shortcut::SendToWorkspace(index) => {
                let id = self.workspaces().id_at(index).map(str::to_owned);
                if let (Some(window), Some(id)) = (self.focused_window(), id) {
                    self.move_to_workspace(&window, &id);
                }
                return;
            }
            _ => {}
        }
        let Some(window) = self.focused_window() else {
            return;
        };
        tracing::debug!(?shortcut, "shortcut");
        let maximized = is_maximized(&window);
        let zone = match shortcut {
            Shortcut::ToggleMaximized if !maximized => Some(Zone::Maximized),
            Shortcut::Snap(zone) => Some(zone),
            _ => None,
        };
        self.snap(&window, zone, None);
    }

    /// Handles an input event from the backend.
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event } => {
                self.keyboard_key(event.key_code(), event.state(), event.time_msec());
            }
            InputEvent::PointerMotionAbsolute { event } => {
                // The backend gives a point of the whole frame, where each screen has its slice.
                let size = self.frame_size();
                let p = event.position_transformed((size.w, size.h).into());
                let Some(pos) = self.frame_to_space(Point::<f64, Physical>::from((p.x, p.y)))
                else {
                    return;
                };
                self.pointer_motion(pos, event.time_msec());
            }
            InputEvent::PointerButton { event } => {
                self.pointer_button(event.button_code(), event.state(), event.time_msec());
            }
            InputEvent::PointerAxis { event } => {
                self.note_input();
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let v120 = event.amount_v120(axis);
                    let amount = event.amount(axis).or(v120.map(|v| v * 15.0 / 120.0));
                    match amount {
                        Some(amount) if amount != 0.0 => {
                            frame = frame.value(axis, amount);
                            if let Some(v120) = v120 {
                                frame = frame.v120(axis, v120 as i32);
                            }
                        }
                        // The fingers left the touchpad: libinput says so with a zero on a
                        // continuous source, which clients need as `axis_stop` to end the
                        // gesture (Qt and GTK otherwise never end it, nor start kinetic
                        // scrolling) (Q-22).
                        Some(_) if event.source() != AxisSource::Wheel => {
                            frame = frame.stop(axis);
                        }
                        _ => {}
                    }
                }
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            // The lid closed: the session locks, whether logind puts the machine to sleep for
            // it or not (lot D-25a3).
            InputEvent::SwitchToggle { event } => {
                if event.switch() == Some(Switch::Lid) && event.state() == SwitchState::On {
                    tracing::info!("lid closed: the session locks");
                    self.lock_session();
                }
            }
            event => {
                self.note_input();
                self.gesture_event(event);
            }
        }
    }

    /// The pointer is at `location`. Entry point of every backend, and of the tests.
    pub fn pointer_motion(&mut self, location: Point<f64, Logical>, time: u32) {
        self.queue_redraw();
        self.note_input();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        // Locked: only a lock surface may have the pointer (lot D-25a). During a gesture, no
        // client has it.
        let under = if self.is_locked() {
            self.lock_hit(location)
        } else if self.gesture_active() {
            self.update_gesture(location);
            self.hover_decoration(None);
            None
        } else if self.picking_area() {
            // The area of a screenshot: no client has the pointer (lot D-35c).
            self.tool_motion(location);
            None
        } else if self.overview_open() {
            // Mission Control: no client has the pointer (lot D-33).
            self.overview_motion(location);
            None
        } else {
            let hit = self.hit(location);
            self.hover_decoration(match &hit {
                Some(Hit::Decoration(window, control)) => Some((window.clone(), *control)),
                _ => None,
            });
            match hit {
                Some(Hit::Surface(_, surface, origin) | Hit::Layer(_, surface, origin)) => {
                    Some((surface, origin))
                }
                _ => None,
            }
        };
        let serial = SERIAL_COUNTER.next_serial();
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location,
                serial,
                time,
            },
        );
        pointer.frame(self);
    }

    /// A button (code from linux/input-event-codes.h) is pressed or released.
    pub fn pointer_button(&mut self, button: u32, state: ButtonState, time: u32) {
        self.queue_redraw();
        self.note_input();
        let (Some(pointer), Some(keyboard)) = (self.seat.get_pointer(), self.seat.get_keyboard())
        else {
            return;
        };
        let location = pointer.current_location();
        let serial = SERIAL_COUNTER.next_serial();
        let event = ButtonEvent {
            button,
            state,
            serial,
            time,
        };
        self.overview.cancel_tap();
        if self.is_locked() {
            if state == ButtonState::Pressed {
                self.lock_input(false);
            }
            // Locked: the button goes to the lock surface under the pointer, which takes the
            // keyboard on a press (lot D-25a).
            if state == ButtonState::Pressed {
                self.set_keyboard_focus(None);
            }
            pointer.button(self, &event);
            pointer.frame(self);
            return;
        }
        if state == ButtonState::Pressed {
            // Show the clicks (lot C-01): a ring at the press, whatever it reaches.
            self.click_at(location);
        }
        if self.picking_area() {
            // The area of a screenshot: the click goes to no client (lot D-35c).
            pointer.button(self, &event);
            pointer.frame(self);
            let pressed = state == ButtonState::Pressed;
            self.tool_button(button == BTN_LEFT, pressed, location);
            return;
        }
        if self.overview_open() {
            // Mission Control: the click goes to no client (lot D-33).
            pointer.button(self, &event);
            pointer.frame(self);
            self.overview_button(button, state == ButtonState::Pressed, location);
            return;
        }
        if let Some(gesture) = self.gesture.take() {
            // The pointer has no focus during the gesture: no client sees the other buttons, but
            // the seat tracks them.
            let ends = state == ButtonState::Released && button == gesture.button;
            if ends {
                self.finish_gesture(gesture);
            } else {
                self.gesture = Some(gesture);
            }
            pointer.button(self, &event);
            pointer.frame(self);
            if ends {
                // The focus goes back to the surface under the pointer.
                self.pointer_motion(location, time);
            }
            return;
        }
        if state == ButtonState::Released && button == BTN_LEFT {
            self.release_decoration(location);
        }
        if state == ButtonState::Pressed && !pointer.is_grabbed() {
            let held = gesture_modifier_held(self.nested, &keyboard.modifier_state());
            // With the gesture key, a window under the top bar or the dock can still be moved:
            // the way back for a window lost there. The overlay (lock screen) keeps its clicks.
            let hit = match held {
                true => self.gesture_hit(location),
                false => self.hit(location),
            };
            // An open menu closes on a click on another client; the click still goes through.
            if let Some(grab) = &self.popup_grab {
                let same_client = match &hit {
                    Some(Hit::Surface(_, surface, _) | Hit::Layer(_, surface, _)) => {
                        surface.id().same_client_as(&grab.root.id())
                    }
                    _ => false,
                };
                if !same_client {
                    self.dismiss_popups();
                }
            }
            let window = match &hit {
                Some(
                    Hit::Surface(window, ..) | Hit::Border(window, _) | Hit::Decoration(window, _),
                ) => Some(window.clone()),
                Some(Hit::Layer(layer, ..)) => {
                    self.click_layer(layer);
                    pointer.button(self, &event);
                    pointer.frame(self);
                    return;
                }
                None => None,
            };
            // A click in an X11 menu goes to the menu: the keyboard stays with its window.
            if window.as_ref().is_some_and(is_override_redirect) {
                pointer.button(self, &event);
                pointer.frame(self);
                return;
            }
            self.focus(window.clone());
            let rect = window.as_ref().and_then(|w| self.space.element_geometry(w));
            // The title bar and its buttons: the client does not see the press.
            if let (false, Some(Hit::Decoration(window, control))) = (held, &hit) {
                self.press_decoration(window.clone(), *control, button, (location, time));
                if self.gesture_active() {
                    self.pointer_motion(location, time);
                    return;
                }
                pointer.button(self, &event);
                pointer.frame(self);
                return;
            }
            let gesture = match (button, held, hit) {
                // Gesture key + button: the client does not see the press.
                (BTN_LEFT, true, _) => Some((None, GestureSource::Modifier)),
                (BTN_RIGHT, true, _) => rect.map(|r| {
                    let edges = Edges::nearest(r, location);
                    (Some(edges), GestureSource::Modifier)
                }),
                // Press in the band around a window: pull that edge or corner.
                (BTN_LEFT, false, Some(Hit::Border(_, edges))) => {
                    Some((Some(edges), GestureSource::Border))
                }
                _ => None,
            };
            if let (Some(window), Some((edges, source))) = (window, gesture) {
                self.start_gesture(window, location, button, edges, source);
                if self.gesture_active() {
                    self.pointer_motion(location, time);
                    return;
                }
            }
        }
        pointer.button(self, &event);
        pointer.frame(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, alt: bool) -> ModifiersState {
        ModifiersState {
            ctrl,
            alt,
            ..ModifiersState::default()
        }
    }

    #[test]
    fn workspace_keys_are_the_top_row_whatever_the_layout() {
        let key = |m: &ModifiersState, code, sym| {
            Shortcut::for_workspaces(false, m, Keycode::new(code), sym)
        };
        let logo = ModifiersState {
            logo: true,
            ..ModifiersState::default()
        };
        let shift = ModifiersState {
            shift: true,
            ..logo
        };
        let ctrl = ModifiersState { ctrl: true, ..logo };
        // Top row of AZERTY: `&` on key 1, `ç` on key 9.
        let one = key(&logo, 10, Keysym::ampersand);
        assert!(matches!(one, Some(Shortcut::Workspace(0))));
        let nine = key(&logo, 18, Keysym::ccedilla);
        assert!(matches!(nine, Some(Shortcut::Workspace(8))));
        let send = key(&shift, 11, Keysym::_2);
        assert!(matches!(send, Some(Shortcut::SendToWorkspace(1))));
        let right = key(&ctrl, 114, Keysym::Right);
        assert!(matches!(right, Some(Shortcut::NeighborWorkspace(1))));
        let left = key(&ctrl, 113, Keysym::Left);
        assert!(matches!(left, Some(Shortcut::NeighborWorkspace(-1))));
        // Key 0, or without the gesture key, or with Ctrl and a digit: for the client.
        assert!(key(&logo, 19, Keysym::agrave).is_none());
        assert!(key(&mods(false, false), 10, Keysym::ampersand).is_none());
        assert!(key(&ctrl, 10, Keysym::ampersand).is_none());
        // In nested mode, Alt is the gesture key and Super goes to the client.
        let alt = mods(false, true);
        let nested = Shortcut::for_workspaces(true, &alt, Keycode::new(10), Keysym::ampersand);
        assert!(matches!(nested, Some(Shortcut::Workspace(0))));
        assert!(key(&ModifiersState { alt: true, ..logo }, 10, Keysym::ampersand).is_none());
    }

    #[test]
    fn session_keys_switch_console_and_quit() {
        let ctrl_alt = mods(true, true);
        let key = |sym| Shortcut::for_session(false, &ctrl_alt, sym);
        assert!(matches!(
            key(Keysym::XF86_Switch_VT_1),
            Some(Shortcut::SwitchVt(1))
        ));
        assert!(matches!(
            key(Keysym::XF86_Switch_VT_12),
            Some(Shortcut::SwitchVt(12))
        ));
        assert!(matches!(key(Keysym::BackSpace), Some(Shortcut::Quit)));
        assert!(matches!(
            key(Keysym::Terminate_Server),
            Some(Shortcut::Quit)
        ));
        // Backspace alone, or with Ctrl only, goes to the client; so do other keys.
        let ctrl = mods(true, false);
        assert!(Shortcut::for_session(false, &ctrl, Keysym::BackSpace).is_none());
        assert!(key(Keysym::F1).is_none());
        // In nested mode, the host session keeps its consoles, and closing the window stops.
        assert!(Shortcut::for_session(true, &ctrl_alt, Keysym::BackSpace).is_none());
        assert!(Shortcut::for_session(true, &ctrl_alt, Keysym::XF86_Switch_VT_2).is_none());
    }

    #[test]
    fn f4_alone_opens_the_applications_view() {
        let none = mods(false, false);
        assert!(matches!(
            Shortcut::for_applications(&none, Keysym::F4),
            Some(Shortcut::Applications)
        ));
        // With a modifier (Alt + F4 closes a window elsewhere), it goes to the client.
        assert!(Shortcut::for_applications(&mods(false, true), Keysym::F4).is_none());
        assert!(Shortcut::for_applications(&mods(true, false), Keysym::F4).is_none());
        assert!(Shortcut::for_applications(&none, Keysym::F3).is_none());
    }
}
