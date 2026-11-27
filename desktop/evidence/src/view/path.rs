//! The path bar (lot E-04): the breadcrumb fitted to the window, the field that takes its
//! place, the completion list, and the « Aller au dossier » sheet.

use iced::widget::{
    Column, Row, button, column, container, mouse_area, opaque, pin, row, scrollable, space, stack,
};
use iced::{Alignment, Element, Length, Padding, mouse};

use sherlock_ui::animated::reveal;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::ellipsis;
use sherlock_ui::evidence as more;
use sherlock_ui::icon::Icon;
use sherlock_ui::texts::evidence as t;
use sherlock_ui::theme;
use sherlock_ui::{color, metrics, typography};

use crate::app::{App, Location, Message, PATH, PathEdit, SUGGESTIONS};
use crate::pathbar;

/// The labels of the breadcrumb and their widths, the folders they open, and how many fold.
type TrailParts = (Vec<(String, f32)>, Vec<(String, Location)>, usize);

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
        let search = if self.search_folded() {
            metrics::ICON_BUTTON
        } else {
            metrics::SEARCH_WIDTH
        };
        let mut right = metrics::GAP_ICON + search + metrics::TOOLBAR_PADDING;
        if let Some(branch) = self.repo.as_ref().and_then(|r| r.branch.as_deref()) {
            let (_, padding) = metrics::BRANCH_CHIP_PADDING;
            let label = t::branch(branch);
            right += metrics::GAP_ICON
                + 2.0 * padding
                + ellipsis::width(&label, typography::CODE, typography::CODE_SMALL_SIZE);
        }
        (left, (self.size.width - left - right).max(0.0))
    }

    /// In a narrow window, the search shows as its icon until it is used.
    pub(crate) fn search_folded(&self) -> bool {
        self.size.width < more::SEARCH_COMPACT_WIDTH && !self.search_open && self.search.is_empty()
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
    /// The segments as drawn: their labels and widths, the folders they open, and how many
    /// fold behind « … » after the first.
    fn trail_parts(&self) -> TrailParts {
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
        (labels, crumbs, folded)
    }

    fn trail(&self) -> Element<'_, Message> {
        let (labels, crumbs, folded) = self.trail_parts();
        let count = crumbs.len();
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
        for (i, ((label, _), (_, location))) in labels.into_iter().zip(crumbs).enumerate() {
            if (1..=folded).contains(&i) {
                if i == 1 {
                    // « … » lists the folded folders, as Finder's path bar does.
                    let open = !self.folded.open;
                    let more = ui::crumb(t::FOLDED.into(), false, Some(Message::Folded(open)));
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
        let list = scrollable(Column::with_children(rows))
            .id(SUGGESTIONS)
            .on_scroll(|v| Message::PathScrolled(v.absolute_offset().y))
            .height(shown * metrics::MENU_ITEM_HEIGHT);
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

    /// The folders folded behind « … », in a menu under it, fading in and out.
    pub(super) fn folded_layer(&self) -> Option<Element<'_, Message>> {
        let level = self.folded.level(self.now());
        if level <= 0.0 || self.folded.items.is_empty() {
            return None;
        }
        let live = self.folded.open;
        let rows = self.folded.items.iter().map(|(label, location)| {
            let icon = ui::icon(
                Icon::Folder,
                metrics::ICON_SIZE_SMALL,
                color::palette().blue,
            );
            let go = live.then(|| Message::Go(location.clone()));
            ui::menu_item_with(Some(icon.into()), label.as_str(), Trailing::None, go).into()
        });
        let menu = ui::floating_menu(rows, metrics::CONTEXT_SUBMENU_WIDTH);
        let slide = (1.0 - level) * more::COMPLETION_SLIDE;
        let menu = reveal(menu).opacity(level).offset(0.0, -slide);
        let top = metrics::TITLE_BAR_HEIGHT
            + metrics::BORDER_WIDTH
            + (metrics::TOOLBAR_HEIGHT + metrics::BREADCRUMB_HEIGHT) / 2.0
            + metrics::MENU_PADDING;
        let menu = pin(menu).x(self.folded.x).y(top);
        if !live {
            return Some(Element::from(menu).map(|_| Message::Inert));
        }
        let catcher = mouse_area(space().width(Length::Fill).height(Length::Fill))
            .on_press(Message::Folded(false));
        Some(stack![opaque(catcher), menu].into())
    }

    /// Where « … » starts in the window, and the folders it folds: what its menu lists.
    pub(crate) fn folded_items(&self) -> (f32, Vec<(String, Location)>) {
        let (labels, crumbs, folded) = self.trail_parts();
        let (left, _) = self.path_bar_span();
        let first = labels.first().map_or(0.0, |(_, w)| *w);
        let chevron = metrics::ICON_SIZE_TINY + 2.0 * metrics::BREADCRUMB_GAP;
        let x = left + metrics::BREADCRUMB_PADDING + first + chevron;
        let items = crumbs.into_iter().skip(1).take(folded).collect();
        (x, items)
    }

    /// The last folders typed, under the sheet's field while nothing completes.
    fn recent_rows(&self, live: bool) -> Option<Element<'_, Message>> {
        if self.recent.is_empty() {
            return None;
        }
        let rows = self.recent.iter().enumerate().map(|(i, (_, label))| {
            let icon = ui::icon(
                Icon::Folder,
                metrics::ICON_SIZE_SMALL,
                color::palette().blue,
            );
            let go = live.then_some(Message::PathRecent(i));
            ui::menu_item_with(Some(icon.into()), label.as_str(), Trailing::None, go).into()
        });
        let list = column![
            ui::section_label(t::RECENT_FOLDERS),
            Column::with_children(rows)
        ];
        Some(list.spacing(metrics::GAP_TIGHT).into())
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
        } else if let Some(recent) = self.recent_rows(live) {
            // Nothing completes: the last folders typed, as macOS keeps them.
            card = card.push(recent);
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
