//! The bubbles: state, messages, update and view. One layer-shell surface holds the stack; it
//! exists only while a bubble shows.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::font::{Style, Weight};
use iced::futures::channel::mpsc::{self, Receiver};
use iced::futures::{Stream, StreamExt, stream};
use iced::widget::{
    Stack, column, container, float, image, mouse_area, pin, rich_text, row, sensor, span, svg,
    text,
};
use iced::{Alignment, Element, Font, Length, Padding, Subscription, Task, Vector, mouse, window};
use iced_exwlshell::actions::RegionActionCB;
use iced_exwlshell::reexport::{ActivationTokenSink, Anchor, LayerSize, XdgActivationTokenV1State};
use iced_exwlshell::shell::{ShellEvent, ShellReceiver};
use iced_exwlshell::to_layer_message;
use sherlock_ui::animated::hoverable;
use sherlock_ui::ellipsis::ellipsis;
use sherlock_ui::motion::{Animation, GEOMETRY_PRECISION};
use sherlock_ui::notifications::{self as ui, SPRING};
use sherlock_ui::{color, typography};

use crate::core::{Core, Event, Sink};
use crate::layout::{self, OUT, SURFACE_WIDTH};
use crate::model::{Icon, Notification, Urgency};
use crate::picture;

/// The messages of a bubble name it by its serial, not by the id of its notification: an id
/// replaced while its old bubble slides out names two bubbles.
#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Core(Event),
    Theme(color::Theme),
    Frame(Instant),
    /// The height a bubble took once laid out.
    Measured(u64, f32),
    Hover(u64, bool),
    /// A button pressed on the bubble, and let go: the click (its default action, or it
    /// closes).
    Press(u64),
    Click(u64),
    /// « × », or the bubble swiped away to the right (D-23d).
    Dismiss(u64),
    /// A horizontal scroll over a bubble: the fingers on the touchpad, in pixels.
    Swipe(u64, f32),
    /// The pile of an application opened, or closed again.
    Expand(String, bool),
    Action(u64, String),
    /// « Options » of a bubble with more actions than its row holds.
    Options(u64),
    /// The screen of the bubbles, its logical height: as many bubbles show as it holds.
    Screen(f32),
}

/// The events of the core waiting for the interface, merged so that they stay few however
/// late the interface is: a newer `Show` of a bubble takes the place of the pending one, and a
/// bubble the interface never got needs no `Close`.
#[derive(Default)]
struct Pending {
    events: Vec<Event>,
    /// The bubbles the interface got and still shows.
    delivered: HashSet<u32>,
}

impl Pending {
    fn add(&mut self, event: Event) {
        match event {
            Event::Show(note) => {
                let same = |e: &&mut Event| matches!(e, Event::Show(n) if n.id == note.id);
                match self.events.iter_mut().find(same) {
                    Some(slot) => *slot = Event::Show(note),
                    None => self.events.push(Event::Show(note)),
                }
            }
            Event::Close(id) => {
                self.events
                    .retain(|e| !matches!(e, Event::Show(n) if n.id == id));
                let pending = self
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::Close(i) if *i == id));
                if self.delivered.contains(&id) && !pending {
                    self.events.push(Event::Close(id));
                }
            }
        }
    }

    fn take(&mut self) -> Vec<Event> {
        let events = std::mem::take(&mut self.events);
        for event in &events {
            match event {
                Event::Show(note) => self.delivered.insert(note.id),
                Event::Close(id) => self.delivered.remove(id),
            };
        }
        events
    }
}

/// The events of the core, handed once to the subscription. The core never waits on the
/// interface: it adds to `pending` and wakes the stream through a channel of one.
#[derive(Clone)]
pub struct Inbox {
    pending: Arc<Mutex<Pending>>,
    wake: Arc<Mutex<Option<Receiver<()>>>>,
}

impl Inbox {
    /// The sink to give the core, and the inbox its events arrive in.
    pub fn new() -> (Sink, Self) {
        let (sender, receiver) = mpsc::channel(0);
        let pending = Arc::new(Mutex::new(Pending::default()));
        let shared = Arc::clone(&pending);
        let sender = Mutex::new(sender);
        let sink: Sink = Box::new(move |event| {
            if let Ok(mut pending) = shared.lock() {
                pending.add(event);
            }
            // Full: a wake is already on its way, and will take this event too.
            if let Ok(mut sender) = sender.lock() {
                let _ = sender.try_send(());
            }
        });
        let wake = Arc::new(Mutex::new(Some(receiver)));
        (sink, Self { pending, wake })
    }

    fn stream(&self) -> impl Stream<Item = Event> + use<> {
        let pending = Arc::clone(&self.pending);
        match self.wake.lock().ok().and_then(|mut r| r.take()) {
            Some(wake) => wake
                .flat_map(move |()| {
                    let events = pending.lock().map(|mut p| p.take()).unwrap_or_default();
                    stream::iter(events)
                })
                .boxed(),
            None => stream::empty().boxed(),
        }
    }
}

impl Hash for Inbox {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "inbox".hash(state);
    }
}

/// The picture of a bubble, built once, from bytes only.
#[derive(Debug, Clone)]
enum Glyph {
    Svg(svg::Handle),
    Image(image::Handle),
    Letter(char),
}

impl Glyph {
    fn of(note: &Notification) -> Self {
        match &note.icon {
            Icon::Pixels {
                width,
                height,
                rgba,
            } => Self::Image(image::Handle::from_rgba(*width, *height, rgba.to_vec())),
            Icon::Svg(bytes) => Self::Svg(svg::Handle::from_memory(bytes.to_vec())),
            Icon::Png(bytes) => Self::Svg(svg::Handle::from_memory(picture::png_in_svg(bytes))),
            // Theme names wait for the shared icon lookup: the initial of the application.
            Icon::Name(_) | Icon::None => {
                let letter = app_name(note).chars().next().unwrap_or('?');
                Self::Letter(letter.to_uppercase().next().unwrap_or(letter))
            }
        }
    }
}

fn app_name(note: &Notification) -> &str {
    if note.app_name.is_empty() {
        ui::text::UNKNOWN_APP
    } else {
        &note.app_name
    }
}

struct Bubble {
    serial: u64,
    note: Arc<Notification>,
    glyph: Glyph,
    /// Unknown until the bubble is laid out: it waits out of sight.
    height: Option<f32>,
    top: Animation,
    slide: Animation,
    leaving: bool,
    /// Its « Options » list is open.
    options: bool,
    /// The pointer is over it; a button is held on it.
    hovered: bool,
    pressed: bool,
    /// How far the fingers pulled it to the right, and when they last moved.
    swipe: f32,
    swiped_at: Option<Duration>,
    /// Laid behind the newest bubble of its application (a pile); the others of its pile,
    /// counted on the newest; the newest of an opened pile.
    behind: bool,
    others: usize,
    expanded_leader: bool,
    /// Behind: the height of the bubble in front, so that its edge peeks out below it.
    front_height: Option<f32>,
}

pub struct Notifications {
    core: Arc<Core>,
    inbox: Inbox,
    /// iced_exwlshell's reports of the screen the surface is on.
    shell_events: Option<ShellReceiver>,
    /// In the order they came: the newest is drawn at the top.
    bubbles: Vec<Bubble>,
    surface: Option<window::Id>,
    height: u32,
    origin: Instant,
    now: Duration,
    serials: u64,
    /// The applications whose pile is open: each of their bubbles shows whole.
    expanded: HashSet<String>,
}

impl Notifications {
    pub fn new(core: Arc<Core>, inbox: Inbox) -> Self {
        Self {
            core,
            inbox,
            shell_events: None,
            bubbles: Vec::new(),
            surface: None,
            height: 0,
            origin: Instant::now(),
            now: Duration::ZERO,
            serials: 0,
            expanded: HashSet::new(),
        }
    }

    /// Told by `shell_events` of the screen of the stack: its height sets how many bubbles
    /// show (D-23c).
    pub fn with_shell_events(mut self, shell_events: ShellReceiver) -> Self {
        self.shell_events = Some(shell_events);
        self
    }

    fn animating(&self) -> bool {
        let now = self.now;
        self.bubbles.iter().any(|b| {
            !b.top.is_done(now, GEOMETRY_PRECISION)
                || !b.slide.is_done(now, GEOMETRY_PRECISION)
                || b.swipe != 0.0
        })
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let frames = if self.animating() {
            window::frames().map(Message::Frame)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            Subscription::run_with(self.inbox.clone(), Inbox::stream).map(Message::Core),
            Subscription::run(appearance).map(Message::Theme),
            self.shell_events
                .as_ref()
                .map_or_else(Subscription::none, |events| {
                    events.listen().filter_map(on_output)
                }),
            frames,
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        self.now = self.origin.elapsed();
        let now = self.now;
        match message {
            Message::Core(Event::Show(note)) => return self.show(note),
            Message::Core(Event::Close(id)) => {
                for bubble in self.bubbles.iter_mut().filter(|b| b.note.id == id) {
                    bubble.leaving = true;
                    bubble.slide.retarget(now, f64::from(OUT));
                }
                // A bubble still out of sight has nothing to slide: it goes at once.
                return self.sweep().unwrap_or_else(|| self.relayout(false));
            }
            Message::Measured(serial, height) => {
                let bubble = self.bubbles.iter_mut().find(|b| b.serial == serial);
                if let Some(bubble) = bubble.filter(|b| !b.leaving) {
                    if bubble.height.is_none() {
                        bubble.slide.retarget(now, 0.0);
                    }
                    bubble.height = Some(height);
                }
                return self.relayout(false);
            }
            Message::Frame(_) => {
                if let Some(task) = self.end_swipes() {
                    return task;
                }
                if let Some(task) = self.sweep() {
                    return task;
                }
                if !self.animating() {
                    return self.relayout(true);
                }
            }
            // A bubble on its way out takes nothing: its id may already name another one.
            Message::Hover(serial, held) => {
                if let Some(bubble) = self.bubble_mut(serial) {
                    bubble.hovered = held;
                    bubble.pressed &= held;
                }
                if let Some(id) = self.live(serial) {
                    self.core.hold(id, held);
                }
            }
            Message::Press(serial) => {
                if let Some(bubble) = self.bubble_mut(serial) {
                    bubble.pressed = true;
                }
            }
            // The application gets the activation token of the click first (lot Q-52).
            Message::Click(serial) => {
                let pressed = self
                    .bubble_mut(serial)
                    .is_some_and(|b| std::mem::replace(&mut b.pressed, false) && b.swipe == 0.0);
                if let (true, Some(id)) = (pressed, self.live(serial)) {
                    let core = Arc::clone(&self.core);
                    return self
                        .after_token(move |token| core.click_activated(id, token.as_deref()));
                }
            }
            Message::Dismiss(serial) => return self.dismiss(serial),
            Message::Swipe(serial, dx) => {
                if let Some(bubble) = self.bubble_mut(serial).filter(|b| !b.leaving) {
                    // Pulled left, it barely moves: it only goes away to the right.
                    let pulled = bubble.swipe + dx;
                    bubble.swipe = if pulled < 0.0 {
                        pulled * ui::SWIPE_RESISTANCE
                    } else {
                        pulled
                    };
                    bubble.swiped_at = Some(now);
                    bubble.pressed = false;
                }
            }
            Message::Expand(app, open) => {
                if open {
                    self.expanded.insert(app);
                } else {
                    self.expanded.remove(&app);
                }
                return self.relayout(false);
            }
            Message::Action(serial, key) => {
                if let Some(id) = self.live(serial) {
                    let core = Arc::clone(&self.core);
                    return self.after_token(move |token| {
                        core.invoke_activated(id, &key, token.as_deref());
                    });
                }
            }
            Message::Options(serial) => {
                if let Some(bubble) = self.bubbles.iter_mut().find(|b| b.serial == serial) {
                    bubble.options = !bubble.options;
                }
            }
            Message::Screen(height) => self.core.set_screen_height(height),
            Message::Theme(theme) => color::set_active(theme),
            _ => {}
        }
        Task::none()
    }

    /// Runs `then` with the activation token the compositor gives for the click on the bubbles'
    /// surface, or without one.
    fn after_token(&self, then: impl FnOnce(Option<String>) + Send + 'static) -> Task<Message> {
        match self.surface {
            Some(id) => Task::done(Message::RequestActivationToken {
                id,
                sink: after_token(then),
            }),
            None => {
                then(None);
                Task::none()
            }
        }
    }

    /// The notification id of the bubble `serial`, unless it is leaving.
    fn live(&self, serial: u64) -> Option<u32> {
        let bubble = self.bubbles.iter().find(|b| b.serial == serial)?;
        (!bubble.leaving).then_some(bubble.note.id)
    }

    fn bubble_mut(&mut self, serial: u64) -> Option<&mut Bubble> {
        self.bubbles.iter_mut().find(|b| b.serial == serial)
    }

    /// Closes the bubble `serial` as the user asked (« × », a swipe): it slides out from where
    /// it is, and the application hears it was dismissed.
    fn dismiss(&mut self, serial: u64) -> Task<Message> {
        let now = self.now;
        let Some(id) = self.live(serial) else {
            return Task::none();
        };
        if let Some(bubble) = self.bubble_mut(serial) {
            let from = f64::from(at(&bubble.slide, now) + bubble.swipe);
            bubble.slide = Animation::new(SPRING, from, f64::from(OUT), now);
            bubble.swipe = 0.0;
            bubble.swiped_at = None;
        }
        self.core.dismiss(id);
        Task::none()
    }

    /// The swipes whose fingers rested `ui::SWIPE_END`: pulled far enough, the bubble goes;
    /// else it springs back.
    fn end_swipes(&mut self) -> Option<Task<Message>> {
        let now = self.now;
        let mut gone = Vec::new();
        for bubble in &mut self.bubbles {
            let Some(at_time) = bubble.swiped_at else {
                continue;
            };
            if now.saturating_sub(at_time) < ui::SWIPE_END {
                continue;
            }
            if bubble.swipe >= ui::WIDTH * ui::SWIPE_DISMISS {
                gone.push(bubble.serial);
            } else {
                let from = f64::from(at(&bubble.slide, now) + bubble.swipe);
                bubble.slide = Animation::new(SPRING, from, 0.0, now);
                bubble.swipe = 0.0;
                bubble.swiped_at = None;
            }
        }
        let tasks: Vec<_> = gone
            .into_iter()
            .map(|serial| self.dismiss(serial))
            .collect();
        (!tasks.is_empty()).then(|| Task::batch(tasks))
    }

    /// Drops the bubbles that slid out, and the surface with the last one.
    fn sweep(&mut self) -> Option<Task<Message>> {
        let now = self.now;
        let gone = |b: &Bubble| b.leaving && b.slide.is_done(now, GEOMETRY_PRECISION);
        self.bubbles.retain(|b| !gone(b));
        if !self.bubbles.is_empty() {
            return None;
        }
        self.height = 0;
        self.surface
            .take()
            .map(|id| Task::done(Message::RemoveWindow(id)))
    }

    /// A new bubble slides in once laid out; a replaced one changes in place.
    fn show(&mut self, note: Arc<Notification>) -> Task<Message> {
        let glyph = Glyph::of(&note);
        let now = self.now;
        match self
            .bubbles
            .iter_mut()
            .find(|b| b.note.id == note.id && !b.leaving)
        {
            Some(bubble) => (bubble.note, bubble.glyph) = (note, glyph),
            None => self.bubbles.push(Bubble {
                serial: {
                    self.serials += 1;
                    self.serials
                },
                note,
                glyph,
                height: None,
                top: Animation::new(SPRING, 0.0, 0.0, now),
                slide: Animation::new(SPRING, f64::from(OUT), f64::from(OUT), now),
                leaving: false,
                options: false,
                hovered: false,
                pressed: false,
                swipe: 0.0,
                swiped_at: None,
                behind: false,
                others: 0,
                expanded_leader: false,
                front_height: None,
            }),
        }
        let open = match self.surface {
            Some(_) => Task::none(),
            None => {
                // Room to lay the first bubble out at its true height.
                self.height = ui::MEASURE_ROOM as u32;
                let (id, task) = Message::layershell_open(crate::surface(self.height));
                self.surface = Some(id);
                task
            }
        };
        Task::batch([open, self.relayout(false)])
    }

    /// Sends the bubbles that stay to their place in the stack, grows the surface for them
    /// (shrinks it too once everything settled, `settled`) and sets the input region.
    fn relayout(&mut self, settled: bool) -> Task<Message> {
        let now = self.now;
        let staying: Vec<usize> = (0..self.bubbles.len())
            .rev()
            .filter(|&i| !self.bubbles[i].leaving && self.bubbles[i].height.is_some())
            .collect();
        // The bubbles of one application pile up behind its newest one, unless its pile is
        // open (D-23d).
        let apps: Vec<String> = staying
            .iter()
            .map(|&i| app_name(&self.bubbles[i].note).to_owned())
            .collect();
        self.expanded
            .retain(|app| apps.iter().filter(|a| *a == app).count() > 1);
        let piles = layout::piles(&apps, |app| self.expanded.contains(app));
        let newest_of_app: Vec<usize> = staying
            .iter()
            .zip(&apps)
            .enumerate()
            .filter(|(k, (_, app))| apps.iter().position(|a| a == *app) == Some(*k))
            .map(|(_, (&i, _))| i)
            .collect();
        let heights: Vec<f32> = staying
            .iter()
            .filter_map(|&i| self.bubbles[i].height)
            .collect();
        for ((&i, top), place) in staying
            .iter()
            .zip(layout::stack_piled(&heights, &piles))
            .zip(&piles)
        {
            let leads = self.expanded.contains(app_name(&self.bubbles[i].note))
                && newest_of_app.contains(&i);
            let front = place
                .behind
                .then(|| {
                    staying
                        .get(place.leader)
                        .and_then(|&j| self.bubbles[j].height)
                })
                .flatten();
            let bubble = &mut self.bubbles[i];
            bubble.front_height = front;
            bubble.behind = place.behind;
            bubble.others = place.others;
            bubble.expanded_leader = leads;
            let animation = &mut bubble.top;
            if (animation.target() - f64::from(top)).abs() > f64::EPSILON {
                animation.retarget(now, f64::from(top));
            }
        }
        let Some(id) = self.surface else {
            return Task::none();
        };
        let waiting = self
            .bubbles
            .iter()
            .any(|b| b.height.is_none() && !b.leaving);
        let room = if waiting {
            ui::MEASURE_ROOM
        } else {
            ui::STACK_SLACK
        };
        let needed = layout::surface_height(self.bubbles.iter().map(|b| {
            let top = b.top.value(now).max(b.top.target()) as f32;
            (top, b.height.unwrap_or(0.0) + room)
        }));
        let mut tasks = Vec::new();
        if needed > self.height || (settled && needed < self.height) {
            self.height = needed;
            tasks.push(Task::done(Message::LayoutChange {
                id,
                anchor: Anchor::Top | Anchor::Right,
                size: LayerSize::px(SURFACE_WIDTH as u32, needed),
            }));
        }
        let areas = layout::region(staying.iter().filter_map(|&i| {
            let bubble = &self.bubbles[i];
            Some((bubble.top.target() as f32, bubble.height?))
        }));
        let callback = RegionActionCB::new(move |region| {
            for &(x, y, width, height) in &areas {
                if let Err(error) = region.add(x, y, width, height) {
                    tracing::warn!(%error, "cannot set the input region");
                }
            }
        });
        tasks.push(Task::done(Message::SetInputRegion { id, callback }));
        Task::batch(tasks)
    }

    pub fn view(&self, _surface: window::Id) -> Element<'_, Message> {
        let now = self.now;
        let layers = self.bubbles.iter().map(|b| {
            let id = b.serial;
            let dx = at(&b.slide, now) + b.swipe;
            let slid = float(bubble(b)).translate(move |_, _| Vector::new(dx, 0.0));
            // Keyed by the serial: a bubble that takes the place of another is measured anew.
            let measured = sensor(slid)
                .key(id)
                .anticipate(f32::MAX)
                .on_show(move |size| Message::Measured(id, size.height))
                .on_resize(move |size| Message::Measured(id, size.height));
            pin(measured).y(at(&b.top, now)).into()
        });
        Stack::with_children(layers)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// The value of `animation` at `now`, its target once settled: a bubble at rest is exactly in
/// place, not floating a fraction of a pixel away.
fn at(animation: &Animation, now: Duration) -> f32 {
    if animation.is_done(now, GEOMETRY_PRECISION) {
        animation.target() as f32
    } else {
        animation.value(now) as f32
    }
}

/// A bubble: application, title, body and action buttons, as a card of the board. A click
/// closes it or runs its default action; the pointer over it holds its timer.
fn bubble(b: &Bubble) -> Element<'_, Message> {
    let note = &b.note;
    let id = b.serial;
    // Behind the newest of its application, only its edge shows: an empty card of its height,
    // so that nothing of its text shows through the glass in front.
    if b.behind {
        let card = container(
            iced::widget::space()
                .width(ui::WIDTH)
                .height(b.front_height.or(b.height).unwrap_or(0.0)),
        )
        .style(ui::bubble(note.urgency == Urgency::Critical));
        return mouse_area(card)
            .on_press(Message::Expand(app_name(note).to_owned(), true))
            .interaction(mouse::Interaction::Pointer)
            .into();
    }
    let glyph: Element<'_, Message> = match &b.glyph {
        Glyph::Svg(handle) => svg(handle.clone()).width(ui::TILE).height(ui::TILE).into(),
        Glyph::Image(handle) => image(handle.clone())
            .width(ui::TILE)
            .height(ui::TILE)
            .into(),
        Glyph::Letter(letter) => container(
            text(letter.to_string())
                .size(ui::TILE_LETTER_SIZE)
                .font(typography::UI_SEMIBOLD),
        )
        .center(ui::TILE)
        .style(ui::tile)
        .into(),
    };
    // The application's name and the title on one line each, ending with « … » (D-23c).
    let mut header = row![
        glyph,
        ellipsis(app_name(note))
            .size(ui::HEADER_SIZE)
            .style(ui::secondary_text)
            .width(Length::Fill)
    ]
    .spacing(ui::HEADER_GAP)
    .align_y(Alignment::Center);
    // « × » while the pointer is over it (D-23d).
    if b.hovered && !b.behind {
        header = header.push(ui::close_button(Message::Dismiss(id)));
    }
    let mut content = column![header].spacing(ui::ROW_GAP);
    if !note.summary.is_empty() {
        content = content.push(ellipsis(note.summary.as_str()).font(typography::UI_SEMIBOLD));
    }
    if !note.body.is_empty() {
        let secondary = color::palette().text_secondary;
        let spans: Vec<text::Span<'_, (), Font>> = note
            .body
            .iter()
            .map(|s| {
                let font = Font {
                    weight: if s.bold {
                        Weight::Semibold
                    } else {
                        Weight::Normal
                    },
                    style: if s.italic {
                        Style::Italic
                    } else {
                        Style::Normal
                    },
                    ..typography::UI
                };
                span(s.text.as_str()).font(font).color(secondary)
            })
            .collect();
        // A word longer than the bubble (an address) breaks where it must; four lines at most.
        let body = rich_text(spans)
            .wrapping(text::Wrapping::WordOrGlyph)
            .line_height(ui::BODY_LINE_HEIGHT)
            .width(Length::Fill);
        let lines = ui::BODY_LINES * typography::UI_SIZE * ui::BODY_LINE_HEIGHT;
        content = content.push(container(body).max_height(lines.ceil()).clip(true));
    }
    let actions: Vec<(&str, Message)> = note
        .buttons()
        .map(|(key, label)| (label.as_str(), Message::Action(id, key.clone())))
        .collect();
    if !actions.is_empty() {
        let actions = ui::action_row(actions, b.options, Message::Options(id));
        content = content.push(container(actions).padding(Padding::ZERO.top(ui::BUTTONS_TOP)));
    }
    // Under the newest of a pile, the others it hides (or « Afficher moins » once open).
    let app = app_name(note).to_owned();
    if b.others > 0 {
        content = content.push(pile_button(
            ui::text::more(b.others),
            Message::Expand(app.clone(), true),
        ));
    } else if b.expanded_leader {
        content = content.push(pile_button(
            ui::text::SHOW_LESS.to_owned(),
            Message::Expand(app.clone(), false),
        ));
    }
    let p = color::palette();
    // The veil of the hover and of the press fades in over the glass, under the content.
    let inside = hoverable(container(content).padding(ui::PADDING), p.hover, ui::RADIUS)
        .pressed(p.pressed)
        .key(id);
    let card = container(inside)
        .width(ui::WIDTH)
        .style(ui::bubble(note.urgency == Urgency::Critical));
    // The blur behind its glass (lot D-37b).
    let card = sherlock_ui::glass::glass(card, ui::RADIUS);
    // A bubble behind another opens its pile; the others take a click, a press, a swipe.
    let area = mouse_area(card)
        .on_enter(Message::Hover(id, true))
        .on_exit(Message::Hover(id, false))
        .interaction(mouse::Interaction::Pointer);
    area.on_press(Message::Press(id))
        .on_release(Message::Click(id))
        .on_scroll(move |delta| match delta {
            mouse::ScrollDelta::Pixels { x, .. } => Message::Swipe(id, x),
            mouse::ScrollDelta::Lines { x, .. } => Message::Swipe(id, x * ui::SWIPE_LINE),
        })
        .into()
}

/// The line under a pile of bubbles: « 2 autres notifications », « Afficher moins ».
fn pile_button<'a>(label: String, message: Message) -> Element<'a, Message> {
    let label = text(label).size(ui::HEADER_SIZE).style(ui::secondary_text);
    iced::widget::button(label)
        .padding(0)
        .style(|_, _| iced::widget::button::Style::default())
        .on_press(message)
        .into()
}

/// The logical height of the screen of the stack, from iced_exwlshell's reports.
fn on_output(event: ShellEvent) -> Option<Message> {
    match event {
        ShellEvent::WindowOutputChanged { output, .. } => {
            let (_, height) = output?.info.logical_size?;
            Some(Message::Screen(height as f32))
        }
        _ => None,
    }
}

/// The theme of the appearance file, each time it changes.
fn appearance() -> impl Stream<Item = color::Theme> {
    let (sender, receiver) = mpsc::unbounded();
    if let Some(path) = appearance_config::path() {
        let followed = appearance_config::follow(path, move |event| match event {
            appearance_config::Event::Theme(theme) => sender.unbounded_send(theme).is_ok(),
            _ => true,
        });
        if let Err(error) = followed {
            tracing::warn!(%error, "cannot follow the theme");
        }
    }
    receiver
}

/// A sink for the activation token the compositor gives on behalf of the surface where the user
/// clicked (lot Q-52): `then` runs once with it, or without one when none comes (no
/// `xdg-activation-v1`, the surface gone, the request dropped).
fn after_token(then: impl FnOnce(Option<String>) + Send + 'static) -> ActivationTokenSink {
    type Then = Box<dyn FnOnce(Option<String>) + Send>;
    struct Once(Mutex<Option<Then>>);
    impl Once {
        fn run(&self, token: Option<String>) {
            let then = self.0.lock().ok().and_then(|mut then| then.take());
            if let Some(then) = then {
                then(token);
            }
        }
    }
    impl Drop for Once {
        fn drop(&mut self) {
            self.run(None);
        }
    }
    let once = Once(Mutex::new(Some(Box::new(then))));
    ActivationTokenSink::callback(move |state| {
        let token = match state {
            XdgActivationTokenV1State::Ready(token) => Some(token.token().to_owned()),
            XdgActivationTokenV1State::Unsupported | XdgActivationTokenV1State::NoSurface => None,
        };
        once.run(token);
    })
}
