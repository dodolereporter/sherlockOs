//! Clavier et souris : focus au clic, gestes de déplacement et de redimensionnement, calage,
//! raccourcis et forme du curseur.
//!
//! Un geste vient de la touche des gestes (voir [`gesture_modifier_held`]) avec un bouton, d'un
//! appui dans la bande qui entoure une fenêtre, ou d'une demande du client (`xdg_toplevel.move`
//! ou `.resize`, sa barre de titre ou ses bords).

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
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Size},
    wayland::{compositor::with_states, shell::xdg::SurfaceCachedState},
};

use crate::{
    State,
    handlers::is_fullscreen,
    layout::{self, Edges, Resize, Zone},
    window_data,
};

// Codes des boutons, tirés de linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Touche des gestes du compositeur. Super en session réelle. En mode imbriqué, la session hôte
/// garde Super pour elle (sous Plasma : actions des fenêtres de KWin, lanceur), alors c'est Alt,
/// comme le fait niri.
pub(crate) fn gesture_modifier_held(nested: bool, modifiers: &ModifiersState) -> bool {
    if nested {
        modifiers.alt
    } else {
        modifiers.logo
    }
}

/// D'où vient un geste, pour le journal.
#[derive(Debug, Clone, Copy)]
pub(crate) enum GestureSource {
    /// Touche des gestes et bouton de la souris.
    Modifier,
    /// Appui dans la bande de saisie qui entoure la fenêtre.
    Border,
    /// Demande du client : `xdg_toplevel.move` ou `xdg_toplevel.resize`.
    Client,
}

/// Geste en cours : le compositeur déplace ou redimensionne une fenêtre.
pub(crate) struct Gesture {
    window: Window,
    source: GestureSource,
    start: Point<f64, Logical>,
    initial: Rectangle<i32, Logical>,
    /// `None` pour un déplacement, les bords tirés pour un redimensionnement.
    edges: Option<Edges>,
    last_size: Size<i32, Logical>,
    /// Zone où la fenêtre déplacée sera calée si on la lâche maintenant.
    zone: Option<Zone>,
}

/// Ce qui est sous le pointeur.
pub(crate) enum Hit {
    /// Une surface de client, avec la position de son origine.
    Surface(Window, WlSurface, Point<f64, Logical>),
    /// La bande de saisie autour d'une fenêtre, et les bords qu'elle tire.
    Border(Window, Edges),
}

/// Raccourci du compositeur : touche des gestes et une flèche.
#[derive(Debug, Clone, Copy)]
enum Shortcut {
    /// Flèche haut : agrandit, ou rend la place d'avant.
    ToggleMaximized,
    /// Flèche gauche ou droite : moitié de l'écran.
    Snap(Zone),
    /// Flèche bas : rend la place d'avant.
    Restore,
}

impl Shortcut {
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

/// Forme du curseur pour tirer ces bords.
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
    /// Lance un geste sur `window`, depuis la position `start` du pointeur. `edges` vaut `None`
    /// pour un déplacement. Une fenêtre en plein écran ne bouge pas.
    pub(crate) fn start_gesture(
        &mut self,
        window: Window,
        start: Point<f64, Logical>,
        edges: Option<Edges>,
        source: GestureSource,
    ) {
        let (Some(toplevel), Some(mut initial)) =
            (window.toplevel(), self.space.element_geometry(&window))
        else {
            return;
        };
        if is_fullscreen(&window) {
            tracing::debug!(?source, "geste refusé : la fenêtre est en plein écran");
            return;
        }
        // Un geste sort la fenêtre de son agrandissement ou de sa zone.
        let data = window_data(&window);
        let was_zoned = data.zone.take().is_some();
        let was_maximized = toplevel.with_pending_state(|s| s.states.unset(XdgState::Maximized));
        let restore = data.restore.take();
        if was_zoned || was_maximized {
            // Déplacée, elle reprend sa taille d'avant sous le pointeur. Redimensionnée, elle
            // part de sa taille actuelle.
            if let (None, Some(restore)) = (edges, restore.filter(|r| !r.size.is_empty())) {
                initial = Rectangle::new(
                    layout::unsnapped(initial, restore.size, start),
                    restore.size,
                );
                toplevel.with_pending_state(|s| s.size = Some(restore.size));
                self.space.map_element(window.clone(), initial.loc, true);
            }
            toplevel.send_pending_configure();
        }
        if let Some(edges) = edges {
            // Le recalage sur le bord opposé vaut pendant tout le geste, pas seulement à la fin.
            data.resize.set(Some(Resize {
                edges,
                initial,
                done: false,
            }));
        }
        tracing::debug!(
            ?source,
            ?edges,
            ?initial,
            "début du geste (bords vides : déplacement)"
        );
        self.gesture = Some(Gesture {
            window,
            source,
            start,
            initial,
            edges,
            last_size: initial.size,
            zone: None,
        });
    }

    fn update_gesture(&mut self, pointer: Point<f64, Logical>) {
        let area = self.output_area();
        let Some(gesture) = self.gesture.as_mut() else {
            return;
        };
        let Some(toplevel) = gesture.window.toplevel() else {
            return;
        };
        let Some(edges) = gesture.edges else {
            let loc = layout::moved(gesture.initial.loc, gesture.start, pointer);
            self.space.map_element(gesture.window.clone(), loc, true);
            gesture.zone =
                area.and_then(|a| Zone::under(a, pointer, ui::SNAP_EDGE, ui::SNAP_CORNER));
            return;
        };
        let (min, max) = with_states(toplevel.wl_surface(), |states| {
            let mut cached = states.cached_state.get::<SurfaceCachedState>();
            (cached.current().min_size, cached.current().max_size)
        });
        gesture.last_size =
            layout::resized(gesture.initial, edges, pointer - gesture.start, min, max);
        toplevel.with_pending_state(|s| {
            s.states.set(XdgState::Resizing);
            s.size = Some(gesture.last_size);
        });
        toplevel.send_pending_configure();
    }

    fn finish_gesture(&mut self, gesture: Gesture) {
        let Gesture {
            window,
            source,
            initial,
            edges,
            last_size,
            zone,
            ..
        } = gesture;
        tracing::debug!(?source, ?edges, size = ?last_size, ?zone, "fin du geste");
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let Some(edges) = edges else {
            if let Some(zone) = zone {
                // Lâchée contre un bord : calée. Elle retrouvera la taille qu'elle avait pendant
                // le déplacement, là où on l'a lâchée.
                let loc = self.space.element_location(&window).unwrap_or(initial.loc);
                window_data(&window)
                    .restore
                    .set(Some(Rectangle::new(loc, initial.size)));
                self.snap(&window, Some(zone));
            }
            return;
        };
        toplevel.with_pending_state(|s| {
            s.states.unset(XdgState::Resizing);
            s.size = Some(last_size);
        });
        toplevel.send_pending_configure();
        // Le recalage de la position se fait au prochain envoi du client, le dernier du geste.
        window_data(&window).resize.set(Some(Resize {
            edges,
            initial,
            done: true,
        }));
    }

    /// Un geste de déplacement ou de redimensionnement est en cours.
    pub fn gesture_active(&self) -> bool {
        self.gesture.is_some()
    }

    /// Rectangle où la fenêtre déplacée sera calée si on la lâche maintenant, pour l'aperçu.
    pub fn snap_preview(&self) -> Option<Rectangle<i32, Logical>> {
        let zone = self.gesture.as_ref()?.zone?;
        Some(zone.geometry(self.output_area()?))
    }

    /// Forme du curseur, `None` pour le cacher. Pendant un geste et au-dessus de la bande de
    /// saisie, c'est le compositeur qui la choisit. Au-dessus d'un client, c'est lui, s'il la
    /// nomme (protocole des formes de curseur) ; s'il dessine son curseur lui-même, la flèche.
    pub fn cursor_icon(&self) -> Option<CursorIcon> {
        if let Some(gesture) = &self.gesture {
            return Some(gesture.edges.map_or(CursorIcon::Move, resize_cursor));
        }
        let location = self.seat.get_pointer()?.current_location();
        match (self.hit(location), &self.cursor_status) {
            (Some(Hit::Border(_, edges)), _) => Some(resize_cursor(edges)),
            (Some(Hit::Surface(..)), CursorImageStatus::Named(icon)) => Some(*icon),
            (Some(Hit::Surface(..)), CursorImageStatus::Hidden) => None,
            _ => Some(CursorIcon::Default),
        }
    }

    /// Ce qui est sous le pointeur en `pos`. La bande d'une fenêtre passe devant les fenêtres
    /// qu'elle recouvre, pas devant celles qui sont au-dessus d'elle.
    pub(crate) fn hit(&self, pos: Point<f64, Logical>) -> Option<Hit> {
        let under = self.space.element_under(pos);
        for window in self.space.elements().rev() {
            if let Some((top, loc)) = under
                && top == window
            {
                let (surface, origin) =
                    window.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)?;
                return Some(Hit::Surface(
                    window.clone(),
                    surface,
                    (origin + loc).to_f64(),
                ));
            }
            let edges = self
                .space
                .element_geometry(window)
                .filter(|_| !is_fullscreen(window))
                .and_then(|r| layout::border_edges(r, pos, ui::RESIZE_BAND, ui::RESIZE_CORNER));
            if let Some(edges) = edges {
                return Some(Hit::Border(window.clone(), edges));
            }
        }
        None
    }

    /// Une touche (code xkb) est appuyée ou relâchée. Point d'entrée de tous les backends, et
    /// des tests.
    pub fn keyboard_key(&mut self, key: Keycode, key_state: KeyState, time: u32) {
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
            match Shortcut::for_key(sym.modified_sym()) {
                Some(shortcut) if gesture_modifier_held(state.nested, mods) => {
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
        let focus = self.seat.get_keyboard().and_then(|k| k.current_focus());
        let Some(window) = focus.and_then(|surface| self.window_for(&surface)) else {
            return;
        };
        tracing::debug!(?shortcut, "raccourci");
        let maximized = window
            .toplevel()
            .is_some_and(|t| t.with_pending_state(|s| s.states.contains(XdgState::Maximized)));
        let zone = match shortcut {
            Shortcut::ToggleMaximized if !maximized => Some(Zone::Maximized),
            Shortcut::ToggleMaximized | Shortcut::Restore => None,
            Shortcut::Snap(zone) => Some(zone),
        };
        self.snap(&window, zone);
    }

    /// Traite un événement d'entrée venu du backend.
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event } => {
                self.keyboard_key(event.key_code(), event.state(), event.time_msec());
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(area) = self.output_area() else {
                    return;
                };
                let pos = event.position_transformed(area.size) + area.loc.to_f64();
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

    /// Le pointeur est à `location`. Point d'entrée de tous les backends, et des tests.
    pub fn pointer_motion(&mut self, location: Point<f64, Logical>, time: u32) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        // Pendant un geste, aucun client n'a le pointeur.
        let under = if self.gesture_active() {
            self.update_gesture(location);
            None
        } else {
            match self.hit(location) {
                Some(Hit::Surface(_, surface, origin)) => Some((surface, origin)),
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

    /// Un bouton (code de linux/input-event-codes.h) est appuyé ou relâché.
    pub fn pointer_button(&mut self, button: u32, state: ButtonState, time: u32) {
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
            if state == ButtonState::Pressed {
                self.gesture = Some(gesture);
            } else {
                self.finish_gesture(gesture);
                // Le pointeur n'a pas de focus pendant le geste : aucun client ne voit ce
                // relâchement, mais le siège oublie le bouton. Puis le focus revient.
                pointer.button(self, &event);
                pointer.frame(self);
                self.pointer_motion(location, time);
            }
            return;
        }
        if state == ButtonState::Pressed && !pointer.is_grabbed() {
            let hit = self.hit(location);
            let window = match &hit {
                Some(Hit::Surface(window, ..) | Hit::Border(window, _)) => Some(window.clone()),
                None => None,
            };
            self.focus(window.clone());
            let rect = window.as_ref().and_then(|w| self.space.element_geometry(w));
            let held = gesture_modifier_held(self.nested, &keyboard.modifier_state());
            let gesture = match (button, held, hit) {
                // Touche des gestes + bouton : le client ne voit pas l'appui.
                (BTN_LEFT, true, _) => Some((None, GestureSource::Modifier)),
                (BTN_RIGHT, true, _) => rect.map(|r| {
                    let edges = Edges::nearest(r, location);
                    (Some(edges), GestureSource::Modifier)
                }),
                // Appui dans la bande autour d'une fenêtre : on tire ce bord ou ce coin.
                (BTN_LEFT, false, Some(Hit::Border(_, edges))) => {
                    Some((Some(edges), GestureSource::Border))
                }
                _ => None,
            };
            if let (Some(window), Some((edges, source))) = (window, gesture) {
                self.start_gesture(window, location, edges, source);
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
