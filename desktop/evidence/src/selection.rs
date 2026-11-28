//! The selection of a listing, as in Finder and Dolphin: the selected items, the cursor (the
//! item the keyboard is on) and the anchor that ranges start from. Pure logic, without iced:
//! items are positions in the listing.

use std::collections::BTreeSet;
use std::ops::RangeInclusive;

/// How a click combines with the selection, from the modifiers held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// Selects the item alone.
    Only,
    /// Ctrl: adds or removes the item.
    Toggle,
    /// Shift: selects the range from the anchor, alone.
    Range,
    /// Ctrl+Shift: adds the range from the anchor.
    AddRange,
}

impl Click {
    pub fn new(ctrl: bool, shift: bool) -> Self {
        match (ctrl, shift) {
            (false, false) => Self::Only,
            (true, false) => Self::Toggle,
            (false, true) => Self::Range,
            (true, true) => Self::AddRange,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    items: BTreeSet<usize>,
    cursor: Option<usize>,
    anchor: Option<usize>,
}

impl Selection {
    pub fn contains(&self, index: usize) -> bool {
        self.items.contains(&index)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.items.iter().copied()
    }

    /// The item the keyboard is on.
    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    /// The item an action on one item acts on: the cursor when it is selected, else the
    /// first selected item. Never an item that is not selected: Ctrl+click can leave the
    /// cursor on the item it just removed.
    pub fn focus(&self) -> Option<usize> {
        self.cursor
            .filter(|c| self.items.contains(c))
            .or_else(|| self.items.first().copied())
    }

    /// Puts the cursor on `index` if it is selected, the selection unchanged: a right click
    /// on a selected item.
    pub fn point(&mut self, index: usize) {
        if self.items.contains(&index) {
            self.cursor = Some(index);
        }
    }

    /// The selected item, when it is alone.
    pub fn single(&self) -> Option<usize> {
        match self.items.len() {
            1 => self.items.first().copied(),
            _ => None,
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Selects `index` alone; ranges start from it.
    pub fn only(&mut self, index: usize) {
        self.items = BTreeSet::from([index]);
        self.cursor = Some(index);
        self.anchor = Some(index);
    }

    /// Selects these items, the first one under the cursor, as after a reload.
    pub fn set(&mut self, items: impl IntoIterator<Item = usize>) {
        self.items = items.into_iter().collect();
        self.cursor = self.items.first().copied();
        self.anchor = self.cursor;
    }

    /// Every item of a listing of `count`.
    pub fn all(&mut self, count: usize) {
        self.items = (0..count).collect();
        self.cursor = self
            .cursor
            .filter(|&c| c < count)
            .or(self.items.first().copied());
        self.anchor = self.anchor.filter(|&a| a < count).or(self.cursor);
    }

    pub fn click(&mut self, index: usize, how: Click) {
        let anchor = self.anchor.unwrap_or(index);
        match how {
            Click::Only => return self.only(index),
            Click::Toggle => {
                if !self.items.remove(&index) {
                    self.items.insert(index);
                }
                self.anchor = Some(index);
            }
            Click::Range => {
                self.items = range(anchor, index).collect();
                self.anchor = Some(anchor);
            }
            Click::AddRange => {
                self.items.extend(range(anchor, index));
                self.anchor = Some(anchor);
            }
        }
        self.cursor = Some(index);
    }

    /// Shift and an arrow: the range from the anchor to `index`, the cursor on `index`.
    pub fn extend_to(&mut self, index: usize) {
        self.click(index, Click::Range);
    }

    /// The selected items that follow each other, as blocks, in order.
    pub fn runs(&self) -> Vec<RangeInclusive<usize>> {
        let mut runs: Vec<RangeInclusive<usize>> = Vec::new();
        for &i in &self.items {
            match runs.last_mut() {
                Some(run) if *run.end() + 1 == i => *run = *run.start()..=i,
                _ => runs.push(i..=i),
            }
        }
        runs
    }
}

fn range(a: usize, b: usize) -> RangeInclusive<usize> {
    a.min(b)..=a.max(b)
}

/// A rectangle drawn by dragging on the background of the listing, in the coordinates of the
/// listing's content (scroll included): it selects the rows it crosses.
#[derive(Debug, Clone, PartialEq)]
pub struct Lasso {
    pub start: (f32, f32),
    pub end: (f32, f32),
    /// What was selected when the drag started, kept with Ctrl or Shift held.
    pub base: Selection,
    /// The pointer went far enough from the press to make it a drag.
    pub dragging: bool,
}

impl Lasso {
    pub fn new(at: (f32, f32), base: Selection) -> Self {
        Self {
            start: at,
            end: at,
            base,
            dragging: false,
        }
    }

    /// Left, top, width and height.
    pub fn rect(&self) -> (f32, f32, f32, f32) {
        let (x0, y0) = self.start;
        let (x1, y1) = self.end;
        (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
    }

    /// The selection with the rows of a listing of `count` it crosses: rows of `row` pixels
    /// that start `top` pixels below the top of the content.
    pub fn selection(&self, top: f32, row: f32, count: usize) -> Selection {
        let mut selection = self.base.clone();
        let (_, y, _, height) = self.rect();
        let first = ((y - top) / row).floor().max(0.0) as usize;
        let last = ((y + height - top) / row).floor();
        if count == 0 || last < 0.0 || first >= count {
            return selection;
        }
        let last = (last as usize).min(count - 1);
        selection.items.extend(first..=last);
        selection.cursor = Some(if self.end.1 < self.start.1 {
            first
        } else {
            last
        });
        selection.anchor = selection.anchor.or(Some(if self.end.1 < self.start.1 {
            last
        } else {
            first
        }));
        selection
    }
}

impl Lasso {
    /// The selection with the items `crossed` (in order) added: a grid's tiles under the
    /// lasso (lot E-13). The cursor goes to the end the lasso is dragged towards.
    pub fn selection_with(&self, crossed: &[usize]) -> Selection {
        let mut selection = self.base.clone();
        let (Some(&first), Some(&last)) = (crossed.first(), crossed.last()) else {
            return selection;
        };
        selection.items.extend(crossed.iter().copied());
        let up = self.end.1 < self.start.1;
        selection.cursor = Some(if up { first } else { last });
        selection.anchor = selection.anchor.or(Some(if up { last } else { first }));
        selection
    }
}

/// How fast a lasso held near an edge of the listing scrolls it, in pixels per second: from 0
/// at `margin` pixels from the edge to `max` beyond it, negative upwards. `y` is the pointer
/// in the visible part, `height` that part's height.
pub fn edge_scroll(y: f32, height: f32, margin: f32, max: f32) -> f32 {
    let depth = if y < margin {
        y - margin
    } else if y > height - margin {
        y - (height - margin)
    } else {
        return 0.0;
    };
    (depth / margin).clamp(-1.0, 1.0) * max
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(s: &Selection) -> Vec<usize> {
        s.iter().collect()
    }

    #[test]
    fn a_click_selects_one_item_and_sets_the_anchor() {
        let mut s = Selection::default();
        s.click(3, Click::Only);
        s.click(5, Click::Only);
        assert_eq!(
            (items(&s), s.cursor(), s.single()),
            (vec![5], Some(5), Some(5))
        );
        s.click(2, Click::Range);
        assert_eq!(items(&s), [2, 3, 4, 5]);
    }

    #[test]
    fn ctrl_adds_and_removes_and_moves_the_anchor() {
        let mut s = Selection::default();
        s.click(1, Click::Only);
        s.click(4, Click::Toggle);
        assert_eq!(items(&s), [1, 4]);
        s.click(1, Click::Toggle);
        assert_eq!(items(&s), [4]);
        // The anchor is the last item clicked with Ctrl, even a removed one.
        assert_eq!(s.single(), Some(4));
        s.click(6, Click::Range);
        assert_eq!(items(&s), [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn the_focus_is_always_a_selected_item() {
        let mut s = Selection::default();
        s.click(1, Click::Only);
        s.click(4, Click::Toggle);
        s.click(4, Click::Toggle);
        assert_eq!((s.cursor(), s.focus()), (Some(4), Some(1)));
        s.point(4);
        assert_eq!(s.cursor(), Some(4), "not selected: the cursor stays");
        s.click(6, Click::Toggle);
        s.point(1);
        assert_eq!(s.focus(), Some(1));
        assert_eq!(Selection::default().focus(), None);
    }

    #[test]
    fn shift_ranges_start_from_the_anchor_and_replace_each_other() {
        let mut s = Selection::default();
        s.click(5, Click::Only);
        s.click(8, Click::Range);
        assert_eq!(items(&s), [5, 6, 7, 8]);
        // A second Shift+click starts from the same anchor: the range shrinks or flips.
        s.click(3, Click::Range);
        assert_eq!(items(&s), [3, 4, 5]);
        assert_eq!(s.cursor(), Some(3));
        // Without an anchor, Shift+click selects the item alone.
        let mut fresh = Selection::default();
        fresh.click(2, Click::Range);
        assert_eq!(items(&fresh), [2]);
    }

    #[test]
    fn ctrl_shift_adds_a_range_to_the_selection() {
        let mut s = Selection::default();
        s.click(0, Click::Only);
        s.click(6, Click::Toggle);
        s.click(8, Click::AddRange);
        assert_eq!(items(&s), [0, 6, 7, 8]);
        assert_eq!(Click::new(true, true), Click::AddRange);
        assert_eq!(Click::new(false, true), Click::Range);
    }

    #[test]
    fn shift_and_arrows_extend_then_shrink() {
        let mut s = Selection::default();
        s.only(4);
        s.extend_to(5);
        s.extend_to(6);
        assert_eq!(items(&s), [4, 5, 6]);
        s.extend_to(5);
        s.extend_to(4);
        s.extend_to(3);
        assert_eq!(items(&s), [3, 4]);
    }

    #[test]
    fn select_all_keeps_the_cursor() {
        let mut s = Selection::default();
        s.only(2);
        s.all(5);
        assert_eq!((s.len(), s.cursor()), (5, Some(2)));
        s.clear();
        assert!(s.is_empty() && s.cursor().is_none());
    }

    #[test]
    fn runs_are_the_blocks_of_following_items() {
        let mut s = Selection::default();
        s.set([1, 2, 3, 5, 8, 9]);
        assert_eq!(s.runs(), [1..=3, 5..=5, 8..=9]);
        assert!(Selection::default().runs().is_empty());
    }

    #[test]
    fn a_lasso_selects_the_rows_it_crosses() {
        // Rows of 40 below a gap of 4: row 2 spans 84 to 124.
        let mut lasso = Lasso::new((10.0, 90.0), Selection::default());
        lasso.end = (200.0, 170.0);
        assert_eq!(items(&lasso.selection(4.0, 40.0, 10)), [2, 3, 4]);
        // Upwards, past the top: from the first row.
        lasso.end = (200.0, -30.0);
        let s = lasso.selection(4.0, 40.0, 10);
        assert_eq!((items(&s), s.cursor()), (vec![0, 1, 2], Some(0)));
        // Below the last row: up to it.
        lasso.end = (200.0, 2000.0);
        assert_eq!(lasso.selection(4.0, 40.0, 4).len(), 2);
        // Entirely below the rows: nothing.
        let below = Lasso::new((0.0, 500.0), Selection::default());
        assert!(below.selection(4.0, 40.0, 4).is_empty());
    }

    #[test]
    fn a_lasso_with_ctrl_keeps_what_was_selected() {
        let mut base = Selection::default();
        base.only(8);
        let mut lasso = Lasso::new((0.0, 5.0), base);
        lasso.end = (0.0, 50.0);
        assert_eq!(items(&lasso.selection(4.0, 40.0, 10)), [0, 1, 8]);
        assert_eq!(lasso.rect(), (0.0, 5.0, 0.0, 45.0));
    }

    #[test]
    fn the_edges_scroll_faster_the_deeper_the_pointer() {
        assert_eq!(edge_scroll(200.0, 400.0, 40.0, 20.0), 0.0);
        assert_eq!(edge_scroll(20.0, 400.0, 40.0, 20.0), -10.0);
        assert_eq!(edge_scroll(-100.0, 400.0, 40.0, 20.0), -20.0);
        assert_eq!(edge_scroll(390.0, 400.0, 40.0, 20.0), 15.0);
        assert_eq!(edge_scroll(900.0, 400.0, 40.0, 20.0), 20.0);
    }
}
