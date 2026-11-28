//! Bar state, messages, and its two views: the bar and the Sherlock menu.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::widget::{container, mouse_area, opaque, row};
use iced::window::Id;
use iced::{Alignment, Element, Length, Padding, Subscription, Task, keyboard, window};
use iced_exwlshell::actions::{IcedNewPopupSettings, RegionActionCB};
use iced_exwlshell::reexport::{
    Margin, PixelSize, PopupAnchor, PopupConstraintAdjustment, PopupGravity, Position,
};
use iced_exwlshell::shell::{ShellEvent, ShellReceiver};
use iced_exwlshell::to_layer_message;
use sherlock_ui::animated;
use sherlock_ui::color::{self, Theme, ThemeFade};
use sherlock_ui::component::{self as ui};
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::icon::{self, BatteryMark, Radio, WifiMark};
use sherlock_ui::scroll::scroll;
use sherlock_ui::surface::{self, Elevation};
use sherlock_ui::{metrics, panels, strings, typography};

mod global;
mod pins;
mod tray;

pub use self::tray::{Click, Tray};

use crate::calendar::Month;
use crate::dbusmenu::Item;
use crate::global_menu::{self, GlobalMenu};
use crate::history::{self, Entry};
use crate::live::{self, Link};
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
use crate::source::workspaces::{AppMenu, Desktop, Group};
use crate::unfold::Unfold;
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
    /// The entry of the Sherlock menu waiting for its second click, and since when (D-36d).
    pub armed: Option<(Action, Instant)>,
    /// What the Sherlock menu starts: Control Room.
    pub programs: crate::programs::Programs,
    /// Bar button that has the keyboard focus, as an index in `targets()`. The bar keeps it in
    /// its own state, like the menu cursor: the ring is drawn from the same value that Enter
    /// activates, in the same update. (iced's focus operations are applied after the redraw
    /// under `iced_exwlshell`, which left the ring one Tab behind.)
    focus: Option<usize>,
    /// The theme of the appearance file, shown with a fade.
    pub theme: ThemeFade,
    /// The panel shown, and its surface: open, or folding back before it closes.
    panel: Option<(Panel, Id)>,
    /// The panel unfolding from the bar, or folding back.
    pub unfold: Unfold,
    /// Whether the panels unfold: on the GPU only, where the surface around them is
    /// transparent (in software it is black). `None`: the renderer decides; set by the tests.
    pub panel_motion: Option<bool>,
    /// Room around the panel in its surface: the gap below the bar, and its shadow on the GPU.
    pub panel_margins: Padding,
    /// Whether the popup holds both panels (on the GPU), so that one turns into the other.
    pub panel_shared: bool,
    /// The panel the one shown replaced in the same popup, and the motion between them.
    pub swap: Option<(Panel, Unfold)>,
    /// The Sherlock menu unfolding from its button, or folding back before it closes.
    pub menu_unfold: Unfold,
    /// Width of each surface, as iced_exwlshell reports it: kept whoever comes first, the
    /// surface's size or its id (UI-02a).
    widths: HashMap<Id, f32>,
    /// The logical width of the screen each surface is on, when the compositor gives it.
    screens: HashMap<Id, f32>,
    /// The month the calendar shows.
    pub month: Option<Month>,
    /// What the notification centre shows.
    pub history: History,
    /// The quick settings of the control centre.
    pub quick: Quick,
    /// The Wi-Fi signal, a connection under way, the Internet, from NetworkManager.
    pub link: Option<Link>,
    /// The Bluetooth radio; `None` without an adapter: no indicator.
    pub radio: Option<Radio>,
    /// The menus of the active application (lot D-36).
    pub global: GlobalMenu,
    /// The name of each application by its `app_id`, from its desktop entry: `None` while it
    /// is read, or when it has none.
    pub app_names: HashMap<String, Option<String>>,
    /// The logical height of the screen each surface is on, when the compositor gives it.
    heights: HashMap<Id, f32>,
    /// The notification area: its icons, its chevron and its popup (lot D-45).
    pub tray: Tray,
}

/// An indicator of the bar. Together they make one button, which opens the control centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indicator {
    Bluetooth,
    Network,
    Sound,
    Battery,
}

/// A bar button that Tab can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Sherlock,
    /// A menu of the active application, by its id (lot D-36).
    AppMenu(i32),
    Workspace(usize),
    /// The chevron of the notification area (lot D-45).
    Tray,
    /// The indicators, one button: the control centre.
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
    /// The screen a surface is on: its name and its logical width and height.
    Output(Id, Option<String>, Option<(f32, f32)>),
    /// The network as NetworkManager reports it on the system bus.
    Link(Option<Link>),
    /// The Bluetooth radio as BlueZ reports it.
    Radio(Option<Radio>),
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
    /// The name of the application of this `app_id`, from its desktop entry.
    AppName(String, Option<String>),
    /// The tree of the application's menus under an id, read, or why not.
    AppMenuRead(AppMenu, i32, Result<Item, String>),
    /// The application says its menus changed.
    AppMenuChanged(AppMenu),
    /// A menu of the bar clicked: it opens, or closes.
    OpenAppMenu(i32),
    /// A menu of the bar opened from the keyboard: its first entry highlighted.
    OpenAppMenuByKey(i32),
    /// The pointer enters a menu of the bar.
    HoverAppMenu(i32),
    /// The pointer over an entry: the menu's level, the entry's index.
    HoverEntry(usize, usize),
    /// An entry clicked: the menu's level, the entry's index.
    PickEntry(usize, usize),
    /// The application got the click on an entry, or why not.
    EntrySent(Result<(), String>),
    /// A click on the shadow of a menu.
    CloseAppMenus,
    /// The notification area learnt something (lot D-45).
    Tray(crate::tray::Event),
    /// The chevron clicked: the popup opens, or closes.
    ToggleTray,
    /// Where the chevron is drawn in the bar surface.
    TrayChevron(iced::Rectangle),
    /// A button of the mouse on the icon of this key.
    TrayClick(String, Click),
    /// The wheel over the icon of this key.
    TrayScroll(String, iced::mouse::ScrollDelta),
    /// The pointer enters the icon of this key, or leaves it.
    TrayHover(Option<String>),
    TrayLeave(String),
    /// The application got the click or the wheel, or why not; the item whose menu a refused
    /// `Activate` opens.
    TraySent(Option<String>, Result<(), String>),
    /// Where the icons kept in the bar are drawn (lot D-45a).
    TrayPinnedRow(iced::Rectangle),
    /// A button or a move of the pointer over a surface of the bar.
    TrayPointer(Id, iced::mouse::Event),
    /// The icons kept in the bar, as the settings file has them now.
    Pins(Vec<String>),
    /// The settings file could not take the icons kept in the bar: why.
    PinsNotSaved(String),
}

impl Bar {
    /// The bar, told of the screens of its surfaces by `shell_events`.
    pub fn new(shell_events: ShellReceiver) -> (Self, Task<Message>) {
        let session = launcher::system::Systemd::from_env();
        let bar = Self {
            user: user::current(),
            programs: crate::programs::Programs(std::sync::Arc::new(crate::programs::Spawn)),
            shell_events: Some(shell_events),
            quick: Quick::new(std::sync::Arc::new(session)),
            ..Self::default()
        };
        (bar, pins::load())
    }

    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// The panel open, if any: not one folding back.
    pub fn panel_open(&self) -> Option<Panel> {
        self.panel
            .filter(|_| !self.unfold.folding())
            .map(|(panel, _)| panel)
    }

    /// The surface of the panel shown, open or folding back.
    pub fn panel_surface(&self) -> Option<Id> {
        self.panel.map(|(_, id)| id)
    }

    /// Something moves: frames are needed. At rest, none.
    pub fn animating(&self) -> bool {
        let swapping = self.swap.is_some_and(|(_, motion)| motion.running());
        self.theme.running()
            || self.shape.running()
            || (self.menu.is_some() && self.menu_unfold.running())
            || self.global.running()
            || self.tray.running()
            || (self.panel.is_some() && (self.unfold.running() || self.quick.running() || swapping))
    }

    fn animates(&self) -> bool {
        self.panel_motion
            .unwrap_or_else(|| surface::gpu_drawn() == Some(true))
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let appearance = match appearance_config::path() {
            Some(path) => Subscription::run_with(path, |path| appearance(path)).map(Message::Theme),
            None => Subscription::none(),
        };
        let frames = if self.animating() {
            window::frames().map(Message::Frame)
        } else {
            Subscription::none()
        };
        let history = match self.panel_open() {
            Some(Panel::Calendar) => {
                Subscription::run(history::watch).map(|()| Message::HistoryChanged)
            }
            Some(Panel::Control) => Subscription::run(quick::watch)
                .map(|service| Message::Quick(quick::Message::Changed(service))),
            None => Subscription::none(),
        };
        let app_menu = match &self.global.address {
            Some(menu) => {
                Subscription::run_with(menu.clone(), watch_menu).map(Message::AppMenuChanged)
            }
            None => Subscription::none(),
        };
        Subscription::batch([
            appearance,
            frames,
            history,
            app_menu,
            Subscription::run(|| watch(SystemClock::default(), "clock")).map(Message::Clock),
            Subscription::run(|| watch(UPower::default(), "battery")).map(Message::Battery),
            Subscription::run(|| watch(NetworkManager::default(), "network")).map(Message::Network),
            Subscription::run(|| watch(Pactl::default(), "sound")).map(Message::Sound),
            Subscription::run(live::link).map(Message::Link),
            Subscription::run(live::bluetooth).map(Message::Radio),
            Subscription::run(|| watch(Auto::default(), "workspaces")).map(Message::Desktop),
            Subscription::run(crate::tray::watch).map(Message::Tray),
            self.shell_events
                .as_ref()
                .map_or_else(Subscription::none, |events| {
                    events.listen().filter_map(on_output)
                }),
            window::close_events().map(Message::Closed),
            keyboard::listen().filter_map(on_key),
            iced::event::listen_with(on_pointer),
            iced::event::listen_with(pins::on_pointer),
            iced::event::listen_with(on_resize),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tray(event) => return self.tray_event(event),
            Message::ToggleTray => return self.toggle_tray(),
            Message::TrayChevron(bounds) => self.tray.chevron = Some(bounds),
            Message::TrayClick(key, click) => return self.tray_click(&key, click),
            Message::TrayScroll(key, delta) => return self.tray_scroll(&key, delta),
            Message::TrayHover(key) => self.tray.hovered = key,
            Message::TrayLeave(key) if self.tray.hovered.as_ref() == Some(&key) => {
                self.tray.hovered = None;
            }
            Message::TrayLeave(_) => {}
            Message::TraySent(Some(key), Err(error)) if error.contains("UnknownMethod") => {
                return self.tray_menu(&key);
            }
            Message::TraySent(_, Err(error)) => {
                tracing::warn!(%error, "the application did not get the click on its icon");
            }
            Message::TraySent(_, Ok(())) => {}
            Message::TrayPinnedRow(bounds) => self.tray.pinned_row = Some(bounds),
            Message::TrayPointer(id, event) => {
                let task = self.tray_pointer(id, event);
                self.follow_chevron();
                return task;
            }
            Message::PinsNotSaved(error) => {
                tracing::warn!(%error, "the icons kept in the bar are not saved");
            }
            Message::Pins(pins) => {
                self.tray.pins = pins;
                self.follow_chevron();
                return self.resize_tray();
            }
            // Escape closes the popup of the notification area, once its menu is closed.
            Message::MenuKey(menu::Key::Close) if self.tray_open() && !self.app_menu_open() => {
                return self.close_tray();
            }
            // An entry of an icon's menu that runs closes the popup too.
            Message::PickEntry(level, index)
                if self.global.item.is_some() && self.picked(level, index).is_some() =>
            {
                return self.pick_entry(level, index).chain(self.close_tray());
            }
            Message::Surface(id) => self.surface = Some(id),
            Message::Clock(time) => self.clock = Some(time),
            Message::Battery(battery) => self.battery = Some(battery),
            Message::Network(network) => self.network = Some(network),
            Message::Sound(sound) => self.sound = Some(sound),
            Message::Link(link) => self.link = link,
            Message::Radio(radio) => self.radio = radio,
            Message::Desktop(desktop) => {
                self.desktop = Some(desktop);
                return self.reshape(Instant::now()).chain(self.follow_app_menu());
            }
            Message::Output(id, name, size) => {
                self.outputs.insert(id, name);
                match size {
                    Some((width, height)) => {
                        self.screens.insert(id, width);
                        self.heights.insert(id, height);
                    }
                    None => {
                        self.screens.remove(&id);
                        self.heights.remove(&id);
                    }
                };
                return self.reshape(Instant::now());
            }
            Message::ToggleMenu => return self.toggle_menu(),
            Message::TogglePanel(panel) => return self.toggle_panel(panel),
            Message::ClosePanel => return self.close_panel(),
            Message::Resized(id, width) => {
                self.widths.insert(id, width);
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
            Message::HistoryChanged if self.panel_open() == Some(Panel::Calendar) => {
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
            Message::Quick(quick::Message::OpenNetworkSettings) => {
                // Control Room's network page, in its window if it is open (lot D-26n).
                let (program, args) =
                    (crate::programs::CONTROL_ROOM, crate::programs::NETWORK_PAGE);
                self.programs.start_with(program, &args);
                return self.close_panel();
            }
            Message::Quick(message) => {
                self.quick.now = animated::clock();
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
            Message::MenuKey(menu::Key::Close) if self.panel_open().is_some() => {
                return self.close_panel();
            }
            Message::MenuKey(menu::Key::First) if self.panel_open() == Some(Panel::Calendar) => {
                self.month = panel::today(self.clock.as_ref()).map(Month::of);
            }
            Message::MenuKey(_) if self.panel_open().is_some() => {}
            Message::Focus(direction) if self.panel_open().is_some() => {
                return focus::advance(direction);
            }
            Message::MenuKey(key) if self.app_menu_open() => return self.app_menu_key(key),
            Message::Focus(direction) if self.app_menu_open() => {
                let key = match direction {
                    Direction::Next => menu::Key::Next,
                    Direction::Previous => menu::Key::Previous,
                };
                return self.app_menu_key(key);
            }
            Message::AppName(id, name) => {
                self.app_names.insert(id, name);
            }
            Message::AppMenuRead(menu, id, read) => return self.app_menu_read(menu, id, read),
            Message::AppMenuChanged(menu) if self.global.address.as_ref() == Some(&menu) => {
                return global::read_menu(menu, 0);
            }
            Message::AppMenuChanged(_) => {}
            Message::OpenAppMenu(id) => return self.toggle_app_menu(id),
            Message::OpenAppMenuByKey(id) => return self.show_app_menu(id, true),
            Message::HoverAppMenu(id) => return self.hover_app_menu(id),
            Message::HoverEntry(level, index) => return self.hover_entry(level, index),
            Message::PickEntry(level, index) => return self.pick_entry(level, index),
            Message::EntrySent(Err(error)) => {
                tracing::warn!(%error, "the application did not get the menu entry");
            }
            Message::EntrySent(Ok(())) => {}
            Message::CloseAppMenus => return self.close_app_menus(),
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
            // An entry that does nothing yet takes no highlight.
            Message::Hover(index) => {
                let usable = Action::ALL.get(index).is_some_and(|a| a.enabled());
                self.cursor = usable.then_some(index);
            }
            Message::Activate(action) => return self.activate(action),
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
                let at = animated::at(now);
                self.unfold.tick(at);
                self.menu_unfold.tick(at);
                self.quick.tick(at);
                if let Some((_, motion)) = &mut self.swap {
                    motion.tick(at);
                    if !motion.running() {
                        self.swap = None;
                    }
                }
                let mut tasks = self.apply_shape();
                if self.unfold.folded() {
                    tasks = tasks.chain(self.drop_panel());
                }
                if self.menu_unfold.folded() {
                    tasks = tasks.chain(self.drop_menu());
                }
                return tasks
                    .chain(self.tick_app_menus(at))
                    .chain(self.tick_tray(at));
            }
            Message::Closed(id) => {
                // The compositor closes the menu and the panels on a click elsewhere
                // (popup_done).
                self.outputs.remove(&id);
                self.app_menu_closed(id);
                self.tray_closed(id);
                if self.menu == Some(id) {
                    self.menu = None;
                    self.cursor = None;
                    self.menu_unfold = Unfold::default();
                }
                if self.panel_surface() == Some(id) {
                    self.panel = None;
                    self.unfold = Unfold::default();
                    return self.quick.close().map(Message::Quick);
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
        let status = self.radio.is_some()
            || self.network.is_some()
            || self.sound.is_some()
            || matches!(self.battery, Some(Some(_)));
        let app_menus = match self.global.root {
            Some(_) => global_menu::GlobalMenu::buttons(&self.placement()),
            None => Vec::new(),
        };
        std::iter::once(Target::Sherlock)
            .chain(app_menus.into_iter().map(Target::AppMenu))
            .chain((0..workspaces).map(Target::Workspace))
            .chain(self.tray.has_icons().then_some(Target::Tray))
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
            Target::AppMenu(id) => Message::OpenAppMenuByKey(id),
            Target::Workspace(i) => Message::SelectWorkspace(i),
            Target::Tray => Message::ToggleTray,
            Target::Status => Message::TogglePanel(Panel::Control),
            Target::Clock => Message::TogglePanel(Panel::Calendar),
        })
    }

    /// Opens `panel` under the right end of the bar, or closes it when it is open. Another
    /// panel or the menu closes first.
    ///
    /// Clicked again while it folds back, it unfolds again from where it is, in the same
    /// popup: a quick series of clicks never piles popups up nor closes one twice.
    fn toggle_panel(&mut self, which: Panel) -> Task<Message> {
        let now = animated::clock();
        match self.panel {
            Some((shown, _)) if shown == which && !self.unfold.folding() => {
                return self.close_panel();
            }
            Some((shown, _)) if shown == which => {
                self.unfold.fold(now, true);
                return self.drop_menu().chain(self.opened(which));
            }
            // The other panel, open in a popup that holds both: it turns into this one.
            Some((shown, id)) if self.panel_shared && !self.unfold.folding() => {
                return self.swap_to(shown, which, id, now);
            }
            _ => {}
        }
        // The menu goes at once: a popup folding back would still hold the grab the panel's
        // popup takes.
        let closed = self
            .drop_panel()
            .chain(self.drop_menu())
            .chain(self.drop_app_menus())
            .chain(self.drop_tray());
        let Some(surface) = self.surface else {
            tracing::warn!("bar surface unknown: the panel cannot open");
            return closed;
        };
        self.panel_margins = panel::margins(surface::gpu_drawn() == Some(true));
        // Room for the keyboard backlight only on a machine that has one (UI-04), and a height
        // within the screen (UI-08).
        panel::reserve_keyboard(self.quick.has_keyboard());
        panel::reserve_screen(self.screen_height());
        self.panel_shared = self.animates();
        self.swap = None;
        let shared = self.panel_shared;
        let (width, height) = panel::surface_size(which, self.panel_margins, shared);
        let (x, y, anchor_width, anchor_height) = self.panel_anchor(which);
        let settings = IcedNewPopupSettings::new(
            surface,
            PixelSize::px(width, height),
            Position::new(x, y),
            PixelSize::px(anchor_width, anchor_height),
        )
        .anchor(PopupAnchor::Bottom)
        .gravity(PopupGravity::Bottom)
        // Kept on the screen by the compositor, sliding along its edges; never flipped above
        // the bar (UI-08).
        .constraint_adjustment(PopupConstraintAdjustment::SlideX | PopupConstraintAdjustment::SlideY);
        let id = Id::unique();
        self.panel = Some((which, id));
        self.unfold = if self.animates() {
            Unfold::opening(now)
        } else {
            Unfold::default()
        };
        closed
            .chain(Task::done(Message::NewPopUp { settings, id }))
            .chain(self.input_region(which, id))
            .chain(self.opened(which))
    }

    /// Limits the input of the popup `id` to `which`: the rest of its room lets the clicks
    /// through to what is under it.
    fn input_region(&self, which: Panel, id: Id) -> Task<Message> {
        let (x, y, width, height) = panel::input_rect(which, self.panel_margins, self.panel_shared);
        let region = RegionActionCB::new(move |region| {
            if let Err(error) = region.add(x, y, width, height) {
                tracing::warn!(%error, "cannot limit the panel's input region");
            }
        });
        Task::done(Message::SetInputRegion {
            id,
            callback: region,
        })
    }

    /// `shown` turns into `which` in the same popup `id`: `shown` slides out and fades while
    /// `which` comes in from the other side, a mask going from the height of one to the
    /// other's (UI-01). What `shown` read stops, what `which` reads starts.
    fn swap_to(&mut self, shown: Panel, which: Panel, id: Id, now: Duration) -> Task<Message> {
        let stopped = match shown {
            Panel::Control => self.quick.close().map(Message::Quick),
            Panel::Calendar => Task::none(),
        };
        self.swap = Some((shown, Unfold::opening(now)));
        self.panel = Some((which, id));
        stopped
            .chain(self.drop_menu())
            .chain(self.input_region(which, id))
            .chain(self.opened(which))
    }

    /// The width of the bar surface: as reported for it, else its screen's width less its
    /// margins. Only without either the reference screen's, with a warning: a panel placed
    /// from a width that is not the bar's lands far from its button (UI-02a).
    pub fn bar_width(&self) -> f32 {
        let surface = self.surface;
        let reported = surface.and_then(|id| self.widths.get(&id).copied());
        let side = self.applied.margin_side as f32;
        let screen = surface
            .and_then(|id| self.screens.get(&id).copied())
            .map(|width| width - 2.0 * side);
        reported.or(screen).unwrap_or_else(|| {
            tracing::warn!("width of the bar unknown: panels placed for the reference screen");
            metrics::REFERENCE_SCREEN.0 - 2.0 * metrics::TOP_BAR_MARGIN_SIDE
        })
    }

    /// The logical height of the bar's screen, when the compositor gives it.
    pub fn screen_height(&self) -> Option<f32> {
        self.surface.and_then(|id| self.heights.get(&id).copied())
    }

    /// The anchor rectangle of `which` in the bar surface, from the bar's real width: the
    /// panel's right edge where the bar ends, under the indicators and the clock, whatever
    /// the size and the scale of the screen.
    pub fn panel_anchor(&self, which: Panel) -> (i32, i32, u32, u32) {
        let side = self.applied.margin_side as f32;
        panel::anchor_rect(
            which,
            self.panel_margins,
            self.bar_width(),
            side,
            self.panel_shared,
        )
    }

    /// What a panel reads when it opens.
    fn opened(&mut self, which: Panel) -> Task<Message> {
        match which {
            Panel::Control => {
                self.quick.now = animated::clock();
                self.quick.open().map(Message::Quick)
            }
            Panel::Calendar => {
                self.month = panel::today(self.clock.as_ref()).map(Month::of);
                self.history = History::Loading;
                load_history()
            }
        }
    }

    /// Closes the panel: it folds back first where it unfolds, and its popup closes once it
    /// has faded out (`Message::Frame`).
    fn close_panel(&mut self) -> Task<Message> {
        let stopped = self.quick.close().map(Message::Quick);
        if self.panel.is_some() && self.animates() {
            self.unfold.fold(animated::clock(), false);
            return stopped;
        }
        stopped.chain(self.drop_panel())
    }

    /// Closes the popup of the panel at once, whatever it was doing.
    fn drop_panel(&mut self) -> Task<Message> {
        let stopped = self.quick.close().map(Message::Quick);
        self.unfold = Unfold::default();
        match self.panel.take() {
            Some((_, id)) => stopped.chain(window::close(id)),
            None => stopped,
        }
    }

    fn toggle_menu(&mut self) -> Task<Message> {
        if self.menu_open() && !self.menu_unfold.folding() {
            return self.close_menu();
        }
        // Clicked again while it folds back: it unfolds again in the same popup.
        if self.menu_open() {
            self.menu_unfold.fold(animated::clock(), true);
            return Task::none();
        }
        let closed = self
            .drop_panel()
            .chain(self.drop_app_menus())
            .chain(self.drop_tray());
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
        self.menu_unfold = if self.animates() {
            Unfold::opening(animated::clock())
        } else {
            Unfold::default()
        };
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

    /// Closes the menu: it folds back first where it unfolds, and its popup closes once it has
    /// faded out (`Message::Frame`).
    /// Runs an entry of the Sherlock menu (D-36d). Logging out and powering off want a second
    /// click, at least `CONFIRM_DELAY` later, the entry reading « Reclique pour confirmer »
    /// meanwhile: the menu stays open for it, as a Mac asks before it powers off.
    pub fn activate(&mut self, action: Action) -> Task<Message> {
        use launcher::source::Power;
        if !action.enabled() {
            return Task::none();
        }
        if action.needs_confirmation() {
            let delay = sherlock_ui::launcher::CONFIRM_DELAY;
            match self.armed {
                Some((armed, at)) if armed == action && at.elapsed() >= delay => {}
                // A double click only arms.
                Some((armed, _)) if armed == action => return Task::none(),
                _ => {
                    self.armed = Some((action, Instant::now()));
                    return Task::none();
                }
            }
        }
        self.armed = None;
        let ran = match action {
            Action::About | Action::Settings => {
                // Control Room has no « À propos » page yet: both open its first page.
                self.programs.start(crate::programs::CONTROL_ROOM);
                Task::none()
            }
            Action::Lock => self
                .quick
                .update(quick::Message::Session(Power::Lock))
                .map(Message::Quick),
            Action::LogOut => {
                self.quick.run_now(Power::LogOut);
                Task::none()
            }
            Action::PowerOff => {
                self.quick.run_now(Power::PowerOff);
                Task::none()
            }
            Action::Updates | Action::GameMode => Task::none(),
        };
        ran.chain(self.close_menu())
    }

    fn close_menu(&mut self) -> Task<Message> {
        self.armed = None;
        self.cursor = None;
        if self.menu.is_some() && self.animates() {
            self.menu_unfold.fold(animated::clock(), false);
            return Task::none();
        }
        self.drop_menu()
    }

    /// Closes the popup of the menu at once.
    fn drop_menu(&mut self) -> Task<Message> {
        self.armed = None;
        self.cursor = None;
        self.menu_unfold = Unfold::default();
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
        if let Some(level) = self.global.level_of(id) {
            return self.view_app_menu(level);
        }
        if self.tray.popup == Some(id) {
            let held = self.tray_dragged_from(pins::Place::Popup);
            return sherlock_ui::pointer::held(self.view_tray(), held).into();
        }
        if self.menu == Some(id) {
            self.view_menu()
        } else {
            let held = self.tray_dragged_from(pins::Place::Bar);
            sherlock_ui::pointer::held(self.view_bar(), held).into()
        }
    }

    /// A panel in its surface, as it unfolds or folds back, or turns into the other one.
    pub fn view_panel(&self, which: Panel) -> Element<'_, Message> {
        let room = panel::room(which, self.panel_shared);
        let coming = self.swap.map_or(1.0, |(_, motion)| motion.opacity());
        let content: Element<'_, Message> = color::with_opacity(self.unfold.opacity(), || {
            let (leaving, coming) = match self.swap {
                Some((leaving, motion)) => {
                    // From the indicators to the clock the new panel comes from the right,
                    // from the clock to the indicators from the left, as they sit in the bar.
                    let side = if leaving == Panel::Control { 1.0 } else { -1.0 };
                    let progress = motion.settled();
                    // The mask goes from one panel's height to the other's, the room for the
                    // shadow below it kept.
                    let height = leaving.size().1
                        + (which.size().1 - leaving.size().1) * progress
                        + self.panel_margins.bottom;
                    let slide = panels::SWITCH_SLIDE;
                    let out = color::with_opacity(1.0 - coming, || self.panel_view(leaving));
                    let out = animated::reveal(panel::place(out, room))
                        .opacity(1.0 - coming)
                        .offset(-side * progress * slide, 0.0)
                        .clip_height(height)
                        .inert();
                    let shown = color::with_opacity(coming, || self.panel_view(which));
                    let shown = animated::reveal(panel::place(shown, room))
                        .opacity(coming)
                        .offset(side * (1.0 - progress) * slide, 0.0)
                        .clip_height(height);
                    (out.into(), shown)
                }
                None => (
                    // A stack takes the size of its first layer: the room.
                    iced::widget::space().width(room.0).height(room.1).into(),
                    animated::reveal(panel::place(self.panel_view(which), room)),
                ),
            };
            let layers: [Element<'_, Message>; 2] = [leaving, coming.into()];
            iced::widget::Stack::with_children(layers).into()
        });
        panel::frame(self.unfold.panel(content, (1.0, 0.0)), self.panel_margins)
    }

    /// The glass of `which`, without its motion.
    fn panel_view(&self, which: Panel) -> Element<'_, Message> {
        match which {
            Panel::Control => {
                let battery = self.battery.flatten();
                let quick = self.quick.view(&self.user, battery).map(Message::Quick);
                let (width, height) = which.size();
                let (_, full) = which.content_size();
                if full <= height {
                    return panel::glass(quick, width, height);
                }
                // A screen too short for it: the settings keep their height and scroll (UI-08).
                let inset = 2.0 * (panels::PADDING + metrics::BORDER_WIDTH);
                let quick = container(quick).height(full - inset);
                panel::glass(scroll(quick).height(Length::Fill), width, height)
            }
            Panel::Calendar => {
                let today = panel::today(self.clock.as_ref());
                let month = self.month.or_else(|| today.map(Month::of));
                let calendar = match month {
                    Some(month) => panel::calendar(month, today),
                    None => iced::widget::space().into(),
                };
                let width = panels::CENTER_WIDTH;
                let both = iced::widget::column![
                    panel::glass(calendar, width, panels::CALENDAR_HEIGHT),
                    panel::glass(panel::center(&self.history), width, panel::center_height()),
                ]
                .spacing(panels::PANEL_GAP);
                let (_, height) = which.size();
                if which.content_size().1 <= height {
                    return both.into();
                }
                // Too short for both: the calendar and the centre scroll together (UI-08).
                scroll(both).width(width).height(height).into()
            }
        }
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
        if let Some(app) = self.app_name() {
            let (pad_left, pad_right) = metrics::APP_NAME_PADDING;
            // A long name ends with « … »: where the left half of the bar ends, or at
            // APP_NAME_MAX when the application's menus follow it.
            let name = ui::ellipsis(app).font(typography::UI_SEMIBOLD);
            let mut name = container(name).padding(Padding {
                left: pad_left,
                right: pad_right,
                ..Padding::ZERO
            });
            if self.global.root.is_some() {
                let placement = self.placement();
                name = name.width(placement.name);
                left = left.push(name);
                for button in self.view_app_menus(&placement) {
                    left = left.push(button);
                }
            } else {
                left = left.push(name);
            }
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
        // The blur behind its glass (lot D-37).
        let bar = sherlock_ui::glass::glass(bar, shape.radius);
        // Flat: no shadow, but drawing it tells the menu which renderer draws the bar.
        surface::shadowed(bar, shape.radius, Elevation::Flat).into()
    }

    /// The indicators, one button with one pill around them all, which opens the control centre
    /// (UI-02: one by one they opened the same panel), then the date and time, which open the
    /// calendar: ghost buttons, whose padding the gaps give back.
    fn view_indicators(
        &self,
        ring: &impl Fn(ui::Control<'static, Message>, Target) -> ui::Control<'static, Message>,
    ) -> Element<'static, Message> {
        let gap = metrics::STATUS_GAP - 2.0 * panels::BAR_BUTTON_PADDING;
        let mut buttons = row![].spacing(gap.max(0.0)).align_y(Alignment::Center);
        if let Some(pinned) = self.view_pinned() {
            buttons = buttons.push(pinned);
        }
        if let Some(chevron) = self.view_chevron() {
            buttons = buttons.push(chevron);
        }
        let status: Vec<_> = self
            .indicators()
            .into_iter()
            .map(|(_, content)| content)
            .collect();
        if !status.is_empty() {
            let line = row(status)
                .spacing(metrics::STATUS_GAP)
                .align_y(Alignment::Center);
            let open = self.panel_open() == Some(Panel::Control);
            let toggle = Some(Message::TogglePanel(Panel::Control));
            let button = panels::bar_button(line, open, toggle);
            buttons = buttons.push(ring(button, Target::Status));
        }
        if let Some(clock) = self.clock.as_ref() {
            let p = color::palette();
            let line = row![
                ui::body(clock.date()).color(p.text_secondary),
                ui::body(clock.time())
                    .font(typography::UI_SEMIBOLD)
                    .color(p.text_primary),
            ]
            .spacing(metrics::STATUS_GAP)
            .align_y(Alignment::Center);
            let open = self.panel_open() == Some(Panel::Calendar);
            let toggle = Some(Message::TogglePanel(Panel::Calendar));
            buttons = buttons.push(ring(panels::bar_button(line, open, toggle), Target::Clock));
        }
        buttons.into()
    }

    /// Bluetooth, network, sound and battery, each with its indicator, drawn from their live
    /// state and faded from one level to the next. One not read yet is not shown.
    fn indicators(&self) -> Vec<(Indicator, Element<'static, Message>)> {
        let p = color::palette();
        let tint = p.text_secondary;
        let glyph = |frames| ui::live_icon(frames, tint);
        let mut items = Vec::new();
        if let Some(radio) = self.radio {
            items.push((
                Indicator::Bluetooth,
                glyph(vec![icon::bluetooth_svg(radio)]),
            ));
        }
        if let Some(network) = &self.network {
            items.push((
                Indicator::Network,
                ui::live_icon_sized(
                    network_frames(network, self.link),
                    metrics::STATUS_ICON_WIDE,
                    tint,
                ),
            ));
        }
        if let Some(sound) = &self.sound {
            let level = icon::sound_level(u32::from(sound.volume));
            items.push((
                Indicator::Sound,
                glyph(vec![icon::sound_svg(level, sound.muted)]),
            ));
        }
        if let Some(Some(battery)) = &self.battery {
            let mark = match battery.charge {
                Charge::Charging => BatteryMark::Charging,
                Charge::Full | Charge::Idle => BatteryMark::Plugged,
                Charge::Discharging => BatteryMark::None,
            };
            let tint = if battery.is_low() { p.amber_text } else { tint };
            let value = strings::percent(battery.percent);
            let gauge = ui::battery(battery.percent, mark, Some(value), tint);
            items.push((Indicator::Battery, gauge));
        }
        items
    }

    pub fn view_menu(&self) -> Element<'_, Message> {
        // Built inside the fade, for the colors its texts take when they are built.
        let panel = color::with_opacity(self.menu_unfold.opacity(), || {
            let mut entries = vec![
                ui::menu_header(self.user.as_str(), strings::top_bar::SESSION_OPEN),
                ui::menu_separator(),
            ];
            for (i, action) in Action::ALL.into_iter().enumerate() {
                if action.starts_group() {
                    entries.push(ui::menu_separator());
                }
                let armed = self.armed.is_some_and(|(a, _)| a == action);
                let label = if armed {
                    panels::text::CONFIRM
                } else {
                    action.label()
                };
                let press = action.enabled().then_some(Message::Activate(action));
                let mut item = ui::menu_item(label, action.trailing(), press);
                if self.cursor == Some(i) || armed {
                    item = ui::open(item);
                }
                entries.push(mouse_area(item).on_enter(Message::Hover(i)).into());
            }
            let panel = ui::menu(entries, metrics::MENU_WIDTH).height(Length::Fill);
            opaque(surface::shadowed(
                panel,
                metrics::MENU_RADIUS,
                Elevation::Menu,
            ))
        });
        // It unfolds from its top left corner, under the Sherlock button.
        let panel = self.menu_unfold.panel(panel, (0.0, 0.0));
        // A click on the shadow, outside the menu, closes it like a click elsewhere, for a
        // compositor that would ignore the input region.
        let surface = container(panel).padding(self.menu_margins);
        mouse_area(surface)
            .on_press(Message::MenuKey(menu::Key::Close))
            .into()
    }
}

/// The Wi-Fi icon of the bar: its arcs by the strength of the signal, a slash when there is no
/// network, an exclamation mark without the Internet, and the arcs lit in turn while
/// NetworkManager connects. A wired network shows every arc; without NetworkManager on the
/// system bus, so does a Wi-Fi one.
pub fn network_frames(network: &Network, link: Option<Link>) -> Vec<String> {
    let link = link.unwrap_or_default();
    if link.connecting {
        return (0..=icon::LEVELS)
            .map(|level| icon::wifi_svg(level, WifiMark::None))
            .collect();
    }
    let mark = if link.no_internet {
        WifiMark::NoInternet
    } else {
        WifiMark::None
    };
    let document = match network {
        Network::Disconnected => icon::wifi_svg(0, WifiMark::Off),
        Network::Connected { .. } => {
            let level = link.strength.map_or(icon::LEVELS, icon::wifi_level);
            icon::wifi_svg(level, mark)
        }
    };
    vec![document]
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
        ShellEvent::WindowOutputChanged { window, output } => {
            let name = output.as_ref().and_then(|o| o.info.name.clone());
            let size = output
                .as_ref()
                .and_then(|o| o.info.logical_size)
                .map(|(width, height)| (width as f32, height as f32));
            Some(Message::Output(window, name, size))
        }
        _ => None,
    }
}

/// The changes of the application's menus at `menu`, a burst at a time.
fn watch_menu(menu: &AppMenu) -> iced::futures::stream::BoxStream<'static, AppMenu> {
    use iced::futures::StreamExt;
    let changed = menu.clone();
    let changes = crate::dbusmenu::watch(menu.clone()).map(move |()| changed.clone());
    bus::coalesce(changes.boxed(), bus::SIGNAL_GAP)
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

    /// What the menu ran: programs started and session commands.
    #[derive(Default)]
    struct Ran(std::sync::Mutex<Vec<String>>);

    impl crate::programs::Start for Ran {
        fn start(&self, program: &str) -> std::io::Result<()> {
            self.0.lock().unwrap().push(program.to_owned());
            Ok(())
        }
    }

    impl launcher::system::Session for Ran {
        fn run(&self, power: launcher::source::Power) -> Result<(), launcher::launch::LaunchError> {
            self.0.lock().unwrap().push(format!("{power:?}"));
            Ok(())
        }
    }

    /// The menu open, its programs and session commands recorded instead of run.
    fn recorded() -> (Bar, std::sync::Arc<Ran>) {
        let ran = std::sync::Arc::new(Ran::default());
        let (mut bar, _) = with_menu();
        bar.programs = crate::programs::Programs(ran.clone());
        bar.quick = Quick::new(ran.clone());
        (bar, ran)
    }

    fn ran(record: &Ran) -> Vec<String> {
        record.0.lock().unwrap().clone()
    }

    #[test]
    fn settings_and_about_start_control_room() {
        for action in [Action::Settings, Action::About] {
            let (mut bar, record) = recorded();
            let _ = bar.update(Message::Activate(action));
            assert_eq!(ran(&record), ["control-room"]);
            assert!(!bar.menu_open(), "the menu closes");
        }
        // The compositor's Super + comma starts the same program (lot Q-31).
        assert_eq!(crate::programs::CONTROL_ROOM, "control-room");
        assert_eq!(
            Action::Settings.trailing(),
            sherlock_ui::component::Trailing::Shortcut("Super+,")
        );
    }

    #[test]
    fn a_network_joined_from_the_settings_opens_its_page() {
        let (mut bar, record) = recorded();
        let _ = bar.update(Message::Quick(quick::Message::OpenNetworkSettings));
        assert_eq!(ran(&record), ["control-room"]);
    }

    #[test]
    fn lock_runs_at_once() {
        let (mut bar, record) = recorded();
        let _ = bar.update(Message::Activate(Action::Lock));
        assert_eq!(ran(&record), ["Lock"]);
        assert!(!bar.menu_open());
    }

    #[test]
    fn logging_out_and_powering_off_ask_a_second_click() {
        for (action, command) in [(Action::LogOut, "LogOut"), (Action::PowerOff, "PowerOff")] {
            let (mut bar, record) = recorded();
            // The first click arms the entry, which then asks to click again; the menu stays.
            let _ = bar.update(Message::Activate(action));
            assert!(ran(&record).is_empty() && bar.menu_open());
            assert_eq!(bar.armed.map(|(a, _)| a), Some(action));
            // A double click only arms.
            let _ = bar.update(Message::Activate(action));
            assert!(ran(&record).is_empty());
            // Clicked again later: it runs, once, and the menu closes.
            let earlier = Instant::now() - sherlock_ui::launcher::CONFIRM_DELAY;
            bar.armed = Some((action, earlier));
            let _ = bar.update(Message::Activate(action));
            assert_eq!(ran(&record), [command]);
            assert!(!bar.menu_open() && bar.armed.is_none());
        }
        // Another entry, or Escape, cancels.
        let (mut bar, record) = recorded();
        let _ = bar.update(Message::Activate(Action::PowerOff));
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        assert!(bar.armed.is_none() && ran(&record).is_empty());
    }

    #[test]
    fn updates_and_game_mode_are_not_there_yet() {
        for action in [Action::Updates, Action::GameMode] {
            let (mut bar, record) = recorded();
            assert!(!action.enabled());
            assert_eq!(
                action.trailing(),
                sherlock_ui::component::Trailing::Shortcut("Bientôt")
            );
            let _ = bar.update(Message::Activate(action));
            assert!(ran(&record).is_empty() && bar.menu_open());
            let index = Action::ALL.iter().position(|a| *a == action).unwrap();
            let _ = bar.update(Message::Hover(index));
            assert_eq!(bar.cursor, None);
        }
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
                Target::Status | Target::Clock | Target::AppMenu(_) | Target::Tray => {
                    panic!("no indicator nor menu in this bar")
                }
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

    /// The bar with its surface, the panels unfolding (`motion`) or not.
    fn with_surface(motion: bool) -> Bar {
        let mut bar = Bar {
            panel_motion: Some(motion),
            ..with_workspaces()
        };
        let _ = bar.update(Message::Surface(Id::unique()));
        bar
    }

    fn frame_in(bar: &mut Bar, later: std::time::Duration) -> Task<Message> {
        bar.update(Message::Frame(Instant::now() + later))
    }

    /// The freeze the author saw after clicking the control centre again: a double click, and
    /// quick series of clicks, must leave one popup at most, never close one twice, and end
    /// at rest.
    #[test]
    fn a_double_click_and_quick_clicks_never_pile_popups_up() {
        let second = std::time::Duration::from_secs(2);
        // Unfolding: the clicks turn the same popup back and forth.
        let mut bar = with_surface(true);
        let _ = bar.update(Message::TogglePanel(Panel::Control));
        let first = bar.panel_surface();
        assert!(first.is_some() && bar.animating());
        for click in 1..=11 {
            let _ = bar.update(Message::TogglePanel(Panel::Control));
            assert_eq!(bar.panel_surface(), first, "click {click}: the same popup");
            assert_eq!(bar.panel_open().is_some(), click % 2 == 0);
        }
        // Eleven more clicks: folding. The popup closes once it has faded out, once.
        let closed = frame_in(&mut bar, second);
        assert!(closed.units() > 0);
        assert_eq!(bar.panel_surface(), None);
        assert!(!bar.animating(), "no frame at rest");
        assert_eq!(frame_in(&mut bar, second).units(), 0, "closed once");
        // The compositor's close of that popup, late, changes nothing.
        let _ = bar.update(Message::TogglePanel(Panel::Control));
        let next = bar.panel_surface();
        let _ = bar.update(Message::Closed(first.unwrap()));
        assert_eq!(bar.panel_surface(), next);
        let _ = frame_in(&mut bar, second);
        assert!(bar.panel_open().is_some() && !bar.animating());

        // In software, without unfolding: each click opens or closes at once.
        let mut bar = with_surface(false);
        let mut popups = Vec::new();
        for _ in 0..10 {
            let _ = bar.update(Message::TogglePanel(Panel::Control));
            if let Some(id) = bar.panel_surface() {
                assert!(!popups.contains(&id), "a new popup each time");
                popups.push(id);
            }
        }
        assert_eq!((popups.len(), bar.panel_surface()), (5, None));
        assert!(!bar.animating());
        // The other panel replaces this one at once.
        let _ = bar.update(Message::TogglePanel(Panel::Control));
        let _ = bar.update(Message::TogglePanel(Panel::Calendar));
        assert_eq!(bar.panel_open(), Some(Panel::Calendar));
    }

    /// On the GPU the control centre turns into the calendar and back in the same popup, a
    /// motion that runs then rests; in software the other panel replaces it at once.
    #[test]
    fn the_control_centre_turns_into_the_calendar_in_the_same_popup() {
        let second = std::time::Duration::from_secs(2);
        let mut bar = with_surface(true);
        let _ = bar.update(Message::TogglePanel(Panel::Control));
        let _ = frame_in(&mut bar, second);
        let popup = bar.panel_surface();
        assert!(popup.is_some() && !bar.animating());
        let _ = bar.update(Message::TogglePanel(Panel::Calendar));
        assert_eq!(bar.panel_surface(), popup, "the same popup");
        assert_eq!(bar.panel_open(), Some(Panel::Calendar));
        assert!(matches!(bar.swap, Some((Panel::Control, _))));
        assert!(bar.animating(), "frames while it turns");
        // And back, from the calendar to the control centre.
        let _ = bar.update(Message::TogglePanel(Panel::Control));
        assert_eq!(bar.panel_surface(), popup);
        assert!(matches!(bar.swap, Some((Panel::Calendar, _))));
        let _ = frame_in(&mut bar, second);
        assert_eq!(bar.swap, None);
        assert!(!bar.animating(), "no frame at rest");
        assert_eq!(bar.panel_open(), Some(Panel::Control));

        let mut bar = with_surface(false);
        let _ = bar.update(Message::TogglePanel(Panel::Control));
        let popup = bar.panel_surface();
        let _ = bar.update(Message::TogglePanel(Panel::Calendar));
        assert_ne!(bar.panel_surface(), popup, "a popup of its own size");
        assert_eq!(bar.swap, None);
    }

    /// The Sherlock menu unfolds and folds back on the GPU before its popup closes.
    #[test]
    fn the_menu_folds_back_before_it_closes() {
        let second = std::time::Duration::from_secs(2);
        let mut bar = with_surface(true);
        let _ = bar.update(Message::ToggleMenu);
        assert!(bar.menu_open() && bar.animating());
        let _ = frame_in(&mut bar, second);
        assert!(!bar.animating());
        let _ = bar.update(Message::ToggleMenu);
        assert!(bar.menu_open(), "still there while it folds");
        assert!(bar.menu_unfold.folding());
        // Clicked again: it unfolds again in the same popup.
        let _ = bar.update(Message::ToggleMenu);
        assert!(!bar.menu_unfold.folding());
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        let closed = frame_in(&mut bar, second);
        assert!(closed.units() > 0);
        assert!(!bar.menu_open() && !bar.animating());
        // A panel opening takes the menu away at once: one grab at a time.
        let _ = bar.update(Message::ToggleMenu);
        let _ = bar.update(Message::TogglePanel(Panel::Calendar));
        assert!(!bar.menu_open());
    }

    /// The panels land under the end of the bar, at 54 from the top and 12 from the right edge,
    /// on the reference screen and on the author's 1920 × 1080, in software and on the GPU,
    /// whether the bar's size comes before or after its id, or only from its screen (UI-02a:
    /// placed for 1440 on a 1920 screen, they opened near its middle).
    #[test]
    fn the_panels_open_under_the_end_of_the_bar_on_any_screen() {
        let side = metrics::TOP_BAR_MARGIN_SIDE;
        for screen in [1440.0, 1920.0] {
            for (gpu, size_first, from_screen) in [
                (false, true, false),
                (true, false, false),
                (true, true, true),
            ] {
                let mut bar = Bar {
                    panel_motion: Some(gpu),
                    ..Bar::default()
                };
                let id = Id::unique();
                if from_screen {
                    let _ = bar.update(Message::Output(id, None, Some((screen, 900.0))));
                    let _ = bar.update(Message::Surface(id));
                } else if size_first {
                    let _ = bar.update(Message::Resized(id, screen - 2.0 * side));
                    let _ = bar.update(Message::Surface(id));
                } else {
                    let _ = bar.update(Message::Surface(id));
                    let _ = bar.update(Message::Resized(id, screen - 2.0 * side));
                }
                bar.panel_margins = panel::margins(gpu);
                bar.panel_shared = gpu;
                for which in [Panel::Control, Panel::Calendar] {
                    let (width, _) = panel::surface_size(which, bar.panel_margins, gpu);
                    let (x, _, anchor, top) = bar.panel_anchor(which);
                    // The popup is centred under its anchor rectangle, in the bar surface.
                    let surface_left = side + (x as f32 + anchor as f32 / 2.0) - width as f32 / 2.0;
                    let (ix, iy, iw, _) = panel::input_rect(which, bar.panel_margins, gpu);
                    let left = surface_left + ix as f32;
                    let case = format!("{which:?} on {screen}, gpu {gpu}");
                    assert_eq!(left + iw as f32, screen - side, "{case}");
                    let y = metrics::TOP_BAR_MARGIN_TOP + top as f32 + iy as f32;
                    assert_eq!(y, 54.0, "{case}");
                }
            }
        }
    }

    #[test]
    fn a_notification_service_that_is_not_ours_leaves_the_centre_unavailable() {
        // Plasma's daemon owns the name: `/org/sherlockos/Notifications` is unknown there.
        let mut bar = with_surface(false);
        let _ = bar.update(Message::TogglePanel(Panel::Calendar));
        assert_eq!(bar.history, History::Loading);
        let error = String::from("org.freedesktop.DBus.Error.UnknownObject");
        let next = bar.update(Message::History(Err(error)));
        assert_eq!(bar.history, History::Unavailable);
        assert_eq!(next.units(), 0, "nothing read again in a loop");
        // Closed, its changes read nothing.
        let _ = bar.update(Message::ClosePanel);
        assert_eq!(bar.update(Message::HistoryChanged).units(), 0);
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

    #[test]
    fn the_wifi_icon_follows_the_signal_the_connection_and_the_internet() {
        use crate::source::network::Link as Carrier;
        let wifi = Network::Connected {
            link: Carrier::Wifi,
            name: String::from("Maison"),
        };
        let at = |strength| Link {
            strength: Some(strength),
            ..Link::default()
        };
        for (strength, level) in [(10, 0), (30, 1), (60, 2), (90, 3)] {
            assert_eq!(
                network_frames(&wifi, Some(at(strength))),
                [icon::wifi_svg(level, WifiMark::None)]
            );
        }
        // Without NetworkManager on the bus, every arc, as before.
        assert_eq!(
            network_frames(&wifi, None),
            [icon::wifi_svg(3, WifiMark::None)]
        );
        let limited = Link {
            no_internet: true,
            ..at(60)
        };
        assert_eq!(
            network_frames(&wifi, Some(limited)),
            [icon::wifi_svg(2, WifiMark::NoInternet)]
        );
        assert_eq!(
            network_frames(&Network::Disconnected, None),
            [icon::wifi_svg(0, WifiMark::Off)]
        );
        // Connecting: the arcs lit one after the other.
        let connecting = Link {
            connecting: true,
            ..Link::default()
        };
        let frames = network_frames(&Network::Disconnected, Some(connecting));
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[3], icon::wifi_svg(3, WifiMark::None));
    }

    #[test]
    fn the_bluetooth_indicator_shows_only_with_an_adapter() {
        let shown = |bar: &Bar| {
            bar.indicators()
                .iter()
                .any(|(indicator, _)| *indicator == Indicator::Bluetooth)
        };
        let mut bar = with_workspaces();
        assert!(!shown(&bar) && !bar.targets().contains(&Target::Status));
        let _ = bar.update(Message::Radio(Some(Radio::Connected)));
        assert!(shown(&bar));
        // The indicators are one button, one stop of Tab.
        assert_eq!(bar.targets()[4], Target::Status);
        let _ = bar.update(Message::Sound(crate::source::sound::Sound {
            volume: 40,
            muted: false,
        }));
        let count = bar
            .targets()
            .iter()
            .filter(|t| **t == Target::Status)
            .count();
        assert_eq!(count, 1);
        let _ = bar.update(Message::Radio(None));
        assert!(!shown(&bar));
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
