//! Rendu de la barre et du menu Sherlock sans écran, à partir de fausses sources.

use std::path::PathBuf;

use barre::app::{Bar, Message};
use barre::menu;
use barre::source::battery::{Battery, Charge};
use barre::source::clock;
use barre::source::network::{Link, Network};
use barre::source::sound::Sound;
use barre::source::workspaces::Workspaces;
use iced_test::simulator::Simulator;
use sherlock_ui::{metrics, theme};

/// La barre de la planche Main : trois bureaux, le premier actif, l'éditeur au premier plan.
fn bar() -> Bar {
    let mut bar = Bar::default();
    let messages = [
        Message::Clock(clock::parse("2 24 11 21 46").unwrap()),
        Message::Battery(Some(Battery {
            percent: 82,
            charge: Charge::Discharging,
        })),
        Message::Network(Network::Connected {
            link: Link::Wifi,
            name: String::from("Maison"),
        }),
        Message::Sound(Sound {
            volume: 55,
            muted: false,
        }),
        Message::Workspaces(Workspaces::placeholder(
            Some("Code,Web,Jeu"),
            Some("Éditeur"),
        )),
    ];
    for message in messages {
        let _ = bar.update(message);
    }
    bar.user = String::from("Dodo");
    bar
}

/// Écrit la capture dans `target/tmp/<nom>-tiny-skia.png` et vérifie qu'elle n'est pas vide.
fn capture(mut ui: Simulator<'_, Message>, name: &str) {
    let snapshot = ui.snapshot(&theme::theme()).expect("capture");
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    let written = path.with_file_name(format!("{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    assert!(
        snapshot
            .matches_image(&path)
            .expect("écriture de la capture")
    );
    let size = std::fs::metadata(&written).expect("capture écrite").len();
    assert!(size > 2_000, "capture trop petite : {size} octets");
    println!("capture : {}", written.display());
}

/// Largeur de la barre sur l'écran de référence, marges ôtées.
fn bar_size() -> (f32, f32) {
    let (width, _) = metrics::REFERENCE_SCREEN;
    let width = width - 2.0 * metrics::TOP_BAR_MARGIN_SIDE;
    (width, metrics::TOP_BAR_HEIGHT)
}

#[test]
fn la_barre_se_rend_sans_ecran() {
    let bar = bar();
    let ui = Simulator::with_size(barre::settings(), bar_size(), bar.view_bar());
    capture(ui, "barre");
}

#[test]
fn le_menu_se_rend_sans_ecran() {
    let mut bar = bar();
    bar.cursor = Some(1);
    let (width, height) = menu::surface_size();
    let size = (width as f32, height as f32);
    let ui = Simulator::with_size(barre::settings(), size, bar.view_menu());
    capture(ui, "menu-sherlock");
}

#[test]
fn la_barre_affiche_l_heure_la_date_et_les_bureaux() {
    let bar = bar();
    let mut ui = Simulator::with_size(barre::settings(), bar_size(), bar.view_bar());
    for text in ["21:46", "mar. 24 nov.", "82\u{202F}%", "Éditeur", "3 · Jeu"] {
        assert!(ui.find(text).is_ok(), "{text} absent de la barre");
    }
    ui.click("2 · Web").expect("bureau trouvé");
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(matches!(messages[..], [Message::SelectWorkspace(1)]));
}

#[test]
fn un_clic_dans_le_menu_choisit_son_entree() {
    let bar = bar();
    let (width, height) = menu::surface_size();
    let mut ui = Simulator::with_size(
        barre::settings(),
        (width as f32, height as f32),
        bar.view_menu(),
    );
    assert!(ui.find("Dodo").is_ok());
    ui.click("Verrouiller").expect("entrée trouvée");
    let activated = ui
        .into_messages()
        .any(|m| matches!(m, Message::Activate(menu::Action::Lock)));
    assert!(activated);
}
