//! The path bar (lot E-04): the breadcrumb fitted to the window, the field that takes its
//! place, the completion list, and the « Aller au dossier » sheet.

use iced::widget::{
    Column, Row, button, column, container, mouse_area, opaque, pin, row, space, stack,
};
use iced::{Alignment, Element, Length, Padding, mouse};

use sherlock_ui::animated::reveal;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::ellipsis;
use sherlock_ui::evidence as more;
use sherlock_ui::icon::Icon;
use sherlock_ui::scroll::scroll;
use sherlock_ui::texts::evidence as t;
use sherlock_ui::theme;
use sherlock_ui::{color, metrics, typography};

use crate::app::{App, Message, PATH, PathEdit};
use crate::pathbar;

/// Width of a label of the breadcrumb, shaped as it is drawn.
fn label_width(label: &str, current: bool) -> f32 {
    let font = if current {
        typography::UI_SEMIBOLD
    } else {
        typography::UI
    };
    ellipsis::width(label, font, typography::UI_SIZE)
}

impl App {
    /// Left edge and width of the path bar in the toolbar, for this window.
    pub(crate) fn path_bar_span(&self) -> (f32, f32) {
        let left = metrics::TOOLBAR_PADDING + 2.0 * (metrics::ICON_BUTTON + metrics::GAP_ICON);
        // The search, then the preview's button (lot E-08).
        let mut right = 2.0 * metrics::GAP_ICON
            + metrics::SEARCH_WIDTH
            + metrics::ICON_BUTTON
            + metrics::TOOLBAR_PADDING;
        if let Some(branch) = self.repo.as_ref().and_then(|r| r.branch.as_deref()) {
            let (_, padding) = metrics::BRANCH_CHIP_PADDING;
            let label = t::branch(branch);
            right += metrics::GAP_ICON
                + 2.0 * padding
                + ellipsis::width(&label, typography::CODE, typography::CODE_SMALL_SIZE);
        }
        (left, (self.size.width - left - right).max(0.0))
    }

    /// The bar: the breadcrumb, the field coming in or going out over it.
    pub(super) fn path_bar(&self) -> Element<'_, Message> {
        let level = self.path_level();
        let edit = (self.path_edit.as_ref())
            .or(self.path_leaving.as_ref())
            .filter(|e| !e.sheet);
        let editing = self.path_edit.as_ref().is_some_and(|e| !e.sheet);
        let (crumbs, field) = match edit {
            Some(edit) => (1.0 - level, Some(edit)),
            None => (1.0, None),
        };
        let mut trail = reveal(self.trail()).opacity(crumbs);
        if editing {
            trail = trail.inert();
        }
        let field: Element<'_, Message> = match field {
            Some(edit) => {
                let field = reveal(self.path_field(edit)).opacity(level);
                if editing {
                    field.into()
                } else {
                    field.inert().into()
                }
            }
            None => space().into(),
        };
        stack![trail, field]
            .width(Length::Fill)
            .height(metrics::BREADCRUMB_HEIGHT)
            .into()
    }

    /// The field, shaking when the path led nowhere.
    fn path_field<'a>(&'a self, edit: &'a PathEdit) -> Element<'a, Message> {
        let field = ui::text_field(t::PATH_PLACEHOLDER, &edit.text, Some(Message::PathInput))
            .id(PATH)
            .on_submit(Message::PathSubmit)
            .width(Length::Fill);
        reveal(field)
            .offset(edit.shake_offset(self.now()), 0.0)
            .into()
    }

    /// Segments of the path, each opening its folder; the empty part of the bar starts the
    /// field. Long names keep their start and their end, and the segments after the first
    /// fold behind « … » until the others fit the bar.
    fn trail(&self) -> Element<'_, Message> {
        let crumbs = self.breadcrumb();
        let count = crumbs.len();
        let (_, padding) = metrics::CRUMB_PADDING;
        let room_label = more::CRUMB_MAX_WIDTH - 2.0 * padding;
        let labels: Vec<(String, f32)> = crumbs
            .iter()
            .enumerate()
            .map(|(i, (label, _))| {
                let current = i + 1 == count;
                let shown = pathbar::visible(label);
                let shown = ellipsis::fit_middle(&shown, room_label, |s| label_width(s, current))
                    .into_owned();
                let width = label_width(&shown, current) + 2.0 * padding;
                (shown, width)
            })
            .collect();
        let chevron = metrics::ICON_SIZE_TINY + 2.0 * metrics::BREADCRUMB_GAP;
        let fold_width = label_width(t::FOLDED, false) + 2.0 * padding + chevron;
        let (_, span) = self.path_bar_span();
        let widths: Vec<f32> = labels.iter().map(|(_, w)| *w).collect();
        let folded = pathbar::fold(
            &widths,
            chevron,
            fold_width,
            span - 2.0 * metrics::BREADCRUMB_PADDING,
        );
        let chevron = || {
            ui::icon(
                Icon::Chevron,
                metrics::ICON_SIZE_TINY,
                color::palette().text_tertiary,
            )
        };
        let mut trail = Row::new()
            .spacing(metrics::BREADCRUMB_GAP)
            .align_y(Alignment::Center);
        // « … » opens the deepest folded folder.
        let deepest = crumbs.get(folded).map(|(_, location)| location.clone());
        for (i, ((label, _), (_, location))) in labels.into_iter().zip(crumbs).enumerate() {
            if (1..=folded).contains(&i) {
                if i == 1 {
                    let more = ui::crumb(t::FOLDED.into(), false, deepest.clone().map(Message::Go));
                    trail = trail.push(chevron()).push(more);
                }
                continue;
            }
            if i > 0 {
                trail = trail.push(chevron());
            }
            let current = i + 1 == count;
            trail = trail.push(ui::crumb(label, current, Some(Message::Go(location))));
        }
        let bar = container(trail)
            .width(Length::Fill)
            .height(metrics::BREADCRUMB_HEIGHT)
            .padding([0.0, metrics::BREADCRUMB_PADDING])
            .align_y(Alignment::Center)
            .clip(true)
            .style(theme::inset);
        mouse_area(bar)
            .on_press(Message::Run(crate::app::Command::EditPath))
            .interaction(mouse::Interaction::Text)
            .into()
    }

    /// The folders that complete the name, one row each, the one picked lit.
    fn suggestion_rows<'a>(
        &'a self,
        edit: &'a PathEdit,
        live: bool,
    ) -> (Element<'a, Message>, usize) {
        let names = if edit.suggestions.is_empty() {
            &self.suggestions_shown
        } else {
            &edit.suggestions
        };
        let rows = names.iter().enumerate().map(|(i, name)| {
            let icon = ui::icon(
                Icon::Folder,
                metrics::ICON_SIZE_SMALL,
                color::palette().blue,
            );
            let message = live.then_some(Message::PathPick(i));
            let control =
                ui::menu_item_with(Some(icon.into()), name.as_str(), Trailing::None, message);
            if edit.highlight == Some(i) {
                control.status(button::Status::Hovered).into()
            } else {
                control.into()
            }
        });
        let shown = (names.len() as f32).min(more::COMPLETION_ROWS);
        let list = scroll(Column::with_children(rows)).height(shown * metrics::MENU_ITEM_HEIGHT);
        (list.into(), names.len())
    }

    /// How far the completion list has come in.
    fn suggest_level(&self, edit: &PathEdit) -> f32 {
        let shown = if edit.suggestions.is_empty() {
            0.0
        } else {
            1.0
        };
        self.suggest_fade
            .as_ref()
            .map_or(shown, |f| f.value(self.now()))
    }

    /// The completion list under the field of the toolbar, sliding in and out.
    pub(super) fn path_suggestions(&self) -> Option<Element<'_, Message>> {
        let live = self.path_edit.is_some();
        let edit = self.path_edit.as_ref().or(self.path_leaving.as_ref())?;
        let level = self.suggest_level(edit);
        if edit.sheet || level <= 0.0 {
            return None;
        }
        let (list, count) = self.suggestion_rows(edit, live);
        if count == 0 {
            return None;
        }
        let (left, span) = self.path_bar_span();
        let menu = ui::floating_menu([list], span.min(more::GO_TO_WIDTH));
        let slide = (1.0 - level) * more::COMPLETION_SLIDE;
        let menu = reveal(menu).opacity(level).offset(0.0, -slide);
        let menu: Element<'_, Message> = if live {
            menu.into()
        } else {
            menu.inert().into()
        };
        let top = metrics::TITLE_BAR_HEIGHT
            + metrics::BORDER_WIDTH
            + (metrics::TOOLBAR_HEIGHT + metrics::BREADCRUMB_HEIGHT) / 2.0
            + metrics::MENU_PADDING;
        Some(pin(menu).x(left).y(top).into())
    }

    /// « Aller au dossier »: a sheet that comes down under the toolbar, as on macOS.
    pub(super) fn go_to_sheet(&self) -> Option<Element<'_, Message>> {
        let live = self.path_edit.as_ref().is_some_and(|e| e.sheet);
        let edit = self.path_edit.as_ref().or(self.path_leaving.as_ref())?;
        let level = self.path_level();
        if !edit.sheet || level <= 0.0 {
            return None;
        }
        let width = more::GO_TO_WIDTH.min(self.size.width - 2.0 * more::CHOOSER_MARGIN);
        let mut card = column![
            ui::body(t::GO_TO_FOLDER)
                .font(typography::UI_SEMIBOLD)
                .size(typography::CARD_TITLE_SIZE),
            self.path_field(edit),
        ]
        .spacing(metrics::PREVIEW_GAP)
        .width(width);
        if self.suggest_level(edit) > 0.0 {
            let (list, count) = self.suggestion_rows(edit, live);
            if count > 0 {
                card = card.push(list);
            }
        }
        card = card.push(
            row![
                space::horizontal(),
                ui::secondary_button(t::CANCEL, Some(Message::PathCancel)),
                ui::primary_button(t::GO, Some(Message::PathSubmit)),
            ]
            .spacing(metrics::BUTTON_STACK_GAP),
        );
        let panel = ui::floating_panel(card, width + 2.0 * metrics::PANEL_PADDING);
        let rise = (1.0 - level) * more::CHOOSER_MARGIN;
        let sheet = reveal(opaque(panel)).opacity(level).offset(0.0, -rise);
        let top = metrics::TITLE_BAR_HEIGHT + metrics::TOOLBAR_HEIGHT + metrics::BORDER_WIDTH;
        let sheet = container(sheet)
            .center_x(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top,
                ..Padding::ZERO
            });
        // The window dims behind the sheet, and comes back as it leaves.
        let scrim = container(space())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::scrim);
        let catcher = mouse_area(reveal(scrim).opacity(level)).on_press(Message::PathCancel);
        let layer: Element<'_, Message> = stack![opaque(catcher), sheet].into();
        Some(if live {
            layer
        } else {
            layer.map(|_| Message::Inert)
        })
    }
}
