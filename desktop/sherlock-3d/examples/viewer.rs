//! A small window that goes through Sherlock's moods, one every three seconds, in his bubble.
//! The model comes from `just mascot`; without it, the still picture shows.
//!
//! ```sh
//! cargo run -p sherlock-3d --example viewer
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::{center, container};
use iced::{Element, Subscription};
use sherlock_3d::{Model, Mood, Sherlock3d, locate};
use sherlock_ui::color::Palette;

/// The moods, in the order they are shown.
const MOODS: [Mood; 8] = [
    Mood::Idle,
    Mood::Wave,
    Mood::EyesClosed,
    Mood::Peek,
    Mood::Sorry,
    Mood::Success,
    Mood::Sleep,
    Mood::Think,
];
/// How long each mood lasts.
const STEP: Duration = Duration::from_secs(3);

struct Viewer {
    model: Option<Arc<Model>>,
    index: usize,
    since: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    Frame(Instant),
}

fn main() -> iced::Result {
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    iced::application(Viewer::new, Viewer::update, Viewer::view)
        .subscription(Viewer::subscription)
        .run()
}

impl Viewer {
    fn new() -> Self {
        let model = locate::find().and_then(|path| match Model::open(&path) {
            Ok(model) => Some(Arc::new(model)),
            Err(error) => {
                tracing::warn!(%error, "cannot read the mascot");
                None
            }
        });
        Self {
            model,
            index: 0,
            since: None,
        }
    }

    fn update(&mut self, message: Message) {
        let Message::Frame(now) = message;
        let since = *self.since.get_or_insert(now);
        if now.saturating_duration_since(since) >= STEP {
            self.index = (self.index + 1) % MOODS.len();
            self.since = Some(now);
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let palette = Palette::DARK;
        let mascot = Sherlock3d::new(self.model.clone())
            .mood(MOODS[self.index])
            .background(palette.system_discs[1].into());
        container(center(mascot))
            .style(move |_| {
                container::Style::default().background(iced::Color::from(palette.system_screen))
            })
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::window::frames().map(Message::Frame)
    }
}
