//! Headless scenarios: each test drives the application in a temporary configuration folder,
//! never the author's, checks the files written, and renders the window without a screen.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use appearance_config::Mode;
use desktop_config::touchpad::{Click, SwipeDown};
use dock_config::{AutoHide, Position};
use iced::{Pixels, Settings as IcedSettings};
use iced_test::simulator::Simulator;
use sherlock_ui::color::{self, Theme};
use sherlock_ui::control_room::{self as cr, texts as t};
use sherlock_ui::{theme, typography};

use crate::app::{App, Change, Config, Delay, Hour, LayoutChoice, Message, Sound};
use crate::apps::{self, Kind};
use crate::search::{Page, Setting};
use crate::settings::{File, Paths, Settings};
use crate::sound;
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
    let mut paths = Paths::under(&tmp.0.join("config"), wallpapers);
    paths.apps = apps_dirs(&tmp.0);
    // Never the real one: a fake `pactl` (`fake_pactl`), or none at all.
    paths.pactl = tmp.0.join("no-pactl").into_os_string();
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
    assert_eq!(paths.keyboard, Path::new("/cfg/sherlockos/keyboard.conf"));
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
        Change::TapAndDrag(false),
        Change::DragLock(true),
        Change::SwipeDown(SwipeDown::Minimize),
        Change::Layout(LayoutChoice::Known(2)),
        Change::RepeatDelay(100),
        Change::RepeatRate(0),
    ] {
        let _ = app.update(Message::Set(change));
    }
    // A slider waits for its rest; closing the window writes what waits.
    let _ = app.update(Message::Set(Change::LockAfter(Delay(15))));
    let _ = app.flush();
    app.finish_writes();
    let (read, locked) = Settings::load(&app.paths);
    assert!(locked.is_empty());
    let s = &read;
    let written = &app.settings;
    assert_eq!(s.appearance, written.appearance);
    assert_eq!(s.wallpaper, written.wallpaper);
    assert_eq!(s.touchpad, written.touchpad);
    assert_eq!(s.pointer, written.pointer);
    assert_eq!(s.keyboard, written.keyboard);
    assert_eq!(s.keyboard.layout.as_deref(), Some("fr"));
    assert_eq!(s.keyboard.variant, "bepo");
    assert_eq!((s.keyboard.repeat_delay, s.keyboard.repeat_rate), (1000, 2));
    assert!(!s.touchpad.tap_and_drag && s.touchpad.drag_lock);
    assert_eq!(s.touchpad.swipe_down, SwipeDown::Minimize);
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
    assert!(text(&app.paths.touchpad).contains("swipe-down = minimize"));
    assert!(text(&app.paths.keyboard).contains("variant = bepo"));
    assert!(text(&app.paths.wallpaper).contains("wallpaper = aurora"));
    assert_eq!(text(&app.paths.lock), "idle-minutes = 15\n");
    assert_eq!(read.lock.idle_minutes, 15);
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
    app.finish_writes();
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
    app.finish_writes();
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
    let _ = app.flush();
    app.finish_writes();
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
    app.finish_writes();
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
    let _ = app.flush();
    app.finish_writes();
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
    app.finish_writes();
    assert_eq!(Settings::load(&app.paths).0.touchpad.speed, -0.6);
}

#[test]
fn the_keyboard_page_survives_a_restart_and_keeps_an_unknown_layout() {
    let (_tmp, mut app) = bench_with(
        "keyboard",
        &[("sherlockos/keyboard.conf", "# mine\nlayout = sv\n")],
    );
    // A layout the menu does not offer is shown as such, and kept.
    assert_eq!(
        LayoutChoice::of(&app.settings.keyboard),
        LayoutChoice::Other
    );
    let _ = app.update(Message::Set(Change::RepeatRate(50)));
    let _ = app.update(Message::Set(Change::RepeatDelay(0)));
    assert_eq!(
        fs::read_to_string(&app.paths.keyboard).expect("file"),
        "# mine\nlayout = sv\n",
        "a slider writes once it rests"
    );
    let _ = app.flush();
    app.finish_writes();
    // Opened again, as after a restart: everything is there.
    let (again, _) = App::new(Config {
        paths: app.paths.clone(),
        page: Page::Keyboard,
    });
    let kb = &again.settings.keyboard;
    assert_eq!(kb.layout.as_deref(), Some("sv"));
    assert_eq!((kb.repeat_delay, kb.repeat_rate), (150, 26));
    // The system's layout empties the key: the compositor goes back to the system's.
    let _ = app.update(Message::Set(Change::Layout(LayoutChoice::System)));
    app.finish_writes();
    let text = fs::read_to_string(&app.paths.keyboard).expect("file");
    assert!(text.starts_with("# mine\nlayout = \n"), "{text}");
    assert_eq!(Settings::load(&app.paths).0.keyboard.layout, None);
    // Every layout offered has its name, and the field to try keeps a bounded text.
    assert_eq!(
        LayoutChoice::offered().len(),
        cr::KEYBOARD_LAYOUTS.len() + 1
    );
    let _ = app.update(Message::TryKeys("a".repeat(cr::TRIAL_MAX + 50)));
    assert_eq!(app.trial.chars().count(), cr::TRIAL_MAX);
}

#[test]
fn the_right_click_offers_two_fingers_first_and_the_corner() {
    let (_tmp, mut app) = bench("right-click", Vec::new());
    assert_eq!(app.settings.touchpad.click, Click::Fingers);
    let _ = app.update(Message::Go(Page::Touchpad));
    app.settle();
    let messages: Vec<Message> = {
        let mut ui = simulator(&app, cr::WINDOW);
        assert!(ui.find(t::TWO_FINGERS).is_ok());
        ui.click(t::CORNER).expect("the corner is offered");
        ui.into_messages().collect()
    };
    assert!(matches!(
        messages.as_slice(),
        [Message::Set(Change::Click(Click::Areas))]
    ));
    // Drag lock waits for tap and drag, which waits for tap to click.
    let _ = app.update(Message::Set(Change::TapToClick(false)));
    let mut ui = simulator(&app, cr::WINDOW);
    assert!(ui.find(t::DRAG_LOCK).is_ok() && ui.find(t::SWIPE_MINIMIZE).is_ok());
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

/// The Bluetooth page (lot D-26o) with what BlueZ would give: a headset connected at 80 %, a
/// mouse paired, a speaker around while the scan runs; then the question of a pairing. Both
/// sizes, both themes: `bluetooth-<size>-<theme>`, `bluetooth-pairing-wide-dark`.
#[test]
fn captures_of_the_bluetooth_page() {
    use crate::bluetooth::Msg;
    use system_services::bluetooth::{Asked, Bluetooth, Device, Kind, Reply};
    let device = |path: &str, name: &str, kind, paired, connected, battery| Device {
        path: path.to_owned(),
        name: name.to_owned(),
        kind,
        paired,
        connected,
        battery,
    };
    let read = Bluetooth {
        adapter: String::from("/org/bluez/hci0"),
        powered: true,
        connected: 1,
        devices: vec![
            device(
                "/dev/1",
                "Casque WH-1000XM4",
                Kind::Audio,
                true,
                true,
                Some(80),
            ),
            device(
                "/dev/2",
                "Souris MX Master 3 de Camille, au bureau",
                Kind::Mouse,
                true,
                false,
                None,
            ),
            device(
                "/dev/3",
                "Enceinte du salon",
                Kind::Audio,
                false,
                false,
                None,
            ),
        ],
    };
    for (which, theme_name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
        let _theme = in_theme(which);
        let (_tmp, mut app) = bench("bluetooth", Vec::new());
        let _ = app.update(Message::Go(Page::Bluetooth));
        let _ = app.update(Message::Bluetooth(Msg::Read(Ok(read.clone()))));
        app.settle();
        for &(size, size_name) in &SIZES {
            let path = capture(&app, &format!("bluetooth-{size_name}-{theme_name}"), size);
            assert!(path.exists(), "{}", path.display());
        }
        if which == Theme::Dark {
            let messages: Vec<Message> = {
                let mut ui = simulator(&app, SIZES[0].0);
                ui.click(t::CONNECT).expect("the mouse connects");
                ui.click(t::PAIR).expect("the speaker pairs");
                ui.into_messages().collect()
            };
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        Message::Bluetooth(Msg::Device(mouse)),
                        Message::Bluetooth(Msg::Device(speaker)),
                    ] if mouse == "/dev/2" && speaker == "/dev/3"
                ),
                "{messages:?}"
            );
            // A pairing asks to confirm a code.
            app.bluetooth.busy = Some(String::from("/dev/3"));
            let (sender, _answer) = iced::futures::channel::oneshot::channel();
            let asked = Asked::Confirm(String::from("123456"));
            app.bluetooth.asked = Some((asked, Reply::new(sender)));
            let path = capture(&app, "bluetooth-pairing-wide-dark", SIZES[0].0);
            assert!(path.exists());
        }
        // Leaving the page stops the scan and what was under way.
        let _ = app.update(Message::Go(Page::Sound));
        assert!(app.bluetooth.asked.is_none() && app.bluetooth.busy.is_none());
    }
}

#[test]
fn captures_of_the_pages_in_both_themes() {
    let (_, dirs) = wallpapers();
    let found = Arc::new(thumbnails::load(dirs));
    assert_eq!(found.installed.len(), sherlock_ui::wallpapers::ALL.len());
    assert_eq!(found.thumbnails.len(), sherlock_ui::wallpapers::ALL.len());
    for (which, theme_name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
        let _theme = in_theme(which);
        for (page, page_name) in [
            (Page::Sound, "sound"),
            (Page::Appearance, "appearance"),
            (Page::Keyboard, "keyboard"),
            (Page::Touchpad, "touchpad"),
            (Page::Lock, "lock"),
            (Page::Clipboard, "clipboard"),
            (Page::DefaultApps, "default-apps"),
        ] {
            let (tmp, mut app) = bench("capture", dirs.clone());
            let _ = app.update(Message::Thumbnails(found.clone()));
            let pactl = fake_pactl(&tmp.0);
            let _ = app.update(Message::Audio(sound::read(&pactl)));
            install_apps(&tmp.0);
            let _ = app.update(Message::Catalog(catalog(&app)));
            let _ = app.update(Message::Go(page));
            let _ = app.update(Message::Set(Change::Mode(Mode::Auto)));
            app.settle();
            // Sound and lock, short pages, in the wide window only: the captures take time.
            let sizes = match page {
                Page::Sound | Page::Lock | Page::Clipboard => &SIZES[..1],
                _ => &SIZES[..],
            };
            for &(size, size_name) in sizes {
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

/// Fake XDG folders of applications under `root`, `mimeapps.list` in its `config`.
fn apps_dirs(root: &Path) -> mime_apps::Dirs {
    mime_apps::Dirs {
        config_home: root.join("config"),
        config_dirs: Vec::new(),
        data_home: root.join("data"),
        data_dirs: Vec::new(),
    }
}

/// A few applications, as a laptop has them.
fn install_apps(root: &Path) {
    let folder = root.join("data/applications");
    fs::create_dir_all(&folder).expect("applications");
    for (id, name, types, categories) in [
        (
            "firefox",
            "Firefox",
            "text/html;x-scheme-handler/http;x-scheme-handler/https;",
            "Network;",
        ),
        (
            "app.zen_browser.zen",
            "Zen Browser",
            "text/html;x-scheme-handler/http;x-scheme-handler/https;",
            "Network;",
        ),
        (
            "thunderbird",
            "Thunderbird",
            "x-scheme-handler/mailto;",
            "Network;",
        ),
        ("org.kde.kate", "Kate", "text/plain;", "Utility;TextEditor;"),
        (
            "code",
            "Visual Studio Code",
            "text/plain;inode/directory;",
            "Development;",
        ),
        ("org.kde.konsole", "Konsole", "", "System;TerminalEmulator;"),
        ("foot", "Foot", "", "System;TerminalEmulator;"),
        (
            "evidence",
            "Evidence",
            "inode/directory;",
            "System;FileManager;",
        ),
        (
            "org.kde.gwenview",
            "Gwenview",
            "image/png;image/jpeg;",
            "Graphics;",
        ),
        (
            "mpv",
            "mpv",
            "video/mp4;video/webm;audio/mpeg;",
            "AudioVideo;",
        ),
        (
            "org.kde.elisa",
            "Elisa",
            "audio/mpeg;audio/flac;",
            "AudioVideo;",
        ),
    ] {
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={id}\nMimeType={types}\nCategories={categories}\n"
        );
        fs::write(folder.join(format!("{id}.desktop")), entry).expect("entry");
    }
}

fn catalog(app: &App) -> Arc<mime_apps::Catalog> {
    let dirs = app.paths.apps.clone();
    Arc::new(mime_apps::Catalog::load_with_terminal(dirs, &[], &[], None))
}

/// A fake `pactl` in `root`: it writes each command it gets to `pactl.log` and answers as
/// PipeWire's server does on a laptop with an HDMI output and a headset.
fn fake_pactl(root: &Path) -> system_services::audio::Pactl {
    let script = root.join("pactl");
    let log = root.join("pactl.log");
    let text = format!(
        r#"#!/bin/sh
echo "$@" >> '{log}'
case "$1 $2" in
  "list sinks") printf 'Sink #1\n\tName: speakers\n\tDescription: Haut-parleurs intégrés\nSink #2\n\tName: hdmi\n\tDescription: Sortie HDMI\n' ;;
  "list sources") printf 'Source #3\n\tName: speakers.monitor\n\tMonitor of Sink: speakers\nSource #4\n\tName: mic\n\tDescription: Microphone intégré\n\tMonitor of Sink: n/a\nSource #5\n\tName: headset\n\tDescription: Casque\n\tMonitor of Sink: n/a\n' ;;
esac
case "$1" in
  get-default-sink) echo speakers ;;
  get-default-source) echo mic ;;
  get-sink-volume) echo 'Volume: front-left: 40632 /  62% / -12.46 dB' ;;
  get-source-volume) echo 'Volume: front-left: 29491 /  45% / -20.81 dB' ;;
  get-sink-mute|get-source-mute) echo 'Mute: no' ;;
esac
"#,
        log = log.display()
    );
    fs::write(&script, text).expect("fake pactl");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("executable");
    system_services::audio::Pactl {
        program: script.into_os_string(),
    }
}

#[test]
fn the_sound_page_reads_and_sets_through_pactl() {
    let (tmp, mut app) = bench("sound", Vec::new());
    let pactl = fake_pactl(&tmp.0);
    let audio = sound::read(&pactl).expect("devices");
    let names: Vec<&str> = audio
        .output
        .devices
        .iter()
        .map(|d| d.description.as_str())
        .collect();
    assert_eq!(names, ["Haut-parleurs intégrés", "Sortie HDMI"]);
    assert_eq!(audio.input.devices.len(), 2, "the monitor is no microphone");
    assert_eq!(
        (audio.output.default.as_str(), audio.output.volume),
        ("speakers", 62)
    );
    assert_eq!(
        (audio.input.default.as_str(), audio.input.volume),
        ("mic", 45)
    );
    let _ = app.update(Message::Audio(Ok(audio)));
    // Chosen in the page: shown at once, then asked of pactl.
    use system_services::audio::Device;
    let choose = sound::Change::Default(Device::Output, "hdmi".into());
    let _ = app.update(Message::SetSound(choose.clone()));
    let Sound::Ready(shown) = &app.sound else {
        panic!("devices shown");
    };
    assert_eq!(shown.output.default, "hdmi");
    sound::apply(&pactl, &choose).expect("set");
    sound::apply(&pactl, &sound::Change::Volume(Device::Microphone, 30)).expect("volume");
    let log = fs::read_to_string(tmp.0.join("pactl.log")).expect("log");
    assert!(log.contains("set-default-sink hdmi\n"), "{log}");
    assert!(
        log.contains("set-source-volume @DEFAULT_SOURCE@ 30%\n"),
        "{log}"
    );
    // No pactl at all: the page says so.
    let missing = system_services::audio::Pactl {
        program: tmp.0.join("no-pactl").into_os_string(),
    };
    let _ = app.update(Message::Audio(sound::read(&missing)));
    assert_eq!(app.sound, Sound::Unavailable);
}

#[test]
fn default_applications_are_written_to_mimeapps_list() {
    let (tmp, mut app) = bench("apps", Vec::new());
    install_apps(&tmp.0);
    let _ = app.update(Message::Catalog(catalog(&app)));
    let catalog = catalog(&app);
    let (offered, _) = apps::choices(&catalog, Kind::Browser);
    let names: Vec<&str> = offered.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Firefox", "Zen Browser"]);
    let zen = offered[1].clone();
    let dirs = &app.paths.apps;
    apps::set(dirs, &[], Kind::Browser, &zen.id).expect("written");
    apps::set(dirs, &[], Kind::Terminal, "foot.desktop").expect("written");
    let list = fs::read_to_string(dirs.config_home.join("mimeapps.list")).expect("list");
    for mime in Kind::Browser.types() {
        assert!(
            list.contains(&format!("{mime}=app.zen_browser.zen.desktop;")),
            "{list}"
        );
    }
    // Read back as every program reads it.
    let catalog = mime_apps::Catalog::load_with_terminal(dirs.clone(), &[], &[], None);
    assert_eq!(apps::choices(&catalog, Kind::Browser).1, Some(zen));
    let terminal = apps::choices(&catalog, Kind::Terminal).1.map(|c| c.name);
    assert_eq!(terminal.as_deref(), Some("Foot"));
    // Nothing else of the author's: everything stays in the temporary folder.
    assert!(dirs.config_home.starts_with(&tmp.0));
}

#[test]
fn pactl_gets_the_changes_one_at_a_time_in_order() {
    use system_services::audio::Device;
    let (tmp, mut app) = bench("sound-order", Vec::new());
    let pactl = fake_pactl(&tmp.0);
    let _ = app.update(Message::Audio(sound::read(&pactl)));
    // Two steps of a slider, then a device chosen: the device goes at once.
    let volume = |v| Message::SetSound(sound::Change::Volume(Device::Output, v));
    let _ = app.update(volume(10));
    let _ = app.update(volume(20));
    let choose = sound::Change::Default(Device::Output, "hdmi".into());
    let _ = app.update(Message::SetSound(choose));
    assert!(app.sound_busy && app.sound_queue.is_empty());
    // The slider rests: its last value waits for pactl to be free.
    let _ = app.flush();
    assert_eq!(app.sound_queue, [sound::Change::Volume(Device::Output, 20)]);
    let _ = app.update(volume(30));
    let _ = app.flush();
    assert_eq!(app.sound_queue, [sound::Change::Volume(Device::Output, 30)]);
    // pactl answers: the next one goes.
    let _ = app.update(Message::SoundSet(sound::read(&pactl)));
    assert!(app.sound_busy && app.sound_queue.is_empty());
    let _ = app.update(Message::SoundSet(sound::read(&pactl)));
    assert!(!app.sound_busy);
}

#[test]
fn settings_are_written_off_the_interface_thread_in_their_order() {
    let (_tmp, mut app) = bench("off-thread", Vec::new());
    let _ = app.update(Message::Set(Change::TapToClick(false)));
    // `update` hands the batch to a task: it writes nothing itself.
    assert!(!app.paths.touchpad.exists());
    assert!(!app.settings.touchpad.tap_to_click, "shown at once");
    // Changed again while the first batch is written: they wait, and go after it.
    let _ = app.update(Message::Set(Change::TapToClick(true)));
    let _ = app.update(Message::Set(Change::NaturalScroll(false)));
    assert!(app.settings.touchpad.tap_to_click);
    app.finish_writes();
    let read = Settings::load(&app.paths).0.touchpad;
    assert!(read.tap_to_click && !read.natural_scroll);
    assert_eq!(app.settings.touchpad, read);
}

#[test]
fn a_default_application_not_written_says_so() {
    let (tmp, mut app) = bench("apps-failed", Vec::new());
    install_apps(&tmp.0);
    let _ = app.update(Message::Catalog(catalog(&app)));
    let _ = app.update(Message::Go(Page::DefaultApps));
    app.settle();
    let _ = app.update(Message::AppWritten(Kind::Browser, false));
    let mut ui = simulator(&app, cr::WINDOW);
    assert!(ui.find(t::LOCKED_FILE).is_ok());
    drop(ui);
    // Written the next time: the sentence goes.
    let _ = app.update(Message::AppWritten(Kind::Browser, true));
    let mut ui = simulator(&app, cr::WINDOW);
    assert!(ui.find(t::LOCKED_FILE).is_err());
}

#[test]
fn the_page_left_fades_out_under_the_new_one() {
    let (_tmp, mut app) = bench("leaving", Vec::new());
    let _ = app.update(Message::Go(Page::Lock));
    assert!(matches!(app.leaving, Some((Page::Appearance, _))));
    std::thread::sleep(cr::PAGE_FADE);
    let _ = app.update(Message::Frame(std::time::Instant::now()));
    assert!(app.leaving.is_none());
}

#[test]
fn a_file_changed_elsewhere_shows_unless_a_change_of_ours_waits() {
    let (_tmp, mut app) = bench_with("follow", &[("sherlockos/touchpad.conf", "speed = 0.4\n")]);
    // Changed by hand while Control Room is open.
    fs::write(&app.paths.touchpad, "speed = 0.4\ntap-to-click = false\n").expect("file");
    let _ = app.update(Message::FilesChanged);
    let _ = app.update(Message::Reloaded(Arc::new(Settings::load(&app.paths))));
    assert!(!app.settings.touchpad.tap_to_click);
    // A change of ours waits: what was read before it does not replace it.
    let stale = Arc::new(Settings::load(&app.paths));
    let _ = app.update(Message::Set(Change::NaturalScroll(false)));
    let _ = app.update(Message::Reloaded(stale));
    assert!(!app.settings.touchpad.natural_scroll);
    app.finish_writes();
    assert!(!Settings::load(&app.paths).0.touchpad.natural_scroll);
}
