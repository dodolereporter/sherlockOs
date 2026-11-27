//! What the window shows at its width (lot E-05): the name of a file always keeps
//! `NAME_MIN_WIDTH`. When the room runs short, the Git column folds first, then Type, the
//! preview, Size, Modified, and last the sidebar, as the Finder and Nautilus give up their
//! panes. No iced here.

use sherlock_ui::evidence as more;
use sherlock_ui::metrics;

/// The columns after the name, in the order of `metrics::LIST_COLUMNS`.
pub const MODIFIED: usize = 0;
pub const SIZE: usize = 1;
pub const KIND: usize = 2;
pub const LAST: usize = 3;

/// The panes and columns shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panes {
    pub sidebar: bool,
    pub preview: bool,
    /// Modified (or Deleted), Size, Type, Git (or Origin).
    pub columns: [bool; 4],
}

/// What folds, in turn, while the name lacks room.
#[derive(Debug, Clone, Copy)]
enum Fold {
    Column(usize),
    Preview,
    Sidebar,
}

const ORDER: [Fold; 6] = [
    Fold::Column(LAST),
    Fold::Column(KIND),
    Fold::Preview,
    Fold::Column(SIZE),
    Fold::Column(MODIFIED),
    Fold::Sidebar,
];

impl Panes {
    /// The panes for a window `width` wide. `preview`: the preview is wanted (Space).
    /// `pinned`: it was asked for at this width, so it stays and the rest folds instead.
    pub fn at(width: f32, preview: bool, pinned: bool) -> Self {
        let mut panes = Self {
            sidebar: true,
            preview,
            columns: [true; 4],
        };
        for fold in ORDER {
            if panes.name_width(width) >= more::NAME_MIN_WIDTH {
                break;
            }
            match fold {
                Fold::Column(i) => panes.columns[i] = false,
                Fold::Preview if !pinned => panes.preview = false,
                Fold::Preview => {}
                Fold::Sidebar => panes.sidebar = false,
            }
        }
        panes
    }

    /// Width of the sidebar with its rule, as laid out.
    pub fn sidebar_width(&self) -> f32 {
        if self.sidebar {
            metrics::SIDEBAR_WIDTH + metrics::BORDER_WIDTH
        } else {
            0.0
        }
    }

    /// Width of the preview with its rule, as laid out.
    pub fn preview_width(&self) -> f32 {
        if self.preview {
            metrics::PREVIEW_WIDTH + metrics::BORDER_WIDTH
        } else {
            0.0
        }
    }

    /// Room left for the name of a row (its icon apart) in a window `width` wide.
    pub fn name_width(&self, width: f32) -> f32 {
        let columns: f32 = metrics::LIST_COLUMNS
            .iter()
            .zip(self.columns)
            .filter(|(_, shown)| *shown)
            .map(|(w, _)| w + metrics::LIST_COLUMN_GAP)
            .sum();
        width
            - self.sidebar_width()
            - self.preview_width()
            - 2.0 * (metrics::LIST_PADDING + metrics::LIST_ROW_PADDING)
            - metrics::LIST_ICON
            - metrics::GAP_ICON
            - columns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_keeps_its_room_at_every_width() {
        for width in [720.0, 800.0, 944.0, 1100.0, 1366.0, 1920.0, 3840.0] {
            let panes = Panes::at(width, true, false);
            assert!(
                panes.name_width(width) >= more::NAME_MIN_WIDTH,
                "{width}: {panes:?}"
            );
        }
    }

    #[test]
    fn panes_fold_in_order() {
        let wide = Panes::at(1920.0, true, false);
        assert_eq!(
            (wide.sidebar, wide.preview, wide.columns),
            (true, true, [true; 4])
        );
        // Half a 1080p screen: the preview and two columns go, the name and the dates stay.
        let half = Panes::at(944.0, true, false);
        assert_eq!(
            (half.sidebar, half.preview, half.columns),
            (true, false, [true, true, false, false])
        );
        // The smallest window keeps the sidebar and the date.
        let small = Panes::at(720.0, true, false);
        assert_eq!(
            (small.sidebar, small.preview, small.columns),
            (true, false, [true, false, false, false])
        );
        // A preview asked for at that width stays: the columns fold instead.
        let pinned = Panes::at(944.0, true, true);
        assert!(pinned.preview && pinned.sidebar);
        assert_eq!(pinned.columns, [false; 4]);
        // Without the preview, the columns come back sooner.
        assert!(Panes::at(1100.0, false, false).columns[KIND]);
    }
}
