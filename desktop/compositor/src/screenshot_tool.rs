//! The screenshot tool (lot D-35e): Super + Shift + S (Alt + Shift + S nested), or Shift +
//! Print Screen, opens it on the screen under the pointer. A toolbar of glass at the bottom
//! chooses an area, a window or the whole screen, a timer and options; the area is drawn, then
//! adjusted by its handles or moved, and taken with Enter or the « Capturer » button; a window
//! is taken by a click on it. Escape or the right button gives up. The geometry and the look
//! are in `sherlock_ui::screenshot_tool`; the drawing offscreen, the worker and the clipboard in
//! `capture`. No client gets the pointer or the keyboard while it is open. See
//! `docs/screenshots.md`.

use std::time::Duration;

use sherlock_ui::{
    color::{self, Color},
    motion::{Animation, Fade, GEOMETRY_PRECISION},
    screenshot_tool::{self as ui, Bar, Control, Entry, Grip, Handle, Menu, Mode, Options, Rect},
};
use smithay::{
    desktop::{Window, layer_map_for_output},
    input::{keyboard::Keysym, pointer::CursorIcon},
    output::Output,
    utils::{Logical, Physical, Point, Rectangle},
};

use crate::{
    State,
    capture::Target,
    damage::{Key, Part},
    input::Hit,
    mission_control::{Label, label_width},
    output::View,
    render::Item,
};

/// What a drag of the left button does.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    /// A new area, from its first corner.
    Draw { anchor: (f64, f64) },
    Resize {
        handle: Handle,
        from: (f64, f64),
        start: Rect,
    },
    /// The area moves with the pointer; `resume`: Space held while drawing, which draws again
    /// once it is let go.
    Move {
        from: (f64, f64),
        start: Rect,
        resume: bool,
    },
    /// A button pressed, taken on its release over it.
    Button(Control),
}

/// What the pointer is over, and since when: its background comes in the time of a hover.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Over {
    Control(Control),
    Entry(Entry),
}

/// The tool on its screen, coming, open, or going.
pub(crate) struct Tool {
    output: Output,
    /// 0 out of sight, 1 in place.
    presence: Fade,
    leaving: bool,
    /// Not drawn: the image is being taken.
    hidden: bool,
    /// In the space, as last let go or as dragged.
    area: Option<Rect>,
    /// The area going back inside the screen: x, y, width, height.
    settle: Option<[Animation; 4]>,
    drag: Option<Drag>,
    pointer: (f64, f64),
    over: Option<(Over, Duration)>,
    menu: bool,
    /// The window under the pointer, in window mode.
    window: Option<Window>,
    /// The screen as it was when the tool opened, for the loupe.
    frozen: Option<std::sync::Arc<crate::capture::Image>>,
}

/// An image to take once the timer is done.
pub(crate) struct Pending {
    pub(crate) target: Target,
    pub(crate) crop: Option<Rectangle<i32, Physical>>,
    pub(crate) output: Output,
    pub(crate) at: Duration,
}

fn rect(r: Rectangle<f64, Logical>) -> Rect {
    Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

fn logical(r: &Rect) -> Rectangle<f64, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

/// Width of a text of the toolbar at scale 1, without its padding.
fn measure(text: &str) -> f64 {
    f64::from(label_width(text) - 2.0 * sherlock_ui::mission_control::LABEL_PADDING)
}

impl Tool {
    fn bounds(&self, state: &State) -> Option<Rect> {
        state
            .space
            .output_geometry(&self.output)
            .map(|g| rect(g.to_f64()))
    }

    /// The area as shown at `now`: stretched past the edges while dragged, or going back.
    fn shown_area(&self, bounds: &Rect, now: Duration) -> Option<Rect> {
        if let Some(s) = &self.settle
            && !s.iter().all(|a| a.is_done(now, GEOMETRY_PRECISION))
        {
            let v: Vec<f64> = s.iter().map(|a| a.value(now)).collect();
            return Some(Rect::new(v[0], v[1], v[2].max(0.0), v[3].max(0.0)));
        }
        let area = self.area?;
        Some(match self.drag {
            Some(Drag::Move { .. }) => ui::elastic(&area, bounds, true),
            Some(Drag::Draw { .. } | Drag::Resize { .. }) => ui::elastic(&area, bounds, false),
            _ => area,
        })
    }

    fn bar(&self, bounds: &Rect) -> Bar {
        let zone = layer_map_for_output(&self.output).non_exclusive_zone();
        let reserved = (bounds.h as i32 - zone.loc.y - zone.size.h).max(0);
        Bar::layout(bounds, f64::from(reserved), measure)
    }

    fn menu(&self, bar: &Bar, bounds: &Rect) -> Option<Menu> {
        let button = bar.rect_of(Control::Options)?;
        self.menu
            .then(|| Menu::layout(&button, bar.fit, bounds, measure))
    }

    /// Whether `control` can be pressed now.
    fn enabled(&self, control: Control, options: &Options) -> bool {
        match (control, options.mode) {
            (Control::Capture, Mode::Area) => self.area.is_some(),
            (Control::Capture, Mode::Window) => self.window.is_some(),
            (control, _) => control.is_button(),
        }
    }
}

impl State {
    /// The tool is open and takes the input.
    pub fn screenshot_tool_open(&self) -> bool {
        self.capture
            .tool
            .as_ref()
            .is_some_and(|t| !t.leaving && !t.hidden)
    }

    /// Super + Shift + S: opens the tool on the screen under the pointer, or closes it.
    pub fn open_screenshot_tool(&mut self) {
        if self.screenshot_tool_open() {
            self.close_screenshot_tool();
            return;
        }
        let (Some(output), Some(pointer)) = (self.pointer_output(), self.seat.get_pointer()) else {
            return;
        };
        let p = pointer.current_location();
        let now = self.now();
        self.capture.tool = Some(Tool {
            output,
            presence: Fade::eased(0.0, 1.0, now, ui::ENTER, ui::ENTER_EASE),
            leaving: false,
            hidden: false,
            area: None,
            settle: None,
            drag: None,
            pointer: (p.x, p.y),
            over: None,
            menu: false,
            window: None,
            frozen: None,
        });
        self.tool_motion(p);
        // The screen without the tool, for the loupe, once the key's event is handled.
        self.loop_handle.insert_idle(|state| {
            let Some(output) = state.capture.tool.as_ref().map(|t| t.output.clone()) else {
                return;
            };
            if let Some(tool) = &mut state.capture.tool {
                tool.hidden = true;
            }
            let image = state.draw_target(&Target::Screen(output));
            if let Some(tool) = &mut state.capture.tool {
                tool.hidden = false;
                tool.frozen = image.map(std::sync::Arc::new);
            }
            state.queue_redraw();
        });
        self.queue_redraw();
    }

    /// Escape, the right button, « Annuler »: the tool goes, nothing taken.
    pub fn close_screenshot_tool(&mut self) {
        let now = self.now();
        if let Some(tool) = &mut self.capture.tool
            && !tool.leaving
        {
            let from = tool.presence.value(now);
            tool.presence = Fade::eased(from, 0.0, now, ui::LEAVE.mul_f32(from), ui::LEAVE_EASE);
            tool.leaving = true;
            tool.drag = None;
            tool.menu = false;
        }
        self.queue_redraw();
    }

    /// The choices of the toolbar.
    pub fn screenshot_options(&self) -> Options {
        self.capture.options
    }

    /// The area drawn, in the space, once let go.
    pub fn screenshot_area(&self) -> Option<Rectangle<f64, Logical>> {
        let tool = self.capture.tool.as_ref()?;
        tool.area.map(|a| logical(&a))
    }

    /// Where the toolbar's `control` is, in the space, while the tool is open.
    pub fn screenshot_control(&self, control: Control) -> Option<Rectangle<f64, Logical>> {
        let tool = self.capture.tool.as_ref()?;
        let bounds = tool.bounds(self)?;
        tool.bar(&bounds).rect_of(control).map(|r| logical(&r))
    }

    /// Where the entry of the options menu is, while it is open.
    pub fn screenshot_entry(&self, entry: Entry) -> Option<Rectangle<f64, Logical>> {
        let tool = self.capture.tool.as_ref()?;
        let bounds = tool.bounds(self)?;
        let menu = tool.menu(&tool.bar(&bounds), &bounds)?;
        let found = menu.entries.iter().find(|(e, _)| *e == entry);
        found.map(|(_, r)| logical(r))
    }

    /// A key while the tool is open; every key stays with it.
    pub(crate) fn tool_key(&mut self, sym: Keysym, pressed: bool) {
        let Some(tool) = &mut self.capture.tool else {
            return;
        };
        match (sym, pressed) {
            (Keysym::Escape, true) if tool.menu => tool.menu = false,
            (Keysym::Escape, true) => self.close_screenshot_tool(),
            (Keysym::Return | Keysym::KP_Enter, true) => self.confirm_screenshot(),
            (Keysym::Tab, true) => {
                let next = match self.capture.options.mode {
                    Mode::Area => Mode::Window,
                    Mode::Window => Mode::Screen,
                    Mode::Screen => Mode::Area,
                };
                self.capture.options.mode = next;
            }
            (Keysym::space, true) => {
                // Space held while drawing moves the area being drawn.
                let p = tool.pointer;
                if let (Some(Drag::Draw { .. } | Drag::Resize { .. }), Some(area)) =
                    (tool.drag, tool.area)
                {
                    tool.drag = Some(Drag::Move {
                        from: p,
                        start: area,
                        resume: true,
                    });
                }
            }
            (Keysym::space, false) => {
                if let (Some(Drag::Move { resume: true, .. }), Some(a)) = (tool.drag, tool.area) {
                    let (px, py) = tool.pointer;
                    let x = if px >= a.x + a.w / 2.0 {
                        a.x
                    } else {
                        a.right()
                    };
                    let y = if py >= a.y + a.h / 2.0 {
                        a.y
                    } else {
                        a.bottom()
                    };
                    tool.drag = Some(Drag::Draw { anchor: (x, y) });
                }
            }
            _ => {}
        }
        self.queue_redraw();
    }

    pub(crate) fn tool_motion(&mut self, location: Point<f64, Logical>) {
        let now = self.now();
        let window = match self.hit(location) {
            Some(Hit::Surface(w, ..) | Hit::Decoration(w, _) | Hit::Border(w, _)) => Some(w),
            _ => None,
        };
        let options = self.capture.options;
        let Some(tool) = &mut self.capture.tool else {
            return;
        };
        let p = (location.x, location.y);
        tool.pointer = p;
        match tool.drag {
            Some(Drag::Draw { anchor }) => tool.area = Some(Rect::between(anchor, p)),
            Some(Drag::Resize {
                handle,
                from,
                start,
            }) => tool.area = Some(ui::resized(&start, handle, (p.0 - from.0, p.1 - from.1))),
            Some(Drag::Move { from, start, .. }) => {
                tool.area = Some(start.moved((p.0 - from.0, p.1 - from.1)));
            }
            Some(Drag::Button(_)) | None => {}
        }
        tool.window = (options.mode == Mode::Window).then_some(window).flatten();
        let over = self.tool_over(location);
        if let Some(tool) = &mut self.capture.tool
            && tool.over.map(|(o, _)| o) != over
        {
            tool.over = over.map(|o| (o, now));
        }
    }

    /// The control or the entry under `location`.
    fn tool_over(&self, location: Point<f64, Logical>) -> Option<Over> {
        let tool = self.capture.tool.as_ref()?;
        let bounds = tool.bounds(self)?;
        let p = (location.x, location.y);
        let bar = tool.bar(&bounds);
        if let Some(menu) = tool.menu(&bar, &bounds)
            && let Some(entry) = menu.entry_at(p, &self.capture.options)
        {
            return Some(Over::Entry(entry));
        }
        bar.button_at(p).map(Over::Control)
    }

    /// The pointer is over the toolbar or its menu.
    fn over_panels(&self, location: Point<f64, Logical>) -> bool {
        let Some(tool) = &self.capture.tool else {
            return false;
        };
        let Some(bounds) = tool.bounds(self) else {
            return false;
        };
        let p = (location.x, location.y);
        let bar = tool.bar(&bounds);
        bar.rect.contains(p) || tool.menu(&bar, &bounds).is_some_and(|m| m.rect.contains(p))
    }

    pub(crate) fn tool_button(&mut self, left: bool, pressed: bool, location: Point<f64, Logical>) {
        self.tool_motion(location);
        let now = self.now();
        let bounds = self.capture.tool.as_ref().and_then(|t| t.bounds(self));
        if !left {
            if pressed {
                self.close_screenshot_tool();
            }
            return;
        }
        let over = self.tool_over(location);
        let on_panels = self.over_panels(location);
        let options = self.capture.options;
        let Some(tool) = &mut self.capture.tool else {
            return;
        };
        let p = (location.x, location.y);
        if pressed {
            tool.settle = None;
            match over {
                Some(Over::Entry(entry)) => {
                    entry.choose(&mut self.capture.options);
                }
                Some(Over::Control(control)) => {
                    if tool.enabled(control, &options) {
                        tool.drag = Some(Drag::Button(control));
                    }
                }
                None if on_panels => {}
                None => {
                    tool.menu = false;
                    match options.mode {
                        Mode::Area => {
                            tool.drag = Some(match tool.area.map(|a| (a, ui::grip(&a, p))) {
                                Some((a, Grip::Handle(handle))) => Drag::Resize {
                                    handle,
                                    from: p,
                                    start: a,
                                },
                                Some((a, Grip::Inside)) => Drag::Move {
                                    from: p,
                                    start: a,
                                    resume: false,
                                },
                                _ => {
                                    tool.area = None;
                                    Drag::Draw { anchor: p }
                                }
                            });
                        }
                        // A click on a window or a screen takes it at once.
                        Mode::Window | Mode::Screen => self.confirm_screenshot(),
                    }
                }
            }
            self.queue_redraw();
            return;
        }
        match tool.drag.take() {
            Some(Drag::Button(control)) => {
                if over == Some(Over::Control(control)) {
                    self.press_control(control);
                }
            }
            Some(drag) => {
                let moving = matches!(drag, Drag::Move { .. });
                if let (Some(area), Some(bounds)) = (tool.area, bounds) {
                    let shown = match moving {
                        true => ui::elastic(&area, &bounds, true),
                        false => ui::elastic(&area, &bounds, false),
                    };
                    let settled = ui::settled(&area, &bounds, moving);
                    if settled.w < ui::MIN_AREA || settled.h < ui::MIN_AREA {
                        tool.area = None;
                    } else {
                        tool.area = Some(settled);
                        if shown != settled {
                            let spring = ui::SETTLE_SPRING;
                            let a = |from: f64, to: f64| Animation::new(spring, from, to, now);
                            tool.settle = Some([
                                a(shown.x, settled.x),
                                a(shown.y, settled.y),
                                a(shown.w, settled.w),
                                a(shown.h, settled.h),
                            ]);
                        }
                    }
                }
            }
            None => {}
        }
        self.queue_redraw();
    }

    fn press_control(&mut self, control: Control) {
        let options = &mut self.capture.options;
        match control {
            Control::Mode(mode) => {
                options.mode = mode;
                if let Some(tool) = &mut self.capture.tool
                    && mode == Mode::Screen
                {
                    tool.area = None;
                }
            }
            Control::Delay(delay) => options.delay = delay,
            Control::Options => {
                if let Some(tool) = &mut self.capture.tool {
                    tool.menu = !tool.menu;
                }
            }
            Control::Cancel => self.close_screenshot_tool(),
            Control::Capture => self.confirm_screenshot(),
            Control::Timer => {}
        }
    }

    /// Enter, « Capturer », or a click in window or screen mode: the image is taken, now or
    /// once the timer is done.
    fn confirm_screenshot(&mut self) {
        let options = self.capture.options;
        let Some(tool) = &self.capture.tool else {
            return;
        };
        let output = tool.output.clone();
        let (target, crop) = match options.mode {
            Mode::Screen => {
                let output = self.pointer_output().unwrap_or(output.clone());
                (Target::Screen(output), None)
            }
            Mode::Window => match &tool.window {
                Some(window) => (Target::Window(window.clone()), None),
                None => return,
            },
            Mode::Area => {
                let (Some(area), Some(screen)) = (tool.area, self.space.output_geometry(&output))
                else {
                    return;
                };
                let scale = output.current_scale().fractional_scale();
                let local = Rectangle::new(
                    (
                        area.x - f64::from(screen.loc.x),
                        area.y - f64::from(screen.loc.y),
                    )
                        .into(),
                    (area.w, area.h).into(),
                );
                let pixels = local.to_physical(scale).to_i32_round();
                if pixels.size.w < 1 || pixels.size.h < 1 {
                    return;
                }
                (Target::Screen(output.clone()), Some(pixels))
            }
        };
        let delay = options.delay.duration();
        if delay.is_zero() {
            // Not drawn in its own image; then gone at once, as the image is. Taken once the
            // event is handled: a nested backend lends its renderer only between events.
            if let Some(tool) = &mut self.capture.tool {
                tool.hidden = true;
            }
            self.loop_handle.insert_idle(move |state| {
                let shadow = matches!(target, Target::Window(_));
                state.take_with(&target, crop, &options, shadow);
                state.capture.tool = None;
                state.queue_redraw();
            });
        } else {
            self.close_screenshot_tool();
            let at = self.now() + delay;
            self.capture.pending = Some(Pending {
                target,
                crop,
                output,
                at,
            });
            self.tick_countdown();
        }
        self.queue_redraw();
    }

    /// The timer: a frame each second for the countdown, then the image.
    fn tick_countdown(&mut self) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        let Some(pending) = &self.capture.pending else {
            return;
        };
        let left = pending.at.saturating_sub(self.now());
        if left.is_zero() {
            if let Some(pending) = self.capture.pending.take() {
                let options = self.capture.options;
                let shadow = matches!(pending.target, Target::Window(_));
                self.take_with(&pending.target, pending.crop, &options, shadow);
            }
            self.queue_redraw();
            return;
        }
        let step = Duration::from_nanos((left.as_nanos() % 1_000_000_000) as u64);
        let step = if step.is_zero() {
            Duration::from_secs(1)
        } else {
            step
        };
        let timer = Timer::from_duration(step);
        let inserted = self.loop_handle.insert_source(timer, |_, _, state| {
            state.tick_countdown();
            state.queue_redraw();
            TimeoutAction::Drop
        });
        if let Err(err) = inserted {
            tracing::warn!("screenshot timer: {err}");
        }
    }

    /// The seconds left on the timer, while one runs.
    pub fn screenshot_countdown(&self) -> Option<u64> {
        let pending = self.capture.pending.as_ref()?;
        let left = pending.at.saturating_sub(self.now());
        Some(left.as_secs() + u64::from(left.subsec_nanos() > 0))
    }

    /// The cursor while the tool is open: the arrow over the toolbar, a cross to draw, the
    /// arrows of an edge over a handle, the four arrows inside the area, the hand over a window
    /// or a screen to take.
    pub(crate) fn tool_cursor(&self) -> Option<CursorIcon> {
        let tool = self.capture.tool.as_ref()?;
        let location = self.seat.get_pointer()?.current_location();
        if self.over_panels(location) {
            return Some(CursorIcon::Default);
        }
        let p = (location.x, location.y);
        Some(match (self.capture.options.mode, tool.drag) {
            (_, Some(Drag::Move { .. })) => CursorIcon::Move,
            (_, Some(Drag::Draw { .. })) => CursorIcon::Crosshair,
            (_, Some(Drag::Resize { handle, .. })) => handle_cursor(handle),
            (Mode::Area, _) => match tool.area.map(|a| ui::grip(&a, p)) {
                Some(Grip::Handle(handle)) => handle_cursor(handle),
                Some(Grip::Inside) => CursorIcon::Move,
                _ => CursorIcon::Crosshair,
            },
            (Mode::Window | Mode::Screen, _) => CursorIcon::Pointer,
        })
    }
}

fn handle_cursor(handle: Handle) -> CursorIcon {
    match handle {
        Handle::TopLeft | Handle::BottomRight => CursorIcon::NwseResize,
        Handle::TopRight | Handle::BottomLeft => CursorIcon::NeswResize,
        Handle::Top | Handle::Bottom => CursorIcon::NsResize,
        Handle::Left | Handle::Right => CursorIcon::EwResize,
    }
}

/// Builds the items of the tool, front to back.
struct Painter<'a> {
    items: &'a mut Vec<Item>,
    view: &'a View,
    alpha: f32,
    labels: usize,
    shapes: usize,
}

impl Painter<'_> {
    fn shape(&mut self, r: &Rect, fill: Color, border: (Color, f32), radius: f32) {
        let scale = self.view.scale as f32;
        self.items.push(Item::Shape {
            key: Key::of(None, Part::CaptureShape(self.shapes)),
            rect: self.view.rect(logical(r)),
            fill: fill.premultiplied(),
            border: border.0.premultiplied(),
            border_width: border.1 * scale,
            radius: radius * scale,
            alpha: self.alpha,
        });
        self.shapes += 1;
    }

    fn fill(&mut self, r: &Rect, fill: Color, radius: f32) {
        self.shape(r, fill, (fill, 0.0), radius);
    }

    fn label(&mut self, r: &Rect, text: &str, k: f64, (background, color): (Color, Color)) {
        let rect = self.view.rect(logical(r));
        if rect.size.w < 1 || rect.size.h < 1 {
            return;
        }
        self.items.push(Item::Label {
            key: Key::of(None, Part::CaptureLabel(self.labels)),
            rect,
            label: Label {
                text: text.to_owned(),
                size: (rect.size.w as u32, rect.size.h as u32),
                scale: (self.view.scale * k) as f32,
                background,
                color,
                close: false,
            },
            alpha: self.alpha,
        });
        self.labels += 1;
    }
}

/// How far a hover begun at `since` has come at `now`, from 0 to 1.
fn hover(since: Duration, now: Duration) -> f32 {
    let t = now.saturating_sub(since).as_secs_f32() / ui::HOVER.as_secs_f32();
    sherlock_ui::motion::Ease::OutCubic.apply(t)
}

/// The tool on `output`, in front of everything, and whether it moves.
pub fn push_items(items: &mut Vec<Item>, state: &State, (output, view): (&Output, &View)) -> bool {
    let now = state.now();
    let mut moving = push_countdown(items, state, (output, view));
    let Some(tool) = &state.capture.tool else {
        return moving;
    };
    if &tool.output != output || tool.hidden {
        return moving;
    }
    let Some(bounds) = tool.bounds(state) else {
        return moving;
    };
    let presence = tool.presence.value(now).clamp(0.0, 1.0);
    moving |= !tool.presence.is_done(now);
    if presence <= 0.0 && tool.leaving {
        return moving;
    }
    let options = state.capture.options;
    let colors = ui::Colors::of(color::palette());
    let mut paint = Painter {
        items,
        view,
        alpha: presence,
        labels: 0,
        shapes: 0,
    };
    let bar = tool.bar(&bounds);
    let rise = ui::RISE * f64::from(1.0 - presence);
    let bar_rect = bar.rect.moved((0.0, rise));
    let k = bar.fit;
    let over = tool
        .over
        .filter(|_| tool.drag.is_none() || matches!(tool.drag, Some(Drag::Button(_))));
    let pressed = match tool.drag {
        Some(Drag::Button(c)) => Some(c),
        _ => None,
    };
    // The size of the area, beside the pointer, while it is drawn or pulled.
    let area = tool.shown_area(&bounds, now);
    moving |= tool
        .settle
        .is_some_and(|s| !s.iter().all(|a| a.is_done(now, GEOMETRY_PRECISION)));
    let scale = output.current_scale().fractional_scale();
    let drawing = matches!(tool.drag, Some(Drag::Draw { .. } | Drag::Resize { .. }));
    let aiming =
        tool.drag.is_none() && area.is_none() && !bar.rect.contains(tool.pointer) && !tool.menu;
    // The loupe, beside the pointer, with the size under it.
    let loupe = f64::from(ui::LOUPE_PIXELS) * f64::from(ui::LOUPE_ZOOM);
    let label_h = f64::from(ui::LABEL_HEIGHT);
    let gap = f64::from(ui::LOUPE_BORDER) * 2.0;
    let (lx, ly) = ui::beside(tool.pointer, (loupe, loupe + gap + label_h), &bounds);
    if let (Some(frozen), true) = (
        &tool.frozen,
        options.mode == Mode::Area && (drawing || aiming),
    ) {
        let (px, py) = (
            ((tool.pointer.0 - bounds.x) * scale).floor() as i64,
            ((tool.pointer.1 - bounds.y) * scale).floor() as i64,
        );
        let diameter = (loupe * scale).round() as u32;
        let ring = (colors.text, ui::LOUPE_BORDER * scale as f32);
        let pixels = (ui::LOUPE_PIXELS, diameter);
        if let Some(picture) = crate::picture::loupe(frozen, (px, py), pixels, ring, colors.accent)
        {
            paint.items.push(Item::Picture {
                key: Key::of(None, Part::CaptureLoupe),
                rect: view.rect(logical(&Rect::new(lx, ly, loupe, loupe))),
                picture,
                alpha: paint.alpha,
            });
        }
    }
    if let (Some(a), true) = (area, drawing) {
        let text = ui::size_text((a.w * scale).round() as u32, (a.h * scale).round() as u32);
        let w = f64::from(label_width(&text));
        let h = label_h;
        let (x, y) = match tool.frozen {
            Some(_) => (lx + (loupe - w) / 2.0, ly + loupe + gap),
            None => ui::beside(tool.pointer, (w, h), &bounds),
        };
        let mc = sherlock_ui::mission_control::Colors::of(color::palette());
        paint.label(
            &Rect::new(x, y, w, h),
            &text,
            1.0,
            (mc.label, mc.label_text),
        );
    }
    // The options menu, above its button.
    if let Some(menu) = tool.menu(&bar, &bounds) {
        for (entry, r) in &menu.entries {
            let (enabled, checked) = (entry.enabled(&options), entry.checked(&options));
            let text = match entry {
                Entry::Title(_) => colors.text_secondary,
                _ if enabled => colors.text,
                _ => colors.text_secondary.lerp(colors.disabled, 0.5),
            };
            let room = f64::from(ui::MARK_ROOM) * k;
            let pad = f64::from(sherlock_ui::mission_control::LABEL_PADDING) * k;
            let label = Rect::new(r.x + room - pad, r.y, r.w - room + pad, r.h);
            if !entry.label().is_empty() {
                paint.label(&label, entry.label(), k, (Color::rgba(0, 0, 0, 0.0), text));
            }
            if checked {
                let d = f64::from(ui::MARK_SIZE) * k;
                let mark = Rect::new(r.x + room / 2.0 - d / 2.0, r.y + r.h / 2.0 - d / 2.0, d, d);
                let mark_color = if enabled {
                    colors.accent
                } else {
                    colors.disabled
                };
                paint.fill(&mark, mark_color, ui::MARK_SIZE / 2.0);
            }
            if let Some((Over::Entry(e), since)) = over
                && e == *entry
            {
                let alpha = paint.alpha;
                paint.alpha = alpha * hover(since, now);
                paint.fill(r, colors.hover, ui::MENU_ENTRY_RADIUS * k as f32);
                paint.alpha = alpha;
                moving |= hover(since, now) < 1.0;
            }
        }
        paint.shape(
            &menu.rect,
            colors.glass,
            (colors.border, sherlock_ui::metrics::BORDER_WIDTH),
            ui::MENU_RADIUS * k as f32,
        );
    }
    // The toolbar: texts, then the backgrounds of the buttons, the dividers, its glass.
    let shift = |r: &Rect| r.moved((0.0, rise));
    for (control, r) in &bar.controls {
        let r = shift(r);
        let enabled = tool.enabled(*control, &options);
        let capture = *control == Control::Capture;
        let text = match (control.is_button(), enabled, capture) {
            (false, ..) => colors.text_secondary,
            (true, false, _) => colors.text_secondary.lerp(colors.disabled, 0.5),
            (true, true, true) => colors.text_on_accent,
            (true, true, false) => colors.text,
        };
        paint.label(&r, control.label(), k, (Color::rgba(0, 0, 0, 0.0), text));
        let radius = ui::BUTTON_RADIUS * k as f32;
        let hovered = match over {
            Some((Over::Control(c), since)) if c == *control && enabled => Some(since),
            _ => None,
        };
        let base = match (capture, enabled, control.chosen(&options)) {
            (true, true, _) => Some(colors.accent),
            (true, false, _) => Some(colors.disabled),
            (false, _, true) => Some(colors.chosen),
            _ => None,
        };
        let state_color = match (pressed == Some(*control), capture) {
            (true, true) => Some(colors.accent_pressed),
            (true, false) => Some(colors.pressed),
            (false, true) => hovered.map(|_| colors.accent_hover),
            (false, false) => hovered.map(|_| colors.hover),
        };
        if let Some(color) = state_color {
            let alpha = paint.alpha;
            let t = hovered.map_or(1.0, |since| hover(since, now));
            moving |= t < 1.0;
            paint.alpha = alpha * t;
            paint.fill(&r, color, radius);
            paint.alpha = alpha;
        }
        if let Some(color) = base {
            paint.fill(&r, color, radius);
        }
    }
    for d in &bar.dividers {
        paint.fill(&shift(d), colors.divider, 0.0);
    }
    paint.shape(
        &bar_rect,
        colors.glass,
        (colors.border, sherlock_ui::metrics::BORDER_WIDTH),
        ui::BAR_RADIUS * k as f32,
    );
    // What is taken: the area and its handles over a dimmed screen, the window or the screen
    // under the pointer outlined.
    match options.mode {
        Mode::Area => {
            if let Some(a) = area {
                if tool.drag.is_none() || tool.settle.is_some() {
                    let s = f64::from(ui::HANDLE);
                    for handle in Handle::ALL {
                        let (x, y) = handle.center(&a);
                        let r = Rect::new(x - s / 2.0, y - s / 2.0, s, s);
                        paint.shape(&r, colors.text, (colors.accent, ui::AREA_BORDER), 2.0);
                    }
                }
                paint.shape(&a, colors.area_fill, (colors.accent, ui::AREA_BORDER), 0.0);
            }
            // The dim around the area: above, below, left, right.
            let a = area
                .map(|a| a.inside(&bounds))
                .filter(|a| a.w > 0.0 && a.h > 0.0);
            let pieces = match a {
                Some(a) => vec![
                    Rect::new(bounds.x, bounds.y, bounds.w, a.y - bounds.y),
                    Rect::new(bounds.x, a.bottom(), bounds.w, bounds.bottom() - a.bottom()),
                    Rect::new(bounds.x, a.y, a.x - bounds.x, a.h),
                    Rect::new(a.right(), a.y, bounds.right() - a.right(), a.h),
                ],
                None => vec![bounds],
            };
            // The dim is the last thing drawn: its opacity stays.
            paint.alpha *= ui::DIM;
            for piece in pieces.iter().filter(|p| p.w > 0.0 && p.h > 0.0) {
                paint.fill(piece, colors.dim, 0.0);
            }
        }
        Mode::Window => {
            if let Some(window) = &tool.window
                && let Some(frame) = crate::window_anim::whole_geometry(state, window)
            {
                let width = f64::from(ui::HIGHLIGHT_WIDTH);
                let r = rect(frame.to_f64());
                let outer = Rect::new(
                    r.x - width,
                    r.y - width,
                    r.w + 2.0 * width,
                    r.h + 2.0 * width,
                );
                let radius = sherlock_ui::metrics::WINDOW_RADIUS + ui::HIGHLIGHT_WIDTH;
                let tint = colors.chosen;
                paint.shape(&outer, tint, (colors.accent, ui::HIGHLIGHT_WIDTH), radius);
            }
        }
        Mode::Screen => {
            let tint = colors.chosen;
            paint.shape(&bounds, tint, (colors.accent, ui::HIGHLIGHT_WIDTH), 0.0);
        }
    }
    moving
}

/// The seconds left on the timer, big, in the middle of the screen of the image.
fn push_countdown(items: &mut Vec<Item>, state: &State, (output, view): (&Output, &View)) -> bool {
    let (Some(pending), Some(left)) = (&state.capture.pending, state.screenshot_countdown()) else {
        return false;
    };
    if &pending.output != output {
        return false;
    }
    let Some(screen) = state.space.output_geometry(output) else {
        return false;
    };
    let bounds = rect(screen.to_f64());
    let size = f64::from(ui::COUNTDOWN_SIZE);
    let r = Rect::new(
        bounds.x + (bounds.w - size) / 2.0,
        bounds.y + (bounds.h - size) / 2.0,
        size,
        size,
    );
    let mc = sherlock_ui::mission_control::Colors::of(color::palette());
    let k = f64::from(ui::COUNTDOWN_SIZE / 2.0 / sherlock_ui::decorations::TITLE_SIZE);
    let mut paint = Painter {
        items,
        view,
        alpha: 1.0,
        labels: 100,
        shapes: 100,
    };
    let text = left.to_string();
    let w = f64::from(label_width(&text)) * k;
    let label = Rect::new(r.x + (r.w - w) / 2.0, r.y, w, r.h);
    paint.label(&label, &text, k, (Color::rgba(0, 0, 0, 0.0), mc.label_text));
    paint.fill(&r, mc.label, ui::COUNTDOWN_SIZE / 2.0);
    true
}
