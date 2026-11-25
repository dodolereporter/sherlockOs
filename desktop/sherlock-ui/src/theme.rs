//! Thème iced construit sur les jetons, et styles de chaque composant selon son état.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Shadow, Theme, Vector, border, theme};

use crate::color::{self, Color};
use crate::metrics;

/// Le thème du desktop. Les widgets iced sans style propre en tirent leurs couleurs.
pub fn theme() -> Theme {
    Theme::custom(
        "Sherlock",
        theme::Palette {
            background: color::DESKTOP.into(),
            text: color::TEXT_PRIMARY.into(),
            primary: color::ACCENT.into(),
            success: color::ACCENT.into(),
            warning: color::AMBER.into(),
            danger: color::SALMON.into(),
        },
    )
}

fn background(c: Color) -> Option<Background> {
    Some(Background::Color(c.into()))
}

fn outline(width: f32, c: Color, radius: f32) -> Border {
    border::width(width).color(c).rounded(radius)
}

fn shadow(c: Color, offset: f32, blur: f32) -> Shadow {
    Shadow {
        color: c.into(),
        offset: Vector::new(0.0, offset),
        blur_radius: blur,
    }
}

fn button_style(fill: Option<Color>, text: Color, edge: Border) -> button::Style {
    button::Style {
        background: fill.and_then(background),
        text_color: text.into(),
        border: edge,
        ..button::Style::default()
    }
}

/// Bouton principal : vert, texte sombre. Désactivé, il perd son vert et garde un contour.
pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let fill = match status {
        button::Status::Active => color::ACCENT,
        button::Status::Hovered => color::ACCENT_HOVER,
        button::Status::Pressed => color::ACCENT_PRESSED,
        button::Status::Disabled => return secondary(theme, status),
    };
    button_style(
        Some(fill),
        color::TEXT_ON_ACCENT,
        border::rounded(metrics::BUTTON_RADIUS),
    )
}

/// Bouton secondaire : sans fond, bordure claire.
pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    let (fill, text, edge) = match status {
        button::Status::Active => (None, color::TEXT_PRIMARY, color::SECONDARY_BORDER),
        button::Status::Hovered => (
            Some(color::HOVER),
            color::TEXT_PRIMARY,
            color::SECONDARY_BORDER,
        ),
        button::Status::Pressed => (
            Some(color::PRESSED),
            color::TEXT_PRIMARY,
            color::SECONDARY_BORDER,
        ),
        button::Status::Disabled => (None, color::TEXT_TERTIARY, color::DIVIDER),
    };
    let edge = outline(metrics::BORDER_WIDTH, edge, metrics::BUTTON_RADIUS);
    button_style(fill, text, edge)
}

fn ghost(status: button::Status, rest: Color, radius: f32, hover: Color) -> button::Style {
    let (fill, text) = match status {
        button::Status::Active => (None, rest),
        button::Status::Hovered => (Some(hover), color::TEXT_PRIMARY),
        button::Status::Pressed => (Some(color::PRESSED), color::TEXT_PRIMARY),
        button::Status::Disabled => (None, color::TEXT_TERTIARY),
    };
    button_style(fill, text, border::rounded(radius))
}

/// Bouton d'icône seule, sans fond au repos.
pub fn icon_button(_: &Theme, status: button::Status) -> button::Style {
    ghost(
        status,
        color::TEXT_SECONDARY,
        metrics::ICON_BUTTON_RADIUS,
        color::HOVER,
    )
}

/// Entrée de menu : la ligne survolée prend le bleu de sélection.
pub fn menu_item(_: &Theme, status: button::Status) -> button::Style {
    ghost(
        status,
        color::TEXT_PRIMARY,
        metrics::MENU_ITEM_RADIUS,
        color::SELECTED_ROW,
    )
}

/// Entrée de menu dont le sous-menu est ouvert.
pub fn menu_item_open(theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Disabled => menu_item(theme, status),
        _ => button_style(
            Some(color::SELECTED_ROW),
            color::TEXT_PRIMARY,
            border::rounded(metrics::MENU_ITEM_RADIUS),
        ),
    }
}

/// Piste de l'interrupteur, verte quand il est actif.
pub fn toggle_track(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fill = match (on, status) {
            (_, button::Status::Disabled) => color::DISABLED,
            (true, button::Status::Hovered) => color::ACCENT_HOVER,
            (true, button::Status::Pressed) => color::ACCENT_PRESSED,
            (true, button::Status::Active) => color::ACCENT,
            (false, button::Status::Active) => color::TOGGLE_OFF,
            (false, _) => color::TOGGLE_OFF_HOVER,
        };
        button_style(
            Some(fill),
            color::TEXT_PRIMARY,
            border::rounded(metrics::TOGGLE_HEIGHT / 2.0),
        )
    }
}

/// Bouton rond de l'interrupteur : sombre sur le vert, clair sur la piste éteinte.
pub fn toggle_knob(on: bool, enabled: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let fill = match (on, enabled) {
            (_, false) => color::TEXT_TERTIARY,
            (true, true) => color::DEEPEST,
            (false, true) => color::TEXT_SECONDARY,
        };
        container::Style {
            background: background(fill),
            border: border::rounded(metrics::TOGGLE_HEIGHT / 2.0),
            ..container::Style::default()
        }
    }
}

/// Champ de texte : fond voilé, anneau de focus quand il a le clavier.
pub fn text_field(_: &Theme, status: text_input::Status) -> text_input::Style {
    let edge = match status {
        text_input::Status::Focused { .. } => outline(
            metrics::FOCUS_RING_WIDTH,
            color::FOCUS_RING,
            metrics::FIELD_RADIUS,
        ),
        text_input::Status::Hovered => outline(
            metrics::BORDER_WIDTH,
            color::SECONDARY_BORDER,
            metrics::FIELD_RADIUS,
        ),
        _ => border::rounded(metrics::FIELD_RADIUS),
    };
    // Le gris tertiaire tombe à 4,25 sur le champ posé sur le disque le plus clair du bureau :
    // l'indication et la valeur désactivée prennent le gris secondaire.
    let value = match status {
        text_input::Status::Disabled => color::TEXT_SECONDARY,
        _ => color::TEXT_PRIMARY,
    };
    text_input::Style {
        background: Background::Color(color::FIELD.into()),
        border: edge,
        icon: color::TEXT_TERTIARY.into(),
        placeholder: color::TEXT_SECONDARY.into(),
        value: value.into(),
        selection: color::SELECTED_ROW.into(),
    }
}

fn surface(fill: Color, edge: Color, radius: f32, drop: Shadow) -> container::Style {
    container::Style {
        text_color: Some(color::TEXT_PRIMARY.into()),
        background: background(fill),
        border: outline(metrics::BORDER_WIDTH, edge, radius),
        shadow: drop,
        ..container::Style::default()
    }
}

/// Carte posée dans une fenêtre.
pub fn card(_: &Theme) -> container::Style {
    surface(
        color::CARD,
        color::CARD_BORDER,
        metrics::CARD_RADIUS,
        Shadow::default(),
    )
}

/// Panneau de verre flottant. Le flou derrière lui est le travail du compositeur.
pub fn glass(_: &Theme) -> container::Style {
    let drop = shadow(color::SHADOW, metrics::SHADOW_OFFSET, metrics::SHADOW_BLUR);
    surface(color::GLASS, color::BORDER, metrics::PANEL_RADIUS, drop)
}

/// Menu et sous-menu.
pub fn menu(_: &Theme) -> container::Style {
    let drop = shadow(
        color::MENU_SHADOW,
        metrics::MENU_SHADOW_OFFSET,
        metrics::MENU_SHADOW_BLUR,
    );
    surface(
        color::MENU_GLASS,
        color::MENU_BORDER,
        metrics::MENU_RADIUS,
        drop,
    )
}

/// Bulle du libellé d'une icône seule : un menu en plus petit.
pub fn tooltip(theme: &Theme) -> container::Style {
    container::Style {
        border: outline(
            metrics::BORDER_WIDTH,
            color::MENU_BORDER,
            metrics::MENU_ITEM_RADIUS,
        ),
        ..menu(theme)
    }
}

/// Séparateur entre deux groupes d'entrées d'un menu.
pub fn separator(_: &Theme) -> container::Style {
    container::Style {
        background: background(color::BORDER),
        ..container::Style::default()
    }
}

/// Trait entre deux lignes d'une carte (planche Réglages).
pub fn divider(_: &Theme) -> container::Style {
    container::Style {
        background: background(color::DIVIDER),
        ..container::Style::default()
    }
}

/// Top bar: glass of the Main design board, radius 12.
pub fn top_bar(_: &Theme) -> container::Style {
    surface(
        color::TOP_BAR_GLASS,
        color::TOP_BAR_BORDER,
        metrics::TOP_BAR_RADIUS,
        Shadow::default(),
    )
}

/// Workspace chip: green when the workspace is active, veiled otherwise.
pub fn chip(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (fill, text) = match (active, status) {
            (_, button::Status::Disabled) => (color::DISABLED, color::TEXT_TERTIARY),
            (true, _) => (color::ACCENT, color::TEXT_ON_ACCENT),
            (false, button::Status::Active) => (color::CHIP, color::TEXT_SECONDARY),
            (false, _) => (color::CHIP_HOVER, color::TEXT_PRIMARY),
        };
        button_style(Some(fill), text, border::rounded(metrics::CHIP_RADIUS))
    }
}

/// Sherlock button: the color logo straight on the bar glass, without a pill, so that its
/// cream dot stays visible. Veiled on hover and press, and while its menu is open, like an open
/// menu button of the Menus board.
pub fn sherlock_button(open: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fill = match (open, status) {
            (true, _) => Some(color::TOP_BAR_BUTTON_OPEN),
            (false, button::Status::Hovered) => Some(color::HOVER),
            (false, button::Status::Pressed) => Some(color::PRESSED),
            (false, _) => None,
        };
        button_style(
            fill,
            color::TEXT_PRIMARY,
            border::rounded(metrics::SHERLOCK_BUTTON_RADIUS),
        )
    }
}

/// Fond d'écran du bureau.
pub fn desktop(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(color::TEXT_PRIMARY.into()),
        background: background(color::DESKTOP),
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les fonds sur lesquels un composant peut se poser : le bureau et ses disques, puis les
    /// verres posés sur chacun d'eux, et les fonds opaques des fenêtres.
    fn surfaces() -> Vec<(String, Color)> {
        surfaces_where(|_| true)
    }

    /// Les menus seuls : une entrée de menu ne se pose que dans un menu.
    fn menus() -> Vec<(String, Color)> {
        surfaces_where(|name| name.starts_with("menu"))
    }

    fn surfaces_where(keep: fn(&str) -> bool) -> Vec<(String, Color)> {
        let desktops = std::iter::once(color::DESKTOP).chain(color::DESKTOP_DISCS);
        let mut all: Vec<(String, Color)> = Vec::new();
        for (i, d) in desktops.enumerate() {
            all.push((format!("bureau {i}"), d));
            all.push((format!("verre sur bureau {i}"), color::GLASS.over(d)));
            all.push((format!("menu sur bureau {i}"), color::MENU_GLASS.over(d)));
        }
        for (name, c) in [
            ("fenêtre", color::WINDOW),
            ("barre de titre", color::TITLE_BAR),
            ("panneau latéral", color::SIDEBAR),
            ("carte", color::CARD),
            ("écran système", color::SYSTEM_SCREEN),
            ("zone de code", color::DEEPEST),
            ("bloc du terminal", color::TERMINAL_BLOCK),
        ] {
            all.push((name.to_owned(), c));
        }
        all.retain(|(name, _)| keep(name));
        all
    }

    const STATUSES: [button::Status; 4] = [
        button::Status::Active,
        button::Status::Hovered,
        button::Status::Pressed,
        button::Status::Disabled,
    ];

    fn check(
        failures: &mut Vec<String>,
        what: &str,
        text: Color,
        fill: Option<Color>,
        on: &(String, Color),
    ) {
        let below = fill.map_or(on.1, |f| f.over(on.1));
        let ratio = text.contrast(below);
        if ratio < 4.5 {
            failures.push(format!("{what} sur {} : {ratio:.2}", on.0));
        }
    }

    fn fill(style: &button::Style) -> Option<Color> {
        match style.background {
            Some(Background::Color(c)) => Some(c.into()),
            _ => None,
        }
    }

    #[test]
    fn chaque_texte_contraste_d_au_moins_4_5_sur_son_fond() {
        let theme = theme();
        let mut failures = Vec::new();
        type Style = fn(&Theme, button::Status) -> button::Style;
        let buttons: [(&str, Style); 3] = [
            ("bouton principal", primary),
            ("bouton secondaire", secondary),
            ("bouton d'icône", icon_button),
        ];
        let entries: [(&str, Style); 2] = [
            ("entrée de menu", menu_item),
            ("entrée de menu ouverte", menu_item_open),
        ];
        for on in menus() {
            for (name, style) in entries {
                for status in STATUSES {
                    let s = style(&theme, status);
                    let what = format!("{name} ({status:?})");
                    check(&mut failures, &what, s.text_color.into(), fill(&s), &on);
                    // Raccourci, en texte tertiaire sur la même ligne.
                    let what = format!("raccourci de {what}");
                    check(&mut failures, &what, color::TEXT_TERTIARY, fill(&s), &on);
                }
            }
        }
        for on in surfaces() {
            for (name, style) in buttons {
                for status in STATUSES {
                    let s = style(&theme, status);
                    let what = format!("{name} ({status:?})");
                    check(&mut failures, &what, s.text_color.into(), fill(&s), &on);
                }
            }
            for status in [
                text_input::Status::Active,
                text_input::Status::Hovered,
                text_input::Status::Focused { is_hovered: false },
                text_input::Status::Disabled,
            ] {
                let s = text_field(&theme, status);
                let Background::Color(field) = s.background else {
                    panic!("champ sans couleur de fond");
                };
                for (part, text) in [("valeur", s.value), ("indication", s.placeholder)] {
                    let what = format!("{part} du champ ({status:?})");
                    check(&mut failures, &what, text.into(), Some(field.into()), &on);
                }
            }
            // Textes posés directement sur la surface : contenu, description, étiquette.
            for (name, text) in [
                ("texte principal", color::TEXT_PRIMARY),
                ("texte secondaire", color::TEXT_SECONDARY),
                ("étiquette de section", color::TEXT_TERTIARY),
            ] {
                check(&mut failures, name, text, None, &on);
            }
        }
        // Code : numéros de ligne, mots-clés et messages, sur les fonds de code et l'éditeur.
        for on in surfaces_where(|n| ["zone de code", "bloc du terminal", "fenêtre"].contains(&n))
        {
            for (name, text) in [
                ("numéro de ligne", color::TEXT_LINE_NUMBER),
                ("mot-clé bleu", color::BLUE_BRIGHT),
                ("mot-clé bleu clair", color::BLUE_LIGHT),
                ("réussite", color::ACCENT),
                ("avertissement", color::AMBER),
                ("erreur", color::SALMON),
            ] {
                check(&mut failures, name, text, None, &on);
            }
        }
        // Listes : ligne sélectionnée dans une fenêtre, un panneau latéral ou une carte, et
        // entrée active d'une navigation sur le panneau latéral.
        for on in surfaces_where(|n| ["fenêtre", "panneau latéral", "carte"].contains(&n)) {
            for (name, text) in [
                ("texte principal", color::TEXT_PRIMARY),
                ("texte secondaire", color::TEXT_SECONDARY),
                ("texte tertiaire", color::TEXT_TERTIARY),
            ] {
                let what = format!("{name} sur ligne sélectionnée");
                check(&mut failures, &what, text, Some(color::SELECTED_ROW), &on);
            }
        }
        for on in surfaces_where(|n| n == "panneau latéral") {
            for (name, text) in [
                ("texte principal", color::TEXT_PRIMARY),
                ("texte secondaire", color::TEXT_SECONDARY),
            ] {
                let what = format!("{name} sur entrée active");
                check(&mut failures, &what, text, Some(color::ACTIVE_ENTRY), &on);
            }
        }
        // Top bar over the desktop and its discs: texts and workspace chips.
        let desktops = std::iter::once(color::DESKTOP).chain(color::DESKTOP_DISCS);
        for (i, d) in desktops.enumerate() {
            let on = (
                format!("top bar on desktop {i}"),
                color::TOP_BAR_GLASS.over(d),
            );
            for (name, text) in [
                ("top bar text", color::TEXT_PRIMARY),
                ("top bar indicator", color::TEXT_SECONDARY),
                ("low battery", color::AMBER),
            ] {
                check(&mut failures, name, text, None, &on);
            }
            // The green ring of the logo, a non-text element: 3 to 1 at least (WCAG 1.4.11),
            // under every state of the Sherlock button.
            for open in [true, false] {
                for status in STATUSES {
                    let s = sherlock_button(open)(&theme, status);
                    let below = fill(&s).map_or(on.1, |f| f.over(on.1));
                    let ratio = color::ACCENT.contrast(below);
                    if ratio < 3.0 {
                        failures.push(format!(
                            "logo ring ({open}, {status:?}) on {}: {ratio:.2}",
                            on.0
                        ));
                    }
                }
            }
            for active in [true, false] {
                for status in STATUSES {
                    let s = chip(active)(&theme, status);
                    let what = format!("workspace chip ({active}, {status:?})");
                    check(&mut failures, &what, s.text_color.into(), fill(&s), &on);
                }
            }
        }
        // Textes sur les fonds opaques qui leur sont réservés.
        let cream = (String::from("crème"), color::CREAM);
        check(
            &mut failures,
            "texte sur crème",
            color::TEXT_ON_CREAM,
            None,
            &cream,
        );
        let menu = (String::from("menu"), color::MENU_GLASS.over(color::DESKTOP));
        check(
            &mut failures,
            "bulle d'icône",
            color::TEXT_PRIMARY,
            None,
            &menu,
        );
        assert!(
            failures.is_empty(),
            "contraste insuffisant :\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn la_selection_des_planches_ferait_echouer_les_raccourcis() {
        // La planche Menus pose une entrée ouverte à 0,30 d'opacité. Avec un raccourci, le
        // texte tertiaire tomberait sous 4,5 : le jeton garde les 0,26 de `contexte.md`.
        let row =
            Color::rgba(91, 163, 208, 0.30).over(color::MENU_GLASS.over(color::DESKTOP_DISCS[2]));
        assert!(color::TEXT_TERTIARY.contrast(row) < 4.5);
        let row = color::SELECTED_ROW.over(color::MENU_GLASS.over(color::DESKTOP_DISCS[2]));
        assert!(color::TEXT_TERTIARY.contrast(row) >= 4.5);
    }
}
