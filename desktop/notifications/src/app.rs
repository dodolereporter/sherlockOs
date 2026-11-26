//! The bubbles: state, messages, update and view. One layer-shell surface holds the stack; it
//! exists only while a bubble shows.

use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::font::{Style, Weight};
use iced::futures::channel::mpsc::{self, UnboundedReceiver};
use iced::futures::{Stream, StreamExt, stream};
use iced::widget::{
    Stack, button, column, container, float, image, mouse_area, pin, rich_text, row, sensor, span,
    svg, text,
};
use iced::{Alignment, Element, Font, Length, Padding, Subscription, Task, Vector, mouse, window};
use iced_exwlshell::actions::RegionActionCB;
use iced_exwlshell::reexport::{Anchor, LayerSize};
use iced_exwlshell::to_layer_message;
use sherlock_ui::motion::{Animation, GEOMETRY_PRECISION};
use sherlock_ui::notifications::{self as ui, SPRING};
use sherlock_ui::{color, typography};

use crate::core::{Core, Event, Sink};
use crate::layout::{self, OUT, SURFACE_WIDTH};
use crate::model::{Icon, Notification, Urgency};

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Core(Event),
    Theme(color::Theme),
    Frame(Instant),
    /// The height a bubble took once laid out.
    Measured(u32, f32),
    Hover(u32, bool),
    Click(u32),
    Action(u32, String),
}

/// The events of the core, handed once to the subscription.
#[derive(Clone)]
pub struct Inbox(Arc<Mutex<Option<UnboundedReceiver<Event>>>>);

impl Inbox {
    /// The sink to give the core, and the inbox its events arrive in.
    pub fn new() -> (Sink, Self) {
        let (sender, receiver) = mpsc::unbounded();
        let sink: Sink = Box::new(move |event| {
            let _ = sender.unbounded_send(event);
        });
        (sink, Self(Arc::new(Mutex::new(Some(receiver)))))
    }

    fn stream(&self) -> impl Stream<Item = Event> + use<> {
        match self.0.lock().ok().and_then(|mut r| r.take()) {
            Some(receiver) => receiver.boxed(),
            None => stream::empty().boxed(),
        }
    }
}

impl Hash for Inbox {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "inbox".hash(state);
    }
}

/// The picture of a bubble, built once.
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
            Icon::File(path) => Self::Svg(file_handle(path)),
            // Theme names wait for the shared icon lookup: the initial of the application.
            Icon::Name(_) | Icon::None => {
                let letter = app_name(note).chars().next().unwrap_or('?');
                Self::Letter(letter.to_uppercase().next().unwrap_or(letter))
            }
        }
    }
}

/// An SVG file as it is; any other picture inside an SVG, which resvg decodes (as the dock).
fn file_handle(path: &Path) -> svg::Handle {
    if path.extension().is_some_and(|e| e == "svg") {
        return svg::Handle::from_path(path);
    }
    let href = path
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let document = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         viewBox=\"0 0 1 1\"><image width=\"1\" height=\"1\" xlink:href=\"{href}\"/></svg>"
    );
    svg::Handle::from_memory(document.into_bytes())
}

fn app_name(note: &Notification) -> &str {
    if note.app_name.is_empty() {
        ui::text::UNKNOWN_APP
    } else {
        &note.app_name
    }
}

struct Bubble {
    note: Arc<Notification>,
    glyph: Glyph,
    /// Unknown until the bubble is laid out: it waits out of sight.
    height: Option<f32>,
    top: Animation,
    slide: Animation,
    leaving: bool,
}

pub struct Notifications {
    core: Arc<Core>,
    inbox: Inbox,
    /// In the order they came: the newest is drawn at the top.
    bubbles: Vec<Bubble>,
    surface: Option<window::Id>,
    height: u32,
    origin: Instant,
    now: Duration,
}

impl Notifications {
    pub fn new(core: Arc<Core>, inbox: Inbox) -> Self {
        Self {
            core,
            inbox,
            bubbles: Vec::new(),
            surface: None,
            height: 0,
            origin: Instant::now(),
            now: Duration::ZERO,
        }
    }

    fn animating(&self) -> bool {
        let now = self.now;
        self.bubbles.iter().any(|b| {
            !b.top.is_done(now, GEOMETRY_PRECISION) || !b.slide.is_done(now, GEOMETRY_PRECISION)
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
            Message::Measured(id, height) => {
                let bubble = self
                    .bubbles
                    .iter_mut()
                    .find(|b| b.note.id == id && !b.leaving);
                if let Some(bubble) = bubble {
                    if bubble.height.is_none() {
                        bubble.slide.retarget(now, 0.0);
                    }
                    bubble.height = Some(height);
                }
                return self.relayout(false);
            }
            Message::Frame(_) => {
                if let Some(task) = self.sweep() {
                    return task;
                }
                if !self.animating() {
                    return self.relayout(true);
                }
            }
            Message::Hover(id, held) => self.core.hold(id, held),
            Message::Click(id) => self.core.click(id),
            Message::Action(id, key) => self.core.invoke(id, &key),
            Message::Theme(theme) => color::set_active(theme),
            _ => {}
        }
        Task::none()
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
                note,
                glyph,
                height: None,
                top: Animation::new(SPRING, 0.0, 0.0, now),
                slide: Animation::new(SPRING, f64::from(OUT), f64::from(OUT), now),
                leaving: false,
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
        let heights: Vec<f32> = staying
            .iter()
            .filter_map(|&i| self.bubbles[i].height)
            .collect();
        for (&i, top) in staying.iter().zip(layout::stack(&heights)) {
            let animation = &mut self.bubbles[i].top;
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
            let id = b.note.id;
            let dx = at(&b.slide, now);
            let slid = float(bubble(b)).translate(move |_, _| Vector::new(dx, 0.0));
            // Keyed by the id: a bubble that takes the place of another is measured anew.
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
    let id = note.id;
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
    let header = row![
        glyph,
        text(app_name(note))
            .size(ui::HEADER_SIZE)
            .style(ui::secondary_text)
    ]
    .spacing(ui::HEADER_GAP)
    .align_y(Alignment::Center);
    let mut content = column![header].spacing(ui::ROW_GAP);
    if !note.summary.is_empty() {
        content = content.push(text(&note.summary).font(typography::UI_SEMIBOLD));
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
        content = content.push(rich_text(spans));
    }
    let buttons: Vec<Element<'_, Message>> = note
        .buttons()
        .enumerate()
        .map(|(i, (key, label))| {
            button(container(text(label.as_str())).center_y(Length::Fill))
                .height(ui::BUTTON_HEIGHT)
                .padding([0.0, ui::BUTTON_PADDING])
                .style(ui::action(i == 0))
                .on_press(Message::Action(id, key.clone()))
                .into()
        })
        .collect();
    if !buttons.is_empty() {
        let actions = row(buttons).spacing(ui::BUTTON_GAP);
        content = content.push(container(actions).padding(Padding::ZERO.top(ui::BUTTONS_TOP)));
    }
    let card = container(content)
        .padding(ui::PADDING)
        .width(ui::WIDTH)
        .style(ui::bubble(note.urgency == Urgency::Critical));
    mouse_area(card)
        .on_press(Message::Click(id))
        .on_enter(Message::Hover(id, true))
        .on_exit(Message::Hover(id, false))
        .interaction(mouse::Interaction::Pointer)
        .into()
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
