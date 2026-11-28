//! The view of the folder (lot E-13): chosen from the toolbar, the « Vue » submenu or
//! Ctrl+1 to Ctrl+4, remembered for each folder (`crate::views`), and the geometry of the grid
//! of icons, which the selection, the lasso, the keyboard and the drops follow.

use iced::Task;
use iced::widget::{operation, scrollable};
use sherlock_ui::evidence as more;
use sherlock_ui::metrics;
use sherlock_ui::motion::Fade;

use super::{App, LIST, Location, Message, STRIP};
use crate::views::{ViewMode, clamp_icon};

impl App {
    /// The views the window can show now: all of them since lot E-13c.
    pub fn view_available(&self, mode: ViewMode) -> bool {
        let _ = mode;
        true
    }

    /// Shows the folder as `mode`, fading in, and remembers it for this folder.
    pub(super) fn set_view(&mut self, mode: ViewMode) -> Task<Message> {
        if !self.view_available(mode) || mode == self.view_mode {
            return Task::none();
        }
        let from = self.preview_level();
        self.view_mode = mode;
        self.highlight = None;
        // The unfolded folders show in the list only (lot E-13d).
        self.refilter();
        // The preview's pane slides away into the columns, and back (lot E-13b).
        let to = if self.panes().preview { 1.0 } else { 0.0 };
        if (to - from).abs() > f32::EPSILON {
            self.preview_slide = Some(Fade::new(from, to, self.now(), more::PREVIEW_SLIDE));
        }
        self.content_fade = Some(Fade::new(0.0, 1.0, self.now(), more::VIEW_SWITCH));
        if let Location::Folder(dir) = &self.location {
            self.view_memory.set(dir, mode);
        }
        self.scroll.0 = 0.0;
        // The gallery always shows an item large: the first, when none is selected.
        if mode == ViewMode::Gallery && self.selection.is_empty() && !self.entries.is_empty() {
            self.select_only(0);
        }
        Task::batch([
            operation::scroll_to(LIST, scrollable::AbsoluteOffset { x: 0.0, y: 0.0 }),
            self.save_views(),
            self.load_columns(),
        ])
    }

    /// The view remembered for the folder just opened, else the list (the columns while
    /// browsing in them).
    pub(super) fn recall_view(&mut self) {
        // The columns are a way of browsing: they stay while one goes down and up in them,
        // unless the folder has its own view (lot E-13b).
        let browsing = self.view_mode == ViewMode::Columns;
        let mode = match &self.location {
            Location::Folder(dir) => match self.view_memory.get(dir) {
                Some(mode) => mode,
                None if browsing => ViewMode::Columns,
                None => ViewMode::List,
            },
            Location::Trash => ViewMode::List,
        };
        self.view_mode = if self.view_available(mode) {
            mode
        } else {
            ViewMode::List
        };
    }

    /// The slider of the grid, from 0 to 100.
    pub fn icon_slider(&self) -> u8 {
        let span = more::GRID_ICON_MAX - more::GRID_ICON_MIN;
        ((self.view_memory.icon_size - more::GRID_ICON_MIN) / span * 100.0).round() as u8
    }

    pub(super) fn set_icon_slider(&mut self, value: u8) {
        let span = more::GRID_ICON_MAX - more::GRID_ICON_MIN;
        self.view_memory.icon_size =
            clamp_icon(more::GRID_ICON_MIN + f32::from(value.min(100)) / 100.0 * span);
    }

    /// Writes the views file, off the interface thread.
    pub(super) fn save_views(&self) -> Task<Message> {
        let Some(path) = self.views_file.clone() else {
            return Task::none();
        };
        let memory = self.view_memory.clone();
        Task::perform(
            async move {
                if let Err(error) = crate::views::write(&path, &memory) {
                    tracing::warn!(%error, "the views are not remembered");
                }
            },
            |()| Message::Inert,
        )
    }

    // The grid of icons.

    /// Width and height of a tile.
    pub fn tile_size(&self) -> (f32, f32) {
        let icon = self.view_memory.icon_size;
        (
            icon + more::GRID_TILE_EXTRA,
            icon + more::GRID_LABEL_HEIGHT + 2.0 * more::GRID_TILE_PADDING,
        )
    }

    /// Width of the listing's content.
    pub fn content_width(&self) -> f32 {
        let panes = self.panes();
        (self.size.width
            - panes.sidebar_width()
            - panes.preview_width()
            - 2.0 * metrics::LIST_PADDING)
            .max(0.0)
    }

    /// Tiles on a line: as many as fit, one at least.
    pub fn grid_columns(&self) -> usize {
        let (width, _) = self.tile_size();
        (((self.content_width() + more::GRID_GAP) / (width + more::GRID_GAP)).floor() as usize)
            .max(1)
    }

    /// Top of the line of tile `index`, and the height of a line, in the content.
    pub fn grid_line(&self, index: usize) -> (f32, f32) {
        let (_, height) = self.tile_size();
        let line = height + more::GRID_GAP;
        (
            metrics::LIST_TOP_GAP + (index / self.grid_columns()) as f32 * line,
            line,
        )
    }

    /// The tile at `(x, y)` of the content, if any.
    pub fn tile_at(&self, x: f32, y: f32) -> Option<usize> {
        let (width, height) = self.tile_size();
        let (x, y) = (x, y - metrics::LIST_TOP_GAP);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (column, line) = (
            (x / (width + more::GRID_GAP)) as usize,
            (y / (height + more::GRID_GAP)) as usize,
        );
        let inside_x = x - column as f32 * (width + more::GRID_GAP) < width;
        let inside_y = y - line as f32 * (height + more::GRID_GAP) < height;
        let index = line * self.grid_columns() + column;
        (column < self.grid_columns() && inside_x && inside_y && index < self.entries.len())
            .then_some(index)
    }

    /// The tiles a rectangle of the content crosses, in order.
    pub fn tiles_in(&self, (x, y, w, h): (f32, f32, f32, f32)) -> Vec<usize> {
        let (width, height) = self.tile_size();
        let columns = self.grid_columns();
        (0..self.entries.len())
            .filter(|i| {
                let left = (i % columns) as f32 * (width + more::GRID_GAP);
                let (top, _) = self.grid_line(*i);
                left < x + w && left + width > x && top < y + h && top + height > y
            })
            .collect()
    }

    /// The item under a point of the content, whatever the view.
    pub fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        match self.view_mode {
            ViewMode::Icons => self.tile_at(x, y),
            // In the gallery, the tiles of the strip at the bottom of the listing.
            ViewMode::Gallery => {
                let top = self.list_height() - more::GALLERY_STRIP + more::GALLERY_STRIP_PADDING;
                let step = more::GALLERY_TILE + more::GRID_GAP;
                // Each tile comes after a gap, the first one too.
                let x = x + self.strip_offset - more::GRID_GAP;
                let index = (x.max(0.0) / step) as usize;
                let inside = x >= 0.0
                    && y >= top
                    && y < top + more::GALLERY_TILE
                    && x - index as f32 * step < more::GALLERY_TILE;
                (inside && index < self.entries.len()).then_some(index)
            }
            // Only the column of the folder shown holds the items.
            ViewMode::Columns
                if x < self.shown_column_left()
                    || x >= self.shown_column_left() + more::COLUMN_WIDTH =>
            {
                None
            }
            _ => {
                let row = ((y - metrics::LIST_TOP_GAP) / metrics::LIST_ROW_HEIGHT).floor();
                (row >= 0.0 && (row as usize) < self.entries.len()).then_some(row as usize)
            }
        }
    }

    /// The tiles of the gallery's strip in sight, with a margin: first and past the last.
    pub fn strip_range(&self) -> (usize, usize) {
        let step = more::GALLERY_TILE + more::GRID_GAP;
        let total = self.entries.len();
        let first = ((self.strip_offset / step).floor().max(0.0) as usize).saturating_sub(2);
        let last = ((self.strip_offset + self.content_width()) / step).ceil() as usize + 2;
        (first.min(total), last.min(total))
    }

    /// Scrolls the gallery's strip so that tile `index` is in sight.
    pub(super) fn reveal_in_strip(&mut self, index: usize) -> Task<Message> {
        let step = more::GALLERY_TILE + more::GRID_GAP;
        let (left, width) = (index as f32 * step, self.content_width());
        let x = if left < self.strip_offset {
            left
        } else if left + step > self.strip_offset + width {
            left + step - width
        } else {
            return Task::none();
        };
        self.strip_offset = x;
        operation::scroll_to(STRIP, scrollable::AbsoluteOffset { x, y: 0.0 })
    }

    /// Items from one line to the next: a line of tiles in the grid, else one row.
    pub fn line_step(&self) -> usize {
        match self.view_mode {
            ViewMode::Icons => self.grid_columns(),
            _ => 1,
        }
    }

    /// Top and height of item `index` in the content, to keep it in sight.
    pub fn item_span(&self, index: usize) -> (f32, f32) {
        match self.view_mode {
            ViewMode::Icons => self.grid_line(index),
            _ => (
                metrics::LIST_TOP_GAP + index as f32 * metrics::LIST_ROW_HEIGHT,
                metrics::LIST_ROW_HEIGHT,
            ),
        }
    }

    /// Height of the whole content.
    pub fn content_height(&self) -> f32 {
        match self.view_mode {
            ViewMode::Icons if !self.entries.is_empty() => {
                let (top, line) = self.grid_line(self.entries.len() - 1);
                top + line
            }
            _ => metrics::LIST_TOP_GAP + self.entries.len() as f32 * metrics::LIST_ROW_HEIGHT,
        }
    }
}
