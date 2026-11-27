//! Scrolling the page to a setting the search opened (audit C11): the row of each setting
//! carries an id; an operation finds it and the page's scrollable in the laid out window, and
//! gives where the page is and where it should go so that the row shows, a little below the
//! top. The application then scrolls there on a spring.

use iced::advanced::widget::operation::{Outcome, Scrollable};
use iced::advanced::widget::{self, Id, Operation};
use iced::{Rectangle, Task, Vector};
use sherlock_ui::metrics;

use crate::search::Setting;

/// The id of the page's scrollable.
pub const PAGE: &str = "page";

/// The id of a setting's row.
pub fn row(setting: Setting) -> Id {
    Id::from(format!("setting-{setting:?}"))
}

/// Where the page is scrolled and where it should be for `setting` to show: `None` when the
/// row or the page are not there, or the row shows already.
pub fn find(setting: Setting) -> Task<Option<(f64, f64)>> {
    widget::operate(Find {
        row: row(setting),
        page: Id::new(PAGE),
        scrollable: None,
        found: None,
    })
}

struct Find {
    row: Id,
    page: Id,
    /// The page: its visible bounds, its content's, and how far it is scrolled.
    scrollable: Option<(Rectangle, Rectangle, Vector)>,
    /// The row, where it is laid out in the content.
    found: Option<Rectangle>,
}

impl Operation<Option<(f64, f64)>> for Find {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<(f64, f64)>>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.row) {
            self.found = Some(bounds);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&self.page) {
            self.scrollable = Some((bounds, content_bounds, translation));
        }
    }

    fn finish(&self) -> Outcome<Option<(f64, f64)>> {
        Outcome::Some(target(self.scrollable, self.found))
    }
}

/// From where the page is to where the row shows, a section's gap below the top; nothing
/// when the row shows whole already.
fn target(
    scrollable: Option<(Rectangle, Rectangle, Vector)>,
    row: Option<Rectangle>,
) -> Option<(f64, f64)> {
    let ((bounds, content, translation), row) = (scrollable?, row?);
    let now = translation.y;
    let top = row.y - content.y;
    let shown = top >= now && top + row.height <= now + bounds.height;
    if shown {
        return None;
    }
    let most = (content.height - bounds.height).max(0.0);
    let to = (top - metrics::GAP_SECTION).clamp(0.0, most);
    Some((f64::from(now), f64::from(to)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(y: f32, height: f32) -> Rectangle {
        Rectangle {
            x: 0.0,
            y,
            width: 100.0,
            height,
        }
    }

    #[test]
    fn a_row_below_the_view_is_scrolled_to() {
        let page = (rect(50.0, 400.0), rect(50.0, 1500.0), Vector::new(0.0, 0.0));
        let (from, to) = target(Some(page), Some(rect(1050.0, 48.0))).unwrap();
        assert_eq!(from, 0.0);
        assert_eq!(to, f64::from(1000.0 - metrics::GAP_SECTION));
        // Shown already: nothing moves.
        assert_eq!(target(Some(page), Some(rect(100.0, 48.0))), None);
        // Near the end: no further than the page goes.
        let (_, to) = target(Some(page), Some(rect(1500.0, 48.0))).unwrap();
        assert_eq!(to, 1100.0);
        assert_eq!(target(None, Some(rect(0.0, 1.0))), None);
    }
}
