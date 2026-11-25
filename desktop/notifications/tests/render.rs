//! The bubbles drawn without a screen: the stack as it settles, written to
//! `target/tmp/notifications-tiny-skia.png`, and what a click on a bubble and on a button
//! sends.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::{container, stack};
use iced::{Element, Event, Length, Point, window};
use iced_test::simulator::{self, Simulator};
use notifications::app::{Inbox, Message, Notifications};
use notifications::core::{Core, Event as CoreEvent};
use notifications::layout::SURFACE_WIDTH;
use notifications::model::{Notification, Request};
use sherlock_ui::notifications as ui;
use sherlock_ui::theme;
use zbus::zvariant::{OwnedValue, Value};

/// Height of the drawn area: room for the stack.
const HEIGHT: f32 = 420.0;

fn note(id: u32, summary: &str, body: &str, actions: &[&str], urgency: u8) -> Arc<Notification> {
    let urgency = OwnedValue::try_from(Value::U8(urgency)).expect("a byte");
    let hints = [("urgency".to_owned(), urgency)].into();
    Arc::new(Notification::new(
        id,
        Request {
            app_name: String::from(if id == 2 { "Conteneurs" } else { "Sherlock" }),
            summary: summary.to_owned(),
            body: body.to_owned(),
            actions: actions.iter().map(|a| (*a).to_owned()).collect(),
            hints,
            ..Request::default()
        },
    ))
}

fn view_on_desktop(app: &Notifications) -> Element<'_, Message> {
    stack![
        container("")
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::desktop),
        app.view(window::Id::unique())
    ]
    .into()
}

/// Lays the bubbles out the way the daemon does: a redraw reports their heights, then the
/// springs run until they settle.
fn settle(app: &mut Notifications) {
    for _ in 0..3 {
        let mut ui = Simulator::with_size(
            notifications::settings(),
            (SURFACE_WIDTH, HEIGHT),
            app.view(window::Id::unique()),
        );
        let redraw = Event::Window(window::Event::RedrawRequested(Instant::now()));
        let _ = ui.simulate([redraw]);
        let messages: Vec<Message> = ui.into_messages().collect();
        for message in messages {
            let _ = app.update(message);
        }
        std::thread::sleep(Duration::from_millis(400));
        let _ = app.update(Message::Frame(Instant::now()));
    }
}

fn bubbles() -> Notifications {
    let (sink, inbox) = Inbox::new();
    let mut app = Notifications::new(Core::new(sink, None), inbox);
    for note in [
        note(
            1,
            "Build terminé",
            "compositeur a compilé <b>sans erreur</b>.",
            &[],
            1,
        ),
        note(
            2,
            "front-next s'est arrêté",
            "Le conteneur ne répond <i>plus</i>.",
            &[
                "default",
                "Ouvrir",
                "restart",
                "Relancer",
                "logs",
                "Voir les logs",
            ],
            2,
        ),
    ] {
        let _ = app.update(Message::Core(CoreEvent::Show(note)));
    }
    settle(&mut app);
    app
}

#[test]
fn the_stack_renders_newest_on_top() {
    let app = bubbles();
    let mut ui = Simulator::with_size(
        notifications::settings(),
        (SURFACE_WIDTH, HEIGHT),
        view_on_desktop(&app),
    );
    let snapshot = ui.snapshot(&theme::theme()).expect("snapshot");
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("notifications.png");
    let written = path.with_file_name("notifications-tiny-skia.png");
    let _ = std::fs::remove_file(&written);
    snapshot.matches_image(&path).expect("screenshot written");
    assert!(std::fs::metadata(&written).expect("screenshot").len() > 2_000);
}

#[test]
fn a_click_on_a_button_or_on_the_bubble() {
    let app = bubbles();
    let mut ui = Simulator::with_size(
        notifications::settings(),
        (SURFACE_WIDTH, HEIGHT),
        app.view(window::Id::unique()),
    );
    // The newest, on top: its first button, then its body.
    let _ = ui.click("Relancer");
    ui.point_at(Point::new(ui::WIDTH - 20.0, ui::PADDING + 4.0));
    let _ = ui.simulate(simulator::click());
    let messages: Vec<String> = ui.into_messages().map(|m| format!("{m:?}")).collect();
    assert!(
        messages.iter().any(|m| m == "Action(2, \"restart\")"),
        "{messages:?}"
    );
    assert!(messages.iter().any(|m| m == "Click(2)"), "{messages:?}");
}
