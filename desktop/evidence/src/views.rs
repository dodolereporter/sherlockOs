//! The ways a folder is shown, as in the Finder (lot E-13): a list, a grid of icons, columns,
//! a gallery. The one chosen is remembered for each folder, and the size of the icons once,
//! in `$XDG_CONFIG_HOME/evidence/views`: `size=72`, then one `<view>\t<folder>` line per
//! folder, the most recent last. Bounded on reading and on writing.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};

/// Longest file read.
const MAX_SIZE: u64 = 256 * 1024;
/// Folders remembered at most: the oldest choice goes first.
pub const MAX_FOLDERS: usize = 512;

/// How the folder is shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ViewMode {
    #[default]
    List,
    Icons,
    Columns,
    Gallery,
}

impl ViewMode {
    /// In the order of the toolbar's buttons and of Ctrl+1 to Ctrl+4.
    pub const ALL: [Self; 4] = [Self::Icons, Self::List, Self::Columns, Self::Gallery];

    /// Its word in the file.
    fn key(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Icons => "icons",
            Self::Columns => "columns",
            Self::Gallery => "gallery",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }

    /// The view of Ctrl+`digit`, from 1.
    pub fn from_digit(digit: &str) -> Option<Self> {
        let n: usize = digit.parse().ok()?;
        Self::ALL.get(n.checked_sub(1)?).copied()
    }
}

/// The views chosen, folder by folder, and the size of the icons.
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    folders: VecDeque<(PathBuf, ViewMode)>,
    /// Side of the icons of the grid, in pixels.
    pub icon_size: f32,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            folders: VecDeque::new(),
            icon_size: sherlock_ui::evidence::GRID_ICON_DEFAULT,
        }
    }
}

impl Memory {
    /// The view chosen for `folder`, if one was.
    pub fn get(&self, folder: &Path) -> Option<ViewMode> {
        self.folders
            .iter()
            .find(|(dir, _)| dir == folder)
            .map(|(_, mode)| *mode)
    }

    /// Remembers `mode` for `folder`, the most recent choice last; the oldest goes beyond
    /// `MAX_FOLDERS`.
    pub fn set(&mut self, folder: &Path, mode: ViewMode) {
        self.folders.retain(|(dir, _)| dir != folder);
        self.folders.push_back((folder.to_path_buf(), mode));
        while self.folders.len() > MAX_FOLDERS {
            self.folders.pop_front();
        }
    }

    /// The file's text.
    pub fn to_text(&self) -> String {
        let mut text = format!("size={}\n", self.icon_size.round());
        for (dir, mode) in &self.folders {
            // A name with a tab or a line break would break the line: such a folder is not
            // remembered.
            let Some(dir) = dir.to_str().filter(|d| !d.contains(['\t', '\n', '\r'])) else {
                continue;
            };
            text.push_str(&format!("{}\t{dir}\n", mode.key()));
        }
        text
    }

    /// Reads the file's text; what it does not understand is skipped.
    pub fn from_text(text: &str) -> Self {
        let mut memory = Self::default();
        for line in text.lines() {
            if let Some(size) = line.strip_prefix("size=") {
                if let Ok(size) = size.trim().parse::<f32>() {
                    memory.icon_size = clamp_icon(size);
                }
            } else if let Some((key, dir)) = line.split_once('\t')
                && let Some(mode) = ViewMode::from_key(key)
                && Path::new(dir).is_absolute()
            {
                memory.set(Path::new(dir), mode);
            }
        }
        memory
    }
}

/// The icon size kept within the slider's range.
pub fn clamp_icon(size: f32) -> f32 {
    use sherlock_ui::evidence::{GRID_ICON_MAX, GRID_ICON_MIN};
    if size.is_finite() {
        size.clamp(GRID_ICON_MIN, GRID_ICON_MAX)
    } else {
        sherlock_ui::evidence::GRID_ICON_DEFAULT
    }
}

/// The file under the configuration folder.
pub fn path(config_home: &Path) -> PathBuf {
    config_home.join("evidence").join("views")
}

/// What the file holds; nothing remembered when it cannot be read.
pub fn read(path: &Path) -> Memory {
    mime_apps::file::read_bounded(path, MAX_SIZE)
        .map(|text| Memory::from_text(&text))
        .unwrap_or_default()
}

/// Writes the file atomically.
pub fn write(path: &Path, memory: &Memory) -> io::Result<()> {
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    mime_apps::file::write_atomically(path, &memory.to_text())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn the_views_are_remembered_folder_by_folder() {
        let tmp = TempDir::new("views");
        let file = path(tmp.path());
        assert_eq!(read(&file), Memory::default());
        let mut memory = Memory::default();
        memory.set(Path::new("/home/ada/Images"), ViewMode::Icons);
        memory.set(Path::new("/home/ada/Code"), ViewMode::Columns);
        memory.set(Path::new("/home/ada/Images"), ViewMode::Gallery);
        memory.set(Path::new("/tmp/a\tb"), ViewMode::Icons);
        memory.icon_size = 96.0;
        write(&file, &memory).expect("write");
        let back = read(&file);
        assert_eq!(
            back.get(Path::new("/home/ada/Images")),
            Some(ViewMode::Gallery)
        );
        assert_eq!(
            back.get(Path::new("/home/ada/Code")),
            Some(ViewMode::Columns)
        );
        assert_eq!(back.get(Path::new("/tmp/a\tb")), None, "not written");
        assert_eq!(back.icon_size, 96.0);
        // Nonsense is skipped, a size out of range is brought back into it.
        let odd = Memory::from_text("size=9000\nweird\tline\nicons\trelative/dir\n");
        assert_eq!(odd.icon_size, sherlock_ui::evidence::GRID_ICON_MAX);
        assert_eq!(odd.folders.len(), 0);
    }

    #[test]
    fn the_oldest_choice_goes_first() {
        let mut memory = Memory::default();
        for i in 0..=MAX_FOLDERS {
            memory.set(&PathBuf::from(format!("/d/{i}")), ViewMode::Icons);
        }
        assert_eq!(memory.get(Path::new("/d/0")), None);
        assert_eq!(memory.get(Path::new("/d/1")), Some(ViewMode::Icons));
        assert_eq!(ViewMode::from_digit("2"), Some(ViewMode::List));
        assert_eq!(ViewMode::from_digit("0"), None);
        assert_eq!(ViewMode::from_digit("5"), None);
    }
}
