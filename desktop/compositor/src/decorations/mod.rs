//! Server-side decorations (lot D-08): the title bar and the three buttons the compositor draws
//! above the windows of the applications that leave their decoration to it.
//!
//! - `protocol`: who gets one. A Wayland client asks through xdg-decoration, or through KDE's
//!   older protocol; an X11 window has one unless it is a menu or asks for none.
//! - here: the frame (the bar above the content), what the pointer is on, and what the
//!   buttons, a double click and a drag on the bar do.
//! - `raster`: the bar drawn on the processor into one texture, made again only when it
//!   changes.
//!
//! The space keeps the geometry of the content, the one the client knows: the bar sits above
//! it, and the frame is both. Sizes are logical; colors, measures and the double click delay
//! come from `sherlock_ui::decorations`. See "Server-side decorations" in
//! `docs/compositor.md`.

mod protocol;
pub(crate) mod raster;

pub use raster::{Bar, ButtonLook, Look, Textures, draw};

use std::time::Duration;

use sherlock_ui::{
    decorations::{self as ui, Button},
    metrics,
    motion::{Animation, Fade},
    window as ui_window,
};
use smithay::{
    desktop::Window,
    reexports::{
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
        wayland_server::DisplayHandle,
    },
    utils::{IsAlive as _, Logical, Point, Rectangle, Size},
    wayland::shell::{kde::decoration::KdeDecorationState, xdg::decoration::XdgDecorationState},
    xwayland::xwm::WmWindowType,
};

use crate::{
    State,
    handlers::{is_fullscreen, is_maximized},
    input::{BTN_LEFT, GestureSource},
    layout::{self, Zone},
};

/// The decoration protocols, the pointer on the decorations, and the motion of the buttons.
pub struct Decorations {
    _xdg: XdgDecorationState,
    kde: KdeDecorationState,
    pointer: Pointer,
    /// The buttons of the windows the pointer is on or has just left, while they move.
    motions: Vec<(Window, Motion)>,
}

impl Decorations {
    pub fn new(dh: &DisplayHandle) -> Self {
        let (xdg, kde) = protocol::globals(dh);
        Self {
            _xdg: xdg,
            kde,
            pointer: Pointer::default(),
            motions: Vec::new(),
        }
    }
}

/// The hover and the press of the buttons of one window (lot C-01): the pill lights its
/// icons when the pointer enters it; a button under the pointer takes its color with a fade
/// and grows a little on a spring, and sinks when pressed.
#[derive(Debug, Clone, Copy)]
pub struct Motion {
    pill: Fade,
    lit: [Fade; 3],
    scale: [Animation; 3],
}

impl Motion {
    fn new(now: Duration) -> Self {
        Self {
            pill: Fade::new(0.0, 0.0, now, Duration::ZERO),
            lit: [Fade::new(0.0, 0.0, now, Duration::ZERO); 3],
            scale: [Animation::new(ui::BUTTON_SPRING, 1.0, 1.0, now); 3],
        }
    }

    /// The pointer is on the pill or not, on `hovered` or none, holding `pressed` or none.
    pub fn aim(
        &mut self,
        now: Duration,
        pill: bool,
        hovered: Option<Button>,
        pressed: Option<Button>,
    ) {
        let target = |on: bool| if on { 1.0 } else { 0.0 };
        let pill = target(pill);
        if self.pill.target() != pill {
            self.pill.retarget(now, pill, ui::HOVER_FADE);
        }
        for (i, button) in Button::ALL.into_iter().enumerate() {
            let lit = target(hovered == Some(button));
            if self.lit[i].target() != lit {
                self.lit[i].retarget(now, lit, ui::HOVER_FADE);
            }
            let scale = match (pressed == Some(button), hovered == Some(button)) {
                (true, true) => ui::PRESS_SCALE,
                (_, true) => ui::HOVER_SCALE,
                _ => 1.0,
            };
            if self.scale[i].target() != f64::from(scale) {
                self.scale[i].retarget(now, f64::from(scale));
            }
        }
    }

    /// What the buttons look like at `now`, and the share of the pill's icons shown.
    pub fn look(&self, now: Duration) -> ([ButtonLook; 3], u8) {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        // A settled spring is at its target exactly: the same look, the same texture.
        let scale = |a: &Animation| match a.is_done(now, sherlock_ui::motion::SCALE_PRECISION) {
            true => a.target(),
            false => a.value(now),
        };
        let buttons = std::array::from_fn(|i| ButtonLook {
            lit: byte(self.lit[i].value(now)),
            scale: (scale(&self.scale[i]) * 1000.0).round().clamp(0.0, 2000.0) as u16,
        });
        (buttons, byte(self.pill.value(now)))
    }

    /// Settled at `now`.
    pub fn is_done(&self, now: Duration) -> bool {
        self.pill.is_done(now)
            && self.lit.iter().all(|f| f.is_done(now))
            && self
                .scale
                .iter()
                .all(|a| a.is_done(now, sherlock_ui::motion::SCALE_PRECISION))
    }

    /// Settled at rest: nothing lit, nothing grown.
    fn at_rest(&self, now: Duration) -> bool {
        self.is_done(now)
            && self.pill.target() == 0.0
            && self.lit.iter().all(|f| f.target() == 0.0)
            && self.scale.iter().all(|a| a.target() == 1.0)
    }
}

/// What the pointer does on the decorations.
#[derive(Default)]
struct Pointer {
    /// What it is on, and of which window.
    hovered: Option<(Window, Control)>,
    /// The button held down: it acts when released over itself.
    pressed: Option<(Window, Button)>,
    /// The last press on a bar, its time and place: the next one may make a double click.
    last_press: Option<(Window, u32, Point<f64, Logical>)>,
}

/// What the pointer can be on, in a decoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Bar,
    /// The pill, between or around its buttons: it acts like the bar, and shows the icons.
    Pill,
    Button(Button),
}

/// Whether the compositor decorates `window`. A Wayland window has asked for it (see
/// `protocol`). An X11 window has it unless it places itself (override-redirect), is a menu,
/// a tooltip, a notification or a splash screen, or asks for no decoration through
/// `_MOTIF_WM_HINTS` (smithay names that `is_decorated`: decorated by the client).
pub fn server_side(window: &Window) -> bool {
    if let Some(toplevel) = window.toplevel() {
        return match toplevel.with_pending_state(|s| s.decoration_mode) {
            Some(mode) => mode == Mode::ServerSide,
            None => protocol::kde_server_side(toplevel.wl_surface()),
        };
    }
    window.x11_surface().is_some_and(|surface| {
        use WmWindowType::*;
        let kind = surface.window_type();
        let bare = matches!(
            kind,
            Some(DropdownMenu | Menu | PopupMenu | Tooltip | Notification | Splash)
        );
        !surface.is_override_redirect() && !surface.is_decorated() && !bare
    })
}

/// Height the decoration adds above the content of `window`: none without one, nor in
/// fullscreen.
pub fn inset(window: &Window) -> i32 {
    if server_side(window) && !is_fullscreen(window) {
        ui::INSET as i32
    } else {
        0
    }
}

/// The whole window, decoration included, around its content `content`.
pub fn frame(window: &Window, content: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let inset = inset(window);
    let size = Size::from((content.size.w, content.size.h + inset));
    Rectangle::new(content.loc - Point::from((0, inset)), size)
}

/// The content of `window` when the whole window takes `frame`.
pub fn content(window: &Window, frame: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let inset = inset(window).min(frame.size.h);
    let size = Size::from((frame.size.w, frame.size.h - inset));
    Rectangle::new(frame.loc + Point::from((0, inset)), size)
}

/// Place of the content of `window`, of `size`, that centers the whole window in `area`.
pub fn centered(
    window: &Window,
    area: Rectangle<i32, Logical>,
    size: Size<i32, Logical>,
) -> Point<i32, Logical> {
    let inset = inset(window);
    layout::centered(area, Size::from((size.w, size.h + inset))) + Point::from((0, inset))
}

/// The pill of the buttons of a window `width` wide, from its top left corner: right aligned
/// in the bar and centered in its height, under the window border.
pub fn pill(width: f64) -> Rectangle<f64, Logical> {
    let (button_w, button_h) = (f64::from(ui::BUTTON.0), f64::from(ui::BUTTON.1));
    let (gap, padding) = (f64::from(ui::BUTTON_GAP), f64::from(ui::CONTROLS_PADDING));
    let border = f64::from(metrics::BORDER_WIDTH);
    let w = 3.0 * button_w + 2.0 * gap + 2.0 * padding;
    let h = button_h + 2.0 * padding;
    let x = width - border - f64::from(ui::PADDING.1) - w;
    let y = border + (f64::from(ui::BAR_HEIGHT) - h) / 2.0;
    Rectangle::new((x, y).into(), (w, h).into())
}

/// Each button and its rectangle, in a window `width` wide, from its top left corner.
pub fn buttons(width: f64) -> [(Button, Rectangle<f64, Logical>); 3] {
    let pill = pill(width);
    let (button_w, button_h) = (f64::from(ui::BUTTON.0), f64::from(ui::BUTTON.1));
    let (gap, padding) = (f64::from(ui::BUTTON_GAP), f64::from(ui::CONTROLS_PADDING));
    std::array::from_fn(|i| {
        let x = pill.loc.x + padding + i as f64 * (button_w + gap);
        let rect = Rectangle::new(
            (x, pill.loc.y + padding).into(),
            (button_w, button_h).into(),
        );
        (Button::ALL[i], rect)
    })
}

/// What of the decoration of a window `width` wide is at `at`, from its top left corner.
/// `None` below the bar, or beside the window.
pub fn control_at(width: f64, at: Point<f64, Logical>) -> Option<Control> {
    if !(0.0..width).contains(&at.x) || !(0.0..f64::from(ui::INSET)).contains(&at.y) {
        return None;
    }
    let button = buttons(width).into_iter().find(|(_, r)| r.contains(at));
    let pill = || match pill(width).contains(at) {
        true => Control::Pill,
        false => Control::Bar,
    };
    Some(button.map_or_else(pill, |(button, _)| Control::Button(button)))
}

impl State {
    /// What of the decoration of `window` is at `pos`, in the space.
    pub(crate) fn decoration_at(
        &self,
        window: &Window,
        pos: Point<f64, Logical>,
    ) -> Option<Control> {
        if inset(window) == 0 {
            return None;
        }
        let frame = frame(window, self.space.element_geometry(window)?);
        control_at(f64::from(frame.size.w), pos - frame.loc.to_f64())
    }

    /// The pointer is now on `hovered`, `None` off every decoration.
    pub(crate) fn hover_decoration(&mut self, hovered: Option<(Window, Control)>) {
        if self.decorations.pointer.hovered != hovered {
            self.decorations.pointer.hovered = hovered;
            self.aim_buttons();
        }
    }

    /// Gives the buttons of each window concerned their new targets, after a change of the
    /// pointer: the window it is on, and those it has left, which go back to rest.
    fn aim_buttons(&mut self) {
        let now = self.now();
        // The motions over at rest, or of windows gone, are forgotten.
        let motions = &mut self.decorations.motions;
        motions.retain(|(w, m)| w.alive() && !m.at_rest(now));
        let pointer = &self.decorations.pointer;
        let on = pointer.hovered.as_ref().map(|(w, _)| w.clone());
        let mut windows: Vec<Window> = self
            .decorations
            .motions
            .iter()
            .map(|(w, _)| w.clone())
            .collect();
        if let Some(window) = on.filter(|w| !windows.contains(w)) {
            windows.push(window);
        }
        for window in windows {
            let (pill, hovered) = match &pointer.hovered {
                Some((w, Control::Button(button))) if *w == window => (true, Some(*button)),
                Some((w, Control::Pill)) if *w == window => (true, None),
                _ => (false, None),
            };
            let pressed = match &pointer.pressed {
                Some((w, button)) if *w == window => Some(*button),
                _ => None,
            };
            let motions = &mut self.decorations.motions;
            let index = match motions.iter().position(|(w, _)| *w == window) {
                Some(index) => index,
                None => {
                    motions.push((window.clone(), Motion::new(now)));
                    motions.len() - 1
                }
            };
            motions[index].1.aim(now, pill, hovered, pressed);
        }
        self.queue_redraw();
    }

    /// `window` goes away: the motion of its buttons, and what the pointer had of it, go
    /// with it at once.
    pub(crate) fn forget_decoration(&mut self, window: &Window) {
        let decorations = &mut self.decorations;
        decorations.motions.retain(|(w, _)| w != window);
        let pointer = &mut decorations.pointer;
        if pointer.hovered.as_ref().is_some_and(|(w, _)| w == window) {
            pointer.hovered = None;
        }
        if pointer.pressed.as_ref().is_some_and(|(w, _)| w == window) {
            pointer.pressed = None;
        }
        if pointer
            .last_press
            .as_ref()
            .is_some_and(|(w, ..)| w == window)
        {
            pointer.last_press = None;
        }
    }

    /// The buttons of a window are moving at `now`: a frame at each blank.
    pub(crate) fn decorations_animating(&self, now: Duration) -> bool {
        let motions = &self.decorations.motions;
        motions.iter().any(|(_, m)| !m.is_done(now))
    }

    /// A press of `button` on `control`, in the decoration of `window`, at `location` and
    /// `time` (ms). Left button: a press on a button arms it, a drag of the bar moves the
    /// window (and snaps it, like any move), a double click maximizes it or gives it its size
    /// back. The others do nothing yet: the menu of the right click is a follow-up.
    pub(crate) fn press_decoration(
        &mut self,
        window: Window,
        control: Control,
        button: u32,
        (location, time): (Point<f64, Logical>, u32),
    ) {
        if button != BTN_LEFT {
            return;
        }
        let Control::Button(pressed) = control else {
            let delay = ui::DOUBLE_CLICK.as_millis() as u32;
            let near = f64::from(ui_window::DRAG_THRESHOLD);
            let last = self.decorations.pointer.last_press.take();
            let double = last.is_some_and(|(w, t, at)| {
                let delta = location - at;
                w == window && time.wrapping_sub(t) <= delay && delta.x.hypot(delta.y) < near
            });
            if double {
                self.toggle_maximized(&window);
            } else {
                self.decorations.pointer.last_press = Some((window.clone(), time, location));
                self.start_gesture(window, location, button, None, GestureSource::TitleBar);
            }
            return;
        };
        let pointer = &mut self.decorations.pointer;
        pointer.last_press = None;
        pointer.pressed = Some((window, pressed));
        self.aim_buttons();
    }

    /// A button is released at `location`: the decoration button held down acts if the pointer
    /// is still on it.
    pub(crate) fn release_decoration(&mut self, location: Point<f64, Logical>) {
        let Some((window, button)) = self.decorations.pointer.pressed.take() else {
            return;
        };
        self.aim_buttons();
        if self.decoration_at(&window, location) != Some(Control::Button(button)) {
            return;
        }
        tracing::debug!(?button, "decoration button");
        match button {
            Button::Minimize => self.minimize(&window),
            Button::Maximize => self.toggle_maximized(&window),
            Button::Close => State::close(&window),
        }
    }

    /// Maximizes `window`, or gives it back its previous place.
    fn toggle_maximized(&mut self, window: &Window) {
        let zone = (!is_maximized(window)).then_some(Zone::Maximized);
        self.snap(window, zone, None);
    }

    /// The bar to draw above `window`, `None` if the compositor does not decorate it.
    pub(crate) fn decoration_bar(&self, window: &Window) -> Option<Bar> {
        self.decoration_bar_at(window, self.now())
    }

    /// The same, its buttons as they are at `now`.
    pub(crate) fn decoration_bar_at(&self, window: &Window, now: Duration) -> Option<Bar> {
        if inset(window) == 0 {
            return None;
        }
        let motion = self.decorations.motions.iter().find(|(w, _)| w == window);
        let (buttons, pill) = motion.map_or_else(Default::default, |(_, m)| m.look(now));
        Some(Bar {
            title: State::title(window).unwrap_or_default(),
            buttons,
            pill,
            maximized: is_maximized(window),
        })
    }

    /// A window asked for a decoration or gave it up: maximized or snapped, its content takes
    /// the place left by the bar, or the bar's.
    fn decoration_changed(&mut self, window: Option<Window>) {
        self.queue_redraw();
        if let Some(window) = window
            && (is_maximized(&window) || crate::window_data(&window).zone.get().is_some())
        {
            self.rearrange(&window, |_, _| {});
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buttons_sit_at_the_right_of_the_bar() {
        // The code window of the Main board: 836 wide.
        let pill = pill(836.0);
        assert_eq!(
            pill,
            Rectangle::new((723.0, 5.0).into(), (104.0, 32.0).into())
        );
        let [minimize, maximize, close] = buttons(836.0);
        assert_eq!(minimize.1.loc, (725.0, 7.0).into());
        assert_eq!(maximize.1.loc, (759.0, 7.0).into());
        assert_eq!(close.1.loc, (793.0, 7.0).into());
        // Targets of 32 × 28 at least.
        assert_eq!(close.1.size, (32.0, 28.0).into());
        // The pill ends 9 before the right edge: the border, then the padding of 8.
        assert_eq!(pill.loc.x + pill.size.w, 836.0 - 9.0);
    }

    #[test]
    fn the_pointer_finds_the_bar_the_pill_and_its_buttons() {
        let at = |x: f64, y: f64| control_at(836.0, (x, y).into());
        assert_eq!(at(10.0, 20.0), Some(Control::Bar));
        assert_eq!(at(726.0, 8.0), Some(Control::Button(Button::Minimize)));
        assert_eq!(at(780.0, 34.0), Some(Control::Button(Button::Maximize)));
        assert_eq!(at(824.0, 20.0), Some(Control::Button(Button::Close)));
        // Between two buttons, around them: the pill. Above and below it, the bar.
        assert_eq!(at(758.0, 20.0), Some(Control::Pill));
        assert_eq!(at(800.0, 6.0), Some(Control::Pill));
        assert_eq!(at(800.0, 4.0), Some(Control::Bar));
        assert_eq!(at(800.0, 41.5), Some(Control::Bar));
        // Below the bar, the content; beside the window, nothing.
        assert_eq!(at(10.0, 42.0), None);
        assert_eq!(at(836.0, 20.0), None);
        assert_eq!(at(-1.0, 20.0), None);
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_hovered_button_lights_up_and_grows_on_a_spring_then_sinks_when_pressed() {
        let mut motion = Motion::new(ms(0));
        assert!(motion.at_rest(ms(0)));
        motion.aim(ms(0), true, Some(Button::Close), None);
        let (half, pill) = motion.look(ms(60));
        assert!(pill > 0 && pill < 255, "the icons show with a fade");
        assert!(half[2].lit > 0 && half[2].lit < 255);
        assert_eq!(half[0].lit, 0, "the other buttons stay quiet");
        // The spring passes its target a little, then settles on it.
        let peak = (0..400)
            .map(|t| motion.look(ms(t)).0[2].scale)
            .max()
            .unwrap();
        let hover = (ui::HOVER_SCALE * 1000.0).round() as u16;
        assert!(peak > hover && peak < hover + 10, "a slight bounce: {peak}");
        let (settled, pill) = motion.look(ms(600));
        assert_eq!((settled[2].lit, settled[2].scale, pill), (255, hover, 255));
        assert!(motion.is_done(ms(600)) && !motion.at_rest(ms(600)));
        // Pressed: it sinks.
        motion.aim(ms(600), true, Some(Button::Close), Some(Button::Close));
        let press = (ui::PRESS_SCALE * 1000.0).round() as u16;
        assert_eq!(motion.look(ms(1200)).0[2].scale, press);
        // Left: back to rest.
        motion.aim(ms(1200), false, None, None);
        assert!(!motion.is_done(ms(1250)));
        assert!(motion.at_rest(ms(2000)));
        assert_eq!(motion.look(ms(2000)), ([ButtonLook::default(); 3], 0));
    }
}
