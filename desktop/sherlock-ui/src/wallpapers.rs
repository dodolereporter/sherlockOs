//! The wallpapers shipped with the desktop: their id, their name as the interface shows it,
//! their category, the theme they belong to, and their pendant in the other theme. Control
//! Room's picker reads this index.
//!
//! The id is the name of the original in `assets/wallpapers/` (`<id>.jpg`) and of the folder of
//! its variants once generated (`<id>/<width>x<height>.png`). See "Wallpapers" in
//! `docs/compositor.md`.

/// Group of wallpapers in the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Default,
    Emblems,
    Investigations,
}

impl Category {
    /// Every category, in the picker's order.
    pub const ALL: [Self; 3] = [Self::Default, Self::Emblems, Self::Investigations];

    /// Name shown in the interface.
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "Par défaut",
            Self::Emblems => "Emblèmes",
            Self::Investigations => "Enquêtes",
        }
    }
}

/// Theme a wallpaper is made for: what goes over it (text, icons) can follow it. The desktop
/// starts in the dark theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeVariant {
    #[default]
    Dark,
    Light,
}

/// A wallpaper of the collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wallpaper {
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub variant: ThemeVariant,
    /// Id of its pendant in the other theme, if the author made one.
    pub pair: Option<&'static str>,
}

const fn wallpaper(
    id: &'static str,
    name: &'static str,
    category: Category,
    variant: ThemeVariant,
) -> Wallpaper {
    Wallpaper {
        id,
        name,
        category,
        variant,
        pair: None,
    }
}

const fn paired(wallpaper: Wallpaper, pair: &'static str) -> Wallpaper {
    Wallpaper {
        pair: Some(pair),
        ..wallpaper
    }
}

use {Category::*, ThemeVariant::*};

/// The collection, in the picker's order: the dark theme, then the light one.
pub const ALL: [Wallpaper; 15] = [
    paired(
        wallpaper("first-clue", "Le premier indice", Default, Dark),
        "first-clue-light",
    ),
    wallpaper("ring", "L'anneau", Emblems, Dark),
    wallpaper("jade-relief", "Relief de jade", Emblems, Dark),
    wallpaper("aurora", "Aurore polaire", Emblems, Dark),
    wallpaper("paper-eclipse", "Éclipse de papier", Emblems, Light),
    wallpaper("rain-trail", "La piste sous la pluie", Investigations, Dark),
    wallpaper("missing-page", "La page disparue", Investigations, Dark),
    wallpaper("last-train", "Le dernier train", Investigations, Dark),
    paired(
        wallpaper("first-clue-light", "Premier indice", Default, Light),
        "first-clue",
    ),
    wallpaper("porcelain", "Porcelaine", Emblems, Light),
    wallpaper("lagoon", "Lagon", Emblems, Light),
    wallpaper("folded-paper", "Papier plié", Emblems, Light),
    wallpaper("greenhouse", "La serre", Investigations, Light),
    wallpaper("observatory", "L'observatoire", Investigations, Light),
    wallpaper("rowboat", "La barque", Investigations, Light),
];

/// Id of the wallpaper of a new session, in the dark theme: Sherlock in the ring. It follows
/// the theme with its pair.
pub const DEFAULT: &str = "first-clue";

/// The wallpaper with this id.
pub fn by_id(id: &str) -> Option<&'static Wallpaper> {
    ALL.iter().find(|w| w.id == id)
}

/// The id to show for the wallpaper `id` in the theme `variant`: its pair if it was made for
/// the other theme and has one, itself otherwise. An id outside the collection (a wallpaper
/// installed by hand) stays as it is.
pub fn for_theme(id: &str, variant: ThemeVariant) -> &str {
    match by_id(id) {
        Some(w) if w.variant != variant => w.pair.unwrap_or(id),
        _ => id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_wallpaper_follows_the_theme() {
        assert_eq!(by_id(DEFAULT).map(|w| w.name), Some("Le premier indice"));
        assert_eq!(for_theme(DEFAULT, Dark), "first-clue");
        assert_eq!(for_theme(DEFAULT, Light), "first-clue-light");
        assert_eq!(for_theme("first-clue-light", Dark), "first-clue");
        assert_eq!(for_theme("first-clue-light", Light), "first-clue-light");
        // Without a pair, a wallpaper stays itself in both themes.
        assert_eq!(for_theme("ring", Light), "ring");
        assert_eq!(for_theme("lagoon", Dark), "lagoon");
        // Outside the collection: untouched.
        assert_eq!(for_theme("my-own", Light), "my-own");
        assert_eq!(by_id("my-own"), None);
    }

    #[test]
    fn every_wallpaper_has_its_original_and_a_consistent_pair() {
        let assets =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wallpapers");
        for (i, w) in ALL.iter().enumerate() {
            assert!(assets.join(format!("{}.jpg", w.id)).is_file(), "{}", w.id);
            assert!(ALL[..i].iter().all(|other| other.id != w.id), "{}", w.id);
            // A pair points back, and lives in the other theme.
            match w.pair {
                Some(pair) => {
                    let pair = by_id(pair);
                    assert_eq!(pair.and_then(|p| p.pair), Some(w.id));
                    assert_ne!(pair.map(|p| p.variant), Some(w.variant));
                }
                None => assert!(ALL.iter().all(|other| other.pair != Some(w.id)), "{}", w.id),
            }
        }
        assert!(
            Category::ALL
                .iter()
                .all(|c| ALL.iter().any(|w| w.category == *c))
        );
    }
}
