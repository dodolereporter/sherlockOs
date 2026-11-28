//! Headless end-to-end scenarios: each test drives the application with the keyboard on a
//! temporary folder, runs the tasks it starts as iced would, and checks the disk. The trash
//! lives in the temporary folder, never in the author's own.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::{Event, Pixels, Settings, Task};
use iced_runtime::Action;
use iced_test::simulator;
use sherlock_ui::color::{self, Palette, Theme};
use sherlock_ui::{metrics, theme, typography};

use crate::app::{App, Config, Location, Message};
use crate::places;
use crate::testing::TempDir;
use crate::trash::Trash;

/// A temporary home folder: `projet/` with `src/`, `a.txt` and `.cache`.
struct Bench {
    tmp: TempDir,
    app: App,
}

impl Bench {
    fn new(name: &str) -> Self {
        let tmp = TempDir::new(name);
        let project = tmp.path().join("projet");
        fs::create_dir_all(project.join("src")).expect("folders");
        fs::write(project.join("a.txt"), "contenu de a").expect("write");
        fs::write(project.join(".cache"), "").expect("write");
        let config = Config {
            home: tmp.path().to_path_buf(),
            start: project,
            favorites: places::favorites(tmp.path(), None),
            trash: Some(Trash::at(&tmp.path().join("donnees/Trash"))),
            measure: None,
            maximized: false,
            devices: None,
            watch_devices: false,
            xdg: None,
            desktops: Vec::new(),
            locales: Vec::new(),
        };
        let (mut app, task) = App::new(config);
        drain(&mut app, task);
        Self { tmp, app }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.tmp.path().join(relative)
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            location,
            text,
            repeat,
            ..
        }) = simulator::press_key(key, None)
        else {
            unreachable!("press_key returns a key press");
        };
        let event = keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            location,
            modifiers,
            text,
            repeat,
        };
        self.send(Message::Key(event));
    }

    fn press(&mut self, key: Named) {
        self.key(Key::Named(key), Modifiers::empty());
    }

    fn ctrl(&mut self, letter: &str) {
        self.key(Key::Character(letter.into()), Modifiers::CTRL);
    }

    fn send(&mut self, message: Message) {
        let task = self.app.update(message);
        drain(&mut self.app, task);
    }

    fn names(&self) -> Vec<&str> {
        self.app.entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn selected(&self) -> Vec<&str> {
        self.app
            .selection
            .iter()
            .map(|i| self.app.entries[i].name.as_str())
            .collect()
    }

    /// Clicks the named item, the pointer kept still.
    fn click(&mut self, name: &str) {
        let index = self.names().iter().position(|n| *n == name);
        let index = index.unwrap_or_else(|| panic!("{name} not found"));
        self.send(Message::Click(index));
    }

    /// Puts the cursor on the named item, with the keyboard.
    fn select(&mut self, name: &str) {
        self.press(Named::Home);
        while self.selected() != [name] {
            let before = self.app.selection.cursor();
            self.press(Named::ArrowDown);
            assert_ne!(before, self.app.selection.cursor(), "{name} not found");
        }
    }
}

/// Runs the tasks as iced would, until none is left. Widget operations (focus, scrolling),
/// the clipboard and the window have no screen here.
fn drain(app: &mut App, task: Task<Message>) {
    let mut queue = vec![task];
    while let Some(task) = queue.pop() {
        let Some(stream) = iced_runtime::task::into_stream(task) else {
            continue;
        };
        let actions: Vec<_> = futures::executor::block_on(stream.collect());
        for action in actions {
            if let Action::Output(message) = action {
                queue.push(app.update(message));
            }
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read")
}

/// Two quick clicks open an item, selected or not; the first one selects it (lot Q-13).
#[test]
fn double_click_opens_an_item_selected_or_not() {
    let mut bench = Bench::new("double-click");
    let project = Location::Folder(bench.path("projet"));
    let src = Location::Folder(bench.path("projet/src"));
    assert!(bench.selected().is_empty());
    bench.click("src");
    assert_eq!(bench.selected(), ["src"]);
    assert_eq!(bench.app.location, project);
    bench.click("src");
    assert_eq!(bench.app.location, src);

    // Already selected.
    bench.press(Named::Backspace);
    assert_eq!(bench.selected(), ["src"]);
    bench.click("src");
    bench.click("src");
    assert_eq!(bench.app.location, src);
}

/// Slow clicks, or quick clicks on two items, open nothing.
#[test]
fn slow_clicks_or_two_items_open_nothing() {
    let mut bench = Bench::new("two-clicks");
    let project = Location::Folder(bench.path("projet"));
    bench.click("a.txt");
    bench.click("src");
    assert_eq!(bench.selected(), ["src"]);
    assert_eq!(bench.app.location, project);
    std::thread::sleep(sherlock_ui::decorations::DOUBLE_CLICK + Duration::from_millis(50));
    bench.click("src");
    assert_eq!(bench.app.location, project);
}

#[test]
fn keyboard_navigates_folders() {
    let mut bench = Bench::new("navigation");
    assert_eq!(bench.names(), ["src", "a.txt"]);
    bench.press(Named::ArrowDown);
    assert_eq!(bench.selected(), ["src"]);
    bench.press(Named::Enter);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
    // Backspace goes up and keeps the folder we came from selected.
    bench.press(Named::Backspace);
    assert_eq!(bench.app.location, Location::Folder(bench.path("projet")));
    assert_eq!(bench.selected(), ["src"]);
    // Alt+Left goes back in history, Alt+Right goes forward again.
    bench.key(Key::Named(Named::ArrowLeft), Modifiers::ALT);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
    bench.key(Key::Named(Named::ArrowRight), Modifiers::ALT);
    assert_eq!(bench.app.location, Location::Folder(bench.path("projet")));
    // Ctrl+H shows hidden files, then hides them.
    bench.ctrl("h");
    assert_eq!(bench.names(), ["src", ".cache", "a.txt"]);
    bench.ctrl("h");
    assert_eq!(bench.names(), ["src", "a.txt"]);
    // Search filters, Escape clears it.
    bench.send(Message::Search("A.T".into()));
    assert_eq!(bench.names(), ["a.txt"]);
    bench.press(Named::Escape);
    assert_eq!(bench.names(), ["src", "a.txt"]);
}

#[test]
fn copy_paste_then_cut_paste_with_the_keyboard() {
    let mut bench = Bench::new("clipboard");
    bench.select("a.txt");
    bench.ctrl("c");
    bench.select("src");
    bench.press(Named::Enter);
    bench.ctrl("v");
    assert_eq!(read(&bench.path("projet/src/a.txt")), "contenu de a");
    assert_eq!(read(&bench.path("projet/a.txt")), "contenu de a");
    // The pasted copy is selected.
    assert_eq!(bench.selected(), ["a.txt"]);
    // Pasting a second time in the same place does not touch the first file.
    bench.ctrl("v");
    assert_eq!(bench.names(), ["a (copie).txt", "a.txt"]);

    // Cutting into a folder that already has this name: refused, nothing is overwritten.
    bench.select("a.txt");
    fs::write(bench.path("projet/src/a.txt"), "modifié").expect("write");
    bench.ctrl("x");
    bench.press(Named::Backspace);
    bench.ctrl("v");
    let notice = bench.app.notice.clone().expect("error message");
    assert!(notice.error, "{notice:?}");
    assert_eq!(read(&bench.path("projet/src/a.txt")), "modifié");
    assert_eq!(read(&bench.path("projet/a.txt")), "contenu de a");

    // Cutting then pasting elsewhere moves.
    bench.select("src");
    bench.press(Named::Enter);
    bench.select("a (copie).txt");
    bench.ctrl("x");
    bench.press(Named::Backspace);
    bench.ctrl("v");
    assert!(!bench.path("projet/src/a (copie).txt").exists());
    assert!(bench.path("projet/a (copie).txt").exists());
    // A cut item is pasted only once.
    bench.ctrl("v");
    assert_eq!(bench.names(), ["src", "a (copie).txt", "a.txt"]);

    // Ctrl+D duplicates alongside.
    bench.select("a.txt");
    bench.ctrl("d");
    assert!(bench.path("projet/a (copie 2).txt").exists());
}

#[test]
fn rename_and_create_a_folder_with_the_keyboard() {
    let mut bench = Bench::new("rename");
    bench.select("a.txt");
    bench.press(Named::F2);
    assert_eq!(
        bench.app.renaming.as_ref().map(|(_, n)| n.as_str()),
        Some("a.txt")
    );
    bench.send(Message::RenameInput("notes.txt".into()));
    bench.send(Message::RenameSubmit);
    assert!(bench.app.renaming.is_none());
    assert_eq!(read(&bench.path("projet/notes.txt")), "contenu de a");
    assert_eq!(bench.selected(), ["notes.txt"]);

    // A taken name is refused with a message.
    bench.press(Named::F2);
    bench.send(Message::RenameInput("src".into()));
    bench.send(Message::RenameSubmit);
    assert!(bench.app.notice.as_ref().is_some_and(|n| n.error));
    assert!(bench.path("projet/notes.txt").exists());

    // Ctrl+Shift+N creates a folder and immediately offers to name it.
    bench.key(
        Key::Character("N".into()),
        Modifiers::CTRL | Modifiers::SHIFT,
    );
    assert!(bench.path("projet/Nouveau dossier").is_dir());
    assert_eq!(bench.selected(), ["Nouveau dossier"]);
    assert!(bench.app.renaming.is_some());
    bench.press(Named::Escape);
    assert!(bench.app.renaming.is_none());
}

#[test]
fn trash_discards_restores_and_empties_with_confirmation() {
    let mut bench = Bench::new("trash");
    let trash = Trash::at(&bench.path("donnees/Trash"));
    bench.select("a.txt");
    bench.press(Named::Delete);
    assert!(!bench.path("projet/a.txt").exists());
    assert_eq!(trash.list().expect("trash").len(), 1);
    assert_eq!(bench.names(), ["src"]);

    // In the trash, the right-click menu restores, with the keyboard.
    bench.send(Message::Go(Location::Trash));
    assert_eq!(bench.names(), ["a.txt"]);
    bench.select("a.txt");
    bench.key(Key::Named(Named::F10), Modifiers::SHIFT);
    assert!(bench.app.menu.is_some());
    bench.press(Named::ArrowDown);
    bench.press(Named::Enter);
    assert!(bench.app.menu.is_none());
    assert_eq!(read(&bench.path("projet/a.txt")), "contenu de a");
    assert!(bench.names().is_empty());

    // Emptying asks for confirmation: Escape cancels, nothing is deleted.
    bench.send(Message::Go(Location::Folder(bench.path("projet"))));
    bench.select("a.txt");
    bench.press(Named::Delete);
    bench.send(Message::Go(Location::Trash));
    bench.send(Message::Run(crate::app::Command::EmptyTrash));
    assert!(bench.app.confirm_empty);
    bench.press(Named::Escape);
    assert!(!bench.app.confirm_empty);
    assert_eq!(trash.list().expect("trash").len(), 1);
    bench.send(Message::Run(crate::app::Command::EmptyTrash));
    bench.send(Message::ConfirmEmpty);
    assert!(trash.list().expect("trash").is_empty());
    assert!(bench.names().is_empty());
}

#[test]
fn right_click_menu_is_driven_by_the_keyboard() {
    let mut bench = Bench::new("menu");
    bench.select("a.txt");
    bench.key(Key::Named(Named::ContextMenu), Modifiers::empty());
    let menu = bench.app.menu.clone().expect("menu open");
    assert!(menu.on_selection);
    // Open, then Open with: the right arrow opens the submenu.
    bench.press(Named::ArrowDown);
    bench.press(Named::ArrowDown);
    bench.press(Named::ArrowRight);
    assert!(bench.app.menu.as_ref().is_some_and(|m| m.submenu.is_some()));
    bench.press(Named::ArrowLeft);
    assert!(bench.app.menu.as_ref().is_some_and(|m| m.submenu.is_none()));
    bench.press(Named::Escape);
    assert!(bench.app.menu.is_none());
    // The menu stays inside the window.
    bench.send(Message::Cursor(iced::Point::new(10_000.0, 10_000.0)));
    bench.send(Message::RightClick(None));
    let menu = bench.app.menu.clone().expect("menu open");
    assert!(!menu.on_selection);
    assert!(menu.at.x + metrics::CONTEXT_MENU_WIDTH <= bench.app.size.width);
}

/// A folder like the mock-up's: a git repository with a modified file, a new one and an
/// ignored folder.
fn mockup_folder() -> (TempDir, PathBuf) {
    let tmp = TempDir::new("mockup");
    let dir = tmp.path().join("Projets/compositeur");
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args([
                "-c",
                "user.name=Essai",
                "-c",
                "user.email=essai@example.invalid",
            ])
            .args(args)
            .output()
            .expect("git is installed")
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    for sub in ["src", "docs", "essais", "target"] {
        fs::create_dir_all(dir.join(sub)).expect("folder");
    }
    for (name, content) in [
        (".gitignore", "target/\n"),
        ("Cargo.toml", "[package]\nname = \"compositeur\"\n"),
        ("Cargo.lock", "# Cargo.lock\n"),
        ("justfile", "verifier:\n    cargo test\n"),
        ("README.md", "# Compositeur\n"),
        ("rust-toolchain.toml", "[toolchain]\n"),
        ("src/main.rs", "fn main() {}\n"),
        ("docs/index.md", "# Docs\n"),
        ("essais/a.rs", "\n"),
    ] {
        fs::write(dir.join(name), content).expect("write");
    }
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "initial"]);
    fs::write(dir.join("src/main.rs"), "fn main() { todo!() }\n").expect("write");
    fs::write(dir.join("target/sortie"), "").expect("write");
    let docker = "FROM rust:1 AS build\nWORKDIR /src\nCOPY . .\nRUN cargo build --release\n";
    fs::write(dir.join("Dockerfile"), docker).expect("write");
    (tmp, dir)
}

/// Capture of the window, git status read, Dockerfile selected, menu and submenu open, in
/// `target/tmp/evidence-tiny-skia.png`. Compare it by eye with the mock-up: see
/// docs/evidence.md.
#[test]
fn window_renders_headless() {
    let _theme = crate::testing::in_theme(Theme::Dark);
    let (tmp, dir) = mockup_folder();
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: dir.clone(),
        favorites: places::favorites(tmp.path(), None),
        trash: Some(Trash::at(&tmp.path().join("donnees/Trash"))),
        measure: None,
        maximized: false,
        devices: None,
        watch_devices: false,
        xdg: None,
        desktops: Vec::new(),
        locales: Vec::new(),
    };
    let (mut app, task) = App::new(config);
    drain(&mut app, task);
    let docker = app
        .entries
        .iter()
        .position(|e| e.name == "Dockerfile")
        .expect("Dockerfile listed");
    let (width, height) = metrics::REFERENCE_WINDOW;
    for message in [
        Message::Click(docker),
        Message::Cursor(iced::Point::new(450.0, 406.0)),
        Message::RightClick(Some(docker)),
        Message::Submenu(Some(crate::app::Sub::OpenWith)),
    ] {
        let task = app.update(message);
        drain(&mut app, task);
    }
    app.settle();

    let settings = Settings {
        fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: Pixels(typography::UI_SIZE),
        ..Settings::default()
    };
    let mut ui = simulator::Simulator::with_size(settings.clone(), (width, height), app.view());
    let snapshot = ui.snapshot(&theme::window_theme()).expect("capture");
    // Like `CARGO_TARGET_TMPDIR`, which Cargo only gives to integration tests.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    fs::create_dir_all(&dir).expect("captures folder");
    let path = dir.join("evidence.png");
    // Named after the renderer: `tiny-skia`, or `wgpu` with `ICED_TEST_BACKEND=wgpu`.
    let renderer = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
    let written = path.with_file_name(format!("evidence-{renderer}.png"));
    let _ = fs::remove_file(&written);
    assert!(snapshot.matches_image(&path).expect("writing the capture"));
    println!("capture: {}", written.display());

    // The open menu and its submenu are drawn whole: every entry row of both shows the menu
    // glass, never the black block a misdrawn shadow left (review 3), and the submenu stands
    // beside the menu without covering it.
    let picture = ::image::open(&written).expect("capture").to_rgb8();
    let scale = picture.width() as f32 / width;
    let menu = app.menu.clone().expect("menu open");
    let main_items = app.menu_items(menu.on_selection);
    let sub_items = app.submenu_items(crate::app::Sub::OpenWith);
    let check = |left: f32, top: f32, items: &[crate::app::MenuItem], what: &str| {
        let mut y = top + metrics::MENU_PADDING + metrics::BORDER_WIDTH;
        for item in items {
            let row = match item {
                crate::app::MenuItem::Separator => {
                    2.0 * metrics::MENU_SEPARATOR_MARGIN.0 + metrics::BORDER_WIDTH
                }
                _ => metrics::MENU_ITEM_HEIGHT,
            };
            if !matches!(item, crate::app::MenuItem::Separator) {
                // Near the right edge of the row, where no text is drawn.
                let x = ((left + 8.0) * scale) as u32;
                let pixel = picture.get_pixel(x, ((y + row / 2.0) * scale) as u32);
                assert!(
                    pixel[2] > 40,
                    "{what}: row at {y} is too dark, {pixel:?}: the menu is not drawn whole"
                );
            }
            y += row;
        }
    };
    check(menu.at.x, menu.at.y, &main_items, "menu");
    let sub_at = crate::menu::submenu_at(&menu, &main_items, &sub_items, app.size);
    assert!(
        sub_at.x >= menu.at.x + metrics::CONTEXT_MENU_WIDTH,
        "submenu overlaps the menu"
    );
    check(sub_at.x, sub_at.y, &sub_items, "submenu");

    // The same window in the light theme, `evidence-light-<renderer>.png`, to compare with
    // FichiersClair.
    color::set_active(Theme::Light);
    let mut ui = simulator::Simulator::with_size(settings.clone(), (width, height), app.view());
    let snapshot = ui.snapshot(&theme::window_theme()).expect("capture");
    let light = written.with_file_name(format!("evidence-light-{renderer}.png"));
    let _ = fs::remove_file(&light);
    assert!(
        snapshot
            .matches_image(dir.join("evidence-light.png"))
            .expect("writing")
    );
    let picture = ::image::open(&light).expect("capture").to_rgb8();
    assert!(
        picture.get_pixel(4, picture.height() / 2)[0] > 200,
        "a light window"
    );
    color::set_active(Theme::Dark);

    // A click on a menu entry runs its command.
    let messages: Vec<_> = {
        let mut ui = simulator::Simulator::with_size(settings, (width, height), app.view());
        ui.click("Dupliquer").expect("entry found");
        ui.into_messages().collect()
    };
    assert!(
        matches!(
            messages.as_slice(),
            [Message::Run(crate::app::Command::Duplicate)]
        ),
        "{messages:?}"
    );
}

#[test]
fn a_new_theme_fades_the_window() {
    let _theme = crate::testing::in_theme(Theme::Dark);
    let mut bench = Bench::new("theme");
    let _ = bench.app.update(Message::Theme(Theme::Dark));
    let start = std::time::Instant::now();
    let _ = bench.app.update(Message::Theme(Theme::Light));
    assert!(
        bench.app.animating(bench.app.now()),
        "frames are asked for the fade"
    );
    let fade = sherlock_ui::motion::THEME_FADE;
    let _ = bench.app.update(Message::Frame(start + fade / 3));
    assert_ne!(color::palette(), &Palette::DARK);
    assert_ne!(color::palette(), &Palette::LIGHT);
    let _ = bench.app.update(Message::Frame(start + fade * 2));
    assert_eq!(color::palette(), &Palette::LIGHT);
    assert_eq!(theme::palette(&theme::window_theme()), &Palette::LIGHT);
    color::set_active(Theme::Dark);
}

/// Captures of the title bar, dark and light, in `target/tmp/evidence-header-<theme>.png`:
/// the badge on the left is the icon of Evidence, not a plain tile (lot Q-13).
#[test]
fn header_shows_the_app_icon() {
    for (which, name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
        let _theme = crate::testing::in_theme(which);
        let mut bench = Bench::new("header");
        bench.app.settle();
        let settings = Settings {
            fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
            default_font: typography::UI,
            default_text_size: Pixels(typography::UI_SIZE),
            ..Settings::default()
        };
        let (width, height) = metrics::REFERENCE_WINDOW;
        let mut ui = simulator::Simulator::with_size(settings, (width, height), bench.app.view());
        let snapshot = ui.snapshot(&theme::window_theme()).expect("capture");
        let full = bench.tmp.path().join("window.png");
        let _ = fs::remove_file(&full);
        assert!(snapshot.matches_image(&full).expect("writing the capture"));
        let written = fs::read_dir(bench.tmp.path())
            .expect("captures")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "png"))
            .expect("capture written");
        let picture = ::image::open(&written).expect("capture").to_rgb8();
        let scale = picture.width() as f32 / width;
        let bar = (metrics::TITLE_BAR_HEIGHT * scale) as u32;
        let header = ::image::imageops::crop_imm(&picture, 0, 0, picture.width(), bar).to_image();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
        fs::create_dir_all(&dir).expect("captures folder");
        header
            .save(dir.join(format!("evidence-header-{name}.png")))
            .expect("header capture");

        // The middle of the badge holds the light strokes of the icon on its blue tile.
        let left = metrics::TITLE_BAR_PADDING.0;
        let middle = metrics::TITLE_BAR_HEIGHT / 2.0;
        let quarter = metrics::APP_BADGE / 4.0;
        let (x0, x1) = ((left + quarter) * scale, (left + 3.0 * quarter) * scale);
        let (y0, y1) = ((middle - quarter) * scale, (middle + quarter) * scale);
        let (mut light, mut tile) = (0, 0);
        for y in y0 as u32..y1 as u32 {
            for x in x0 as u32..x1 as u32 {
                let [r, g, b] = picture.get_pixel(x, y).0.map(u16::from);
                light += usize::from(r > 200 && g > 200 && b > 180);
                tile += usize::from(b > r + 40 && b < 160);
            }
        }
        assert!(
            light > 0 && tile > 0,
            "{name}: {light} light, {tile} tile pixels"
        );
    }
}

/// Cost of one frame with 5,000 entries: view, layout, then software rendering. Run with
/// `cargo test --release -p evidence frame_cost -- --ignored --nocapture`.
#[test]
#[ignore = "measurement, not a test"]
fn frame_cost() {
    use std::time::Instant;
    let tmp = TempDir::new("cost");
    let dir = tmp.path().join("grand");
    fs::create_dir_all(&dir).expect("folder");
    for i in 0..5000 {
        fs::write(dir.join(format!("fichier-{i:05}.txt")), "x").expect("write");
    }
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: dir.clone(),
        favorites: places::favorites(tmp.path(), None),
        trash: None,
        measure: None,
        maximized: false,
        devices: None,
        watch_devices: false,
        xdg: None,
        desktops: Vec::new(),
        locales: Vec::new(),
    };
    let (mut app, task) = App::new(config);
    drain(&mut app, task);
    app.settle();
    let settings = Settings {
        fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: Pixels(typography::UI_SIZE),
        ..Settings::default()
    };
    let (width, height) = metrics::REFERENCE_WINDOW;
    {
        let mut warm =
            simulator::Simulator::with_size(settings.clone(), (width, height), app.view());
        let _ = warm.snapshot(&theme::window_theme());
    }
    let mut report = String::new();
    for offset in [0.0, 24.0, 48.0, 4000.0] {
        app.scroll.0 = offset;
        let t0 = Instant::now();
        let view = app.view();
        let t1 = Instant::now();
        let mut ui = simulator::Simulator::with_size(settings.clone(), (width, height), view);
        let t2 = Instant::now();
        let _ = ui.snapshot(&theme::window_theme()).expect("capture");
        let t3 = Instant::now();
        drop(ui);
        let line = format!(
            "offset {offset}: view {:?}, layout {:?}, render {:?}\n",
            t1 - t0,
            t2 - t1,
            t3 - t2
        );
        print!("{line}");
        report.push_str(&line);
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    fs::create_dir_all(&dir).expect("folder");
    fs::write(dir.join("frame_cost.txt"), report).expect("write");
    if false {}
}

#[test]
fn devices_mount_open_unmount_and_eject_through_their_source() {
    use crate::devices::tests::FakeSource;
    use crate::devices::{DeviceKind, Volume};
    use std::sync::{Arc, Mutex};
    let tmp = TempDir::new("devices");
    let volume = |id: &str, name: &str, ejectable| Volume {
        id: id.into(),
        name: name.into(),
        kind: if ejectable {
            DeviceKind::UsbKey
        } else {
            DeviceKind::Internal
        },
        mount_point: None,
        size: 16_000_000_000,
        free: None,
        locked: false,
        ejectable,
    };
    let source = Arc::new(FakeSource {
        volumes: Mutex::new(vec![
            volume("/sda1", "CLE", true),
            volume("/sdb1", "disque occupé", false),
            Volume {
                locked: true,
                ..volume("/sdc1", "Coffre", true)
            },
        ]),
        root: tmp.path().join("media"),
    });
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: tmp.path().to_path_buf(),
        favorites: places::favorites(tmp.path(), None),
        trash: None,
        measure: None,
        maximized: false,
        devices: Some(source.clone()),
        watch_devices: false,
        xdg: None,
        desktops: Vec::new(),
        locales: Vec::new(),
    };
    let (app, task) = App::new(config);
    let mut bench = Bench { tmp, app };
    drain(&mut bench.app, task);
    assert_eq!(bench.app.volumes.len(), 3);

    // A click on an unmounted key mounts it, then opens it.
    bench.send(Message::OpenDevice("/sda1".into()));
    let point = bench.path("media/CLE");
    assert_eq!(bench.app.location, Location::Folder(point.clone()));
    assert!(bench.app.volumes[0].mount_point.is_some());

    // Its menu offers to open, unmount and eject it.
    bench.send(Message::DeviceMenu("/sda1".into()));
    let labels: Vec<_> = bench
        .app
        .menu_items(false)
        .into_iter()
        .filter_map(|item| match item {
            crate::app::MenuItem::Action { label, .. } => Some(label),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["Ouvrir", "Démonter", "Éjecter"]);
    bench.press(Named::Escape);

    // Ejecting leaves the volume first, then it disappears.
    bench.send(Message::EjectDevice("/sda1".into()));
    assert_eq!(bench.app.location, Location::Folder(bench.path("")));
    assert!(bench.app.volumes.iter().all(|v| v.id != "/sda1"));
    assert_eq!(bench.app.notice.as_ref().map(|n| n.error), Some(false));

    // A locked volume and a busy one each give a short message.
    bench.send(Message::OpenDevice("/sdc1".into()));
    assert!(bench.app.notice.as_ref().is_some_and(|n| n.error));
    bench.send(Message::OpenDevice("/sdb1".into()));
    bench.send(Message::DeviceMenu("/sdb1".into()));
    bench.send(Message::Run(crate::app::Command::DeviceUnmount));
    let notice = bench.app.notice.clone().expect("message");
    assert!(notice.error);
    assert_eq!(notice.text, sherlock_ui::texts::evidence::DEVICE_BUSY);
}

#[test]
fn open_with_lists_installed_applications_and_remembers_a_default() {
    let tmp = TempDir::new("open-with");
    let xdg = mime_apps::Dirs {
        config_home: tmp.path().join("xdg/config"),
        config_dirs: Vec::new(),
        data_home: tmp.path().join("xdg/data"),
        data_dirs: vec![tmp.path().join("xdg/share")],
    };
    let write = |relative: &str, content: &str| {
        let path = tmp.path().join("xdg").join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("folder");
        fs::write(path, content).expect("write");
    };
    write(
        "share/mime/globs2",
        "50:text/plain:*.txt\n50:text/markdown:*.md\n",
    );
    // `true` stands in for the editors: opening runs it and nothing happens.
    for (id, name) in [("kate", "Kate"), ("code", "Visual Studio Code")] {
        write(
            &format!("share/applications/{id}.desktop"),
            &format!(
                "[Desktop Entry]\nType=Application\nName={name}\nExec=true %F\nMimeType=text/plain;\n"
            ),
        );
    }
    write(
        "share/applications/viewer.desktop",
        "[Desktop Entry]\nType=Application\nName=Images\nExec=true %U\nMimeType=image/png;\n",
    );
    let project = tmp.path().join("projet");
    fs::create_dir_all(&project).expect("folder");
    fs::write(project.join("notes.txt"), "notes").expect("write");
    fs::write(project.join("readme.md"), "# readme").expect("write");
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: project,
        favorites: places::favorites(tmp.path(), None),
        trash: None,
        measure: None,
        maximized: false,
        devices: None,
        watch_devices: false,
        xdg: Some(xdg.clone()),
        desktops: Vec::new(),
        locales: Vec::new(),
    };
    let (app, task) = App::new(config);
    let mut bench = Bench { tmp, app };
    drain(&mut bench.app, task);
    bench.select("notes.txt");
    bench.key(Key::Named(Named::F10), Modifiers::SHIFT);
    bench.send(Message::Submenu(Some(crate::app::Sub::OpenWith)));
    let names: Vec<&str> = bench
        .app
        .open_with
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(names, ["Visual Studio Code", "Kate"]);

    // The "always" switch keeps the menu open; choosing an application then writes it as the
    // default for the type, and the next open uses it.
    bench.send(Message::Run(crate::app::Command::ToggleAlways));
    assert!(bench.app.always && bench.app.menu.is_some());
    bench.send(Message::OpenWithApp("code.desktop".into()));
    let list = fs::read_to_string(xdg.config_home.join("mimeapps.list")).expect("mimeapps.list");
    assert!(
        list.contains("[Default Applications]\ntext/plain=code.desktop;"),
        "{list}"
    );
    let catalog = bench.app.catalog.clone().expect("catalog");
    assert_eq!(
        catalog.default_for("text/plain").map(|a| a.id.as_str()),
        Some("code.desktop")
    );

    // The switch went back off with that choice: opening another type once writes nothing.
    assert!(!bench.app.always);
    bench.select("readme.md");
    bench.key(Key::Named(Named::F10), Modifiers::SHIFT);
    bench.send(Message::Submenu(Some(crate::app::Sub::OpenWith)));
    assert!(!bench.app.always);
    bench.send(Message::OpenWithApp("kate.desktop".into()));
    let after = fs::read_to_string(xdg.config_home.join("mimeapps.list")).expect("mimeapps.list");
    assert_eq!(
        after, list,
        "a choice without the switch must not touch mimeapps.list"
    );
    // Turning the switch on, then closing the menu, turns it off again.
    bench.key(Key::Named(Named::F10), Modifiers::SHIFT);
    bench.send(Message::Submenu(Some(crate::app::Sub::OpenWith)));
    bench.send(Message::Run(crate::app::Command::ToggleAlways));
    assert!(bench.app.always);
    bench.press(Named::Escape);
    assert!(!bench.app.always);

    // « Autre application… » opens the chooser of every application; Escape closes it.
    bench.select("notes.txt");
    bench.key(Key::Named(Named::F10), Modifiers::SHIFT);
    bench.send(Message::Run(crate::app::Command::ChooseApp));
    assert!(bench.app.chooser.is_some());
    bench.press(Named::Escape);
    assert!(bench.app.chooser.is_none());
}

/// A bench whose `projet` folder also holds `n` text files, `f00.txt` onwards.
fn bench_with_files(name: &str, n: usize) -> Bench {
    let mut bench = Bench::new(name);
    for i in 0..n {
        fs::write(bench.path(&format!("projet/f{i:02}.txt")), "x").expect("write");
    }
    bench.press(Named::F5);
    bench
}

/// Clicks the named item with these modifiers held.
fn click_with(bench: &mut Bench, name: &str, modifiers: Modifiers) {
    bench.send(Message::Modifiers(modifiers));
    bench.click(name);
    bench.send(Message::Modifiers(Modifiers::empty()));
    // Far enough apart in time: never a double click.
    bench.app.clicks = Default::default();
}

#[test]
fn shift_and_ctrl_clicks_select_ranges_from_the_anchor() {
    let mut bench = bench_with_files("ranges", 6);
    click_with(&mut bench, "f01.txt", Modifiers::empty());
    click_with(&mut bench, "f04.txt", Modifiers::SHIFT);
    assert_eq!(
        bench.selected(),
        ["f01.txt", "f02.txt", "f03.txt", "f04.txt"]
    );
    // A second Shift+click starts from the same anchor.
    click_with(&mut bench, "a.txt", Modifiers::SHIFT);
    assert_eq!(bench.selected(), ["a.txt", "f00.txt", "f01.txt"]);
    // Ctrl+click adds an item and moves the anchor; Ctrl+Shift+click adds a range.
    click_with(&mut bench, "f03.txt", Modifiers::CTRL);
    click_with(&mut bench, "f05.txt", Modifiers::CTRL | Modifiers::SHIFT);
    assert_eq!(
        bench.selected(),
        [
            "a.txt", "f00.txt", "f01.txt", "f03.txt", "f04.txt", "f05.txt"
        ]
    );
    // Ctrl+click removes; a plain click selects one item again.
    click_with(&mut bench, "f00.txt", Modifiers::CTRL);
    assert!(!bench.selected().contains(&"f00.txt"));
    click_with(&mut bench, "src", Modifiers::empty());
    assert_eq!(bench.selected(), ["src"]);
    // A double click still opens, after ranges.
    bench.click("src");
    bench.click("src");
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
}

#[test]
fn shift_and_arrows_extend_the_selection_and_escape_clears_it() {
    let mut bench = bench_with_files("arrows", 4);
    bench.select("f00.txt");
    bench.key(Key::Named(Named::ArrowDown), Modifiers::SHIFT);
    bench.key(Key::Named(Named::ArrowDown), Modifiers::SHIFT);
    assert_eq!(bench.selected(), ["f00.txt", "f01.txt", "f02.txt"]);
    bench.key(Key::Named(Named::ArrowUp), Modifiers::SHIFT);
    assert_eq!(bench.selected(), ["f00.txt", "f01.txt"]);
    bench.key(Key::Named(Named::Home), Modifiers::SHIFT);
    assert_eq!(bench.selected(), ["src", "a.txt", "f00.txt"]);
    // The preview shows nothing for several items; Delete trashes them all.
    assert!(bench.app.single().is_none());
    bench.ctrl("a");
    assert_eq!(bench.selected().len(), 6);
    bench.press(Named::Escape);
    assert!(bench.selected().is_empty());
    bench.select("f02.txt");
    bench.key(Key::Named(Named::End), Modifiers::SHIFT);
    bench.press(Named::Delete);
    assert_eq!(bench.names(), ["src", "a.txt", "f00.txt", "f01.txt"]);
    // F2 on several items renames the one under the cursor, alone.
    bench.select("f00.txt");
    bench.key(Key::Named(Named::ArrowDown), Modifiers::SHIFT);
    bench.press(Named::F2);
    assert_eq!(bench.selected(), ["f01.txt"]);
    assert_eq!(
        bench.app.renaming.as_ref().map(|(_, n)| n.as_str()),
        Some("f01.txt")
    );
}

#[test]
fn a_lasso_on_the_background_selects_rows_and_scrolls_at_the_edge() {
    let mut bench = bench_with_files("lasso", 40);
    bench.app.settle();
    let origin = |x: f32, y: f32| {
        iced::Point::new(
            metrics::SIDEBAR_WIDTH + metrics::LIST_PADDING + x,
            metrics::TITLE_BAR_HEIGHT
                + metrics::TOOLBAR_HEIGHT
                + metrics::LIST_HEADER_HEIGHT
                + 3.0 * metrics::BORDER_WIDTH
                + y,
        )
    };
    let row = metrics::LIST_ROW_HEIGHT;
    // Pressed beside row 1, dragged down to row 3: rows 1 to 3.
    bench.send(Message::Cursor(origin(600.0, row * 1.5)));
    bench.send(Message::Blank);
    bench.send(Message::Cursor(origin(601.0, row * 1.5 + 2.0)));
    assert!(bench.selected().is_empty(), "a slip is not a lasso yet");
    bench.send(Message::Cursor(origin(400.0, row * 3.5)));
    assert_eq!(bench.selected(), ["a.txt", "f00.txt", "f01.txt"]);
    // Back up: the selection follows the rectangle.
    bench.send(Message::Cursor(origin(400.0, row * 2.2)));
    assert_eq!(bench.selected(), ["a.txt", "f00.txt"]);
    // Held near the bottom edge, the listing scrolls at each frame, and the lasso goes on
    // with it.
    let bottom = bench.app.size.height - metrics::STATUS_BAR_HEIGHT - 12.0;
    bench.send(Message::Cursor(iced::Point::new(500.0, bottom)));
    assert!(
        bench.app.animating(bench.app.now()),
        "frames for the scroll"
    );
    let before = bench.selected().len();
    bench.send(Message::Scrolled(row * 3.0, bench.app.scroll.1));
    assert!(bench.selected().len() > before);
    bench.send(Message::Released);
    assert!(bench.app.lasso.is_none());
    assert!(
        !bench.app.animating(bench.app.now()),
        "no frame once released"
    );
    // With Ctrl held, a new lasso adds to the selection.
    let kept = bench.selected().len();
    bench.send(Message::Scrolled(0.0, bench.app.scroll.1));
    bench.send(Message::Modifiers(Modifiers::CTRL));
    bench.send(Message::Cursor(origin(600.0, row * 0.5)));
    bench.send(Message::Blank);
    bench.send(Message::Cursor(origin(600.0, row * 0.6 + 10.0)));
    bench.send(Message::Released);
    assert_eq!(bench.selected().len(), kept + 1);
    // Without, a press on the background clears it.
    bench.send(Message::Modifiers(Modifiers::empty()));
    bench.send(Message::Blank);
    bench.send(Message::Released);
    assert!(bench.selected().is_empty());
    // A press beside a name starts one too; on the name, it does not.
    bench.send(Message::Cursor(origin(600.0, row * 1.5)));
    bench.send(Message::Click(1));
    bench.send(Message::Cursor(origin(600.0, row * 2.5)));
    bench.send(Message::Released);
    assert_eq!(bench.selected(), ["a.txt", "f00.txt"]);
    bench.app.clicks = Default::default();
    bench.send(Message::Cursor(origin(40.0, row * 1.5)));
    bench.send(Message::Click(1));
    bench.send(Message::Cursor(origin(40.0, row * 3.5)));
    bench.send(Message::Released);
    assert_eq!(bench.selected(), ["a.txt"]);
}

/// Captures of a selection of two runs, dark and light, in
/// `target/tmp/evidence-selection-<theme>.png`: each run is one block, rounded at its two
/// ends only, with no seam between its rows.
#[test]
fn following_selected_rows_form_one_block() {
    for (which, name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
        let _theme = crate::testing::in_theme(which);
        let mut bench = bench_with_files("runs", 6);
        click_with(&mut bench, "a.txt", Modifiers::empty());
        click_with(&mut bench, "f01.txt", Modifiers::SHIFT);
        click_with(&mut bench, "f03.txt", Modifiers::CTRL);
        click_with(&mut bench, "f04.txt", Modifiers::CTRL);
        assert_eq!(bench.app.selection.runs().len(), 2);
        bench.app.settle();
        let settings = Settings {
            fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
            default_font: typography::UI,
            default_text_size: Pixels(typography::UI_SIZE),
            ..Settings::default()
        };
        let (width, height) = metrics::REFERENCE_WINDOW;
        let mut ui = simulator::Simulator::with_size(settings, (width, height), bench.app.view());
        let snapshot = ui.snapshot(&theme::window_theme()).expect("capture");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
        fs::create_dir_all(&dir).expect("captures folder");
        let base = dir.join(format!("evidence-selection-{name}.png"));
        let renderer = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
        let written = base.with_file_name(format!("evidence-selection-{name}-{renderer}.png"));
        let _ = fs::remove_file(&written);
        assert!(snapshot.matches_image(&base).expect("writing the capture"));
        println!("capture: {}", written.display());

        let picture = ::image::open(&written).expect("capture").to_rgb8();
        let scale = picture.width() as f32 / width;
        let top = metrics::TITLE_BAR_HEIGHT
            + metrics::TOOLBAR_HEIGHT
            + metrics::LIST_HEADER_HEIGHT
            + 3.0 * metrics::BORDER_WIDTH
            + metrics::LIST_TOP_GAP;
        let left = metrics::SIDEBAR_WIDTH + metrics::BORDER_WIDTH + metrics::LIST_PADDING;
        let row = metrics::LIST_ROW_HEIGHT;
        let pixel = |x: f32, y: f32| *picture.get_pixel((x * scale) as u32, (y * scale) as u32);
        // Blank, away from any text: between the names and the dates.
        let x = left + 200.0;
        let background = pixel(x, top + 0.5 * row);
        let selected = pixel(x, top + 1.5 * row);
        assert_ne!(background, selected, "{name}: the selection shows");
        // Rows 1 and 2 (a.txt, f00.txt) are one block: the boundary between them is selected.
        assert_eq!(
            pixel(x, top + 2.0 * row),
            selected,
            "{name}: a seam in the run"
        );
        // The block's corners are rounded at its ends only: the very corner of its first row
        // is background, the corner where two of its rows meet is filled.
        let corner = left + 0.5;
        assert_eq!(
            pixel(corner, top + row + 0.5),
            background,
            "{name}: top corner"
        );
        assert_eq!(
            pixel(corner, top + 2.0 * row),
            selected,
            "{name}: middle corner"
        );
        // Row 4 (f02.txt) is not selected: two blocks, not one.
        assert_eq!(pixel(x, top + 4.5 * row), background, "{name}: rows apart");
        assert_eq!(pixel(x, top + 5.5 * row), selected, "{name}: second block");
    }
}

/// The labels of menu entries, `-` before a greyed one; submenus by their label.
fn menu_labels(items: &[crate::app::MenuItem]) -> Vec<String> {
    use crate::app::{MenuItem, Sub};
    use sherlock_ui::evidence as more;
    use sherlock_ui::texts::evidence as t;
    items
        .iter()
        .filter_map(|item| match item {
            MenuItem::Action { label, .. } => Some((*label).to_owned()),
            MenuItem::Disabled(label) => Some(format!("-{label}")),
            MenuItem::Submenu(Sub::OpenWith) => Some(t::OPEN_WITH.to_owned()),
            MenuItem::Submenu(Sub::SortBy) => Some(more::SORT_BY.to_owned()),
            MenuItem::Submenu(Sub::View) => Some(more::VIEW.to_owned()),
            MenuItem::App(_) | MenuItem::Separator => None,
        })
        .collect()
}

/// The entries of the open menu, or of its open submenu.
fn open_menu(bench: &Bench) -> Vec<String> {
    let menu = bench.app.menu.clone().expect("menu open");
    match menu.submenu {
        Some(sub) => menu_labels(&bench.app.submenu_items(sub)),
        None => menu_labels(&bench.app.menu_items(menu.on_selection)),
    }
}

#[test]
fn the_menu_offers_what_fits_its_target() {
    let mut bench = bench_with_files("menu-target", 2);
    bench.select("a.txt");
    bench.key(Key::Named(Named::ContextMenu), Modifiers::empty());
    let on_item = [
        "Ouvrir",
        "Ouvrir avec",
        "Copier",
        "Couper",
        "-Coller",
        "Renommer",
        "Dupliquer",
        "-Compresser",
        "Copier le chemin",
        "-Propriétés",
        "Mettre à la corbeille",
    ];
    assert_eq!(open_menu(&bench), on_item);
    // The keyboard passes over greyed entries.
    let items = bench.app.menu_items(true);
    for _ in 0..items.len() * 2 {
        bench.press(Named::ArrowDown);
        let lit = bench.app.menu.as_ref().and_then(|m| m.highlight);
        let lit = lit.and_then(|i| items.get(i)).expect("an entry is lit");
        assert!(!matches!(lit, crate::app::MenuItem::Disabled(_)), "{lit:?}");
    }
    bench.press(Named::Escape);
    // Something copied: « Coller » answers. Several items: the same menu.
    bench.ctrl("c");
    bench.ctrl("a");
    bench.send(Message::RightClick(Some(1)));
    assert!(bench.app.menu.as_ref().is_some_and(|m| m.on_selection));
    assert_eq!(
        bench.selected().len(),
        4,
        "a right click inside keeps the selection"
    );
    assert!(open_menu(&bench).contains(&"Coller".to_owned()));
    bench.press(Named::Escape);
    // On the background.
    bench.send(Message::RightClick(None));
    assert_eq!(
        open_menu(&bench),
        [
            "Nouveau dossier",
            "Coller",
            "Fichiers cachés",
            "Trier par",
            "Vue"
        ]
    );
}

#[test]
fn the_background_menu_sorts_and_shows_the_views() {
    use crate::app::{MenuItem, Sub};
    let mut bench = bench_with_files("menu-sort", 2);
    fs::write(bench.path("projet/f00.txt"), "a longer content").expect("write");
    bench.press(Named::F5);
    bench.send(Message::RightClick(None));
    // Down to « Trier par », then Right opens it, its first entry lit.
    while bench.app.menu.as_ref().and_then(|m| m.highlight)
        != bench
            .app
            .menu_items(false)
            .iter()
            .position(|i| *i == MenuItem::Submenu(Sub::SortBy))
    {
        bench.press(Named::ArrowDown);
    }
    bench.press(Named::ArrowRight);
    assert_eq!(
        open_menu(&bench),
        ["Nom", "Modifié", "Taille", "Type", "Git"]
    );
    let checked = |bench: &Bench| {
        bench
            .app
            .submenu_items(Sub::SortBy)
            .iter()
            .position(|i| matches!(i, MenuItem::Action { checked: true, .. }))
    };
    assert_eq!(checked(&bench), Some(0), "by name");
    bench.press(Named::ArrowDown);
    bench.press(Named::ArrowDown);
    bench.press(Named::Enter);
    assert!(bench.app.menu.is_none());
    assert_eq!(bench.app.sort.column, crate::listing::Column::Size);
    assert_eq!(checked(&bench), Some(2));
    // Folders first, then the smallest file first.
    assert_eq!(bench.names(), ["src", "f01.txt", "a.txt", "f00.txt"]);
    // The view: the list, checked; the grid and the columns do not exist yet.
    bench.send(Message::RightClick(None));
    bench.send(Message::Submenu(Some(Sub::View)));
    assert_eq!(open_menu(&bench), ["Liste", "-Grille", "-Colonnes"]);
}

#[test]
fn a_right_click_with_the_menu_open_opens_it_on_the_new_target() {
    let mut bench = Bench::new("menu-reopen");
    bench.select("a.txt");
    bench.key(Key::Named(Named::ContextMenu), Modifiers::empty());
    // Over the first row, src.
    let row = |i: f32| {
        iced::Point::new(
            metrics::SIDEBAR_WIDTH + 100.0,
            metrics::TITLE_BAR_HEIGHT
                + metrics::TOOLBAR_HEIGHT
                + metrics::LIST_HEADER_HEIGHT
                + 3.0 * metrics::BORDER_WIDTH
                + metrics::LIST_TOP_GAP
                + (i + 0.5) * metrics::LIST_ROW_HEIGHT,
        )
    };
    bench.send(Message::Cursor(row(0.0)));
    bench.send(Message::Reopen);
    let menu = bench.app.menu.clone().expect("open again");
    assert!(menu.on_selection);
    assert_eq!(bench.selected(), ["src"]);
    assert!((menu.at.y - row(0.0).y).abs() < 1.0, "where the pointer is");
    // Below the rows: the menu of the background.
    bench.send(Message::Cursor(row(5.0)));
    bench.send(Message::Reopen);
    assert!(bench.app.menu.as_ref().is_some_and(|m| !m.on_selection));
    // Over the sidebar: it only closes.
    bench.send(Message::Cursor(iced::Point::new(40.0, 200.0)));
    bench.send(Message::Reopen);
    assert!(bench.app.menu.is_none());
}

#[test]
fn the_menu_unfolds_and_folds_back() {
    let mut bench = Bench::new("menu-motion");
    bench.app.settle();
    bench.select("a.txt");
    bench.key(Key::Named(Named::ContextMenu), Modifiers::empty());
    let now = bench.app.now();
    assert!(bench.app.animating(now), "frames while it unfolds");
    assert!(bench.app.menu_motion.opacity(now) < 1.0);
    bench.press(Named::Escape);
    assert!(bench.app.menu.is_none());
    assert!(bench.app.closing.is_some(), "drawn while it folds back");
    std::thread::sleep(sherlock_ui::panels::OPEN_FADE + Duration::from_millis(30));
    bench.send(Message::Frame(std::time::Instant::now()));
    assert!(bench.app.closing.is_none());
    let now = bench.app.now();
    assert!(!bench.app.menu_motion.running(now), "no frame at rest");
}

/// Renders the window, writes `target/tmp/<name>-<renderer>.png`, returns the picture and its
/// scale.
fn capture(app: &App, name: &str) -> (::image::RgbImage, f32) {
    capture_at(app, name, metrics::REFERENCE_WINDOW)
}

/// `capture` in a window of `size`.
fn capture_at(app: &App, name: &str, size: (f32, f32)) -> (::image::RgbImage, f32) {
    let settings = Settings {
        fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: Pixels(typography::UI_SIZE),
        ..Settings::default()
    };
    let (width, height) = size;
    let mut ui = simulator::Simulator::with_size(settings, (width, height), app.view());
    let snapshot = ui.snapshot(&theme::window_theme()).expect("capture");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    fs::create_dir_all(&dir).expect("captures folder");
    let renderer = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
    let written = dir.join(format!("{name}-{renderer}.png"));
    let _ = fs::remove_file(&written);
    assert!(
        snapshot
            .matches_image(dir.join(format!("{name}.png")))
            .expect("writing the capture")
    );
    println!("capture: {}", written.display());
    let picture = ::image::open(&written).expect("capture").to_rgb8();
    let scale = picture.width() as f32 / width;
    (picture, scale)
}

/// Captures of the menu on several items and on the background with « Trier par » open, dark
/// and light, in `target/tmp/evidence-menu-*.png`, and of the menu while it unfolds.
#[test]
fn menu_captures() {
    use crate::app::Sub;
    for (which, name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
        let _theme = crate::testing::in_theme(which);
        let mut bench = bench_with_files("menu-capture", 4);
        click_with(&mut bench, "a.txt", Modifiers::empty());
        click_with(&mut bench, "f01.txt", Modifiers::SHIFT);
        bench.ctrl("c");
        bench.send(Message::Cursor(iced::Point::new(520.0, 230.0)));
        bench.send(Message::RightClick(Some(2)));
        bench.app.settle();
        let (picture, scale) = capture(&bench.app, &format!("evidence-menu-selection-{name}"));
        let menu = bench.app.menu.clone().expect("menu open");
        // The menu is drawn where it was opened, on its glass.
        let inside = picture.get_pixel(
            ((menu.at.x + 20.0) * scale) as u32,
            ((menu.at.y + 15.0) * scale) as u32,
        );
        let outside = picture.get_pixel(
            ((menu.at.x - 30.0) * scale) as u32,
            ((menu.at.y + 300.0) * scale) as u32,
        );
        assert_ne!(inside, outside, "{name}: the menu shows");

        bench.press(Named::Escape);
        bench.send(Message::Cursor(iced::Point::new(700.0, 480.0)));
        bench.send(Message::RightClick(None));
        bench.send(Message::Submenu(Some(Sub::SortBy)));
        bench.app.settle();
        capture(&bench.app, &format!("evidence-menu-background-{name}"));
    }
    // Halfway through unfolding, in the software renderer: drawn whole, faded, never the black
    // block a misdrawn shadow once left (see Animations).
    let _theme = crate::testing::in_theme(Theme::Dark);
    let mut bench = bench_with_files("menu-opening", 4);
    bench.send(Message::Cursor(iced::Point::new(520.0, 230.0)));
    bench.send(Message::RightClick(Some(2)));
    bench.app.settle();
    let now = bench.app.now();
    let start = now.saturating_sub(sherlock_ui::panels::OPEN_FADE / 3);
    bench.app.menu_motion = crate::menu::MenuMotion::opening(start);
    let (picture, scale) = capture(&bench.app, "evidence-menu-opening");
    let menu = bench.app.menu.clone().expect("menu open");
    let items = bench.app.menu_items(true);
    let height = App::menu_height(&items) * bench.app.menu_motion.scale(now);
    let mut dark = 0;
    for step in 1..10 {
        let y = menu.at.y + height * step as f32 / 10.0;
        let pixel = picture.get_pixel(((menu.at.x + 8.0) * scale) as u32, (y * scale) as u32);
        dark += usize::from(pixel.0.iter().all(|&c| c < 12));
    }
    assert_eq!(dark, 0, "no black block in the unfolding menu");
}

/// The item a rename or an open acts on is always selected: Ctrl+click can leave the cursor
/// on the item it just removed (review of E-01a).
#[test]
fn rename_and_open_act_on_a_selected_item_only() {
    let mut bench = bench_with_files("focus", 2);
    let index = |bench: &Bench, name: &str| bench.names().iter().position(|n| *n == name);
    click_with(&mut bench, "a.txt", Modifiers::empty());
    click_with(&mut bench, "f00.txt", Modifiers::CTRL);
    click_with(&mut bench, "f00.txt", Modifiers::CTRL);
    assert_eq!(bench.selected(), ["a.txt"]);
    // A right click on the selected item, then « Renommer »: that item, not f00.txt.
    bench.send(Message::RightClick(index(&bench, "a.txt")));
    bench.send(Message::Run(crate::app::Command::Rename));
    let renaming = bench.app.renaming.clone().expect("rename open");
    assert_eq!(renaming.0, bench.path("projet/a.txt"));
    bench.press(Named::Escape);
    // F2 the same.
    bench.press(Named::F2);
    let renaming = bench.app.renaming.clone().expect("rename open");
    assert_eq!(renaming.0, bench.path("projet/a.txt"));
    bench.press(Named::Escape);
    // Several selected: a right click on one of them puts the rename on it.
    click_with(&mut bench, "f01.txt", Modifiers::CTRL);
    bench.send(Message::RightClick(index(&bench, "a.txt")));
    bench.send(Message::Run(crate::app::Command::Rename));
    assert_eq!(bench.selected(), ["a.txt"]);
    bench.press(Named::Escape);
    // Enter opens the selected folder, never the item the cursor was left on.
    click_with(&mut bench, "src", Modifiers::empty());
    click_with(&mut bench, "a.txt", Modifiers::CTRL);
    click_with(&mut bench, "a.txt", Modifiers::CTRL);
    bench.press(Named::Enter);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
}

/// A sort during a rename moves the rows, not the target: the item is kept by its path.
#[test]
fn a_sort_during_a_rename_renames_the_same_item() {
    let mut bench = bench_with_files("rename-sort", 3);
    bench.select("f00.txt");
    bench.press(Named::F2);
    bench.send(Message::Sort(crate::listing::Column::Name));
    assert_eq!(
        bench.names(),
        ["src", "f02.txt", "f01.txt", "f00.txt", "a.txt"]
    );
    bench.send(Message::RenameInput("nouveau.txt".into()));
    bench.send(Message::RenameSubmit);
    assert!(bench.path("projet/nouveau.txt").exists());
    assert!(!bench.path("projet/f00.txt").exists());
    for kept in ["a.txt", "f01.txt", "f02.txt"] {
        assert!(bench.path(&format!("projet/{kept}")).exists(), "{kept}");
    }
}

/// A bus that never answers (a socket that takes connections and says nothing, not even the
/// greeting) delays neither the application nor its first view; its devices are left out
/// after the delay (lot E-01: Evidence once took more than 10 s to open).
#[test]
fn a_silent_bus_never_delays_the_first_view() {
    use std::os::unix::net::UnixListener;
    use std::sync::Arc;
    use std::time::Instant;
    let tmp = TempDir::new("silent-bus");
    let socket = tmp.path().join("bus");
    let _silent = UnixListener::bind(&socket).expect("socket");
    let project = tmp.path().join("projet");
    fs::create_dir_all(&project).expect("folder");
    fs::write(project.join("a.txt"), "a").expect("write");
    let delay = Duration::from_millis(300);
    let udisks = crate::udisks::UDisks::at(&format!("unix:path={}", socket.display()), delay);
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: project,
        favorites: places::favorites(tmp.path(), None),
        trash: None,
        measure: None,
        maximized: false,
        devices: Some(Arc::new(udisks)),
        watch_devices: false,
        xdg: None,
        desktops: Vec::new(),
        locales: Vec::new(),
    };
    let start = Instant::now();
    let (mut app, task) = App::new(config);
    drop(app.view());
    let ready = start.elapsed();
    assert!(
        ready < Duration::from_millis(500),
        "first view after {ready:?}"
    );
    // The devices read gives up after its delay, and the folder is listed meanwhile.
    drain(&mut app, task);
    let done = start.elapsed();
    assert!(
        done < delay + Duration::from_secs(2),
        "tasks done after {done:?}"
    );
    assert!(app.volumes.is_empty());
    assert_eq!(app.entries.len(), 1);
}

/// A bench whose catalog has `names.len()` applications for text files, some with an icon of
/// 16 px, some of 128, some with none, and a `notes.txt` selected.
fn bench_with_apps(name: &str, names: &[String]) -> Bench {
    let tmp = TempDir::new(name);
    let share = tmp.path().join("xdg/share");
    fs::create_dir_all(share.join("mime")).expect("folder");
    fs::create_dir_all(share.join("applications")).expect("folder");
    fs::write(share.join("mime/globs2"), "50:text/plain:*.txt\n").expect("write");
    for (side, file) in [(16, "small.png"), (128, "large.png")] {
        ::image::RgbaImage::from_pixel(side, side, ::image::Rgba([91, 163, 208, 255]))
            .save(tmp.path().join(file))
            .expect("icon");
    }
    for (i, app) in names.iter().enumerate() {
        let icon = match i % 3 {
            0 => format!("Icon={}\n", tmp.path().join("small.png").display()),
            1 => format!("Icon={}\n", tmp.path().join("large.png").display()),
            _ => String::new(),
        };
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName={app}\n{icon}Exec=true %F\nMimeType=text/plain;\n"
        );
        fs::write(share.join(format!("applications/app{i:02}.desktop")), entry).expect("write");
    }
    let project = tmp.path().join("projet");
    fs::create_dir_all(&project).expect("folder");
    fs::write(project.join("notes.txt"), "notes").expect("write");
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: project,
        favorites: places::favorites(tmp.path(), None),
        trash: None,
        measure: None,
        maximized: false,
        devices: None,
        watch_devices: false,
        xdg: Some(mime_apps::Dirs {
            config_home: tmp.path().join("xdg/config"),
            config_dirs: Vec::new(),
            data_home: tmp.path().join("xdg/data"),
            data_dirs: vec![share],
        }),
        desktops: Vec::new(),
        locales: Vec::new(),
    };
    let (app, task) = App::new(config);
    let mut bench = Bench { tmp, app };
    drain(&mut bench.app, task);
    bench.select("notes.txt");
    bench
}

/// The other applications of « Ouvrir avec »: 30 of them, very long names, a small window
/// (review of the author: « toute écrasée dans certains cas »). Rows keep their height and one
/// line each; the list scrolls; the chooser and the submenu stay inside the window.
#[test]
fn many_applications_and_long_names_stay_readable() {
    let _theme = crate::testing::in_theme(Theme::Dark);
    let long = "Un éditeur de texte au nom vraiment très long, qui ne tient pas sur une ligne";
    let mut names: Vec<String> = (1..=27).map(|i| format!("Application {i:02}")).collect();
    names.insert(1, long.to_owned());
    names.insert(5, format!("{long}, deuxième"));
    names.push("Éditeur".to_owned());
    let mut bench = bench_with_apps("many-apps", &names);
    assert_eq!(bench.app.icons.len(), 20, "two in three have an icon");

    // The chooser, in the reference window: the list is as on the design and scrolls.
    bench.send(Message::Run(crate::app::Command::ChooseApp));
    assert_eq!(
        bench.app.chooser_list_height(),
        metrics::CHOOSER_LIST_HEIGHT
    );
    capture(&bench.app, "evidence-chooser-many");

    // The same in the smallest window: a shorter list, still several whole rows.
    let (width, height) = sherlock_ui::evidence::MIN_WINDOW;
    bench.send(Message::Resized(iced::Size::new(width, height)));
    let list = bench.app.chooser_list_height();
    assert!(list < metrics::CHOOSER_LIST_HEIGHT);
    assert!(list >= sherlock_ui::evidence::CHOOSER_MIN_ROWS * metrics::MENU_ITEM_HEIGHT);
    let (picture, scale) = capture_at(&bench.app, "evidence-chooser-small", (width, height));
    // The panel fits: above and below it, its middle column shows the scrimmed window, as
    // its side does.
    let pixel = |x: f32, y: f32| *picture.get_pixel((x * scale) as u32, (y * scale) as u32);
    for y in [4.0, height - 4.0] {
        let (middle, side) = (pixel(width / 2.0, y), pixel(8.0, y));
        let near = middle.0.iter().zip(side.0).all(|(a, b)| a.abs_diff(b) <= 3);
        assert!(near, "the panel stays inside, at {y}: {middle:?}, {side:?}");
    }
    bench.press(Named::Escape);

    // « Ouvrir avec » opened near the bottom right corner of the small window: it lists the
    // applications it holds, on the left of the menu, inside the window.
    bench.send(Message::Cursor(iced::Point::new(
        width - 40.0,
        height - 40.0,
    )));
    let index = bench.names().iter().position(|n| *n == "notes.txt");
    bench.send(Message::RightClick(index));
    bench.send(Message::Submenu(Some(crate::app::Sub::OpenWith)));
    bench.app.settle();
    let menu = bench.app.menu.clone().expect("menu open");
    let items = bench.app.menu_items(true);
    let sub = bench.app.submenu_items(crate::app::Sub::OpenWith);
    let at = crate::menu::submenu_at(&menu, &items, &sub, bench.app.size);
    assert!(at.x + metrics::OPEN_WITH_WIDTH <= width && at.x < menu.at.x);
    assert!(at.y >= 0.0 && at.y + App::menu_height(&sub) <= height);
    assert!(menu.at.y + App::menu_height(&items) <= height);
    capture_at(&bench.app, "evidence-open-with-small", (width, height));

    // Light theme, long names.
    color::set_active(Theme::Light);
    bench.press(Named::Escape);
    bench.send(Message::Resized(iced::Size::new(
        metrics::REFERENCE_WINDOW.0,
        metrics::REFERENCE_WINDOW.1,
    )));
    bench.send(Message::Run(crate::app::Command::ChooseApp));
    bench.send(Message::ChooserSearch("éditeur".into()));
    bench.app.settle();
    capture(&bench.app, "evidence-chooser-long-light");
    color::set_active(Theme::Dark);
}

/// The middle of the row `index` of the listing, in window coordinates.
fn row_point(index: usize) -> (f32, f32) {
    let x = metrics::SIDEBAR_WIDTH + metrics::BORDER_WIDTH + metrics::LIST_PADDING + 40.0;
    let y = metrics::TITLE_BAR_HEIGHT
        + metrics::TOOLBAR_HEIGHT
        + metrics::LIST_HEADER_HEIGHT
        + 3.0 * metrics::BORDER_WIDTH
        + metrics::LIST_TOP_GAP
        + metrics::LIST_ROW_HEIGHT * (index as f32 + 0.5);
    (x, y)
}

#[test]
fn files_dropped_go_into_the_folder_under_the_pointer_or_the_one_shown() {
    use crate::dnd::{Action, Event};
    let mut bench = Bench::new("drop");
    let (one, two) = (bench.path("one.txt"), bench.path("two.txt"));
    fs::write(&one, "1").expect("write");
    fs::write(&two, "2").expect("write");
    assert_eq!(bench.names(), ["src", "a.txt"]);

    // Over the folder `src`: it lights up; dropped with Ctrl, the file is copied into it.
    let (x, y) = row_point(0);
    bench.send(Message::Dnd(Event::Over(x, y)));
    assert_eq!(bench.app.drop_light().map(|l| l.0), Some(Some(0)));
    bench.send(Message::Dnd(Event::Dropped(
        x,
        y,
        Action::Copy,
        vec![one.clone()],
    )));
    assert_eq!(read(&bench.path("projet/src/one.txt")), "1");
    assert!(one.exists());
    assert!(bench.app.drop_light().is_none());

    // Over a file: the folder shown lights up, and takes the drop, moved.
    let (x, y) = row_point(1);
    bench.send(Message::Dnd(Event::Over(x, y)));
    assert_eq!(bench.app.drop_light().map(|l| l.0), Some(None));
    bench.send(Message::Dnd(Event::Dropped(
        x,
        y,
        Action::Move,
        vec![two.clone()],
    )));
    assert_eq!(read(&bench.path("projet/two.txt")), "2");
    assert!(!two.exists());

    // Left: nothing lights up any more.
    bench.send(Message::Dnd(Event::Over(x, y)));
    bench.send(Message::Dnd(Event::Left));
    assert!(bench.app.drop_light().is_none());

    // Files cut in another application, pasted: moved here.
    fs::write(&one, "1 again").expect("write");
    let cut = Some((crate::transfer::Mode::Cut, vec![one.clone()]));
    bench.send(Message::Dnd(Event::Pasted(cut)));
    assert!(!one.exists());
    assert_eq!(read(&bench.path("projet/one.txt")), "1 again");
}

#[test]
fn a_folder_under_a_drop_opens_after_a_rest() {
    use crate::dnd::Event;
    let mut bench = Bench::new("spring");
    let (x, y) = row_point(0);
    bench.send(Message::Dnd(Event::Over(x, y)));
    assert!(
        bench.app.animating(Duration::ZERO),
        "the frames tick for the spring"
    );
    std::thread::sleep(sherlock_ui::drag::SPRING_DELAY + Duration::from_millis(20));
    bench.send(Message::Frame(std::time::Instant::now()));
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
}

fn path_text(bench: &Bench) -> Option<String> {
    bench.app.path_edit.as_ref().map(|e| e.text.clone())
}

/// The path bar as a field (lot E-04): Ctrl+L shows the folder, Tab completes the folders,
/// Enter opens, a path that leads nowhere keeps the field open and shakes it, Escape and a
/// click elsewhere give the breadcrumb back.
#[test]
fn the_path_field_completes_opens_and_says_no() {
    let mut bench = Bench::new("path-field");
    fs::create_dir_all(bench.path("projet/src/deep")).expect("folders");
    fs::create_dir_all(bench.path("projet/srv")).expect("folders");
    bench.ctrl("l");
    let shown = format!("{}/", bench.path("projet").display());
    assert_eq!(path_text(&bench), Some(shown));

    bench.send(Message::PathInput("~/pro".into()));
    let edit = bench.app.path_edit.clone().expect("field open");
    assert_eq!(edit.suggestions, ["projet"]);
    bench.press(Named::Tab);
    assert_eq!(path_text(&bench).as_deref(), Some("~/projet/"));
    // Two folders start with « sr »: Tab goes as far as they agree, then the arrows pick.
    bench.send(Message::PathInput("~/projet/s".into()));
    bench.press(Named::Tab);
    assert_eq!(path_text(&bench).as_deref(), Some("~/projet/sr"));
    bench.press(Named::Tab);
    bench.press(Named::ArrowDown);
    let edit = bench.app.path_edit.clone().expect("field open");
    assert_eq!(
        (edit.suggestions.as_slice(), edit.highlight),
        (&["src".to_owned(), "srv".to_owned()][..], Some(1))
    );
    bench.press(Named::ArrowUp);
    bench.send(Message::PathSubmit);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
    assert!(bench.app.path_edit.is_none(), "the breadcrumb is back");

    // A relative path, typed from the listing.
    bench.key(Key::Character("~".into()), Modifiers::empty());
    assert_eq!(path_text(&bench).as_deref(), Some("~"));
    bench.send(Message::PathInput("deep".into()));
    bench.send(Message::PathSubmit);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src/deep"))
    );

    // Nowhere: the field stays, shakes, and says so.
    bench.key(Key::Character("/".into()), Modifiers::empty());
    bench.send(Message::PathInput("/nulle/part/ici".into()));
    bench.send(Message::PathSubmit);
    let edit = bench.app.path_edit.clone().expect("still open");
    assert!(edit.shake.is_some());
    assert!(bench.app.notice.as_ref().is_some_and(|n| n.error));
    bench.send(Message::Escaped);
    assert!(bench.app.path_edit.is_none() && bench.app.path_leaving.is_some());

    // A click on the listing closes it too; Maj+Ctrl+G opens the sheet.
    bench.ctrl("l");
    bench.send(Message::Blank);
    assert!(bench.app.path_edit.is_none());
    bench.key(
        Key::Character("G".into()),
        Modifiers::CTRL | Modifiers::SHIFT,
    );
    assert!(bench.app.path_edit.as_ref().is_some_and(|e| e.sheet));
    bench.send(Message::PathInput("~".into()));
    bench.send(Message::PathSubmit);
    assert_eq!(bench.app.location, Location::Folder(bench.path("")));

    // The history and the parent folder.
    bench.key(Key::Named(Named::ArrowLeft), Modifiers::ALT);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src/deep"))
    );
    bench.key(Key::Named(Named::ArrowUp), Modifiers::ALT);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
    bench.send(Message::Back);
    bench.send(Message::Forward);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
}

/// Captures of the path bar in a deep folder with a long name, at 1920 × 1080 and in the
/// smallest window: the breadcrumb, the field with its completion, the sheet.
#[test]
fn path_bar_captures() {
    let _theme = crate::testing::in_theme(Theme::Dark);
    let mut bench = Bench::new("path-capture");
    let deep = "projet/rapports-annuels-de-la-societe-version-definitive/2026/novembre/semaine-48";
    fs::create_dir_all(bench.path(deep)).expect("folders");
    for name in ["notes", "photos", "plans", "presentations"] {
        fs::create_dir_all(bench.path(&format!("projet/{name}"))).expect("folders");
    }
    bench.send(Message::Go(Location::Folder(bench.path(deep))));
    for (size, tag) in [
        ((1920.0, 1080.0), "1080p"),
        (sherlock_ui::evidence::MIN_WINDOW, "small"),
    ] {
        bench.send(Message::Resized(iced::Size::new(size.0, size.1)));
        bench.app.settle();
        capture_at(&bench.app, &format!("evidence-path-crumbs-{tag}"), size);
        bench.ctrl("l");
        bench.send(Message::PathInput("~/projet/p".into()));
        bench.app.settle();
        capture_at(&bench.app, &format!("evidence-path-field-{tag}"), size);
        bench.send(Message::Escaped);
        bench.key(
            Key::Character("g".into()),
            Modifiers::CTRL | Modifiers::SHIFT,
        );
        bench.send(Message::PathInput("~/projet/".into()));
        bench.app.settle();
        capture_at(&bench.app, &format!("evidence-path-sheet-{tag}"), size);
        bench.send(Message::PathCancel);
    }
}

/// The window at the widths of a small laptop, half a 1080p screen, a 1366 laptop and a 1080p
/// screen (lot E-05, audit B-1): the names stay readable, the columns, the preview and the
/// sidebar fold around them. Captures in `target/tmp/evidence-width-*.png`.
#[test]
fn the_names_stay_at_every_width() {
    let _theme = crate::testing::in_theme(Theme::Dark);
    let mut bench = bench_with_files("widths", 3);
    fs::write(
        bench.path("projet/compte-rendu-de-la-reunion-du-conseil-d-administration.txt"),
        "x",
    )
    .expect("write");
    bench.press(Named::F5);
    bench.select("a.txt");
    for (width, height) in [
        (720.0, 460.0),
        (944.0, 1048.0),
        (1366.0, 768.0),
        (1920.0, 1080.0),
    ] {
        bench.send(Message::Resized(iced::Size::new(width, height)));
        bench.app.settle();
        let panes = bench.app.panes();
        assert!(
            panes.name_width(width) >= sherlock_ui::evidence::NAME_MIN_WIDTH,
            "{width}: {panes:?}"
        );
        assert_eq!(panes.preview, width >= 1366.0, "{width}");
        capture_at(
            &bench.app,
            &format!("evidence-width-{width}"),
            (width, height),
        );
    }
    // Space brings back a preview the width had folded; again, it goes.
    bench.send(Message::Resized(iced::Size::new(944.0, 1048.0)));
    bench.press(Named::Space);
    assert!(bench.app.panes().preview);
    bench.press(Named::Space);
    assert!(!bench.app.panes().preview);
}

/// On Wayland the menu and its submenu are popups of their own (lot E-02b): they open with
/// the menu, follow its submenu, and go once it has folded back; a click outside, which the
/// compositor reports by closing the popup, closes the menu.
#[test]
fn the_menu_and_its_submenu_are_popups_on_wayland() {
    use crate::app::Sub;
    let mut bench = bench_with_files("popups", 2);
    let main = bench.app.pretend_wayland();
    bench.send(Message::Cursor(iced::Point::new(1250.0, 650.0)));
    bench.send(Message::RightClick(None));
    let menu = bench.app.menu.clone().expect("menu open");
    // A popup is not kept inside the window: it opens where the pointer is.
    assert_eq!(menu.at, iced::Point::new(1250.0, 650.0));
    let (popup, _) = bench.app.popups.menu.expect("menu popup");
    assert!(bench.app.popups.submenu.is_none());
    bench.send(Message::Submenu(Some(Sub::SortBy)));
    let (sub, which) = bench.app.popups.submenu.expect("submenu popup");
    assert_eq!(which, Sub::SortBy);
    // Each popup draws its own menu; the window no longer does.
    assert_ne!(popup, sub);
    assert!(bench.app.popup_view(popup).is_some() && bench.app.popup_view(main).is_none());
    // A click outside: the compositor closes the popup, the menu closes with it.
    bench.send(Message::WindowClosed(popup));
    assert!(bench.app.menu.is_none() && bench.app.popups.menu.is_none());
    // Escape: the popups stay while the menu folds back, then go.
    bench.send(Message::RightClick(None));
    assert!(bench.app.popups.menu.is_some());
    bench.press(Named::Escape);
    assert!(bench.app.menu.is_none() && bench.app.popups.menu.is_some());
    bench.app.settle();
    bench.send(Message::Inert);
    assert!(bench.app.popups.menu.is_none(), "gone once folded back");
}
