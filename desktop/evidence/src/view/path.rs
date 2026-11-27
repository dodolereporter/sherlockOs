//! The path bar (lot E-04): the breadcrumb fitted to the window.

use iced::widget::{Row, container};
use iced::{Alignment, Element, Length};

use sherlock_ui::component as ui;
use sherlock_ui::ellipsis;
use sherlock_ui::evidence as more;
use sherlock_ui::icon::Icon;
use sherlock_ui::texts::evidence as t;
use sherlock_ui::theme;
use sherlock_ui::{color, metrics, typography};

use crate::app::{App, Message};
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
        let mut right = metrics::GAP_ICON + metrics::SEARCH_WIDTH + metrics::TOOLBAR_PADDING;
        if let Some(branch) = self.repo.as_ref().and_then(|r| r.branch.as_deref()) {
            let (_, padding) = metrics::BRANCH_CHIP_PADDING;
            let label = t::branch(branch);
            right += metrics::GAP_ICON
                + 2.0 * padding
                + ellipsis::width(&label, typography::CODE, typography::CODE_SMALL_SIZE);
        }
        (left, (self.size.width - left - right).max(0.0))
    }

    /// The bar: the breadcrumb.
    pub(super) fn path_bar(&self) -> Element<'_, Message> {
        self.trail()
    }

    /// Segments of the path, each opening its folder. Long names keep their start and their
    /// end, and the segments after the first fold behind « … » until the others fit the bar.
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
        bar.into()
    }
}
