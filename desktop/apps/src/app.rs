//! State of the applications view, its messages, update and view.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::keyboard::{self, key::Named};
use iced::mouse::{self, ScrollDelta};
use iced::widget::{
    Space, button, column, container, image, mouse_area, operation, pin, row, stack, text,
};
use iced::window::{self, Id};
use iced::{
    Alignment, ContentFit, Element, Event, Length, Padding, Point, Size, Subscription, Task, event,
};
use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize};
use iced_exwlshell::shell::{ShellEvent, ShellReceiver};
use iced_exwlshell::to_layer_message;
use launcher::app::{Spawn, hide_other, icon_handle, view_warm_up};
use launcher::service::{self, Inbox, Request};
use launcher::source::Command;
use launcher::startup;
use sherlock_ui::animated::reveal;
use sherlock_ui::apps::{self as look, text as texts};
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::dock::text as dock_texts;
use sherlock_ui::icon::Icon;
use sherlock_ui::motion::Animation;
use sherlock_ui::{color, metrics, typography};

use crate::backdrop::{self, Outcome, Picture};
use crate::catalog::{Catalog, Tile};
use crate::icons::{self, Rasters};
use crate::pager::{Pager, page_count};

/// Id of the search field, to give it the keyboard.
pub const QUERY: &str = "query";

/// The keys the view acts on, whatever has the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Enter,
    Escape,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Surface(Id),
    Resized(Size),
    Query(String),
    Key(Key),
    /// A tile clicked, by its place among those shown.
    Launch(usize),
    /// A page dot clicked.
    Page(usize),
    /// A click beside the tiles and the field: the view closes, as Launchpad does.
    Dismiss,
    /// The pointer moved: where a menu opens.
    Pointer(Point),
    /// A right click on a tile, by its place among those shown: its menu.
    Menu(usize),
    MenuPick(Entry),
    /// A click beside the menu: it closes.
    CloseMenu,
    Scroll(ScrollDelta),
    Frame(Instant),
    /// A request of a client of the resident service (lot P-02).
    Service(Request),
    /// The scale of the surface: icons are drawn for it.
    Scale(f32),
    /// Icons drawn on the pool.
    Rasters(Arc<Rasters>),
    /// The applications read again for a new opening.
    Reloaded(Arc<Catalog>),
    /// The theme of the appearance file changed.
    Theme(color::Theme),
    /// The screen a surface of the view landed on: its logical size and scale, so that the
    /// icons are drawn before the first opening.
    Output(Option<(Size, f32)>),
    /// The blurred wallpaper, loaded on the pool (lot P-02c).
    Backdrop(Arc<Outcome>),
}

/// What reads the applications again at each opening of a resident view, off the interface's
/// thread.
pub type Reload = Arc<dyn Fn() -> Catalog + Send + Sync>;

/// What loads the blurred wallpaper for a screen in pixels and a theme, given the key of the
/// one shown, off the interface's thread (`backdrop::load`).
pub type Backdrop = Arc<dyn Fn((u32, u32), color::Theme, Option<u64>) -> Outcome + Send + Sync>;

/// The blurred wallpaper of this session, kept in its cache.
pub fn session_backdrop() -> Backdrop {
    Arc::new(|screen, theme, current| {
        let var = |key: &str| std::env::var_os(key);
        let cache = backdrop::cache_dir(var);
        backdrop::load(var, theme, screen, cache.as_deref(), current)
    })
}

/// Desktop file id of Evidence, which « Afficher dans Evidence » starts.
const EVIDENCE: &str = "evidence";

/// An entry of a tile's menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// « Garder dans le Dock ».
    Keep,
    /// « Retirer du Dock ».
    Unpin,
    /// « Afficher dans Evidence »: the folder of its desktop file.
    Show,
}

impl Entry {
    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => dock_texts::KEEP,
            Self::Unpin => dock_texts::UNPIN,
            Self::Show => texts::SHOW,
        }
    }
}

/// The menu of a tile.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    /// The tile, by its place among those shown.
    pub place: usize,
    /// Where it opened.
    pub at: Point,
    /// Its entries, each with whether it acts.
    pub entries: Vec<(Entry, bool)>,
}

pub struct View {
    catalog: Arc<Catalog>,
    spawn: Spawn,
    pub query: String,
    /// The tiles shown, by their place in the catalog: all of them, or those the query finds.
    pub shown: Vec<usize>,
    /// The tile chosen by the arrows or the search, among those shown.
    pub selected: Option<usize>,
    /// Size of the surface: the screen.
    pub size: Size,
    pub pager: Pager,
    /// How far the view is shown, 0 to 1: the icons' scale and opacity, the veil.
    presence: Animation,
    /// The tile started, which grows as the view fades out.
    pub launched: Option<usize>,
    origin: Instant,
    /// Time of the last event, from `origin`: the moment the view draws.
    now: Duration,
    surface: Option<Id>,
    /// The dock's configuration file, where pinning writes.
    dock_file: Option<PathBuf>,
    pointer: Point,
    /// The menu of a tile, while it is open.
    pub menu: Option<Menu>,
    /// Asked to close: it fades out, then the surface goes, or the program ends.
    pub closing: bool,
    /// The resident service's requests; `None`: the program ends when the view closes.
    service: Option<Inbox>,
    reload: Option<Reload>,
    /// The catalog was read for the opening to come.
    fresh: bool,
    /// The catalog is to be read again once the view is open.
    reload_due: bool,
    /// Scale of the surface, from the compositor.
    scale: f32,
    /// The icons drawn at their size on the screen, and whether more are being drawn.
    rasters: Rasters,
    drawing: bool,
    /// Whether the icons are drawn once as pixels; off, iced's SVG widget draws them at every
    /// size, as before lot P-02 (the `pages` example compares both).
    pub rasterize: bool,
    /// The small surface that readies the renderer as the service starts, and its frames.
    warming: Option<(Id, u8)>,
    /// The surface is hidden but kept (`park`).
    parked: bool,
    /// The screens the surfaces land on, from the runtime.
    outputs: Option<ShellReceiver>,
    frames: startup::Frames,
    /// The longest time spent building the view since the last frame.
    view_time: std::cell::Cell<Duration>,
    /// What the frames being counted show.
    moving: &'static str,
    /// The blurred wallpaper shown behind, and its key; `None`: the plain veil.
    backdrop: Option<(u64, image::Handle)>,
    /// What loads it, whether it is loading, and whether to load it again after.
    backdrop_loader: Option<Backdrop>,
    backdrop_loading: bool,
    backdrop_again: bool,
    /// The surface is new: its wallpaper is to be blurred once its size is known.
    backdrop_due: bool,
}

impl View {
    /// The view of `catalog`, opening; `spawn` starts the applications.
    pub fn new(catalog: Catalog, spawn: Spawn) -> Self {
        let (width, height) = metrics::REFERENCE_SCREEN;
        let size = Size::new(width, height);
        let shown: Vec<usize> = Vec::new();
        let pager = Pager::new(page_count(shown.len(), per_page(size)));
        Self {
            catalog: Arc::new(Catalog::default()),
            spawn,
            query: String::new(),
            shown,
            selected: None,
            size,
            pager,
            presence: Animation::new(look::OPEN_SPRING, 0.0, 1.0, Duration::ZERO),
            launched: None,
            origin: Instant::now(),
            now: Duration::ZERO,
            surface: None,
            dock_file: dock_config::path(),
            pointer: Point::ORIGIN,
            menu: None,
            closing: false,
            service: None,
            reload: None,
            fresh: true,
            reload_due: false,
            scale: 1.0,
            rasters: Rasters::default(),
            drawing: false,
            rasterize: true,
            warming: None,
            parked: false,
            outputs: None,
            frames: startup::Frames::default(),
            view_time: std::cell::Cell::new(Duration::ZERO),
            moving: "open",
            backdrop: None,
            backdrop_loader: None,
            backdrop_loading: false,
            backdrop_again: false,
            backdrop_due: false,
        }
        .with_catalog(catalog)
    }

    fn with_catalog(mut self, catalog: Catalog) -> Self {
        self.catalog = Arc::new(catalog);
        self.paginate();
        self
    }

    /// A resident view (lot P-02): hidden at first, shown and hidden by the requests of
    /// `inbox`, its applications read again by `reload` at each opening.
    pub fn resident(mut self, inbox: Inbox, reload: Reload) -> Self {
        self.service = Some(inbox);
        self.reload = Some(reload);
        self.closing = true;
        self.presence = Animation::new(look::OPEN_SPRING, 0.0, 0.0, Duration::ZERO);
        self
    }

    pub fn is_resident(&self) -> bool {
        self.service.is_some()
    }

    /// Whether the view is shown, or opening: not hidden nor closing.
    pub fn shown(&self) -> bool {
        self.surface.is_some() && !self.closing && !self.parked
    }

    /// Whether the view is hidden: no surface, or its surface parked.
    fn hidden(&self) -> bool {
        self.surface.is_none() || self.parked
    }

    /// Follows the screens the view's surfaces land on (`ExWlSettings::shell_broadcast`).
    pub fn with_outputs(mut self, outputs: ShellReceiver) -> Self {
        self.outputs = Some(outputs);
        self
    }

    /// The icons drawn at their size on the screen.
    pub fn rasters(&self) -> &Rasters {
        &self.rasters
    }

    /// Opens the small surface that readies the renderer (`launcher::warm_surface`).
    pub fn warm_up(&mut self) -> Task<Message> {
        let (id, task) = Message::layershell_open(launcher::warm_surface(crate::NAMESPACE));
        self.warming = Some((id, 0));
        task
    }

    /// Loads the blurred wallpaper with `loader` (`session_backdrop`); without one, the plain
    /// veil.
    pub fn with_backdrop_loader(mut self, loader: Backdrop) -> Self {
        self.backdrop_loader = Some(loader);
        self
    }

    /// Shows `picture` behind, as if loaded (the captures).
    pub fn with_backdrop(mut self, picture: Picture) -> Self {
        self.set_backdrop(0, picture);
        self
    }

    /// Whether a blurred wallpaper is shown behind.
    pub fn has_backdrop(&self) -> bool {
        self.backdrop.is_some()
    }

    fn set_backdrop(&mut self, key: u64, picture: Picture) {
        let handle = image::Handle::from_rgba(picture.width, picture.height, picture.rgba);
        self.backdrop = Some((key, handle));
    }

    /// Loads the blurred wallpaper on the pool, if it may have changed: another screen, another
    /// theme, or an opening (the wallpaper may have changed meanwhile).
    fn load_backdrop(&mut self) -> Task<Message> {
        let Some(loader) = self.backdrop_loader.clone() else {
            return Task::none();
        };
        if self.backdrop_loading {
            self.backdrop_again = true;
            return Task::none();
        }
        self.backdrop_loading = true;
        let screen = (
            (self.size.width * self.scale).round() as u32,
            (self.size.height * self.scale).round() as u32,
        );
        let (theme, current) = (color::active(), self.backdrop.as_ref().map(|(k, _)| *k));
        Task::perform(
            async move { Arc::new(loader(screen, theme, current)) },
            Message::Backdrop,
        )
    }

    /// The view already open, for the captures: no opening to wait for.
    /// Pins to the dock in the file at `path` instead of the user's (the tests).
    pub fn with_dock_file(mut self, path: PathBuf) -> Self {
        self.dock_file = Some(path);
        self
    }

    /// The dock's file the menu writes.
    pub fn dock_file(&self) -> Option<&std::path::Path> {
        self.dock_file.as_deref()
    }

    pub fn opened(mut self) -> Self {
        self.presence = Animation::new(look::OPEN_SPRING, 1.0, 1.0, Duration::ZERO);
        self
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// The tiles shown, in their order.
    pub fn tiles(&self) -> impl Iterator<Item = &Tile> {
        self.shown
            .iter()
            .filter_map(|index| self.catalog.tiles().get(*index))
    }

    /// The view laid out for the surface's size.
    pub fn layout(&self) -> look::Layout {
        look::Layout::fit(self.size.width, self.size.height)
    }

    /// How far the view is shown at the last event.
    pub fn presence(&self) -> f64 {
        self.presence.value(self.now)
    }

    /// Whether something still moves: frames are asked for only that long.
    pub fn animating(&self) -> bool {
        let hidden = self.is_resident() && self.hidden();
        self.warming.is_some()
            || (!hidden
                && (!self.presence.is_done(self.now, look::OPEN_PRECISION)
                    || self.pager.moving(self.now)))
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
        // Followed while hidden only: once shown, the surface gives its own size.
        let outputs = match &self.outputs {
            Some(outputs) if self.hidden() => outputs.listen().map(output),
            _ => Subscription::none(),
        };
        Subscription::batch([
            event::listen_with(on_event),
            frames,
            service,
            outputs,
            Subscription::run(launcher::appearance).map(Message::Theme),
        ])
    }

    /// The time of the clock, never before the last event's.
    fn clock(&mut self) -> Duration {
        self.now = self.now.max(self.origin.elapsed());
        self.now
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) if self.warming.is_some_and(|(w, _)| w == id) => {}
            Message::Surface(id) => {
                self.surface = Some(id);
                if !self.closing {
                    let now = self.clock();
                    self.presence = Animation::new(look::OPEN_SPRING, 0.0, 1.0, now);
                }
                // iced_exwlshell reports no size for a new surface: asked for, with its scale.
                let scale = window::scale_factor(id).map(Message::Scale);
                let size = window::size(id).map(Message::Resized);
                // The wallpaper is blurred once the size is known.
                self.backdrop_due = true;
                return Task::batch([operation::focus(QUERY), size, scale]);
            }
            Message::Service(Request::Toggle) if self.shown() => return self.close(),
            Message::Service(Request::Toggle) => return self.show(),
            Message::Service(Request::Hide) if self.shown() => return self.close(),
            Message::Service(Request::Show) if !self.shown() => return self.show(),
            Message::Service(_) => {}
            Message::Scale(scale) if scale > 0.0 => {
                let changed = scale != self.scale;
                self.scale = scale;
                let backdrop = if changed {
                    self.load_backdrop()
                } else {
                    Task::none()
                };
                return Task::batch([self.draw_icons(), backdrop]);
            }
            Message::Backdrop(outcome) => {
                self.backdrop_loading = false;
                match Arc::unwrap_or_clone(outcome) {
                    Outcome::New(key, picture) => self.set_backdrop(key, picture),
                    Outcome::Nothing => self.backdrop = None,
                    Outcome::Unchanged => {}
                }
                if std::mem::take(&mut self.backdrop_again) {
                    return self.load_backdrop();
                }
            }
            Message::Rasters(rasters) => {
                self.drawing = false;
                self.rasters
                    .merge(Arc::try_unwrap(rasters).unwrap_or_else(|r| (*r).clone()));
                return self.draw_icons();
            }
            Message::Reloaded(catalog) => {
                self.catalog = catalog;
                self.paginate();
                return self.draw_icons();
            }
            Message::Theme(theme) => {
                let changed = theme != color::active();
                color::set_active(theme);
                // The wallpaper of the other theme, blurred.
                if changed {
                    return self.load_backdrop();
                }
            }
            // Hidden: the screen it will open on, its icons drawn meanwhile.
            Message::Output(Some((size, scale))) if self.hidden() => {
                tracing::debug!(?size, scale, "screen");
                let changed = size != self.size || scale != self.scale;
                if size != self.size {
                    self.size = size;
                    self.paginate();
                }
                self.scale = scale;
                let backdrop = if changed {
                    self.load_backdrop()
                } else {
                    Task::none()
                };
                return Task::batch([self.draw_icons(), backdrop]);
            }
            // The surface that readies the renderer is one pixel: not the screen.
            Message::Resized(size) if size.width > 1.0 && size.height > 1.0 => {
                let due = std::mem::take(&mut self.backdrop_due);
                if size == self.size && !due {
                    return Task::none();
                }
                tracing::debug!(?size, "surface size");
                let icons = if size == self.size {
                    Task::none()
                } else {
                    self.size = size;
                    self.paginate();
                    self.draw_icons()
                };
                return Task::batch([icons, self.load_backdrop()]);
            }
            Message::Query(query) => {
                self.menu = None;
                self.query = query;
                self.paginate();
                let typed = !self.query.trim().is_empty() && !self.shown.is_empty();
                self.selected = typed.then_some(0);
            }
            Message::Key(key) => return self.key(key),
            // With a menu open, a click elsewhere only closes it.
            Message::Launch(_) | Message::Dismiss | Message::CloseMenu if self.menu.is_some() => {
                self.menu = None;
            }
            Message::Launch(place) => return self.launch(place),
            Message::Pointer(point) => self.pointer = point,
            // Fading out, the view takes no more input: no menu opens nor runs (lot UI-06).
            Message::Menu(_) | Message::MenuPick(_) if self.closing => {}
            Message::Menu(place) => self.open_menu(place),
            Message::MenuPick(entry) => return self.pick(entry),
            Message::Page(page) => {
                let now = self.clock();
                self.pager.go(page, now);
                self.moving = "page";
            }
            Message::Dismiss => return self.close(),
            Message::Scroll(delta) => {
                self.moving = "page";
                let now = self.clock();
                match delta {
                    ScrollDelta::Lines { x, y } => {
                        let along = if y.abs() >= x.abs() { y } else { x };
                        if along != 0.0 {
                            // Down (or left) shows the next page.
                            self.pager.notch(along < 0.0, now);
                        }
                    }
                    ScrollDelta::Pixels { x, y } => {
                        let along = if x.abs() >= y.abs() { x } else { y };
                        self.pager.swipe(along, self.size.width, now);
                    }
                }
            }
            Message::Frame(at) => return self.frame(at),
            _ => {}
        }
        Task::none()
    }

    /// A frame: the time moves on; the surface goes once the view faded out.
    fn frame(&mut self, at: Instant) -> Task<Message> {
        self.now = self.now.max(at.saturating_duration_since(self.origin));
        self.frames.tick(at);
        self.frames.viewed(self.view_time.take());
        self.pager.tick(self.now, self.size.width);
        if let Some((id, seen)) = self.warming {
            if seen >= 1 {
                self.warming = None;
                startup::step("renderer ready", Duration::ZERO);
                return Task::done(Message::RemoveWindow(id));
            }
            self.warming = Some((id, seen + 1));
        }
        let done = self.presence.is_done(self.now, look::OPEN_PRECISION);
        if done && !self.pager.moving(self.now) {
            self.frames.report(self.moving);
        }
        if done
            && !self.closing
            && std::mem::take(&mut self.reload_due)
            && let Some(reload) = self.reload.clone()
        {
            return Task::perform(async move { Arc::new(reload()) }, Message::Reloaded);
        }
        if !(self.closing && done) {
            return Task::none();
        }
        if !self.is_resident() {
            return iced::exit();
        }
        match self.surface {
            Some(id) if sherlock_ui::surface::gpu_drawn() == Some(true) => self.park(id),
            Some(id) => {
                self.surface = None;
                Task::done(Message::RemoveWindow(id))
            }
            None => Task::none(),
        }
    }

    /// Hides the surface without removing it: one transparent pixel in a corner, under the
    /// windows, without the keyboard. Its renderer keeps what it drew (the icons uploaded, the
    /// glyphs, the shaped names): a new surface took 60 ms more to show its first frame. On the
    /// GPU only, whose surfaces have alpha; in software the surface goes.
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
        let screen = crate::surface();
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

    /// Shows the view again, its search empty, on its first page, as a new view would be.
    fn show(&mut self) -> Task<Message> {
        let now = self.clock();
        // The frames of this opening only.
        self.frames = startup::Frames::default();
        self.query.clear();
        self.menu = None;
        self.launched = None;
        self.closing = false;
        self.moving = "open";
        self.paginate();
        self.presence.retarget(now, 1.0);
        let mut tasks = vec![hide_other(service::LAUNCHER), self.load_backdrop()];
        // Read again once the view is open, so that the reading never delays the opening.
        self.reload_due = !std::mem::replace(&mut self.fresh, false);
        match self.surface {
            Some(id) if self.parked => {
                // The opening starts from nothing, now.
                self.presence = Animation::new(look::OPEN_SPRING, 0.0, 1.0, now);
                tasks.push(self.unpark(id));
            }
            Some(_) => tasks.push(operation::focus(QUERY)),
            None => {
                let (id, task) = Message::layershell_open(crate::new_surface());
                self.surface = Some(id);
                tasks.push(task);
            }
        }
        Task::batch(tasks)
    }

    /// Side of an icon on the screen, in its pixels.
    fn icon_side(&self) -> u32 {
        icons::side(self.layout().icon, self.scale)
    }

    /// Draws on the pool the icons not drawn yet at their size on the screen, one batch at a
    /// time.
    fn draw_icons(&mut self) -> Task<Message> {
        if self.drawing || !self.rasterize {
            return Task::none();
        }
        let side = self.icon_side();
        let missing = self.catalog.tiles().iter().any(|tile| {
            self.catalog
                .icon(tile)
                .is_some_and(|path| self.rasters.get(&path, side).is_none())
        });
        if !missing {
            return Task::none();
        }
        self.drawing = true;
        let catalog = Arc::clone(&self.catalog);
        let done = if self.rasters.side == side {
            self.rasters.clone()
        } else {
            Rasters::default()
        };
        Task::perform(
            async move {
                let started = Instant::now();
                let rasters = icons::draw(&catalog, side, &done);
                tracing::info!(
                    icons = rasters.icons.len(),
                    side,
                    took_ms = startup::millis(started.elapsed()),
                    "icons drawn"
                );
                Arc::new(rasters)
            },
            Message::Rasters,
        )
    }

    /// The tiles of the query, from the first page.
    fn paginate(&mut self) {
        self.shown = self.catalog.filter(&self.query);
        self.selected = None;
        self.pager
            .reset(page_count(self.shown.len(), per_page(self.size)));
    }

    fn key(&mut self, key: Key) -> Task<Message> {
        match key {
            Key::Enter => {
                let first = (!self.query.trim().is_empty()).then_some(0);
                match self.selected.or(first) {
                    Some(place) => self.launch(place),
                    None => Task::none(),
                }
            }
            // Escape closes the menu, empties the search, then closes the view.
            Key::Escape if self.menu.is_some() => {
                self.menu = None;
                Task::none()
            }
            Key::Escape if !self.query.is_empty() => {
                self.query.clear();
                self.paginate();
                Task::none()
            }
            Key::Escape => self.close(),
            arrow => {
                self.select(arrow);
                Task::none()
            }
        }
    }

    /// Moves the selection with an arrow; the first one selects the first tile of the page.
    /// Up and Down stay on the page, Left and Right go on to the next one.
    fn select(&mut self, arrow: Key) {
        let count = self.shown.len();
        let layout = self.layout();
        let (columns, per_page) = (layout.columns, layout.per_page());
        let next = match self.selected {
            None => Some(self.pager.page() * per_page),
            Some(at) => match arrow {
                Key::Left => at.checked_sub(1),
                Key::Right => Some(at + 1),
                Key::Up => (at % per_page >= columns).then(|| at - columns),
                Key::Down => Some(at + columns).filter(|n| n / per_page == at / per_page),
                Key::Enter | Key::Escape => None,
            },
        };
        if let Some(next) = next.filter(|n| *n < count) {
            self.selected = Some(next);
            let now = self.clock();
            self.pager.go(next / per_page, now);
        }
    }

    /// Starts tile `place`; the view fades out once it is started.
    fn launch(&mut self, place: usize) -> Task<Message> {
        // A second Enter, or a click while fading out, starts nothing.
        if self.closing {
            return Task::none();
        }
        let Some(tile) = self
            .shown
            .get(place)
            .and_then(|index| self.catalog.tiles().get(*index))
        else {
            return Task::none();
        };
        let command = tile.command.clone();
        self.launched = Some(place);
        self.start(command)
    }

    /// Starts `command` once the compositor gave the activation token of the click (lot Q-52):
    /// an application already open that gets the launch brings its window forward with it.
    /// The view fades out meanwhile; a start that fails is only logged.
    fn start(&mut self, command: Command) -> Task<Message> {
        let spawn = Arc::clone(&self.spawn);
        let start = move |token: Option<String>| match spawn(&command, token.as_deref()) {
            Ok(pid) => tracing::info!(program = ?command.program, pid, "started"),
            Err(error) => tracing::error!(%error, "cannot start the application"),
        };
        let ask = match self.surface {
            Some(id) => Task::done(Message::RequestActivationToken {
                id,
                sink: launcher::launch::after_token(start),
            }),
            None => {
                start(None);
                Task::none()
            }
        };
        ask.chain(self.close())
    }

    /// Opens the menu of tile `place` where the pointer is.
    fn open_menu(&mut self, place: usize) {
        let Some(tile) = self.tile_at(place) else {
            return;
        };
        let pinned = self
            .dock_file
            .as_deref()
            .is_some_and(|path| dock::pins::is_pinned(path, tile.id()));
        let pin = if pinned { Entry::Unpin } else { Entry::Keep };
        let show = self.evidence().is_some();
        self.menu = Some(Menu {
            place,
            at: self.pointer,
            entries: vec![(pin, self.dock_file.is_some()), (Entry::Show, show)],
        });
    }

    /// The tile at `place` among those shown.
    fn tile_at(&self, place: usize) -> Option<&Tile> {
        self.shown
            .get(place)
            .and_then(|index| self.catalog.tiles().get(*index))
    }

    /// Evidence, when it is installed.
    fn evidence(&self) -> Option<&Tile> {
        self.catalog.tiles().iter().find(|t| t.id() == EVIDENCE)
    }

    fn pick(&mut self, entry: Entry) -> Task<Message> {
        let Some(menu) = self.menu.take() else {
            return Task::none();
        };
        let Some(tile) = self.tile_at(menu.place) else {
            return Task::none();
        };
        let id = tile.id().to_owned();
        match entry {
            Entry::Keep | Entry::Unpin => {
                let Some(path) = self.dock_file.clone() else {
                    return Task::none();
                };
                match dock::pins::set(&path, &id, entry == Entry::Keep) {
                    Ok(_) => tracing::info!(id, pinned = entry == Entry::Keep, "dock"),
                    Err(error) => tracing::error!(%error, "cannot write the dock's file"),
                }
                Task::none()
            }
            Entry::Show => self.show_in_evidence(menu.place),
        }
    }

    /// Opens Evidence on the folder of the desktop file of tile `place`, then fades out.
    fn show_in_evidence(&mut self, place: usize) -> Task<Message> {
        let folder = self
            .tile_at(place)
            .and_then(|t| t.entry.path.parent())
            .map(PathBuf::from);
        let command = self.evidence().zip(folder).and_then(|(evidence, folder)| {
            let (program, args) = evidence.entry.command(&[folder])?;
            Some(Command {
                program,
                args,
                dir: None,
            })
        });
        let Some(command) = command else {
            return Task::none();
        };
        self.start(command)
    }

    /// Fades the view out; at the last frame, the surface goes, or the program ends.
    fn close(&mut self) -> Task<Message> {
        if !self.closing {
            self.closing = true;
            self.moving = "close";
            let now = self.clock();
            self.presence.retarget(now, 0.0);
        }
        Task::none()
    }

    pub fn view(&self, id: Id) -> Element<'_, Message> {
        if self.warming.is_some_and(|(w, _)| w == id) {
            return view_warm_up();
        }
        let started = Instant::now();
        let view = self.view_screen();
        self.view_time
            .set(self.view_time.get().max(started.elapsed()));
        view
    }

    /// The whole screen: the blurred wallpaper (or the veil), the field, the pages, their dots.
    pub fn view_screen(&self) -> Element<'_, Message> {
        let shown = self.presence() as f32;
        let opacity = shown.clamp(0.0, 1.0);
        // The icons arrive from `OPEN_SCALE` and leave towards it; an application started stays
        // in place, its tile going to the dock.
        let scale = if self.launched.is_some() {
            1.0
        } else {
            look::OPEN_SCALE - (look::OPEN_SCALE - 1.0) * shown
        };
        let field = self.view_field();
        let layout = self.layout();
        let pages = reveal(self.view_pages(&layout, opacity)).scale(scale, (0.5, 0.5));
        let content = column![
            Space::new().height(layout.search_top),
            container(field).center_x(Length::Fill),
            Space::new().height(layout.grid_top - layout.search_top - look::SEARCH_HEIGHT),
            container(pages)
                .width(Length::Fill)
                .height(layout.rows as f32 * layout.cell.1),
            Space::new().height(Length::Fill),
            self.view_dots(),
            Space::new().height(look::DOTS_BOTTOM - look::DOT),
        ]
        .width(Length::Fill)
        .height(Length::Fill);
        let backdrop: Element<'_, Message> = match &self.backdrop {
            // An image takes its own opacity: `reveal` fades quads and texts.
            Some((_, picture)) => image(picture.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(ContentFit::Cover)
                .opacity(opacity)
                .into(),
            None => container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(look::veil)
                .into(),
        };
        let mut layers = vec![
            backdrop,
            mouse_area(content).on_press(Message::Dismiss).into(),
        ];
        if let Some(menu) = &self.menu {
            layers.push(self.view_menu(menu));
        }
        reveal(stack(layers)).opacity(opacity).into()
    }

    /// The search pill, the magnifier inside it.
    fn view_field(&self) -> Element<'_, Message> {
        let height = look::SEARCH_HEIGHT;
        let inset = metrics::FIELD_PADDING + metrics::ICON_SIZE_SMALL + metrics::SEARCH_ICON_GAP;
        let vertical = (height - typography::UI_SIZE * typography::LINE_HEIGHT) / 2.0;
        let field = ui::text_field(texts::PLACEHOLDER, &self.query, Some(Message::Query))
            .id(QUERY)
            .width(look::SEARCH_WIDTH)
            .padding(Padding {
                top: vertical,
                bottom: vertical,
                left: inset,
                right: metrics::FIELD_PADDING,
            })
            .style(look::field);
        let tint = look::Colors::of(color::palette()).field_icon;
        let magnifier = container(ui::icon(Icon::Search, metrics::ICON_SIZE_SMALL, tint))
            .padding(Padding {
                left: metrics::FIELD_PADDING,
                ..Padding::ZERO
            })
            .height(height)
            .center_y(height);
        stack![field, magnifier].into()
    }

    /// The page shown, and its neighbour while they slide.
    fn view_pages(&self, layout: &look::Layout, opacity: f32) -> Element<'_, Message> {
        if self.shown.is_empty() {
            let nothing = text(texts::NOTHING)
                .size(typography::UI_SIZE)
                .style(look::nothing);
            return container(nothing).center_x(Length::Fill).into();
        }
        let width = self.size.width.max(1.0);
        let at = self.pager.position(self.now, width);
        let first = (at / width).floor().max(0.0) as usize;
        let last = (first + 1).min(self.pager.count() - 1);
        let mut layers: Vec<Element<'_, Message>> = Vec::new();
        for page in first..=last {
            let offset = page as f32 * width - at;
            if offset.abs() >= width {
                continue;
            }
            let layer = reveal(self.view_page(page, layout, opacity)).offset(offset, 0.0);
            // Only the page the pages settle on takes the clicks.
            let layer = if page == self.pager.page() {
                layer
            } else {
                layer.inert()
            };
            layers.push(layer.into());
        }
        stack(layers)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_page(&self, page: usize, layout: &look::Layout, opacity: f32) -> Element<'_, Message> {
        let (columns, (width, height)) = (layout.columns, layout.cell);
        let start = page * layout.per_page();
        let lines = (0..layout.rows).map(|line| {
            let cells = (0..columns).map(|column| {
                let place = start + line * columns + column;
                let cell = match self.shown.get(place) {
                    Some(index) => self.view_tile(place, *index, layout, opacity),
                    None => Space::new().into(),
                };
                container(cell).center_x(width).center_y(height).into()
            });
            row(cells).into()
        });
        container(column(lines)).center_x(Length::Fill).into()
    }

    /// An application: its icon, or its initial, and its name cut with « … », white over a
    /// soft shadow.
    fn view_tile(
        &self,
        place: usize,
        index: usize,
        layout: &look::Layout,
        opacity: f32,
    ) -> Element<'_, Message> {
        let Some(tile) = self.catalog.tiles().get(index) else {
            return Space::new().into();
        };
        let icon: Element<'_, Message> = match self.catalog.icon(tile) {
            // Smooth edges on the GPU, rasterized once per size step (UI-05).
            Some(path) => sherlock_ui::raster::icon(&icon_handle(&path), layout.icon)
                .opacity(opacity)
                .into(),
            None => {
                let initial = tile
                    .name()
                    .chars()
                    .next()
                    .and_then(|c| c.to_uppercase().next())
                    .unwrap_or('?');
                container(
                    text(initial.to_string())
                        .font(typography::UI_SEMIBOLD)
                        .size(layout.letter_size()),
                )
                .center(layout.icon)
                .style(look::tile(layout.tile_radius()))
                .into()
            }
        };
        let name = |style: fn(&iced::Theme) -> text::Style| {
            container(ui::ellipsis(tile.name()).style(style)).center_x(layout.label_width())
        };
        let shadow = reveal(name(look::label_shadow)).offset(0.0, look::LABEL_SHADOW_OFFSET);
        let body = column![icon, stack![shadow, name(look::label)]]
            .spacing(look::LABEL_GAP * layout.factor)
            .align_x(Alignment::Center);
        let chosen =
            self.selected == Some(place) || self.menu.as_ref().is_some_and(|m| m.place == place);
        let cell = button(container(body).center(Length::Fill))
            .on_press(Message::Launch(place))
            .width(layout.button.0)
            .height(layout.button.1)
            .padding(look::CELL_PADDING)
            .style(look::cell(chosen));
        let cell = mouse_area(cell).on_right_press(Message::Menu(place));
        if self.launched != Some(place) {
            return cell.into();
        }
        // Started: it goes to the middle of the dock, shrinking to the dock's icon, as the view
        // fades out.
        let gone = (1.0 - self.presence() as f32).clamp(0.0, 1.0);
        let (from_x, from_y) = layout.center(place, self.size.width);
        let (to_x, to_y) = look::dock_target(self.size.width, self.size.height);
        let side = 1.0 + (look::LAUNCH_SIDE / layout.icon - 1.0) * gone;
        reveal(cell)
            .offset((to_x - from_x) * gone, (to_y - from_y) * gone)
            .scale(side, (0.5, 0.5))
            .into()
    }

    /// The menu of a tile at the pointer, kept on the screen; a click beside closes it.
    fn view_menu(&self, menu: &Menu) -> Element<'_, Message> {
        let items = menu.entries.iter().map(|(entry, enabled)| {
            let pick = enabled.then_some(Message::MenuPick(*entry));
            ui::menu_item(entry.label(), Trailing::None, pick).into()
        });
        let width = look::MENU_WIDTH;
        let height =
            2.0 * metrics::MENU_PADDING + menu.entries.len() as f32 * metrics::MENU_ITEM_HEIGHT;
        let x = menu
            .at
            .x
            .min(self.size.width - width - look::MENU_MARGIN)
            .max(0.0);
        let y = menu
            .at
            .y
            .min(self.size.height - height - look::MENU_MARGIN)
            .max(0.0);
        let placed = pin(ui::floating_menu(items, width)).x(x).y(y);
        mouse_area(placed)
            .on_press(Message::CloseMenu)
            .on_right_press(Message::CloseMenu)
            .into()
    }

    /// A dot for each page, when there are several; a click goes to that page.
    fn view_dots(&self) -> Element<'_, Message> {
        let count = self.pager.count();
        if count < 2 {
            return Space::new().height(look::DOT).into();
        }
        let dots = (0..count).map(|page| {
            let dot = container(Space::new())
                .width(look::DOT)
                .height(look::DOT)
                .style(look::dot(page == self.pager.page()));
            mouse_area(dot).on_press(Message::Page(page)).into()
        });
        container(row(dots).spacing(look::DOT_GAP))
            .center_x(Length::Fill)
            .into()
    }
}

/// The logical size and scale of the screen a surface landed on.
fn output(event: ShellEvent) -> Message {
    let output = match event {
        ShellEvent::WindowOutputChanged {
            output: Some(output),
            ..
        }
        | ShellEvent::OutputAdded(output)
        | ShellEvent::OutputUpdated(output) => output,
        _ => return Message::Output(None),
    };
    let found = output.logical_size.filter(|(w, h)| *w > 0 && *h > 0);
    Message::Output(found.map(|(w, h)| {
        (
            Size::new(w as f32, h as f32),
            output.scale_factor.max(1) as f32,
        )
    }))
}

/// Applications on a page of a screen of `size`.
fn per_page(size: Size) -> usize {
    look::Layout::fit(size.width, size.height).per_page()
}

/// The arrows, Enter, Escape, the wheel and the touchpad, and the size of the surface, even
/// when the search field took them. A held Enter starts nothing more.
pub fn on_event(event: Event, _: event::Status, _: Id) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed { key, repeat, .. }) => {
            let key = match key {
                keyboard::Key::Named(Named::ArrowLeft) => Key::Left,
                keyboard::Key::Named(Named::ArrowRight) => Key::Right,
                keyboard::Key::Named(Named::ArrowUp) => Key::Up,
                keyboard::Key::Named(Named::ArrowDown) => Key::Down,
                keyboard::Key::Named(Named::Enter) if !repeat => Key::Enter,
                keyboard::Key::Named(Named::Escape) => Key::Escape,
                _ => return None,
            };
            Some(Message::Key(key))
        }
        Event::Mouse(mouse::Event::WheelScrolled { delta }) => Some(Message::Scroll(delta)),
        Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::Pointer(position)),
        Event::Window(window::Event::Resized(size)) => Some(Message::Resized(size)),
        _ => None,
    }
}
