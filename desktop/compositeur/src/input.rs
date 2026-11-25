//! Clavier et souris : focus au clic, Super + glisser pour déplacer, Super + clic droit
//! pour redimensionner.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, ButtonState, Event, InputBackend, InputEvent,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    desktop::{Space, Window},
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState,
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Size},
    wayland::{compositor::with_states, shell::xdg::SurfaceCachedState},
};

use crate::{
    State,
    layout::{self, Edges, Resize},
    window_data,
};

// Codes des boutons, tirés de linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Geste Super + bouton en cours : le compositeur déplace ou redimensionne une fenêtre.
pub(crate) struct Gesture {
    window: Window,
    start: Point<f64, Logical>,
    initial: Rectangle<i32, Logical>,
    /// `None` pour un déplacement, les bords tirés pour un redimensionnement.
    edges: Option<Edges>,
    last_size: Size<i32, Logical>,
}

impl Gesture {
    fn begin(
        window: Window,
        start: Point<f64, Logical>,
        initial: Rectangle<i32, Logical>,
        edges: Option<Edges>,
    ) -> Self {
        let data = window_data(&window);
        match (edges, window.toplevel()) {
            // Le recalage sur le bord opposé vaut pendant tout le geste, pas seulement à la fin.
            (Some(edges), _) => data.resize.set(Some(Resize {
                edges,
                initial,
                done: false,
            })),
            // Déplacer une fenêtre agrandie la rend à sa taille d'avant, sous le pointeur.
            (None, Some(toplevel)) => {
                let unmaximized = toplevel.with_pending_state(|s| {
                    let maximized = s.states.contains(XdgState::Maximized)
                        && !s.states.contains(XdgState::Fullscreen);
                    if maximized {
                        s.states.unset(XdgState::Maximized);
                        s.size = data
                            .restore
                            .take()
                            .map(|r| r.size)
                            .filter(|r| r.w > 0 && r.h > 0);
                    }
                    maximized
                });
                if unmaximized {
                    toplevel.send_pending_configure();
                }
            }
            (None, None) => {}
        }
        Self {
            window,
            start,
            initial,
            edges,
            last_size: initial.size,
        }
    }

    fn update(&mut self, space: &mut Space<Window>, pointer: Point<f64, Logical>) {
        let Some(toplevel) = self.window.toplevel() else {
            return;
        };
        let Some(edges) = self.edges else {
            let loc = layout::moved(self.initial.loc, self.start, pointer);
            space.map_element(self.window.clone(), loc, true);
            return;
        };
        let (min, max) = with_states(toplevel.wl_surface(), |states| {
            let mut cached = states.cached_state.get::<SurfaceCachedState>();
            (cached.current().min_size, cached.current().max_size)
        });
        self.last_size = layout::resized(self.initial, edges, pointer - self.start, min, max);
        toplevel.with_pending_state(|s| {
            s.states.set(XdgState::Resizing);
            s.size = Some(self.last_size);
        });
        toplevel.send_pending_configure();
    }

    fn finish(self) {
        let (Some(toplevel), Some(edges)) = (self.window.toplevel(), self.edges) else {
            return;
        };
        toplevel.with_pending_state(|s| {
            s.states.unset(XdgState::Resizing);
            s.size = Some(self.last_size);
        });
        toplevel.send_pending_configure();
        // Le recalage de la position se fait au prochain envoi du client, le dernier du geste.
        window_data(&self.window).resize.set(Some(Resize {
            edges,
            initial: self.initial,
            done: true,
        }));
    }
}

impl State {
    /// Traite un événement d'entrée venu du backend.
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event } => {
                let Some(keyboard) = self.seat.get_keyboard() else {
                    return;
                };
                let serial = SERIAL_COUNTER.next_serial();
                let (key, state, time) = (event.key_code(), event.state(), event.time_msec());
                keyboard.input::<(), _>(self, key, state, serial, time, |_, _, _| {
                    FilterResult::Forward
                });
            }
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

    fn on_motion(&mut self, location: Point<f64, Logical>, time: u32) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        // Pendant un geste, aucun client n'a le pointeur.
        let under = match self.gesture.as_mut() {
            Some(gesture) => {
                gesture.update(&mut self.space, location);
                None
            }
            None => self.surface_under(location),
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

    fn on_button(&mut self, button: u32, state: ButtonState, time: u32) {
        let (Some(pointer), Some(keyboard)) = (self.seat.get_pointer(), self.seat.get_keyboard())
        else {
            return;
        };
        let location = pointer.current_location();
        // Le client n'a vu ni l'appui ni le relâchement d'un geste.
        if let Some(gesture) = self.gesture.take() {
            if state == ButtonState::Pressed {
                self.gesture = Some(gesture);
            } else {
                gesture.finish();
                self.on_motion(location, time);
            }
            return;
        }
        if state == ButtonState::Pressed && !pointer.is_grabbed() {
            let window = self.space.element_under(location).map(|(w, _)| w.clone());
            self.focus(window.clone());
            let rect = window.as_ref().and_then(|w| self.space.element_geometry(w));
            let edges = match button {
                BTN_LEFT => Some(None),
                BTN_RIGHT => rect.map(|r| Some(Edges::nearest(r, location))),
                _ => None,
            };
            if let (Some(window), Some(initial), Some(edges), true) =
                (window, rect, edges, keyboard.modifier_state().logo)
            {
                self.gesture = Some(Gesture::begin(window, location, initial, edges));
                self.on_motion(location, time);
                return;
            }
        }
        let serial = SERIAL_COUNTER.next_serial();
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
