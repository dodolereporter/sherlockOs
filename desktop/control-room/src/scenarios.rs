//! Headless scenarios: each test drives the application in a temporary configuration folder,
//! never the author's, checks the files written, and renders the window without a screen.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use appearance_config::Mode;
use desktop_config::touchpad::Click;
use dock_config::{AutoHide, Position};
use iced::{Pixels, Settings as IcedSettings};
use iced_test::simulator::Simulator;
use sherlock_ui::color::{self, Theme};
use sherlock_ui::control_room::{self as cr, texts as t};
use sherlock_ui::{theme, typography};

use crate::app::{App, Change, Config, Hour, Message};
use crate::search::{Page, Setting};
use crate::settings::{File, Paths, Settings};
use crate::thumbnails;

/// A temporary folder, removed at the end.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("control-room-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temporary folder");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The application on a temporary `XDG_CONFIG_HOME`, with the wallpapers of `wallpapers`.
fn bench(name: &str, wallpapers: Vec<PathBuf>) -> (TempDir, App) {
    let tmp = TempDir::new(name);
    let paths = Paths::under(&tmp.0.join("config"), wallpapers);
    let (app, _) = App::new(Config {
        paths,
        page: Page::Appearance,
    });
    (tmp, app)
}

#[test]
fn the_files_are_found_where_the_programs_read_them() {
    let paths = Paths::under(Path::new("/cfg"), Vec::new());
    assert_eq!(
        paths.appearance,
        Path::new("/cfg/appearance/appearance.conf")
    );
    assert_eq!(paths.dock, Path::new("/cfg/dock/dock.conf"));
    assert_eq!(paths.wallpaper, Path::new("/cfg/sherlockos/wallpaper.conf"));
    assert_eq!(paths.touchpad, Path::new("/cfg/sherlockos/touchpad.conf"));
    assert_eq!(paths.pointer, Path::new("/cfg/sherlockos/pointer.conf"));
}

#[test]
fn every_setting_is_written_and_read_back() {
    let (tmp, mut app) = bench("write", Vec::new());
    // Nothing is written before a change: the defaults stay the programs' own.
    assert!(!tmp.0.join("config").exists());
    for change in [
        Change::Mode(Mode::Auto),
        Change::LightFrom(Hour(6 * 60 + 30)),
        Change::DarkFrom(Hour(20 * 60)),
        Change::Wallpaper("aurora"),
        Change::FollowTheme(false),
        Change::DockSize(0),
        Change::Magnification(false),
        Change::MagnificationFactor(100),
        Change::AutoHide(AutoHide::Always),
        Change::Position(Position::Left),
        Change::Click(Click::Areas),
        Change::TapToClick(false),
        Change::NaturalScroll(false),
        Change::Speed(75),
        Change::Typing(false),
        Change::ShowClicks(true),
    ] {
        let _ = app.update(Message::Set(change));
    }
    // A slider waits for its rest; closing the window writes what waits.
    app.flush();
    let (read, locked) = Settings::load(&app.paths);
    assert!(locked.is_empty());
    let s = &read;
    let written = &app.settings;
    assert_eq!(s.appearance, written.appearance);
    assert_eq!(s.wallpaper, written.wallpaper);
    assert_eq!(s.touchpad, written.touchpad);
    assert_eq!(s.pointer, written.pointer);
    assert_eq!(s.appearance.mode, Mode::Auto);
    assert_eq!(s.appearance.light_from.minutes(), 6 * 60 + 30);
    assert_eq!(s.wallpaper.wallpaper.as_deref(), Some("aurora"));
    assert!(!s.wallpaper.follow_theme);
    assert_eq!(s.dock.icon_size, sherlock_ui::dock::ICON_SIZE_MIN);
    let factor = s.dock.magnification_factor;
    assert_eq!(factor, sherlock_ui::dock::MAGNIFICATION_MAX);
    let dock = (s.dock.auto_hide, s.dock.position);
    assert_eq!(dock, (AutoHide::Always, Position::Left));
    assert_eq!(s.touchpad.click, Click::Areas);
    assert_eq!(s.touchpad.speed, 0.5);
    assert!(!s.touchpad.tap_to_click && !s.touchpad.natural_scroll);
    assert!(!s.touchpad.disable_while_typing && s.pointer.show_clicks);
    // In the words each program reads.
    let text = |path: &Path| fs::read_to_string(path).expect("written");
    assert!(text(&app.paths.touchpad).contains("click = areas"));
    assert!(text(&app.paths.appearance).contains("AutoLightFrom=06:30"));
    assert!(text(&app.paths.dock).contains("Position=left"));
    assert!(text(&app.paths.pointer).contains("show-clicks = true"));
    assert!(text(&app.paths.wallpaper).contains("wallpaper = aurora"));
}

#[test]
fn a_file_keeps_what_control_room_does_not_show() {
    let tmp = TempDir::new("keep");
    let paths = Paths::under(&tmp.0, Vec::new());
    fs::create_dir_all(paths.dock.parent().expect("folder")).expect("folder");
    fs::write(&paths.dock, "[Dock]\nPinned=evidence;\nIconTheme=breeze\n").expect("dock");
    let (mut app, _) = App::new(Config {
        paths: paths.clone(),
        page: Page::Appearance,
    });
    let _ = app.update(Message::Set(Change::Position(Position::Right)));
    let text = fs::read_to_string(&paths.dock).expect("dock");
    assert!(text.contains("Pinned=evidence;") && text.contains("IconTheme=breeze"));
    assert!(text.contains("Position=right"));
}

/// The application on files already there, in a temporary `XDG_CONFIG_HOME`.
fn bench_with(name: &str, files: &[(&str, &str)]) -> (TempDir, App) {
    let tmp = TempDir::new(name);
    let paths = Paths::under(&tmp.0.join("config"), Vec::new());
    for (relative, text) in files {
        let path = tmp.0.join("config").join(relative);
        fs::create_dir_all(path.parent().expect("folder")).expect("folder");
        fs::write(path, text).expect("settings file");
    }
    let (app, _) = App::new(Config {
        paths,
        page: Page::Appearance,
    });
    (tmp, app)
}

#[test]
fn an_unknown_key_and_the_comments_stay() {
    let touchpad = "# my touchpad\nspeed = 0.4\nfuture-key = 3\nclick = middle\n";
    let appearance = "# mine\n[Appearance]\nTheme=dark\nFuture=x\n\n[Other]\nA=1\n";
    let (_tmp, mut app) = bench_with(
        "unknown",
        &[
            ("sherlockos/touchpad.conf", touchpad),
            ("appearance/appearance.conf", appearance),
        ],
    );
    // Read as far as it goes: the speed counts, the rest is left aside, nothing is locked.
    assert_eq!(app.settings.touchpad.speed, 0.4);
    assert!(app.locked.is_empty());
    let _ = app.update(Message::Set(Change::TapToClick(false)));
    let _ = app.update(Message::Set(Change::Mode(Mode::Light)));
    let text = |path: &Path| fs::read_to_string(path).expect("file");
    assert_eq!(
        text(&app.paths.touchpad),
        "# my touchpad\nspeed = 0.4\nfuture-key = 3\nclick = middle\ntap-to-click = false\n"
    );
    assert_eq!(
        text(&app.paths.appearance),
        "# mine\n[Appearance]\nTheme=light\nFuture=x\n\n[Other]\nA=1\n"
    );
}

#[test]
fn a_file_that_cannot_be_read_is_never_written() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new("unreadable");
    let paths = Paths::under(&tmp.0.join("config"), Vec::new());
    fs::create_dir_all(paths.touchpad.parent().expect("folder")).expect("folder");
    fs::write(&paths.touchpad, "speed = 0.4\n").expect("file");
    fs::set_permissions(&paths.touchpad, fs::Permissions::from_mode(0o000)).expect("chmod");
    // Not text: a NUL byte.
    fs::write(&paths.pointer, "show-clicks = true\0\n").expect("file");
    let (mut app, _) = App::new(Config {
        paths: paths.clone(),
        page: Page::Touchpad,
    });
    if fs::read(&paths.touchpad).is_ok() {
        // Run as root: permissions do not stop it, nothing to check for that file.
        return;
    }
    assert!(app.locked.contains(&File::Touchpad) && app.locked.contains(&File::Pointer));
    let _ = app.update(Message::Set(Change::TapToClick(false)));
    let _ = app.update(Message::Set(Change::ShowClicks(false)));
    app.flush();
    fs::set_permissions(&paths.touchpad, fs::Permissions::from_mode(0o644)).expect("chmod");
    assert_eq!(
        fs::read_to_string(&paths.touchpad).expect("file"),
        "speed = 0.4\n"
    );
    assert_eq!(
        fs::read(&paths.pointer).expect("file"),
        b"show-clicks = true\0\n"
    );
    // On screen: the setting says why, and its control does nothing.
    assert!(app.message(Change::TapToClick(false)).is_none());
    let mut ui = simulator(&app, cr::WINDOW);
    assert!(ui.find(t::LOCKED_FILE).is_ok());
}

#[test]
fn permissions_and_a_dangling_link_are_kept() {
    use std::os::unix::fs::PermissionsExt;
    let (tmp, mut app) = bench_with("modes", &[("sherlockos/touchpad.conf", "speed = 0.4\n")]);
    fs::set_permissions(&app.paths.touchpad, fs::Permissions::from_mode(0o600)).expect("chmod");
    // The wallpaper file is a link into dotfiles not written yet.
    let dotfiles = tmp.0.join("dotfiles/wallpaper.conf");
    std::os::unix::fs::symlink(&dotfiles, &app.paths.wallpaper).expect("link");
    let _ = app.update(Message::Set(Change::TapToClick(false)));
    let _ = app.update(Message::Set(Change::Wallpaper("ring")));
    let mode = fs::metadata(&app.paths.touchpad)
        .expect("file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let link = fs::symlink_metadata(&app.paths.wallpaper).expect("link");
    assert!(link.file_type().is_symlink(), "the link stays a link");
    assert!(
        fs::read_to_string(dotfiles)
            .expect("target")
            .contains("wallpaper = ring")
    );
}

#[test]
fn an_application_pinned_in_the_dock_meanwhile_stays_pinned() {
    let (_tmp, mut app) = bench_with("dock", &[("dock/dock.conf", "[Dock]\nPinned=evidence;\n")]);
    // While Control Room is open, the dock pins an application.
    let pinned = "[Dock]\n# pinned in the dock\nPinned=evidence;firefox;\n";
    fs::write(&app.paths.dock, pinned).expect("dock pins");
    let _ = app.update(Message::Set(Change::DockSize(100)));
    app.flush();
    let text = fs::read_to_string(&app.paths.dock).expect("dock");
    assert!(
        text.contains("# pinned in the dock\nPinned=evidence;firefox;"),
        "{text}"
    );
    assert!(text.contains("IconSize=96"), "{text}");
    // And Control Room now knows it.
    let pins = app.settings.dock.pinned.clone().expect("pinned");
    assert_eq!(pins, ["evidence", "firefox"]);
}

#[test]
fn a_slider_writes_once_it_rests() {
    let (_tmp, mut app) = bench("slider", Vec::new());
    let _ = app.update(Message::Set(Change::Speed(10)));
    let _ = app.update(Message::Set(Change::Speed(20)));
    assert!(!app.paths.touchpad.exists(), "not while it moves");
    std::thread::sleep(cr::SAVE_DELAY);
    let _ = app.update(Message::Frame(std::time::Instant::now()));
    assert_eq!(Settings::load(&app.paths).0.touchpad.speed, -0.6);
}

#[test]
fn the_hours_of_the_two_themes_never_meet() {
    let (_tmp, mut app) = bench("hours", Vec::new());
    let dark = app.settings.appearance.dark_from.minutes();
    let _ = app.update(Message::Set(Change::LightFrom(Hour(dark))));
    let appearance = app.settings.appearance;
    assert_ne!(appearance.light_from.minutes(), dark);
    let offered = crate::app::hours(appearance.light_from, appearance.dark_from);
    assert!(!offered.contains(&Hour(dark)));
    assert_eq!(offered.len(), 24 * 60 / cr::HOUR_STEP as usize - 1);
}

#[test]
fn the_search_opens_the_page_of_the_setting_and_lights_it() {
    let (_tmp, mut app) = bench("search", Vec::new());
    let _ = app.update(Message::Search("défil".into()));
    assert_eq!(app.hits[0].setting, Setting::NaturalScroll);
    let _ = app.update(Message::OpenFirst);
    assert_eq!(app.page, Page::Touchpad);
    assert!(matches!(app.spotlight, Some((Setting::NaturalScroll, _))));
    // Escape empties the search, and the pages come back.
    let _ = app.update(Message::Escape);
    assert!(app.query.is_empty() && app.hits.is_empty());
}

#[test]
fn the_sidebar_goes_from_page_to_page() {
    let (_tmp, mut app) = bench("navigation", Vec::new());
    let messages: Vec<Message> = {
        let mut ui = simulator(&app, cr::WINDOW);
        ui.click(t::TOUCHPAD)
            .expect("the touchpad page is in the sidebar");
        ui.into_messages().collect()
    };
    assert!(matches!(messages.as_slice(), [Message::Go(Page::Touchpad)]));
    for message in messages {
        let _ = app.update(message);
    }
    assert_eq!(app.page, Page::Touchpad);
    assert!(app.page_slide.running(app.now()), "the page comes in");
    // In a narrow window the sidebar is folded: the button of the title bar brings it over
    // the page, and choosing a page folds it again.
    app.settle();
    let messages: Vec<Message> = {
        let mut ui = simulator(&app, SIZES[1].0);
        assert!(ui.find(t::APPEARANCE).is_err(), "folded");
        // The icon button, first in the title bar: its label is a tooltip, not a text.
        let (left, _) = sherlock_ui::metrics::TITLE_BAR_PADDING;
        let middle = sherlock_ui::metrics::ICON_BUTTON / 2.0;
        let height = sherlock_ui::metrics::TITLE_BAR_HEIGHT;
        ui.point_at(iced::Point::new(left + middle, height / 2.0));
        let _ = ui.simulate(iced_test::simulator::click());
        ui.into_messages().collect()
    };
    assert!(matches!(messages.as_slice(), [Message::ToggleSidebar]));
    let _ = app.update(Message::ToggleSidebar);
    assert!(app.sidebar.shown());
    let _ = app.update(Message::Go(Page::Appearance));
    assert!(!app.sidebar.shown());
}

/// Thumbnails for the captures: the originals of `assets/wallpapers`, reduced once into a
/// temporary folder of variants.
fn wallpapers() -> &'static (TempDir, Vec<PathBuf>) {
    static WALLPAPERS: OnceLock<(TempDir, Vec<PathBuf>)> = OnceLock::new();
    WALLPAPERS.get_or_init(|| {
        let tmp = TempDir::new("wallpapers");
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wallpapers");
        let (width, height) = thumbnails::size();
        for w in sherlock_ui::wallpapers::ALL {
            let original = image::open(assets.join(format!("{}.jpg", w.id))).expect("original");
            let folder = tmp.0.join(w.id);
            fs::create_dir_all(&folder).expect("folder");
            let variant = folder.join(format!("{width}x{height}.png"));
            original
                .thumbnail_exact(width, height)
                .save(variant)
                .expect("variant");
        }
        let dirs = vec![tmp.0.clone()];
        (tmp, dirs)
    })
}

/// A test that sets the active theme holds it: captures run one at a time.
fn in_theme(which: Theme) -> MutexGuard<'static, ()> {
    static THEME: Mutex<()> = Mutex::new(());
    let guard = THEME.lock().unwrap_or_else(PoisonError::into_inner);
    color::set_active(which);
    guard
}

fn simulator(app: &App, (width, height): (f32, f32)) -> Simulator<'_, Message> {
    let settings = IcedSettings {
        fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: Pixels(typography::UI_SIZE),
        ..IcedSettings::default()
    };
    Simulator::with_size(settings, (width, height), app.view())
}

/// Renders the window and writes `target/tmp/control-room-<name>-<renderer>.png`.
fn capture(app: &App, name: &str, size: (f32, f32)) -> PathBuf {
    let mut ui = simulator(app, size);
    let snapshot = ui.snapshot(&theme::window_theme()).expect("capture");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    fs::create_dir_all(&dir).expect("captures folder");
    let renderer = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
    let name = format!("control-room-{name}");
    let written = dir.join(format!("{name}-{renderer}.png"));
    let _ = fs::remove_file(&written);
    let expected = dir.join(format!("{name}.png"));
    assert!(snapshot.matches_image(expected).expect("writing"));
    written
}

/// The two sizes of every capture: a window maximized on a 1920 by 1080 screen, under the
/// bar, and a narrow one, where the sidebar folds.
const SIZES: [((f32, f32), &str); 2] = [((1920.0, 1032.0), "wide"), ((520.0, 760.0), "narrow")];

#[test]
fn captures_of_the_pages_in_both_themes() {
    let (_, dirs) = wallpapers();
    let found = Arc::new(thumbnails::load(dirs));
    assert_eq!(found.len(), sherlock_ui::wallpapers::ALL.len());
    for (which, theme_name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
        let _theme = in_theme(which);
        for (page, page_name) in [
            (Page::Appearance, "appearance"),
            (Page::Touchpad, "touchpad"),
        ] {
            let (_tmp, mut app) = bench("capture", dirs.clone());
            let _ = app.update(Message::Thumbnails(found.clone()));
            let _ = app.update(Message::Go(page));
            let _ = app.update(Message::Set(Change::Mode(Mode::Auto)));
            app.settle();
            for (size, size_name) in SIZES {
                let path = capture(&app, &format!("{page_name}-{size_name}-{theme_name}"), size);
                assert!(path.exists(), "{}", path.display());
            }
        }
    }
}

#[test]
fn captures_of_the_search_and_the_folded_sidebar() {
    let _theme = in_theme(Theme::Dark);
    let (_tmp, mut app) = bench("capture-search", Vec::new());
    let _ = app.update(Message::Search("clic".into()));
    app.settle();
    capture(&app, "search", cr::WINDOW);
    let _ = app.update(Message::ToggleSidebar);
    app.settle();
    capture(&app, "sidebar-narrow", SIZES[1].0);
}
