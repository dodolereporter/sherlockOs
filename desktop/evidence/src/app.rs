//! État de l'application et réponse à chaque message : navigation, sélection, opérations,
//! clavier. La vue est dans `view`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::widget::{operation, scrollable};
use iced::{Event, Point, Size, Subscription, Task, event, mouse, window};
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::metrics;
use sherlock_ui::texts::evidence as texts;

use crate::git::{self, Repo};
use crate::listing::{self, Column, Entry, Kind, Sort};
use crate::ops;
use crate::places::{self, Place};
use crate::preview::{self, Preview};
use crate::probe::{Probe, Step};
use crate::trash::Trash;
use sherlock_ui::motion::{self, Animation, Fade};
use std::time::{Duration, Instant};

/// Identifiants des widgets que l'application pilote.
pub const LIST: &str = "liste";
pub const SEARCH: &str = "recherche";
pub const RENAME: &str = "renommer";

/// Ce que l'application reçoit au démarrage.
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
}

impl Config {
    /// Dossier personnel, dossier donné en argument, corbeille de l'utilisateur.
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
        Self {
            home,
            start,
            favorites,
            trash: Trash::user(),
            measure,
            maximized,
        }
    }
}

/// Où l'on se trouve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Folder(PathBuf),
    Trash,
}

/// Les commandes du menu et des raccourcis.
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
}

/// Une entrée du menu du clic droit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Action {
        label: &'static str,
        keys: Option<&'static str>,
        command: Command,
        danger: bool,
        checked: bool,
    },
    /// Ouvre le sous-menu « Ouvrir avec ».
    OpenWith,
    /// Prévu, pas encore disponible.
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
        matches!(self, Self::Action { .. } | Self::OpenWith)
    }
}

/// Le menu du clic droit ouvert.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    /// Coin haut gauche, dans la fenêtre.
    pub at: Point,
    /// Ouvert sur la sélection, ou sur le fond de la liste.
    pub on_selection: bool,
    pub submenu: bool,
    /// Entrée surlignée au clavier, dans le menu ou dans le sous-menu.
    pub highlight: Option<usize>,
}

/// Message de la barre d'état.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

/// Fin d'une opération lancée hors du fil de l'interface : texte à montrer et éléments à
/// sélectionner, ou l'erreur.
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
    /// Clic sur le fond de la liste.
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
    /// Une image de la fenêtre, pendant une animation ou une mesure.
    Frame(Instant),
    /// Contenu d'un emplacement, lu hors du fil de l'interface. Le numéro écarte une lecture
    /// dépassée par une navigation plus récente.
    Loaded(u64, Result<Vec<Entry>, ()>),
    /// Aperçu préparé hors du fil de l'interface.
    Preview(PathBuf, Preview),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clip {
    Copy,
    Cut,
}

/// L'application Fichiers.
pub struct App {
    pub(crate) config: Config,
    pub(crate) location: Location,
    history: Vec<Location>,
    future: Vec<Location>,
    /// Tout le dossier, puis ce que montre la liste (recherche appliquée, trié).
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
    /// Élément en cours de renommage et son nouveau nom.
    pub(crate) renaming: Option<(usize, String)>,
    pub(crate) menu: Option<Menu>,
    pub(crate) confirm_empty: bool,
    pub(crate) notice: Option<Notice>,
    pub(crate) busy: usize,
    pointer: Point,
    modifiers: Modifiers,
    pub(crate) size: Size,
    /// Défilement de la liste et hauteur visible, pour garder le curseur en vue.
    pub(crate) scroll: (f32, f32),
    probe: Option<Probe>,
    /// Numéro de la dernière lecture demandée, éléments à sélectionner quand elle arrive, et
    /// renommage à ouvrir alors (nouveau dossier).
    generation: u64,
    pending: Vec<PathBuf>,
    pending_rename: bool,
    /// L'emplacement vient de changer : son contenu arrivera en fondu.
    fresh: bool,
    /// Élément dont l'aperçu est montré ou attendu.
    preview_path: Option<PathBuf>,
    /// Origine du temps des animations.
    origin: Instant,
    /// Position verticale du surlignage de la sélection, qui glisse d'une ligne à l'autre.
    pub(crate) highlight: Option<Animation>,
    /// Fondu du contenu d'un dossier qui s'ouvre, et de l'aperçu.
    pub(crate) content_fade: Option<Fade>,
    pub(crate) preview_fade: Option<Fade>,
    /// Échelle du menu qui s'ouvre.
    pub(crate) menu_scale: Option<Animation>,
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
            menu_scale: None,
        };
        app.probe = app.config.measure.clone().map(Probe::new);
        let task = app.load(&[]);
        (app, task)
    }

    /// Clavier, pointeur et taille de la fenêtre.
    pub fn subscription(&self) -> Subscription<Message> {
        // Les images ne battent que pendant une animation ou une mesure, jamais au repos.
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
        Subscription::batch([events, frames])
    }

    /// Window title: the application name only. The folder is shown in the drawn title bar.
    pub fn title(&self) -> String {
        texts::APP_NAME.to_owned()
    }

    /// Nom du dossier courant, pour la barre de titre et la recherche.
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

    /// Temps des animations.
    pub fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    /// Une animation de l'application court encore.
    pub fn animating(&self, now: Duration) -> bool {
        let spring = |a: &Option<Animation>, precision| {
            a.as_ref().is_some_and(|a| !a.is_done(now, precision))
        };
        let fade = |f: &Option<Fade>| f.as_ref().is_some_and(|f| !f.is_done(now));
        spring(&self.highlight, motion::GEOMETRY_PRECISION)
            || spring(&self.menu_scale, motion::SCALE_PRECISION)
            || fade(&self.content_fade)
            || fade(&self.preview_fade)
    }

    /// Pose toutes les animations à leur fin. Pour les captures.
    pub fn settle(&mut self) {
        let now = self.now();
        self.content_fade = None;
        self.preview_fade = None;
        self.menu_scale = None;
        if let Some(target) = self.highlight.as_ref().map(Animation::target) {
            self.highlight = Some(Animation::new(motion::PREVIEW_SPRING, target, target, now));
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        let now = self.now();
        // Le surlignage suit le curseur quand un seul élément est sélectionné.
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
        // Le menu s'ouvre à l'échelle.
        match (&self.menu, &self.menu_scale) {
            (Some(_), None) => {
                self.menu_scale = Some(Animation::new(
                    motion::OPEN_SPRING,
                    motion::OPEN_SCALE,
                    1.0,
                    now,
                ));
            }
            (None, Some(_)) => self.menu_scale = None,
            _ => {}
        }
        Task::batch([task, self.preview_task()])
    }

    /// Demande l'aperçu de l'élément sélectionné, s'il a changé.
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
                self.menu = None;
                return self.run(command);
            }
            Message::Submenu(open) => {
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
                    Ok((text, select)) => {
                        self.notice = Some(Notice { text, error: false });
                        select
                    }
                    Err(text) => {
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
        // Une animation finie est oubliée : l'abonnement aux images s'arrête.
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

    /// Relit l'emplacement, garde la sélection (ou sélectionne `select`), et demande l'état
    /// git hors du fil de l'interface.
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
        Task::batch([listing, git])
    }

    /// Le contenu de l'emplacement est arrivé.
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

    /// Applique la recherche et le tri, en gardant la sélection.
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

    // Sélection.

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

    /// L'élément sélectionné, s'il est seul.
    pub fn single(&self) -> Option<&Entry> {
        match self.selection.len() {
            1 => self.cursor.and_then(|i| self.entries.get(i)),
            _ => None,
        }
    }

    /// Déplace le curseur au clavier et garde la ligne visible.
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

    // Menu du clic droit.

    /// Entrées du menu : sur la sélection ou sur le fond, dans un dossier ou la corbeille.
    pub fn menu_items(&self, on_selection: bool) -> Vec<MenuItem> {
        use MenuItem::{Action, OpenWith, Separator};
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

    /// Entrées du sous-menu « Ouvrir avec ».
    pub fn submenu_items() -> Vec<MenuItem> {
        vec![
            MenuItem::Action {
                label: texts::DEFAULT_APP,
                keys: None,
                command: Command::Open,
                danger: false,
                checked: true,
            },
            MenuItem::action(texts::TERMINAL_HERE, None, Command::OpenTerminal),
            MenuItem::Separator,
            MenuItem::Disabled(texts::OTHER_APP),
        ]
    }

    /// Hauteur d'un menu, pour qu'il tienne dans la fenêtre.
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
        let on_selection = !self.selection.is_empty();
        let height = Self::menu_height(&self.menu_items(on_selection));
        let width = metrics::CONTEXT_MENU_WIDTH + metrics::SUBMENU_GAP;
        let width = width + metrics::CONTEXT_SUBMENU_WIDTH;
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

    /// Ouvre le menu au clavier, sous la ligne du curseur.
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

    /// Flèches, Entrée et Échap dans le menu ouvert.
    fn menu_key(&mut self, key: &Key) -> Task<Message> {
        let Some(menu) = &self.menu else {
            return Task::none();
        };
        let items = if menu.submenu {
            Self::submenu_items()
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
        match key {
            Key::Named(Named::ArrowDown) => menu.highlight = step(true),
            Key::Named(Named::ArrowUp) => menu.highlight = step(false),
            Key::Named(Named::ArrowRight) if highlighted == Some(MenuItem::OpenWith) => {
                menu.submenu = true;
                menu.highlight = Some(0);
            }
            Key::Named(Named::ArrowLeft) if menu.submenu => {
                menu.submenu = false;
                menu.highlight = None;
            }
            Key::Named(Named::Escape) => self.menu = None,
            Key::Named(Named::Enter | Named::Space) => match highlighted {
                Some(MenuItem::Action { command, .. }) => {
                    self.menu = None;
                    return self.run(command);
                }
                Some(MenuItem::OpenWith) => {
                    menu.submenu = true;
                    menu.highlight = Some(0);
                }
                _ => {}
            },
            _ => {}
        }
        Task::none()
    }

    // Clavier.

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
        if self.menu.is_some() {
            return self.menu_key(&key);
        }
        if self.renaming.is_some() && key == Key::Named(Named::Escape) {
            // Le champ a déjà rendu le clavier au premier Échap : celui-ci annule.
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

    // Commandes.

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
                    self.launch(terminal);
                }
            }
            Command::Rename if !in_trash => return self.start_rename(),
            Command::Duplicate if !in_trash && !selected.is_empty() => {
                let text = texts::copying(selected.len());
                return self.spawn(text, move || {
                    let created = ops::duplicate(&selected)?;
                    Ok((texts::DONE.into(), created))
                });
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
            Command::EmptyTrash if in_trash => self.confirm_empty = !self.all.is_empty(),
            Command::Refresh => return self.load(&[]),
            Command::TogglePreview => self.show_preview = !self.show_preview,
            Command::SelectAll => {
                self.selection = (0..self.entries.len()).collect();
                self.cursor = self.selection.iter().next().copied();
            }
            _ => {}
        }
        Task::none()
    }

    /// Lance une opération hors du fil de l'interface : les grosses copies ne la bloquent pas.
    fn spawn(
        &mut self,
        text: String,
        job: impl FnOnce() -> ops::Result<(String, Vec<PathBuf>)> + Send + 'static,
    ) -> Task<Message> {
        self.busy += 1;
        self.notice = Some(Notice { text, error: false });
        Task::perform(
            async move { job().map_err(|e| e.to_string()) },
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
        let mut open = Process::new("xdg-open");
        open.arg(&entry.path);
        self.launch(open);
        Task::none()
    }

    /// Lance un programme sans l'attendre ; un fil le récupère quand il se termine.
    fn launch(&mut self, mut command: Process) {
        match command.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(error) => {
                tracing::warn!(%error, "lancement impossible");
                self.notice = Some(Notice {
                    text: texts::CANNOT_OPEN.into(),
                    error: true,
                });
            }
        }
    }

    fn paste(&mut self) -> Task<Message> {
        let (Some(dest), Some((mode, sources))) = (self.current_dir(), self.clipboard.clone())
        else {
            return Task::none();
        };
        match mode {
            Clip::Copy => self.spawn(texts::copying(sources.len()), move || {
                let created = ops::copy_into(&sources, &dest)?;
                Ok((texts::DONE.into(), created))
            }),
            Clip::Cut => {
                // Un élément coupé ne se colle qu'une fois.
                self.clipboard = None;
                self.spawn(texts::moving(sources.len()), move || {
                    let moved = ops::move_into(&sources, &dest)?;
                    Ok((texts::DONE.into(), moved))
                })
            }
        }
    }

    fn new_folder(&mut self) -> Task<Message> {
        let Some(dir) = self.current_dir() else {
            return Task::none();
        };
        match ops::new_folder(&dir) {
            Ok(path) => {
                // Le dossier se crée vide, la recherche le cacherait. Le renommage s'ouvre
                // quand la liste relue l'a sélectionné.
                self.search.clear();
                self.pending_rename = true;
                self.load(&[path])
            }
            Err(error) => {
                self.fail(&error);
                Task::none()
            }
        }
    }

    fn start_rename(&mut self) -> Task<Message> {
        let Some((index, entry)) = self.cursor.zip(self.single().cloned()) else {
            return Task::none();
        };
        // Le nom est sélectionné sans son extension, comme partout ailleurs.
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
        match ops::rename(&entry.path, &name) {
            Ok(path) => self.load(&[path]),
            Err(error) => {
                self.fail(&error);
                Task::none()
            }
        }
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
        let mut restored = 0;
        for id in &ids {
            match trash.restore(id) {
                Ok(_) => restored += 1,
                Err(error) => {
                    let task = self.load(&[]);
                    self.fail(&error);
                    return task;
                }
            }
        }
        let task = self.load(&[]);
        self.notice = Some(Notice {
            text: texts::restored(restored),
            error: false,
        });
        task
    }

    fn empty_trash(&mut self) -> Task<Message> {
        let Some(trash) = self.config.trash.clone() else {
            return Task::none();
        };
        self.spawn(texts::EMPTY_TRASH.into(), move || {
            trash.empty()?;
            Ok((texts::TRASH_EMPTIED.into(), Vec::new()))
        })
    }

    fn fail(&mut self, error: &ops::Error) {
        tracing::warn!(%error, "opération refusée");
        self.notice = Some(Notice {
            text: error.to_string(),
            error: true,
        });
    }

    /// Le chemin affiché par le fil d'Ariane : segments et dossier de chacun. Sous le dossier
    /// personnel, le chemin part d'« Accueil ».
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

    /// Le dossier d'origine d'un élément de la corbeille, pour la liste.
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

/// Lit un emplacement : un dossier, ou la corbeille avec le nom et la place d'origine de
/// chaque élément. Hors du fil de l'interface.
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
