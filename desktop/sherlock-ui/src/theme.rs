//! iced theme built on the tokens, and the style of each component by its state.
//!
//! Every style reads its colors from the palette of the iced theme it is given (`palette`):
//! a program that returns `theme()` from its iced `theme` function follows the active theme
//! at its next redraw.

use std::sync::OnceLock;

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Shadow, Theme, border, theme};

use crate::color::{self, Color, Palette};
use crate::metrics;

/// The iced theme of `palette`, with `background` as its background color.
fn custom(name: &'static str, p: &Palette, background: Color) -> Theme {
    Theme::custom(
        name,
        theme::Palette {
            background: background.into(),
            text: p.text_primary.into(),
            primary: p.accent.into(),
            success: p.accent.into(),
            warning: p.amber.into(),
            danger: p.salmon.into(),
        },
    )
}

/// The iced themes of each theme, built once: a view asks for them for every control. The
/// desktop one, then the window one.
fn themes(which: color::Theme) -> &'static (Theme, Theme) {
    static THEMES: OnceLock<[(Theme, Theme); 2]> = OnceLock::new();
    let [dark, light] = THEMES.get_or_init(|| {
        color::Theme::ALL.map(|t| {
            let p = t.palette();
            let (desktop, window) = match t {
                color::Theme::Dark => ("Sherlock", "Sherlock, fenêtre"),
                color::Theme::Light => ("Sherlock clair", "Sherlock clair, fenêtre"),
            };
            (custom(desktop, p, p.desktop), custom(window, p, p.window))
        })
    });
    match which {
        color::Theme::Dark => dark,
        color::Theme::Light => light,
    }
}

/// The desktop theme of `which`.
pub fn theme_for(which: color::Theme) -> Theme {
    themes(which).0.clone()
}

/// The desktop theme, in the active theme. iced widgets without a style of their own take
/// their colors from it.
pub fn theme() -> Theme {
    theme_for(color::active())
}

/// The theme of an application window, in the active theme: its background is the opaque
/// window body, cleared by the renderer itself instead of drawn as a quad. With the software
/// renderer, a quad as large as the window is filled again for every damaged rectangle of
/// every frame.
pub fn window_theme() -> Theme {
    themes(color::active()).1.clone()
}

/// The palette an iced theme was built on, known by its text color, which is exact and
/// differs between the two palettes (a transparent background would say nothing). A theme
/// that is not ours gets the dark palette.
pub fn palette(theme: &Theme) -> &'static Palette {
    if theme.palette().text == iced::Color::from(Palette::LIGHT.text_primary) {
        &Palette::LIGHT
    } else {
        &Palette::DARK
    }
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
    let p = palette(theme);
    let fill = match status {
        button::Status::Active => p.accent,
        button::Status::Hovered => p.accent_hover,
        button::Status::Pressed => p.accent_pressed,
        button::Status::Disabled => return secondary(theme, status),
    };
    button_style(
        Some(fill),
        p.text_on_accent,
        border::rounded(metrics::BUTTON_RADIUS),
    )
}

/// Secondary button: no background, light border.
pub fn secondary(theme: &Theme, status: button::Status) -> button::Style {
    let p = palette(theme);
    let (fill, text, edge) = match status {
        button::Status::Active => (None, p.text_primary, p.secondary_border),
        button::Status::Hovered => (Some(p.hover), p.text_primary, p.secondary_border),
        button::Status::Pressed => (Some(p.pressed), p.text_primary, p.secondary_border),
        button::Status::Disabled => (None, p.text_tertiary, p.divider),
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
            text_color: palette(theme).salmon_text.into(),
            ..secondary(theme, status)
        },
    }
}

fn ghost(
    p: &Palette,
    status: button::Status,
    rest: Color,
    radius: f32,
    hover: Color,
) -> button::Style {
    let (fill, text) = match status {
        button::Status::Active => (None, rest),
        button::Status::Hovered => (Some(hover), p.text_primary),
        button::Status::Pressed => (Some(p.pressed), p.text_primary),
        button::Status::Disabled => (None, p.text_tertiary),
    };
    button_style(fill, text, border::rounded(radius))
}

/// Icon-only button, without background at rest.
pub fn icon_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = palette(theme);
    let radius = metrics::ICON_BUTTON_RADIUS;
    ghost(p, status, p.text_secondary, radius, p.hover)
}

/// Toolbar button: veiled background at rest, lighter on hover, faded when disabled.
pub fn toolbar_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = palette(theme);
    let (fill, text) = match status {
        button::Status::Active => (Some(p.field), p.text_primary),
        button::Status::Hovered | button::Status::Pressed => (Some(p.pressed), p.text_primary),
        button::Status::Disabled => (None, p.text_tertiary),
    };
    button_style(fill, text, border::rounded(metrics::ICON_BUTTON_RADIUS))
}

/// Menu entry: the hovered row takes the selection blue.
pub fn menu_item(theme: &Theme, status: button::Status) -> button::Style {
    let p = palette(theme);
    let radius = metrics::MENU_ITEM_RADIUS;
    ghost(p, status, p.text_primary, radius, p.selected_row)
}

/// Menu entry whose submenu is open.
pub fn menu_item_open(theme: &Theme, status: button::Status) -> button::Style {
    let p = palette(theme);
    match status {
        button::Status::Disabled => menu_item(theme, status),
        _ => button_style(
            Some(p.selected_row),
            p.text_primary,
            border::rounded(metrics::MENU_ITEM_RADIUS),
        ),
    }
}

/// Track of the toggle, green when it is on.
pub fn toggle_track(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = palette(theme);
        let fill = match (on, status) {
            (_, button::Status::Disabled) => p.disabled,
            (true, button::Status::Hovered) => p.accent_hover,
            (true, button::Status::Pressed) => p.accent_pressed,
            (true, button::Status::Active) => p.accent,
            (false, button::Status::Active) => p.toggle_off,
            (false, _) => p.toggle_off_hover,
        };
        button_style(
            Some(fill),
            p.text_primary,
            border::rounded(metrics::TOGGLE_HEIGHT / 2.0),
        )
    }
}

/// Round knob of the toggle: dark on the green in the dark theme, light in the light one,
/// grey on the off track.
pub fn toggle_knob(on: bool, enabled: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let p = palette(theme);
        let fill = match (on, enabled) {
            (_, false) => p.text_tertiary,
            (true, true) => p.deepest,
            (false, true) => p.text_secondary,
        };
        container::Style {
            background: background(fill),
            border: border::rounded(metrics::TOGGLE_HEIGHT / 2.0),
            ..container::Style::default()
        }
    }
}

/// Text field: veiled background, focus ring when it has the keyboard.
pub fn text_field(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let p = palette(theme);
    let radius = metrics::FIELD_RADIUS;
    let edge = match status {
        text_input::Status::Focused { .. } => {
            outline(metrics::FOCUS_RING_WIDTH, p.focus_ring, radius)
        }
        text_input::Status::Hovered => outline(metrics::BORDER_WIDTH, p.secondary_border, radius),
        _ => border::rounded(radius),
    };
    // The tertiary grey drops to 4.25 on the field laid over the lightest disc of the
    // desktop: the placeholder and the disabled value take the secondary grey.
    let value = match status {
        text_input::Status::Disabled => p.text_secondary,
        _ => p.text_primary,
    };
    text_input::Style {
        background: Background::Color(p.field.into()),
        border: edge,
        icon: p.text_tertiary.into(),
        placeholder: p.text_secondary.into(),
        value: value.into(),
        selection: p.selected_row.into(),
    }
}

fn surface(theme: &Theme, fill: Color, edge: Color, radius: f32) -> container::Style {
    container::Style {
        text_color: Some(palette(theme).text_primary.into()),
        background: background(fill),
        border: outline(metrics::BORDER_WIDTH, edge, radius),
        shadow: Shadow::default(),
        ..container::Style::default()
    }
}

/// Card laid in a window.
pub fn card(theme: &Theme) -> container::Style {
    let p = palette(theme);
    surface(theme, p.card, p.card_border, metrics::CARD_RADIUS)
}

/// Floating glass panel. The blur behind it is the compositor's job.
/// Its shadow is drawn by `surface::Shadowed` (see `component::floating_panel`): blurred on the
/// GPU, crisp in software, whose renderer paints iced shadows outside the redrawn area.
pub fn glass(theme: &Theme) -> container::Style {
    let p = palette(theme);
    surface(theme, p.glass, p.border, metrics::PANEL_RADIUS)
}

/// Menu and submenu.
/// Its shadow is drawn by `surface::Shadowed` (see `component::floating_menu`): blurred on the
/// GPU, crisp in software, whose renderer paints iced shadows outside the redrawn area.
pub fn menu(theme: &Theme) -> container::Style {
    let p = palette(theme);
    surface(theme, p.menu_glass, p.menu_border, metrics::MENU_RADIUS)
}

/// Label bubble of an icon-only button: a smaller menu.
pub fn tooltip(theme: &Theme) -> container::Style {
    let edge = palette(theme).menu_border;
    container::Style {
        border: outline(metrics::BORDER_WIDTH, edge, metrics::MENU_ITEM_RADIUS),
        ..menu(theme)
    }
}

/// Separator between two groups of entries of a menu.
pub fn separator(theme: &Theme) -> container::Style {
    veil(palette(theme).border)
}

/// Line between two rows of a card (« Réglages » design board).
pub fn divider(theme: &Theme) -> container::Style {
    veil(palette(theme).divider)
}

/// Top bar: glass of the Main design board, radius 12.
pub fn top_bar(theme: &Theme) -> container::Style {
    let p = palette(theme);
    surface(
        theme,
        p.top_bar_glass,
        p.top_bar_border,
        metrics::TOP_BAR_RADIUS,
    )
}

/// Workspace chip: green when the workspace is active, veiled otherwise.
pub fn chip(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = palette(theme);
        let (fill, text) = match (active, status) {
            (_, button::Status::Disabled) => (p.disabled, p.text_tertiary),
            (true, _) => (p.accent, p.text_on_accent),
            (false, button::Status::Active) => (p.chip, p.text_secondary),
            (false, _) => (p.chip_hover, p.text_primary),
        };
        button_style(Some(fill), text, border::rounded(metrics::CHIP_RADIUS))
    }
}

/// Sherlock button: the color logo straight on the bar glass, without a pill, so that its
/// cream dot stays visible. Veiled on hover and press, and while its menu is open, like an open
/// menu button of the Menus board.
pub fn sherlock_button(open: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = palette(theme);
        let fill = match (open, status) {
            (true, _) => Some(p.top_bar_button_open),
            (false, button::Status::Hovered) => Some(p.hover),
            (false, button::Status::Pressed) => Some(p.pressed),
            (false, _) => None,
        };
        button_style(
            fill,
            p.text_primary,
            border::rounded(metrics::SHERLOCK_BUTTON_RADIUS),
        )
    }
}

/// Desktop wallpaper.
pub fn desktop(theme: &Theme) -> container::Style {
    plain(theme, palette(theme).desktop, 0.0)
}

/// Menu entry that destroys or discards: salmon text.
pub fn menu_item_danger(theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Disabled => menu_item(theme, status),
        _ => button::Style {
            text_color: palette(theme).salmon_text.into(),
            ..menu_item(theme, status)
        },
    }
}

/// Navigation entry: green when active, otherwise veiled on hover.
pub fn nav_item(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = palette(theme);
        let radius = metrics::NAV_ITEM_RADIUS;
        match (active, status) {
            (true, button::Status::Disabled) | (false, _) => {
                ghost(p, status, p.text_secondary, radius, p.hover)
            }
            (true, _) => button_style(
                Some(p.active_entry),
                p.text_primary,
                border::rounded(radius),
            ),
        }
    }
}

/// Window button (minimize, maximize, close), inside its pill.
pub fn window_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = palette(theme);
    let radius = metrics::WINDOW_BUTTON_RADIUS;
    ghost(p, status, p.text_secondary, radius, p.hover)
}

/// Breadcrumb segment. The current folder uses the primary text colour.
pub fn crumb(current: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = palette(theme);
        let rest = if current {
            p.text_primary
        } else {
            p.text_secondary
        };
        ghost(p, status, rest, metrics::CRUMB_RADIUS, p.hover)
    }
}

/// Clickable column header: tertiary grey, lighter on hover.
pub fn column_header(theme: &Theme, status: button::Status) -> button::Style {
    let p = palette(theme);
    let text = match status {
        button::Status::Active | button::Status::Disabled => p.text_tertiary,
        _ => p.text_primary,
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
    pub fn colors(self, p: &Palette, selected: bool) -> (Color, Color) {
        let (text, soft) = match self {
            Self::Warning => (p.amber_text, p.amber_soft),
            Self::Success => (p.accent_text, p.accent_soft),
            Self::Neutral => (p.text_tertiary, p.neutral_soft),
        };
        let fill = if selected { soft.over(p.deepest) } else { soft };
        (text, fill)
    }
}

/// Status badge, like the git status of a file, on a selected row or not.
pub fn badge(tone: Tone, selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let (text, fill) = tone.colors(palette(theme), selected);
        container::Style {
            text_color: Some(text.into()),
            background: background(fill),
            border: border::rounded(metrics::BADGE_RADIUS),
            ..container::Style::default()
        }
    }
}

/// Green badge without background, like the git branch.
pub fn branch_chip(theme: &Theme) -> container::Style {
    let p = palette(theme);
    container::Style {
        text_color: Some(p.accent_text.into()),
        border: outline(
            metrics::BORDER_WIDTH,
            p.accent_border,
            metrics::BRANCH_CHIP_RADIUS,
        ),
        ..container::Style::default()
    }
}

/// Row of a list, blue when selected.
pub fn list_row(selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let p = palette(theme);
        container::Style {
            text_color: Some(p.text_primary.into()),
            background: selected.then_some(p.selected_row).and_then(background),
            border: border::rounded(metrics::LIST_ROW_RADIUS),
            ..container::Style::default()
        }
    }
}

fn plain(theme: &Theme, fill: Color, radius: f32) -> container::Style {
    container::Style {
        text_color: Some(palette(theme).text_primary.into()),
        background: background(fill),
        border: border::rounded(radius),
        ..container::Style::default()
    }
}

/// Body of an opaque window.
pub fn window(theme: &Theme) -> container::Style {
    plain(theme, palette(theme).window, 0.0)
}

/// Title bar of a window.
pub fn title_bar(theme: &Theme) -> container::Style {
    plain(theme, palette(theme).title_bar, 0.0)
}

/// Side panel: navigation on the left, preview on the right.
pub fn sidebar(theme: &Theme) -> container::Style {
    plain(theme, palette(theme).sidebar, 0.0)
}

/// Application badge, in its title bar.
pub fn app_badge(theme: &Theme) -> container::Style {
    plain(theme, palette(theme).blue_dark, metrics::APP_BADGE_RADIUS)
}

/// Pill behind the window buttons.
pub fn window_controls(theme: &Theme) -> container::Style {
    let fill = palette(theme).controls;
    plain(theme, fill, metrics::WINDOW_CONTROLS_RADIUS)
}

/// Inset area of a toolbar, like the breadcrumb.
pub fn inset(theme: &Theme) -> container::Style {
    plain(theme, palette(theme).inset, metrics::BREADCRUMB_RADIUS)
}

/// File tile in the preview: bright blue, dark icon.
pub fn preview_tile(theme: &Theme) -> container::Style {
    let fill = palette(theme).blue_bright;
    plain(theme, fill, metrics::PREVIEW_TILE_RADIUS)
}

/// Code snippet: darkest background.
pub fn code_block(theme: &Theme) -> container::Style {
    let p = palette(theme);
    container::Style {
        text_color: Some(p.text_code.into()),
        ..plain(theme, p.deepest, metrics::CODE_BLOCK_RADIUS)
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
pub fn scrim(theme: &Theme) -> container::Style {
    plain(theme, palette(theme).scrim, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The backgrounds a component can sit on: the desktop and its discs, then the glasses
    /// laid over each of them, and the opaque backgrounds of windows.
    fn surfaces(p: &Palette) -> Vec<(String, Color)> {
        surfaces_where(p, |_| true)
    }

    /// Menus only: a menu entry only sits in a menu.
    fn menus(p: &Palette) -> Vec<(String, Color)> {
        surfaces_where(p, |name| name.starts_with("menu"))
    }

    fn surfaces_where(p: &Palette, keep: fn(&str) -> bool) -> Vec<(String, Color)> {
        let desktops = std::iter::once(p.desktop).chain(p.desktop_discs);
        let mut all: Vec<(String, Color)> = Vec::new();
        for (i, d) in desktops.enumerate() {
            all.push((format!("desktop {i}"), d));
            all.push((format!("glass on desktop {i}"), p.glass.over(d)));
            all.push((format!("menu on desktop {i}"), p.menu_glass.over(d)));
        }
        for (name, c) in [
            ("window", p.window),
            ("title bar", p.title_bar),
            ("sidebar", p.sidebar),
            ("card", p.card),
            ("system screen", p.system_screen),
            ("code area", p.deepest),
            ("terminal block", p.terminal_block),
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

    /// A text below its threshold: where, its color, its contrast.
    type Failure = (String, Color, f32);

    fn check(
        failures: &mut Vec<Failure>,
        what: &str,
        text: Color,
        fill: Option<Color>,
        on: &(String, Color),
    ) {
        let below = fill.map_or(on.1, |f| f.over(on.1));
        let ratio = text.contrast(below);
        if ratio < 4.5 {
            failures.push((format!("{what} on {}", on.0), text, ratio));
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
        let mut failures = Vec::new();
        for which in color::Theme::ALL {
            for (what, _, ratio) in contrasts(which) {
                failures.push(format!("{which:?}: {what}: {ratio:.2}"));
            }
        }
        assert!(
            failures.is_empty(),
            "insufficient contrast:\n{}",
            failures.join("\n")
        );
    }

    /// Every text of `which` below 4.5 to 1 on a background it can sit on, and every
    /// non-text element below 3 to 1.
    fn contrasts(which: color::Theme) -> Vec<Failure> {
        let theme = theme_for(which);
        let p = which.palette();
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
        for on in menus(p) {
            for (name, style) in entries {
                for status in STATUSES {
                    let s = style(&theme, status);
                    let what = format!("{name} ({status:?})");
                    check(&mut failures, &what, s.text_color.into(), fill(&s), &on);
                    // Shortcut, in tertiary text on the same row.
                    let what = format!("shortcut of {what}");
                    check(&mut failures, &what, p.text_tertiary, fill(&s), &on);
                }
            }
        }
        for on in surfaces(p) {
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
                ("primary text", p.text_primary),
                ("secondary text", p.text_secondary),
                ("section label", p.text_tertiary),
            ] {
                check(&mut failures, name, text, None, &on);
            }
        }
        // Code: line numbers, keywords and messages, on the code backgrounds and the editor.
        for on in surfaces_where(p, |n| {
            ["code area", "terminal block", "window"].contains(&n)
        }) {
            for (name, text) in [
                ("line number", p.text_line_number),
                ("keyword", p.blue_light),
                ("type", p.text_code_type),
                ("success", p.accent_text),
                ("warning", p.amber_text),
                ("error", p.salmon_text),
            ] {
                check(&mut failures, name, text, None, &on);
            }
        }
        // Lists: selected row in a window, a sidebar or a card, and active entry of a
        // navigation on the sidebar.
        for on in surfaces_where(p, |n| ["window", "sidebar", "card"].contains(&n)) {
            for (name, text) in [
                ("primary text", p.text_primary),
                ("secondary text", p.text_secondary),
                ("tertiary text", p.text_tertiary),
            ] {
                let what = format!("{name} on selected row");
                check(&mut failures, &what, text, Some(p.selected_row), &on);
            }
        }
        for on in surfaces_where(p, |n| n == "sidebar") {
            for (name, text) in [
                ("primary text", p.text_primary),
                ("secondary text", p.text_secondary),
            ] {
                let what = format!("{name} on active entry");
                check(&mut failures, &what, text, Some(p.active_entry), &on);
            }
        }
        // Evidence mock-up: navigation, window buttons, breadcrumb, headers, status badges on
        // a selected row or not, code snippet.
        for on in surfaces_where(p, |n| ["window", "title bar", "sidebar"].contains(&n)) {
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
            let inset = (format!("inset on {}", on.0), p.inset.over(on.1));
            let pill = (format!("pill on {}", on.0), p.controls.over(on.1));
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
                p.selected_row.over(on.1),
            );
            for tone in [Tone::Warning, Tone::Success, Tone::Neutral] {
                for (selected, below) in [(false, &on), (true, &row)] {
                    let (text, fill) = tone.colors(p, selected);
                    let what = format!("badge {tone:?}");
                    check(&mut failures, &what, text, Some(fill), below);
                }
            }
            check(&mut failures, "git branch", p.accent_text, None, &inset);
        }
        // The keyboard focus ring, a non-text element, around a component on any background.
        for on in surfaces(p) {
            let ratio = p.focus_ring.contrast(on.1);
            if ratio < 3.0 {
                failures.push((format!("focus ring on {}", on.0), p.focus_ring, ratio));
            }
        }
        let code = (String::from("code snippet"), p.deepest);
        check(&mut failures, "code text", p.text_code, None, &code);
        let tile = (String::from("tile"), p.blue_bright);
        let tile_icon = p.text_on_accent.contrast(tile.1);
        if tile_icon < 3.0 {
            failures.push((String::from("tile icon"), p.text_on_accent, tile_icon));
        }
        // Top bar over the desktop and its discs: texts and workspace chips.
        let desktops = std::iter::once(p.desktop).chain(p.desktop_discs);
        for (i, d) in desktops.enumerate() {
            let on = (format!("top bar on desktop {i}"), p.top_bar_glass.over(d));
            for (name, text) in [
                ("top bar text", p.text_primary),
                ("top bar indicator", p.text_secondary),
                ("low battery", p.amber_text),
            ] {
                check(&mut failures, name, text, None, &on);
            }
            // The green ring of the logo, a non-text element: 3 to 1 at least (WCAG 1.4.11),
            // under every state of the Sherlock button.
            for open in [true, false] {
                for status in STATUSES {
                    let s = sherlock_button(open)(&theme, status);
                    let below = fill(&s).map_or(on.1, |f| f.over(on.1));
                    let ratio = p.logo_ring.contrast(below);
                    if ratio < 3.0 {
                        let what = format!("logo ring ({open}, {status:?}) on {}", on.0);
                        failures.push((what, p.logo_ring, ratio));
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
        let cream = (String::from("cream"), p.cream);
        check(
            &mut failures,
            "text on cream",
            p.text_on_cream,
            None,
            &cream,
        );
        let menu = (String::from("menu"), p.menu_glass.over(p.desktop));
        check(&mut failures, "icon tooltip", p.text_primary, None, &menu);
        failures
    }

    #[test]
    fn each_iced_theme_gives_back_its_palette() {
        for which in color::Theme::ALL {
            assert_eq!(palette(&theme_for(which)), which.palette());
            assert_eq!(palette(&themes(which).1), which.palette());
        }
        // A theme that is not ours, even a light one.
        assert_eq!(palette(&Theme::Light), &Palette::DARK);
    }

    #[test]
    fn green_and_amber_texts_stay_off_a_light_selected_row() {
        // No component lays a colored text on a selected row: badges move to the darkest
        // background there (`Tone::colors`). In the light theme, the green and the amber would
        // drop below 4.5 to 1 (4.06 and 3.90 on a card): a component that needs them there must
        // do the same, or the author must choose darker values.
        for which in color::Theme::ALL {
            let p = which.palette();
            let rows: Vec<Color> = surfaces(p)
                .into_iter()
                .filter(|(n, _)| {
                    ["window", "sidebar", "card"].contains(&n.as_str()) || n.contains(" on desktop")
                })
                .map(|(_, c)| p.selected_row.over(c))
                .collect();
            let lowest = |text: Color| {
                rows.iter()
                    .map(|row| text.contrast(*row))
                    .fold(f32::MAX, f32::min)
            };
            for (name, text) in [("salmon", p.salmon_text), ("keyword", p.blue_light)] {
                assert!(lowest(text) >= 4.5, "{which:?} {name}: {:.2}", lowest(text));
            }
            for (name, text) in [("green", p.accent_text), ("amber", p.amber_text)] {
                let ratio = lowest(text);
                match which {
                    color::Theme::Dark => assert!(ratio >= 4.5, "{name}: {ratio:.2}"),
                    color::Theme::Light => assert!(ratio < 4.5, "{name} passes now: {ratio:.2}"),
                }
            }
        }
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
