//! Clavier et souris : focus au clic, Super + glisser pour déplacer, Super + clic droit
//! pour redimensionner, Super + Q pour fermer.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, ButtonState, Event, InputBackend, InputEvent,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    desktop::Window,
    input::{
        keyboard::{FilterResult, Keysym},
        pointer::{
            AxisFrame, ButtonEvent, Focus, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData,
            MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
        },
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Size},
    wayland::{compositor::with_states, shell::xdg::SurfaceCachedState},
};

use crate::{
    State,
    layout::{self, Edges},
    window_data,
};

// Codes des boutons, tirés de linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

impl State {
    /// Traite un événement d'entrée venu du backend.
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event } => self.on_key::<I>(&event),
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(area) = self.output_area() else {
                    return;
                };
                let pos = event.position_transformed(area.size) + area.loc.to_f64();
                self.on_motion(pos, event.time_msec());
            }
            InputEvent::PointerButton { event } => {
                self.on_button(event.button_code(), event.state(), event.time_msec());
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

    fn on_key<I: InputBackend>(&mut self, event: &I::KeyboardKeyEvent) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        let close = keyboard.input(
            self,
            event.key_code(),
            event.state(),
            serial,
            event.time_msec(),
            |_, modifiers, handle| {
                let pressed = event.state() == smithay::backend::input::KeyState::Pressed;
                let is_q = handle.modified_sym() == Keysym::q || handle.modified_sym() == Keysym::Q;
                if pressed && modifiers.logo && is_q {
                    FilterResult::Intercept(true)
                } else {
                    FilterResult::Forward
                }
            },
        );
        if close == Some(true) {
            self.close_focused();
        }
    }

    fn on_motion(&mut self, pos: Point<f64, Logical>, time: u32) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let under = self.surface_under(pos);
        let serial = SERIAL_COUNTER.next_serial();
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time,
            },
        );
        pointer.frame(self);
    }

    fn on_button(&mut self, button: u32, state: ButtonState, time: u32) {
        let (Some(pointer), Some(keyboard)) = (self.seat.get_pointer(), self.seat.get_keyboard())
        else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        if state == ButtonState::Pressed && !pointer.is_grabbed() {
            let location = pointer.current_location();
            let window = self.space.element_under(location).map(|(w, _)| w.clone());
            self.focus(window.clone());
            // Super + bouton : le compositeur prend le geste, le client ne voit pas le clic.
            if let Some(window) = window.filter(|_| keyboard.modifier_state().logo) {
                let edges = match button {
                    BTN_LEFT => Some(None),
                    BTN_RIGHT => self
                        .space
                        .element_geometry(&window)
                        .map(|rect| Some(Edges::nearest(rect, location))),
                    _ => None,
                };
                let start = GrabStartData {
                    focus: None,
                    button,
                    location,
                };
                if let Some(grab) = edges.and_then(|e| self.start_grab(start, window, e)) {
                    pointer.set_grab(self, grab, serial, Focus::Clear);
                }
            }
        }
        pointer.button(
            self,
            &ButtonEvent {
                button,
                state,
                serial,
                time,
            },
        );
        pointer.frame(self);
    }
}

/// Ce que fait un geste en cours.
pub(crate) enum GrabKind {
    Move {
        initial: Point<i32, Logical>,
    },
    Resize {
        edges: Edges,
        initial: Rectangle<i32, Logical>,
        last: Size<i32, Logical>,
    },
}

/// Geste du pointeur sur une fenêtre, jusqu'au relâchement de tous les boutons.
pub(crate) struct WindowGrab {
    pub(crate) start: GrabStartData<State>,
    pub(crate) window: Window,
    pub(crate) kind: GrabKind,
}

impl WindowGrab {
    fn update(&mut self, data: &mut State, pointer: Point<f64, Logical>) {
        let Some(toplevel) = self.window.toplevel() else {
            return;
        };
        match &mut self.kind {
            GrabKind::Move { initial } => {
                let loc = layout::moved(*initial, self.start.location, pointer);
                data.space.map_element(self.window.clone(), loc, true);
            }
            GrabKind::Resize {
                edges,
                initial,
                last,
            } => {
                let (min, max) = with_states(toplevel.wl_surface(), |states| {
                    let mut cached = states.cached_state.get::<SurfaceCachedState>();
                    let current = cached.current();
                    (current.min_size, current.max_size)
                });
                let delta = pointer - self.start.location;
                *last = layout::resized(*initial, *edges, delta, min, max);
                toplevel.with_pending_state(|s| {
                    s.states.set(xdg_toplevel::State::Resizing);
                    s.size = Some(*last);
                });
                toplevel.send_pending_configure();
            }
        }
    }

    fn finish(&mut self) {
        let (
            Some(toplevel),
            GrabKind::Resize {
                edges,
                initial,
                last,
            },
        ) = (self.window.toplevel(), &self.kind)
        else {
            return;
        };
        toplevel.with_pending_state(|s| {
            s.states.unset(xdg_toplevel::State::Resizing);
            s.size = Some(*last);
        });
        toplevel.send_pending_configure();
        // La position se recale au prochain envoi du client, qui sera le dernier du geste.
        window_data(&self.window)
            .resize
            .set(Some((*edges, *initial, true)));
    }
}

/// Les événements que le geste ne traite pas passent tels quels.
macro_rules! forward {
    ($($name:ident: $event:ty),* $(,)?) => {$(
        fn $name(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &$event) {
            handle.$name(data, event);
        }
    )*};
}

impl PointerGrab<State> for WindowGrab {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // Pendant le geste, aucun client n'a le pointeur.
        handle.motion(data, None, event);
        self.update(data, event.location);
    }

    fn relative_motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, None, event);
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            self.finish();
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(data);
    }

    forward! {
        gesture_swipe_begin: GestureSwipeBeginEvent,
        gesture_swipe_update: GestureSwipeUpdateEvent,
        gesture_swipe_end: GestureSwipeEndEvent,
        gesture_pinch_begin: GesturePinchBeginEvent,
        gesture_pinch_update: GesturePinchUpdateEvent,
        gesture_pinch_end: GesturePinchEndEvent,
        gesture_hold_begin: GestureHoldBeginEvent,
        gesture_hold_end: GestureHoldEndEvent,
    }

    fn start_data(&self) -> &GrabStartData<State> {
        &self.start
    }

    fn unset(&mut self, _data: &mut State) {}
}
