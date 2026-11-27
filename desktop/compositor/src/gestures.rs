//! Touchpad gestures (lot D-31), as libinput recognizes them: swipes, pinches and holds.
//!
//! Gestures of three fingers or more belong to the compositor. Three fingers left or right move
//! the row of workspaces with the fingers, then let it go with their velocity on the spring of
//! the workspace slide (`anim::WorkspaceSlide`); up opens Mission Control, which follows the
//! fingers, and down closes it; down on the desktop is the setting `swipe-down`, nothing by
//! default. Four fingers and more, and pinches of three fingers or more, are reserved: taken,
//! without effect. Gestures of fewer fingers go to the client under the pointer, through
//! `zwp_pointer_gestures_v1`: pinch to zoom, holds.

use std::collections::VecDeque;

use desktop_config::touchpad::SwipeDown;
use smithay::{
    backend::input::{
        Event, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent as _,
        GestureSwipeUpdateEvent as _, InputBackend, InputEvent,
    },
    delegate_pointer_gestures,
    input::pointer::{
        GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
        GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent, PointerHandle,
    },
    reexports::wayland_server::DisplayHandle,
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::pointer_gestures::PointerGesturesState,
};

use crate::State;

/// Fingers from which a gesture belongs to the compositor.
pub const COMPOSITOR_FINGERS: u32 = 3;
/// Travel of the fingers before a swipe picks its direction, in libinput's units (a pixel of a
/// 1000 dpi pointer, accelerated).
pub const LOCK_DISTANCE: f64 = 16.0;
/// Travel of the fingers across a whole workspace: GNOME's choice for its workspace swipe.
pub const WORKSPACE_DISTANCE: f64 = 400.0;
/// Travel up or down that triggers the vertical swipe.
pub const VERTICAL_DISTANCE: f64 = 100.0;
/// Travel up that opens Mission Control fully while the fingers hold it (lot D-33).
pub const OVERVIEW_DISTANCE: f64 = 300.0;
/// The velocity is measured over the last movements of this window, in milliseconds: fingers
/// that stopped before lifting throw nothing.
pub const VELOCITY_WINDOW: u32 = 100;
/// How far the velocity carries the view, in seconds, when choosing where it lands.
pub const PROJECTION: f64 = 0.2;
/// Highest velocity handed to the spring, in workspaces per second.
pub const MAX_VELOCITY: f64 = 6.0;
/// Resistance past the bounds, as in iOS's rubber band: the view moves less and less.
const RUBBER: f64 = 0.55;

/// Direction a swipe took once its fingers travelled `LOCK_DISTANCE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Up,
    Down,
}

/// A three finger swipe, followed by the compositor. Pure computation: the view is a position
/// along the row of workspaces, in workspaces.
#[derive(Debug, Clone)]
pub struct Swipe {
    travel: Point<f64, Logical>,
    direction: Option<Direction>,
    /// Horizontal movements since the direction was picked: time in ms and delta.
    samples: VecDeque<(u32, f64)>,
    /// View at the start.
    origin: f64,
    /// What the view reaches without resistance: the neighbours of the active workspace.
    bounds: (f64, f64),
}

impl Swipe {
    /// A swipe from the view `origin`, the workspace at `active` being active among `count`.
    pub fn new(origin: f64, active: usize, count: usize) -> Self {
        let last = count.saturating_sub(1);
        let bounds = (active.saturating_sub(1), (active + 1).min(last));
        Self {
            travel: Point::default(),
            direction: None,
            samples: VecDeque::new(),
            origin,
            bounds: (bounds.0 as f64, bounds.1 as f64),
        }
    }

    pub fn direction(&self) -> Option<Direction> {
        self.direction
    }

    /// The fingers moved by `delta` at `time`. Returns the view when the swipe is horizontal.
    pub fn update(&mut self, time: u32, delta: Point<f64, Logical>) -> Option<f64> {
        self.travel += delta;
        let Point { x, y, .. } = self.travel;
        if self.direction.is_none() && x.hypot(y) >= LOCK_DISTANCE {
            self.direction = Some(if x.abs() >= y.abs() {
                Direction::Horizontal
            } else if y < 0.0 {
                Direction::Up
            } else {
                Direction::Down
            });
            // The movement so far counts for the velocity: the fingers were already moving.
            self.samples.push_back((time, x));
            return (self.direction == Some(Direction::Horizontal)).then(|| self.view());
        }
        if self.direction != Some(Direction::Horizontal) {
            return None;
        }
        self.samples.push_back((time, delta.x));
        while self
            .samples
            .front()
            .is_some_and(|s| s.0.saturating_add(VELOCITY_WINDOW) < time)
        {
            self.samples.pop_front();
        }
        Some(self.view())
    }

    /// Where the fingers put the view: the row follows them (natural direction), with a rubber
    /// band past the neighbours of the active workspace and past the ends of the row.
    pub fn view(&self) -> f64 {
        let view = self.origin - self.travel.x / WORKSPACE_DISTANCE;
        let band = |past: f64| 1.0 - 1.0 / (past * RUBBER + 1.0);
        let (low, high) = self.bounds;
        if view < low {
            low - band(low - view)
        } else if view > high {
            high + band(view - high)
        } else {
            view
        }
    }

    /// Velocity of the view when the fingers lift at `time`, in workspaces per second, from
    /// the movements of the last `VELOCITY_WINDOW`. None past the bounds: the view comes back
    /// from where it is.
    pub fn velocity(&self, time: u32) -> f64 {
        let (low, high) = self.bounds;
        let recent: Vec<_> = self
            .samples
            .iter()
            .filter(|s| s.0.saturating_add(VELOCITY_WINDOW) >= time)
            .collect();
        let (Some(first), Some(last)) = (recent.first(), recent.last()) else {
            return 0.0;
        };
        let span = f64::from(last.0.saturating_sub(first.0)) / 1000.0;
        if span <= 0.0 || !(low..=high).contains(&self.view()) {
            return 0.0;
        }
        let travel: f64 = recent.iter().skip(1).map(|s| s.1).sum();
        (-travel / WORKSPACE_DISTANCE / span).clamp(-MAX_VELOCITY, MAX_VELOCITY)
    }

    /// Index of the workspace the view lands on when the fingers lift at `time`: the nearest
    /// one to where the velocity carries it, among the neighbours of the active one.
    pub fn landing(&self, time: u32) -> usize {
        let projected = self.view() + self.velocity(time) * PROJECTION;
        projected.round().clamp(self.bounds.0, self.bounds.1) as usize
    }

    /// Vertical travel of the fingers since the start, up being negative.
    pub fn vertical(&self) -> f64 {
        self.travel.y
    }

    /// The fingers travelled far enough up or down for the vertical swipe.
    pub fn reached(&self) -> bool {
        self.travel.y.abs() >= VERTICAL_DISTANCE
    }
}

/// Where the gesture in progress goes, chosen at its start.
#[derive(Debug)]
enum Route {
    /// The client under the pointer.
    Client,
    /// A swipe of the compositor's.
    Swipe(Swipe),
    /// Reserved: taken, without effect.
    Reserved,
}

/// The gestures in progress, and the protocol that hands them to clients.
pub(crate) struct Gestures {
    _protocol: PointerGesturesState,
    swipe: Option<Route>,
    pinch: Option<Route>,
    hold: Option<Route>,
}

impl Gestures {
    pub(crate) fn new(dh: &DisplayHandle) -> Self {
        Self {
            _protocol: PointerGesturesState::new::<State>(dh),
            swipe: None,
            pinch: None,
            hold: None,
        }
    }
}

/// Route of a pinch or a hold of `fingers`.
fn route(fingers: u32) -> Route {
    if fingers < COMPOSITOR_FINGERS {
        Route::Client
    } else {
        Route::Reserved
    }
}

impl State {
    /// Hands a gesture event of the backend to the methods below.
    pub(crate) fn gesture_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        // The screens off: a gesture brings them back, and does nothing else (lot D-38).
        if self.input_wakes() {
            self.screens_on();
            return;
        }
        // Locked: no gesture acts, nor reaches a client (lot D-25a).
        if self.is_locked() {
            return;
        }
        match event {
            InputEvent::GestureSwipeBegin { event } => {
                self.gesture_swipe_begin(event.fingers(), event.time_msec());
            }
            InputEvent::GestureSwipeUpdate { event } => {
                self.gesture_swipe_update(event.time_msec(), event.delta());
            }
            InputEvent::GestureSwipeEnd { event } => {
                self.gesture_swipe_end(event.time_msec(), event.cancelled());
            }
            InputEvent::GesturePinchBegin { event } => {
                self.gesture_pinch_begin(event.fingers(), event.time_msec());
            }
            InputEvent::GesturePinchUpdate { event } => {
                let (delta, scale, rotation) = (event.delta(), event.scale(), event.rotation());
                self.gesture_pinch_update(event.time_msec(), delta, scale, rotation);
            }
            InputEvent::GesturePinchEnd { event } => {
                self.gesture_pinch_end(event.time_msec(), event.cancelled());
            }
            InputEvent::GestureHoldBegin { event } => {
                self.gesture_hold_begin(event.fingers(), event.time_msec());
            }
            InputEvent::GestureHoldEnd { event } => {
                self.gesture_hold_end(event.time_msec(), event.cancelled());
            }
            _ => {}
        }
    }

    /// Runs `send` with the pointer, which hands the gesture to the client it is over.
    fn send_to_client(&mut self, send: impl FnOnce(&PointerHandle<State>, &mut State)) {
        if let Some(pointer) = self.seat.get_pointer() {
            send(&pointer, self);
        }
    }

    pub fn gesture_swipe_begin(&mut self, fingers: u32, time: u32) {
        self.queue_redraw();
        let route = match fingers {
            COMPOSITOR_FINGERS => {
                let view = self.workspace_view(self.now());
                let model = self.workspaces();
                Route::Swipe(Swipe::new(view, model.active_index(), model.list().len()))
            }
            f if f > COMPOSITOR_FINGERS => Route::Reserved,
            _ => {
                let serial = SERIAL_COUNTER.next_serial();
                let event = GestureSwipeBeginEvent {
                    serial,
                    time,
                    fingers,
                };
                self.send_to_client(|p, state| p.gesture_swipe_begin(state, &event));
                Route::Client
            }
        };
        tracing::debug!(fingers, ?route, "swipe");
        self.gestures.swipe = Some(route);
    }

    pub fn gesture_swipe_update(&mut self, time: u32, delta: Point<f64, Logical>) {
        self.queue_redraw();
        match &mut self.gestures.swipe {
            Some(Route::Client) => {
                let event = GestureSwipeUpdateEvent { time, delta };
                self.send_to_client(|p, state| p.gesture_swipe_update(state, &event));
            }
            Some(Route::Swipe(swipe)) => {
                if let Some(view) = swipe.update(time, delta) {
                    self.hold_workspaces(view);
                } else if matches!(swipe.direction(), Some(Direction::Up | Direction::Down)) {
                    // Mission Control follows the fingers (lot D-33).
                    let travel = swipe.vertical();
                    self.follow_overview(travel);
                }
            }
            Some(Route::Reserved) | None => {}
        }
    }

    pub fn gesture_swipe_end(&mut self, time: u32, cancelled: bool) {
        self.queue_redraw();
        match self.gestures.swipe.take() {
            Some(Route::Client) => {
                let serial = SERIAL_COUNTER.next_serial();
                let event = GestureSwipeEndEvent {
                    serial,
                    time,
                    cancelled,
                };
                self.send_to_client(|p, state| p.gesture_swipe_end(state, &event));
            }
            Some(Route::Swipe(swipe)) => self.finish_swipe(&swipe, time, cancelled),
            Some(Route::Reserved) | None => {}
        }
    }

    /// The fingers of a three finger swipe lifted at `time`.
    fn finish_swipe(&mut self, swipe: &Swipe, time: u32, cancelled: bool) {
        let done = !cancelled && swipe.reached();
        match swipe.direction() {
            Some(Direction::Horizontal) if cancelled => {
                let active = self.workspaces().active_index();
                self.release_workspaces(active, 0.0);
            }
            Some(Direction::Horizontal) => {
                self.release_workspaces(swipe.landing(time), swipe.velocity(time));
            }
            Some(direction @ (Direction::Up | Direction::Down))
                if self.release_overview(direction == Direction::Up, done) => {}
            Some(Direction::Down)
                if done && self.touchpad.config().swipe_down == SwipeDown::Minimize =>
            {
                if let Some(window) = self.focused_window() {
                    self.minimize(&window);
                }
            }
            _ => {}
        }
    }

    /// Ends every gesture in progress, cancelled: the touchpad goes away in the middle of one
    /// (console switch), and its end would never come. A held row of workspaces comes back.
    pub fn cancel_gestures(&mut self) {
        let time = self.now().as_millis() as u32;
        if self.gestures.swipe.is_some() {
            self.gesture_swipe_end(time, true);
        }
        if self.gestures.pinch.is_some() {
            self.gesture_pinch_end(time, true);
        }
        if self.gestures.hold.is_some() {
            self.gesture_hold_end(time, true);
        }
    }

    pub fn gesture_pinch_begin(&mut self, fingers: u32, time: u32) {
        let route = route(fingers);
        if let Route::Client = route {
            let serial = SERIAL_COUNTER.next_serial();
            let event = GesturePinchBeginEvent {
                serial,
                time,
                fingers,
            };
            self.send_to_client(|p, state| p.gesture_pinch_begin(state, &event));
        }
        self.gestures.pinch = Some(route);
    }

    pub fn gesture_pinch_update(
        &mut self,
        time: u32,
        delta: Point<f64, Logical>,
        scale: f64,
        rotation: f64,
    ) {
        if let Some(Route::Client) = self.gestures.pinch {
            let event = GesturePinchUpdateEvent {
                time,
                delta,
                scale,
                rotation,
            };
            self.send_to_client(|p, state| p.gesture_pinch_update(state, &event));
        }
    }

    pub fn gesture_pinch_end(&mut self, time: u32, cancelled: bool) {
        if let Some(Route::Client) = self.gestures.pinch.take() {
            let serial = SERIAL_COUNTER.next_serial();
            let event = GesturePinchEndEvent {
                serial,
                time,
                cancelled,
            };
            self.send_to_client(|p, state| p.gesture_pinch_end(state, &event));
        }
    }

    pub fn gesture_hold_begin(&mut self, fingers: u32, time: u32) {
        let route = route(fingers);
        if let Route::Client = route {
            let serial = SERIAL_COUNTER.next_serial();
            let event = GestureHoldBeginEvent {
                serial,
                time,
                fingers,
            };
            self.send_to_client(|p, state| p.gesture_hold_begin(state, &event));
        }
        self.gestures.hold = Some(route);
    }

    pub fn gesture_hold_end(&mut self, time: u32, cancelled: bool) {
        if let Some(Route::Client) = self.gestures.hold.take() {
            let serial = SERIAL_COUNTER.next_serial();
            let event = GestureHoldEndEvent {
                serial,
                time,
                cancelled,
            };
            self.send_to_client(|p, state| p.gesture_hold_end(state, &event));
        }
    }
}

delegate_pointer_gestures!(State);

#[cfg(test)]
mod tests {
    use super::*;

    /// A swipe from the first of three workspaces, moved by `steps` of `dx` every 10 ms.
    fn swiped(origin: f64, active: usize, steps: u32, dx: f64) -> Swipe {
        let mut swipe = Swipe::new(origin, active, 3);
        for i in 1..=steps {
            swipe.update(i * 10, (dx, 0.0).into());
        }
        swipe
    }

    #[test]
    fn the_direction_waits_for_the_lock_distance() {
        let mut swipe = Swipe::new(0.0, 0, 3);
        assert_eq!(swipe.update(10, (-5.0, 3.0).into()), None);
        assert_eq!(swipe.direction(), None);
        // Past the lock, the larger axis wins.
        let view = swipe.update(20, (-12.0, 2.0).into());
        assert_eq!(swipe.direction(), Some(Direction::Horizontal));
        assert_eq!(view, Some(17.0 / WORKSPACE_DISTANCE));
        let mut up = Swipe::new(0.0, 0, 3);
        assert_eq!(up.update(10, (4.0, -20.0).into()), None);
        assert_eq!(up.direction(), Some(Direction::Up));
        // Once vertical, a horizontal movement does not move the workspaces.
        assert_eq!(up.update(20, (-300.0, 0.0).into()), None);
        assert!(!up.reached());
        up.update(30, (0.0, -80.0).into());
        assert!(up.reached());
        let mut down = Swipe::new(0.0, 0, 3);
        down.update(10, (0.0, 30.0).into());
        assert_eq!(down.direction(), Some(Direction::Down));
    }

    #[test]
    fn the_view_follows_the_fingers_with_a_rubber_band_at_the_ends() {
        // Fingers to the left: the next workspace comes in from the right.
        let swipe = swiped(0.0, 0, 10, -20.0);
        assert!((swipe.view() - 0.5).abs() < 1e-9);
        // Fingers to the right on the first workspace: it resists, less and less.
        let past = swiped(0.0, 0, 10, 20.0).view();
        assert!(past < 0.0 && past > -0.5 * RUBBER, "{past}");
        let further = swiped(0.0, 0, 40, 20.0).view();
        assert!(further < past && further > -1.0, "{further}");
        // At most one workspace away from the active one.
        let beyond = swiped(0.0, 0, 40, -20.0).view();
        assert!(beyond > 1.0 && beyond < 2.0, "{beyond}");
        // From the middle of a slide, the view starts where it was.
        assert!((swiped(0.6, 1, 1, -40.0).view() - 0.7).abs() < 1e-9);
    }

    #[test]
    fn the_velocity_decides_where_the_view_lands() {
        // Slow, a quarter of the way: back to where it started.
        let slow = swiped(0.0, 0, 50, -2.0);
        assert!((slow.velocity(500) - 0.5).abs() < 1e-9);
        assert_eq!(slow.landing(500), 0);
        // Thrown, a quarter of the way: on to the next one.
        let thrown = swiped(0.0, 0, 5, -20.0);
        let velocity = thrown.velocity(50);
        assert!((velocity - 5.0).abs() < 1e-9, "{velocity}");
        assert_eq!(thrown.landing(50), 1);
        // Past the middle, without speed: the next one.
        let far = swiped(0.0, 0, 30, -8.0);
        assert_eq!(far.landing(300 + VELOCITY_WINDOW + 1), 1);
        // Fingers that stopped before lifting throw nothing.
        assert_eq!(thrown.velocity(50 + VELOCITY_WINDOW + 1), 0.0);
        assert_eq!(thrown.landing(50 + VELOCITY_WINDOW + 1), 0);
        // A throw never goes past the neighbour, nor out of the row.
        let hard = swiped(1.0, 1, 5, -80.0);
        assert!(hard.velocity(50) <= MAX_VELOCITY);
        assert_eq!(hard.landing(50), 2);
        assert_eq!(swiped(0.0, 0, 5, 80.0).landing(50), 0);
    }
}
