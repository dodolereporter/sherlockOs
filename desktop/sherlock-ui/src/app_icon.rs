//! Application icons drawn by the author (batch D-03): a glyph on a colored tile, in
//! `icons/apps/`. They are installed as a hicolor theme (`icons/hicolor/`, written by
//! `icons/export.py`), where every program finds them by the `Icon` key of its desktop entry; a
//! program of the shell that draws one itself takes it from here. See `docs/sherlock-ui.md`.

use crate::color::{self, Color};

/// The icons, by application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    /// The file manager.
    Evidence,
    /// The terminal.
    Inquiry,
    /// The settings.
    ControlRoom,
    /// The agent, in the middle of the dock.
    Sherlock,
    /// The development environment. Its name is not chosen yet: never shown.
    Lab,
    /// The game library. Its name is not chosen yet: never shown.
    Games,
    /// The installer. Its name is not chosen yet: never shown.
    Installer,
}

/// Sizes the theme holds as PNG, besides the scalable SVG.
pub const PNG_SIZES: [u32; 2] = [16, 32];

/// Size at and below which an icon takes its optical version, when it has one.
pub const OPTICAL_SIZE: f32 = 16.0;

impl AppIcon {
    pub const ALL: [Self; 7] = [
        Self::Evidence,
        Self::Inquiry,
        Self::ControlRoom,
        Self::Sherlock,
        Self::Lab,
        Self::Games,
        Self::Installer,
    ];

    /// Its name in the icon theme: the `Icon` key of the application's desktop entry, and the
    /// desktop file id (`evidence` for `evidence.desktop`).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Evidence => "evidence",
            Self::Inquiry => "inquiry",
            Self::ControlRoom => "control-room",
            Self::Sherlock => "sherlock",
            Self::Lab => "lab",
            Self::Games => "games",
            Self::Installer => "installer",
        }
    }

    /// Background of its tile.
    pub const fn tile(self) -> Color {
        match self {
            Self::Evidence => color::TILE_FILES,
            Self::Inquiry => color::TILE_TERMINAL,
            Self::ControlRoom => color::TILE_SETTINGS,
            Self::Sherlock => color::TILE_SHERLOCK,
            Self::Lab => color::TILE_LAB,
            Self::Games => color::TILE_GAMES,
            Self::Installer => color::TILE_INSTALLER,
        }
    }

    /// The SVG as the author drew it, on a 24 grid.
    pub const fn svg(self) -> &'static str {
        match self {
            Self::Evidence => include_str!("../icons/apps/evidence.svg"),
            Self::Inquiry => include_str!("../icons/apps/inquiry.svg"),
            Self::ControlRoom => include_str!("../icons/apps/control-room.svg"),
            Self::Sherlock => include_str!("../icons/apps/sherlock.svg"),
            Self::Lab => include_str!("../icons/apps/lab.svg"),
            Self::Games => include_str!("../icons/apps/games.svg"),
            Self::Installer => include_str!("../icons/apps/installer.svg"),
        }
    }

    /// The optical version for 16 px, on a 16 grid with every line on whole pixels: thicker
    /// lines, fewer details. Only where the drawing does not hold at 16 px.
    pub const fn optical(self) -> Option<&'static str> {
        match self {
            Self::Evidence => Some(include_str!("../icons/apps/evidence-16.svg")),
            Self::ControlRoom => Some(include_str!("../icons/apps/control-room-16.svg")),
            _ => None,
        }
    }

    /// The SVG to draw at `size` logical pixels times `scale`: the optical version at 16 device
    /// pixels and below, the drawing of the author above.
    pub fn at(self, size: f32, scale: f32) -> &'static str {
        match self.optical() {
            Some(optical) if size * scale <= OPTICAL_SIZE => optical,
            _ => self.svg(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn hex(c: Color) -> String {
        format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
    }

    fn icons() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("icons")
    }

    /// Width and height of a PNG, read from its header.
    fn png_size(path: &Path) -> (u32, u32) {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{}", path.display());
        let word = |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        (word(16), word(20))
    }

    /// Every SVG: the author's, and the optical versions.
    fn drawings() -> Vec<(AppIcon, &'static str)> {
        AppIcon::ALL
            .iter()
            .flat_map(|&i| std::iter::once(i.svg()).chain(i.optical()).map(move |s| (i, s)))
            .collect()
    }

    #[test]
    fn every_icon_is_in_the_theme_at_every_size() {
        let theme = icons().join("hicolor");
        for icon in AppIcon::ALL {
            let scalable = theme.join(format!("scalable/apps/{}.svg", icon.id()));
            let copied = std::fs::read_to_string(&scalable).expect("scalable icon");
            assert_eq!(copied, icon.svg(), "{scalable:?}: run icons/export.py");
            for size in PNG_SIZES {
                let png = theme.join(format!("{size}x{size}/apps/{}.png", icon.id()));
                assert_eq!(png_size(&png), (size, size), "{png:?}");
            }
        }
        // No index.theme: the system's hicolor one describes the folders (docs/sherlock-ui.md).
        assert!(!theme.join("index.theme").exists());
    }

    #[test]
    fn ids_are_unique_and_lowercase() {
        for (i, icon) in AppIcon::ALL.iter().enumerate() {
            let id = icon.id();
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{id}"
            );
            assert!(AppIcon::ALL[..i].iter().all(|other| other.id() != id));
        }
    }

    #[test]
    fn the_desktop_entries_name_icons_of_the_theme() {
        // The desktop entries of our programs, one folder per crate under `desktop/`.
        let desktop = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut seen = Vec::new();
        for program in std::fs::read_dir(&desktop).expect("desktop folder").flatten() {
            let Ok(files) = std::fs::read_dir(program.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().is_none_or(|e| e != "desktop") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("desktop entry");
                let Some(icon) = text.lines().find_map(|l| l.strip_prefix("Icon=")) else {
                    continue;
                };
                let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                assert!(
                    AppIcon::ALL.iter().any(|i| i.id() == icon),
                    "{path:?}: Icon={icon} is not in the theme"
                );
                assert_eq!(icon, id, "{path:?}: the icon takes the desktop file id");
                seen.push(icon.to_owned());
            }
        }
        assert!(seen.iter().any(|i| i == AppIcon::Evidence.id()), "{seen:?}");
    }

    #[test]
    fn tiles_take_their_tokens() {
        for (icon, svg) in drawings() {
            let tile = svg
                .split("clip-path=\"url(#tile)\"><rect")
                .nth(1)
                .and_then(|rest| rest.split("fill=\"").nth(1))
                .and_then(|rest| rest.split('"').next())
                .expect("tile");
            assert_eq!(tile, hex(icon.tile()), "{icon:?}");
        }
    }

    #[test]
    fn line_icons_use_only_tokens() {
        let allowed = [
            color::TILE_FILES,
            color::TILE_TERMINAL,
            color::TILE_SETTINGS,
            color::TILE_LAB,
            color::TILE_GAMES,
            color::TILE_INSTALLER,
            color::TILE_KNOB,
            color::CREAM,
            color::ACCENT,
            color::DEEPEST,
        ]
        .map(hex);
        // Sherlock is an illustration of the mascot: its tile only is a token (above).
        let lines = drawings()
            .into_iter()
            .filter(|(icon, _)| *icon != AppIcon::Sherlock);
        for (icon, svg) in lines {
            for value in svg.split('"').filter(|v| v.starts_with('#')) {
                assert!(allowed.contains(&value.to_owned()), "{icon:?}: {value}");
            }
        }
    }

    #[test]
    fn sixteen_device_pixels_take_the_optical_version() {
        let evidence = AppIcon::Evidence;
        assert_eq!(evidence.at(16.0, 1.0), evidence.optical().expect("optical"));
        assert_eq!(evidence.at(16.0, 2.0), evidence.svg());
        assert_eq!(evidence.at(32.0, 1.0), evidence.svg());
        assert_eq!(AppIcon::Inquiry.at(16.0, 1.0), AppIcon::Inquiry.svg());
        for (_, svg) in drawings() {
            assert!(!svg.contains("<text") && !svg.contains("<image"), "{svg}");
        }
    }
}
