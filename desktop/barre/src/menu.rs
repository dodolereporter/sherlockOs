//! Menu Sherlock : ses entrées (planche Menus), sa taille et sa navigation au clavier.

use iced::keyboard::{self, Key as IcedKey, key::Named};
use sherlock_ui::component::Trailing;
use sherlock_ui::{metrics, strings::top_bar as text};

/// Les entrées du menu. Aucune n'est encore branchée : l'activer l'écrit au journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    About,
    Settings,
    Updates,
    GameMode,
    Lock,
    LogOut,
    PowerOff,
}

impl Action {
    pub const ALL: [Self; 7] = [
        Self::About,
        Self::Settings,
        Self::Updates,
        Self::GameMode,
        Self::Lock,
        Self::LogOut,
        Self::PowerOff,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::About => text::ABOUT,
            Self::Settings => text::SETTINGS,
            Self::Updates => text::UPDATES,
            Self::GameMode => text::GAME_MODE,
            Self::Lock => text::LOCK,
            Self::LogOut => text::LOG_OUT,
            Self::PowerOff => text::POWER_OFF,
        }
    }

    pub fn trailing(self) -> Trailing<'static> {
        match self {
            Self::Settings => Trailing::Shortcut(text::SETTINGS_SHORTCUT),
            Self::GameMode => Trailing::Shortcut(text::GAME_MODE_SHORTCUT),
            Self::Lock => Trailing::Shortcut(text::LOCK_SHORTCUT),
            // La planche annonce un sous-menu, qu'elle ne dessine pas.
            Self::PowerOff => Trailing::Submenu,
            _ => Trailing::None,
        }
    }

    /// Première entrée d'un groupe : un séparateur la précède.
    pub fn starts_group(self) -> bool {
        self == Self::GameMode
    }
}

/// Taille de la surface du menu : largeur, puis hauteur avec l'écart sous la barre.
pub fn surface_size() -> (u32, u32) {
    let (top, _, bottom) = metrics::MENU_HEADER_PADDING;
    let header = top + metrics::MENU_AVATAR + bottom;
    let separator = metrics::BORDER_WIDTH + 2.0 * metrics::MENU_SEPARATOR_MARGIN.0;
    let separators = 1 + Action::ALL.iter().filter(|a| a.starts_group()).count();
    let items = Action::ALL.len() as f32 * metrics::MENU_ITEM_HEIGHT;
    let height = metrics::TOP_BAR_MENU_GAP
        + 2.0 * metrics::MENU_PADDING
        + header
        + separators as f32 * separator
        + items;
    (metrics::MENU_WIDTH as u32, height.ceil() as u32)
}

/// Touche utile dans le menu ouvert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Next,
    Previous,
    First,
    Last,
    Activate,
    Close,
}

/// Les touches du menu : flèches, Début, Fin, Entrée, Espace, Échap. Tab passe par le focus.
pub fn key(event: keyboard::Event) -> Option<Key> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };
    let IcedKey::Named(named) = key else {
        return None;
    };
    Some(match named {
        Named::ArrowDown => Key::Next,
        Named::ArrowUp => Key::Previous,
        Named::Home => Key::First,
        Named::End => Key::Last,
        Named::Enter | Named::Space => Key::Activate,
        Named::Escape => Key::Close,
        _ => return None,
    })
}

/// Entrée surlignée après `key`. Les flèches bouclent ; sans entrée surlignée, Bas prend la
/// première et Haut la dernière.
pub fn move_cursor(cursor: Option<usize>, key: Key) -> Option<usize> {
    let last = Action::ALL.len() - 1;
    match (key, cursor) {
        (Key::First, _) | (Key::Next, None) => Some(0),
        (Key::Last, _) | (Key::Previous, None) => Some(last),
        (Key::Next, Some(i)) => Some(if i >= last { 0 } else { i + 1 }),
        (Key::Previous, Some(i)) => Some(if i == 0 { last } else { i - 1 }),
        (Key::Activate | Key::Close, cursor) => cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_fleches_parcourent_et_bouclent() {
        assert_eq!(move_cursor(None, Key::Next), Some(0));
        assert_eq!(move_cursor(None, Key::Previous), Some(6));
        assert_eq!(move_cursor(Some(0), Key::Next), Some(1));
        assert_eq!(move_cursor(Some(6), Key::Next), Some(0));
        assert_eq!(move_cursor(Some(0), Key::Previous), Some(6));
        assert_eq!(move_cursor(Some(3), Key::First), Some(0));
        assert_eq!(move_cursor(Some(3), Key::Last), Some(6));
    }

    #[test]
    fn le_menu_suit_la_planche() {
        let labels: Vec<_> = Action::ALL.iter().map(|a| a.label()).collect();
        assert_eq!(labels[0], "À propos de cet ordinateur");
        assert_eq!(labels[6], "Éteindre");
        // Planche Menus : 256 de large ; en-tête de 54, deux séparateurs de 9, sept entrées
        // de 30, marges de 6, plus l'écart de 6 sous la barre.
        assert_eq!(surface_size(), (256, 6 + 12 + 54 + 18 + 210));
    }
}
