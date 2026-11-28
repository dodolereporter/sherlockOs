//! The « Écrans » page (ADR 0025, lot D-26b): the arrangement of the screens, to drag, then
//! the screen chosen in it: resolution, refresh, scale, rotation, main screen, on or off.
//! Every change goes to the compositor as a whole configuration (`displays.rs`); a change that
//! can leave a screen black waits `CONFIRM_DELAY` for « Garder » and is undone without it.
//! The compositor saves each screen's setting under its EDID names.

use std::cell::Cell;
use std::sync::mpsc;
use std::time::Instant;

use iced::widget::{Column, Row, column, container, mouse_area, pin, row, space, stack};
use iced::{Alignment, Element, Length, Point, mouse};
use sherlock_ui::component as ui;
use sherlock_ui::control_room::{self as cr, texts as t};
use sherlock_ui::{color, metrics, theme, typography};

use crate::app::{App, Message};
use crate::arrangement::{Fit, Rect, snap};
use crate::displays::{Answer, Event, Lit, Mode, Request, Screen};
use crate::search::Setting;

/// What the page knows of the screens.
#[derive(Debug, Default)]
pub struct Displays {
    pub(crate) state: State,
    pub(crate) screens: Vec<Screen>,
    /// The screen whose settings are shown, by its connector name.
    pub(crate) selected: Option<String>,
    sender: Option<mpsc::Sender<Request>>,
    /// The screen held by the pointer: its name, where it was grabbed in it, and where the
    /// pointer is, in the space.
    drag: Option<Drag>,
    /// How the space was last drawn: where the pointer is in it.
    fit: Cell<Option<Fit>>,
    /// The configuration sent, the one to go back to, and whether it waits for « Garder ».
    waiting: Option<(Request, bool)>,
    /// The configuration to go back to, and when.
    pub(crate) confirm: Option<(Request, Instant)>,
    /// The compositor refused the last change.
    pub(crate) refused: bool,
}

/// A screen held: its name, where it was grabbed in it, where the pointer is.
type Drag = (String, (f64, f64), (f64, f64));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum State {
    #[default]
    Loading,
    Ready,
    Unavailable,
}

#[derive(Debug, Clone)]
pub enum Change {
    Event(Event),
    Select(String),
    Grab(String),
    Drag(Point),
    Drop,
    Resolution(i32, i32),
    Refresh(i32),
    Scale(f64),
    Rotation(u32),
    Main,
    On(bool),
    Keep,
    Revert,
}

/// The labels of `cr::SCALES`.
const SCALE_LABELS: [&str; 5] = t::SCALE_LABELS;

/// A resolution in its menu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size(i32, i32, bool);

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&t::resolution(self.0, self.1, self.2))
    }
}

/// A refresh in its menu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hertz(i32);

impl std::fmt::Display for Hertz {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&t::hertz(self.0))
    }
}

/// A rotation in its menu, a quarter turn each.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Turn(u32);

impl std::fmt::Display for Turn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(t::ROTATIONS[(self.0 % 4) as usize])
    }
}

/// The space's rectangle of a screen that is on.
fn rect(lit: &Lit) -> Rect {
    let (w, h) = lit.logical_size();
    Rect {
        x: f64::from(lit.position.0),
        y: f64::from(lit.position.1),
        w,
        h,
    }
}

/// The main screen: the one at the origin, or the first one on.
fn main_screen(screens: &[Screen]) -> Option<&str> {
    let on = screens.iter().filter(|s| s.on.is_some());
    let origin = on
        .clone()
        .find(|s| s.on.is_some_and(|l| l.position == (0, 0)));
    origin
        .or(on.min_by_key(|s| s.on.map(|l| l.position)))
        .map(|s| s.name.as_str())
}

/// The whole configuration as the screens are now.
fn current(screens: &[Screen]) -> Request {
    Request(screens.iter().map(|s| (s.name.clone(), s.on)).collect())
}

impl Displays {
    fn screen(&self) -> Option<&Screen> {
        let name = self.selected.as_ref()?;
        self.screens.iter().find(|s| &s.name == name)
    }

    /// Sends `request` unless it changes nothing; `confirm`: it waits for « Garder ».
    fn send(&mut self, request: Request, confirm: bool) {
        if request == current(&self.screens) {
            return;
        }
        self.send_anyway(request, confirm);
    }

    fn send_anyway(&mut self, request: Request, confirm: bool) {
        let Some(sender) = &self.sender else {
            return;
        };
        let previous = current(&self.screens);
        self.refused = false;
        if sender.send(request).is_ok() {
            self.waiting = Some((previous, confirm));
        }
    }

    /// The configuration with the chosen screen changed by `change`.
    fn with(&self, change: impl FnOnce(&Screen, &mut Option<Lit>)) -> Option<Request> {
        let screen = self.screen()?;
        let mut request = current(&self.screens);
        let entry = request.0.iter_mut().find(|(n, _)| *n == screen.name)?;
        change(screen, &mut entry.1);
        Some(request)
    }

    /// The configuration with the screen dropped at `at`, along another, the main screen
    /// staying at the origin.
    fn dropped(&self, name: &str, at: (f64, f64)) -> Option<Request> {
        let moving = self.screens.iter().find(|s| s.name == name)?.on?;
        let others: Vec<Rect> = self
            .screens
            .iter()
            .filter(|s| s.name != name)
            .filter_map(|s| s.on.as_ref().map(rect))
            .collect();
        let placed = Rect {
            x: at.0,
            y: at.1,
            ..rect(&moving)
        };
        let (x, y) = snap(placed, &others);
        let mut request = current(&self.screens);
        for (n, lit) in &mut request.0 {
            if let Some(lit) = lit
                && n == name
            {
                lit.position = (x, y);
            }
        }
        let main = main_screen(&self.screens).map(str::to_owned);
        let origin = request
            .0
            .iter()
            .find(|(n, _)| Some(n) == main.as_ref())
            .and_then(|(_, l)| l.map(|l| l.position))
            .unwrap_or((0, 0));
        for lit in request.0.iter_mut().filter_map(|(_, l)| l.as_mut()) {
            lit.position = (lit.position.0 - origin.0, lit.position.1 - origin.1);
        }
        Some(request)
    }
}

impl App {
    pub(crate) fn update_displays(&mut self, change: Change) {
        let d = &mut self.displays;
        match change {
            Change::Event(Event::Ready(sender)) => d.sender = Some(sender),
            Change::Event(Event::Screens(screens)) => {
                d.state = State::Ready;
                let known = d
                    .selected
                    .as_ref()
                    .is_some_and(|n| screens.iter().any(|s| &s.name == n));
                if !known {
                    d.selected = main_screen(&screens)
                        .or(screens.first().map(|s| s.name.as_str()))
                        .map(str::to_owned);
                }
                d.screens = screens;
            }
            Change::Event(Event::Answer(answer)) => {
                let waiting = d.waiting.take();
                match (answer, waiting) {
                    (Answer::Applied, Some((previous, true))) => {
                        d.confirm = Some((previous, Instant::now() + cr::CONFIRM_DELAY));
                    }
                    (Answer::Refused, _) => d.refused = true,
                    _ => {}
                }
            }
            Change::Event(Event::Unavailable) => {
                d.sender = None;
                if d.state == State::Loading {
                    d.state = State::Unavailable;
                }
            }
            Change::Select(name) => d.selected = Some(name),
            Change::Grab(name) => {
                let Some((fit, lit)) = d
                    .fit
                    .get()
                    .zip(d.screens.iter().find(|s| s.name == name).and_then(|s| s.on))
                else {
                    return;
                };
                let r = rect(&lit);
                d.selected = Some(name.clone());
                // Grabbed where the pointer will say on its first move: at its center.
                let _ = fit;
                d.drag = Some((
                    name,
                    (r.w / 2.0, r.h / 2.0),
                    (r.x + r.w / 2.0, r.y + r.h / 2.0),
                ));
            }
            Change::Drag(point) => {
                if let (Some((_, _, at)), Some(fit)) = (d.drag.as_mut(), d.fit.get()) {
                    *at = fit.to_space((f64::from(point.x), f64::from(point.y)));
                }
            }
            Change::Drop => {
                if let Some((name, grab, at)) = d.drag.take()
                    && let Some(request) = d.dropped(&name, (at.0 - grab.0, at.1 - grab.1))
                {
                    // Moving a screen never leaves one black: no confirmation.
                    d.send(request, false);
                }
            }
            Change::Resolution(w, h) => {
                let request = d.with(|screen, lit| {
                    let size = |m: &&Mode| (m.width, m.height) == (w, h);
                    let mode = screen.modes.iter().find(size);
                    if let (Some(lit), Some(mode)) = (lit.as_mut(), mode) {
                        lit.mode = *mode;
                    }
                });
                if let Some(request) = request {
                    d.send(request, true);
                }
            }
            Change::Refresh(refresh) => {
                let request = d.with(|screen, lit| {
                    let Some(lit) = lit.as_mut() else { return };
                    let size = (lit.mode.width, lit.mode.height);
                    let same = |m: &&Mode| (m.width, m.height) == size && m.refresh == refresh;
                    if let Some(mode) = screen.modes.iter().find(same) {
                        lit.mode = *mode;
                    }
                });
                if let Some(request) = request {
                    d.send(request, true);
                }
            }
            Change::Scale(scale) => {
                if let Some(request) = d.with(|_, lit| lit.iter_mut().for_each(|l| l.scale = scale))
                {
                    d.send(request, true);
                }
            }
            Change::Rotation(turn) => {
                let request = d.with(|_, lit| lit.iter_mut().for_each(|l| l.transform = turn));
                if let Some(request) = request {
                    d.send(request, true);
                }
            }
            Change::Main => {
                let origin = d.screen().and_then(|s| s.on).map(|l| l.position);
                if let Some(origin) = origin {
                    let mut request = current(&d.screens);
                    for lit in request.0.iter_mut().filter_map(|(_, l)| l.as_mut()) {
                        lit.position = (lit.position.0 - origin.0, lit.position.1 - origin.1);
                    }
                    d.send(request, false);
                }
            }
            Change::On(on) => {
                let right = d
                    .screens
                    .iter()
                    .filter_map(|s| s.on.as_ref().map(rect))
                    .map(|r| (r.x + r.w).round() as i32)
                    .max()
                    .unwrap_or(0);
                let request = d.with(|screen, lit| {
                    *lit = match on {
                        false => None,
                        true => screen.modes.first().map(|mode| Lit {
                            mode: *mode,
                            position: (right, 0),
                            transform: 0,
                            scale: 1.0,
                        }),
                    };
                });
                if let Some(request) = request {
                    d.send(request, true);
                }
            }
            Change::Keep => d.confirm = None,
            Change::Revert => {
                if let Some((previous, _)) = d.confirm.take() {
                    d.send_anyway(previous, false);
                }
            }
        }
    }

    /// Goes back once the confirmation has waited long enough.
    pub(crate) fn tick_displays(&mut self, now: Instant) {
        if self
            .displays
            .confirm
            .as_ref()
            .is_some_and(|(_, until)| now >= *until)
        {
            self.update_displays(Change::Revert);
        }
    }

    pub(crate) fn displays(&self, width: f32) -> Vec<Element<'_, Message>> {
        let d = &self.displays;
        let note = |text| {
            let note = container(ui::description(text)).padding([metrics::GAP_ICON, 0.0]);
            vec![self.card(None, vec![note.into()])]
        };
        match d.state {
            State::Loading => return note(t::DISPLAYS_LOADING),
            State::Unavailable => return note(t::DISPLAYS_UNAVAILABLE),
            State::Ready => {}
        }
        let mut cards = Vec::new();
        if let Some((_, until)) = &d.confirm {
            let left = until.saturating_duration_since(Instant::now()).as_secs() + 1;
            let line = row![
                column![
                    ui::body(t::KEEP_DISPLAYS).font(typography::UI_SEMIBOLD),
                    ui::description(t::reverting_in(left.min(cr::CONFIRM_DELAY.as_secs()))),
                ]
                .width(Length::Fill),
                ui::secondary_button(t::REVERT, Some(Message::Displays(Change::Revert))),
                ui::primary_button(t::KEEP, Some(Message::Displays(Change::Keep))),
            ]
            .spacing(metrics::GAP_ICON)
            .align_y(Alignment::Center);
            cards.push(self.card(None, vec![line.into()]));
        }
        let arrangement = self.arrangement(width);
        let mut rows = vec![self.row(
            Setting::Arrangement,
            width,
            t::ARRANGEMENT_HELP,
            space().into(),
            false,
        )];
        rows.push(arrangement);
        cards.push(self.card(None, rows));
        let Some(screen) = d.screen() else {
            return cards;
        };
        let title = match screen.model.is_empty() {
            true => screen.name.as_str(),
            false => screen.model.as_str(),
        };
        let mut rows: Vec<Element<'_, Message>> = Vec::new();
        if d.refused {
            rows.push(ui::description(t::DISPLAY_REFUSED).into());
        }
        let several = d.screens.len() > 1;
        if several {
            let others_on = d
                .screens
                .iter()
                .any(|s| s.name != screen.name && s.on.is_some());
            let switch: fn(bool) -> Message = |on| Message::Displays(Change::On(on));
            // The last screen on cannot be switched off.
            let toggle = ui::toggle(
                screen.on.is_some(),
                (others_on || screen.on.is_none()).then_some(switch),
            );
            rows.push(self.row(
                Setting::DisplayOn,
                width,
                t::DISPLAY_ON_HELP,
                toggle.into(),
                false,
            ));
        }
        if let Some(lit) = screen.on {
            rows.extend(self.screen_rows(screen, lit, width, several));
        }
        cards.push(self.card(Some(title), rows));
        cards
    }

    fn screen_rows<'a>(
        &'a self,
        screen: &'a Screen,
        lit: Lit,
        width: f32,
        several: bool,
    ) -> Vec<Element<'a, Message>> {
        let mut sizes: Vec<Size> = Vec::new();
        for (i, m) in screen.modes.iter().enumerate() {
            if !sizes.iter().any(|s| (s.0, s.1) == (m.width, m.height)) {
                sizes.push(Size(m.width, m.height, i == 0));
            }
        }
        // The preferred first, then the largest.
        let first = 1.min(sizes.len());
        sizes[first..].sort_by_key(|s| std::cmp::Reverse(i64::from(s.0) * i64::from(s.1)));
        let chosen = sizes
            .iter()
            .copied()
            .find(|s| (s.0, s.1) == (lit.mode.width, lit.mode.height));
        let resolution = ui::dropdown(
            sizes,
            chosen,
            |s: Size| Message::Displays(Change::Resolution(s.0, s.1)),
            cr::DISPLAY_MENU_WIDTH,
        );
        let mut rates: Vec<Hertz> = screen
            .modes
            .iter()
            .filter(|m| (m.width, m.height) == (lit.mode.width, lit.mode.height))
            .map(|m| Hertz(m.refresh))
            .collect();
        rates.sort_by_key(|h| std::cmp::Reverse(h.0));
        rates.dedup();
        let refresh = ui::dropdown(
            rates,
            Some(Hertz(lit.mode.refresh)),
            |h: Hertz| Message::Displays(Change::Refresh(h.0)),
            cr::DISPLAY_MENU_WIDTH,
        );
        let scales = cr::SCALES
            .iter()
            .zip(SCALE_LABELS)
            .map(|(s, label)| {
                let chosen = (lit.scale - s).abs() < 0.01;
                (label, chosen, Some(Message::Displays(Change::Scale(*s))))
            })
            .collect();
        let turns: Vec<Turn> = (0..4).map(Turn).collect();
        let rotation = ui::dropdown(
            turns,
            Some(Turn(lit.transform % 4)),
            |turn: Turn| Message::Displays(Change::Rotation(turn.0)),
            cr::DISPLAY_MENU_WIDTH,
        );
        let mut rows = vec![
            self.row(
                Setting::Resolution,
                width,
                t::RESOLUTION_HELP,
                resolution,
                false,
            ),
            self.row(Setting::Refresh, width, t::REFRESH_HELP, refresh, false),
            self.row(
                Setting::Scale,
                width,
                t::SCALE_HELP,
                ui::segmented(scales),
                true,
            ),
            self.row(Setting::Rotation, width, t::ROTATION_HELP, rotation, false),
        ];
        if several {
            let is_main = main_screen(&self.displays.screens) == Some(screen.name.as_str());
            let control: Element<'_, Message> = match is_main {
                true => ui::description(t::IS_MAIN).into(),
                false => {
                    ui::secondary_button(t::MAKE_MAIN, Some(Message::Displays(Change::Main))).into()
                }
            };
            rows.push(self.row(
                Setting::MainDisplay,
                width,
                t::MAIN_DISPLAY_HELP,
                control,
                false,
            ));
        }
        rows
    }

    /// The screens drawn to scale, side by side as in the space: a click chooses one, a drag
    /// moves it. The main screen has the bar on top.
    fn arrangement(&self, width: f32) -> Element<'_, Message> {
        let d = &self.displays;
        let area = (width - 2.0 * metrics::CARD_PADDING.1).max(1.0);
        let mut rects: Vec<(&Screen, Rect)> = d
            .screens
            .iter()
            .filter_map(|s| s.on.as_ref().map(|l| (s, rect(l))))
            .collect();
        if let Some((name, grab, at)) = &d.drag
            && let Some((_, r)) = rects.iter_mut().find(|(s, _)| &s.name == name)
        {
            (r.x, r.y) = (at.0 - grab.0, at.1 - grab.1);
        }
        // Fitted on the screens at rest: the drawing does not shrink while one is dragged.
        let at_rest: Vec<Rect> = d
            .screens
            .iter()
            .filter_map(|s| s.on.as_ref().map(rect))
            .collect();
        let fit = Fit::new(
            &at_rest,
            f64::from(area),
            f64::from(cr::ARRANGEMENT_HEIGHT),
            f64::from(cr::ARRANGEMENT_MARGIN),
        );
        d.fit.set(Some(fit));
        let main = main_screen(&d.screens);
        let mut layers: Vec<Element<'_, Message>> = vec![
            space()
                .width(Length::Fill)
                .height(cr::ARRANGEMENT_HEIGHT)
                .into(),
        ];
        // The one dragged is drawn last, over the others.
        rects.sort_by_key(|(s, _)| d.drag.as_ref().is_some_and(|(n, _, _)| *n == s.name));
        for (screen, r) in rects {
            let box_ = fit.to_area(r);
            let selected = d.selected.as_deref() == Some(screen.name.as_str());
            let is_main = main == Some(screen.name.as_str());
            let label = match screen.model.is_empty() {
                true => screen.name.as_str(),
                false => screen.model.as_str(),
            };
            let bar = container(space())
                .width(Length::Fill)
                .height(if is_main { cr::MAIN_BAR } else { 0.0 })
                .style(|th| theme::veil(theme::palette(th).text_tertiary));
            let face = column![
                bar,
                container(ui::ellipsis(label).size(typography::SMALL_SIZE)).center(Length::Fill)
            ];
            let face = container(face)
                .width(box_.w as f32)
                .height(box_.h as f32)
                .clip(true)
                .style(move |th| {
                    let p = theme::palette(th);
                    let edge = if selected { p.accent } else { p.divider };
                    iced::widget::container::Style {
                        border: iced::border::rounded(cr::SCREEN_RADIUS)
                            .width(if selected { 2.0 } else { metrics::BORDER_WIDTH })
                            .color(iced::Color::from(edge)),
                        ..theme::veil(p.selected_row)
                    }
                });
            let name = screen.name.clone();
            let handle = mouse_area(face)
                .on_press(Message::Displays(Change::Grab(name)))
                .interaction(mouse::Interaction::Grab);
            layers.push(pin(handle).x(box_.x as f32).y(box_.y as f32).into());
        }
        let dragging = d.drag.is_some();
        let area = mouse_area(
            stack(layers)
                .width(Length::Fill)
                .height(cr::ARRANGEMENT_HEIGHT),
        )
        .on_move(|p| Message::Displays(Change::Drag(p)))
        .on_release(Message::Displays(Change::Drop))
        .on_exit(Message::Displays(Change::Drop));
        let area = match dragging {
            true => area.interaction(mouse::Interaction::Grabbing),
            false => area,
        };
        let _ = color::palette;
        let _ = (Row::<Message>::new(), Column::<Message>::new());
        container(area).width(Length::Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(name: &str, position: (i32, i32)) -> Screen {
        let mode = Mode {
            width: 1920,
            height: 1080,
            refresh: 60_000,
        };
        Screen {
            name: name.into(),
            make: String::new(),
            model: String::new(),
            modes: vec![mode],
            on: Some(Lit {
                mode,
                position,
                transform: 0,
                scale: 1.0,
            }),
        }
    }

    #[test]
    fn the_main_screen_is_at_the_origin() {
        let screens = [screen("A", (1920, 0)), screen("B", (0, 0))];
        assert_eq!(main_screen(&screens), Some("B"));
        let screens = [screen("A", (100, 0)), screen("B", (-50, 0))];
        assert_eq!(main_screen(&screens), Some("B"));
    }

    #[test]
    fn a_screen_dropped_left_of_the_main_one_keeps_the_main_at_the_origin() {
        let d = Displays {
            screens: vec![screen("main", (0, 0)), screen("other", (1920, 0))],
            ..Displays::default()
        };
        let request = d.dropped("other", (-1700.0, 40.0)).unwrap();
        let at = |n: &str| {
            request
                .0
                .iter()
                .find(|(m, _)| m == n)
                .unwrap()
                .1
                .unwrap()
                .position
        };
        assert_eq!(at("main"), (0, 0));
        assert_eq!(at("other"), (-1920, 40));
        // Dragging the main screen below the other moves the other instead: above it.
        let request = d.dropped("main", (1920.0, 900.0)).unwrap();
        let at = |n: &str| {
            request
                .0
                .iter()
                .find(|(m, _)| m == n)
                .unwrap()
                .1
                .unwrap()
                .position
        };
        assert_eq!(at("main"), (0, 0));
        assert_eq!(at("other"), (0, -1080));
    }
}
