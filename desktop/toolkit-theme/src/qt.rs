//! Qt and KDE: the palette of qt5ct and qt6ct, their settings file, and a KDE color scheme
//! (`.colors`).

use std::fmt::Write as _;
use std::path::Path;

use sherlock_ui::color::{Color, Palette, Theme};

use crate::{flat, popover, selection};

/// Number of roles of `QPalette` in Qt 5 (up to `PlaceholderText`) and in Qt 6 (`Accent`
/// added in 6.6). qt5ct and qt6ct read a palette only when it has exactly as many colors.
pub const QT5_ROLES: usize = 21;
pub const QT6_ROLES: usize = 22;

/// The three groups of a Qt palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Active,
    Inactive,
    Disabled,
}

/// The colors of a group, in the order of `QPalette::ColorRole`: WindowText, Button, Light,
/// Midlight, Dark, Mid, Text, BrightText, ButtonText, Base, Window, Shadow, Highlight,
/// HighlightedText, Link, LinkVisited, AlternateBase, NoRole, ToolTipBase, ToolTipText,
/// PlaceholderText, Accent.
pub fn roles(theme: Theme, group: Group) -> [Color; QT6_ROLES] {
    let p = theme.palette();
    let text = if group == Group::Disabled {
        p.text_tertiary
    } else {
        p.text_primary
    };
    let highlight = if group == Group::Disabled {
        flat(p.disabled, p.card)
    } else {
        selection(p)
    };
    [
        text,
        p.card,
        flat(p.chip_hover, p.card),
        flat(p.chip, p.card),
        flat(p.secondary_border, p.window),
        flat(p.border, p.window),
        text,
        p.cream,
        text,
        p.window,
        p.sidebar,
        flat(p.shadow_base, p.window),
        highlight,
        text,
        p.blue_light,
        p.blue_light,
        p.card,
        p.window,
        popover(p),
        p.text_primary,
        p.text_tertiary,
        p.accent,
    ]
}

/// A palette file of qt5ct (`roles` = `QT5_ROLES`) or qt6ct (`QT6_ROLES`).
pub fn qtct_palette(theme: Theme, roles_count: usize) -> String {
    let line = |group| {
        roles(theme, group)
            .iter()
            .take(roles_count)
            .map(|c| format!("#ff{:02x}{:02x}{:02x}", c.r, c.g, c.b))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "[ColorScheme]\nactive_colors={}\ndisabled_colors={}\ninactive_colors={}\n",
        line(Group::Active),
        line(Group::Disabled),
        line(Group::Inactive)
    )
}

/// Key of the settings of qt5ct and qt6ct that names the palette file.
pub const SCHEME_KEY: &str = "color_scheme_path=";

/// The settings file of qt5ct or qt6ct: the palette at `palette`, on the Fusion style, which
/// draws with the palette alone and comes with Qt.
pub fn qtct_settings(palette: &Path) -> String {
    format!(
        "[Appearance]\n{SCHEME_KEY}{}\ncustom_palette=true\nstandard_dialogs=default\nstyle=Fusion\n",
        palette.display()
    )
}

/// `settings` with its palette at `palette`, every other line kept: what the user set in
/// qt5ct or qt6ct stays. `None` when nothing changes.
pub fn with_palette(settings: &str, palette: &Path) -> Option<String> {
    let wanted = format!("{SCHEME_KEY}{}", palette.display());
    let mut found = false;
    let mut out = String::with_capacity(settings.len());
    for line in settings.lines() {
        if line.starts_with(SCHEME_KEY) && !found {
            found = true;
            out.push_str(&wanted);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !found {
        // No key: the settings are not ours to complete. Written again in full.
        return Some(qtct_settings(palette));
    }
    (out != settings).then_some(out)
}

/// File name of a KDE color scheme.
pub fn kde_file_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "NOMOSDark.colors",
        Theme::Light => "NOMOSLight.colors",
    }
}

/// A KDE color scheme. Colors are written `r,g,b`: KDE takes no opacity, so veils are laid
/// on the window first.
pub fn kde_colors(theme: Theme) -> String {
    let p = theme.palette();
    let mut out = String::new();
    let sets = [
        ("Button", p.card, p.title_bar, p),
        (
            "Complementary",
            Palette::DARK.window,
            Palette::DARK.card,
            &Palette::DARK,
        ),
        ("Header", p.title_bar, p.sidebar, p),
        ("Header][Inactive", p.title_bar, p.sidebar, p),
        ("Selection", selection(p), selection(p), p),
        ("Tooltip", popover(p), popover(p), p),
        ("View", p.window, p.card, p),
        ("Window", p.sidebar, p.card, p),
    ];
    for (name, background, alternate, q) in sets {
        let _ = writeln!(out, "[Colors:{name}]");
        let entries = [
            ("BackgroundAlternate", alternate),
            ("BackgroundNormal", background),
            ("DecorationFocus", q.focus_ring),
            ("DecorationHover", q.blue_bright),
            ("ForegroundActive", q.accent_text),
            ("ForegroundInactive", q.text_tertiary),
            ("ForegroundLink", q.blue_light),
            ("ForegroundNegative", q.salmon_text),
            ("ForegroundNeutral", q.amber_text),
            ("ForegroundNormal", q.text_primary),
            ("ForegroundPositive", q.accent_text),
            ("ForegroundVisited", q.blue_light),
        ];
        for (key, color) in entries {
            let _ = writeln!(out, "{key}={},{},{}", color.r, color.g, color.b);
        }
        out.push('\n');
    }
    let name = crate::shown_name(theme);
    let scheme = kde_file_name(theme).trim_end_matches(".colors");
    let _ = writeln!(
        out,
        "[General]\nColorScheme={scheme}\nName={name}\nshadeSortColumn=true\n"
    );
    out.push_str("[KDE]\ncontrast=4\n\n[WM]\n");
    let wm = [
        ("activeBackground", p.title_bar),
        ("activeBlend", p.title_bar),
        ("activeForeground", p.text_primary),
        ("inactiveBackground", p.title_bar),
        ("inactiveBlend", p.title_bar),
        ("inactiveForeground", p.text_tertiary),
    ];
    for (key, color) in wm {
        let _ = writeln!(out, "{key}={},{},{}", color.r, color.g, color.b);
    }
    out
}
