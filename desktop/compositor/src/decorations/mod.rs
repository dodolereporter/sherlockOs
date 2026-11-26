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

pub use raster::{Bar, Look, Textures, draw};

use sherlock_ui::{
    color,
    decorations::{self as ui, Button},
    metrics, window as ui_window,
};
use smithay::{
    desktop::Window,
    reexports::{
        calloop::{EventLoop, channel},
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
        wayland_server::DisplayHandle,
    },
    utils::{Logical, Point, Rectangle, Size},
    wayland::shell::{kde::decoration::KdeDecorationState, xdg::decoration::XdgDecorationState},
    xwayland::xwm::WmWindowType,
};

use crate::{
    State,
    handlers::{is_fullscreen, is_maximized},
    input::{BTN_LEFT, GestureSource},
    layout::{self, Zone},
};

/// The decoration protocols, and the pointer on the decorations.
pub struct Decorations {
    _xdg: XdgDecorationState,
    kde: KdeDecorationState,
    pointer: Pointer,
}

impl Decorations {
    pub fn new(dh: &DisplayHandle) -> Self {
        let (xdg, kde) = protocol::globals(dh);
        Self {
            _xdg: xdg,
            kde,
            pointer: Pointer::default(),
        }
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
    Some(button.map_or(Control::Bar, |(button, _)| Control::Button(button)))
}

/// Makes the theme of the appearance file active, then follows it: the decorations take the
/// colors of the active palette (`color::palette`). Without the file, the dark theme.
pub fn follow_theme(event_loop: &EventLoop<'static, State>) {
    let Some(path) = appearance_config::path() else {
        return;
    };
    color::set_active(appearance_config::theme_now(&path));
    let (sender, themes) = channel::channel();
    let source = event_loop
        .handle()
        .insert_source(themes, |event, _, state| {
            if let channel::Event::Msg(theme) = event {
                color::set_active(theme);
                state.queue_redraw();
            }
        });
    if let Err(err) = source {
        tracing::warn!("following the theme: {}", err.error);
        return;
    }
    let follow = appearance_config::follow_themes(path, move |theme| sender.send(theme).is_ok());
    if let Err(err) = follow {
        tracing::warn!("following the theme: {err}");
    }
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
        self.decorations.pointer.hovered = hovered;
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
    }

    /// A button is released at `location`: the decoration button held down acts if the pointer
    /// is still on it.
    pub(crate) fn release_decoration(&mut self, location: Point<f64, Logical>) {
        let Some((window, button)) = self.decorations.pointer.pressed.take() else {
            return;
        };
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
        if inset(window) == 0 {
            return None;
        }
        let pointer = &self.decorations.pointer;
        let hovered = match &pointer.hovered {
            Some((w, Control::Button(button))) if w == window => Some(*button),
            _ => None,
        };
        let pressed = match &pointer.pressed {
            Some((w, button)) if w == window && hovered == Some(*button) => Some(*button),
            _ => None,
        };
        Some(Bar {
            title: State::title(window).unwrap_or_default(),
            hovered,
            pressed,
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
    fn the_buttons_sit_like_on_the_main_board() {
        // The code window of the Main board: 836 wide.
        let pill = pill(836.0);
        assert_eq!(
            pill,
            Rectangle::new((735.0, 7.0).into(), (92.0, 28.0).into())
        );
        let [minimize, maximize, close] = buttons(836.0);
        assert_eq!(minimize.1.loc, (737.0, 9.0).into());
        assert_eq!(maximize.1.loc, (767.0, 9.0).into());
        assert_eq!(close.1.loc, (797.0, 9.0).into());
        assert_eq!(close.1.size, (28.0, 24.0).into());
        // The pill ends 9 before the right edge: the border, then the padding of 8.
        assert_eq!(pill.loc.x + pill.size.w, 836.0 - 9.0);
    }

    #[test]
    fn the_pointer_finds_the_bar_and_its_buttons() {
        let at = |x: f64, y: f64| control_at(836.0, (x, y).into());
        assert_eq!(at(10.0, 20.0), Some(Control::Bar));
        assert_eq!(at(740.0, 10.0), Some(Control::Button(Button::Minimize)));
        assert_eq!(at(780.0, 32.0), Some(Control::Button(Button::Maximize)));
        assert_eq!(at(824.0, 20.0), Some(Control::Button(Button::Close)));
        // Between two buttons, above them, below the pill: the bar.
        assert_eq!(at(766.0, 20.0), Some(Control::Bar));
        assert_eq!(at(800.0, 8.0), Some(Control::Bar));
        assert_eq!(at(800.0, 41.5), Some(Control::Bar));
        // Below the bar, the content; beside the window, nothing.
        assert_eq!(at(10.0, 42.0), None);
        assert_eq!(at(836.0, 20.0), None);
        assert_eq!(at(-1.0, 20.0), None);
    }
}
