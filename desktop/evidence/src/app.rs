//! Application state and the response to each message: navigation, selection, operations,
//! keyboard. The view lives in `view`.

use std::path::{Path, PathBuf};
use std::process::Command as Process;

mod drag;

use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::widget::{operation, scrollable};
use iced::{Event, Point, Size, Subscription, Task, event, mouse, window};
use iced_exwlshell::to_layer_message;
use sherlock_ui::click::DoubleClick;
use sherlock_ui::color::{Theme, ThemeFade};
use sherlock_ui::evidence as more;
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::texts::evidence as texts;
use sherlock_ui::{metrics, typography};

use crate::devices::{DeviceError, DeviceSource, Volume};
use crate::git::{self, Repo};
use crate::icons::AppIcon;
use crate::listing::{self, Column, Entry, Kind, Sort};
use crate::menu::{Closing, MenuMotion};
use crate::ops;
use crate::places::{self, Place};
use crate::preview::{self, Preview};
use crate::probe::{Probe, Step};
use crate::selection::{Click, Lasso, Selection, edge_scroll};
use crate::trash::Trash;
use crate::udisks::UDisks;
use sherlock_ui::motion::{self, Animation, Fade};

mod network;
mod path;
mod popup;
mod tabs;
pub use path::{FoldedMenu, PathEdit};
pub use popup::Popups;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// The themes of the appearance file at `path`, live (`appearance_config::follow_themes`).
pub fn appearance(path: &Path) -> UnboundedReceiver<Theme> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let follow = appearance_config::follow_themes(path.to_path_buf(), move |theme| {
        sender.unbounded_send(theme).is_ok()
    });
    if let Err(error) = follow {
        tracing::warn!(%error, "Evidence will not follow the appearance file");
    }
    receiver
}

/// What the start log reports once.
static OPENED: AtomicBool = AtomicBool::new(false);
static LISTED: AtomicBool = AtomicBool::new(false);
static CATALOG: AtomicBool = AtomicBool::new(false);
static DEVICES: AtomicBool = AtomicBool::new(false);
pub(crate) static FIRST_VIEW: AtomicBool = AtomicBool::new(false);

/// Identifiers of the widgets the application drives.
pub const LIST: &str = "list";
pub const SEARCH: &str = "search";
pub const RENAME: &str = "rename";
pub const CHOOSER_SEARCH: &str = "chooser-search";
pub const PATH: &str = "path";
pub const SUGGESTIONS: &str = "path-suggestions";

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
            // Not connected yet: the bus is reached off the interface thread, on first use.
            devices: Some(Arc::new(UDisks::system()) as Arc<dyn DeviceSource>),
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
    /// Sort by a column, from the « Trier par » submenu: the same column again reverses.
    SortBy(Column),
    /// The large preview of the item under the cursor, opened or closed (lot E-12).
    QuickLook,
    /// The list view, from the « Vue » submenu: the only view so far.
    ViewList,
    /// The path bar as a field (Ctrl+L), and the « Aller au dossier » sheet (Maj+Ctrl+G).
    EditPath,
    GoToFolder,
}

/// The submenus of the right-click menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sub {
    OpenWith,
    SortBy,
    View,
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
    /// Opens a submenu: « Ouvrir avec », « Trier par », « Vue ».
    Submenu(Sub),
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
        matches!(self, Self::Action { .. } | Self::Submenu(_) | Self::App(_))
    }
}

/// The open right-click menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    /// Top left corner, in the window.
    pub at: Point,
    /// Opened on the selection, or on the background of the listing.
    pub on_selection: bool,
    pub submenu: Option<Sub>,
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

/// The first state of the menus in the bar: the subscription starts once, whatever it holds.
struct MenubarStart(crate::menubar::State);

impl std::hash::Hash for MenubarStart {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "menubar".hash(state);
    }
}

/// On Wayland, the messages also carry the surface requests of `iced_exwlshell` (ADR 0029):
/// the window, and later the menus as popups.
#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    /// The menus in the bar: served, or an entry clicked there (lot D-36c).
    MenuBar(crate::menubar::Event),
    Go(Location),
    Back,
    Forward,
    Up,
    Click(usize),
    RightClick(Option<usize>),
    Sort(Column),
    Search(String),
    Run(Command),
    Submenu(Option<Sub>),
    CloseMenu,
    /// A right click while the menu is open: it opens again where the pointer is.
    Reopen,
    /// What a closing menu's entries send: nothing.
    Inert,
    /// Click on the background of the listing.
    Blank,
    Key(keyboard::Event),
    Modifiers(Modifiers),
    /// The pointer, in the window, read by the view itself: it arrives before a press in the
    /// same batch of events, where a subscription's message would come after it.
    Cursor(Point),
    /// The left button went up, wherever the pointer is: a lasso ends.
    Released,
    Resized(Size),
    /// The window exists (for the start log).
    Opened,
    Scrolled(f32, f32),
    RenameInput(String),
    RenameSubmit,
    ConfirmEmpty,
    CancelEmpty,
    Done(Outcome),
    /// The names a paste would take, found off the interface thread: the items, where they
    /// go, whether they move, and the names already there (lot E-11).
    Taken(Vec<PathBuf>, PathBuf, bool, Vec<String>),
    /// The choice for the taken names; none: the paste is called off.
    TakenChoice(Option<crate::transfer_job::Taken>),
    /// Stop the copy or the move under way.
    StopTransfer,
    Git(PathBuf, Option<Repo>),
    Window(WindowAction),
    /// A frame of the window, during an animation or a measurement.
    Frame(Instant),
    /// The theme of the appearance file, at start then at each change.
    Theme(Theme),
    /// Content of a location, read off the interface thread. The number discards a read
    /// overtaken by a more recent navigation.
    Loaded(u64, Result<Vec<Entry>, ()>),
    /// Preview prepared off the interface thread.
    Preview(PathBuf, Preview, Option<preview::Real>),
    /// A program was started, or could not be.
    Launched(bool),
    /// A program or a launcher was not handed to xdg-open.
    ProgramRefused,
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
    /// Tabs (lot D-27b): a new one on this folder, one closed, one shown, one pressed (to
    /// drag), the pointer over one while dragging, a folder opened in a tab by the middle
    /// button.
    NewTab,
    CloseTab(usize),
    SwitchTab(usize),
    PressTab(usize),
    OverTab(usize),
    MiddleClick(usize),
    /// The file servers of the network, found now (lot E-07).
    Servers(Vec<crate::network::Server>),
    /// A click on a server: connect to it, or open it when it is.
    OpenServer(String),
    /// The eject button beside a connected server.
    EjectServer(String),
    /// A server connected (its folder), or not; disconnected, or not.
    ServerMounted(String, Result<PathBuf, crate::smb::Error>),
    ServerUnmounted(String, Result<(), crate::smb::Error>),
    /// An action on a device is over.
    DeviceDone(DeviceAction, Result<Option<PathBuf>, DeviceError>),
    /// The window's Wayland handles, for drag and drop with the other applications (lot E-03).
    DndReady(Option<crate::dnd::Handles>),
    /// Drag and drop and the clipboard of files, from the Wayland side.
    Dnd(crate::dnd::Event),
    /// The path field: typed, Enter, a suggestion clicked, closed.
    PathInput(String),
    PathSubmit,
    PathPick(usize),
    PathCancel,
    /// Subfolders read for the completion, with the number of the read.
    PathFolders(u64, Option<crate::pathbar::Folders>),
    /// What the typed path leads to: a folder, something else, nothing.
    PathResolved(PathBuf, crate::pathbar::Found),
    /// A window closed: the main one, or a menu's popup (`popup_done`).
    WindowClosed(window::Id),
    /// Escape, taken by a focused field: it still closes the path field.
    Escaped,
    /// The menu of the folders folded behind « … », opened or closed.
    Folded(bool),
    /// The completion list scrolled to this offset.
    PathScrolled(f32),
    /// A recent folder clicked in the « Aller au dossier » sheet.
    PathRecent(usize),
    /// The search, folded to its icon in a narrow window, opens.
    OpenSearch,
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
    /// The `xdg_toplevel` opened by the Wayland daemon (`shell`); none under winit.
    main: Option<window::Id>,
    /// The popups of the menu and its submenu, on Wayland (`popup`).
    pub(crate) popups: Popups,
    /// The menus exported to the bar, once served (lot D-36c).
    menubar: Option<crate::menubar::MenuBar>,
    pub(crate) config: Config,
    pub(crate) location: Location,
    history: Vec<Location>,
    future: Vec<Location>,
    /// The whole folder, then what the listing shows (search applied, sorted).
    all: Vec<Entry>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) selection: Selection,
    /// The rectangle being dragged on the background of the listing.
    pub(crate) lasso: Option<Lasso>,
    /// The last frame that scrolled under the lasso.
    lasso_frame: Option<Instant>,
    pub(crate) sort: Sort,
    pub(crate) show_hidden: bool,
    pub(crate) show_preview: bool,
    /// The preview sliding in or out (lot E-08), and the file that remembers it.
    pub(crate) preview_slide: Option<Fade>,
    prefs: Option<PathBuf>,
    /// The large preview is open, and its zoom in or out (lot E-12).
    pub(crate) looking: bool,
    pub(crate) look_fade: Option<Fade>,
    /// The preview was asked for while the width had folded it: it stays, the columns fold.
    pub(crate) preview_pinned: bool,
    pub(crate) search: String,
    pub(crate) repo: Option<Repo>,
    pub(crate) preview: Preview,
    /// What the selected file really is, when its name hides it (audit I-3).
    pub(crate) disguise: Option<preview::Real>,
    clipboard: Option<(Clip, Vec<PathBuf>)>,
    /// Item being renamed and its new name.
    pub(crate) renaming: Option<(PathBuf, String)>,
    pub(crate) menu: Option<Menu>,
    /// The menu unfolding or folding back, and the menu that is closing.
    pub(crate) menu_motion: MenuMotion,
    pub(crate) closing: Option<Closing>,
    pub(crate) confirm_empty: bool,
    pub(crate) notice: Option<Notice>,
    pub(crate) busy: usize,
    pointer: Point,
    /// Presses on the rows, by path, to recognise a double click whatever the first click
    /// changes in the view or a reload in the order of the rows.
    pub(crate) clicks: DoubleClick<PathBuf>,
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
    /// The tabs of the window (lot D-27b).
    pub(crate) tabs: tabs::Tabs,
    /// The copy or move under way, and the paste waiting for a choice on taken names.
    pub(crate) progress: Option<Arc<crate::transfer_job::Progress>>,
    pub(crate) pending_paste: Option<(Vec<PathBuf>, PathBuf, bool, Vec<String>)>,
    /// The servers of the network, and the folders of those connected, by host.
    pub(crate) servers: Vec<crate::network::Server>,
    pub(crate) mounted: HashMap<String, PathBuf>,
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
    /// The theme of the appearance file, shown with a fade.
    pub(crate) theme: ThemeFade,
    /// Drag and drop and the clipboard of files with the other applications (lot E-03).
    pub(crate) dnd: drag::Dnd,
    /// The path field, open or fading out, and how far it has come in.
    pub(crate) path_edit: Option<PathEdit>,
    pub(crate) path_leaving: Option<PathEdit>,
    path_fade: Option<Fade>,
    /// The completion list: what it last showed, kept while it fades out, and its fade.
    pub(crate) suggestions_shown: Vec<String>,
    pub(crate) suggest_fade: Option<Fade>,
    /// The folder the completion last read, and the number of the last read asked.
    folders: Option<crate::pathbar::Folders>,
    folders_generation: u64,
    /// The menu of the folded folders, the completion list's scroll, the folders last
    /// reached by the field (with their labels), and the search opened from its icon.
    pub(crate) folded: FoldedMenu,
    suggest_offset: f32,
    pub(crate) recent: Vec<(PathBuf, String)>,
    pub(crate) search_open: bool,
}

impl App {
    pub fn new(config: Config) -> (Self, Task<Message>) {
        let built = Instant::now();
        let start = Location::Folder(config.start.clone());
        let (width, height) = metrics::REFERENCE_WINDOW;
        let mut app = Self {
            main: None,
            popups: Popups::default(),
            menubar: None,
            config,
            location: start,
            history: Vec::new(),
            future: Vec::new(),
            all: Vec::new(),
            entries: Vec::new(),
            selection: Selection::default(),
            lasso: None,
            lasso_frame: None,
            sort: Sort::default(),
            show_hidden: false,
            show_preview: true,
            preview_slide: None,
            prefs: None,
            looking: false,
            look_fade: None,
            preview_pinned: false,
            search: String::new(),
            repo: None,
            preview: Preview::Info,
            disguise: None,
            clipboard: None,
            renaming: None,
            menu: None,
            menu_motion: MenuMotion::default(),
            closing: None,
            confirm_empty: false,
            notice: None,
            busy: 0,
            pointer: Point::ORIGIN,
            clicks: DoubleClick::default(),
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
            tabs: tabs::Tabs::default(),
            progress: None,
            pending_paste: None,
            servers: Vec::new(),
            mounted: HashMap::new(),
            device_menu: None,
            available: Vec::new(),
            empty_ids: Vec::new(),
            catalog: None,
            open_with: Vec::new(),
            always: false,
            chooser: None,
            icons: Arc::default(),
            theme: ThemeFade::default(),
            dnd: drag::Dnd::default(),
            path_edit: None,
            path_leaving: None,
            path_fade: None,
            suggestions_shown: Vec::new(),
            suggest_fade: None,
            folders: None,
            folders_generation: 0,
            folded: FoldedMenu::default(),
            suggest_offset: 0.0,
            recent: Vec::new(),
            search_open: false,
        };
        app.probe = app.config.measure.clone().map(Probe::new);
        // The preview as it was left (lot E-08): a file of a few bytes, read before the first
        // frame so that the window opens as it was.
        app.prefs = app
            .config
            .xdg
            .as_ref()
            .map(|dirs| crate::prefs::path(&dirs.config_home));
        if let Some(shown) = app.prefs.as_deref().and_then(crate::prefs::read_preview) {
            app.show_preview = shown;
        }
        let task = app.load(&[]);
        let devices = app.refresh_devices();
        let catalog = app.load_catalog();
        crate::startup::step("application built", built.elapsed());
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
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(Named::Escape),
                ..
            }) => Some(Message::Escaped),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                Some(Message::Released)
            }
            // The side buttons of a mouse, as in every file manager.
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Back)) => Some(Message::Back),
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Forward)) => {
                Some(Message::Forward)
            }
            Event::Window(window::Event::Resized(size)) => Some(Message::Resized(size)),
            Event::Window(window::Event::Opened { .. }) => Some(Message::Opened),
            // The compositor closed the window (Wayland daemon): the program ends with it.
            _ => None,
        });
        // A window closed: the main one ends the program, a menu's popup closes its menu.
        let closed = window::close_events().map(Message::WindowClosed);
        let devices = if self.config.watch_devices && self.config.devices.is_some() {
            Subscription::run(crate::udisks::stream).map(Message::Devices)
        } else {
            Subscription::none()
        };
        let appearance = match appearance_config::path() {
            Some(path) => Subscription::run_with(path, |path| appearance(path)).map(Message::Theme),
            None => Subscription::none(),
        };
        let menubar = if crate::menubar::enabled() {
            let state = self.menubar_state();
            Subscription::run_with(MenubarStart(state), |start| {
                crate::menubar::events(start.0.clone())
            })
            .map(Message::MenuBar)
        } else {
            Subscription::none()
        };
        // The network is listened to only with the live sources (not in the tests).
        let network = if self.config.watch_devices {
            Subscription::run(crate::network::stream).map(Message::Servers)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            closed, events, frames, devices, network, appearance, menubar,
        ])
    }

    /// Asks the Wayland daemon for the window (`shell`), and keeps its id.
    pub fn open_window(
        &mut self,
        settings: iced_exwlshell::actions::IcedXdgWindowSettings,
    ) -> Task<Message> {
        let (id, open) = Message::base_window_open(settings);
        self.main = Some(id);
        open
    }

    /// `act` on the main window: the one opened on Wayland (`shell`), else iced's only window.
    fn on_window(
        &self,
        act: impl Fn(window::Id) -> Task<Message> + Send + 'static,
    ) -> Task<Message> {
        match self.main {
            Some(id) => act(id),
            None => window::latest().and_then(act),
        }
    }

    /// The view of a window of the Wayland daemon: Evidence has only its main window.
    pub fn view_window(&self, window: window::Id) -> iced::Element<'_, Message> {
        self.popup_view(window).unwrap_or_else(|| self.view())
    }

    /// Window title: the application name only. The folder is shown in the drawn title bar.
    pub fn title(&self) -> String {
        texts::APP_NAME.to_owned()
    }

    /// Name of the current folder, for the title bar and the search.
    pub fn location_name(&self) -> String {
        self.name_of(&self.location)
    }

    /// Name of a location: « Accueil », « Corbeille », « Disque », or its folder's name.
    pub fn name_of(&self, location: &Location) -> String {
        match location {
            Location::Trash => texts::TRASH.into(),
            Location::Folder(dir) if *dir == self.config.home => texts::HOME.into(),
            Location::Folder(dir) => dir
                .file_name()
                .map_or_else(|| texts::ROOT.into(), |n| n.to_string_lossy().into_owned()),
        }
    }

    /// The panes and columns the window's width leaves room for (lot E-05).
    pub fn panes(&self) -> crate::layout::Panes {
        crate::layout::Panes::at(self.size.width, self.show_preview, self.preview_pinned)
    }

    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.future.is_empty()
    }

    /// Shows or hides the preview, sliding, and remembers it off the interface thread.
    fn toggle_preview(&mut self) -> Task<Message> {
        let from = self.preview_level();
        self.show_preview = !self.show_preview;
        self.preview_pinned = false;
        let now = self.now();
        let to = if self.panes().preview { 1.0 } else { 0.0 };
        self.preview_slide = Some(Fade::new(from, to, now, more::PREVIEW_SLIDE));
        let Some(path) = self.prefs.clone() else {
            return Task::none();
        };
        let shown = self.show_preview;
        Task::perform(
            async move {
                if let Err(error) = crate::prefs::write_preview(&path, shown) {
                    tracing::warn!(%error, "the preview choice is not remembered");
                }
            },
            |()| Message::Inert,
        )
    }

    /// How far the preview is in, from 0 (hidden) to 1 (shown).
    pub(crate) fn preview_level(&self) -> f32 {
        // What the width leaves room for (E-05), at rest.
        let rest = if self.panes().preview { 1.0 } else { 0.0 };
        self.preview_slide
            .as_ref()
            .map_or(rest, |f| f.value(self.now()))
    }

    /// Opens or closes the large preview, zooming in or out. It opens on one item only.
    fn quick_look(&mut self, open: bool) {
        if open && self.single().is_none() {
            return;
        }
        let now = self.now();
        let from = self.look_level();
        self.looking = open;
        let to = if open { 1.0 } else { 0.0 };
        self.look_fade = Some(Fade::new(from, to, now, more::QUICK_LOOK_ZOOM));
    }

    /// How far the large preview has come in, from 0 to 1.
    pub(crate) fn look_level(&self) -> f32 {
        let rest = if self.looking { 1.0 } else { 0.0 };
        self.look_fade
            .as_ref()
            .map_or(rest, |f| f.value(self.now()))
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
        self.theme.running()
            || self.lasso_speed() != 0.0
            || ((self.menu.is_some() || self.closing.is_some()) && self.menu_motion.running(now))
            || spring(&self.highlight, motion::GEOMETRY_PRECISION)
            || fade(&self.content_fade)
            || fade(&self.preview_fade)
            || fade(&self.tabs.fade)
            || fade(&self.preview_slide)
            || fade(&self.look_fade)
            || self.progress.is_some()
            || self.dnd_waiting()
            || fade(&self.path_fade)
            || fade(&self.suggest_fade)
            || fade(&self.folded.fade)
            || self
                .path_edit
                .as_ref()
                .and_then(|e| e.shake)
                .is_some_and(|at| now < at + more::PATH_SHAKE)
    }

    /// Sets every animation to its end. For captures.
    pub fn settle(&mut self) {
        let now = self.now();
        self.content_fade = None;
        self.preview_fade = None;
        self.preview_slide = None;
        self.look_fade = None;
        self.path_fade = None;
        self.path_leaving = None;
        self.suggest_fade = None;
        self.folded.fade = None;
        if let Some(edit) = &mut self.path_edit {
            edit.shake = None;
        }
        self.menu_motion = MenuMotion::default();
        self.closing = None;
        if let Some(target) = self.highlight.as_ref().map(Animation::target) {
            self.highlight = Some(Animation::new(motion::PREVIEW_SPRING, target, target, now));
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        // What the open menu shows, to draw it while it folds back if this message closes it.
        let shown = self.menu.as_ref().map(|menu| Closing {
            menu: menu.clone(),
            items: self.menu_items(menu.on_selection),
            sub: menu
                .submenu
                .map(|s| self.submenu_items(s))
                .unwrap_or_default(),
        });
        // A click or a move elsewhere gives the breadcrumb back, as in Nautilus.
        let elsewhere = matches!(
            message,
            Message::Click(_)
                | Message::Blank
                | Message::RightClick(_)
                | Message::Back
                | Message::Forward
                | Message::Up
                | Message::Go(_)
        );
        if elsewhere && self.path_edit.as_ref().is_some_and(|e| !e.sheet) {
            self.close_path_edit();
        }
        if elsewhere {
            self.folded_menu(false);
        }
        let task = self.handle(message);
        self.sync_menubar();
        let now = self.now();
        if self.menu.is_some() {
            self.closing = None;
        } else if shown.is_some() && !std::mem::take(&mut self.popups.dismissed) {
            // Folds back, unless the compositor already took its popup away.
            self.closing = shown;
            self.menu_motion.close(now);
        } else if self.closing.is_some() && self.menu_motion.gone(now) {
            self.closing = None;
        }
        // The highlight follows the item when it is selected alone.
        let target = self.selection.single().map(|i| {
            f64::from(metrics::LIST_TOP_GAP) + i as f64 * f64::from(metrics::LIST_ROW_HEIGHT)
        });
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
        let popups = self.sync_popups();
        Task::batch([task, self.preview_task(), popups])
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
        self.disguise = None;
        match (wanted, path) {
            (Some(entry), Some(path)) => Task::perform(
                async move { (preview::load(&entry), preview::disguise(&entry)) },
                move |(preview, disguise)| Message::Preview(path.clone(), preview, disguise),
            ),
            _ => Task::none(),
        }
    }

    /// What the menus in the bar depend on, now.
    pub fn menubar_state(&self) -> crate::menubar::State {
        crate::menubar::State {
            selection: !self.selection.is_empty(),
            in_trash: self.location == Location::Trash,
            show_hidden: self.show_hidden,
            show_preview: self.show_preview,
            can_back: self.can_go_back(),
            can_forward: self.can_go_forward(),
            places: self
                .config
                .favorites
                .iter()
                .map(|p| (p.label, p.path.clone()))
                .collect(),
        }
    }

    /// Tells the menus in the bar what changed.
    fn sync_menubar(&self) {
        if let Some(menubar) = &self.menubar {
            menubar.update(self.menubar_state());
        }
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::MenuBar(crate::menubar::Event::Ready(menubar)) => self.menubar = Some(menubar),
            Message::MenuBar(crate::menubar::Event::Click(action)) => {
                use crate::menubar::Action;
                let message = match action {
                    Action::Run(command) => Message::Run(command),
                    Action::Back => Message::Back,
                    Action::Forward => Message::Forward,
                    Action::Up => Message::Up,
                    Action::Go(Some(path)) => Message::Go(Location::Folder(path)),
                    Action::Go(None) => Message::Go(Location::Trash),
                    Action::Window(action) => Message::Window(action),
                };
                return self.update(message);
            }
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
                let base = self.lasso_base();
                let how = Click::new(self.modifiers.command(), self.modifiers.shift());
                // A press on the selection may become a drag out of the window (lot E-03).
                let on_name = !self.beside_name(index);
                if self.dnd_press(index, how == Click::Only, on_name) {
                    self.selection.click(index, how);
                }
                // Beside the name, a press may start a lasso too, as in Dolphin: a long
                // listing has no background left to start one from.
                if self.beside_name(index) {
                    self.lasso = Some(Lasso::new(self.in_list(self.pointer), base));
                }
                // The first click selects, which changes the view: the second is recognised
                // here, not by the row widget, whose state that change may drop.
                let position = (self.pointer.x, self.pointer.y);
                let target = self.entries.get(index).map(|e| e.path.clone());
                if let Some(target) = target
                    && self.clicks.press(target, position, self.now())
                {
                    self.select_only(index);
                    return self.open();
                }
            }
            Message::RightClick(index) => {
                self.renaming = None;
                match index {
                    Some(index) if self.selection.contains(index) => self.selection.point(index),
                    Some(index) => self.select_only(index),
                    None => {}
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
            Message::Submenu(sub) => self.open_submenu(sub, None),
            Message::CloseMenu => self.menu = None,
            Message::Reopen => return self.reopen(),
            Message::Inert => {}
            Message::Blank => {
                self.menu = None;
                self.renaming = None;
                // A press on the background may start a lasso; with Ctrl or Shift held, it
                // adds to the selection.
                let base = self.lasso_base();
                self.selection = base.clone();
                let at = self.in_list(self.pointer);
                self.lasso = Some(Lasso::new(at, base));
            }
            Message::Released => {
                self.tabs.dragging = None;
                self.dnd_release();
                self.lasso = None;
                self.lasso_frame = None;
            }
            Message::Key(event) => return self.key(event),
            Message::Modifiers(modifiers) => {
                self.modifiers = modifiers;
                self.dnd_modifiers();
            }
            Message::Cursor(position) => {
                self.pointer = position;
                self.dnd_moved();
                self.drag_lasso();
            }
            Message::Resized(size) => self.size = size,
            Message::Opened => {
                crate::startup::first(&OPENED, "window opened");
                return window::latest().and_then(|id| {
                    window::run(id, |window| crate::dnd::handles(window)).map(Message::DndReady)
                });
            }
            Message::DndReady(handles) => return self.dnd_ready(handles),
            Message::Dnd(event) => return self.dnd_event(event),
            Message::Scrolled(offset, height) => {
                self.scroll = (offset, height);
                self.drag_lasso();
            }
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
            Message::Taken(sources, dest, moving, names) => {
                if names.is_empty() {
                    return self.transfer(
                        sources,
                        dest,
                        moving,
                        crate::transfer_job::Taken::KeepBoth,
                    );
                }
                self.pending_paste = Some((sources, dest, moving, names));
            }
            Message::TakenChoice(choice) => {
                if let (Some(taken), Some((sources, dest, moving, _))) =
                    (choice, self.pending_paste.take())
                {
                    return self.transfer(sources, dest, moving, taken);
                }
                self.pending_paste = None;
            }
            Message::StopTransfer => {
                if let Some(progress) = &self.progress {
                    progress.stop();
                }
            }
            Message::Done(outcome) => {
                self.busy = self.busy.saturating_sub(1);
                if self.busy == 0 {
                    self.progress = None;
                }
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
            Message::Frame(now) => {
                self.theme.tick(now);
                let spring = self.dnd_spring();
                return Task::batch([spring, self.frame(now)]);
            }
            Message::Theme(theme) => self.theme.set(theme, Instant::now()),
            Message::Devices(volumes) => {
                crate::startup::first(&DEVICES, "devices read");
                self.volumes = volumes;
            }
            Message::Places(available) => self.available = available,
            Message::Catalog(catalog) => {
                crate::startup::first(&CATALOG, "applications read");
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
            Message::ProgramRefused => {
                self.notice = Some(Notice {
                    text: texts::IS_PROGRAM.into(),
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
            Message::NewTab => {
                let here = self.location.clone();
                return self.open_tabs(vec![here], true);
            }
            Message::CloseTab(index) => return self.close_tab(index),
            Message::SwitchTab(index) => return self.switch_tab(index),
            Message::PressTab(index) => {
                self.tabs.dragging = Some(index);
                return self.switch_tab(index);
            }
            Message::OverTab(index) => self.drag_tab(index),
            Message::MiddleClick(index) => {
                if let Some(entry) = self.entries.get(index)
                    && entry.kind == Kind::Folder
                    && self.location != Location::Trash
                {
                    let folder = Location::Folder(entry.path.clone());
                    return self.open_tabs(vec![folder], false);
                }
            }
            Message::Servers(servers) => self.servers = servers,
            Message::OpenServer(host) => return self.open_server(&host),
            Message::EjectServer(host) => return self.eject_server(&host),
            Message::ServerMounted(host, result) => return self.server_mounted(host, result),
            Message::ServerUnmounted(host, result) => self.server_unmounted(&host, result),
            Message::PathInput(text) => return self.path_input(text),
            Message::PathSubmit => return self.path_submit(),
            Message::PathPick(index) => return self.path_pick(index),
            Message::PathCancel => self.close_path_edit(),
            Message::PathFolders(generation, folders) => {
                return self.path_folders(generation, folders);
            }
            Message::PathResolved(path, found) => return self.path_resolved(path, found),
            Message::Escaped => {
                self.close_path_edit();
                self.folded_menu(false);
            }
            Message::Folded(open) => self.folded_menu(open),
            Message::PathScrolled(offset) => self.suggest_offset = offset,
            Message::PathRecent(index) => return self.path_recent(index),
            Message::OpenSearch => {
                self.search_open = true;
                return operation::focus(SEARCH);
            }
            Message::Loaded(generation, result) => {
                crate::startup::first(&LISTED, "folder listed");
                if generation == self.generation {
                    return self.loaded(result);
                }
            }
            Message::Preview(path, preview, disguise) => {
                if self.preview_path.as_ref() == Some(&path) {
                    self.preview = preview;
                    self.disguise = disguise;
                    self.preview_fade = Some(Fade::new(0.0, 1.0, self.now(), motion::FADE));
                }
            }
            // Closing the window ends the program, whichever runtime holds it: the Wayland
            // daemon would otherwise go on with no window.
            Message::Window(WindowAction::Close) => return iced::exit(),
            Message::Window(action) => {
                return self.on_window(move |id| match action {
                    WindowAction::Drag => window::drag(id),
                    WindowAction::Minimize => window::minimize(id, true),
                    WindowAction::Maximize => window::toggle_maximize(id),
                    WindowAction::Close => window::close(id),
                });
            }
            Message::WindowClosed(id) => {
                if self.main.is_none_or(|main| main == id) {
                    return iced::exit();
                }
                self.popup_closed(id);
            }
            // The surface requests of `iced_exwlshell`, which its runtime takes before `update`.
            _ => {}
        }
        Task::none()
    }

    fn frame(&mut self, now: Instant) -> Task<Message> {
        // A finished animation is dropped: the frame subscription stops.
        let t = self.now();
        // A lasso held near an edge of the listing scrolls it, at a pace in pixels per second
        // whatever the frame rate.
        let speed = self.lasso_speed();
        let last = if speed == 0.0 {
            self.lasso_frame.take()
        } else {
            self.lasso_frame.replace(now)
        };
        let step = last.map_or(Duration::ZERO, |t| now.saturating_duration_since(t));
        let step = step
            .min(sherlock_ui::evidence::LASSO_LONGEST_STEP)
            .as_secs_f32();
        let scroll = if speed == 0.0 || step == 0.0 {
            Task::none()
        } else {
            let by = scrollable::AbsoluteOffset {
                x: 0.0,
                y: speed * step,
            };
            operation::scroll_by(LIST, by)
        };
        if self.content_fade.as_ref().is_some_and(|f| f.is_done(t)) {
            self.content_fade = None;
        }
        if self.preview_fade.as_ref().is_some_and(|f| f.is_done(t)) {
            self.preview_fade = None;
        }
        if self.preview_slide.as_ref().is_some_and(|f| f.is_done(t)) {
            self.preview_slide = None;
        }
        if self.path_fade.as_ref().is_some_and(|f| f.is_done(t)) {
            self.path_fade = None;
            self.path_leaving = None;
        }
        if self.suggest_fade.as_ref().is_some_and(|f| f.is_done(t)) {
            self.suggest_fade = None;
        }
        if self.folded.fade.as_ref().is_some_and(|f| f.is_done(t)) {
            self.folded.fade = None;
        }
        let Some(probe) = &mut self.probe else {
            return scroll;
        };
        let target = Location::Folder(probe.target.clone());
        let loaded = self.location == target && !self.entries.is_empty();
        match probe.frame(now, loaded) {
            Step::Open if self.config.maximized => Task::batch([
                self.go(target),
                self.on_window(|id| window::maximize(id, true)),
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
                let next = self.selection.cursor().map_or(0, |c| c + 1);
                self.move_cursor(next % self.entries.len().max(1), false)
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
        self.search_open = false;
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
            .filter_map(|i| self.entries.get(i))
            .map(|e| e.path.clone())
            .collect()
    }

    fn select_paths(&mut self, paths: &[PathBuf]) {
        let found = self.entries.iter().enumerate();
        let found = found
            .filter(|(_, e)| paths.contains(&e.path))
            .map(|(i, _)| i);
        self.selection.set(found);
    }

    fn select_only(&mut self, index: usize) {
        self.selection.only(index);
    }

    fn clear_selection(&mut self) {
        self.selection.clear();
        self.lasso = None;
    }

    /// The selected item, if it is alone.
    pub fn single(&self) -> Option<&Entry> {
        self.selection.single().and_then(|i| self.entries.get(i))
    }

    /// Moves the cursor from the keyboard, alone or extending the selection from its anchor
    /// (Shift), and keeps the row visible.
    fn move_cursor(&mut self, to: usize, extend: bool) -> Task<Message> {
        if self.entries.is_empty() {
            return Task::none();
        }
        let index = to.min(self.entries.len() - 1);
        if extend {
            self.selection.extend_to(index);
        } else {
            self.select_only(index);
        }
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

    // Lasso.

    /// Top left corner of the listing's rows in the window, below its column headers.
    fn list_origin(&self) -> Point {
        // The tab bar, when shown, pushes the listing down (D-27b).
        let tabs = match self.tab_bar_level() {
            level if level > 0.0 => (more::TAB_BAR_HEIGHT * level).round() + metrics::BORDER_WIDTH,
            _ => 0.0,
        };
        Point::new(
            self.panes().sidebar_width() + metrics::LIST_PADDING,
            metrics::TITLE_BAR_HEIGHT
                + metrics::TOOLBAR_HEIGHT
                + metrics::LIST_HEADER_HEIGHT
                + 3.0 * metrics::BORDER_WIDTH
                + tabs,
        )
    }

    /// Height of the visible part of the listing.
    fn list_height(&self) -> f32 {
        let below = metrics::BORDER_WIDTH + metrics::STATUS_BAR_HEIGHT;
        self.size.height - self.list_origin().y - below
    }

    /// A point of the window in the content of the listing, scroll included.
    fn in_list(&self, at: Point) -> (f32, f32) {
        let origin = self.list_origin();
        (at.x - origin.x, at.y - origin.y + self.scroll.0)
    }

    /// What a lasso starting now keeps: the selection with Ctrl or Shift held, else nothing.
    fn lasso_base(&self) -> Selection {
        if self.modifiers.command() || self.modifiers.shift() {
            self.selection.clone()
        } else {
            Selection::default()
        }
    }

    /// The pointer is on the row `index` but past its icon and name.
    fn beside_name(&self, index: usize) -> bool {
        let Some(entry) = self.entries.get(index) else {
            return false;
        };
        let name = entry.name.chars().count() as f32 * typography::UI_CHAR_WIDTH;
        let end = metrics::LIST_ROW_PADDING + metrics::LIST_ICON + metrics::GAP_ICON + name;
        self.in_list(self.pointer).0 > end
    }

    /// The lasso follows the pointer, and selects the rows it crosses once it is a drag.
    fn drag_lasso(&mut self) {
        let end = self.in_list(self.pointer);
        let count = self.entries.len();
        let Some(lasso) = &mut self.lasso else {
            return;
        };
        lasso.end = end;
        let (dx, dy) = (end.0 - lasso.start.0, end.1 - lasso.start.1);
        lasso.dragging |= dx.hypot(dy) > sherlock_ui::evidence::LASSO_THRESHOLD;
        if lasso.dragging {
            let (top, row) = (metrics::LIST_TOP_GAP, metrics::LIST_ROW_HEIGHT);
            self.selection = lasso.selection(top, row, count);
        }
    }

    /// How fast the listing scrolls, in pixels per second, under a lasso held near one of its
    /// edges.
    fn lasso_speed(&self) -> f32 {
        if !self.lasso.as_ref().is_some_and(|l| l.dragging) {
            return 0.0;
        }
        let y = self.pointer.y - self.list_origin().y;
        let (offset, height) = (self.scroll.0, self.list_height());
        let content = metrics::LIST_TOP_GAP + self.entries.len() as f32 * metrics::LIST_ROW_HEIGHT;
        let speed = edge_scroll(
            y,
            height,
            sherlock_ui::evidence::LASSO_EDGE,
            sherlock_ui::evidence::LASSO_SCROLL,
        );
        // Nothing left to scroll that way.
        let room = if speed < 0.0 {
            offset
        } else {
            content - height - offset
        };
        if room > 0.5 { speed } else { 0.0 }
    }

    // Right-click menu.

    /// Menu entries: on the selection or on the background, in a folder or the trash.
    pub fn menu_items(&self, on_selection: bool) -> Vec<MenuItem> {
        use MenuItem::{Action, Separator, Submenu};
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
        let paste = match self.clipboard {
            Some(_) => MenuItem::action(texts::PASTE, Some(texts::KEY_PASTE), Command::Paste),
            None => MenuItem::Disabled(texts::PASTE),
        };
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
                Submenu(Sub::OpenWith),
                Separator,
                MenuItem::action(texts::COPY, Some(texts::KEY_COPY), Command::Copy),
                MenuItem::action(texts::CUT, Some(texts::KEY_CUT), Command::Cut),
                paste,
                Separator,
                MenuItem::action(texts::RENAME, Some(texts::KEY_RENAME), Command::Rename),
                MenuItem::action(
                    texts::DUPLICATE,
                    Some(texts::KEY_DUPLICATE),
                    Command::Duplicate,
                ),
                // Not in Evidence yet: shown, greyed.
                MenuItem::Disabled(more::COMPRESS),
                MenuItem::action(
                    texts::COPY_PATH,
                    Some(texts::KEY_COPY_PATH),
                    Command::CopyPath,
                ),
                MenuItem::Disabled(more::PROPERTIES),
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
                Submenu(Sub::SortBy),
                Submenu(Sub::View),
            ],
        }
    }

    /// Entries of a submenu.
    pub fn submenu_items(&self, sub: Sub) -> Vec<MenuItem> {
        match sub {
            Sub::OpenWith => self.open_with_items(),
            Sub::SortBy => {
                let in_trash = self.location == Location::Trash;
                let columns = [
                    (texts::COLUMN_NAME, Column::Name),
                    (texts::COLUMN_MODIFIED, Column::Modified),
                    (texts::COLUMN_SIZE, Column::Size),
                    (texts::COLUMN_KIND, Column::Kind),
                    (texts::COLUMN_GIT, Column::Git),
                ];
                columns
                    .into_iter()
                    .filter(|(_, column)| !(in_trash && *column == Column::Git))
                    .map(|(label, column)| MenuItem::Action {
                        label,
                        keys: None,
                        command: Command::SortBy(column),
                        danger: false,
                        checked: self.sort.column == column,
                    })
                    .collect()
            }
            Sub::View => vec![
                MenuItem::Action {
                    label: more::VIEW_LIST,
                    keys: None,
                    command: Command::ViewList,
                    danger: false,
                    checked: true,
                },
                MenuItem::Disabled(more::VIEW_GRID),
                MenuItem::Disabled(more::VIEW_COLUMNS),
            ],
        }
    }

    /// Entries of the « Ouvrir avec » submenu: the applications that open the selected files'
    /// type, the default first and checked, then a terminal here, the chooser of every
    /// application, and the "always" switch.
    fn open_with_items(&self) -> Vec<MenuItem> {
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

    /// MIME type of the selected files, from their names, when they all share it.
    fn selected_type(&self) -> Option<String> {
        let catalog = self.catalog.as_ref()?;
        let mut types = self
            .selection
            .iter()
            .filter_map(|i| self.entries.get(i))
            .map(|e| catalog.mime.type_of(&e.path, e.kind == Kind::Folder));
        let first = types.next()?;
        types.all(|t| t == first).then_some(first)
    }

    /// Fills the « Ouvrir avec » submenu for the selection, at most eight applications.
    fn prepare_open_with(&mut self) {
        self.open_with = match (self.selected_type(), &self.catalog) {
            (Some(mime), Some(catalog)) => catalog
                .handlers(&mime)
                .into_iter()
                .take(self.open_with_room())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
    }

    /// How many applications the « Ouvrir avec » submenu lists: at most `OPEN_WITH_MAX`, and
    /// no more than the window's height holds with the submenu's other entries; the others
    /// are in « Autre application… ».
    fn open_with_room(&self) -> usize {
        let others = Self::menu_height(&self.open_with_items()) - metrics::MENU_ITEM_HEIGHT;
        let room = ((self.size.height - others) / metrics::MENU_ITEM_HEIGHT).floor();
        (room.max(1.0) as usize).min(metrics::OPEN_WITH_MAX)
    }

    /// Height of the list of the application chooser: as on the design, less in a window too
    /// low for it, never less than a few rows; the list scrolls.
    pub fn chooser_list_height(&self) -> f32 {
        let room = self.size.height - more::CHOOSER_CHROME;
        let least = more::CHOOSER_MIN_ROWS * metrics::MENU_ITEM_HEIGHT;
        room.clamp(least, metrics::CHOOSER_LIST_HEIGHT)
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
        let paths = self.selected_paths();
        let Some(catalog) = self.catalog.clone().filter(|_| !paths.is_empty()) else {
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
        let launch = match catalog.command(&app, &paths) {
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
        // Inside the window; a submenu opens on the left when the right has no room. A popup
        // is placed by the compositor instead, which flips it at the screen's edges.
        let at = if self.menus_in_popups() {
            at
        } else {
            Point::new(
                at.x.min(self.size.width - metrics::CONTEXT_MENU_WIDTH)
                    .max(0.0),
                at.y.min(self.size.height - height).max(0.0),
            )
        };
        self.menu = Some(Menu {
            at,
            on_selection,
            submenu: None,
            highlight: None,
        });
        self.menu_motion = MenuMotion::opening(self.now());
    }

    /// Opens the menu from the keyboard, under the cursor row.
    fn open_menu_at_cursor(&mut self) {
        let row =
            self.selection.cursor().map_or(0.0, |i| (i + 1) as f32) * metrics::LIST_ROW_HEIGHT;
        let top = metrics::TITLE_BAR_HEIGHT
            + metrics::TOOLBAR_HEIGHT
            + metrics::LIST_HEADER_HEIGHT
            + 3.0 * metrics::BORDER_WIDTH
            + metrics::LIST_TOP_GAP;
        let left = self.panes().sidebar_width() + metrics::LIST_PADDING + metrics::LIST_ROW_HEIGHT;
        self.open_menu(Point::new(left, top + row - self.scroll.0));
    }

    /// A right click while the menu is open: on a row of the listing, or its background, the
    /// menu opens again there, as in any file manager; elsewhere it only closes.
    fn reopen(&mut self) -> Task<Message> {
        self.menu = None;
        let origin = self.list_origin();
        let right = self.size.width - self.panes().preview_width() - metrics::LIST_PADDING;
        let (x, y) = (self.pointer.x, self.pointer.y);
        let inside = x >= origin.x && x < right && y >= origin.y;
        if !inside || y >= origin.y + self.list_height() {
            return Task::none();
        }
        let (_, in_list) = self.in_list(self.pointer);
        let row = ((in_list - metrics::LIST_TOP_GAP) / metrics::LIST_ROW_HEIGHT).floor();
        let index = (row >= 0.0 && (row as usize) < self.entries.len()).then_some(row as usize);
        self.handle(Message::RightClick(index))
    }

    /// Arrows, Enter and Escape in the open menu.
    fn menu_key(&mut self, key: &Key) -> Task<Message> {
        let Some(menu) = &self.menu else {
            return Task::none();
        };
        let items = match menu.submenu {
            Some(sub) => self.submenu_items(sub),
            None => self.menu_items(menu.on_selection),
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
        let mut open_submenu = None;
        match key {
            Key::Named(Named::ArrowDown) => menu.highlight = step(true),
            Key::Named(Named::ArrowUp) => menu.highlight = step(false),
            Key::Named(Named::ArrowRight) => {
                if let Some(MenuItem::Submenu(sub)) = highlighted {
                    open_submenu = Some(sub);
                }
            }
            Key::Named(Named::ArrowLeft) if menu.submenu.is_some() => {
                menu.submenu = None;
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
                Some(MenuItem::Submenu(sub)) => open_submenu = Some(sub),
                _ => {}
            },
            _ => {}
        }
        if open_submenu.is_some() {
            self.open_submenu(open_submenu, Some(0));
        }
        Task::none()
    }

    /// Opens a submenu, or closes it, with an entry highlighted from the keyboard.
    fn open_submenu(&mut self, sub: Option<Sub>, highlight: Option<usize>) {
        if sub == Some(Sub::OpenWith) {
            self.prepare_open_with();
        }
        if let Some(menu) = &mut self.menu {
            menu.submenu = sub;
            menu.highlight = highlight;
        }
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
        // The large preview: Space or Escape close it, the arrows go to the next item and it
        // follows (lot E-12).
        if self.looking {
            match key.as_ref() {
                Key::Named(Named::Space | Named::Escape) => {
                    self.quick_look(false);
                    return Task::none();
                }
                Key::Named(Named::ArrowDown | Named::ArrowRight) => {
                    let to = self.selection.cursor().map_or(0, |c| c + 1);
                    return self.move_cursor(to.min(self.entries.len().saturating_sub(1)), false);
                }
                Key::Named(Named::ArrowUp | Named::ArrowLeft) => {
                    let to = self.selection.cursor().map_or(0, |c| c.saturating_sub(1));
                    return self.move_cursor(to, false);
                }
                _ => return Task::none(),
            }
        }
        if self.path_edit.is_some() {
            return self.path_key(&key, modifiers);
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
            // Ctrl+Tab and Ctrl+Maj+Tab go from tab to tab (D-27b).
            Key::Named(Named::Tab) if ctrl => self.cycle_tab(shift),
            Key::Named(Named::Tab) => focus::advance(direction(modifiers)),
            Key::Named(Named::ArrowLeft) if modifiers.alt() => self.update(Message::Back),
            Key::Named(Named::ArrowRight) if modifiers.alt() => self.update(Message::Forward),
            Key::Named(Named::ArrowUp) if modifiers.alt() => self.update(Message::Up),
            Key::Named(Named::ArrowDown) => {
                let to = self.selection.cursor().map_or(0, |c| c + 1);
                self.move_cursor(to, shift)
            }
            Key::Named(Named::ArrowUp) => {
                let to = self
                    .selection
                    .cursor()
                    .map_or(last, |c| c.saturating_sub(1));
                self.move_cursor(to, shift)
            }
            Key::Named(Named::Home) => self.move_cursor(0, shift),
            Key::Named(Named::End) => self.move_cursor(last, shift),
            Key::Named(Named::Backspace) => self.update(Message::Up),
            Key::Named(Named::Enter) => self.run(Command::Open),
            Key::Named(Named::F2) => self.run(Command::Rename),
            Key::Named(Named::F5) => self.run(Command::Refresh),
            Key::Named(Named::Delete) => self.run(Command::Trash),
            // Space opens the large preview, as Quick Look in the Finder (lot E-12).
            Key::Named(Named::Space) => self.run(Command::QuickLook),
            Key::Named(Named::ContextMenu) => {
                self.open_menu_at_cursor();
                Task::none()
            }
            Key::Named(Named::F10) if shift => {
                self.open_menu_at_cursor();
                Task::none()
            }
            Key::Named(Named::Escape) => {
                self.search_open = false;
                self.folded_menu(false);
                self.search.clear();
                self.notice = None;
                self.refilter();
                self.clear_selection();
                Task::none()
            }
            // As Alt+Cmd+P in the Finder (lot E-08).
            Key::Character(c) if ctrl && modifiers.alt() && c.eq_ignore_ascii_case("p") => {
                self.run(Command::TogglePreview)
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
                ("t", _) => self.update(Message::NewTab),
                ("w", _) => {
                    let active = self.tabs.active;
                    self.close_tab(active)
                }
                ("f", _) => {
                    self.search_open = true;
                    operation::focus(SEARCH)
                }
                ("r", _) => self.run(Command::Refresh),
                ("l", _) => self.run(Command::EditPath),
                ("g", true) => self.run(Command::GoToFolder),
                _ => Task::none(),
            },
            // Typing a path starts the field, as in Nautilus.
            Key::Character(c @ ("/" | "~")) => self.start_path_edit(false, Some(c.to_owned())),
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
                self.dnd_clip(Clip::Copy, &selected);
                self.clipboard = Some((Clip::Copy, selected));
            }
            Command::Cut if !in_trash && !selected.is_empty() => {
                self.dnd_clip(Clip::Cut, &selected);
                self.clipboard = Some((Clip::Cut, selected));
            }
            // The session's clipboard first: it may hold files of another application.
            Command::Paste if self.dnd_paste() => {}
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
            Command::SortBy(column) => {
                self.sort = self.sort.toggle(column);
                self.refilter();
            }
            Command::ViewList => {}
            Command::QuickLook => self.quick_look(!self.looking),
            Command::EditPath => return self.start_path_edit(false, None),
            Command::GoToFolder => return self.start_path_edit(true, None),
            // A preview the width had folded comes back, pinned, rather than a hidden one
            // being hidden (E-05); it slides in (E-08).
            Command::TogglePreview if self.show_preview && !self.panes().preview => {
                let from = self.preview_level();
                self.preview_pinned = true;
                let now = self.now();
                self.preview_slide = Some(Fade::new(from, 1.0, now, more::PREVIEW_SLIDE));
            }
            Command::TogglePreview => return self.toggle_preview(),
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
                self.selection.all(self.entries.len());
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
        let Some(entry) = self.selection.focus().and_then(|i| self.entries.get(i)) else {
            return Task::none();
        };
        if entry.kind == Kind::Folder && self.location != Location::Trash {
            // Several folders selected: each in a tab of its own, the first shown (D-27b).
            let folders = self.selected_folders();
            if folders.len() > 1 {
                return self.open_tabs(folders, true);
            }
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
        let Some(entry) = self.selection.focus().and_then(|i| self.entries.get(i)) else {
            return Task::none();
        };
        let path = entry.path.clone();
        self.open_blind(path)
    }

    /// Opens a file no application was chosen for, with `xdg-open`, unless it is a program or
    /// a launcher: `xdg-open` may run those, depending on the desktop (audit I-14).
    pub(crate) fn open_blind(&mut self, path: PathBuf) -> Task<Message> {
        let mut command = Process::new("xdg-open");
        command.arg(&path);
        if let Some(parent) = path.parent() {
            command.current_dir(parent);
        }
        sherlock_ui::gpu::for_child(&mut command);
        Task::perform(
            async move {
                if preview::is_program(&path) {
                    return None;
                }
                Some(spawn_reaped(&mut command))
            },
            |started| started.map_or(Message::ProgramRefused, Message::Launched),
        )
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
        // The program gets the environment Evidence started with, not its renderer choice.
        sherlock_ui::gpu::for_child(&mut command);
        Task::perform(async move { spawn_reaped(&mut command) }, Message::Launched)
    }

    fn paste(&mut self) -> Task<Message> {
        let (Some(dest), Some((mode, sources))) = (self.current_dir(), self.clipboard.clone())
        else {
            return Task::none();
        };
        // A cut item is pasted only once.
        let moving = mode == Clip::Cut;
        if moving {
            self.clipboard = None;
        }
        // The names already taken there are looked for first, off the interface thread: if
        // any, the user chooses (lot E-11).
        Task::perform(
            async move {
                let names = crate::transfer_job::taken(&sources, &dest);
                (sources, dest, names)
            },
            move |(sources, dest, names)| Message::Taken(sources, dest, moving, names),
        )
    }

    /// Copies or moves with a progress the status bar shows, and that it can stop.
    fn transfer(
        &mut self,
        sources: Vec<PathBuf>,
        dest: PathBuf,
        moving: bool,
        taken: crate::transfer_job::Taken,
    ) -> Task<Message> {
        let progress = Arc::new(crate::transfer_job::Progress::default());
        self.progress = Some(progress.clone());
        let trash = self.config.trash.clone();
        let text = if moving {
            texts::moving(sources.len())
        } else {
            texts::copying(sources.len())
        };
        self.spawn(text, move || {
            let report = if moving {
                crate::transfer_job::move_(&sources, &dest, taken, trash.as_ref(), progress)?
            } else {
                crate::transfer_job::copy(&sources, &dest, taken, trash.as_ref(), progress)?
            };
            done(report)
        })
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
        // With several items selected, F2 renames the one under the cursor (or right-clicked),
        // alone; never an item that is not selected.
        let Some(index) = self.selection.focus() else {
            return Task::none();
        };
        let Some(entry) = self.entries.get(index).cloned() else {
            return Task::none();
        };
        self.select_only(index);
        // The name is selected without its extension, as everywhere else.
        let stem = match (entry.kind, entry.name.rfind('.')) {
            (Kind::File, Some(dot)) if dot > 0 => entry.name[..dot].chars().count(),
            _ => entry.name.chars().count(),
        };
        self.renaming = Some((entry.path.clone(), entry.name));
        Task::batch([
            operation::focus(RENAME),
            operation::select_range(RENAME, 0, stem),
        ])
    }

    fn finish_rename(&mut self) -> Task<Message> {
        // The item is kept by its path: a sort or a search during the rename moves the rows,
        // never the target.
        let Some((path, name)) = self.renaming.take() else {
            return Task::none();
        };
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

/// Starts a program without waiting for it; a thread reaps it when it ends.
fn spawn_reaped(command: &mut Process) -> bool {
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
