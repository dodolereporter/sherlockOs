//! Barre du haut du desktop : un programme séparé, ancré en haut de l'écran par layer-shell
//! (ADR 0005), écrit en iced 0.14 en rendu logiciel (ADR 0014). Voir `docs/barre.md`.

pub mod app;
pub mod menu;
pub mod source;
pub mod user;

use iced::{Color, Settings};
use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize, Margin};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use sherlock_ui::{color, metrics, theme, typography};

use app::{Bar, Message};

/// Nom de la surface layer-shell, que le compositeur peut lire.
const NAMESPACE: &str = "barre";

/// Réglages de la surface : ancrée en haut et sur les côtés, 36 de haut, marges de la
/// planche, zone exclusive de sa hauteur. Le clavier ne vient que si on clique dans la barre.
pub fn layer_settings() -> LayerShellSettings {
    let side = metrics::TOP_BAR_MARGIN_SIDE as i32;
    LayerShellSettings {
        anchor: Anchor::Top | Anchor::Left | Anchor::Right,
        layer: Layer::Top,
        size: LayerSize::fill_width(metrics::TOP_BAR_HEIGHT as u32),
        exclusive_zone: metrics::TOP_BAR_HEIGHT as i32,
        margin: Margin {
            top: metrics::TOP_BAR_MARGIN_TOP as i32,
            right: side,
            bottom: 0,
            left: side,
        },
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        start_mode: StartMode::Active,
        ..LayerShellSettings::default()
    }
}

/// Réglages d'iced : polices de sherlock-ui, texte de l'interface.
pub fn settings() -> Settings {
    Settings {
        fonts: typography::FONTS.map(std::borrow::Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: typography::UI_SIZE.into(),
        ..Settings::default()
    }
}

/// Lance la barre jusqu'à la fin de la session.
pub fn run() -> Result<(), iced_exwlshell::Error> {
    // La barre n'a aucun champ de saisie : inutile de lancer le fil du presse-papiers.
    iced_exwlshell::disable_clipboard();
    iced_exwlshell::daemon(Bar::new, namespace, Bar::update, Bar::view)
        .theme(|_: &Bar, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: color::TEXT_PRIMARY.into(),
        })
        .subscription(Bar::subscription)
        .on_new_shell(|info: ShellInfo| {
            (info.shell == ShellType::LayerShell).then_some(Message::Surface(info.window))
        })
        .settings(settings())
        .wl_settings(ExWlSettings {
            layer_settings: layer_settings(),
            ..ExWlSettings::default()
        })
        .run()
}

fn namespace() -> String {
    String::from(NAMESPACE)
}
