//! The colors of the desktop for the applications that draw their own interface: a GTK 3 and
//! GTK 4 theme over the built-in one (or adw-gtk3), the libadwaita named colors, a qt5ct and
//! qt6ct palette and a KDE color scheme, each in a dark and a light variant, all generated from
//! the palettes of sherlock-ui. `install` puts them in the user's folders with a backup of
//! every file it replaces, `apply` switches the session to a theme. See
//! `docs/third-party-theming.md`.

pub mod apply;
pub mod gtk;
pub mod install;
pub mod qt;

use sherlock_ui::color::{Color, Palette, Theme};

/// Name of the GTK theme of a variant, which is also the folder it goes in.
pub fn gtk_theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "NOM_OS-dark",
        Theme::Light => "NOM_OS-light",
    }
}

/// Short name of a variant in file names.
pub fn variant(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "dark",
        Theme::Light => "light",
    }
}

/// The opaque color seen when `color` lies over `below`: the toolkits have no glass, and Qt
/// and KDE palettes take no opacity.
pub fn flat(color: Color, below: Color) -> Color {
    let c = color.over(below);
    Color::rgb(c.r, c.g, c.b)
}

/// CSS form: `#RRGGBB` when opaque, `rgba(r, g, b, a)` otherwise.
pub fn css(c: Color) -> String {
    if c.a >= 1.0 {
        format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
    } else {
        format!("rgba({}, {}, {}, {})", c.r, c.g, c.b, round(c.a))
    }
}

/// An opacity with at most three decimals, as written in CSS.
fn round(a: f32) -> f32 {
    (a * 1000.0).round() / 1000.0
}

/// Background of popovers, menus and tooltips: the glass of the menus laid on the window.
pub fn popover(p: &Palette) -> Color {
    flat(p.menu_glass, p.window)
}

/// Background of the selected row, laid on the window.
pub fn selection(p: &Palette) -> Color {
    flat(p.selected_row, p.window)
}
