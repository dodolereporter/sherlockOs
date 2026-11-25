//! Scénarios de bout en bout, sans écran : chaque test pilote l'application au clavier sur un
//! dossier temporaire, exécute les tâches qu'elle lance comme le ferait iced, et regarde le
//! disque. La corbeille est posée dans le dossier temporaire, jamais dans celle de l'auteur.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use futures::StreamExt;
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::{Event, Pixels, Settings, Task};
use iced_runtime::Action;
use iced_test::simulator;
use sherlock_ui::{metrics, theme, typography};

use crate::app::{App, Config, Location, Message};
use crate::places;
use crate::testing::TempDir;
use crate::trash::Trash;

/// Un dossier personnel temporaire : `projet/` avec `src/`, `a.txt` et `.cache`.
struct Bench {
    tmp: TempDir,
    app: App,
}

impl Bench {
    fn new(name: &str) -> Self {
        let tmp = TempDir::new(name);
        let project = tmp.path().join("projet");
        fs::create_dir_all(project.join("src")).expect("dossiers");
        fs::write(project.join("a.txt"), "contenu de a").expect("écriture");
        fs::write(project.join(".cache"), "").expect("écriture");
        let config = Config {
            home: tmp.path().to_path_buf(),
            start: project,
            favorites: places::favorites(tmp.path(), None),
            trash: Some(Trash::at(&tmp.path().join("donnees/Trash"))),
            measure: None,
            maximized: false,
            devices: None,
            watch_devices: false,
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
            unreachable!("press_key rend un appui");
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
            .map(|&i| self.app.entries[i].name.as_str())
            .collect()
    }

    /// Place le curseur sur l'élément nommé, au clavier.
    fn select(&mut self, name: &str) {
        self.press(Named::Home);
        while self.selected() != [name] {
            let before = self.app.cursor;
            self.press(Named::ArrowDown);
            assert_ne!(before, self.app.cursor, "{name} introuvable");
        }
    }
}

/// Exécute les tâches comme le ferait iced, jusqu'à ce qu'il n'en reste plus. Les opérations
/// de widgets (focus, défilement), le presse-papiers et la fenêtre n'ont pas d'écran ici.
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
    fs::read_to_string(path).expect("lecture")
}

#[test]
fn le_clavier_navigue_dans_les_dossiers() {
    let mut bench = Bench::new("navigation");
    assert_eq!(bench.names(), ["src", "a.txt"]);
    bench.press(Named::ArrowDown);
    assert_eq!(bench.selected(), ["src"]);
    bench.press(Named::Enter);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
    // Retour arrière remonte et garde le dossier d'où l'on vient sélectionné.
    bench.press(Named::Backspace);
    assert_eq!(bench.app.location, Location::Folder(bench.path("projet")));
    assert_eq!(bench.selected(), ["src"]);
    // Alt+Gauche revient en arrière dans l'historique, Alt+Droite repart.
    bench.key(Key::Named(Named::ArrowLeft), Modifiers::ALT);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
    bench.key(Key::Named(Named::ArrowRight), Modifiers::ALT);
    assert_eq!(bench.app.location, Location::Folder(bench.path("projet")));
    // Ctrl+H montre les fichiers cachés, puis les cache.
    bench.ctrl("h");
    assert_eq!(bench.names(), ["src", ".cache", "a.txt"]);
    bench.ctrl("h");
    assert_eq!(bench.names(), ["src", "a.txt"]);
    // La recherche filtre, Échap l'efface.
    bench.send(Message::Search("A.T".into()));
    assert_eq!(bench.names(), ["a.txt"]);
    bench.press(Named::Escape);
    assert_eq!(bench.names(), ["src", "a.txt"]);
}

#[test]
fn copier_coller_puis_couper_coller_au_clavier() {
    let mut bench = Bench::new("presse-papiers");
    bench.select("a.txt");
    bench.ctrl("c");
    bench.select("src");
    bench.press(Named::Enter);
    bench.ctrl("v");
    assert_eq!(read(&bench.path("projet/src/a.txt")), "contenu de a");
    assert_eq!(read(&bench.path("projet/a.txt")), "contenu de a");
    // La copie collée est sélectionnée.
    assert_eq!(bench.selected(), ["a.txt"]);
    // Coller une seconde fois au même endroit ne touche pas au premier fichier.
    bench.ctrl("v");
    assert_eq!(bench.names(), ["a (copie).txt", "a.txt"]);

    // Couper vers un dossier qui a déjà ce nom : refusé, rien n'est écrasé.
    bench.select("a.txt");
    fs::write(bench.path("projet/src/a.txt"), "modifié").expect("écriture");
    bench.ctrl("x");
    bench.press(Named::Backspace);
    bench.ctrl("v");
    let notice = bench.app.notice.clone().expect("message d'erreur");
    assert!(notice.error, "{notice:?}");
    assert_eq!(read(&bench.path("projet/src/a.txt")), "modifié");
    assert_eq!(read(&bench.path("projet/a.txt")), "contenu de a");

    // Couper puis coller ailleurs déplace.
    bench.select("src");
    bench.press(Named::Enter);
    bench.select("a (copie).txt");
    bench.ctrl("x");
    bench.press(Named::Backspace);
    bench.ctrl("v");
    assert!(!bench.path("projet/src/a (copie).txt").exists());
    assert!(bench.path("projet/a (copie).txt").exists());
    // Un élément coupé ne se colle qu'une fois.
    bench.ctrl("v");
    assert_eq!(bench.names(), ["src", "a (copie).txt", "a.txt"]);

    // Ctrl+D duplique à côté.
    bench.select("a.txt");
    bench.ctrl("d");
    assert!(bench.path("projet/a (copie 2).txt").exists());
}

#[test]
fn renommer_et_creer_un_dossier_au_clavier() {
    let mut bench = Bench::new("renommage");
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

    // Un nom pris est refusé avec un message.
    bench.press(Named::F2);
    bench.send(Message::RenameInput("src".into()));
    bench.send(Message::RenameSubmit);
    assert!(bench.app.notice.as_ref().is_some_and(|n| n.error));
    assert!(bench.path("projet/notes.txt").exists());

    // Ctrl+Maj+N crée un dossier et propose aussitôt de le nommer.
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
fn la_corbeille_jette_restaure_et_se_vide_avec_confirmation() {
    let mut bench = Bench::new("corbeille");
    let trash = Trash::at(&bench.path("donnees/Trash"));
    bench.select("a.txt");
    bench.press(Named::Delete);
    assert!(!bench.path("projet/a.txt").exists());
    assert_eq!(trash.list().expect("corbeille").len(), 1);
    assert_eq!(bench.names(), ["src"]);

    // Dans la corbeille, le menu du clic droit restaure, au clavier.
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

    // Vider demande une confirmation : Échap annule, rien n'est effacé.
    bench.send(Message::Go(Location::Folder(bench.path("projet"))));
    bench.select("a.txt");
    bench.press(Named::Delete);
    bench.send(Message::Go(Location::Trash));
    bench.send(Message::Run(crate::app::Command::EmptyTrash));
    assert!(bench.app.confirm_empty);
    bench.press(Named::Escape);
    assert!(!bench.app.confirm_empty);
    assert_eq!(trash.list().expect("corbeille").len(), 1);
    bench.send(Message::Run(crate::app::Command::EmptyTrash));
    bench.send(Message::ConfirmEmpty);
    assert!(trash.list().expect("corbeille").is_empty());
    assert!(bench.names().is_empty());
}

#[test]
fn le_menu_du_clic_droit_se_pilote_au_clavier() {
    let mut bench = Bench::new("menu");
    bench.select("a.txt");
    bench.key(Key::Named(Named::ContextMenu), Modifiers::empty());
    let menu = bench.app.menu.clone().expect("menu ouvert");
    assert!(menu.on_selection);
    // Ouvrir, puis Ouvrir avec : la flèche droite ouvre le sous-menu.
    bench.press(Named::ArrowDown);
    bench.press(Named::ArrowDown);
    bench.press(Named::ArrowRight);
    assert!(bench.app.menu.as_ref().is_some_and(|m| m.submenu));
    bench.press(Named::ArrowLeft);
    assert!(bench.app.menu.as_ref().is_some_and(|m| !m.submenu));
    bench.press(Named::Escape);
    assert!(bench.app.menu.is_none());
    // Le menu ne sort pas de la fenêtre.
    bench.send(Message::Cursor(iced::Point::new(10_000.0, 10_000.0)));
    bench.send(Message::RightClick(None));
    let menu = bench.app.menu.clone().expect("menu ouvert");
    assert!(!menu.on_selection);
    assert!(menu.at.x + metrics::CONTEXT_MENU_WIDTH <= bench.app.size.width);
}

/// Un dossier comme celui de la planche : un dépôt git avec un fichier modifié, un nouveau et
/// un dossier ignoré.
fn planche() -> (TempDir, PathBuf) {
    let tmp = TempDir::new("planche");
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
            .expect("git est installé")
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    for sub in ["src", "docs", "essais", "target"] {
        fs::create_dir_all(dir.join(sub)).expect("dossier");
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
        fs::write(dir.join(name), content).expect("écriture");
    }
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "départ"]);
    fs::write(dir.join("src/main.rs"), "fn main() { todo!() }\n").expect("écriture");
    fs::write(dir.join("target/sortie"), "").expect("écriture");
    let docker = "FROM rust:1 AS build\nWORKDIR /src\nCOPY . .\nRUN cargo build --release\n";
    fs::write(dir.join("Dockerfile"), docker).expect("écriture");
    (tmp, dir)
}

/// Capture de la fenêtre, état git lu, Dockerfile sélectionné, menu et sous-menu ouverts, dans
/// `target/tmp/fichiers-tiny-skia.png`. Elle se compare à l'œil à la planche : voir
/// docs/fichiers.md.
#[test]
fn la_fenetre_se_rend_sans_ecran() {
    let (tmp, dir) = planche();
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: dir.clone(),
        favorites: places::favorites(tmp.path(), None),
        trash: Some(Trash::at(&tmp.path().join("donnees/Trash"))),
        measure: None,
        maximized: false,
        devices: None,
        watch_devices: false,
    };
    let (mut app, task) = App::new(config);
    drain(&mut app, task);
    let docker = app
        .entries
        .iter()
        .position(|e| e.name == "Dockerfile")
        .expect("Dockerfile listé");
    let (width, height) = metrics::REFERENCE_WINDOW;
    for message in [
        Message::Click(docker),
        Message::Cursor(iced::Point::new(450.0, 406.0)),
        Message::RightClick(Some(docker)),
        Message::Submenu(true),
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
    // Comme `CARGO_TARGET_TMPDIR`, que Cargo ne donne qu'aux tests d'intégration.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    fs::create_dir_all(&dir).expect("dossier des captures");
    let path = dir.join("evidence.png");
    let written = path.with_file_name("evidence-tiny-skia.png");
    let _ = fs::remove_file(&written);
    assert!(
        snapshot
            .matches_image(&path)
            .expect("écriture de la capture")
    );
    println!("capture : {}", written.display());

    // Un clic sur une entrée du menu lance sa commande.
    let messages: Vec<_> = {
        let mut ui = simulator::Simulator::with_size(settings, (width, height), app.view());
        ui.click("Dupliquer").expect("entrée trouvée");
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

/// Cost of one frame with 5,000 entries: view, layout, then software rendering. Run with
/// `cargo test --release -p evidence frame_cost -- --ignored --nocapture`.
#[test]
#[ignore = "mesure, pas un test"]
fn frame_cost() {
    use std::time::Instant;
    let tmp = TempDir::new("cout");
    let dir = tmp.path().join("grand");
    fs::create_dir_all(&dir).expect("dossier");
    for i in 0..5000 {
        fs::write(dir.join(format!("fichier-{i:05}.txt")), "x").expect("écriture");
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
    fs::create_dir_all(&dir).expect("dossier");
    fs::write(dir.join("frame_cost.txt"), report).expect("écriture");
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
