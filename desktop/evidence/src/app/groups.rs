//! « Grouper par » (lot E-15b), as the Finder's: the list's items in groups by kind, by date
//! or by size, each under a header row, sorted inside as the list is. A header takes the
//! place of a row: the rows' places count the headers above them.

use std::time::{Duration, SystemTime};

use sherlock_ui::metrics;
use sherlock_ui::texts::evidence as texts;

use super::App;
use crate::filter::KindFilter;
use crate::listing::{Entry, Kind};
use crate::views::ViewMode;

/// How the list groups its items.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum GroupBy {
    #[default]
    None,
    Kind,
    Date,
    Size,
}

impl GroupBy {
    pub const ALL: [Self; 4] = [Self::None, Self::Kind, Self::Date, Self::Size];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => texts::GROUP_NONE,
            Self::Kind => texts::FILTER_KIND,
            Self::Date => texts::FILTER_DATE,
            Self::Size => texts::FILTER_SIZE,
        }
    }

    /// The group of `entry`: its rank, then its header.
    pub fn group(self, entry: &Entry, now: SystemTime, midnight: SystemTime) -> (u8, &'static str) {
        match self {
            Self::None => (0, ""),
            Self::Kind => KindFilter::ALL.iter().position(|k| k.holds(entry)).map_or(
                (u8::MAX, texts::GROUP_OTHER),
                |i| {
                    // Folders first, as in the list.
                    let kind = KindFilter::ALL[i];
                    let rank = if kind == KindFilter::Folder {
                        0
                    } else {
                        i as u8 + 1
                    };
                    (rank, crate::app::search::kind_label(kind))
                },
            ),
            Self::Date => {
                let day = Duration::from_secs(86_400);
                match entry.modified {
                    Some(m) if m >= midnight => (0, texts::FILTER_TODAY),
                    Some(m) if m >= midnight - day => (1, texts::GROUP_YESTERDAY),
                    Some(m) if m >= now - 7 * day => (2, texts::FILTER_WEEK),
                    Some(m) if m >= now - 30 * day => (3, texts::FILTER_MONTH),
                    _ => (4, texts::GROUP_OLDER),
                }
            }
            Self::Size => {
                const MB: u64 = 1_000_000;
                match entry.kind {
                    Kind::Folder => (0, texts::FILTER_FOLDERS),
                    _ if entry.size < MB => (1, texts::FILTER_SMALL),
                    _ if entry.size <= 100 * MB => (2, texts::FILTER_MEDIUM),
                    _ => (3, texts::FILTER_LARGE),
                }
            }
        }
    }
}

/// Sorts `entries` by group, keeping their order inside each group, and returns where each
/// group starts with its header.
pub fn arrange(entries: &mut [Entry], by: GroupBy) -> Vec<(usize, &'static str)> {
    if by == GroupBy::None {
        return Vec::new();
    }
    let now = SystemTime::now();
    let midnight = crate::filter::midnight(now);
    // A stable sort: the list's order holds inside each group.
    entries.sort_by_key(|e| by.group(e, now, midnight).0);
    let mut starts: Vec<(usize, &'static str)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let (_, label) = by.group(entry, now, midnight);
        if starts.last().is_none_or(|(_, last)| *last != label) {
            starts.push((i, label));
        }
    }
    starts
}

impl App {
    /// Whether the list shows groups now.
    pub fn grouped(&self) -> bool {
        self.view_mode == ViewMode::List && !self.groups.is_empty()
    }

    /// The headers above item `index`, its own included.
    fn headers_before(&self, index: usize) -> usize {
        if !self.grouped() {
            return 0;
        }
        self.groups.partition_point(|(start, _)| *start <= index)
    }

    /// The row of item `index`, headers counted.
    pub fn row_of(&self, index: usize) -> usize {
        index + self.headers_before(index)
    }

    /// The item at `row`, if it is not a header.
    pub fn item_of_row(&self, row: usize) -> Option<usize> {
        if !self.grouped() {
            return (row < self.entries.len()).then_some(row);
        }
        let mut headers = 0;
        for (start, _) in &self.groups {
            if start + headers == row {
                return None;
            }
            if start + headers > row {
                break;
            }
            headers += 1;
        }
        let index = row.checked_sub(headers)?;
        (index < self.entries.len()).then_some(index)
    }

    /// Rows of the list, headers counted.
    pub fn row_count(&self) -> usize {
        self.entries.len() + if self.grouped() { self.groups.len() } else { 0 }
    }

    /// The header shown at `row`, if one is.
    pub fn header_at(&self, row: usize) -> Option<&'static str> {
        if !self.grouped() {
            return None;
        }
        self.groups
            .iter()
            .enumerate()
            .find(|(n, (start, _))| start + n == row)
            .map(|(_, (_, label))| *label)
    }

    /// Top of item `index`'s row in the list's content.
    pub fn row_top(&self, index: usize) -> f32 {
        metrics::LIST_TOP_GAP + self.row_of(index) as f32 * metrics::LIST_ROW_HEIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_follow_their_rank_and_keep_the_order_inside() {
        let tmp = crate::testing::TempDir::new("groups");
        for name in ["b.png", "notes.md", "a.png", "film.mkv"] {
            std::fs::write(tmp.path().join(name), "x").expect("write");
        }
        std::fs::create_dir(tmp.path().join("dossier")).expect("folder");
        let mut entries = crate::listing::read_dir(tmp.path(), false).expect("read");
        crate::listing::sort(&mut entries, crate::listing::Sort::default());
        let starts = arrange(&mut entries, GroupBy::Kind);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["dossier", "a.png", "b.png", "notes.md", "film.mkv"]);
        let labels: Vec<(usize, &str)> = starts;
        assert_eq!(
            labels,
            [
                (0, "Dossiers"),
                (1, "Images"),
                (3, "Documents"),
                (4, "Vidéos")
            ]
        );
    }
}
