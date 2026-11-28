//! The panel on screens of every size (lot P-02e, L-1 and L-2): never under the dock nor out
//! of the screen, its long titles ending with « … ». Screenshots in `target/tmp/`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::container;
use iced::window::Id;
use iced::{Element, Length};
use iced_test::simulator::Simulator;
use launcher::app::{Launcher, Message};
use launcher::source::{Action, Hit, Query, Source, Tile};
use sherlock_ui::launcher::{self as look, Glyph};
use sherlock_ui::theme;

/// A source that finds `count` results with long titles and paths, whatever the query.
struct Long {
    section: &'static str,
    count: usize,
}

impl Source for Long {
    fn section(&self) -> &'static str {
        self.section
    }

    fn search(&self, _: &Query, limit: usize) -> Vec<Hit> {
        (0..self.count.min(limit))
            .map(|n| Hit {
                title: format!(
                    "Compte rendu de la réunion du conseil scientifique numéro {n} avec toutes ses annexes et les tableaux.pdf"
                ),
                subtitle: Some(String::from(
                    "Documents, Projets, Laboratoire, Rapports annuels, Archives, Deux mille vingt-six, Conseil, Annexes",
                )),
                tile: Tile::Glyph(Glyph::File),
                action: Action::Copy(String::from("rapport")),
                app: None,
            })
            .collect()
    }
}

fn launcher(screen: (f32, f32)) -> Launcher {
    let sources: Vec<Box<dyn Source>> = ["APPLICATIONS", "FICHIERS", "SYSTÈME", "SHERLOCK"]
        .into_iter()
        .map(|section| Box::new(Long { section, count: 5 }) as Box<dyn Source>)
        .collect();
    let mut launcher = Launcher::new(sources, Arc::new(|_| Ok(1))).with_screen(screen);
    let _ = launcher.update(Message::Query(String::from("rapport")));
    let _ = launcher.update(Message::Frame(Instant::now() + Duration::from_secs(5)));
    launcher
}

/// Screens in logical pixels: 1366 by 768, 1920 by 1080 at scales 1, 1.25, 1.5 and 2 (4K at 2
/// is 1920 by 1080 too), 4K at 1.5, a 1280 by 800 laptop at 1.5.
const SCREENS: [(f32, f32); 7] = [
    (1366.0, 768.0),
    (1920.0, 1080.0),
    (1536.0, 864.0),
    (1280.0, 720.0),
    (960.0, 540.0),
    (2560.0, 1440.0),
    (853.0, 533.0),
];

#[test]
fn the_panel_stays_on_the_screen_above_the_dock() {
    for screen in SCREENS {
        let launcher = launcher(screen);
        let fit = launcher.fit();
        let rows = launcher.hits().count();
        assert!(rows >= 1 && rows <= fit.results, "{screen:?}: {rows}");
        let bottom = fit.top + launcher.height();
        let dock = look::dock_room(sherlock_ui::metrics::DOCK_ICON);
        assert!(bottom <= screen.1 - dock, "{screen:?}: {bottom}");
        assert!(fit.width <= screen.0 - 2.0 * look::SCREEN_GUTTER);
        // The surface of the panel alone takes that place too.
        let surface = launcher::surface(launcher.height(), fit);
        assert_eq!(surface.margin.top, fit.top.round() as i32);
    }
    // 1920 by 1080: every result, at the board's place.
    let large = launcher((1920.0, 1080.0));
    assert_eq!(large.fit(), look::Fit::default());
    assert_eq!(large.hits().count(), look::MAX_RESULTS);
    // 1366 by 768: fewer results, so that the panel ends above the dock.
    assert!(launcher((1366.0, 768.0)).hits().count() < look::MAX_RESULTS);
}

#[test]
fn a_new_screen_moves_and_resizes_the_panel() {
    let mut launcher = launcher((1920.0, 1080.0));
    let _ = launcher.update(Message::Surface(Id::unique()));
    let before = launcher.hits().count();
    assert!(launcher.update(Message::Screen(Some((1366, 768)))).units() > 0);
    assert!(launcher.hits().count() < before);
    // The same screen again, or none: nothing to do.
    assert_eq!(
        launcher.update(Message::Screen(Some((1366, 768)))).units(),
        0
    );
    assert_eq!(launcher.update(Message::Screen(None)).units(), 0);
}

/// The whole screen, the panel at its place, on the desktop background.
fn capture(screen: (f32, f32), name: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let launcher = launcher(screen);
    let screen_view: Element<'_, Message> = container(launcher.view_screen())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::desktop)
        .into();
    let mut ui = Simulator::with_size(launcher::settings(), screen, screen_view);
    // Long titles end with « … » instead of wrapping: see the screenshots.
    let snapshot = ui.snapshot(&theme::theme())?;
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    let written = path.with_file_name(format!("{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    snapshot.matches_image(&path)?;
    Ok(std::fs::metadata(&written)?.len())
}

#[test]
fn screenshots_at_two_sizes() {
    for (screen, name) in [
        ((1366.0, 768.0), "launcher-screen-1366x768"),
        ((1920.0, 1080.0), "launcher-screen-1920x1080"),
        ((853.0, 533.0), "launcher-screen-853x533"),
    ] {
        assert!(capture(screen, name).expect("screenshot written") > 2_000);
    }
}
