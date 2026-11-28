//! The bubbles drawn without a screen: the stack as it settles, written to
//! `target/tmp/notifications-tiny-skia.png`, and what a click on a bubble and on a button
//! sends.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
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

    fn note(
        id: u32,
        summary: &str,
        body: &str,
        actions: &[&str],
        urgency: u8,
    ) -> Arc<Notification> {
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

    fn capture(app: &Notifications, name: &str) {
        let mut ui = Simulator::with_size(
            notifications::settings(),
            (SURFACE_WIDTH, HEIGHT),
            view_on_desktop(app),
        );
        let snapshot = ui.snapshot(&theme::theme()).expect("snapshot");
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        let written = path.with_file_name(format!("{name}-tiny-skia.png"));
        let _ = std::fs::remove_file(&written);
        snapshot.matches_image(&path).expect("screenshot written");
        assert!(std::fs::metadata(&written).expect("screenshot").len() > 2_000);
    }

    /// A bubble that asks too much (D-23c): an application name of 48 characters and a title
    /// on one line each with « … », an address without a space broken inside the bubble, five
    /// actions: two buttons and « Options » sharing the row, the others in its list.
    /// `notifications-crowded`, then `notifications-options` with the list open.
    #[test]
    fn a_crowded_bubble_stays_inside_its_card() {
        let (sink, inbox) = Inbox::new();
        let mut app = Notifications::new(Core::new(sink, None), inbox);
        let urgency = OwnedValue::try_from(Value::U8(1)).expect("a byte");
        let crowded = Arc::new(Notification::new(
            7,
            Request {
                app_name: "Gestionnaire de téléchargements très très long".to_owned(),
                summary: "Le téléchargement de l'image disque est terminé depuis un moment"
                    .to_owned(),
                body: "https://exemple.org/telechargements/sherlockos/images/nocturne/sherlockos-amd64-20261127.iso"
                    .to_owned(),
                actions: [
                    "open", "Ouvrir", "folder", "Afficher dans le dossier", "copy",
                    "Copier le lien", "verify", "Vérifier", "delete", "Supprimer",
                ]
                .map(String::from)
                .to_vec(),
                hints: [("urgency".to_owned(), urgency)].into(),
                ..Request::default()
            },
        ));
        let _ = app.update(Message::Core(CoreEvent::Show(crowded)));
        settle(&mut app);
        capture(&app, "notifications-crowded");
        let mut ui = Simulator::with_size(
            notifications::settings(),
            (SURFACE_WIDTH, HEIGHT),
            app.view(window::Id::unique()),
        );
        assert!(
            ui.find("Supprimer").is_err(),
            "beyond the row, in « Options »"
        );
        let _ = ui.click(ui::text::OPTIONS);
        let messages: Vec<Message> = ui.into_messages().collect();
        assert!(
            messages.iter().any(|m| matches!(m, Message::Options(1))),
            "{messages:?}"
        );
        for message in messages {
            let _ = app.update(message);
        }
        settle(&mut app);
        capture(&app, "notifications-options");
        let mut ui = Simulator::with_size(
            notifications::settings(),
            (SURFACE_WIDTH, HEIGHT),
            app.view(window::Id::unique()),
        );
        assert!(ui.find("Supprimer").is_ok());
    }

    fn from(id: u32, app: &str, summary: &str) -> Arc<Notification> {
        Arc::new(Notification::new(
            id,
            Request {
                app_name: app.to_owned(),
                summary: summary.to_owned(),
                body: String::from("Un message de démonstration."),
                ..Request::default()
            },
        ))
    }

    fn messages_of(
        app: &Notifications,
        act: impl FnOnce(&mut Simulator<'_, Message>),
    ) -> Vec<Message> {
        let mut ui = Simulator::with_size(
            notifications::settings(),
            (SURFACE_WIDTH, HEIGHT),
            app.view(window::Id::unique()),
        );
        act(&mut ui);
        ui.into_messages().collect()
    }

    /// D-23d: three bubbles of « Courrier » pile up behind the newest, « Discussion » below;
    /// « 2 autres notifications » opens the pile. `notifications-pile`, `notifications-pile-open`.
    #[test]
    fn the_bubbles_of_one_application_pile_up_and_open() {
        let (sink, inbox) = Inbox::new();
        let core = Core::new(sink, None);
        let mut app = Notifications::new(core, inbox);
        for note in [
            from(1, "Courrier", "Réunion à 15 h"),
            from(2, "Discussion", "Camille : on y va ?"),
            from(3, "Courrier", "Facture de novembre"),
            from(4, "Courrier", "Votre colis arrive"),
        ] {
            let _ = app.update(Message::Core(CoreEvent::Show(note)));
        }
        settle(&mut app);
        capture(&app, "notifications-pile");
        let more = ui::text::more(2);
        let messages = messages_of(&app, |ui| {
            let _ = ui.click(more.as_str());
        });
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::Expand(a, true) if a == "Courrier")),
            "{messages:?}"
        );
        for message in messages {
            let _ = app.update(message);
        }
        settle(&mut app);
        capture(&app, "notifications-pile-open");
        let messages = messages_of(&app, |ui| {
            assert!(ui.find("Réunion à 15 h").is_ok());
            let _ = ui.click(ui::text::SHOW_LESS);
        });
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::Expand(_, false)))
        );
    }

    /// D-23d: under the pointer a bubble shows « × », which dismisses it; swiped to the right
    /// past a third of its width it goes, a shorter swipe springs back. D-23f: the « × » is the
    /// macOS badge, half outside the top left corner, faded in. `notifications-hover`.
    #[test]
    fn a_bubble_goes_with_its_close_button_or_a_swipe() {
        let (sink, inbox) = Inbox::new();
        let core = Core::new(sink, None);
        let mut app = Notifications::new(core.clone(), inbox);
        for (app_name, summary) in [("Courrier", "Un"), ("Discussion", "Deux")] {
            let id = core.notify(Request {
                app_name: app_name.to_owned(),
                summary: summary.to_owned(),
                ..Request::default()
            });
            let note = core.open(id).expect("open");
            let _ = app.update(Message::Core(CoreEvent::Show(note)));
        }
        settle(&mut app);
        // Serial 2, the newest, is on top.
        let _ = app.update(Message::Hover(2, true));
        settle(&mut app);
        capture(&app, "notifications-hover");
        // A taller bubble, with its actions: the badge on its corner too.
        let mut tall = bubbles();
        let _ = tall.update(Message::Hover(2, true));
        settle(&mut tall);
        capture(&tall, "notifications-hover-actions");
        // « × » on the top left corner of the bubble on top, its outer half included.
        let messages = messages_of(&app, |ui| {
            ui.point_at(Point::new(6.0, 6.0));
            let _ = ui.simulate(simulator::click());
        });
        assert!(
            messages.iter().any(|m| matches!(m, Message::Dismiss(2))),
            "{messages:?}"
        );
        // A short swipe springs back, a long one dismisses.
        let _ = app.update(Message::Swipe(1, 30.0));
        std::thread::sleep(ui::SWIPE_END + Duration::from_millis(50));
        let _ = app.update(Message::Frame(Instant::now()));
        let _ = app.update(Message::Swipe(1, ui::WIDTH * 0.6));
        std::thread::sleep(ui::SWIPE_END + Duration::from_millis(50));
        let _ = app.update(Message::Frame(Instant::now()));
        assert!(core.open(1).is_none(), "swiped away");
        assert!(core.open(2).is_some());
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
}
