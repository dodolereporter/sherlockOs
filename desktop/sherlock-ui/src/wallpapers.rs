//! The wallpapers shipped with the desktop: their id, their name as the interface shows it,
//! their category, and whether they are light or dark. Control Room's picker reads this index.
//!
//! The id is the name of the original in `assets/wallpapers/` (`<id>.jpg`) and of the folder of
//! its variants once generated (`<id>/<width>x<height>.png`). See "Wallpapers" in
//! `docs/compositor.md`.

/// Group of wallpapers in the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Default,
    Emblem,
    Investigations,
}

impl Category {
    /// Every category, in the picker's order.
    pub const ALL: [Self; 3] = [Self::Default, Self::Emblem, Self::Investigations];

    /// Name shown in the interface.
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "Par défaut",
            Self::Emblem => "Emblème",
            Self::Investigations => "Les enquêtes de Sherlock",
        }
    }
}

/// Overall brightness of a wallpaper: what goes over it (text, icons) can follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Light,
    Dark,
}

/// A wallpaper of the collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wallpaper {
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub tone: Tone,
}

const fn wallpaper(
    id: &'static str,
    name: &'static str,
    category: Category,
    tone: Tone,
) -> Wallpaper {
    Wallpaper {
        id,
        name,
        category,
        tone,
    }
}

/// The collection, in the picker's order.
pub const ALL: [Wallpaper; 8] = [
    wallpaper(
        "first-clue",
        "Le premier indice",
        Category::Default,
        Tone::Dark,
    ),
    wallpaper("ring", "L'anneau", Category::Emblem, Tone::Dark),
    wallpaper(
        "jade-relief",
        "Relief de jade",
        Category::Emblem,
        Tone::Dark,
    ),
    wallpaper("aurora", "Aurore polaire", Category::Emblem, Tone::Dark),
    wallpaper(
        "paper-eclipse",
        "Éclipse de papier",
        Category::Emblem,
        Tone::Light,
    ),
    wallpaper(
        "rain-trail",
        "La piste sous la pluie",
        Category::Investigations,
        Tone::Dark,
    ),
    wallpaper(
        "missing-page",
        "La page disparue",
        Category::Investigations,
        Tone::Dark,
    ),
    wallpaper(
        "last-train",
        "Le dernier train",
        Category::Investigations,
        Tone::Dark,
    ),
];

/// Id of the wallpaper of a new session: Sherlock in the ring.
pub const DEFAULT: &str = "first-clue";

/// The wallpaper with this id.
pub fn by_id(id: &str) -> Option<&'static Wallpaper> {
    ALL.iter().find(|w| w.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_wallpaper_is_in_the_collection() {
        let default = by_id(DEFAULT);
        assert_eq!(default.map(|w| w.name), Some("Le premier indice"));
        assert_eq!(by_id("unknown"), None);
    }

    #[test]
    fn every_wallpaper_has_its_original_and_a_unique_id() {
        let assets =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wallpapers");
        for (i, w) in ALL.iter().enumerate() {
            assert!(assets.join(format!("{}.jpg", w.id)).is_file(), "{}", w.id);
            assert!(ALL[..i].iter().all(|other| other.id != w.id), "{}", w.id);
        }
        // Each category has at least one wallpaper, and only one is light.
        assert!(
            Category::ALL
                .iter()
                .all(|c| ALL.iter().any(|w| w.category == *c))
        );
        assert_eq!(ALL.iter().filter(|w| w.tone == Tone::Light).count(), 1);
    }
}
