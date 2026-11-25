//! Application state and the response to each message: navigation, selection, operations,
//! keyboard. The view lives in `view`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::widget::{operation, scrollable};
use iced::{Event, Point, Size, Subscription, Task, event, mouse, window};
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::metrics;
use sherlock_ui::texts::evidence as texts;

use crate::devices::{DeviceError, DeviceSource, Volume};
use crate::git::{self, Repo};
use crate::icons::AppIcon;
use crate::listing::{self, Column, Entry, Kind, Sort};
use crate::ops;
use crate::places::{self, Place};
use crate::preview::{self, Preview};
use crate::probe::{Probe, Step};
use crate::trash::Trash;
use crate::udisks::UDisks;
use sherlock_ui::motion::{self, Animation, Fade};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Identifiers of the widgets the application drives.
pub const LIST: &str = "list";
pub const SEARCH: &str = "search";
pub const RENAME: &str = "rename";
pub const CHOOSER_SEARCH: &str = "chooser-search";

/// What the application receives at startup.
#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    pub start: PathBuf,
    pub favorites: Vec<Place>,
    pub trash: Option<Trash>,
    /// `--measure <folder> [--maximized]`: measures smoothness on this folder, then quits.
    pub measure: Option<PathBuf>,
    /// The measure runs in a maximized window.
    pub maximized: bool,
    /// Disks, keys and cards, and what can be done to them.
    pub devices: Option<Arc<dyn DeviceSource>>,
    /// Follow UDisks2 signals so that devices appear and disappear by themselves.
    pub watch_devices: bool,
    /// Where applications and associations are read and written; none disables « Ouvrir
    /// avec » (tests that do not need it).
    pub xdg: Option<mime_apps::Dirs>,
    /// Current desktop names, lower case (`XDG_CURRENT_DESKTOP`), and locales to try for
    /// application names.
    pub desktops: Vec<String>,
    pub locales: Vec<String>,
}

impl Config {
    /// Home folder, folder given as argument, the user's trash.
    pub fn from_env() -> Self {
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut args: Vec<_> = std::env::args_os().skip(1).collect();
        let measure = match args.first() {
            Some(flag) if flag == "--measure" => {
                args.remove(0);
                args.first().and_then(|p| std::path::absolute(p).ok())
            }
            _ => None,
        };
        let maximized = measure.is_some() && args.iter().any(|a| a == "--maximized");
        let start = match measure {
            Some(_) => home.clone(),
            None => args
                .first()
                .and_then(|p| std::path::absolute(p).ok())
                .unwrap_or_else(|| home.clone()),
        };
        let favorites = places::favorites(&home, places::read_user_dirs(&home).as_deref());
        let home_for_xdg = home.clone();
        Self {
            home,
            start,
            favorites,
            trash: Trash::user(),
            measure,
            maximized,
            devices: UDisks::connect()
                .ok()
                .map(|u| Arc::new(u) as Arc<dyn DeviceSource>),
            watch_devices: true,
            xdg: Some(mime_apps::Dirs::from_env(&home_for_xdg)),
            desktops: std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .filter(|d| !d.is_empty())
                .map(str::to_lowercase)
                .collect(),
            locales: locales(),
        }
    }
}

/// Where we are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Folder(PathBuf),
    Trash,
}

/// The commands of the menu and of the shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Open,
    OpenTerminal,
    Rename,
    Duplicate,
    CopyPath,
    Copy,
    Cut,
    Paste,
    NewFolder,
    ToggleHidden,
    Trash,
    Restore,
    EmptyTrash,
    Refresh,
    TogglePreview,
    SelectAll,
    /// « Autre application… »: the chooser of every application.
    ChooseApp,
    /// "Always open this type with the chosen application", on or off.
    ToggleAlways,
    /// On the device whose menu is open: open (mounting first), unmount, eject.
    DeviceOpen,
    DeviceUnmount,
    DeviceEject,
}

/// An entry of the right-click menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Action {
        label: &'static str,
        keys: Option<&'static str>,
        command: Command,
        danger: bool,
        checked: bool,
    },
    /// Opens the « Ouvrir avec » submenu.
    OpenWith,
    /// An application of the « Ouvrir avec » submenu, by its place in `App::open_with`.
    App(usize),
    /// Planned, not available yet.
    Disabled(&'static str),
    Separator,
}

impl MenuItem {
    const fn action(label: &'static str, keys: Option<&'static str>, command: Command) -> Self {
        Self::Action {
            label,
            keys,
            command,
            danger: false,
            checked: false,
        }
    }

    fn selectable(self) -> bool {
        matches!(self, Self::Action { .. } | Self::OpenWith | Self::App(_))
    }
}

/// The open right-click menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    /// Top left corner, in the window.
    pub at: Point,
    /// Opened on the selection, or on the background of the listing.
    pub on_selection: bool,
    pub submenu: bool,
    /// Entry highlighted from the keyboard, in the menu or in the submenu.
    pub highlight: Option<usize>,
}

/// Status bar message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

/// End of an operation run off the interface thread: text to show and items to select, or
/// the error.
pub type Outcome = Result<(String, Vec<PathBuf>), String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    Drag,
    Minimize,
    Maximize,
    Close,
}

#[derive(Debug, Clone)]
pub enum Message {
    Go(Location),
    Back,
    Forward,
    Up,
    Click(usize),
    DoubleClick(usize),
    RightClick(Option<usize>),
    Sort(Column),
    Search(String),
    Run(Command),
    Submenu(bool),
    CloseMenu,
    /// Click on the background of the listing.
    Blank,
    Key(keyboard::Event),
    Modifiers(Modifiers),
    Cursor(Point),
    Resized(Size),
    Scrolled(f32, f32),
    RenameInput(String),
    RenameSubmit,
    ConfirmEmpty,
    CancelEmpty,
    Done(Outcome),
    Git(PathBuf, Option<Repo>),
    Window(WindowAction),
    /// A frame of the window, during an animation or a measurement.
    Frame(Instant),
    /// Content of a location, read off the interface thread. The number discards a read
    /// overtaken by a more recent navigation.
    Loaded(u64, Result<Vec<Entry>, ()>),
    /// Preview prepared off the interface thread.
    Preview(PathBuf, Preview),
    /// A program was started, or could not be.
    Launched(bool),
    /// Installed applications and associations, read off the interface thread.
    Catalog(Arc<mime_apps::Catalog>),
    /// Application icons, decoded off the interface thread.
    Icons(Arc<HashMap<String, AppIcon>>),
    /// Open the selection with this application (desktop id).
    OpenWithApp(String),
    /// The chooser's search field.
    ChooserSearch(String),
    /// Close the chooser.
    CloseChooser,
    /// The chooser's "always" switch.
    Always(bool),
    /// A default application was written to mimeapps.list, or could not be.
    DefaultSet(bool),
    /// Which favourites exist, checked off the interface thread.
    Places(Vec<bool>),
    /// The volumes of the sidebar, as UDisks2 sees them now.
    Devices(Vec<Volume>),
    /// A click on a volume: open it, mounting it first if needed.
    OpenDevice(String),
    /// A right click on a volume.
    DeviceMenu(String),
    /// The eject button beside a volume.
    EjectDevice(String),
    /// An action on a device is over.
    DeviceDone(DeviceAction, Result<Option<PathBuf>, DeviceError>),
}

/// What was asked of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAction {
    Open,
    Unmount,
    Eject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clip {
    Copy,
    Cut,
}

/// The Evidence application.
pub struct App {
    pub(crate) config: Config,
    pub(crate) location: Location,
    history: Vec<Location>,
    future: Vec<Location>,
    /// The whole folder, then what the listing shows (search applied, sorted).
    all: Vec<Entry>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) selection: BTreeSet<usize>,
    pub(crate) cursor: Option<usize>,
    pub(crate) sort: Sort,
    pub(crate) show_hidden: bool,
    pub(crate) show_preview: bool,
    pub(crate) search: String,
    pub(crate) repo: Option<Repo>,
    pub(crate) preview: Preview,
    clipboard: Option<(Clip, Vec<PathBuf>)>,
    /// Item being renamed and its new name.
    pub(crate) renaming: Option<(usize, String)>,
    pub(crate) menu: Option<Menu>,
    pub(crate) confirm_empty: bool,
    pub(crate) notice: Option<Notice>,
    pub(crate) busy: usize,
    pointer: Point,
    modifiers: Modifiers,
    pub(crate) size: Size,
    /// Scroll of the listing and visible height, to keep the cursor in view.
    pub(crate) scroll: (f32, f32),
    probe: Option<Probe>,
    /// Number of the last requested read, items to select when it arrives, and rename to
    /// open then (new folder).
    generation: u64,
    pending: Vec<PathBuf>,
    pending_rename: bool,
    /// The location just changed: its content will fade in.
    fresh: bool,
    /// Item whose preview is shown or awaited.
    preview_path: Option<PathBuf>,
    /// Time origin of the animations.
    origin: Instant,
    /// Vertical position of the selection highlight, which glides from one row to another.
    pub(crate) highlight: Option<Animation>,
    /// Fade of the content of a folder that opens, and of the preview.
    pub(crate) content_fade: Option<Fade>,
    pub(crate) preview_fade: Option<Fade>,
    /// Volumes of the sidebar, and the one whose menu is open.
    pub(crate) volumes: Vec<Volume>,
    device_menu: Option<String>,
    /// Which favourites exist, as last checked off the interface thread.
    pub(crate) available: Vec<bool>,
    /// The trash items shown when the user was asked to empty it: only those are deleted.
    empty_ids: Vec<String>,
    /// Installed applications, file types and associations.
    pub(crate) catalog: Option<Arc<mime_apps::Catalog>>,
    /// Applications of the open « Ouvrir avec » submenu, the default first.
    pub(crate) open_with: Vec<mime_apps::DesktopEntry>,
    /// "Always open this type with the chosen application".
    pub(crate) always: bool,
    /// The chooser of every application is open, with its search.
    pub(crate) chooser: Option<String>,
    /// Application icons, by desktop id.
    pub(crate) icons: Arc<HashMap<String, AppIcon>>,
}

impl App {
    pub fn new(config: Config) -> (Self, Task<Message>) {
        let start = Location::Folder(config.start.clone());
        let (width, height) = metrics::REFERENCE_WINDOW;
        let mut app = Self {
            config,
            location: start,
            history: Vec::new(),
            future: Vec::new(),
            all: Vec::new(),
            entries: Vec::new(),
            selection: BTreeSet::new(),
            cursor: None,
            sort: Sort::default(),
            show_hidden: false,
            show_preview: true,
            search: String::new(),
            repo: None,
            preview: Preview::Info,
            clipboard: None,
            renaming: None,
            menu: None,
            confirm_empty: false,
            notice: None,
            busy: 0,
            pointer: Point::ORIGIN,
            modifiers: Modifiers::default(),
            size: Size::new(width, height),
            scroll: (0.0, height),
            probe: None,
            generation: 0,
            pending: Vec::new(),
            pending_rename: false,
            fresh: true,
            preview_path: None,
            origin: Instant::now(),
            highlight: None,
            content_fade: None,
            preview_fade: None,
            volumes: Vec::new(),
            device_menu: None,
            available: Vec::new(),
            empty_ids: Vec::new(),
            catalog: None,
            open_with: Vec::new(),
            always: false,
            chooser: None,
            icons: Arc::default(),
        };
        app.probe = app.config.measure.clone().map(Probe::new);
        let task = app.load(&[]);
        let devices = app.refresh_devices();
        let catalog = app.load_catalog();
        (app, Task::batch([task, devices, catalog]))
    }

    /// Keyboard, pointer and window size.
    pub fn subscription(&self) -> Subscription<Message> {
        // Frames tick only during an animation or a measurement, never at rest.
        let measuring = self.probe.as_ref().is_some_and(Probe::active);
        let frames = if measuring || self.animating(self.now()) {
            window::frames().map(Message::Frame)
        } else {
            Subscription::none()
        };
        let events = event::listen_with(|event, status, _| match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                Some(Message::Modifiers(modifiers))
            }
            Event::Keyboard(key) if status == event::Status::Ignored => Some(Message::Key(key)),
            Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::Cursor(position)),
            Event::Window(window::Event::Resized(size)) => Some(Message::Resized(size)),
            _ => None,
        });
        let devices = if self.config.watch_devices && self.config.devices.is_some() {
            Subscription::run(crate::udisks::stream).map(Message::Devices)
        } else {
            Subscription::none()
        };
        Subscription::batch([events, frames, devices])
    }

    /// Window title: the application name only. The folder is shown in the drawn title bar.
    pub fn title(&self) -> String {
        texts::APP_NAME.to_owned()
    }

    /// Name of the current folder, for the title bar and the search.
    pub fn location_name(&self) -> String {
        match &self.location {
            Location::Trash => texts::TRASH.into(),
            Location::Folder(dir) if *dir == self.config.home => texts::HOME.into(),
            Location::Folder(dir) => dir
                .file_name()
                .map_or_else(|| texts::ROOT.into(), |n| n.to_string_lossy().into_owned()),
        }
    }

    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.future.is_empty()
    }

    /// Animation time.
    pub fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    /// An animation of the application is still running.
    pub fn animating(&self, now: Duration) -> bool {
        let spring = |a: &Option<Animation>, precision| {
            a.as_ref().is_some_and(|a| !a.is_done(now, precision))
        };
        let fade = |f: &Option<Fade>| f.as_ref().is_some_and(|f| !f.is_done(now));
        spring(&self.highlight, motion::GEOMETRY_PRECISION)
            || fade(&self.content_fade)
            || fade(&self.preview_fade)
    }

    /// Sets every animation to its end. For captures.
    pub fn settle(&mut self) {
        let now = self.now();
        self.content_fade = None;
        self.preview_fade = None;
        if let Some(target) = self.highlight.as_ref().map(Animation::target) {
            self.highlight = Some(Animation::new(motion::PREVIEW_SPRING, target, target, now));
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        let now = self.now();
        // The highlight follows the cursor when a single item is selected.
        let target = match (self.selection.len(), self.cursor) {
            (1, Some(i)) => Some(
                f64::from(metrics::LIST_TOP_GAP) + i as f64 * f64::from(metrics::LIST_ROW_HEIGHT),
            ),
            _ => None,
        };
        match (target, &mut self.highlight) {
            (None, _) => self.highlight = None,
            (Some(y), Some(anim)) if (anim.target() - y).abs() > f64::EPSILON => {
                anim.retarget(now, y)
            }
            (Some(_), Some(_)) => {}
            (Some(y), None) => {
                self.highlight = Some(Animation::new(motion::PREVIEW_SPRING, y, y, now))
            }
        }
        if self.menu.is_none() {
            self.device_menu = None;
            // Closing the menu (or the chooser) forgets the "always" switch.
            if self.chooser.is_none() {
                self.always = false;
            }
        }
        Task::batch([task, self.preview_task()])
    }

    /// Requests the preview of the selected item, if it changed.
    fn preview_task(&mut self) -> Task<Message> {
        let wanted = self.single().cloned();
        let path = wanted.as_ref().map(|e| e.path.clone());
        if path == self.preview_path {
            return Task::none();
        }
        self.preview_path.clone_from(&path);
        self.preview = Preview::Info;
        match (wanted, path) {
            (Some(entry), Some(path)) => {
                Task::perform(async move { preview::load(&entry) }, move |preview| {
                    Message::Preview(path.clone(), preview)
                })
            }
            _ => Task::none(),
        }
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Go(location) => return self.go(location),
            Message::Back => {
                if let Some(previous) = self.history.pop() {
                    let current = std::mem::replace(&mut self.location, previous);
                    self.future.push(current);
                    return self.reset_and_load(&[]);
                }
            }
            Message::Forward => {
                if let Some(next) = self.future.pop() {
                    let current = std::mem::replace(&mut self.location, next);
                    self.history.push(current);
                    return self.reset_and_load(&[]);
                }
            }
            Message::Up => {
                if let Location::Folder(dir) = &self.location
                    && let Some(parent) = dir.parent()
                {
                    let (child, parent) = (dir.clone(), parent.to_path_buf());
                    return self.go_select(Location::Folder(parent), vec![child]);
                }
            }
            Message::Click(index) => {
                self.menu = None;
                self.renaming = None;
                if self.modifiers.control() {
                    if !self.selection.remove(&index) {
                        self.selection.insert(index);
                    }
                    self.cursor = Some(index);
                } else {
                    self.select_only(index);
                }
            }
            Message::DoubleClick(index) => {
                self.select_only(index);
                return self.open();
            }
            Message::RightClick(index) => {
                self.renaming = None;
                if let Some(index) = index
                    && !self.selection.contains(&index)
                {
                    self.select_only(index);
                }
                if index.is_none() {
                    self.clear_selection();
                }
                self.open_menu(self.pointer);
            }
            Message::Sort(column) => {
                self.sort = self.sort.toggle(column);
                self.refilter();
            }
            Message::Search(text) => {
                self.search = text;
                self.refilter();
            }
            Message::Run(command) => {
                // The "always" switch stays in its open menu, like a check box.
                if command != Command::ToggleAlways {
                    self.menu = None;
                }
                return self.run(command);
            }
            Message::Submenu(open) => {
                if open {
                    self.prepare_open_with();
                }
                if let Some(menu) = &mut self.menu {
                    menu.submenu = open;
                    menu.highlight = None;
                }
            }
            Message::CloseMenu => self.menu = None,
            Message::Blank => {
                self.menu = None;
                self.renaming = None;
                self.clear_selection();
            }
            Message::Key(event) => return self.key(event),
            Message::Modifiers(modifiers) => self.modifiers = modifiers,
            Message::Cursor(position) => self.pointer = position,
            Message::Resized(size) => self.size = size,
            Message::Scrolled(offset, height) => self.scroll = (offset, height),
            Message::RenameInput(name) => {
                if let Some((_, value)) = &mut self.renaming {
                    *value = name;
                }
            }
            Message::RenameSubmit => return self.finish_rename(),
            Message::ConfirmEmpty => {
                self.confirm_empty = false;
                return self.empty_trash();
            }
            Message::CancelEmpty => self.confirm_empty = false,
            Message::Done(outcome) => {
                self.busy = self.busy.saturating_sub(1);
                let select = match outcome {
                    // A quiet operation (rename, new folder) leaves no message behind.
                    Ok((text, select)) => {
                        self.notice = (!text.is_empty()).then_some(Notice { text, error: false });
                        select
                    }
                    Err(text) => {
                        self.pending_rename = false;
                        self.notice = Some(Notice { text, error: true });
                        Vec::new()
                    }
                };
                return self.load(&select);
            }
            Message::Git(dir, repo) => {
                if self.location == Location::Folder(dir) {
                    self.repo = repo;
                    self.apply_repo();
                    self.refilter();
                }
            }
            Message::Frame(now) => return self.frame(now),
            Message::Devices(volumes) => self.volumes = volumes,
            Message::Places(available) => self.available = available,
            Message::Catalog(catalog) => {
                self.catalog = Some(catalog.clone());
                return Task::perform(
                    async move { Arc::new(crate::icons::load_all(&catalog)) },
                    Message::Icons,
                );
            }
            Message::Icons(icons) => self.icons = icons,
            Message::OpenWithApp(id) => {
                self.menu = None;
                self.chooser = None;
                return self.open_with_app(&id);
            }
            Message::ChooserSearch(text) => self.chooser = Some(text),
            Message::CloseChooser => self.chooser = None,
            Message::Always(on) => self.always = on,
            Message::DefaultSet(written) => {
                if written {
                    return self.load_catalog();
                }
                self.notice = Some(Notice {
                    text: texts::DEFAULT_NOT_SAVED.into(),
                    error: true,
                });
            }
            Message::Launched(started) => {
                if !started {
                    self.notice = Some(Notice {
                        text: texts::CANNOT_OPEN.into(),
                        error: true,
                    });
                }
            }
            Message::OpenDevice(id) => return self.device(DeviceAction::Open, id),
            Message::EjectDevice(id) => return self.device(DeviceAction::Eject, id),
            Message::DeviceMenu(id) => {
                self.renaming = None;
                self.device_menu = Some(id);
                self.open_menu(self.pointer);
            }
            Message::DeviceDone(action, result) => return self.device_done(action, result),
            Message::Loaded(generation, result) => {
                if generation == self.generation {
                    return self.loaded(result);
                }
            }
            Message::Preview(path, preview) => {
                if self.preview_path.as_ref() == Some(&path) {
                    self.preview = preview;
                    self.preview_fade = Some(Fade::new(0.0, 1.0, self.now(), motion::FADE));
                }
            }
            Message::Window(action) => {
                return window::latest().and_then(move |id| match action {
                    WindowAction::Drag => window::drag(id),
                    WindowAction::Minimize => window::minimize(id, true),
                    WindowAction::Maximize => window::toggle_maximize(id),
                    WindowAction::Close => window::close(id),
                });
            }
        }
        Task::none()
    }

    fn frame(&mut self, now: Instant) -> Task<Message> {
        // A finished animation is dropped: the frame subscription stops.
        let t = self.now();
        if self.content_fade.as_ref().is_some_and(|f| f.is_done(t)) {
            self.content_fade = None;
        }
        if self.preview_fade.as_ref().is_some_and(|f| f.is_done(t)) {
            self.preview_fade = None;
        }
        let Some(probe) = &mut self.probe else {
            return Task::none();
        };
        let target = Location::Folder(probe.target.clone());
        let loaded = self.location == target && !self.entries.is_empty();
        match probe.frame(now, loaded) {
            Step::Open if self.config.maximized => Task::batch([
                self.go(target),
                window::latest().and_then(|id| window::maximize(id, true)),
            ]),
            Step::Open => self.go(target),
            Step::Scroll => operation::scroll_by(
                LIST,
                scrollable::AbsoluteOffset {
                    x: 0.0,
                    y: crate::probe::SCROLL_STEP,
                },
            ),
            Step::Select => {
                let next = self.cursor.map_or(0, |c| c + 1) % self.entries.len().max(1);
                self.move_cursor(next)
            }
            Step::Wait | Step::Idle => Task::none(),
            Step::Exit => iced::exit(),
        }
    }

    // Devices.

    /// How many items the open "empty the trash" question is about.
    pub fn empty_ids_len(&self) -> usize {
        self.empty_ids.len()
    }

    /// The volume whose menu is open.
    fn menu_volume(&self) -> Option<&Volume> {
        let id = self.device_menu.as_ref()?;
        self.volumes.iter().find(|v| &v.id == id)
    }

    /// Asks the source for the volumes, off the interface thread.
    fn refresh_devices(&self) -> Task<Message> {
        match self.config.devices.clone() {
            Some(source) => Task::perform(async move { source.volumes() }, |result| {
                Message::Devices(result.unwrap_or_default())
            }),
            None => Task::none(),
        }
    }

    /// Opens, unmounts or ejects a volume. Opening a mounted volume only navigates.
    fn device(&mut self, action: DeviceAction, id: String) -> Task<Message> {
        let Some(volume) = self.volumes.iter().find(|v| v.id == id).cloned() else {
            return Task::none();
        };
        let Some(source) = self.config.devices.clone() else {
            return Task::none();
        };
        match action {
            DeviceAction::Open if volume.locked => {
                return self.device_done(action, Err(DeviceError::Locked));
            }
            DeviceAction::Open => {
                if let Some(point) = volume.mount_point {
                    return self.go(Location::Folder(point));
                }
            }
            DeviceAction::Unmount | DeviceAction::Eject => {
                // Leave the volume before it goes away.
                if let (Some(point), Location::Folder(dir)) = (&volume.mount_point, &self.location)
                    && dir.starts_with(point)
                {
                    let home = self.config.home.clone();
                    let _ = self.go(Location::Folder(home));
                }
            }
        }
        self.busy += 1;
        Task::perform(
            async move {
                match action {
                    DeviceAction::Open => source.mount(&id).map(Some),
                    DeviceAction::Unmount => source.unmount(&id).map(|()| None),
                    DeviceAction::Eject => source.eject(&id).map(|()| None),
                }
            },
            move |result| Message::DeviceDone(action, result),
        )
    }

    fn device_done(
        &mut self,
        action: DeviceAction,
        result: Result<Option<PathBuf>, DeviceError>,
    ) -> Task<Message> {
        self.busy = self.busy.saturating_sub(1);
        let refresh = self.refresh_devices();
        match result {
            Ok(Some(point)) => {
                let go = self.go(Location::Folder(point));
                Task::batch([go, refresh])
            }
            Ok(None) => {
                let text = match action {
                    DeviceAction::Eject => texts::EJECTED,
                    _ => texts::UNMOUNTED,
                };
                self.notice = Some(Notice {
                    text: text.into(),
                    error: false,
                });
                refresh
            }
            Err(error) => {
                tracing::warn!(?error, "device action refused");
                self.notice = Some(Notice {
                    text: error.to_string(),
                    error: true,
                });
                refresh
            }
        }
    }

    // Navigation.

    fn go(&mut self, location: Location) -> Task<Message> {
        self.go_select(location, Vec::new())
    }

    fn go_select(&mut self, location: Location, select: Vec<PathBuf>) -> Task<Message> {
        if location == self.location {
            return Task::none();
        }
        let previous = std::mem::replace(&mut self.location, location);
        self.history.push(previous);
        self.future.clear();
        self.reset_and_load(&select)
    }

    fn reset_and_load(&mut self, select: &[PathBuf]) -> Task<Message> {
        self.search.clear();
        self.clear_selection();
        self.repo = None;
        self.notice = None;
        self.fresh = true;
        self.scroll.0 = 0.0;
        let load = self.load(select);
        Task::batch([
            load,
            operation::scroll_to(LIST, scrollable::AbsoluteOffset { x: 0.0, y: 0.0 }),
        ])
    }

    /// Rereads the location, keeps the selection (or selects `select`), and requests the git
    /// status off the interface thread.
    fn load(&mut self, select: &[PathBuf]) -> Task<Message> {
        self.menu = None;
        self.renaming = None;
        self.pending = if select.is_empty() {
            self.selected_paths()
        } else {
            select.to_vec()
        };
        self.generation += 1;
        let generation = self.generation;
        let location = self.location.clone();
        let hidden = self.show_hidden;
        let trash = self.config.trash.clone();
        let listing = Task::perform(
            async move { read_location(&location, hidden, trash.as_ref()) },
            move |result| Message::Loaded(generation, result),
        );
        let git = match self.location.clone() {
            Location::Folder(dir) => {
                let target = dir.clone();
                Task::perform(async move { git::status(&target) }, move |repo| {
                    Message::Git(dir.clone(), repo)
                })
            }
            Location::Trash => Task::none(),
        };
        let places: Vec<PathBuf> = self
            .config
            .favorites
            .iter()
            .map(|p| p.path.clone())
            .collect();
        let check = Task::perform(
            async move { places.iter().map(|p| p.is_dir()).collect() },
            Message::Places,
        );
        Task::batch([listing, git, check])
    }

    /// The content of the location has arrived.
    fn loaded(&mut self, result: Result<Vec<Entry>, ()>) -> Task<Message> {
        match result {
            Ok(entries) => self.all = entries,
            Err(()) => {
                self.all.clear();
                self.notice = Some(Notice {
                    text: texts::UNREADABLE.into(),
                    error: true,
                });
            }
        }
        self.apply_repo();
        self.refilter();
        let pending = std::mem::take(&mut self.pending);
        self.select_paths(&pending);
        if std::mem::take(&mut self.fresh) {
            self.highlight = None;
            self.content_fade = Some(Fade::new(0.0, 1.0, self.now(), motion::FADE));
        }
        if std::mem::take(&mut self.pending_rename) {
            return self.start_rename();
        }
        Task::none()
    }

    fn apply_repo(&mut self) {
        for entry in &mut self.all {
            entry.git = self.repo.as_ref().and_then(|repo| repo.status(&entry.name));
        }
    }

    /// Applies the search and the sort, keeping the selection.
    fn refilter(&mut self) {
        let keep = self.selected_paths();
        let needle = self.search.to_lowercase();
        self.entries = self
            .all
            .iter()
            .filter(|e| needle.is_empty() || e.labels.lower.contains(&needle))
            .cloned()
            .collect();
        listing::sort(&mut self.entries, self.sort);
        self.select_paths(&keep);
    }

    // Selection.

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selection
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .map(|e| e.path.clone())
            .collect()
    }

    fn select_paths(&mut self, paths: &[PathBuf]) {
        self.selection = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| paths.contains(&e.path))
            .map(|(i, _)| i)
            .collect();
        self.cursor = self.selection.iter().next().copied();
    }

    fn select_only(&mut self, index: usize) {
        self.selection = BTreeSet::from([index]);
        self.cursor = Some(index);
    }

    fn clear_selection(&mut self) {
        self.selection.clear();
        self.cursor = None;
    }

    /// The selected item, if it is alone.
    pub fn single(&self) -> Option<&Entry> {
        match self.selection.len() {
            1 => self.cursor.and_then(|i| self.entries.get(i)),
            _ => None,
        }
    }

    /// Moves the cursor from the keyboard and keeps the row visible.
    fn move_cursor(&mut self, to: usize) -> Task<Message> {
        if self.entries.is_empty() {
            return Task::none();
        }
        let index = to.min(self.entries.len() - 1);
        self.select_only(index);
        let row = metrics::LIST_ROW_HEIGHT;
        let top = metrics::LIST_TOP_GAP + index as f32 * row;
        let (offset, height) = self.scroll;
        let y = if top < offset {
            top
        } else if top + row > offset + height {
            top + row - height
        } else {
            return Task::none();
        };
        operation::scroll_to(LIST, scrollable::AbsoluteOffset { x: 0.0, y })
    }

    // Right-click menu.

    /// Menu entries: on the selection or on the background, in a folder or the trash.
    pub fn menu_items(&self, on_selection: bool) -> Vec<MenuItem> {
        use MenuItem::{Action, OpenWith, Separator};
        if let Some(volume) = self.menu_volume() {
            let open = if volume.mount_point.is_some() {
                texts::OPEN
            } else {
                texts::MOUNT
            };
            let mut items = vec![MenuItem::action(open, None, Command::DeviceOpen)];
            if volume.mount_point.is_some() || volume.ejectable {
                items.push(Separator);
            }
            if volume.mount_point.is_some() {
                items.push(MenuItem::action(
                    texts::UNMOUNT,
                    None,
                    Command::DeviceUnmount,
                ));
            }
            if volume.ejectable {
                items.push(MenuItem::action(texts::EJECT, None, Command::DeviceEject));
            }
            return items;
        }
        let paste = MenuItem::action(texts::PASTE, Some(texts::KEY_PASTE), Command::Paste);
        let empty = Action {
            label: texts::EMPTY_TRASH,
            keys: None,
            command: Command::EmptyTrash,
            danger: true,
            checked: false,
        };
        match (&self.location, on_selection) {
            (Location::Trash, true) => vec![
                MenuItem::action(texts::RESTORE, None, Command::Restore),
                Separator,
                empty,
            ],
            (Location::Trash, false) => vec![empty],
            (Location::Folder(_), true) => vec![
                MenuItem::action(texts::OPEN, Some(texts::KEY_OPEN), Command::Open),
                OpenWith,
                Separator,
                MenuItem::action(texts::COPY, Some(texts::KEY_COPY), Command::Copy),
                MenuItem::action(texts::CUT, Some(texts::KEY_CUT), Command::Cut),
                MenuItem::action(texts::RENAME, Some(texts::KEY_RENAME), Command::Rename),
                MenuItem::action(
                    texts::DUPLICATE,
                    Some(texts::KEY_DUPLICATE),
                    Command::Duplicate,
                ),
                MenuItem::action(
                    texts::COPY_PATH,
                    Some(texts::KEY_COPY_PATH),
                    Command::CopyPath,
                ),
                Separator,
                Action {
                    label: texts::MOVE_TO_TRASH,
                    keys: Some(texts::KEY_TRASH),
                    command: Command::Trash,
                    danger: true,
                    checked: false,
                },
            ],
            (Location::Folder(_), false) => vec![
                MenuItem::action(
                    texts::NEW_FOLDER,
                    Some(texts::KEY_NEW_FOLDER),
                    Command::NewFolder,
                ),
                paste,
                Separator,
                Action {
                    label: texts::SHOW_HIDDEN,
                    keys: Some(texts::KEY_HIDDEN),
                    command: Command::ToggleHidden,
                    danger: false,
                    checked: self.show_hidden,
                },
            ],
        }
    }

    /// Entries of the « Ouvrir avec » submenu: the applications that open the selected file's
    /// type, the default first and checked, then a terminal here, the chooser of every
    /// application, and the "always" switch.
    pub fn submenu_items(&self) -> Vec<MenuItem> {
        let mut items: Vec<MenuItem> = (0..self.open_with.len()).map(MenuItem::App).collect();
        if self.open_with.is_empty() {
            items.push(MenuItem::action(texts::DEFAULT_APP, None, Command::Open));
        }
        items.push(MenuItem::Separator);
        items.push(MenuItem::action(
            texts::TERMINAL_HERE,
            None,
            Command::OpenTerminal,
        ));
        let other = if self.catalog.is_some() {
            MenuItem::action(texts::OTHER_APP, None, Command::ChooseApp)
        } else {
            MenuItem::Disabled(texts::OTHER_APP)
        };
        items.push(other);
        items.push(MenuItem::Separator);
        items.push(MenuItem::Action {
            label: texts::ALWAYS_SHORT,
            keys: None,
            command: Command::ToggleAlways,
            danger: false,
            checked: self.always,
        });
        items
    }

    /// MIME type of the selected file, from its name.
    fn selected_type(&self) -> Option<String> {
        let entry = self.single()?;
        let catalog = self.catalog.as_ref()?;
        Some(
            catalog
                .mime
                .type_of(&entry.path, entry.kind == Kind::Folder),
        )
    }

    /// Fills the « Ouvrir avec » submenu for the selection, at most eight applications.
    fn prepare_open_with(&mut self) {
        self.open_with = match (self.selected_type(), &self.catalog) {
            (Some(mime), Some(catalog)) => catalog
                .handlers(&mime)
                .into_iter()
                .take(metrics::OPEN_WITH_MAX)
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
    }

    /// Reads the applications and associations off the interface thread.
    fn load_catalog(&self) -> Task<Message> {
        let Some(dirs) = self.config.xdg.clone() else {
            return Task::none();
        };
        let (desktops, locales) = (self.config.desktops.clone(), self.config.locales.clone());
        Task::perform(
            async move { Arc::new(mime_apps::Catalog::load(dirs, &desktops, &locales)) },
            Message::Catalog,
        )
    }

    /// Opens the selection with an application; with "always" on, makes it the default for
    /// the selection's type in mimeapps.list first.
    fn open_with_app(&mut self, id: &str) -> Task<Message> {
        let (Some(catalog), Some(entry)) = (self.catalog.clone(), self.single().cloned()) else {
            return Task::none();
        };
        let Some(app) = catalog.application(id).cloned() else {
            return Task::none();
        };
        // "Always" applies to this one choice only: it goes back off at once, so that a
        // later "open once with" never rewrites the defaults shared with the desktop.
        let always = std::mem::replace(&mut self.always, false);
        let save = match (always, self.selected_type(), self.config.xdg.clone()) {
            (true, Some(mime), Some(dirs)) => {
                let id = app.id.clone();
                let desktops = self.config.desktops.clone();
                Task::perform(
                    async move { mime_apps::set_default(&dirs, &desktops, &mime, &id).is_ok() },
                    Message::DefaultSet,
                )
            }
            _ => Task::none(),
        };
        let launch = match catalog.command(&app, &[entry.path]) {
            Some((program, args)) => {
                let mut command = Process::new(program);
                command.args(args);
                self.launch(command)
            }
            None => Task::none(),
        };
        Task::batch([save, launch])
    }

    /// Height of a menu, so that it fits in the window.
    pub fn menu_height(items: &[MenuItem]) -> f32 {
        let (top, _) = metrics::MENU_SEPARATOR_MARGIN;
        let rows: f32 = items
            .iter()
            .map(|item| match item {
                MenuItem::Separator => 2.0 * top + metrics::BORDER_WIDTH,
                _ => metrics::MENU_ITEM_HEIGHT,
            })
            .sum();
        rows + 2.0 * (metrics::MENU_PADDING + metrics::BORDER_WIDTH)
    }

    fn open_menu(&mut self, at: Point) {
        let on_selection = !self.selection.is_empty() && self.device_menu.is_none();
        let height = Self::menu_height(&self.menu_items(on_selection));
        let width = metrics::CONTEXT_MENU_WIDTH + metrics::SUBMENU_GAP;
        let width = width + metrics::OPEN_WITH_WIDTH;
        let at = Point::new(
            at.x.min(self.size.width - width).max(0.0),
            at.y.min(self.size.height - height).max(0.0),
        );
        self.menu = Some(Menu {
            at,
            on_selection,
            submenu: false,
            highlight: None,
        });
    }

    /// Opens the menu from the keyboard, under the cursor row.
    fn open_menu_at_cursor(&mut self) {
        let row = self.cursor.map_or(0.0, |i| (i + 1) as f32) * metrics::LIST_ROW_HEIGHT;
        let top = metrics::TITLE_BAR_HEIGHT
            + metrics::TOOLBAR_HEIGHT
            + metrics::LIST_HEADER_HEIGHT
            + 3.0 * metrics::BORDER_WIDTH
            + metrics::LIST_TOP_GAP;
        let left = metrics::SIDEBAR_WIDTH + metrics::LIST_PADDING + metrics::LIST_ROW_HEIGHT;
        self.open_menu(Point::new(left, top + row - self.scroll.0));
    }

    /// Arrows, Enter and Escape in the open menu.
    fn menu_key(&mut self, key: &Key) -> Task<Message> {
        let Some(menu) = &self.menu else {
            return Task::none();
        };
        let items = if menu.submenu {
            self.submenu_items()
        } else {
            self.menu_items(menu.on_selection)
        };
        let selectable: Vec<usize> = (0..items.len())
            .filter(|&i| items[i].selectable())
            .collect();
        let position = menu
            .highlight
            .and_then(|h| selectable.iter().position(|&i| i == h));
        let step = |forward: bool| {
            let count = selectable.len();
            let next = match (position, forward) {
                (None, true) => 0,
                (None, false) => count.saturating_sub(1),
                (Some(p), true) => (p + 1) % count,
                (Some(p), false) => (p + count - 1) % count,
            };
            selectable.get(next).copied()
        };
        let highlighted = menu.highlight.and_then(|h| items.get(h).copied());
        let Some(menu) = &mut self.menu else {
            return Task::none();
        };
        let mut open_submenu = false;
        match key {
            Key::Named(Named::ArrowDown) => menu.highlight = step(true),
            Key::Named(Named::ArrowUp) => menu.highlight = step(false),
            Key::Named(Named::ArrowRight) if highlighted == Some(MenuItem::OpenWith) => {
                open_submenu = true;
            }
            Key::Named(Named::ArrowLeft) if menu.submenu => {
                menu.submenu = false;
                menu.highlight = None;
            }
            Key::Named(Named::Escape) => self.menu = None,
            Key::Named(Named::Enter | Named::Space) => match highlighted {
                Some(MenuItem::Action { command, .. }) => {
                    if command != Command::ToggleAlways {
                        self.menu = None;
                    }
                    return self.run(command);
                }
                Some(MenuItem::App(index)) => {
                    let id = self.open_with.get(index).map(|app| app.id.clone());
                    self.menu = None;
                    return id.map_or_else(Task::none, |id| self.open_with_app(&id));
                }
                Some(MenuItem::OpenWith) => open_submenu = true,
                _ => {}
            },
            _ => {}
        }
        if open_submenu {
            self.prepare_open_with();
            if let Some(menu) = &mut self.menu {
                menu.submenu = true;
                menu.highlight = Some(0);
            }
        }
        Task::none()
    }

    // Keyboard.

    fn key(&mut self, event: keyboard::Event) -> Task<Message> {
        let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
            return Task::none();
        };
        if self.confirm_empty {
            return match key {
                Key::Named(Named::Escape) => {
                    self.confirm_empty = false;
                    Task::none()
                }
                Key::Named(Named::Tab) => focus::advance(direction(modifiers)),
                _ => Task::none(),
            };
        }
        if self.chooser.is_some() {
            return match key {
                Key::Named(Named::Escape) => {
                    self.chooser = None;
                    Task::none()
                }
                Key::Named(Named::Tab) => focus::advance(direction(modifiers)),
                _ => Task::none(),
            };
        }
        if self.menu.is_some() {
            return self.menu_key(&key);
        }
        if self.renaming.is_some() && key == Key::Named(Named::Escape) {
            // The field already gave the keyboard back on the first Escape: this one cancels.
            self.renaming = None;
            return Task::none();
        }
        let ctrl = modifiers.command();
        let shift = modifiers.shift();
        let last = self.entries.len().saturating_sub(1);
        match key.as_ref() {
            Key::Named(Named::Tab) => focus::advance(direction(modifiers)),
            Key::Named(Named::ArrowLeft) if modifiers.alt() => self.update(Message::Back),
            Key::Named(Named::ArrowRight) if modifiers.alt() => self.update(Message::Forward),
            Key::Named(Named::ArrowUp) if modifiers.alt() => self.update(Message::Up),
            Key::Named(Named::ArrowDown) => {
                let to = self.cursor.map_or(0, |c| c + 1);
                self.move_cursor(to)
            }
            Key::Named(Named::ArrowUp) => {
                let to = self.cursor.map_or(last, |c| c.saturating_sub(1));
                self.move_cursor(to)
            }
            Key::Named(Named::Home) => self.move_cursor(0),
            Key::Named(Named::End) => self.move_cursor(last),
            Key::Named(Named::Backspace) => self.update(Message::Up),
            Key::Named(Named::Enter) => self.run(Command::Open),
            Key::Named(Named::F2) => self.run(Command::Rename),
            Key::Named(Named::F5) => self.run(Command::Refresh),
            Key::Named(Named::Delete) => self.run(Command::Trash),
            Key::Named(Named::Space) => self.run(Command::TogglePreview),
            Key::Named(Named::ContextMenu) => {
                self.open_menu_at_cursor();
                Task::none()
            }
            Key::Named(Named::F10) if shift => {
                self.open_menu_at_cursor();
                Task::none()
            }
            Key::Named(Named::Escape) => {
                self.search.clear();
                self.notice = None;
                self.refilter();
                self.clear_selection();
                Task::none()
            }
            Key::Character(c) if ctrl => match (c.to_lowercase().as_str(), shift) {
                ("c", true) => self.run(Command::CopyPath),
                ("c", false) => self.run(Command::Copy),
                ("x", _) => self.run(Command::Cut),
                ("v", _) => self.run(Command::Paste),
                ("d", _) => self.run(Command::Duplicate),
                ("n", true) => self.run(Command::NewFolder),
                ("h", _) => self.run(Command::ToggleHidden),
                ("a", _) => self.run(Command::SelectAll),
                ("f", _) => operation::focus(SEARCH),
                ("r", _) => self.run(Command::Refresh),
                _ => Task::none(),
            },
            _ => Task::none(),
        }
    }

    // Commands.

    fn current_dir(&self) -> Option<PathBuf> {
        match &self.location {
            Location::Folder(dir) => Some(dir.clone()),
            Location::Trash => None,
        }
    }

    fn run(&mut self, command: Command) -> Task<Message> {
        let selected = self.selected_paths();
        let in_trash = self.location == Location::Trash;
        match command {
            Command::Open if in_trash => {}
            Command::Open => return self.open(),
            Command::OpenTerminal => {
                if let Some(dir) = self.current_dir() {
                    let mut terminal = Process::new("x-terminal-emulator");
                    terminal.current_dir(dir);
                    return self.launch(terminal);
                }
            }
            Command::Rename if !in_trash => return self.start_rename(),
            Command::Duplicate if !in_trash && !selected.is_empty() => {
                let text = texts::copying(selected.len());
                return self.spawn(text, move || done(ops::duplicate(&selected)?));
            }
            Command::CopyPath if !selected.is_empty() => {
                let paths: Vec<String> = selected.iter().map(|p| p.display().to_string()).collect();
                self.notice = Some(Notice {
                    text: texts::PATH_COPIED.into(),
                    error: false,
                });
                return iced::clipboard::write(paths.join("\n"));
            }
            Command::Copy if !in_trash && !selected.is_empty() => {
                self.clipboard = Some((Clip::Copy, selected));
            }
            Command::Cut if !in_trash && !selected.is_empty() => {
                self.clipboard = Some((Clip::Cut, selected));
            }
            Command::Paste => return self.paste(),
            Command::NewFolder => return self.new_folder(),
            Command::ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                return self.load(&[]);
            }
            Command::Trash if !in_trash && !selected.is_empty() => {
                let Some(trash) = self.config.trash.clone() else {
                    return Task::none();
                };
                let count = selected.len();
                return self.spawn(texts::MOVE_TO_TRASH.into(), move || {
                    for path in &selected {
                        trash.put(path)?;
                    }
                    Ok((texts::trashed(count), Vec::new()))
                });
            }
            Command::Restore if in_trash => return self.restore(),
            Command::EmptyTrash if in_trash => {
                self.empty_ids = self
                    .all
                    .iter()
                    .filter_map(|e| e.path.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .collect();
                self.confirm_empty = !self.empty_ids.is_empty();
            }
            Command::Refresh => return self.load(&[]),
            Command::TogglePreview => self.show_preview = !self.show_preview,
            Command::DeviceOpen | Command::DeviceUnmount | Command::DeviceEject => {
                let Some(id) = self.device_menu.clone() else {
                    return Task::none();
                };
                let action = match command {
                    Command::DeviceUnmount => DeviceAction::Unmount,
                    Command::DeviceEject => DeviceAction::Eject,
                    _ => DeviceAction::Open,
                };
                return self.device(action, id);
            }
            Command::ChooseApp => {
                self.chooser = Some(String::new());
                return operation::focus(CHOOSER_SEARCH);
            }
            Command::ToggleAlways => self.always = !self.always,
            Command::SelectAll => {
                self.selection = (0..self.entries.len()).collect();
                self.cursor = self.selection.iter().next().copied();
            }
            _ => {}
        }
        Task::none()
    }

    /// Runs an operation off the interface thread: big copies do not block it.
    fn spawn(
        &mut self,
        text: String,
        job: impl FnOnce() -> ops::Result<(String, Vec<PathBuf>)> + Send + 'static,
    ) -> Task<Message> {
        self.busy += 1;
        self.notice = (!text.is_empty()).then_some(Notice { text, error: false });
        Task::perform(
            async move {
                job().map_err(|error| {
                    tracing::warn!(%error, "operation refused");
                    error.to_string()
                })
            },
            Message::Done,
        )
    }

    fn open(&mut self) -> Task<Message> {
        let Some(entry) = self.cursor.and_then(|i| self.entries.get(i)) else {
            return Task::none();
        };
        if entry.kind == Kind::Folder && self.location != Location::Trash {
            let path = entry.path.clone();
            return self.go(Location::Folder(path));
        }
        // The default application for the type, as mimeapps.list says; xdg-open when there
        // is none or the catalog is not read yet.
        let default = self.selected_type().and_then(|mime| {
            self.catalog
                .as_ref()?
                .default_for(&mime)
                .map(|a| a.id.clone())
        });
        if let Some(id) = default {
            return self.open_with_app_once(&id);
        }
        let Some(entry) = self.cursor.and_then(|i| self.entries.get(i)) else {
            return Task::none();
        };
        let mut open = Process::new("xdg-open");
        open.arg(&entry.path);
        self.launch(open)
    }

    /// Opens with an application without touching the defaults.
    fn open_with_app_once(&mut self, id: &str) -> Task<Message> {
        let always = std::mem::replace(&mut self.always, false);
        let task = self.open_with_app(id);
        self.always = always;
        task
    }

    /// Starts a program off the interface thread, without waiting for it; a thread reaps
    /// it when it ends.
    fn launch(&mut self, mut command: Process) -> Task<Message> {
        Task::perform(
            async move {
                match command.spawn() {
                    Ok(mut child) => {
                        std::thread::spawn(move || child.wait());
                        true
                    }
                    Err(error) => {
                        tracing::warn!(%error, "launch failed");
                        false
                    }
                }
            },
            Message::Launched,
        )
    }

    fn paste(&mut self) -> Task<Message> {
        let (Some(dest), Some((mode, sources))) = (self.current_dir(), self.clipboard.clone())
        else {
            return Task::none();
        };
        match mode {
            Clip::Copy => self.spawn(texts::copying(sources.len()), move || {
                done(ops::copy_into(&sources, &dest)?)
            }),
            Clip::Cut => {
                // A cut item is pasted only once.
                self.clipboard = None;
                self.spawn(texts::moving(sources.len()), move || {
                    done(ops::move_into(&sources, &dest)?)
                })
            }
        }
    }

    fn new_folder(&mut self) -> Task<Message> {
        let Some(dir) = self.current_dir() else {
            return Task::none();
        };
        // The folder is created empty, the search would hide it. The rename opens when the
        // reread listing has selected it.
        self.search.clear();
        self.pending_rename = true;
        self.spawn(String::new(), move || {
            let path = ops::new_folder(&dir)?;
            Ok((String::new(), vec![path]))
        })
    }

    fn start_rename(&mut self) -> Task<Message> {
        let Some((index, entry)) = self.cursor.zip(self.single().cloned()) else {
            return Task::none();
        };
        // The name is selected without its extension, as everywhere else.
        let stem = match (entry.kind, entry.name.rfind('.')) {
            (Kind::File, Some(dot)) if dot > 0 => entry.name[..dot].chars().count(),
            _ => entry.name.chars().count(),
        };
        self.renaming = Some((index, entry.name));
        Task::batch([
            operation::focus(RENAME),
            operation::select_range(RENAME, 0, stem),
        ])
    }

    fn finish_rename(&mut self) -> Task<Message> {
        let Some((index, name)) = self.renaming.take() else {
            return Task::none();
        };
        let Some(entry) = self.entries.get(index) else {
            return Task::none();
        };
        let path = entry.path.clone();
        self.spawn(String::new(), move || {
            let renamed = ops::rename(&path, &name)?;
            Ok((String::new(), vec![renamed]))
        })
    }

    fn restore(&mut self) -> Task<Message> {
        let Some(trash) = self.config.trash.clone() else {
            return Task::none();
        };
        let ids: Vec<String> = self
            .selected_paths()
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        let count = ids.len();
        self.spawn(String::new(), move || {
            for id in &ids {
                trash.restore(id)?;
            }
            Ok((texts::restored(count), Vec::new()))
        })
    }

    fn empty_trash(&mut self) -> Task<Message> {
        let Some(trash) = self.config.trash.clone() else {
            return Task::none();
        };
        // Only what the user saw when asked is deleted.
        let ids = std::mem::take(&mut self.empty_ids);
        self.spawn(texts::EMPTY_TRASH.into(), move || {
            trash.remove(&ids)?;
            Ok((texts::TRASH_EMPTIED.into(), Vec::new()))
        })
    }

    /// The path shown by the breadcrumb: segments and the folder of each. Under the home
    /// folder, the path starts from "Accueil".
    pub fn breadcrumb(&self) -> Vec<(String, Location)> {
        let dir = match &self.location {
            Location::Trash => return vec![(texts::TRASH.into(), Location::Trash)],
            Location::Folder(dir) => dir,
        };
        let home = &self.config.home;
        let (mut path, mut crumbs, rest) = match dir.strip_prefix(home) {
            Ok(rest) => (
                home.clone(),
                vec![(texts::HOME.to_owned(), Location::Folder(home.clone()))],
                rest,
            ),
            Err(_) => (
                PathBuf::from("/"),
                vec![(texts::ROOT.to_owned(), Location::Folder(PathBuf::from("/")))],
                dir.strip_prefix("/").unwrap_or(dir),
            ),
        };
        for part in rest.iter() {
            path.push(part);
            crumbs.push((
                part.to_string_lossy().into_owned(),
                Location::Folder(path.clone()),
            ));
        }
        crumbs
    }

    /// The original folder of a trash item, for the listing.
    pub fn origin_label(&self, origin: &Path) -> String {
        let parent = origin.parent().unwrap_or(origin);
        match parent.strip_prefix(&self.config.home) {
            Ok(rest) if rest.as_os_str().is_empty() => texts::HOME.into(),
            Ok(rest) => rest.display().to_string(),
            Err(_) => parent.display().to_string(),
        }
    }
}

fn direction(modifiers: Modifiers) -> Direction {
    if modifiers.shift() {
        Direction::Previous
    } else {
        Direction::Next
    }
}

/// Reads a location: a folder, or the trash with the original name and place of each item.
/// Off the interface thread.
fn read_location(
    location: &Location,
    hidden: bool,
    trash: Option<&Trash>,
) -> Result<Vec<Entry>, ()> {
    let now = crate::clock::now();
    match location {
        Location::Folder(dir) => listing::read_dir(dir, hidden).map_err(|_| ()),
        Location::Trash => {
            let items = trash.and_then(|t| t.list().ok()).unwrap_or_default();
            Ok(items
                .into_iter()
                .filter_map(|item| {
                    let mut entry = Entry::read(&item.path, true).ok()?;
                    if let Some(name) = item.original.file_name() {
                        entry.name = name.to_string_lossy().into_owned();
                    }
                    entry.modified = item.deleted;
                    entry.origin = Some(item.original);
                    entry.relabel(now);
                    Some(entry)
                })
                .collect())
        }
    }
}

/// The message of a finished copy or move, and what to select.
fn done(report: ops::Report) -> ops::Result<(String, Vec<PathBuf>)> {
    let text = match report.skipped {
        0 => texts::DONE.to_owned(),
        n => texts::done_skipping(n),
    };
    Ok((text, report.paths))
}

/// Locales to try for application names, most specific first: `fr_FR`, then `fr`.
fn locales() -> Vec<String> {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let base = raw.split(['.', '@']).next().unwrap_or_default().to_owned();
    let mut out = Vec::new();
    if !base.is_empty() && base != "C" && base != "POSIX" {
        if let Some((language, _)) = base.split_once('_') {
            out.push(base.clone());
            out.push(language.to_owned());
        } else {
            out.push(base);
        }
    }
    out
}
