//! Launcher state, messages, update and view.

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::futures::channel::oneshot;
use iced::keyboard::{self, key::Named};
use iced::widget::{
    Space, button, column, container, mouse_area, operation, row, stack, svg, text, text_input,
};
use iced::window::{self, Id};
use iced::{Alignment, Element, Event, Length, Padding, Subscription, Task, event};
use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize};
use iced_exwlshell::to_layer_message;
use sherlock_ui::animated::reveal;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::dock::text as dock_texts;
use sherlock_ui::icon::Icon;
use sherlock_ui::launcher::{self as look, Glyph, text as texts};
use sherlock_ui::logo::Logo;
use sherlock_ui::motion::{Animation, Ease};
use sherlock_ui::surface::{Elevation, shadowed};
use sherlock_ui::{color, metrics, typography};

use crate::service::{self, Inbox, Request};
use crate::source::{self, Action, Command, Hit, Section, Source, Tile};
use crate::startup;
use crate::system::{NoSession, Session};

/// Id of the search field, to give it the keyboard.
pub const QUERY: &str = "query";

/// The keys the launcher acts on, whatever has the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Escape,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Surface(Id),
    Query(String),
    Key(Key),
    /// A result clicked, by its place in the list.
    Pick(usize),
    /// A source has more to say (the files, once walked): search again.
    Refresh,
    /// A right click on an application, by its place in the list: its menu.
    Menu(usize),
    /// « Garder dans le Dock » or « Retirer du Dock » in that menu.
    Pin,
    /// A request of a client of the resident service (lot P-02).
    Service(Request),
    /// A click beside the panel: it closes, and the click goes no further.
    Dismiss,
    /// A click on the panel, beside its results: nothing.
    Inside,
    Frame(Instant),
    /// The sources rebuilt for a new opening.
    Reloaded(Reloaded),
    /// The theme of the appearance file changed.
    Theme(color::Theme),
}

/// Sources and what tells when their files are walked, as `crate::sources` gives them.
pub type Sources = (Vec<Box<dyn Source>>, oneshot::Receiver<usize>);

/// What rebuilds the sources at each opening of a resident launcher, off the interface's
/// thread: the applications installed or removed meanwhile, the files walked again.
pub type Reload = Arc<dyn Fn() -> Sources + Send + Sync>;

/// Sources rebuilt, handed once to `update`.
#[derive(Clone)]
pub struct Reloaded(Arc<Mutex<Option<Sources>>>);

impl std::fmt::Debug for Reloaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Reloaded")
    }
}

/// What starts a program: `launch::spawn`, or a recorder in tests.
pub type Spawn = Arc<dyn Fn(&Command) -> Result<u32, crate::launch::LaunchError> + Send + Sync>;

/// How the calculator's result is copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clipboard {
    /// `wl-copy`: it keeps offering the text once the launcher is closed, so the launcher
    /// closes as after any start.
    WlCopy(OsString),
    /// iced's clipboard: a Wayland selection lives as long as its program, so the launcher
    /// stays open, the result marked « Copié », until Escape.
    Toolkit,
}

impl Clipboard {
    /// `wl-copy` when it is installed.
    pub fn detect() -> Self {
        if mime_apps::entry::is_executable("wl-copy") {
            Self::WlCopy("wl-copy".into())
        } else {
            Self::Toolkit
        }
    }
}

/// An item of the list, a section label or a result, where it is drawn from (lot P-02d).
#[derive(Debug, Clone)]
struct Place {
    /// The section and the title, unique in the list.
    key: String,
    /// Its top in the list, moving to its new place.
    top: Animation,
    /// When it appeared: it fades in from then.
    born: Option<Duration>,
}

/// What an item of the list is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The label of the section of this place.
    Label(usize),
    /// The result of this place in the list.
    Hit(usize),
}

pub struct Launcher {
    sources: Vec<Box<dyn Source>>,
    spawn: Spawn,
    session: Arc<dyn Session>,
    clipboard: Clipboard,
    /// The result waiting for its confirmation (« Se déconnecter », « Redémarrer »,
    /// « Éteindre »), and when it was armed.
    armed: Option<(usize, Instant)>,
    /// The result just copied with iced's clipboard.
    pub copied: Option<usize>,
    pub query: String,
    pub sections: Vec<Section>,
    /// The chosen result, in the order of the list, sections one after the other.
    pub selected: usize,
    surface: Option<Id>,
    /// Asked to close: after a start, or on Escape.
    pub closing: bool,
    /// The application whose menu is open, by its place in the list, and whether it is pinned.
    pub menu: Option<(usize, bool)>,
    /// The dock's configuration file, where the menu pins.
    dock_file: Option<std::path::PathBuf>,
    /// The resident service's requests; `None`: the program ends when the launcher closes.
    service: Option<Inbox>,
    reload: Option<Reload>,
    /// The sources were built for the opening to come: no reload needed.
    fresh: bool,
    /// The sources are to be built again once the panel is open.
    reload_due: bool,
    /// How far the panel is shown, 0 to 1: its opacity and scale.
    presence: Animation,
    origin: Instant,
    /// Time of the last event, from `origin`: the moment the panel draws.
    now: Duration,
    /// The surface covers the screen, the panel at its place: a click beside it closes the
    /// launcher. Only on the GPU, whose surfaces have alpha (ADR 0018).
    pub full: bool,
    /// The small surface drawn once at the start of the service, so that the renderer, its
    /// pipelines and the fonts are ready before the first opening; the frames seen since.
    warming: Option<(Id, u8)>,
    frames: startup::Frames,
    /// The surface is hidden but kept (`park`).
    parked: bool,
    /// The panel's height as drawn, following the rows; the rows, their places; the selection
    /// veil, by its top in the list (lot P-02d).
    drawn: Animation,
    places: Vec<Place>,
    highlight: Animation,
}

impl Launcher {
    pub fn new(sources: Vec<Box<dyn Source>>, spawn: Spawn) -> Self {
        Self {
            sources,
            spawn,
            session: Arc::new(NoSession),
            clipboard: Clipboard::Toolkit,
            armed: None,
            copied: None,
            query: String::new(),
            sections: Vec::new(),
            selected: 0,
            surface: None,
            closing: false,
            menu: None,
            dock_file: dock_config::path(),
            service: None,
            reload: None,
            fresh: true,
            reload_due: false,
            presence: Animation::new(look::OPEN_SPRING, 0.0, 1.0, Duration::ZERO),
            origin: Instant::now(),
            now: Duration::ZERO,
            full: false,
            warming: None,
            frames: startup::Frames::default(),
            parked: false,
            drawn: still(look::panel_height(0, 0)),
            places: Vec::new(),
            highlight: still(0.0),
        }
    }

    /// A resident launcher (lot P-02): hidden at first, shown and hidden by the requests of
    /// `inbox`, its sources rebuilt by `reload` at each opening.
    pub fn resident(mut self, inbox: Inbox, reload: Reload) -> Self {
        self.service = Some(inbox);
        self.reload = Some(reload);
        self.closing = true;
        self.presence = Animation::new(look::OPEN_SPRING, 0.0, 0.0, Duration::ZERO);
        self
    }

    /// Whether the launcher is resident.
    pub fn is_resident(&self) -> bool {
        self.service.is_some()
    }

    /// Whether the panel is shown, or opening: not hidden nor closing.
    pub fn shown(&self) -> bool {
        self.surface.is_some() && !self.closing && !self.parked
    }

    /// How far the panel is shown at the last event.
    pub fn presence(&self) -> f64 {
        self.presence.value(self.now)
    }

    /// Whether something still moves: frames are asked for only that long.
    pub fn animating(&self) -> bool {
        self.warming.is_some()
            || (self.surface.is_some()
                && !self.parked
                && (!self.presence.is_done(self.now, look::OPEN_PRECISION)
                    || self.list_moving(self.now)))
    }

    /// The rows move, appear, the panel's height or the selection follow them.
    fn list_moving(&self, now: Duration) -> bool {
        let moving = |a: &Animation| !a.is_done(now, look::LIST_PRECISION);
        moving(&self.drawn)
            || moving(&self.highlight)
            || self.places.iter().any(|p| {
                moving(&p.top) || p.born.is_some_and(|b| now.saturating_sub(b) < look::APPEAR)
            })
    }

    /// Height of the panel as drawn at the last event.
    pub fn drawn_height(&self) -> f32 {
        self.drawn.value(self.now) as f32
    }

    /// The items of the list in order, with their key and their top in the list.
    fn items(&self) -> Vec<(String, f32, Kind)> {
        let mut items = Vec::new();
        let (mut top, mut index) = (0.0, 0);
        for (place, section) in self.sections.iter().enumerate() {
            items.push((section.title.to_owned(), top, Kind::Label(place)));
            top += look::label_height();
            let mut seen: Vec<&str> = Vec::new();
            for hit in &section.hits {
                // Two results of a section may share a title: told apart by their rank.
                let twin = seen.iter().filter(|t| **t == hit.title).count();
                seen.push(&hit.title);
                let key = format!("{}\u{1f}{}\u{1f}{twin}", section.title, hit.title);
                items.push((key, top, Kind::Hit(index)));
                top += look::ROW_HEIGHT;
                index += 1;
            }
        }
        items
    }

    /// The results changed: each one still there slides from where it is drawn to its new
    /// place, a new one fades in, the panel's height and the selection follow.
    fn place(&mut self, now: Duration) {
        let items = self.items();
        let before = std::mem::take(&mut self.places);
        let had_hits = before.iter().any(|p| p.key.contains('\u{1f}'));
        for (key, top, _) in &items {
            let old = before.iter().find(|p| p.key == *key);
            let place = match old {
                Some(old) => {
                    let mut moved = old.clone();
                    moved.top.retarget(now, f64::from(*top));
                    moved
                }
                None => Place {
                    key: key.clone(),
                    top: still(*top),
                    born: Some(now),
                },
            };
            self.places.push(place);
        }
        self.drawn.retarget(now, f64::from(self.height()));
        let selected = self.selected_top(&items);
        match selected {
            Some(top) if had_hits => self.highlight.retarget(now, f64::from(top)),
            Some(top) => self.highlight = still(top),
            None => {}
        }
    }

    /// The top of the selected result in the list.
    fn selected_top(&self, items: &[(String, f32, Kind)]) -> Option<f32> {
        items
            .iter()
            .find(|(_, _, kind)| *kind == Kind::Hit(self.selected))
            .map(|(_, top, _)| *top)
    }

    /// The selection moved: its veil slides to its row.
    fn slide_selection(&mut self) {
        let now = self.clock();
        if let Some(top) = self.selected_top(&self.items()) {
            self.highlight.retarget(now, f64::from(top));
        }
    }

    /// Everything at its place at once: the panel opens on an empty search.
    fn settle_list(&mut self) {
        self.places.clear();
        self.drawn = still(self.height());
        self.highlight = still(0.0);
    }

    /// The time of the clock, never before the last event's.
    fn clock(&mut self) -> Duration {
        self.now = self.now.max(self.origin.elapsed());
        self.now
    }

    /// Opens the small surface that readies the renderer (see `warming`).
    pub fn warm_up(&mut self) -> Task<Message> {
        let (id, task) = Message::layershell_open(crate::warm_surface(crate::NAMESPACE));
        self.warming = Some((id, 0));
        task
    }

    /// Pins to the dock in the file at `path` instead of the user's (the tests).
    pub fn with_dock_file(mut self, path: std::path::PathBuf) -> Self {
        self.dock_file = Some(path);
        self
    }

    /// Runs the system commands with `session`; without one, they are refused.
    pub fn with_session(mut self, session: Arc<dyn Session>) -> Self {
        self.session = session;
        self
    }

    pub fn with_clipboard(mut self, clipboard: Clipboard) -> Self {
        self.clipboard = clipboard;
        self
    }

    /// The result waiting for its confirmation.
    pub fn armed(&self) -> Option<usize> {
        self.armed.map(|(index, _)| index)
    }

    /// Every result, in the order of the list.
    pub fn hits(&self) -> impl Iterator<Item = &Hit> {
        self.sections.iter().flat_map(|s| s.hits.iter())
    }

    /// Height of the panel as it is now: the search row, then the sections, or one line
    /// saying nothing matches.
    pub fn height(&self) -> f32 {
        let rows = self.hits().count();
        if rows == 0 && !self.query.trim().is_empty() {
            return look::panel_height(0, 1);
        }
        look::panel_height(self.sections.len(), rows)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let frames = if self.animating() {
            window::frames().map(Message::Frame)
        } else {
            Subscription::none()
        };
        let service = match &self.service {
            Some(inbox) => {
                Subscription::run_with(inbox.clone(), Inbox::stream).map(Message::Service)
            }
            None => Subscription::none(),
        };
        Subscription::batch([
            event::listen_with(on_event),
            frames,
            service,
            Subscription::run(crate::appearance).map(Message::Theme),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        // What the user did not do leaves the menu as it is.
        match message {
            Message::Frame(at) => return self.frame(at),
            Message::Reloaded(sources) => return self.reloaded(&sources),
            Message::Theme(theme) => {
                color::set_active(theme);
                return Task::none();
            }
            _ => {}
        }
        // Anything but its own entry closes the menu; Escape closes it alone.
        let menu = self.menu.take();
        match message {
            Message::Menu(index) => {
                let id = self.hits().nth(index).and_then(|h| h.app.clone());
                if let (Some(id), Some(path)) = (id, self.dock_file.as_deref()) {
                    self.menu = Some((index, dock::pins::is_pinned(path, &id)));
                }
                return Task::none();
            }
            Message::Pin => {
                if let Some((index, pinned)) = menu {
                    self.pin(index, !pinned);
                }
                return Task::none();
            }
            Message::Key(Key::Escape) if menu.is_some() => return Task::none(),
            _ => {}
        }
        match message {
            Message::Surface(id) if self.warming.is_some_and(|(w, _)| w == id) => {}
            Message::Surface(id) => {
                self.surface = Some(id);
                // The opening starts with the surface's first frame.
                if !self.closing {
                    let now = self.clock();
                    self.presence = Animation::new(look::OPEN_SPRING, 0.0, 1.0, now);
                }
                return operation::focus(QUERY);
            }
            Message::Service(Request::Toggle) if self.shown() => return self.close(),
            Message::Service(Request::Toggle) => return self.show(),
            Message::Service(Request::Hide) if self.shown() => return self.close(),
            Message::Dismiss if !self.closing => return self.close(),
            Message::Query(query) => {
                let before = self.height();
                self.query = query;
                self.selected = 0;
                return self.search(before);
            }
            Message::Refresh => {
                let before = self.height();
                self.selected = self.selected.min(self.hits().count().saturating_sub(1));
                let task = self.search(before);
                self.selected = self.selected.min(self.hits().count().saturating_sub(1));
                self.slide_selection();
                return task;
            }
            Message::Key(Key::Down) => {
                let last = self.hits().count().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
                self.armed = None;
                self.slide_selection();
            }
            Message::Key(Key::Up) => {
                self.selected = self.selected.saturating_sub(1);
                self.armed = None;
                self.slide_selection();
            }
            Message::Key(Key::Enter) => return self.choose(self.selected),
            Message::Key(Key::Escape) => return self.close(),
            Message::Pick(index) => return self.choose(index),
            _ => {}
        }
        Task::none()
    }

    /// Asks the sources again for the query; the panel follows the number of rows, `before`
    /// being its height as drawn.
    fn search(&mut self, before: f32) -> Task<Message> {
        self.sections = source::search(
            &self.sources,
            &self.query,
            look::SECTION_RESULTS,
            look::MAX_RESULTS,
        );
        self.armed = None;
        self.copied = None;
        let now = self.clock();
        self.place(now);
        if self.height() != before {
            return self.resize();
        }
        Task::none()
    }

    /// Pins result `index` to the dock, or unpins it: the dock reads its file again at once.
    fn pin(&self, index: usize, pinned: bool) {
        let id = self.hits().nth(index).and_then(|h| h.app.clone());
        let (Some(id), Some(path)) = (id, self.dock_file.as_deref()) else {
            return;
        };
        match dock::pins::set(path, &id, pinned) {
            Ok(_) => tracing::info!(id, pinned, "dock"),
            Err(error) => tracing::error!(%error, "cannot write the dock's file"),
        }
    }

    /// Fades the panel out; then the surface goes, or the program ends when it is not resident.
    fn close(&mut self) -> Task<Message> {
        if !self.closing {
            self.closing = true;
            let now = self.clock();
            self.presence.retarget(now, 0.0);
        }
        Task::none()
    }

    /// Shows the panel again, its search empty, as a new launcher would be.
    fn show(&mut self) -> Task<Message> {
        let now = self.clock();
        // The frames of this opening only.
        self.frames = startup::Frames::default();
        self.query.clear();
        self.sections.clear();
        self.selected = 0;
        self.armed = None;
        self.copied = None;
        self.menu = None;
        self.closing = false;
        self.settle_list();
        self.presence.retarget(now, 1.0);
        let mut tasks = vec![hide_other(service::APPLICATIONS)];
        // Read again once the panel is open, so that the reading never delays the opening.
        self.reload_due = !std::mem::replace(&mut self.fresh, false);
        match self.surface {
            Some(id) if self.parked => {
                // The opening starts from nothing, now.
                self.presence = Animation::new(look::OPEN_SPRING, 0.0, 1.0, now);
                tasks.push(self.unpark(id));
            }
            // Still fading out: it comes back from where it is.
            Some(_) => tasks.push(operation::focus(QUERY)),
            None => {
                // A surface over the whole screen once the renderer is known to have alpha.
                self.full = sherlock_ui::surface::gpu_drawn() == Some(true);
                let settings = if self.full {
                    crate::screen_surface()
                } else {
                    crate::panel_surface(self.height())
                };
                let (id, task) = Message::layershell_open(settings);
                self.surface = Some(id);
                tasks.push(task);
            }
        }
        Task::batch(tasks)
    }

    /// Takes the sources rebuilt for this opening.
    fn reloaded(&mut self, sources: &Reloaded) -> Task<Message> {
        let Some((sources, walked)) = sources.0.lock().ok().and_then(|mut s| s.take()) else {
            return Task::none();
        };
        self.sources = sources;
        let before = self.height();
        self.selected = self.selected.min(self.hits().count().saturating_sub(1));
        let task = self.search(before);
        self.selected = self.selected.min(self.hits().count().saturating_sub(1));
        self.slide_selection();
        Task::batch([task, Task::perform(walked, |_| Message::Refresh)])
    }

    /// A frame: the time moves on; the surface goes once the panel faded out.
    fn frame(&mut self, at: Instant) -> Task<Message> {
        self.now = self.now.max(at.saturating_duration_since(self.origin));
        self.frames.tick(at);
        if let Some((id, seen)) = self.warming {
            // Drawn once: the renderer is ready, the surface can go.
            if seen >= 1 {
                self.warming = None;
                startup::step("renderer ready", Duration::ZERO);
                return Task::done(Message::RemoveWindow(id));
            }
            self.warming = Some((id, seen + 1));
        }
        let now = self.now;
        for place in &mut self.places {
            if place
                .born
                .is_some_and(|b| now.saturating_sub(b) >= look::APPEAR)
            {
                place.born = None;
            }
        }
        let done = self.presence.is_done(self.now, look::OPEN_PRECISION);
        if done && !self.list_moving(now) {
            self.frames
                .report(if self.closing { "close" } else { "open" });
        }
        if done
            && !self.closing
            && std::mem::take(&mut self.reload_due)
            && let Some(reload) = self.reload.clone()
        {
            return Task::perform(async move { reload() }, |sources| {
                Message::Reloaded(Reloaded(Arc::new(Mutex::new(Some(sources)))))
            });
        }
        if !(self.closing && done) {
            return Task::none();
        }
        if !self.is_resident() {
            return iced::exit();
        }
        match self.surface {
            Some(id) if self.full => self.park(id),
            Some(id) => {
                self.surface = None;
                Task::done(Message::RemoveWindow(id))
            }
            None => Task::none(),
        }
    }

    /// Hides the surface without removing it: one transparent pixel in a corner, under the
    /// windows, without the keyboard. Its renderer keeps what it drew (the glyphs, the shaped
    /// texts, the layout): a new surface took 60 ms more to show its first frame. Only for the
    /// surface over the screen, drawn on the GPU with alpha; the panel's own surface goes.
    fn park(&mut self, id: Id) -> Task<Message> {
        self.parked = true;
        // The field let go: its caret would blink, and draw the surface again, while hidden.
        let unfocus = iced::advanced::widget::operate(
            iced::advanced::widget::operation::focusable::unfocus(),
        );
        Task::batch([
            unfocus,
            Task::done(Message::KeyboardInteractivityChange {
                id,
                keyboard_interactivity: KeyboardInteractivity::None,
            }),
            Task::done(Message::LayerChange {
                id,
                layer: Layer::Background,
            }),
            Task::done(Message::LayoutChange {
                id,
                anchor: Anchor::Top | Anchor::Left,
                size: LayerSize::px(1, 1),
            }),
        ])
    }

    /// Brings the parked surface back over the screen, with the keyboard.
    fn unpark(&mut self, id: Id) -> Task<Message> {
        self.parked = false;
        let screen = crate::screen_surface();
        Task::batch([
            Task::done(Message::LayoutChange {
                id,
                anchor: screen.anchor,
                size: screen.size,
            }),
            Task::done(Message::LayerChange {
                id,
                layer: screen.layer,
            }),
            Task::done(Message::KeyboardInteractivityChange {
                id,
                keyboard_interactivity: screen.keyboard_interactivity,
            }),
            operation::focus(QUERY),
        ])
    }

    /// Runs result `index`; the launcher closes once it is done.
    fn choose(&mut self, index: usize) -> Task<Message> {
        // A second Enter, or Enter then a click, before the exit is handled starts nothing.
        if self.closing {
            return Task::none();
        }
        let Some(action) = self.hits().nth(index).map(|h| h.action.clone()) else {
            return Task::none();
        };
        match action {
            Action::Start(command) => self.start(&command),
            Action::Copy(value) => self.copy(index, value),
            Action::Session(power) if power.needs_confirmation() => match self.armed {
                // Confirmed by a new press, not too soon: a double click or a key bounce
                // arms, never confirms (a held Enter never gets here, see `on_event`).
                Some((armed, at)) if armed == index && at.elapsed() >= look::CONFIRM_DELAY => {
                    self.run_session(power)
                }
                Some((armed, _)) if armed == index => Task::none(),
                _ => {
                    self.armed = Some((index, Instant::now()));
                    Task::none()
                }
            },
            Action::Session(power) => self.run_session(power),
            Action::Disabled => Task::none(),
        }
    }

    fn run_session(&mut self, power: crate::source::Power) -> Task<Message> {
        match self.session.run(power) {
            Ok(()) => {
                tracing::info!(?power, "session command");
                self.close()
            }
            Err(error) => {
                tracing::error!(%error, "cannot run the session command");
                self.armed = None;
                Task::none()
            }
        }
    }

    fn copy(&mut self, index: usize, value: String) -> Task<Message> {
        match &self.clipboard {
            Clipboard::WlCopy(program) => {
                // `--`: the value is never read as an option.
                let command = Command {
                    program: program.clone(),
                    args: vec!["--".into(), value.into()],
                    dir: None,
                };
                self.start(&command)
            }
            Clipboard::Toolkit => {
                self.copied = Some(index);
                iced::clipboard::write(value)
            }
        }
    }

    fn start(&mut self, command: &Command) -> Task<Message> {
        match (self.spawn)(command) {
            Ok(pid) => {
                tracing::info!(program = ?command.program, pid, "started");
                self.close()
            }
            Err(error) => {
                tracing::error!(%error, "cannot start the result");
                Task::none()
            }
        }
    }

    /// Tells the compositor the new height of the surface, when it is the panel alone.
    fn resize(&self) -> Task<Message> {
        let Some(id) = self.surface.filter(|_| !self.full) else {
            return Task::none();
        };
        let settings = crate::surface(self.height());
        Task::done(Message::LayoutChange {
            id,
            anchor: settings.anchor,
            size: settings.size,
        })
    }

    pub fn view(&self, id: Id) -> Element<'_, Message> {
        if self.warming.is_some_and(|(w, _)| w == id) {
            return view_warm_up();
        }
        let started = Instant::now();
        let view = if self.full {
            self.view_screen()
        } else {
            self.view_moving()
        };
        startup::first(&FIRST_VIEW, "first view");
        tracing::trace!(took_ms = startup::millis(started.elapsed()), "view");
        view
    }

    /// The panel at its opacity and scale of the moment.
    fn view_moving(&self) -> Element<'_, Message> {
        let shown = self.presence() as f32;
        let scale = look::OPEN_SCALE + (1.0 - look::OPEN_SCALE) * shown;
        reveal(self.view_panel())
            .opacity(shown.clamp(0.0, 1.0))
            .scale(scale, (0.5, 0.0))
            .into()
    }

    /// The whole screen: the panel at its place, and around it what closes the launcher on a
    /// click, which goes no further, as Spotlight does.
    pub fn view_screen(&self) -> Element<'_, Message> {
        let panel = mouse_area(self.view_moving())
            .on_press(Message::Inside)
            .on_right_press(Message::Inside);
        let placed = container(panel)
            .padding(Padding {
                top: look::TOP,
                ..Padding::ZERO
            })
            .center_x(Length::Fill);
        let beside = mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
            .on_press(Message::Dismiss)
            .on_right_press(Message::Dismiss)
            .on_middle_press(Message::Dismiss);
        stack![beside, placed]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn view_panel(&self) -> Element<'_, Message> {
        let p = color::palette();
        let field = text_input(texts::PLACEHOLDER, &self.query)
            .id(QUERY)
            .on_input(Message::Query)
            .size(look::QUERY_SIZE)
            .padding(0)
            .style(look::query);
        let search = row![
            ui::icon(Icon::Search, look::SEARCH_ICON, p.accent_text),
            field,
            key_cap(texts::ESCAPE, false),
        ]
        .spacing(look::GAP)
        .align_y(Alignment::Center)
        .padding([0.0, look::SEARCH_PADDING])
        .height(look::SEARCH_HEIGHT);
        let divider = container(Space::new())
            .width(Length::Fill)
            .height(metrics::BORDER_WIDTH)
            .style(look::divider);
        let mut list = column![search, divider];
        let items = self.items();
        for (key, top, kind) in &items {
            let item = match kind {
                Kind::Label(place) => {
                    let Some(section) = self.sections.get(*place) else {
                        continue;
                    };
                    let (top, side, bottom) = look::SECTION_PADDING;
                    container(ui::section_label(section.title))
                        .padding(iced::Padding {
                            top,
                            right: side,
                            bottom,
                            left: side,
                        })
                        .into()
                }
                Kind::Hit(index) => match self.hits().nth(*index) {
                    Some(hit) => self.view_hit(hit, *index),
                    None => continue,
                },
            };
            list = list.push(self.moved(key, *top, item));
        }
        if items.is_empty() && !self.query.trim().is_empty() {
            let nothing = ui::body(texts::NOTHING).color(p.text_secondary);
            list = list.push(
                container(nothing)
                    .height(look::ROW_HEIGHT)
                    .padding([0.0, look::ROW_PADDING])
                    .center_y(look::ROW_HEIGHT),
            );
        }
        // The selection, a veil sliding under the rows.
        let content: Element<'_, Message> = if self.hits().next().is_some() {
            let above = look::SEARCH_HEIGHT + metrics::BORDER_WIDTH;
            let veil = column![
                Space::new().height(above + self.highlight.value(self.now) as f32),
                container(Space::new())
                    .width(Length::Fill)
                    .height(look::ROW_HEIGHT)
                    .style(look::selection),
            ]
            // The first layer gives the stack its size: the whole panel, not the veil's rows.
            .height(Length::Fill);
            stack![veil, list].into()
        } else {
            list.into()
        };
        container(content)
            .width(look::WIDTH)
            .height(self.drawn_height())
            .padding(look::PADDING + metrics::BORDER_WIDTH)
            .clip(true)
            .style(look::panel)
            .into()
    }

    /// An item of the list where it is drawn: on its way to its place `top`, fading in when new.
    fn moved<'a>(&self, key: &str, top: f32, item: Element<'a, Message>) -> Element<'a, Message> {
        let (mut shift, mut opacity) = (0.0, 1.0);
        if let Some(place) = self.places.iter().find(|p| p.key == key) {
            shift = place.top.value(self.now) as f32 - top;
            if let Some(born) = place.born {
                let t = self.now.saturating_sub(born).as_secs_f32() / look::APPEAR.as_secs_f32();
                let shown = Ease::OutCubic.apply(t.clamp(0.0, 1.0));
                opacity = shown;
                shift -= look::APPEAR_OFFSET * (1.0 - shown);
            }
        }
        reveal(item).offset(0.0, shift).opacity(opacity).into()
    }

    /// The line under a result: its own, or what its first Enter or copy did.
    fn subtitle<'a>(&self, hit: &'a Hit, index: usize) -> Option<&'a str> {
        if self.armed() == Some(index) {
            Some(texts::CONFIRM)
        } else if self.copied == Some(index) {
            Some(texts::COPIED)
        } else {
            hit.subtitle.as_deref()
        }
    }

    fn view_hit<'a>(&self, hit: &'a Hit, index: usize) -> Element<'a, Message> {
        let selected = index == self.selected;
        // « Demander à Sherlock » reads as any other result: Sherlock's tile tells it apart, the
        // cream row of the board stood out too much on the glass (UI-03).
        let sherlock = hit.tile == Tile::Sherlock;
        let p = color::palette();
        let (title_font, title_color, line_color) = if sherlock {
            (typography::UI_SEMIBOLD, p.text_primary, p.text_secondary)
        } else {
            (typography::UI_MEDIUM, p.text_primary, p.text_secondary)
        };
        let mut words = column![
            ui::body(hit.title.as_str())
                .font(title_font)
                .color(title_color)
        ];
        if let Some(subtitle) = self.subtitle(hit, index) {
            words = words.push(
                text(subtitle)
                    .size(typography::SMALL_SIZE)
                    .color(line_color),
            );
        }
        let mut line = row![
            tile(&hit.tile, &hit.title),
            words,
            Space::new().width(Length::Fill)
        ]
        .spacing(look::GAP)
        // The whole height of the row, so that a result without a second line has its tile and
        // its title in the middle of the row, not at its top (Q-21).
        .height(Length::Fill)
        .align_y(Alignment::Center);
        if let Some((_, pinned)) = self.menu.filter(|(at, _)| *at == index) {
            // The menu, inside the row on its right: it never leaves the panel.
            let label = if pinned {
                dock_texts::UNPIN
            } else {
                dock_texts::KEEP
            };
            let item = ui::menu_item(label, Trailing::None, Some(Message::Pin));
            line = line.push(ui::floating_menu([item.into()], look::MENU_WIDTH));
        } else if selected && hit.action != Action::Disabled {
            line = line.push(key_cap(texts::ENTER, true));
        }
        let row = button(line)
            .on_press(Message::Pick(index))
            .width(Length::Fill)
            .height(look::ROW_HEIGHT)
            .padding([0.0, look::ROW_PADDING]);
        // The selection is the veil sliding under the rows (`view_panel`).
        let row: Element<'a, Message> = row.style(look::row(false)).into();
        if hit.app.is_some() {
            mouse_area(row).on_right_press(Message::Menu(index)).into()
        } else {
            row
        }
    }
}

/// Set once the first view is built, for the log of the start.
static FIRST_VIEW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the surface that readies the renderer draws: text in the interface fonts and an icon,
/// so that the fonts and the SVG renderer are loaded too, and whether the GPU draws
/// (`Elevation::Flat` learns it).
pub fn view_warm_up<'a, M: 'a>() -> Element<'a, M> {
    let p = color::palette();
    let content = row![
        ui::icon(Icon::Search, look::SEARCH_ICON, p.accent_text),
        text(texts::PLACEHOLDER).size(look::QUERY_SIZE),
        ui::body(texts::APPLICATIONS).font(typography::UI_MEDIUM),
        ui::section_label(texts::FILES),
    ];
    shadowed(content, 0.0, Elevation::Flat).into()
}

/// Asks the resident service `name` to hide, if it runs: another overlay of the shell opens.
pub fn hide_other<M: Send + 'static>(name: &'static str) -> Task<M> {
    let Some(dir) = crate::instance::runtime_dir() else {
        return Task::none();
    };
    Task::future(async move {
        // Nobody listening is the usual case: nothing to hide.
        let _ = service::send(&service::socket(&dir, name), Request::Hide);
    })
    .discard()
}

/// The tile left of a result: an application's icon or initial, a glyph, or Sherlock.
fn tile<'a>(tile: &Tile, title: &str) -> Element<'a, Message> {
    match tile {
        // In a box of the tile's size, as every other tile: icons centred, titles aligned (UI-03).
        // Smooth edges on the GPU (UI-05).
        Tile::App(Some(path)) => container(sherlock_ui::raster::icon(
            &icon_handle(path),
            look::APP_ICON,
        ))
        .center(look::TILE)
        .into(),
        Tile::App(None) => {
            let initial = title
                .chars()
                .next()
                .and_then(|c| c.to_uppercase().next())
                .unwrap_or('?');
            container(
                text(initial.to_string())
                    .font(typography::UI_SEMIBOLD)
                    .size(look::TILE_LETTER_SIZE),
            )
            .center(look::TILE)
            .style(look::tile)
            .into()
        }
        Tile::Glyph(glyph) => {
            let stroke = look::Colors::of(color::palette()).glyph_tile(*glyph).1;
            container(
                svg(glyph_handle(*glyph))
                    .width(look::TILE_GLYPH)
                    .height(look::TILE_GLYPH)
                    .style(move |_, _| svg::Style {
                        color: Some(stroke.into()),
                    }),
            )
            .center(look::TILE)
            .style(look::glyph_tile(*glyph))
            .into()
        }
        Tile::Sherlock => container(ui::logo(Logo::Color, look::TILE))
            .center(look::TILE)
            .into(),
    }
}

/// The SVG of a glyph, built once.
fn glyph_handle(glyph: Glyph) -> svg::Handle {
    static HANDLES: std::sync::OnceLock<Vec<svg::Handle>> = std::sync::OnceLock::new();
    let all = HANDLES.get_or_init(|| {
        Glyph::ALL
            .iter()
            .map(|g| svg::Handle::from_memory(g.svg().into_bytes()))
            .collect()
    });
    Glyph::ALL
        .iter()
        .position(|g| *g == glyph)
        .and_then(|i| all.get(i))
        .cloned()
        .unwrap_or_else(|| svg::Handle::from_memory(glyph.svg().into_bytes()))
}

/// An SVG icon as it is; a PNG inside a small SVG, which the SVG renderer reads by path, as
/// the dock does: iced's `image` feature would bring its own decoders.
pub fn icon_handle(path: &Path) -> svg::Handle {
    if path.extension().is_some_and(|e| e == "svg") {
        return svg::Handle::from_path(path);
    }
    svg::Handle::from_memory(picture_document(path).into_bytes())
}

/// A small SVG document that shows the picture in `path` (a PNG), for the SVG renderer.
pub fn picture_document(path: &Path) -> String {
    let href = path
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         viewBox=\"0 0 1 1\"><image width=\"1\" height=\"1\" xlink:href=\"{href}\"/></svg>"
    )
}

/// A key name in its cap, on the search row or on the selected result (`in_row`).
fn key_cap<'a>(name: &'a str, in_row: bool) -> Element<'a, Message> {
    let (vertical, side) = if in_row {
        look::KEY_PADDING_ROW
    } else {
        look::KEY_PADDING
    };
    container(text(name).size(look::KEY_SIZE))
        .padding([vertical, side])
        .style(look::key(in_row))
        .into()
}

/// The arrows, Enter and Escape, even when the search field took them. A held Enter repeats
/// nothing: every Enter is a new press, so a key held down never confirms a session command.
pub fn on_event(event: Event, _: event::Status, _: Id) -> Option<Message> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, repeat, .. }) = event else {
        return None;
    };
    let key = match key {
        keyboard::Key::Named(Named::ArrowUp) => Key::Up,
        keyboard::Key::Named(Named::ArrowDown) => Key::Down,
        keyboard::Key::Named(Named::Enter) if !repeat => Key::Enter,
        keyboard::Key::Named(Named::Escape) => Key::Escape,
        _ => return None,
    };
    Some(Message::Key(key))
}

/// An animation at rest on `value`.
fn still(value: f32) -> Animation {
    Animation::new(
        look::LIST_SPRING,
        f64::from(value),
        f64::from(value),
        Duration::ZERO,
    )
}
