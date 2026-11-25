//! iced theme built on the tokens, and the style of each component by its state.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Shadow, Theme, border, theme};

use crate::color::{self, Color};
use crate::metrics;

/// The desktop theme. iced widgets without a style of their own take their colors from it.
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

/// Primary button: green, dark text. Disabled, it loses its green and keeps an outline.
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

/// Secondary button: no background, light border.
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

/// Icon-only button, without background at rest.
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

/// Menu entry: the hovered row takes the selection blue.
pub fn menu_item(_: &Theme, status: button::Status) -> button::Style {
    ghost(
        status,
        color::TEXT_PRIMARY,
        metrics::MENU_ITEM_RADIUS,
        color::SELECTED_ROW,
    )
}

/// Menu entry whose submenu is open.
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

/// Track of the toggle, green when it is on.
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

/// Round knob of the toggle: dark on the green, light on the off track.
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

/// Text field: veiled background, focus ring when it has the keyboard.
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
    // The tertiary grey drops to 4.25 on the field laid over the lightest disc of the
    // desktop: the placeholder and the disabled value take the secondary grey.
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

/// Card laid in a window.
pub fn card(_: &Theme) -> container::Style {
    surface(
        color::CARD,
        color::CARD_BORDER,
        metrics::CARD_RADIUS,
        Shadow::default(),
    )
}

/// Floating glass panel. The blur behind it is the compositor's job.
/// Its shadow is drawn by `surface::Shadowed` (see `component::floating_panel`): blurred on the
/// GPU, crisp in software, whose renderer paints iced shadows outside the redrawn area.
pub fn glass(_: &Theme) -> container::Style {
    surface(
        color::GLASS,
        color::BORDER,
        metrics::PANEL_RADIUS,
        Shadow::default(),
    )
}

/// Menu and submenu.
/// Its shadow is drawn by `surface::Shadowed` (see `component::floating_menu`): blurred on the
/// GPU, crisp in software, whose renderer paints iced shadows outside the redrawn area.
pub fn menu(_: &Theme) -> container::Style {
    surface(
        color::MENU_GLASS,
        color::MENU_BORDER,
        metrics::MENU_RADIUS,
        Shadow::default(),
    )
}

/// Label bubble of an icon-only button: a smaller menu.
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

/// Separator between two groups of entries of a menu.
pub fn separator(_: &Theme) -> container::Style {
    container::Style {
        background: background(color::BORDER),
        ..container::Style::default()
    }
}

/// Line between two rows of a card (« Réglages » design board).
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

/// Desktop wallpaper.
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
pub fn branch_chip(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(color::ACCENT.into()),
        border: outline(
            metrics::BORDER_WIDTH,
            color::ACCENT_BORDER,
            metrics::BRANCH_CHIP_RADIUS,
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

    /// The backgrounds a component can sit on: the desktop and its discs, then the glasses
    /// laid over each of them, and the opaque backgrounds of windows.
    fn surfaces() -> Vec<(String, Color)> {
        surfaces_where(|_| true)
    }

    /// Menus only: a menu entry only sits in a menu.
    fn menus() -> Vec<(String, Color)> {
        surfaces_where(|name| name.starts_with("menu"))
    }

    fn surfaces_where(keep: fn(&str) -> bool) -> Vec<(String, Color)> {
        let desktops = std::iter::once(color::DESKTOP).chain(color::DESKTOP_DISCS);
        let mut all: Vec<(String, Color)> = Vec::new();
        for (i, d) in desktops.enumerate() {
            all.push((format!("desktop {i}"), d));
            all.push((format!("glass on desktop {i}"), color::GLASS.over(d)));
            all.push((format!("menu on desktop {i}"), color::MENU_GLASS.over(d)));
        }
        for (name, c) in [
            ("window", color::WINDOW),
            ("title bar", color::TITLE_BAR),
            ("sidebar", color::SIDEBAR),
            ("card", color::CARD),
            ("system screen", color::SYSTEM_SCREEN),
            ("code area", color::DEEPEST),
            ("terminal block", color::TERMINAL_BLOCK),
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
            failures.push(format!("{what} on {}: {ratio:.2}", on.0));
        }
    }

    fn fill(style: &button::Style) -> Option<Color> {
        match style.background {
            Some(Background::Color(c)) => Some(c.into()),
            _ => None,
        }
    }

    #[test]
    fn every_text_contrasts_at_least_4_5_on_its_background() {
        let theme = theme();
        let mut failures = Vec::new();
        type Style = fn(&Theme, button::Status) -> button::Style;
        let buttons: [(&str, Style); 4] = [
            ("primary button", primary),
            ("secondary button", secondary),
            ("destructive button", danger),
            ("icon button", icon_button),
        ];
        let entries: [(&str, Style); 3] = [
            ("menu entry", menu_item),
            ("open menu entry", menu_item_open),
            ("destructive menu entry", menu_item_danger),
        ];
        for on in menus() {
            for (name, style) in entries {
                for status in STATUSES {
                    let s = style(&theme, status);
                    let what = format!("{name} ({status:?})");
                    check(&mut failures, &what, s.text_color.into(), fill(&s), &on);
                    // Shortcut, in tertiary text on the same row.
                    let what = format!("shortcut of {what}");
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
                    panic!("field without a background color");
                };
                for (part, text) in [("value", s.value), ("placeholder", s.placeholder)] {
                    let what = format!("field {part} ({status:?})");
                    check(&mut failures, &what, text.into(), Some(field.into()), &on);
                }
            }
            // Texts laid straight on the surface: content, description, label.
            for (name, text) in [
                ("primary text", color::TEXT_PRIMARY),
                ("secondary text", color::TEXT_SECONDARY),
                ("section label", color::TEXT_TERTIARY),
            ] {
                check(&mut failures, name, text, None, &on);
            }
        }
        // Code: line numbers, keywords and messages, on the code backgrounds and the editor.
        for on in surfaces_where(|n| ["code area", "terminal block", "window"].contains(&n)) {
            for (name, text) in [
                ("line number", color::TEXT_LINE_NUMBER),
                ("blue keyword", color::BLUE_BRIGHT),
                ("light blue keyword", color::BLUE_LIGHT),
                ("success", color::ACCENT),
                ("warning", color::AMBER),
                ("error", color::SALMON),
            ] {
                check(&mut failures, name, text, None, &on);
            }
        }
        // Lists: selected row in a window, a sidebar or a card, and active entry of a
        // navigation on the sidebar.
        for on in surfaces_where(|n| ["window", "sidebar", "card"].contains(&n)) {
            for (name, text) in [
                ("primary text", color::TEXT_PRIMARY),
                ("secondary text", color::TEXT_SECONDARY),
                ("tertiary text", color::TEXT_TERTIARY),
            ] {
                let what = format!("{name} on selected row");
                check(&mut failures, &what, text, Some(color::SELECTED_ROW), &on);
            }
        }
        for on in surfaces_where(|n| n == "sidebar") {
            for (name, text) in [
                ("primary text", color::TEXT_PRIMARY),
                ("secondary text", color::TEXT_SECONDARY),
            ] {
                let what = format!("{name} on active entry");
                check(&mut failures, &what, text, Some(color::ACTIVE_ENTRY), &on);
            }
        }
        // Evidence mock-up: navigation, window buttons, breadcrumb, headers, status badges on
        // a selected row or not, code snippet.
        for on in surfaces_where(|n| ["window", "title bar", "sidebar"].contains(&n)) {
            type Boxed = Box<dyn Fn(&Theme, button::Status) -> button::Style>;
            let styles: [(&str, Boxed); 7] = [
                ("navigation entry", Box::new(nav_item(false))),
                ("active navigation entry", Box::new(nav_item(true))),
                ("window button", Box::new(window_button)),
                ("breadcrumb segment", Box::new(crumb(false))),
                ("current folder of the breadcrumb", Box::new(crumb(true))),
                ("column header", Box::new(column_header)),
                ("toolbar button", Box::new(toolbar_button)),
            ];
            let inset = (format!("inset on {}", on.0), color::INSET.over(on.1));
            let pill = (format!("pill on {}", on.0), color::CONTROLS.over(on.1));
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
                format!("selected row on {}", on.0),
                color::SELECTED_ROW.over(on.1),
            );
            for tone in [Tone::Warning, Tone::Success, Tone::Neutral] {
                for (selected, below) in [(false, &on), (true, &row)] {
                    let (text, fill) = tone.colors(selected);
                    let what = format!("badge {tone:?}");
                    check(&mut failures, &what, text, Some(fill), below);
                }
            }
            check(&mut failures, "git branch", color::ACCENT, None, &inset);
        }
        let code = (String::from("code snippet"), color::DEEPEST);
        check(&mut failures, "code text", color::TEXT_CODE, None, &code);
        let tile = (String::from("tile"), color::BLUE_BRIGHT);
        let tile_icon = color::DEEPEST.contrast(tile.1);
        if tile_icon < 3.0 {
            failures.push(format!("tile icon: {tile_icon:.2}"));
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
        // Texts on the opaque backgrounds reserved for them.
        let cream = (String::from("cream"), color::CREAM);
        check(
            &mut failures,
            "text on cream",
            color::TEXT_ON_CREAM,
            None,
            &cream,
        );
        let menu = (String::from("menu"), color::MENU_GLASS.over(color::DESKTOP));
        check(
            &mut failures,
            "icon tooltip",
            color::TEXT_PRIMARY,
            None,
            &menu,
        );
        assert!(
            failures.is_empty(),
            "insufficient contrast:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn the_design_board_selection_would_fail_the_shortcuts() {
        // The Menus board lays an open entry at 0.30 opacity. With a shortcut, the tertiary
        // text would drop below 4.5: the token keeps the 0.26 of `docs/contexte.md`.
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
