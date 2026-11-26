//! Keyboard and mouse: click to focus, move and resize gestures, snapping, shortcuts and cursor
//! shape.
//!
//! A gesture comes from the gesture key (see [`gesture_modifier_held`]) with a button, from a
//! press in the band around a window, or from a client request (`xdg_toplevel.move` or
//! `.resize`, its title bar or its edges).

use sherlock_ui::window as ui;
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, ButtonState, Event, InputBackend, InputEvent, KeyState,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    desktop::{Window, WindowSurfaceType},
    input::{
        keyboard::{FilterResult, Keycode, Keysym, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, MotionEvent},
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
    State,
    decorations::{self, Control},
    handlers::{Arrangement, is_fullscreen, is_maximized},
    is_override_redirect,
    layout::{self, Edges, Resize, Zone},
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
    /// Ctrl + Alt + F1 to F12, in a real session: switches to that console.
    SwitchVt(i32),
    /// Ctrl + Alt + Backspace, in a real session: stops the compositor, the way out when
    /// nothing else answers.
    Quit,
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

    fn for_key(sym: Keysym) -> Option<Self> {
        match sym {
            Keysym::Up => Some(Self::ToggleMaximized),
            Keysym::Left => Some(Self::Snap(Zone::Left)),
            Keysym::Right => Some(Self::Snap(Zone::Right)),
            Keysym::Down => Some(Self::Restore),
            _ => None,
        }
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
        gesture.last_size =
            layout::resized(gesture.initial, edges, pointer - gesture.start, min, max);
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
            gesture.initial = Rectangle::new(loc, size);
            self.move_window(&gesture.window, loc, true);
        }
        gesture.floating = Rectangle::new(gesture.initial.loc, restore.size);
        if let Some(toplevel) = gesture.window.toplevel() {
            toplevel.send_pending_configure();
        }
    }

    fn finish_gesture(&mut self, gesture: Gesture) {
        let Gesture {
            window,
            source,
            initial,
            edges,
            last_size,
            floating,
            zone,
            ..
        } = gesture;
        tracing::debug!(?source, ?edges, size = ?last_size, ?zone, "gesture end");
        let Some(edges) = edges else {
            if let Some((zone, output)) = zone {
                // Dropped against an edge: snapped. It will return to its place from before the
                // drag.
                window_data(&window).restore.set(Some(floating));
                self.snap(&window, Some(zone), Some(output));
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
    pub fn forget_intercepted_keys(&mut self) {
        self.intercepted_keys.clear();
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
        if let Some(gesture) = &self.gesture {
            return Some(gesture.edges.map_or(CursorIcon::Move, resize_cursor));
        }
        let location = self.seat.get_pointer()?.current_location();
        match (self.hit(location), &self.cursor_status) {
            (Some(Hit::Border(_, edges)), _) => Some(resize_cursor(edges)),
            (Some(Hit::Surface(..) | Hit::Layer(..)), CursorImageStatus::Named(icon)) => {
                Some(*icon)
            }
            (Some(Hit::Surface(..) | Hit::Layer(..)), CursorImageStatus::Hidden) => None,
            _ => Some(CursorIcon::Default),
        }
    }

    /// What is under the pointer at `pos`. The band of a window comes before the windows it
    /// covers, not before those above it. Overlay and top anchored
    /// surfaces come before the windows, bottom and background ones after.
    pub(crate) fn hit(&self, pos: Point<f64, Logical>) -> Option<Hit> {
        self.layer_hit(pos, &[Layer::Overlay, Layer::Top])
            .or_else(|| self.window_hit(pos))
            .or_else(|| self.layer_hit(pos, &[Layer::Bottom, Layer::Background]))
    }

    fn window_hit(&self, pos: Point<f64, Logical>) -> Option<Hit> {
        // Hidden windows (other workspaces, minimized) are skipped, which the space does not do.
        for window in self.space.elements().rev().filter(|w| self.is_visible(w)) {
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

    /// A key (xkb code) is pressed or released. Entry point of every backend, and of the
    /// tests.
    pub fn keyboard_key(&mut self, key: Keycode, key_state: KeyState, time: u32) {
        self.queue_redraw();
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        let shortcut = keyboard.input(self, key, key_state, serial, time, |state, mods, sym| {
            if key_state == KeyState::Released {
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
            let sym = sym.modified_sym();
            let shortcut = Shortcut::for_session(state.nested, mods, sym)
                .or_else(|| Shortcut::for_workspaces(state.nested, mods, key, sym));
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
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let v120 = event.amount_v120(axis);
                    let amount = event.amount(axis).or(v120.map(|v| v * 15.0 / 120.0));
                    if let Some(amount) = amount.filter(|a| *a != 0.0) {
                        frame = frame.value(axis, amount);
                        if let Some(v120) = v120 {
                            frame = frame.v120(axis, v120 as i32);
                        }
                    }
                }
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    /// The pointer is at `location`. Entry point of every backend, and of the tests.
    pub fn pointer_motion(&mut self, location: Point<f64, Logical>, time: u32) {
        self.queue_redraw();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        // During a gesture, no client has the pointer.
        let under = if self.gesture_active() {
            self.update_gesture(location);
            self.hover_decoration(None);
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
            let hit = self.hit(location);
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
            let held = gesture_modifier_held(self.nested, &keyboard.modifier_state());
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
}
