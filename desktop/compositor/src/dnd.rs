//! Drag and drop started by a Wayland client (lot D-29c): the icon under the pointer, Escape to
//! cancel, the icon gliding back when the drop is refused, and the bridge to the X11 windows
//! (`xdnd.rs`).
//!
//! Between Wayland clients, smithay's grab (`DnDGrab`) does the protocol. Over an X11 window,
//! the grab is given no surface (it would cancel the drag at the release, since XWayland has no
//! data device): the compositor speaks XDND to the window instead. When that window accepts,
//! the release is held back from the grab until the window has taken the data and said so
//! (`XdndFinished`), then the Wayland source is told `dnd_drop_performed` and `dnd_finished`.

use std::time::Duration;

use sherlock_ui::motion;
use smithay::{
    backend::input::ButtonState,
    input::{
        Seat,
        pointer::{ButtonEvent, CursorIcon, MotionEvent},
    },
    output::Output,
    reexports::{
        calloop::{
            Interest, Mode, PostAction,
            channel::{self, Sender},
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        rustix,
        wayland_server::{
            Resource,
            protocol::{
                wl_data_device_manager::DndAction,
                wl_data_source::{self, WlDataSource},
                wl_surface::WlSurface,
            },
        },
    },
    utils::{IsAlive, Logical, Point, SERIAL_COUNTER},
    wayland::{
        compositor::{self, SurfaceAttributes},
        selection::data_device::{ClientDndGrabHandler, with_source_metadata},
    },
    xwayland::X11Surface,
};

use crate::{
    State,
    input::Hit,
    output::View,
    render::Item,
    xdnd::{self, Action, Note, Request, Xdnd},
};

/// Longest wait for the X11 window dropped on to take the data and say so.
const DROP_PATIENCE: Duration = Duration::from_secs(5);
/// Longest wait for the Wayland source to go away once told the drop is done, before the
/// grab ends (and tells the source, still there, `cancelled` too).
const FINISH_PATIENCE: Duration = Duration::from_millis(500);
/// Pace of that wait.
const FINISH_STEP: Duration = Duration::from_millis(20);

/// The drags, the icon going back, and the X11 side.
#[derive(Default)]
pub(crate) struct Dnd {
    drag: Option<Drag>,
    back: Option<Back>,
    pub(crate) x11: Option<X11Side>,
    /// A drag of an X11 application, carried to the Wayland clients (`dnd_x11.rs`).
    pub(crate) from_x11: Option<crate::dnd_x11::FromX11>,
}

/// A drag started by a Wayland client.
struct Drag {
    source: Option<WlDataSource>,
    icon: Option<WlSurface>,
    /// The icon's top left corner from the pointer (`wl_surface.offset`).
    offset: Point<i32, Logical>,
    start: Point<f64, Logical>,
    /// Where the pointer is: smithay calls `dropped` with the pointer locked, which must not
    /// be asked then.
    at: Point<f64, Logical>,
    /// The X11 side was told about this drag (`Xdnd::start`).
    x11_started: bool,
    /// Over an X11 window that speaks XDND, and what it said last.
    x11_over: bool,
    x11_status: Option<(bool, Option<Action>)>,
    /// Dropped on an X11 window: the button whose release is held back, and when the wait
    /// for that window ends.
    held: Option<(u32, Duration)>,
    /// The X11 window is done: whether it took the drop, and since when.
    finished: Option<(bool, Duration)>,
}

/// An icon going back to where its drag started.
struct Back {
    icon: WlSurface,
    from: Point<f64, Logical>,
    to: Point<f64, Logical>,
    fade: motion::Fade,
}

/// The XDND source and its sources in the loop.
pub(crate) struct X11Side {
    pub(crate) xdnd: Xdnd,
    reads: Sender<(Request, Option<Vec<u8>>)>,
}

impl ClientDndGrabHandler for State {
    fn started(&mut self, source: Option<WlDataSource>, icon: Option<WlSurface>, seat: Seat<Self>) {
        let start = seat
            .get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default();
        tracing::debug!(with_data = source.is_some(), "drag started");
        self.dnd.back = None;
        self.dnd.drag = Some(Drag {
            source,
            icon,
            offset: Point::default(),
            start,
            at: start,
            x11_started: false,
            x11_over: false,
            x11_status: None,
            held: None,
            finished: None,
        });
        self.queue_redraw();
    }

    fn dropped(&mut self, _target: Option<WlSurface>, validated: bool, _seat: Seat<Self>) {
        let Some(drag) = self.dnd.drag.take() else {
            return;
        };
        let taken = validated || drag.finished.is_some_and(|f| f.0);
        tracing::debug!(taken, "drag ended");
        if drag.x11_started
            && let Some(x11) = self.dnd.x11.as_mut()
            && let Err(err) = x11.xdnd.end()
        {
            tracing::debug!("XDND end: {err}");
        }
        // Refused or cancelled: the icon goes back to where it was taken, while its client
        // keeps it.
        if !taken
            && let Some(icon) = drag.icon
            && icon.alive()
        {
            let from = drag.at + drag.offset.to_f64();
            let to = drag.start + drag.offset.to_f64();
            let fade = motion::Fade::new(0.0, 1.0, self.now(), motion::DRAG_RETURN);
            self.dnd.back = Some(Back {
                icon,
                from,
                to,
                fade,
            });
        }
        self.queue_redraw();
    }
}

/// The action a drop to X11 asks for: copy unless Shift asks to move, among what the source
/// allows.
fn action_for(allowed: DndAction, shift: bool) -> Action {
    let wanted = match shift {
        true => [DndAction::Move, DndAction::Copy, DndAction::Ask],
        false => [DndAction::Copy, DndAction::Move, DndAction::Ask],
    };
    match wanted.into_iter().find(|a| allowed.contains(*a)) {
        Some(DndAction::Move) => Action::Move,
        Some(DndAction::Ask) => Action::Ask,
        _ => Action::Copy,
    }
}

fn wayland_action(action: Option<Action>) -> DndAction {
    match action {
        Some(Action::Copy) => DndAction::Copy,
        Some(Action::Move) => DndAction::Move,
        Some(Action::Ask) => DndAction::Ask,
        Some(Action::Link) | None => DndAction::None,
    }
}

fn since_v3(source: &WlDataSource) -> bool {
    source.version() >= wl_data_source::EVT_DND_FINISHED_SINCE
}

impl State {
    /// XWayland is ready: the compositor connects as the XDND source of the drags towards X11.
    pub(crate) fn connect_xdnd(&mut self, display: u32) {
        let xdnd = match Xdnd::connect(display) {
            Ok(xdnd) => xdnd,
            Err(err) => {
                tracing::warn!("no drag and drop towards X11: {err}");
                return;
            }
        };
        let fd = match xdnd.fd() {
            Ok(fd) => fd,
            Err(err) => {
                tracing::warn!("no drag and drop towards X11: {err}");
                return;
            }
        };
        let (reads, done) = channel::channel();
        let listened = self.loop_handle.insert_source(
            Generic::new(fd, Interest::READ, Mode::Level),
            |_, _, state| {
                state.xdnd_events();
                if state.dnd.x11.as_ref().is_none_or(|x11| !x11.xdnd.alive()) {
                    state.dnd.x11 = None;
                    return Ok(PostAction::Remove);
                }
                Ok(PostAction::Continue)
            },
        );
        let answered = self.loop_handle.insert_source(done, |event, _, state| {
            if let channel::Event::Msg((request, data)) = event
                && let Some(x11) = state.dnd.x11.as_mut()
                && let Err(err) = x11.xdnd.answer(request, data)
            {
                tracing::debug!("XDND answer: {err}");
            }
        });
        match (listened, answered) {
            (Ok(_), Ok(_)) => {}
            (listened, answered) => {
                tracing::warn!("no drag and drop towards X11: a source of the loop refused");
                for token in [listened.ok(), answered.ok()].into_iter().flatten() {
                    self.loop_handle.remove(token);
                }
                return;
            }
        }
        self.dnd.x11 = Some(X11Side { xdnd, reads });
    }

    /// XWayland stops: its sources in the loop remove themselves.
    pub(crate) fn disconnect_xdnd(&mut self) {
        self.dnd.x11 = None;
    }

    /// A drag is under way (started by a Wayland client).
    pub fn dragging(&self) -> bool {
        self.dnd.drag.is_some()
    }

    /// The pointer moved to `location` during a drag, over `hit`. `true`: smithay's grab must
    /// not see the surface under it (an X11 window, or the drop being held).
    pub(crate) fn drag_motion(&mut self, hit: Option<&Hit>, location: Point<f64, Logical>) -> bool {
        let shift = self
            .seat
            .get_keyboard()
            .is_some_and(|k| k.modifier_state().shift);
        let Some(drag) = self.dnd.drag.as_mut() else {
            return false;
        };
        drag.at = location;
        if drag.held.is_some() {
            return true;
        }
        let x11 = match hit {
            Some(Hit::Surface(window, _, origin)) => window
                .x11_surface()
                .filter(|s| !s.is_override_redirect())
                .map(|s| (s.clone(), *origin)),
            _ => None,
        };
        tracing::trace!(?location, over_x11 = x11.is_some(), "drag motion");
        let (Some(source), Some(side)) = (drag.source.clone(), self.dnd.x11.as_mut()) else {
            return x11.is_some();
        };
        let over_x11 = x11.is_some();
        let result = (|| -> Result<bool, xdnd::Error> {
            let Some((surface, origin)) = x11 else {
                if drag.x11_over {
                    side.xdnd.leave()?;
                }
                return Ok(false);
            };
            let (types, allowed) =
                with_source_metadata(&source, |m| (m.mime_types.clone(), m.dnd_action))
                    .unwrap_or_else(|_| (Vec::new(), DndAction::empty()));
            if !drag.x11_started {
                side.xdnd.start(&types)?;
                drag.x11_started = true;
            }
            side.xdnd.hover(
                Some(surface.window_id()),
                root_point(&surface, origin, location),
                action_for(allowed, shift),
            )
        })();
        let speaks = result.unwrap_or_else(|err| {
            tracing::debug!("XDND: {err}");
            false
        });
        if !speaks {
            drag.x11_status = None;
        }
        drag.x11_over = speaks;
        self.xdnd_events();
        over_x11
    }

    /// A button during a drag. `true`: it is held back (dropped on an X11 window, which
    /// takes the data now), or ignored while that lasts.
    pub(crate) fn drag_button(&mut self, button: u32, state: ButtonState, last: bool) -> bool {
        let now = self.now();
        let Some(drag) = self.dnd.drag.as_mut() else {
            return false;
        };
        if drag.held.is_some() {
            return true;
        }
        if state != ButtonState::Released || !last || !drag.x11_over {
            return false;
        }
        let Some(side) = self.dnd.x11.as_mut() else {
            return false;
        };
        let dropped = side.xdnd.drop_here().unwrap_or_else(|err| {
            tracing::debug!("XDND drop: {err}");
            false
        });
        if !dropped {
            drag.x11_over = false;
            return false;
        }
        if let Some(source) = drag.source.as_ref().filter(|s| since_v3(s)) {
            source.dnd_drop_performed();
        }
        drag.held = Some((button, now + DROP_PATIENCE));
        let timer = Timer::from_duration(DROP_PATIENCE);
        let waited = self.loop_handle.insert_source(timer, |_, _, state| {
            state.drop_timed_out();
            TimeoutAction::Drop
        });
        if let Err(err) = waited {
            tracing::warn!("drop wait: {err}");
        }
        true
    }

    /// Escape: the drag is cancelled, unless it was already dropped. `false`: no drag.
    pub(crate) fn cancel_drag(&mut self, time: u32) -> bool {
        let Some(drag) = self.dnd.drag.as_mut() else {
            return false;
        };
        if drag.held.is_some() {
            return true;
        }
        if drag.x11_over
            && let Some(side) = self.dnd.x11.as_mut()
            && let Err(err) = side.xdnd.leave()
        {
            tracing::debug!("XDND leave: {err}");
        }
        drag.x11_over = false;
        let Some(pointer) = self.seat.get_pointer() else {
            return true;
        };
        let location = pointer.current_location();
        // The grab leaves the surface under the pointer first: ended there, it would drop.
        let serial = SERIAL_COUNTER.next_serial();
        let event = MotionEvent {
            location,
            serial,
            time,
        };
        pointer.motion(self, None, &event);
        pointer.unset_grab(self, serial, time);
        pointer.frame(self);
        tracing::debug!("drag cancelled with Escape");
        true
    }

    /// What XWayland said about the drag.
    pub(crate) fn xdnd_events(&mut self) {
        let Some(side) = self.dnd.x11.as_mut() else {
            return;
        };
        for note in side.xdnd.events() {
            self.xdnd_note(note);
        }
    }

    fn xdnd_note(&mut self, note: Note) {
        let now = self.now();
        let (Some(drag), Some(side)) = (self.dnd.drag.as_mut(), self.dnd.x11.as_mut()) else {
            return;
        };
        let source = drag.source.clone().filter(IsAlive::alive);
        match note {
            Note::Status { accepted, action } => {
                drag.x11_status = Some((accepted, action));
                if let Some(source) = source.filter(since_v3) {
                    source.action(wayland_action(action.filter(|_| accepted)));
                }
                self.queue_redraw();
            }
            Note::Finished { accepted, action } => {
                if drag.held.is_none() || drag.finished.is_some() {
                    return;
                }
                drag.finished = Some((accepted, now));
                if let Some(source) = source {
                    match accepted && since_v3(&source) {
                        true => {
                            source.action(wayland_action(action));
                            source.dnd_finished();
                        }
                        false => source.cancelled(),
                    }
                }
                tracing::debug!(accepted, "X11 window done with the drop");
                self.release_when_source_gone();
            }
            Note::Read(request) => {
                let Some(source) = source else {
                    if let Err(err) = side.xdnd.answer(request, None) {
                        tracing::debug!("XDND answer: {err}");
                    }
                    return;
                };
                match rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC) {
                    Ok((read, write)) => {
                        source.send(request.mime_type.clone(), std::os::fd::AsFd::as_fd(&write));
                        drop(write);
                        xdnd::read_source(request, read, side.reads.clone());
                    }
                    Err(err) => {
                        tracing::warn!("drag pipe: {err}");
                        if let Err(err) = side.xdnd.answer(request, None) {
                            tracing::debug!("XDND answer: {err}");
                        }
                    }
                }
            }
        }
    }

    /// The X11 window never finished: the drop is cancelled.
    fn drop_timed_out(&mut self) {
        let now = self.now();
        let Some(drag) = self.dnd.drag.as_mut() else {
            return;
        };
        let Some((_, until)) = drag.held else {
            return;
        };
        if drag.finished.is_some() || now < until {
            return;
        }
        tracing::debug!("X11 window silent after the drop: cancelled");
        drag.finished = Some((false, now));
        if let Some(source) = drag.source.as_ref().filter(|s| s.alive()) {
            source.cancelled();
        }
        self.release_when_source_gone();
    }

    /// Ends the grab with the release held back, once the Wayland source is gone (so that it
    /// is not also told `cancelled`), or after `FINISH_PATIENCE`.
    fn release_when_source_gone(&mut self) {
        let now = self.now();
        let Some(drag) = self.dnd.drag.as_ref() else {
            return;
        };
        let (Some((button, _)), Some((_, since))) = (drag.held, drag.finished) else {
            return;
        };
        let gone = drag.source.as_ref().is_none_or(|s| !s.alive());
        if !gone && now < since + FINISH_PATIENCE {
            let timer = Timer::from_duration(FINISH_STEP);
            let waited = self.loop_handle.insert_source(timer, |_, _, state| {
                state.release_when_source_gone();
                TimeoutAction::Drop
            });
            if waited.is_ok() {
                return;
            }
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        if let Some(drag) = self.dnd.drag.as_mut() {
            drag.held = None;
            drag.x11_over = false;
        }
        let time = u32::try_from(now.as_millis()).unwrap_or(u32::MAX);
        let event = ButtonEvent {
            button,
            state: ButtonState::Released,
            serial: SERIAL_COUNTER.next_serial(),
            time,
        };
        pointer.button(self, &event);
        pointer.frame(self);
        self.dnd.drag = None;
        let location = pointer.current_location();
        self.pointer_motion(location, time);
    }

    /// A commit of the drag icon: `wl_surface.offset` moves it from the pointer.
    pub(crate) fn drag_icon_commit(&mut self, surface: &WlSurface) {
        let Some(drag) = self
            .dnd
            .drag
            .as_mut()
            .filter(|d| d.icon.as_ref() == Some(surface))
        else {
            return;
        };
        let delta = compositor::with_states(surface, |states| {
            let mut attributes = states.cached_state.get::<SurfaceAttributes>();
            attributes.current().buffer_delta.take()
        });
        if let Some(delta) = delta {
            drag.offset += delta;
        }
    }

    /// The cursor during a drag, `None` without one: the arrow, with a plus sign when an X11
    /// window takes a copy, and the refusal when it takes nothing.
    pub(crate) fn drag_cursor(&self) -> Option<CursorIcon> {
        let drag = self.dnd.drag.as_ref()?;
        Some(match (drag.x11_over, drag.x11_status) {
            (true, Some((true, Some(Action::Copy)))) => CursorIcon::Copy,
            (true, Some((false, _))) => CursorIcon::NoDrop,
            _ => CursorIcon::Default,
        })
    }

    /// Frame callbacks of the drag icon, drawn on `output`.
    pub fn send_drag_frames(&self, output: &Output, time: Duration) {
        let icon = self.dnd.drag.as_ref().and_then(|d| d.icon.as_ref());
        let back = self.dnd.back.as_ref().map(|b| &b.icon);
        for icon in icon.into_iter().chain(back) {
            smithay::desktop::utils::send_frames_surface_tree(
                icon,
                output,
                time,
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        }
    }

    /// The icon is gone back, or its client destroyed it.
    pub(crate) fn end_drag_return(&mut self) {
        let now = self.now();
        if self
            .dnd
            .back
            .as_ref()
            .is_some_and(|b| b.fade.is_done(now) || !b.icon.alive())
        {
            self.dnd.back = None;
        }
    }
}

/// Where the pointer at `location` is in the X11 root window, over the X11 window `surface`
/// whose Wayland surface is at `origin`: the same logical units, at every scale.
fn root_point(
    surface: &X11Surface,
    origin: Point<f64, Logical>,
    location: Point<f64, Logical>,
) -> (i16, i16) {
    let at = surface.geometry().loc.to_f64() + (location - origin);
    let clamp = |v: f64| v.round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
    (clamp(at.x), clamp(at.y))
}

/// The drag icon at the pointer, or going back, in front of everything but the cursor.
/// `true` while it is going back.
pub(crate) fn push_icon(items: &mut Vec<Item>, state: &State, view: &View, now: Duration) -> bool {
    let (icon, loc, alpha, moving) = if let Some(drag) = state.dnd.drag.as_ref() {
        let Some((icon, pointer)) = drag.icon.as_ref().zip(state.seat.get_pointer()) else {
            return false;
        };
        let loc = pointer.current_location() + drag.offset.to_f64();
        (icon, loc, 1.0, false)
    } else if let Some(back) = state.dnd.back.as_ref() {
        let t = f64::from(back.fade.value(now));
        let loc = back.from + (back.to - back.from).upscale(t);
        let alpha = 1.0 - (1.0 - motion::DRAG_RETURN_ALPHA) * t as f32;
        (&back.icon, loc, alpha, !back.fade.is_done(now))
    } else {
        return false;
    };
    if !icon.alive() {
        return false;
    }
    items.push(Item::Surface {
        surface: icon.clone(),
        origin: view.point(loc).to_i32_round(),
        scale: view.scale,
        alpha,
    });
    moving
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_to_x11_copies_unless_shift_asks_to_move() {
        let both = DndAction::Copy | DndAction::Move;
        assert_eq!(action_for(both, false), Action::Copy);
        assert_eq!(action_for(both, true), Action::Move);
        assert_eq!(action_for(DndAction::Move, false), Action::Move);
        assert_eq!(action_for(DndAction::Copy, true), Action::Copy);
        assert_eq!(action_for(DndAction::Ask, false), Action::Ask);
    }
}
