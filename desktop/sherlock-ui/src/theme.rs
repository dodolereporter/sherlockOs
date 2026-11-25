//! Thème iced construit sur les jetons, et styles de chaque composant selon son état.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Shadow, Theme, border, theme};

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

/// The theme of an application window: its background is the opaque window body, cleared
/// by the renderer itself instead of drawn as a quad. With the software renderer, a quad as
/// large as the window is filled again for every damaged rectangle of every frame.
pub fn window_theme() -> Theme {
    Theme::custom(
        "Sherlock, fenêtre",
        theme::Palette {
            background: color::WINDOW.into(),
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

/// Destructive button: a secondary button with salmon text. Pressed, it keeps the hover veil:
/// the lighter veil would drop the salmon below a 4.5 contrast on the lightest disc of the
/// desktop.
pub fn danger(theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Disabled => secondary(theme, status),
        button::Status::Pressed => danger(theme, button::Status::Hovered),
        _ => button::Style {
            text_color: color::SALMON.into(),
            ..secondary(theme, status)
        },
    }
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

/// Toolbar button: veiled background at rest, lighter on hover, faded when disabled.
pub fn toolbar_button(_: &Theme, status: button::Status) -> button::Style {
    let (fill, text) = match status {
        button::Status::Active => (Some(color::FIELD), color::TEXT_PRIMARY),
        button::Status::Hovered | button::Status::Pressed => {
            (Some(color::PRESSED), color::TEXT_PRIMARY)
        }
        button::Status::Disabled => (None, color::TEXT_TERTIARY),
    };
    button_style(fill, text, border::rounded(metrics::ICON_BUTTON_RADIUS))
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
/// Its shadow is drawn by `surface::Shadowed` (see `component::glass_panel`), never as an
/// iced shadow: the software renderer paints those outside the redrawn area.
pub fn glass(_: &Theme) -> container::Style {
    surface(
        color::GLASS,
        color::BORDER,
        metrics::PANEL_RADIUS,
        Shadow::default(),
    )
}

/// Menu et sous-menu.
/// Its shadow is drawn by `surface::Shadowed` (see `component::menu`), never as an iced
/// shadow: the software renderer paints those outside the redrawn area.
pub fn menu(_: &Theme) -> container::Style {
    surface(
        color::MENU_GLASS,
        color::MENU_BORDER,
        metrics::MENU_RADIUS,
        Shadow::default(),
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

/// Fond d'écran du bureau.
pub fn desktop(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(color::TEXT_PRIMARY.into()),
        background: background(color::DESKTOP),
        ..container::Style::default()
    }
}

/// Menu entry that destroys or discards: salmon text.
pub fn menu_item_danger(theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Disabled => menu_item(theme, status),
        _ => button::Style {
            text_color: color::SALMON.into(),
            ..menu_item(theme, status)
        },
    }
}

/// Navigation entry: green when active, otherwise veiled on hover.
pub fn nav_item(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| match (active, status) {
        (true, button::Status::Disabled) | (false, _) => ghost(
            status,
            color::TEXT_SECONDARY,
            metrics::NAV_ITEM_RADIUS,
            color::HOVER,
        ),
        (true, _) => button_style(
            Some(color::ACTIVE_ENTRY),
            color::TEXT_PRIMARY,
            border::rounded(metrics::NAV_ITEM_RADIUS),
        ),
    }
}

/// Window button (minimize, maximize, close), inside its pill.
pub fn window_button(_: &Theme, status: button::Status) -> button::Style {
    ghost(
        status,
        color::TEXT_SECONDARY,
        metrics::WINDOW_BUTTON_RADIUS,
        color::HOVER,
    )
}

/// Breadcrumb segment. The current folder uses the primary text colour.
pub fn crumb(current: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    let rest = if current {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_SECONDARY
    };
    move |_, status| ghost(status, rest, metrics::CRUMB_RADIUS, color::HOVER)
}

/// Clickable column header: tertiary grey, lighter on hover.
pub fn column_header(_: &Theme, status: button::Status) -> button::Style {
    let text = match status {
        button::Status::Active | button::Status::Disabled => color::TEXT_TERTIARY,
        _ => color::TEXT_PRIMARY,
    };
    button_style(None, text, border::rounded(0.0))
}

/// Tone of a status badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Amber: worth watching, like a modified file.
    Warning,
    /// Green: new, successful.
    Success,
    /// Grey: of no consequence, like an ignored file.
    Neutral,
}

impl Tone {
    /// Text and background of the badge. On a selected row, the tone's veil would drop below
    /// a 4.5 contrast: the badge then sits on the darkest background.
    pub fn colors(self, selected: bool) -> (Color, Color) {
        let (text, soft) = match self {
            Self::Warning => (color::AMBER, color::AMBER_SOFT),
            Self::Success => (color::ACCENT, color::ACCENT_SOFT),
            Self::Neutral => (color::TEXT_TERTIARY, color::NEUTRAL_SOFT),
        };
        let fill = if selected {
            soft.over(color::DEEPEST)
        } else {
            soft
        };
        (text, fill)
    }
}

/// Status badge, like the git status of a file, on a selected row or not.
pub fn badge(tone: Tone, selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let (text, fill) = tone.colors(selected);
        container::Style {
            text_color: Some(text.into()),
            background: background(fill),
            border: border::rounded(metrics::BADGE_RADIUS),
            ..container::Style::default()
        }
    }
}

/// Green badge without background, like the git branch.
pub fn chip(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(color::ACCENT.into()),
        border: outline(
            metrics::BORDER_WIDTH,
            color::ACCENT_BORDER,
            metrics::CHIP_RADIUS,
        ),
        ..container::Style::default()
    }
}

/// Row of a list, blue when selected.
pub fn list_row(selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(color::TEXT_PRIMARY.into()),
        background: selected.then_some(color::SELECTED_ROW).and_then(background),
        border: border::rounded(metrics::LIST_ROW_RADIUS),
        ..container::Style::default()
    }
}

fn plain(fill: Color, radius: f32) -> container::Style {
    container::Style {
        text_color: Some(color::TEXT_PRIMARY.into()),
        background: background(fill),
        border: border::rounded(radius),
        ..container::Style::default()
    }
}

/// Body of an opaque window.
pub fn window(_: &Theme) -> container::Style {
    plain(color::WINDOW, 0.0)
}

/// Title bar of a window.
pub fn title_bar(_: &Theme) -> container::Style {
    plain(color::TITLE_BAR, 0.0)
}

/// Side panel: navigation on the left, preview on the right.
pub fn sidebar(_: &Theme) -> container::Style {
    plain(color::SIDEBAR, 0.0)
}

/// Application badge, in its title bar.
pub fn app_badge(_: &Theme) -> container::Style {
    plain(color::BLUE_DARK, metrics::APP_BADGE_RADIUS)
}

/// Pill behind the window buttons.
pub fn window_controls(_: &Theme) -> container::Style {
    plain(color::CONTROLS, metrics::WINDOW_CONTROLS_RADIUS)
}

/// Inset area of a toolbar, like the breadcrumb.
pub fn inset(_: &Theme) -> container::Style {
    plain(color::INSET, metrics::BREADCRUMB_RADIUS)
}

/// File tile in the preview: bright blue, dark icon.
pub fn preview_tile(_: &Theme) -> container::Style {
    plain(color::BLUE_BRIGHT, metrics::PREVIEW_TILE_RADIUS)
}

/// Code snippet: darkest background.
pub fn code_block(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(color::TEXT_CODE.into()),
        ..plain(color::DEEPEST, metrics::CODE_BLOCK_RADIUS)
    }
}

/// Veil of any colour, for a fade made of a flat fill that fades away.
pub fn veil(fill: Color) -> container::Style {
    container::Style {
        background: background(fill),
        ..container::Style::default()
    }
}

/// Veil behind a question awaiting an answer.
pub fn scrim(_: &Theme) -> container::Style {
    plain(color::SCRIM, 0.0)
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
        let buttons: [(&str, Style); 4] = [
            ("bouton principal", primary),
            ("bouton secondaire", secondary),
            ("bouton destructif", danger),
            ("bouton d'icône", icon_button),
        ];
        let entries: [(&str, Style); 3] = [
            ("entrée de menu", menu_item),
            ("entrée de menu ouverte", menu_item_open),
            ("entrée de menu destructive", menu_item_danger),
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
        // Evidence mock-up: navigation, window buttons, breadcrumb, headers, status badges on
        // a selected row or not, code snippet.
        for on in surfaces_where(|n| ["fenêtre", "barre de titre", "panneau latéral"].contains(&n))
        {
            type Boxed = Box<dyn Fn(&Theme, button::Status) -> button::Style>;
            let styles: [(&str, Boxed); 7] = [
                ("entrée de navigation", Box::new(nav_item(false))),
                ("entrée de navigation active", Box::new(nav_item(true))),
                ("bouton de fenêtre", Box::new(window_button)),
                ("segment du fil d'Ariane", Box::new(crumb(false))),
                ("dossier courant du fil d'Ariane", Box::new(crumb(true))),
                ("en-tête de colonne", Box::new(column_header)),
                ("bouton de barre d'outils", Box::new(toolbar_button)),
            ];
            let inset = (format!("retrait sur {}", on.0), color::INSET.over(on.1));
            let pill = (format!("pastille sur {}", on.0), color::CONTROLS.over(on.1));
            for (name, style) in &styles {
                for status in STATUSES {
                    let s = style(&theme, status);
                    let what = format!("{name} ({status:?})");
                    for below in [&on, &inset, &pill] {
                        check(&mut failures, &what, s.text_color.into(), fill(&s), below);
                    }
                }
            }
            let row = (
                format!("ligne sélectionnée sur {}", on.0),
                color::SELECTED_ROW.over(on.1),
            );
            for tone in [Tone::Warning, Tone::Success, Tone::Neutral] {
                for (selected, below) in [(false, &on), (true, &row)] {
                    let (text, fill) = tone.colors(selected);
                    let what = format!("étiquette {tone:?}");
                    check(&mut failures, &what, text, Some(fill), below);
                }
            }
            check(&mut failures, "branche git", color::ACCENT, None, &inset);
        }
        let code = (String::from("extrait de code"), color::DEEPEST);
        check(
            &mut failures,
            "texte du code",
            color::TEXT_CODE,
            None,
            &code,
        );
        let tile = (String::from("vignette"), color::BLUE_BRIGHT);
        let tile_icon = color::DEEPEST.contrast(tile.1);
        if tile_icon < 3.0 {
            failures.push(format!("icône de la vignette : {tile_icon:.2}"));
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

    #[test]
    fn floating_surfaces_carry_no_iced_shadow() {
        // iced's software renderer draws a quad shadow outside the clip of the redrawn area:
        // with several damaged rectangles in a frame, a menu turned into a black block. Their
        // shadow is drawn by `surface::Shadowed` instead.
        let theme = theme();
        for (name, style) in [
            ("menu", menu(&theme)),
            ("glass", glass(&theme)),
            ("tooltip", tooltip(&theme)),
            ("card", card(&theme)),
        ] {
            assert_eq!(style.shadow.color.a, 0.0, "{name}");
        }
    }
}
