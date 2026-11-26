//! Bar state, messages, and its two views: the bar and the Sherlock menu.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::widget::{container, mouse_area, opaque, row, text};
use iced::window::Id;
use iced::{Alignment, Element, Length, Padding, Subscription, Task, keyboard, window};
use iced_exwlshell::actions::{IcedNewPopupSettings, RegionActionCB};
use iced_exwlshell::reexport::{Margin, PixelSize, PopupAnchor, PopupGravity, Position};
use iced_exwlshell::shell::{ShellEvent, ShellReceiver};
use iced_exwlshell::to_layer_message;
use sherlock_ui::color::{self, Theme, ThemeFade};
use sherlock_ui::component::{self as ui};
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::icon::Icon;
use sherlock_ui::surface::{self, Elevation};
use sherlock_ui::{metrics, panels, strings, typography};

use crate::calendar::Month;
use crate::history::{self, Entry};
use crate::menu::{self, Action};
use crate::panel::{self, History, Panel};
use crate::quick::{self, Quick};
use crate::shape::{Geometry, Shape};
use crate::source::battery::{Battery, Charge, UPower};
use crate::source::clock::{LocalTime, SystemClock};
use crate::source::network::{Network, NetworkManager};
use crate::source::shell::{self, Auto};
use crate::source::sound::{Pactl, Sound};
use crate::source::watch;
use crate::source::workspaces::{Desktop, Group};
use crate::{bus, user};

#[derive(Debug, Default)]
pub struct Bar {
    pub clock: Option<LocalTime>,
    /// `Some(None)`: the machine has no battery.
    pub battery: Option<Option<Battery>>,
    pub network: Option<Network>,
    pub sound: Option<Sound>,
    pub desktop: Option<Desktop>,
    pub user: String,
    /// Surface of the bar, which the menu attaches to.
    surface: Option<Id>,
    /// Name of the screen each surface is on, as iced_exwlshell reports it.
    outputs: HashMap<Id, Option<String>>,
    /// Where iced_exwlshell reports the screens of the surfaces.
    shell_events: Option<ShellReceiver>,
    /// Floating or attached to the top edge, and the spring between the two.
    pub shape: Shape,
    /// The margins last given to the surface.
    applied: Geometry,
    /// Surface of the Sherlock menu while it is open.
    menu: Option<Id>,
    /// Room around the menu in its surface: the gap below the bar, and its shadow on the GPU.
    pub menu_margins: Padding,
    /// Menu entry highlighted by the keyboard or the pointer.
    pub cursor: Option<usize>,
    /// Bar button that has the keyboard focus, as an index in `targets()`. The bar keeps it in
    /// its own state, like the menu cursor: the ring is drawn from the same value that Enter
    /// activates, in the same update. (iced's focus operations are applied after the redraw
    /// under `iced_exwlshell`, which left the ring one Tab behind.)
    focus: Option<usize>,
    /// The theme of the appearance file, shown with a fade.
    pub theme: ThemeFade,
    /// The panel open, and its surface.
    panel: Option<(Panel, Id)>,
    /// Room around the panel in its surface: the gap below the bar, and its shadow on the GPU.
    pub panel_margins: Padding,
    /// Width of the bar surface, which the panels are placed from.
    width: Option<f32>,
    /// The month the calendar shows.
    pub month: Option<Month>,
    /// What the notification centre shows.
    pub history: History,
    /// The quick settings of the control centre.
    pub quick: Quick,
}

/// A bar button that Tab can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Sherlock,
    Workspace(usize),
    /// The indicators: the control centre.
    Status,
    /// The date and time: the calendar.
    Clock,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Surface(Id),
    Clock(LocalTime),
    Battery(Option<Battery>),
    Network(Network),
    Sound(Sound),
    Desktop(Desktop),
    /// The screen a surface is on.
    Output(Id, Option<String>),
    ToggleMenu,
    MenuKey(menu::Key),
    Hover(usize),
    Activate(Action),
    SelectWorkspace(usize),
    Focus(Direction),
    /// A mouse button was pressed somewhere in the bar or its menu.
    Pointer,
    Closed(Id),
    /// The width of a surface, when it is configured.
    Resized(Id, f32),
    /// Opens a panel, or closes it when it is open.
    TogglePanel(Panel),
    ClosePanel,
    /// The calendar shows this month.
    ShowMonth(Month),
    /// Page Up or Page Down in the calendar: the month before or after.
    Page(bool),
    /// The history of the notification daemon, or why it could not be read.
    History(Result<Vec<Entry>, String>),
    /// The daemon says the history changed.
    HistoryChanged,
    /// « Tout effacer ».
    ClearHistory,
    /// An action of an open notification, from the centre.
    InvokeAction(u32, String),
    /// A call to the daemon ended.
    HistoryDone(Result<(), String>),
    /// The quick settings.
    Quick(quick::Message),
    /// The theme of the appearance file, at start then at each change.
    Theme(Theme),
    Frame(Instant),
}

impl Bar {
    /// The bar, told of the screens of its surfaces by `shell_events`.
    pub fn new(shell_events: ShellReceiver) -> (Self, Task<Message>) {
        let session = launcher::system::Systemd::from_env();
        let bar = Self {
            user: user::current(),
            shell_events: Some(shell_events),
            quick: Quick::new(std::sync::Arc::new(session)),
            ..Self::default()
        };
        (bar, Task::none())
    }

    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// The panel open, if any.
    pub fn panel_open(&self) -> Option<Panel> {
        self.panel.map(|(panel, _)| panel)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let appearance = match appearance_config::path() {
            Some(path) => Subscription::run_with(path, |path| appearance(path)).map(Message::Theme),
            None => Subscription::none(),
        };
        let frames = if self.theme.running() || self.shape.running() {
            window::frames().map(Message::Frame)
        } else {
            Subscription::none()
        };
        let history = if self.panel_open() == Some(Panel::Control) {
            Subscription::batch([
                Subscription::run(history::watch).map(|()| Message::HistoryChanged),
                Subscription::run(quick::watch)
                    .map(|service| Message::Quick(quick::Message::Changed(service))),
            ])
        } else {
            Subscription::none()
        };
        Subscription::batch([
            appearance,
            frames,
            history,
            Subscription::run(|| watch(SystemClock::default(), "clock")).map(Message::Clock),
            Subscription::run(|| watch(UPower::default(), "battery")).map(Message::Battery),
            Subscription::run(|| watch(NetworkManager::default(), "network")).map(Message::Network),
            Subscription::run(|| watch(Pactl::default(), "sound")).map(Message::Sound),
            Subscription::run(|| watch(Auto::default(), "workspaces")).map(Message::Desktop),
            self.shell_events
                .as_ref()
                .map_or_else(Subscription::none, |events| {
                    events.listen().filter_map(on_output)
                }),
            window::close_events().map(Message::Closed),
            keyboard::listen().filter_map(on_key),
            iced::event::listen_with(on_pointer),
            iced::event::listen_with(on_resize),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) => self.surface = Some(id),
            Message::Clock(time) => self.clock = Some(time),
            Message::Battery(battery) => self.battery = Some(battery),
            Message::Network(network) => self.network = Some(network),
            Message::Sound(sound) => self.sound = Some(sound),
            Message::Desktop(desktop) => {
                self.desktop = Some(desktop);
                return self.reshape(Instant::now());
            }
            Message::Output(id, name) => {
                self.outputs.insert(id, name);
                return self.reshape(Instant::now());
            }
            Message::ToggleMenu => return self.toggle_menu(),
            Message::TogglePanel(panel) => return self.toggle_panel(panel),
            Message::ClosePanel => return self.close_panel(),
            Message::Resized(id, width) => {
                if Some(id) == self.surface {
                    self.width = Some(width);
                }
            }
            Message::ShowMonth(month) => self.month = Some(month),
            Message::Page(forward) => {
                if let Some(month) = self.month {
                    self.month = Some(if forward {
                        month.next()
                    } else {
                        month.previous()
                    });
                }
            }
            Message::History(Ok(entries)) => {
                let today = panel::today(self.clock.as_ref());
                let entries = entries
                    .into_iter()
                    .map(|entry| Entry {
                        when: panel::when(entry.received, today),
                        ..entry
                    })
                    .collect();
                self.history = History::Ready(history::group(entries));
            }
            Message::History(Err(error)) => {
                tracing::warn!(%error, "cannot read the notification history");
                self.history = History::Unavailable;
            }
            Message::HistoryChanged if self.panel_open() == Some(Panel::Control) => {
                return load_history();
            }
            Message::HistoryChanged => {}
            Message::ClearHistory => {
                return Task::perform(
                    async { history::clear(&bus::session().await?).await },
                    |done| Message::HistoryDone(done.map_err(|e| e.to_string())),
                );
            }
            Message::InvokeAction(id, key) => {
                return Task::perform(
                    async move { history::invoke(&bus::session().await?, id, &key).await },
                    |done| Message::HistoryDone(done.map_err(|e| e.to_string())),
                );
            }
            Message::Quick(message) => {
                return self.quick.update(message).map(Message::Quick);
            }
            Message::HistoryDone(done) => {
                if let Err(error) = done {
                    tracing::warn!(%error, "the notification daemon refused");
                }
                return load_history();
            }
            // Escape closes the panel; Enter and Space belong to the control that has the
            // focus in it.
            Message::MenuKey(menu::Key::Close) if self.panel.is_some() => {
                return self.close_panel();
            }
            Message::MenuKey(menu::Key::First) if self.panel_open() == Some(Panel::Calendar) => {
                self.month = panel::today(self.clock.as_ref()).map(Month::of);
            }
            Message::MenuKey(_) if self.panel.is_some() => {}
            Message::Focus(direction) if self.panel.is_some() => {
                return focus::advance(direction);
            }
            Message::MenuKey(key) if self.menu_open() => match key {
                menu::Key::Close => return self.close_menu(),
                menu::Key::Activate => {
                    if let Some(action) = self.cursor.and_then(|i| Action::ALL.get(i)) {
                        return self.update(Message::Activate(*action));
                    }
                }
                key => self.cursor = menu::move_cursor(self.cursor, key),
            },
            Message::MenuKey(menu::Key::Activate) => {
                if let Some(message) = self.activation() {
                    return self.update(message);
                }
            }
            Message::MenuKey(menu::Key::Close) => self.focus = None,
            Message::MenuKey(_) => {}
            Message::Hover(index) => self.cursor = Some(index),
            Message::Activate(action) => {
                // Nothing exists behind these entries yet: say so, never pretend.
                tracing::info!(?action, "Sherlock menu entry not available yet");
                return self.close_menu();
            }
            Message::SelectWorkspace(index) => match self.to_activate(index) {
                Some(id) => shell::switch_to(id),
                None if self.desktop.as_ref().is_some_and(|d| !d.live) => {
                    tracing::info!(index, "placeholder workspaces: nothing to switch");
                }
                None => {}
            },
            Message::Focus(direction) if self.menu_open() => {
                let key = match direction {
                    Direction::Next => menu::Key::Next,
                    Direction::Previous => menu::Key::Previous,
                };
                self.cursor = menu::move_cursor(self.cursor, key);
            }
            Message::Focus(direction) => {
                let count = self.targets().len();
                self.focus = match (direction, self.focus) {
                    _ if count == 0 => None,
                    (Direction::Next, Some(i)) if i + 1 < count => Some(i + 1),
                    (Direction::Next, _) => Some(0),
                    (Direction::Previous, Some(i)) if i > 0 && i <= count => Some(i - 1),
                    (Direction::Previous, _) => Some(count - 1),
                };
            }
            // Like `:focus-visible` in a browser: the mouse removes the keyboard ring.
            Message::Pointer => self.focus = None,
            Message::Theme(theme) => self.theme.set(theme, Instant::now()),
            Message::Frame(now) => {
                self.theme.tick(now);
                self.shape.tick(now);
                return self.apply_shape();
            }
            Message::Closed(id) => {
                // The compositor closes the menu and the panels on a click elsewhere
                // (popup_done).
                self.outputs.remove(&id);
                if self.menu == Some(id) {
                    self.menu = None;
                    self.cursor = None;
                }
                if self.panel.is_some_and(|(_, panel)| panel == id) {
                    self.panel = None;
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// Name of the screen the bar is on, once known.
    pub fn output(&self) -> Option<&str> {
        self.surface
            .and_then(|id| self.outputs.get(&id))
            .and_then(Option::as_deref)
    }

    /// The workspaces of the bar's screen.
    pub fn group(&self) -> Option<&Group> {
        self.desktop.as_ref()?.group(self.output())
    }

    /// Id of the workspace a click on the chip `index` switches to: one of the compositor's,
    /// not already active.
    pub fn to_activate(&self, index: usize) -> Option<&str> {
        let live = self.desktop.as_ref().is_some_and(|d| d.live);
        let workspace = self.group()?.workspaces.get(index)?;
        (live && !workspace.active).then_some(workspace.id.as_str())
    }

    /// Attaches the bar to the top edge while a maximized or fullscreen window is on its
    /// screen, floats it otherwise.
    fn reshape(&mut self, now: Instant) -> Task<Message> {
        let output = self.output().map(str::to_owned);
        let covered = self
            .desktop
            .as_ref()
            .is_some_and(|d| d.covers(output.as_deref()));
        if self.shape.set(covered, now) {
            tracing::debug!(?output, attached = covered, "bar shape");
        }
        self.apply_shape()
    }

    /// Gives the surface the margins of the current shape, when they changed by a pixel.
    fn apply_shape(&mut self) -> Task<Message> {
        let geometry = self.shape.geometry();
        let Some(id) = self.surface else {
            return Task::none();
        };
        let moved = (geometry.margin_top, geometry.margin_side)
            != (self.applied.margin_top, self.applied.margin_side);
        if !moved {
            return Task::none();
        }
        self.applied = geometry;
        let side = geometry.margin_side;
        let margin = Margin {
            top: geometry.margin_top,
            right: side,
            bottom: 0,
            left: side,
        };
        Task::done(Message::MarginChange { id, margin })
    }

    /// The bar buttons, in Tab order: Sherlock, the workspaces, the indicators, the clock.
    pub fn targets(&self) -> Vec<Target> {
        let workspaces = self.group().map_or(0, |g| g.workspaces.len());
        let status = self.network.is_some() || self.sound.is_some() || self.battery.is_some();
        std::iter::once(Target::Sherlock)
            .chain((0..workspaces).map(Target::Workspace))
            .chain(status.then_some(Target::Status))
            .chain(self.clock.is_some().then_some(Target::Clock))
            .collect()
    }

    /// The bar button that has the keyboard focus: the one drawn with the ring.
    pub fn focused(&self) -> Option<Target> {
        self.focus.and_then(|i| self.targets().get(i).copied())
    }

    /// What Enter or Space does: activate the focused button.
    pub fn activation(&self) -> Option<Message> {
        self.focused().map(|target| match target {
            Target::Sherlock => Message::ToggleMenu,
            Target::Workspace(i) => Message::SelectWorkspace(i),
            Target::Status => Message::TogglePanel(Panel::Control),
            Target::Clock => Message::TogglePanel(Panel::Calendar),
        })
    }

    /// Opens `panel` under the right end of the bar, or closes it when it is open. Another
    /// panel or the menu closes first.
    fn toggle_panel(&mut self, which: Panel) -> Task<Message> {
        let open = self.panel_open();
        let closed = self.close_panel().chain(self.close_menu());
        if open == Some(which) {
            return closed;
        }
        let Some(surface) = self.surface else {
            tracing::warn!("bar surface unknown: the panel cannot open");
            return closed;
        };
        self.panel_margins = panel::margins(surface::gpu_drawn() == Some(true));
        let (width, height) = panel::surface_size(which, self.panel_margins);
        let bar_width = self
            .width
            .unwrap_or(metrics::REFERENCE_SCREEN.0 - 2.0 * metrics::TOP_BAR_MARGIN_SIDE);
        let side = self.applied.margin_side as f32;
        let (x, y, anchor_width, anchor_height) =
            panel::anchor_rect(which, self.panel_margins, bar_width, side);
        let settings = IcedNewPopupSettings::new(
            surface,
            PixelSize::px(width, height),
            Position::new(x, y),
            PixelSize::px(anchor_width, anchor_height),
        )
        .anchor(PopupAnchor::Bottom)
        .gravity(PopupGravity::Bottom);
        let id = Id::unique();
        self.panel = Some((which, id));
        let (x, y, width, height) = panel::input_rect(which, self.panel_margins);
        let region = RegionActionCB::new(move |region| {
            if let Err(error) = region.add(x, y, width, height) {
                tracing::warn!(%error, "cannot limit the panel's input region");
            }
        });
        let opened = closed
            .chain(Task::done(Message::NewPopUp { settings, id }))
            .chain(Task::done(Message::SetInputRegion {
                id,
                callback: region,
            }));
        match which {
            Panel::Control => {
                self.history = History::Loading;
                let quick = self.quick.open().map(Message::Quick);
                opened.chain(Task::batch([load_history(), quick]))
            }
            Panel::Calendar => {
                self.month = panel::today(self.clock.as_ref()).map(Month::of);
                opened
            }
        }
    }

    fn close_panel(&mut self) -> Task<Message> {
        match self.panel.take() {
            Some((_, id)) => window::close(id),
            None => Task::none(),
        }
    }

    fn toggle_menu(&mut self) -> Task<Message> {
        if self.menu_open() {
            return self.close_menu();
        }
        let closed = self.close_panel();
        let Some(surface) = self.surface else {
            tracing::warn!("bar surface unknown: the menu cannot open");
            return Task::none();
        };
        // The bar has been drawn by then: its renderer is known.
        self.menu_margins = menu::margins(surface::gpu_drawn() == Some(true));
        let (width, height) = menu::surface_size(self.menu_margins);
        let (x, y, anchor_width, anchor_height) = menu::anchor_rect(self.menu_margins);
        let settings = IcedNewPopupSettings::new(
            surface,
            PixelSize::px(width, height),
            Position::new(x, y),
            PixelSize::px(anchor_width, anchor_height),
        )
        .anchor(PopupAnchor::Bottom)
        .gravity(PopupGravity::Bottom);
        let id = Id::unique();
        self.menu = Some(id);
        self.cursor = None;
        // Only the menu takes the pointer: the transparent room for the shadow lies over the
        // bar, whose buttons stay reachable through it.
        let (x, y, width, height) = menu::input_rect(self.menu_margins);
        let region = RegionActionCB::new(move |region| {
            if let Err(error) = region.add(x, y, width, height) {
                tracing::warn!(%error, "cannot limit the menu's input region");
            }
        });
        closed
            .chain(Task::done(Message::NewPopUp { settings, id }))
            .chain(Task::done(Message::SetInputRegion {
                id,
                callback: region,
            }))
    }

    fn close_menu(&mut self) -> Task<Message> {
        self.cursor = None;
        match self.menu.take() {
            Some(id) => window::close(id),
            None => Task::none(),
        }
    }

    pub fn view(&self, id: Id) -> Element<'_, Message> {
        match self.panel {
            Some((which, panel)) if panel == id => return self.view_panel(which),
            _ => {}
        }
        if self.menu == Some(id) {
            self.view_menu()
        } else {
            self.view_bar()
        }
    }

    /// A panel in its surface.
    pub fn view_panel(&self, which: Panel) -> Element<'_, Message> {
        let glasses = match which {
            Panel::Control => vec![
                panel::glass(panel::center(&self.history), which, panels::CENTER_WIDTH),
                panel::glass(
                    self.quick.view(&self.user).map(Message::Quick),
                    which,
                    panels::QUICK_WIDTH,
                ),
            ],
            Panel::Calendar => {
                let today = panel::today(self.clock.as_ref());
                let month = self.month.or_else(|| today.map(Month::of));
                let content = match month {
                    Some(month) => panel::calendar(month, today),
                    None => iced::widget::space().into(),
                };
                vec![panel::glass(content, which, panels::CALENDAR_WIDTH)]
            }
        };
        panel::frame(glasses, self.panel_margins)
    }

    pub fn view_bar(&self) -> Element<'_, Message> {
        let ring = |control: ui::Control<'static, Message>, target| {
            if self.focused() == Some(target) {
                control.focus_ring()
            } else {
                control
            }
        };
        let sherlock = ring(
            ui::sherlock_button(self.menu_open(), Some(Message::ToggleMenu)),
            Target::Sherlock,
        );
        let mut left = row![sherlock]
            .spacing(metrics::TOP_BAR_LEFT_GAP)
            .align_y(Alignment::Center);
        if let Some(app) = self.desktop.as_ref().and_then(|d| d.app.as_deref()) {
            let (pad_left, pad_right) = metrics::APP_NAME_PADDING;
            // A long window title is cut where the left half of the bar ends.
            let name = ui::body(app)
                .font(typography::UI_SEMIBOLD)
                .wrapping(text::Wrapping::None);
            let name = container(name).clip(true).padding(Padding {
                left: pad_left,
                right: pad_right,
                ..Padding::ZERO
            });
            left = left.push(name);
        }

        let chips = self.group().into_iter().flat_map(|g| {
            g.workspaces.iter().enumerate().map(move |(i, w)| {
                let label = strings::top_bar::workspace(i + 1, w.name.as_deref());
                let select = Some(Message::SelectWorkspace(i));
                let chip = ui::workspace_chip(label, w.active, w.occupied(), select);
                ring(chip, Target::Workspace(i)).into()
            })
        });
        let center = row(chips)
            .spacing(metrics::CHIP_GAP)
            .align_y(Alignment::Center);

        let right = self.view_indicators(&ring);

        // Both sides share the remaining space: the workspaces land in the middle.
        let (pad_left, pad_right) = metrics::TOP_BAR_PADDING;
        let line = row![
            container(left).width(Length::Fill),
            center,
            container(right)
                .width(Length::Fill)
                .align_right(Length::Fill),
        ]
        .align_y(Alignment::Center)
        .height(Length::Fill)
        .padding(Padding {
            left: pad_left + metrics::BORDER_WIDTH,
            right: pad_right - panels::BAR_BUTTON_PADDING + metrics::BORDER_WIDTH,
            ..Padding::ZERO
        });
        let shape = self.shape.geometry();
        let bar = container(line)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(sherlock_ui::theme::top_bar_shaped(
                shape.solid,
                shape.radius,
            ));
        // Flat: no shadow, but drawing it tells the menu which renderer draws the bar.
        surface::shadowed(bar, shape.radius, Elevation::Flat).into()
    }

    /// The indicators, which open the control centre, then the date and time, which open the
    /// calendar: two ghost buttons, whose padding the gaps give back.
    fn view_indicators(
        &self,
        ring: &impl Fn(ui::Control<'static, Message>, Target) -> ui::Control<'static, Message>,
    ) -> Element<'static, Message> {
        let gap = metrics::STATUS_GAP - 2.0 * panels::BAR_BUTTON_PADDING;
        let (status, clock) = self.indicators();
        let mut buttons = row![].spacing(gap.max(0.0)).align_y(Alignment::Center);
        if !status.is_empty() {
            let open = self.panel_open() == Some(Panel::Control);
            let line = row(status)
                .spacing(metrics::STATUS_GAP)
                .align_y(Alignment::Center);
            let toggle = Some(Message::TogglePanel(Panel::Control));
            buttons = buttons.push(ring(panels::bar_button(line, open, toggle), Target::Status));
        }
        if !clock.is_empty() {
            let open = self.panel_open() == Some(Panel::Calendar);
            let line = row(clock)
                .spacing(metrics::STATUS_GAP)
                .align_y(Alignment::Center);
            let toggle = Some(Message::TogglePanel(Panel::Calendar));
            buttons = buttons.push(ring(panels::bar_button(line, open, toggle), Target::Clock));
        }
        buttons.into()
    }

    /// Network, sound and battery; then date and time. An indicator not read yet is not shown.
    #[allow(clippy::type_complexity)]
    fn indicators(
        &self,
    ) -> (
        Vec<Element<'static, Message>>,
        Vec<Element<'static, Message>>,
    ) {
        let p = color::palette();
        let tint = p.text_secondary;
        let mut items = Vec::new();
        if let Some(network) = &self.network {
            let glyph = match network {
                Network::Connected { .. } => Icon::Network,
                Network::Disconnected => Icon::NetworkOff,
            };
            items.push(ui::status(glyph, None, tint));
        }
        if let Some(sound) = &self.sound {
            let glyph = if sound.muted {
                Icon::SoundMuted
            } else {
                Icon::Sound
            };
            items.push(ui::status(glyph, None, tint));
        }
        if let Some(Some(battery)) = &self.battery {
            let glyph = match battery.charge {
                Charge::Charging => Icon::BatteryCharging,
                _ => Icon::Battery,
            };
            let tint = if battery.is_low() { p.amber_text } else { tint };
            let value = strings::percent(battery.percent);
            items.push(ui::status(glyph, Some(value), tint));
        }
        let mut clock_items = Vec::new();
        if let Some(clock) = &self.clock {
            clock_items.push(ui::body(clock.date()).color(tint).into());
            let time = ui::body(clock.time())
                .font(typography::UI_SEMIBOLD)
                .color(p.text_primary);
            clock_items.push(time.into());
        }
        (items, clock_items)
    }

    pub fn view_menu(&self) -> Element<'_, Message> {
        let mut entries = vec![
            ui::menu_header(self.user.as_str(), strings::top_bar::SESSION_OPEN),
            ui::menu_separator(),
        ];
        for (i, action) in Action::ALL.into_iter().enumerate() {
            if action.starts_group() {
                entries.push(ui::menu_separator());
            }
            let mut item = ui::menu_item(
                action.label(),
                action.trailing(),
                Some(Message::Activate(action)),
            );
            if self.cursor == Some(i) {
                item = ui::open(item);
            }
            entries.push(mouse_area(item).on_enter(Message::Hover(i)).into());
        }
        let panel = ui::menu(entries, metrics::MENU_WIDTH).height(Length::Fill);
        let panel = surface::shadowed(panel, metrics::MENU_RADIUS, Elevation::Menu);
        // A click on the shadow, outside the menu, closes it like a click elsewhere, for a
        // compositor that would ignore the input region.
        let surface = container(opaque(panel)).padding(self.menu_margins);
        mouse_area(surface)
            .on_press(Message::MenuKey(menu::Key::Close))
            .into()
    }
}

/// The themes of the appearance file at `path`, live (`appearance_config::follow_themes`).
pub fn appearance(path: &Path) -> UnboundedReceiver<Theme> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let follow = appearance_config::follow_themes(path.to_path_buf(), move |theme| {
        sender.unbounded_send(theme).is_ok()
    });
    if let Err(error) = follow {
        tracing::warn!(%error, "the bar will not follow the appearance file");
    }
    receiver
}

/// The screen of a surface, from iced_exwlshell's reports.
fn on_output(event: ShellEvent) -> Option<Message> {
    match event {
        ShellEvent::WindowOutputChanged { window, output } => Some(Message::Output(
            window,
            output.and_then(|o| o.info.name.clone()),
        )),
        _ => None,
    }
}

/// Reads the history of the notification daemon.
fn load_history() -> Task<Message> {
    Task::perform(
        async { history::list(&bus::session().await?).await },
        |listed| Message::History(listed.map_err(|e| e.to_string())),
    )
}

/// The width of a surface, each time it is configured.
fn on_resize(event: iced::Event, _: iced::event::Status, id: Id) -> Option<Message> {
    match event {
        iced::Event::Window(window::Event::Resized(size)) => Some(Message::Resized(id, size.width)),
        _ => None,
    }
}

/// Any mouse button press, captured by a widget or not.
fn on_pointer(event: iced::Event, _: iced::event::Status, _: Id) -> Option<Message> {
    matches!(
        event,
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
    )
    .then_some(Message::Pointer)
}

/// Tab moves the focus; Enter, Space and Escape go to the menu when it is open, to the focused
/// bar button otherwise.
fn on_key(event: keyboard::Event) -> Option<Message> {
    focus::tab(event.clone())
        .map(Message::Focus)
        .or_else(|| page(&event).map(Message::Page))
        .or_else(|| menu::key(event).map(Message::MenuKey))
}

/// Page Up and Page Down: the calendar's month before and after.
fn page(event: &keyboard::Event) -> Option<bool> {
    match event {
        keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::PageDown),
            ..
        } => Some(true),
        keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::PageUp),
            ..
        } => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_menu() -> (Bar, Id) {
        let id = Id::unique();
        let bar = Bar {
            menu: Some(id),
            ..Bar::default()
        };
        (bar, id)
    }

    #[test]
    fn arrows_and_tab_walk_the_menu() {
        let (mut bar, _) = with_menu();
        let _ = bar.update(Message::MenuKey(menu::Key::Next));
        let _ = bar.update(Message::MenuKey(menu::Key::Next));
        assert_eq!(bar.cursor, Some(1));
        let _ = bar.update(Message::Focus(Direction::Previous));
        assert_eq!(bar.cursor, Some(0));
        let _ = bar.update(Message::MenuKey(menu::Key::Last));
        assert_eq!(bar.cursor, Some(Action::ALL.len() - 1));
    }

    #[test]
    fn enter_activates_the_highlighted_entry_and_closes_the_menu() {
        let (mut bar, _) = with_menu();
        let _ = bar.update(Message::Hover(4));
        let _ = bar.update(Message::MenuKey(menu::Key::Activate));
        assert!(!bar.menu_open());
        assert_eq!(bar.cursor, None);
    }

    #[test]
    fn escape_closes_the_menu() {
        let (mut bar, _) = with_menu();
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        assert!(!bar.menu_open());
    }

    #[test]
    fn menu_keys_do_nothing_while_the_menu_is_closed() {
        let mut bar = Bar::default();
        let _ = bar.update(Message::MenuKey(menu::Key::Next));
        let _ = bar.update(Message::MenuKey(menu::Key::Activate));
        assert_eq!(bar.cursor, None);
        assert!(!bar.menu_open());
    }

    #[test]
    fn the_compositor_can_close_the_menu() {
        let (mut bar, id) = with_menu();
        let _ = bar.update(Message::Hover(2));
        let _ = bar.update(Message::Closed(Id::unique()));
        assert!(bar.menu_open());
        let _ = bar.update(Message::Closed(id));
        assert!(!bar.menu_open());
        assert_eq!(bar.cursor, None);
    }

    fn with_workspaces() -> Bar {
        let mut bar = Bar::default();
        let desktop = Desktop::placeholder(Some("Code,Web,Jeu"), None);
        let _ = bar.update(Message::Desktop(desktop));
        bar
    }

    #[test]
    fn the_first_tab_already_shows_the_ring() {
        let mut bar = with_workspaces();
        assert_eq!(bar.focused(), None);
        let _ = bar.update(Message::Focus(Direction::Next));
        assert_eq!(bar.focused(), Some(Target::Sherlock));
    }

    #[test]
    fn the_ringed_button_is_the_one_enter_activates() {
        let mut bar = with_workspaces();
        let expected = [
            Target::Sherlock,
            Target::Workspace(0),
            Target::Workspace(1),
            Target::Workspace(2),
            Target::Sherlock,
        ];
        for target in expected {
            let _ = bar.update(Message::Focus(Direction::Next));
            // The view rings `focused()`; Enter activates `activation()`: they must agree.
            assert_eq!(bar.focused(), Some(target));
            let activated = bar.activation();
            match target {
                Target::Sherlock => assert!(matches!(activated, Some(Message::ToggleMenu))),
                Target::Workspace(i) => {
                    assert!(matches!(activated, Some(Message::SelectWorkspace(j)) if j == i));
                }
                Target::Status | Target::Clock => panic!("no indicator in this bar"),
            }
        }
        let _ = bar.update(Message::Focus(Direction::Previous));
        assert_eq!(bar.focused(), Some(Target::Workspace(2)));
    }

    #[test]
    fn tab_reaches_the_indicators_and_the_clock_which_open_their_panels() {
        let mut bar = with_workspaces();
        let _ = bar.update(Message::Sound(crate::source::sound::Sound {
            volume: 40,
            muted: false,
        }));
        let _ = bar.update(Message::Clock(LocalTime {
            year: 2026,
            weekday: 2,
            day: 24,
            month: 11,
            hour: 21,
            minute: 46,
        }));
        let _ = bar.update(Message::Focus(Direction::Previous));
        assert_eq!(bar.focused(), Some(Target::Clock));
        assert!(matches!(
            bar.activation(),
            Some(Message::TogglePanel(Panel::Calendar))
        ));
        let _ = bar.update(Message::Focus(Direction::Previous));
        assert_eq!(bar.focused(), Some(Target::Status));
        assert!(matches!(
            bar.activation(),
            Some(Message::TogglePanel(Panel::Control))
        ));
        // Opened, then the other one: one panel at a time, and Escape closes it.
        let _ = bar.update(Message::Surface(Id::unique()));
        let _ = bar.update(Message::TogglePanel(Panel::Calendar));
        assert_eq!(bar.panel_open(), Some(Panel::Calendar));
        assert_eq!(
            bar.month,
            Some(Month {
                year: 2026,
                month: 11
            })
        );
        let _ = bar.update(Message::Page(true));
        assert_eq!(
            bar.month,
            Some(Month {
                year: 2026,
                month: 12
            })
        );
        let _ = bar.update(Message::MenuKey(menu::Key::First));
        assert_eq!(
            bar.month,
            Some(Month {
                year: 2026,
                month: 11
            })
        );
        let _ = bar.update(Message::TogglePanel(Panel::Control));
        assert_eq!(bar.panel_open(), Some(Panel::Control));
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        assert_eq!(bar.panel_open(), None);
    }

    #[test]
    fn enter_on_the_ringed_sherlock_button_opens_the_menu() {
        let mut bar = with_workspaces();
        let _ = bar.update(Message::Surface(Id::unique()));
        let _ = bar.update(Message::Focus(Direction::Next));
        let _ = bar.update(Message::MenuKey(menu::Key::Activate));
        assert!(bar.menu_open());
    }

    #[test]
    fn a_click_clears_the_bar_focus() {
        let mut bar = with_workspaces();
        let _ = bar.update(Message::Focus(Direction::Next));
        assert!(bar.focused().is_some());
        let _ = bar.update(Message::Pointer);
        assert_eq!(bar.focused(), None);
    }

    #[test]
    fn escape_clears_the_bar_focus() {
        let mut bar = with_workspaces();
        let _ = bar.update(Message::Focus(Direction::Previous));
        assert_eq!(bar.focused(), Some(Target::Workspace(2)));
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        assert_eq!(bar.focused(), None);
    }

    /// The only test of this binary that changes the theme: the others draw nothing.
    #[test]
    fn a_new_theme_in_the_file_fades_the_bar_without_restarting() {
        use iced::futures::StreamExt;
        use sherlock_ui::color::Palette;

        let dir = std::env::temp_dir().join(format!("bar-theme-{}", std::process::id()));
        let file = dir.join("appearance.conf");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, "[Appearance]\nTheme=dark\n").unwrap();
        let mut themes = appearance(&file);
        let mut next = || iced::futures::executor::block_on(themes.next()).unwrap();
        let mut bar = with_workspaces();
        let _ = bar.update(Message::Theme(next()));
        assert_eq!(color::palette(), &Palette::DARK);

        std::fs::write(&file, "[Appearance]\nTheme=light\n").unwrap();
        let theme = next();
        assert_eq!(theme, Theme::Light);
        let start = Instant::now();
        let _ = bar.update(Message::Theme(theme));
        assert!(bar.theme.running(), "frames are asked for the fade");
        let _ = bar.update(Message::Frame(start + sherlock_ui::motion::THEME_FADE / 3));
        assert_ne!(color::palette(), &Palette::DARK);
        assert_ne!(color::palette(), &Palette::LIGHT);
        let _ = bar.update(Message::Frame(start + sherlock_ui::motion::THEME_FADE * 2));
        assert!(!bar.theme.running());
        assert_eq!(color::palette(), &Palette::LIGHT);
        color::set_active(Theme::Dark);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_surface_the_menu_does_not_open() {
        let mut bar = Bar::default();
        let _ = bar.update(Message::ToggleMenu);
        assert!(!bar.menu_open());
        let _ = bar.update(Message::Surface(Id::unique()));
        let _ = bar.update(Message::ToggleMenu);
        assert!(bar.menu_open());
    }
}
