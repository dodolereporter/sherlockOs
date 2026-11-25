//! Composants de base, conformes aux planches Menus et Réglages.
//!
//! Les composants cliquables rendent un `Control` : un bouton iced, son style par état, et le
//! focus clavier. Sans message, un composant est désactivé et sort du parcours au clavier.

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

/// Un composant cliquable, prêt à devenir un `Element`.
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

    /// Impose un état, au lieu de celui que donnent la souris et le clavier. Pour la galerie.
    pub fn status(mut self, status: button::Status) -> Self {
        self.forced = Some(status);
        self
    }

    /// Dessine l'anneau de focus sans focus. Pour la galerie.
    ///
    /// Also used by programs that keep the focus in their own state, such as the bar.
    pub fn focus_ring(mut self) -> Self {
        self.ring = true;
        self
    }

    /// Largeur du composant, `Shrink` par défaut.
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
        let styled = button.style(move |theme, status| style(theme, forced.unwrap_or(status)));
        let focusable = Focusable::new(styled, on_press, radius).ring(ring);
        match label {
            // iced 0.14 n'expose rien aux lecteurs d'écran : le libellé d'une icône seule
            // s'affiche en bulle au survol, en attendant AccessKit (voir docs/sherlock-ui.md).
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

/// Texte courant de l'interface.
pub fn body<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
    text(content).font(typography::UI).size(typography::UI_SIZE)
}

/// Texte secondaire : description, valeur.
pub fn description<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
    text(content)
        .font(typography::UI)
        .size(typography::DESCRIPTION_SIZE)
        .color(color::TEXT_SECONDARY)
}

/// Titre de page, en Space Grotesk.
pub fn title<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
    text(content)
        .font(typography::TITLE)
        .size(typography::TITLE_SIZE)
}

/// Étiquette de section : petites capitales grises.
pub fn section_label<'a>(content: &str) -> Text<'a> {
    text(content.to_uppercase())
        .font(typography::UI_SEMIBOLD)
        .size(typography::SECTION_LABEL_SIZE)
        .color(color::TEXT_TERTIARY)
}

/// Icône au trait, à la couleur donnée.
pub fn icon<'a>(icon: Icon, size: f32, tint: Color) -> Svg<'a> {
    svg(svg::Handle::from_memory(icon.svg().into_bytes()))
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

/// Bouton principal, vert.
pub fn primary_button<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    label_button(label, typography::UI_SEMIBOLD, theme::primary, on_press)
}

/// Bouton secondaire, bordé.
pub fn secondary_button<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Option<Message>,
) -> Control<'a, Message> {
    label_button(label, typography::UI, theme::secondary, on_press)
}

/// Bouton d'icône seule. `label` est obligatoire : c'est son nom pour qui ne voit pas l'icône.
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

/// Interrupteur de 44 par 26. `on_toggle` reçoit la nouvelle valeur.
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

/// Ce qui suit le libellé d'une entrée de menu, à droite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trailing<'a> {
    None,
    /// Raccourci clavier, comme « Super+M ».
    Shortcut(&'a str),
    /// Ouvre un sous-menu, à droite.
    Submenu,
    /// Choix actif.
    Checked,
}

/// Entrée de menu de 30 de haut, sur toute la largeur du menu.
pub fn menu_item<'a, Message: Clone + 'a>(
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
    let line = row![body(label), space::horizontal(), end]
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

/// Marque une entrée de menu comme ouverte : son sous-menu est affiché.
pub fn open<'a, Message: Clone + 'a>(mut item: Control<'a, Message>) -> Control<'a, Message> {
    item.style = Box::new(theme::menu_item_open);
    item
}

/// Trait entre deux groupes d'entrées de menu.
pub fn menu_separator<'a, Message: 'a>() -> Element<'a, Message> {
    let (top, side) = metrics::MENU_SEPARATOR_MARGIN;
    let line = container(space())
        .width(Length::Fill)
        .height(metrics::BORDER_WIDTH)
        .style(theme::separator);
    container(line).padding(Padding::from([top, side])).into()
}

/// Menu ou sous-menu : verre des menus, entrées en colonne.
pub fn menu<'a, Message: 'a>(
    entries: impl IntoIterator<Item = Element<'a, Message>>,
    width: f32,
) -> Container<'a, Message> {
    container(iced::widget::Column::with_children(entries))
        .width(width)
        .padding(metrics::MENU_PADDING)
        .style(theme::menu)
}

/// Champ de texte de 32 de haut. Sans `on_input`, il est désactivé.
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

/// Trait entre deux lignes d'une carte.
pub fn divider<'a, Message: 'a>() -> Element<'a, Message> {
    container(space())
        .width(Length::Fill)
        .height(metrics::BORDER_WIDTH)
        .style(theme::divider)
        .into()
}

/// Ligne de réglage de la planche Réglages : libellé, aide en dessous, contrôle à droite.
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

/// Carte posée dans une fenêtre.
pub fn card<'a, Message: 'a>(content: impl Into<Element<'a, Message>>) -> Container<'a, Message> {
    let (vertical, horizontal) = metrics::CARD_PADDING;
    container(content)
        .padding([vertical, horizontal])
        .style(theme::card)
}

/// Panneau de verre flottant.
pub fn glass_panel<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Container<'a, Message> {
    container(content)
        .padding(metrics::PANEL_PADDING)
        .style(theme::glass)
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
