//! Icônes au trait : grille de 24, trait de 1,7, bouts et angles arrondis.
//!
//! Une icône n'est qu'un tracé SVG. Le cadre (grille, trait, arrondis) est commun et vient des
//! jetons ; la couleur se donne à l'affichage. Pour en ajouter une : une variante, son tracé
//! dessiné sur la grille de 24, et une entrée dans `Icon::ALL`.

use crate::metrics;

/// Les icônes de la bibliothèque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Ouvre un sous-menu.
    Chevron,
    /// Choix coché dans un menu.
    Check,
    /// Recherche.
    Search,
    /// Fermer.
    Close,
    /// Réseau connecté.
    Network,
    /// Son activé.
    Sound,
    /// Revenir en arrière : dossier précédent.
    Back,
    /// Tri croissant et décroissant.
    SortUp,
    SortDown,
    /// Réduire et agrandir une fenêtre.
    Minimize,
    Maximize,
    /// Emplacements et types de fichiers (planche Fichiers).
    Home,
    Desktop,
    Document,
    Download,
    Folder,
    Image,
    Trash,
    /// Conteneur : Dockerfile, fichier compose.
    Cube,
    /// Devices: system disk (from the planche), internal disk, USB key, external disk,
    /// memory card, optical disc; eject and lock.
    SystemDisk,
    Drive,
    UsbKey,
    ExternalDrive,
    Card,
    Optical,
    Eject,
    Lock,
}

impl Icon {
    /// Toutes les icônes, pour les tests et la galerie.
    pub const ALL: [Self; 27] = [
        Self::Chevron,
        Self::Check,
        Self::Search,
        Self::Close,
        Self::Network,
        Self::Sound,
        Self::Back,
        Self::SortUp,
        Self::SortDown,
        Self::Minimize,
        Self::Maximize,
        Self::Home,
        Self::Desktop,
        Self::Document,
        Self::Download,
        Self::Folder,
        Self::Image,
        Self::Trash,
        Self::Cube,
        Self::SystemDisk,
        Self::Drive,
        Self::UsbKey,
        Self::ExternalDrive,
        Self::Card,
        Self::Optical,
        Self::Eject,
        Self::Lock,
    ];

    /// Tracé sur la grille de 24. Tous sont ceux des planches, sauf la croix et les deux
    /// flèches de tri. The device icons are drawn for Evidence, on the same grid and stroke,
    /// except the system disk, which is the planche's.
    pub const fn path(self) -> &'static str {
        match self {
            Self::Chevron => "M9 6l6 6-6 6",
            Self::Check => "M5 12.5l4.5 4.5L19 7.5",
            Self::Search => "M10.5 4a6.5 6.5 0 1 0 0 13a6.5 6.5 0 0 0 0-13zM15.3 15.3L20 20",
            Self::Close => "M6 6l12 12M18 6L6 18",
            Self::Network => {
                "M3 9.5a13 13 0 0 1 18 0M6.2 13a8.5 8.5 0 0 1 11.6 0M9.4 16.4a4 4 0 0 1 5.2 0M12 19.5h.01"
            }
            Self::Sound => {
                "M4 10v4h3.5L12 18V6L7.5 10zM15.5 9.5a3.5 3.5 0 0 1 0 5M18 7a7 7 0 0 1 0 10"
            }
            Self::Back => "M15 6l-6 6 6 6",
            Self::SortUp => "M6 15l6-6 6 6",
            Self::SortDown => "M6 9l6 6 6-6",
            Self::Minimize => "M6 12h12",
            Self::Maximize => "M7 7h10v10H7z",
            Self::Home => "M4 11l8-7 8 7v8a1 1 0 0 1-1 1h-4v-6h-6v6H5a1 1 0 0 1-1-1z",
            Self::Desktop => "M3 5h18v11H3zM9 20h6M12 16v4",
            Self::Document => {
                "M7 3h7l5 5v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM14 3v5h5"
            }
            Self::Download => "M12 4v11M7 11l5 5 5-5M5 20h14",
            Self::Folder => {
                "M3 7.5A1.5 1.5 0 0 1 4.5 6h4.2l2 2.2h8.8A1.5 1.5 0 0 1 21 9.7v7.8A1.5 1.5 0 0 1 19.5 19h-15A1.5 1.5 0 0 1 3 17.5z"
            }
            Self::Image => "M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4",
            Self::Trash => "M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13",
            Self::Cube => "M12 3l8 4.2v9.6L12 21l-8-4.2V7.2zM4 7.2l8 4.2 8-4.2M12 11.4V21",
            Self::SystemDisk => {
                "M4 7h9M17 7h3M4 12h3M11 12h9M4 17h11M19 17h1M15 5v4M9 10v4M17 15v4"
            }
            Self::Drive => {
                "M4 6h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zM7 14h.01M11 14h6"
            }
            Self::UsbKey => {
                "M9 3h6v5H9zM8 8h8v11a2 2 0 0 1-2 2h-4a2 2 0 0 1-2-2zM11 5.5h.01M13 5.5h.01"
            }
            Self::ExternalDrive => {
                "M7 3h10a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM12 17h.01M9.5 7h5"
            }
            Self::Card => {
                "M9 3h8a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7zM10 7v3M13 7v3M16 7v3"
            }
            Self::Optical => {
                "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18zM12 9.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5z"
            }
            Self::Eject => "M12 5l7 8H5zM5 18h14",
            Self::Lock => "M7 11h10v9H7zM9 11V8a3 3 0 0 1 6 0v3",
        }
    }

    /// Document SVG complet, tracé en noir : l'affichage le recolore.
    pub fn svg(self) -> String {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {grid} {grid}\" fill=\"none\" \
             stroke=\"#000\" stroke-width=\"{stroke}\" stroke-linecap=\"round\" \
             stroke-linejoin=\"round\"><path d=\"{path}\"/></svg>",
            grid = metrics::ICON_GRID,
            stroke = metrics::ICON_STROKE,
            path = self.path(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_icone_reste_dans_la_grille() {
        for icon in Icon::ALL {
            let numbers = icon
                .path()
                .split(|c: char| c.is_ascii_alphabetic() || c == ' ' || c == ',')
                .flat_map(|chunk| chunk.split_inclusive('-'))
                .filter_map(|n| n.trim_end_matches('-').parse::<f32>().ok());
            for n in numbers {
                assert!(
                    n.abs() <= metrics::ICON_GRID,
                    "{icon:?} sort de la grille : {n}"
                );
            }
        }
    }

    #[test]
    fn le_cadre_porte_le_trait_et_les_arrondis() {
        let svg = Icon::Check.svg();
        assert!(svg.contains("stroke-width=\"1.7\""));
        assert!(svg.contains("stroke-linecap=\"round\""));
        assert!(svg.contains("viewBox=\"0 0 24 24\""));
    }
}
