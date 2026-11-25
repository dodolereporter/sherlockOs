//! Base components, true to the Menus and « Réglages » design boards.
//!
//! Clickable components return a `Control`: an iced button, its style by state, and the
//! keyboard focus. Without a message, a component is disabled and leaves the keyboard order.

use iced::widget::{
    Button, Container, Svg, Text, TextInput, button, column, container, row, space, svg, text,
    text_input, tooltip,
};
use iced::{Alignment, Element, Length, Padding, Theme};

use crate::color::{self, Color};
use crate::focus::Focusable;
use crate::icon::Icon;
use crate::logo::Logo;
use crate::{metrics, theme, typography};

type StyleFn<'a> = Box<dyn Fn(&Theme, button::Status) -> button::Style + 'a>;

/// A clickable component, ready to become an `Element`.
pub struct Control<'a, Message> {
    button: Button<'a, Message>,
    style: StyleFn<'a>,
    on_press: Option<Message>,
    radius: f32,
    forced: Option<button::Status>,
    ring: bool,
    label: Option<&'a str>,
}

impl<'a, Message: Clone + 'a> Control<'a, Message> {
    fn new(
        content: impl Into<Element<'a, Message>>,
        style: impl Fn(&Theme, button::Status) -> button::Style + 'a,
        on_press: Option<Message>,
        radius: f32,
    ) -> Self {
        Self {
            button: button(content).on_press_maybe(on_press.clone()),
            style: Box::new(style),
            on_press,
            radius,
            forced: None,
            ring: false,
            label: None,
        }
    }

    /// Forces a state, instead of the one the mouse and the keyboard give. For the gallery.
    pub fn status(mut self, status: button::Status) -> Self {
        self.forced = Some(status);
        self
    }

    /// Draws the focus ring without focus. For the gallery.
    ///
    /// Also used by programs that keep the focus in their own state, such as the bar.
    pub fn focus_ring(mut self) -> Self {
        self.ring = true;
        self
    }

    /// Width of the component, `Shrink` by default.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.button = self.button.width(width);
        self
    }
}

impl<'a, Message: Clone + 'a> From<Control<'a, Message>> for Element<'a, Message> {
    fn from(control: Control<'a, Message>) -> Self {
        let Control {
            button,
            style,
            on_press,
            radius,
            forced,
            ring,
            label,
        } = control;
        // A component without a background at rest (menu entry, icon button, secondary
        // button…) fades its hover veil in: the button no longer draws it, `Hoverable` lays it
        // underneath. A forced state (gallery) is still drawn as is.
        let hover = hover_fill(&style, forced.is_some(), on_press.is_some());
        let styled = button.style(move |theme, status| {
            let mut s = style(theme, forced.unwrap_or(status));
            if hover.is_some() && status == button::Status::Hovered {
                s.background = None;
            }
            s
        });
        let content: Element<'a, Message> = match hover {
            Some(fill) => crate::animated::hoverable(styled, fill.into(), radius).into(),
            None => styled.into(),
        };
        let focusable = Focusable::new(content, on_press, radius).ring(ring);
        match label {
            // iced 0.14 exposes nothing to screen readers: the label of an icon-only button
            // shows as a tooltip on hover, until AccessKit (see docs/sherlock-ui.md).
            Some(label) => tooltip(
                focusable,
                container(body(label))
                    .padding(metrics::MENU_PADDING)
                    .style(theme::tooltip),
                tooltip::Position::Bottom,
            )
            .into(),
            None => focusable.into(),
        }
    }
}

/// The veil a control fades in on hover: only for an enabled control without a background
/// at rest, and not when the gallery forces its state.
fn hover_fill(style: &StyleFn<'_>, forced: bool, enabled: bool) -> Option<iced::Color> {
    if forced || !enabled {
        return None;
    }
    let rest = style(&Theme::Dark, button::Status::Active).background;
    match style(&Theme::Dark, button::Status::Hovered).background {
        Some(iced::Background::Color(c)) if rest.is_none() => Some(c),
        _ => None,
    }
}

/// Body text of the interface.
pub fn body<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
    text(content).font(typography::UI).size(typography::UI_SIZE)
}

/// Secondary text: description, value.
pub fn description<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
    text(content)
        .font(typography::UI)
        .size(typography::DESCRIPTION_SIZE)
        .color(color::TEXT_SECONDARY)
}

/// Page title, in Space Grotesk.
pub fn title<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
    text(content)
        .font(typography::TITLE)
        .size(typography::TITLE_SIZE)
}

/// Section label: small grey capitals.
pub fn section_label<'a>(content: &str) -> Text<'a> {
    text(content.to_uppercase())
        .font(typography::UI_SEMIBOLD)
        .size(typography::SECTION_LABEL_SIZE)
        .line_height(text::LineHeight::Relative(typography::LINE_HEIGHT))
        .color(color::TEXT_TERTIARY)
}

/// The SVG document of an icon, built once and for all: a view that draws hundreds of them
/// rebuilds neither the string nor its hash.
fn handle(glyph: Icon) -> svg::Handle {
    static HANDLES: std::sync::OnceLock<Vec<svg::Handle>> = std::sync::OnceLock::new();
    let all = HANDLES.get_or_init(|| {
        Icon::ALL
            .iter()
            .map(|i| svg::Handle::from_memory(i.svg().into_bytes()))
            .collect()
    });
    Icon::ALL
        .iter()
        .position(|i| *i == glyph)
        .and_then(|p| all.get(p))
        .cloned()
        .unwrap_or_else(|| svg::Handle::from_memory(glyph.svg().into_bytes()))
}

/// Line icon, in the given color.
pub fn icon<'a>(icon: Icon, size: f32, tint: Color) -> Svg<'a> {
    svg(handle(icon))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style {
            color: Some(tint.into()),
        })
}

fn label_button<'a, Message: Clone + 'a>(
    label: &'a str,
    font: iced::Font,
    style: fn(&Theme, button::Status) -> button::Style,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    let content = container(body(label).font(font)).center_y(Length::Fill);
    let mut control = Control::new(content, style, on_press, metrics::BUTTON_RADIUS);
    control.button = control
        .button
        .height(metrics::BUTTON_HEIGHT)
        .padding([0.0, metrics::BUTTON_PADDING]);
    control
}

fn block_button<'a, Message: Clone + 'a>(
    label: &'a str,
    font: iced::Font,
    style: fn(&Theme, button::Status) -> button::Style,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    let content = container(body(label).font(font)).center(Length::Fill);
    let mut control = Control::new(content, style, on_press, metrics::BUTTON_RADIUS);
    control.button = control
        .button
        .width(Length::Fill)
        .height(metrics::BUTTON_HEIGHT)
        .padding(0);
    control
}

/// Full-width primary button, centred label (preview panel).
pub fn primary_block<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    block_button(label, typography::UI_SEMIBOLD, theme::primary, on_press)
}

/// Full-width secondary button, centred label.
pub fn secondary_block<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    block_button(label, typography::UI, theme::secondary, on_press)
}

/// Icon button of a toolbar (back, forward): veiled background when it acts, faded when it
/// can do nothing. `label` is mandatory.
pub fn toolbar_button<'a, Message: Clone + 'a>(
    glyph: Icon,
    label: &'a str,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    let tint = if on_press.is_some() {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_TERTIARY
    };
    let content = container(icon(glyph, metrics::ICON_SIZE, tint)).center(Length::Fill);
    let mut control = Control::new(
        content,
        theme::toolbar_button,
        on_press,
        metrics::ICON_BUTTON_RADIUS,
    );
    control.button = control
        .button
        .width(metrics::ICON_BUTTON)
        .height(metrics::ICON_BUTTON)
        .padding(0);
    control.label = Some(label);
    control
}

/// Search field: the magnifier on the left, inside the field. `id` lets the caller focus it.
pub fn search_field<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    on_input: Option<fn(String) -> Message>,
    id: impl Into<iced::widget::Id>,
    width: f32,
) -> Element<'a, Message> {
    let inset = metrics::FIELD_PADDING + metrics::ICON_SIZE_SMALL + metrics::SEARCH_ICON_GAP;
    let line = typography::UI_SIZE * typography::LINE_HEIGHT;
    let vertical = (metrics::FIELD_HEIGHT - line) / 2.0;
    let field = text_field(placeholder, value, on_input)
        .id(id)
        .width(width)
        .padding(Padding {
            top: vertical,
            bottom: vertical,
            left: inset,
            right: metrics::FIELD_PADDING,
        });
    let glass = container(icon(
        Icon::Search,
        metrics::ICON_SIZE_SMALL,
        color::TEXT_TERTIARY,
    ))
    .padding(Padding {
        left: metrics::FIELD_PADDING,
        ..Padding::ZERO
    })
    .height(metrics::FIELD_HEIGHT)
    .center_y(metrics::FIELD_HEIGHT);
    iced::widget::stack![field, glass].into()
}

/// Primary button, green.
pub fn primary_button<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    label_button(label, typography::UI_SEMIBOLD, theme::primary, on_press)
}

/// Secondary button, outlined.
pub fn secondary_button<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    label_button(label, typography::UI, theme::secondary, on_press)
}

/// Destructive button: secondary, salmon text. To empty, delete, discard.
pub fn danger_button<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    label_button(label, typography::UI, theme::danger, on_press)
}

/// Icon-only button. `label` is mandatory: it is its name for whoever cannot see the icon.
pub fn icon_button<'a, Message: Clone + 'a>(
    glyph: Icon,
    label: &'a str,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    let tint = if on_press.is_some() {
        color::TEXT_SECONDARY
    } else {
        color::TEXT_TERTIARY
    };
    let content = container(icon(glyph, metrics::ICON_SIZE, tint)).center(Length::Fill);
    let mut control = Control::new(
        content,
        theme::icon_button,
        on_press,
        metrics::ICON_BUTTON_RADIUS,
    );
    control.button = control
        .button
        .width(metrics::ICON_BUTTON)
        .height(metrics::ICON_BUTTON)
        .padding(0);
    control.label = Some(label);
    control
}

/// Toggle of 44 by 26. `on_toggle` receives the new value.
pub fn toggle<'a, Message: Clone + 'a>(
    on: bool,
    on_toggle: Option<fn(bool) -> Message>,
) -> Control<'a, Message> {
    let knob_size = metrics::TOGGLE_HEIGHT - 2.0 * metrics::TOGGLE_INSET;
    let knob = container(space()).width(knob_size).height(knob_size);
    let knob = knob.style(theme::toggle_knob(on, on_toggle.is_some()));
    let align = if on { Alignment::End } else { Alignment::Start };
    let track = container(knob).width(Length::Fill).align_x(align);
    let mut control = Control::new(
        track,
        theme::toggle_track(on),
        on_toggle.map(|f| f(!on)),
        metrics::TOGGLE_HEIGHT / 2.0,
    );
    control.button = control
        .button
        .width(metrics::TOGGLE_WIDTH)
        .height(metrics::TOGGLE_HEIGHT)
        .padding(metrics::TOGGLE_INSET);
    control
}

/// What follows the label of a menu entry, on the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trailing<'a> {
    None,
    /// Keyboard shortcut, such as « Super+M ».
    Shortcut(&'a str),
    /// Opens a submenu, on the right.
    Submenu,
    /// Active choice.
    Checked,
}

/// Menu entry 30 high, across the full width of the menu.
pub fn menu_item<'a, Message: Clone + 'a>(
    label: &'a str,
    trailing: Trailing<'a>,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    menu_item_with(None, label, trailing, on_press)
}

/// Menu entry with a leading picture, such as an application's icon (`metrics::ICON_SIZE`).
pub fn menu_item_with<'a, Message: Clone + 'a>(
    leading: Option<Element<'a, Message>>,
    label: &'a str,
    trailing: Trailing<'a>,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    let end: Element<'a, Message> = match trailing {
        Trailing::None => space().into(),
        Trailing::Shortcut(keys) => text(keys)
            .font(typography::UI)
            .size(typography::SMALL_SIZE)
            .color(color::TEXT_TERTIARY)
            .into(),
        Trailing::Submenu => icon(
            Icon::Chevron,
            metrics::ICON_SIZE_SMALL,
            color::TEXT_SECONDARY,
        )
        .into(),
        Trailing::Checked => icon(Icon::Check, metrics::ICON_SIZE_SMALL, color::ACCENT).into(),
    };
    let start: Element<'a, Message> = match leading {
        Some(leading) => row![leading, body(label).wrapping(text::Wrapping::None)]
            .spacing(metrics::GAP_ICON)
            .align_y(Alignment::Center)
            .into(),
        None => body(label).wrapping(text::Wrapping::None).into(),
    };
    let line = row![start, space::horizontal(), end]
        .spacing(metrics::GAP_INLINE)
        .align_y(Alignment::Center)
        .height(Length::Fill);
    let mut control = Control::new(line, theme::menu_item, on_press, metrics::MENU_ITEM_RADIUS);
    control.button = control
        .button
        .width(Length::Fill)
        .height(metrics::MENU_ITEM_HEIGHT)
        .padding([0.0, metrics::MENU_ITEM_PADDING]);
    control
}

/// Marks a menu entry as open: its submenu is shown.
pub fn open<'a, Message: Clone + 'a>(mut item: Control<'a, Message>) -> Control<'a, Message> {
    item.style = Box::new(theme::menu_item_open);
    item
}

/// Line between two groups of menu entries.
pub fn menu_separator<'a, Message: 'a>() -> Element<'a, Message> {
    let (top, side) = metrics::MENU_SEPARATOR_MARGIN;
    let line = container(space())
        .width(Length::Fill)
        .height(metrics::BORDER_WIDTH)
        .style(theme::separator);
    container(line).padding(Padding::from([top, side])).into()
}

/// Menu or submenu: menu glass, entries in a column.
pub fn menu<'a, Message: 'a>(
    entries: impl IntoIterator<Item = Element<'a, Message>>,
    width: f32,
) -> Container<'a, Message> {
    container(iced::widget::Column::with_children(entries))
        .width(width)
        .padding(metrics::MENU_PADDING)
        .style(theme::menu)
}

/// A menu floating over a window, over its shadow (`surface::Shadowed`): context menus.
/// A menu that fills its own surface (the bar's) uses `menu`, and lays its shadow itself.
pub fn floating_menu<'a, Message: 'a>(
    entries: impl IntoIterator<Item = Element<'a, Message>>,
    width: f32,
) -> Element<'a, Message> {
    crate::surface::shadowed(
        menu(entries, width),
        metrics::MENU_RADIUS,
        crate::surface::Elevation::Menu,
    )
    .into()
}

/// Text field 32 high. Without `on_input`, it is disabled.
pub fn text_field<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    on_input: Option<fn(String) -> Message>,
) -> TextInput<'a, Message> {
    let line = typography::UI_SIZE * typography::LINE_HEIGHT;
    let vertical = (metrics::FIELD_HEIGHT - line) / 2.0;
    text_input(placeholder, value)
        .on_input_maybe(on_input)
        .font(typography::UI)
        .size(typography::UI_SIZE)
        .line_height(text::LineHeight::Relative(typography::LINE_HEIGHT))
        .padding([vertical, metrics::FIELD_PADDING])
        .style(theme::text_field)
}

/// Line between two rows of a card.
pub fn divider<'a, Message: 'a>() -> Element<'a, Message> {
    container(space())
        .width(Length::Fill)
        .height(metrics::BORDER_WIDTH)
        .style(theme::divider)
        .into()
}

/// Setting row of the « Réglages » design board: label, help below, control on the right.
pub fn setting_row<'a, Message: Clone + 'a>(
    label: &'a str,
    help: &'a str,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let help = text(help)
        .font(typography::UI)
        .size(typography::SMALL_SIZE)
        .color(color::TEXT_SECONDARY);
    row![
        column![body(label).font(typography::UI_MEDIUM), help],
        space::horizontal(),
        control.into(),
    ]
    .align_y(Alignment::Center)
    .spacing(metrics::GAP_ROW)
    .height(metrics::SETTING_ROW_HEIGHT)
    .into()
}

/// Card laid in a window.
pub fn card<'a, Message: 'a>(content: impl Into<Element<'a, Message>>) -> Container<'a, Message> {
    let (vertical, horizontal) = metrics::CARD_PADDING;
    container(content)
        .padding([vertical, horizontal])
        .style(theme::card)
}

/// Floating glass panel.
pub fn glass_panel<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Container<'a, Message> {
    container(content)
        .padding(metrics::PANEL_PADDING)
        .style(theme::glass)
}

/// A glass panel of the given width over its shadow: dialogs.
pub fn floating_panel<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    width: f32,
) -> Element<'a, Message> {
    crate::surface::shadowed(
        glass_panel(content).width(width),
        metrics::PANEL_RADIUS,
        crate::surface::Elevation::Panel,
    )
    .into()
}

/// Marks a menu entry as destructive: salmon text (Evidence mock-up).
pub fn danger<'a, Message: Clone + 'a>(mut item: Control<'a, Message>) -> Control<'a, Message> {
    item.style = Box::new(theme::menu_item_danger);
    item
}

/// Entry of a side navigation: icon, label, optional counter on the right.
pub fn nav_item<'a, Message: Clone + 'a>(
    glyph: Icon,
    label: impl text::IntoFragment<'a>,
    count: Option<String>,
    active: bool,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    let tint = if active {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_SECONDARY
    };
    let font = if active {
        typography::UI_SEMIBOLD
    } else {
        typography::UI
    };
    let mut line = row![
        icon(glyph, metrics::ICON_SIZE, tint),
        body(label).font(font)
    ]
    .spacing(metrics::GAP_ICON)
    .align_y(Alignment::Center)
    .height(Length::Fill);
    if let Some(count) = count {
        line = line
            .push(space::horizontal())
            .push(body(count).color(color::TEXT_TERTIARY));
    }
    let mut control = Control::new(
        line,
        theme::nav_item(active),
        on_press,
        metrics::NAV_ITEM_RADIUS,
    );
    control.button = control
        .button
        .width(Length::Fill)
        .height(metrics::NAV_ITEM_HEIGHT)
        .padding([0.0, metrics::NAV_ITEM_PADDING]);
    control
}

/// Same as `nav_item`, with a label computed by the caller (a volume name cut to fit).
pub fn nav_item_owned<'a, Message: Clone + 'a>(
    glyph: Icon,
    label: String,
    count: Option<String>,
    active: bool,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    nav_item(glyph, label, count, active, on_press)
}

/// Section label of a side navigation, with its padding.
pub fn nav_section<'a, Message: 'a>(label: &str) -> Element<'a, Message> {
    let (top, side, bottom) = metrics::NAV_SECTION_PADDING;
    container(section_label(label))
        .padding(Padding {
            top,
            right: side,
            bottom,
            left: side,
        })
        .into()
}

/// Status badge, like the git status of a file. `selected`: placed on a selected row.
pub fn badge<'a, Message: 'a>(
    label: &'a str,
    tone: theme::Tone,
    selected: bool,
) -> Element<'a, Message> {
    let (vertical, horizontal) = metrics::BADGE_PADDING;
    container(
        text(label)
            .font(typography::UI)
            .size(typography::BADGE_SIZE),
    )
    .padding([vertical, horizontal])
    .style(theme::badge(tone, selected))
    .into()
}

/// Green badge in the code font, like the git branch.
pub fn branch_chip<'a, Message: 'a>(label: String) -> Element<'a, Message> {
    let (vertical, horizontal) = metrics::BRANCH_CHIP_PADDING;
    container(
        text(label)
            .font(typography::CODE)
            .size(typography::CODE_SMALL_SIZE),
    )
    .padding([vertical, horizontal])
    .style(theme::branch_chip)
    .into()
}

/// The three window buttons in their pill: minimize, maximize, close.
pub fn window_controls<'a, Message: Clone + 'a>(
    labels: [&'a str; 3],
    on_press: [Message; 3],
) -> Element<'a, Message> {
    let glyphs = [Icon::Minimize, Icon::Maximize, Icon::Close];
    let (width, height) = metrics::WINDOW_BUTTON;
    let buttons = glyphs
        .into_iter()
        .zip(labels)
        .zip(on_press)
        .map(|((glyph, label), message)| {
            let content = container(icon(glyph, metrics::ICON_SIZE_SMALL, color::TEXT_SECONDARY))
                .center(Length::Fill);
            let mut control = Control::new(
                content,
                theme::window_button,
                Some(message),
                metrics::WINDOW_BUTTON_RADIUS,
            );
            control.button = control.button.width(width).height(height).padding(0);
            control.label = Some(label);
            control.into()
        });
    container(iced::widget::Row::with_children(buttons).spacing(metrics::GAP_TIGHT))
        .padding(metrics::WINDOW_CONTROLS_PADDING)
        .style(theme::window_controls)
        .into()
}

/// Clickable breadcrumb segment. The current folder is semibold.
pub fn crumb<'a, Message: Clone + 'a>(
    label: String,
    current: bool,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    let font = if current {
        typography::UI_SEMIBOLD
    } else {
        typography::UI
    };
    let mut control = Control::new(
        body(label).font(font),
        theme::crumb(current),
        on_press,
        metrics::CRUMB_RADIUS,
    );
    let (vertical, horizontal) = metrics::CRUMB_PADDING;
    control.button = control.button.padding([vertical, horizontal]);
    control
}

/// Column header of a list, clickable to sort. `sorted` gives the sort direction when the
/// list is sorted on this column: true for ascending.
pub fn column_header<'a, Message: Clone + 'a>(
    label: &str,
    sorted: Option<bool>,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    let label = text(label.to_uppercase())
        .font(typography::UI_SEMIBOLD)
        .size(typography::COLUMN_HEADER_SIZE);
    let mut line = row![label]
        .spacing(metrics::GAP_TIGHT)
        .align_y(Alignment::Center);
    if let Some(ascending) = sorted {
        let glyph = if ascending {
            Icon::SortUp
        } else {
            Icon::SortDown
        };
        line = line.push(icon(glyph, metrics::ICON_SIZE_TINY, color::TEXT_TERTIARY));
    }
    let mut control = Control::new(line, theme::column_header, on_press, 0.0);
    control.button = control.button.padding(0);
    control
}

/// Role of a piece of code: its colour in a snippet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeRole {
    Plain,
    /// Keyword, in light blue as in the mock-ups.
    Keyword,
    /// Comment, in the grey of line numbers.
    Comment,
}

impl CodeRole {
    pub const fn color(self) -> Color {
        match self {
            Self::Plain => color::TEXT_CODE,
            Self::Keyword => color::BLUE_LIGHT,
            Self::Comment => color::TEXT_LINE_NUMBER,
        }
    }
}

/// Code snippet or start of a text file, in JetBrains Mono: one line per entry, each made of
/// pieces coloured by their role.
pub fn code_block<'a, Message: 'a>(lines: &'a [Vec<(String, CodeRole)>]) -> Container<'a, Message> {
    let lines = lines.iter().map(|line| {
        let spans: Vec<iced::widget::text::Span<'a, (), iced::Font>> = line
            .iter()
            .map(|(piece, role)| iced::widget::span(piece.as_str()).color(role.color()))
            .collect();
        iced::widget::rich_text(spans)
            .font(typography::CODE)
            .size(typography::CODE_SMALL_SIZE)
            .line_height(text::LineHeight::Absolute(
                typography::CODE_LINE_HEIGHT.into(),
            ))
            .wrapping(text::Wrapping::None)
            .into()
    });
    let (vertical, horizontal) = metrics::CODE_BLOCK_PADDING;
    // No clip: the caller cuts the lines to the width (`metrics::PREVIEW_COLUMNS`), since a
    // clip costs the software renderer a full mask on every frame.
    container(iced::widget::Column::with_children(lines))
        .padding([vertical, horizontal])
        .width(Length::Fill)
        .style(theme::code_block)
}

/// Workspace chip in the top bar, green when the workspace is active.
pub fn chip<'a, Message: Clone + 'a>(
    label: impl text::IntoFragment<'a>,
    active: bool,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    let font = if active {
        typography::UI_SEMIBOLD
    } else {
        typography::UI
    };
    let content = container(body(label).font(font)).center_y(Length::Fill);
    let mut control = Control::new(content, theme::chip(active), on_press, metrics::CHIP_RADIUS);
    control.button = control
        .button
        .height(metrics::CHIP_HEIGHT)
        .padding([0.0, metrics::CHIP_PADDING]);
    control
}

/// Logo symbol, without its square, at the given size. The bare symbol has no optical
/// version: it fills its frame and reads at 16 px.
pub fn logo<'a>(version: Logo, size: f32) -> Svg<'a> {
    svg(svg::Handle::from_memory(version.bare().as_bytes()))
        .width(size)
        .height(size)
}

/// Logo in its square, at the given size: the optical version at 32 px and below.
pub fn logo_square<'a>(version: Logo, size: f32) -> Svg<'a> {
    svg(svg::Handle::from_memory(version.square(size).as_bytes()))
        .width(size)
        .height(size)
}

/// Sherlock button of the top bar: the color logo symbol, green ring and cream dot.
pub fn sherlock_button<'a, Message: Clone + 'a>(
    open: bool,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    let (width, height) = metrics::SHERLOCK_BUTTON;
    let content = container(logo(Logo::Color, metrics::SHERLOCK_GLYPH)).center(Length::Fill);
    let mut control = Control::new(
        content,
        theme::sherlock_button(open),
        on_press,
        metrics::SHERLOCK_BUTTON_RADIUS,
    );
    control.button = control.button.width(width).height(height).padding(0);
    control
}

/// Top bar indicator: an icon, followed by its value if there is one.
pub fn status<'a, Message: 'a>(
    glyph: Icon,
    value: Option<String>,
    tint: Color,
) -> Element<'a, Message> {
    let mut line = row![icon(glyph, metrics::ICON_SIZE, tint)]
        .spacing(metrics::STATUS_ICON_GAP)
        .align_y(Alignment::Center);
    if let Some(value) = value {
        line = line.push(body(value).color(tint));
    }
    line.into()
}

/// Menu header: the color logo in its night blue square, title and subtitle.
pub fn menu_header<'a, Message: 'a>(
    title: impl text::IntoFragment<'a>,
    subtitle: &'a str,
) -> Element<'a, Message> {
    let avatar = logo_square(Logo::Color, metrics::MENU_AVATAR);
    let subtitle = text(subtitle)
        .font(typography::UI)
        .size(typography::SMALL_SIZE)
        .color(color::TEXT_SECONDARY);
    let (top, side, bottom) = metrics::MENU_HEADER_PADDING;
    row![
        avatar,
        column![body(title).font(typography::UI_SEMIBOLD), subtitle]
    ]
    .spacing(metrics::GAP_ICON)
    .align_y(Alignment::Center)
    .padding(Padding {
        top,
        right: side,
        bottom,
        left: side,
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_enabled_ghost_controls_fade_their_hover() {
        let ghost: StyleFn<'_> = Box::new(theme::menu_item);
        let filled: StyleFn<'_> = Box::new(theme::primary);
        assert!(hover_fill(&ghost, false, true).is_some());
        assert!(hover_fill(&ghost, false, false).is_none(), "disabled");
        assert!(
            hover_fill(&ghost, true, true).is_none(),
            "forced by the gallery"
        );
        assert!(
            hover_fill(&filled, false, true).is_none(),
            "background at rest"
        );
    }
}
