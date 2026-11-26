//! Where the windows go in Mission Control: a balanced grid that keeps their proportions. Every
//! window takes the same scale, so that their sizes compare as on the desktop; the rows follow
//! their height on the screen and each row their order from left to right, so that each window
//! travels as little as it can. Pure computation, on rectangles of the space.

use smithay::utils::{Logical, Rectangle};

pub type RectF = Rectangle<f64, Logical>;

/// Spaces of the grid, in logical pixels.
#[derive(Debug, Clone, Copy)]
pub struct Spacing {
    /// Between two windows, side by side and from one row to the next.
    pub gap: f64,
    /// Below each window, for its title.
    pub label: f64,
    /// Largest scale of a window.
    pub max_scale: f64,
}

fn center(r: RectF) -> (f64, f64) {
    (r.loc.x + r.size.w / 2.0, r.loc.y + r.size.h / 2.0)
}

/// The places in `area` of the windows whose frames are `windows`, in the same order. They never
/// overlap, title below each included, and stay inside `area` (unless it is too small for even
/// a pixel of each).
pub fn arrange(windows: &[RectF], area: RectF, spacing: Spacing) -> Vec<RectF> {
    let n = windows.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| center(windows[a]).1.total_cmp(&center(windows[b]).1));
    // The number of rows that gives the windows the largest scale; the fewest rows on a tie.
    let mut best: Option<(f64, Vec<Vec<usize>>)> = None;
    for count in 1..=n {
        let rows = split(&order, count, windows);
        let scale = fit(&rows, windows, area, spacing);
        if best.as_ref().is_none_or(|(s, _)| scale > *s + 1e-9) {
            best = Some((scale, rows));
        }
    }
    let Some((scale, rows)) = best else {
        return Vec::new();
    };
    let row_height = |row: &[usize]| {
        let tallest = row.iter().map(|&i| windows[i].size.h).fold(0.0, f64::max);
        tallest * scale
    };
    let total: f64 = rows
        .iter()
        .map(|row| row_height(row) + spacing.label)
        .sum::<f64>()
        + spacing.gap * (rows.len() as f64 - 1.0);
    let mut slots = vec![RectF::default(); n];
    let mut y = area.loc.y + (area.size.h - total) / 2.0;
    for row in &rows {
        let width: f64 = row.iter().map(|&i| windows[i].size.w * scale).sum::<f64>()
            + spacing.gap * (row.len() as f64 - 1.0);
        let height = row_height(row);
        let mut x = area.loc.x + (area.size.w - width) / 2.0;
        for &i in row {
            let size = windows[i].size.to_f64().upscale(scale);
            let top = y + (height - size.h) / 2.0;
            slots[i] = Rectangle::new((x, top).into(), size);
            x += size.w + spacing.gap;
        }
        y += height + spacing.label + spacing.gap;
    }
    slots
}

/// `order` (by height on the screen) cut into `count` rows as even as possible, the first
/// rows taking one more, each row from left to right.
fn split(order: &[usize], count: usize, windows: &[RectF]) -> Vec<Vec<usize>> {
    let (base, extra) = (order.len() / count, order.len() % count);
    let mut rest = order;
    (0..count)
        .map(|r| {
            let (row, tail) = rest.split_at(base + usize::from(r < extra));
            rest = tail;
            let mut row = row.to_vec();
            row.sort_by(|&a, &b| center(windows[a]).0.total_cmp(&center(windows[b]).0));
            row
        })
        .collect()
}

/// The largest scale at which `rows` fit in `area`.
fn fit(rows: &[Vec<usize>], windows: &[RectF], area: RectF, spacing: Spacing) -> f64 {
    let gaps = |k: usize| spacing.gap * (k as f64 - 1.0);
    let mut scale = spacing.max_scale;
    let mut heights = 0.0;
    for row in rows {
        let width: f64 = row.iter().map(|&i| windows[i].size.w).sum();
        scale = scale.min((area.size.w - gaps(row.len())) / width.max(1.0));
        heights += row.iter().map(|&i| windows[i].size.h).fold(0.0, f64::max);
    }
    let room = area.size.h - gaps(rows.len()) - spacing.label * rows.len() as f64;
    scale.min(room / heights.max(1.0)).max(0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACING: Spacing = Spacing {
        gap: 24.0,
        label: 32.0,
        max_scale: 0.8,
    };

    fn rect(x: f64, y: f64, w: f64, h: f64) -> RectF {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// Windows of every size, spread over a screen of 1440 × 900 and piled up in its middle.
    fn windows(n: usize) -> Vec<RectF> {
        (0..n)
            .map(|i| {
                let i = i as f64;
                let (w, h) = (300.0 + (i * 137.0) % 700.0, 200.0 + (i * 89.0) % 500.0);
                rect((i * 211.0) % 900.0, (i * 97.0) % 400.0, w, h)
            })
            .collect()
    }

    /// The slots overlap nowhere, titles included, stay in `area` and keep the proportions
    /// and the relative sizes of the windows.
    fn check(windows: &[RectF], area: RectF) -> Vec<RectF> {
        let slots = arrange(windows, area, SPACING);
        assert_eq!(slots.len(), windows.len());
        let with_label = |r: RectF| rect(r.loc.x, r.loc.y, r.size.w, r.size.h + SPACING.label);
        for (i, a) in slots.iter().enumerate() {
            let inside = area.loc.x - 1e-6 <= a.loc.x
                && area.loc.y - 1e-6 <= a.loc.y
                && a.loc.x + a.size.w <= area.loc.x + area.size.w + 1e-6
                && a.loc.y + a.size.h + SPACING.label <= area.loc.y + area.size.h + 1e-6;
            assert!(inside, "slot {i} {a:?} out of {area:?}");
            for b in &slots[i + 1..] {
                let (a, b) = (with_label(*a), with_label(*b));
                let apart = a.loc.x + a.size.w <= b.loc.x + 1e-6
                    || b.loc.x + b.size.w <= a.loc.x + 1e-6
                    || a.loc.y + a.size.h <= b.loc.y + 1e-6
                    || b.loc.y + b.size.h <= a.loc.y + 1e-6;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        let scale = slots[0].size.w / windows[0].size.w;
        assert!(scale <= SPACING.max_scale + 1e-9);
        for (slot, window) in slots.iter().zip(windows) {
            assert!(
                (slot.size.w / window.size.w - scale).abs() < 1e-9,
                "one scale"
            );
            assert!(
                (slot.size.h / window.size.h - scale).abs() < 1e-9,
                "proportions"
            );
        }
        slots
    }

    #[test]
    fn one_two_five_and_twelve_windows_never_overlap() {
        let area = rect(32.0, 180.0, 1376.0, 600.0);
        for n in [1, 2, 5, 12] {
            check(&windows(n), area);
        }
        // Many on a small screen: still apart, only smaller.
        check(&windows(12), rect(0.0, 0.0, 400.0, 300.0));
        assert!(arrange(&[], area, SPACING).is_empty());
    }

    #[test]
    fn a_window_alone_shrinks_a_little_and_stays_centered() {
        let area = rect(0.0, 0.0, 1440.0, 900.0);
        let slot = check(&[rect(100.0, 100.0, 800.0, 600.0)], area)[0];
        assert_eq!(slot.size, (640.0, 480.0).into());
        assert_eq!(slot.loc.x, (1440.0 - 640.0) / 2.0);
        assert_eq!(slot.loc.y, (900.0 - 480.0 - SPACING.label) / 2.0);
    }

    #[test]
    fn the_grid_keeps_the_order_of_the_desktop() {
        let area = rect(0.0, 0.0, 1440.0, 900.0);
        // Two windows side by side: the left one stays on the left, whatever the stack.
        let pair = [
            rect(800.0, 100.0, 600.0, 400.0),
            rect(0.0, 120.0, 600.0, 400.0),
        ];
        let slots = check(&pair, area);
        assert!(slots[1].loc.x < slots[0].loc.x);
        // Four in the corners: two rows, the top ones above, each row left to right.
        let corners = [
            rect(800.0, 500.0, 600.0, 380.0),
            rect(0.0, 0.0, 600.0, 380.0),
            rect(0.0, 500.0, 600.0, 380.0),
            rect(800.0, 0.0, 600.0, 380.0),
        ];
        let slots = check(&corners, area);
        let (top_left, top_right) = (slots[1], slots[3]);
        let (bottom_left, bottom_right) = (slots[2], slots[0]);
        assert_eq!(top_left.loc.y, top_right.loc.y);
        assert!(top_left.loc.x < top_right.loc.x && bottom_left.loc.x < bottom_right.loc.x);
        assert!(top_left.loc.y < bottom_left.loc.y);
    }
}
