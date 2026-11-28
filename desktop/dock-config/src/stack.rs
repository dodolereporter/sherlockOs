//! The settings of the downloads stack of the dock (lot D-21k2), as macOS offers them on the
//! menu of a stack: the order of its items, the stack shown as a stack of its newest items or
//! as the folder's icon, how its content opens, and whether it is in the dock at all.

/// The order of the items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Sort {
    Name,
    /// When it came into the folder (its status change): what a download is.
    #[default]
    Added,
    Modified,
    Created,
    Kind,
}

/// The icon of the stack in the dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    /// Its newest items, stacked.
    #[default]
    Stack,
    /// The folder's icon.
    Folder,
}

/// How its content opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// The items rise in an arc over the icon.
    Fan,
    Grid,
    /// A menu, its folders in cascade.
    List,
    /// A fan for a few items, else a grid.
    #[default]
    Automatic,
}

impl Sort {
    pub const ALL: [Self; 5] = [
        Self::Name,
        Self::Added,
        Self::Modified,
        Self::Created,
        Self::Kind,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Created => "created",
            Self::Kind => "kind",
        }
    }
}

impl Display {
    pub const ALL: [Self; 2] = [Self::Folder, Self::Stack];

    pub fn key(self) -> &'static str {
        match self {
            Self::Stack => "stack",
            Self::Folder => "folder",
        }
    }
}

impl View {
    pub const ALL: [Self; 4] = [Self::Fan, Self::Grid, Self::List, Self::Automatic];

    pub fn key(self) -> &'static str {
        match self {
            Self::Fan => "fan",
            Self::Grid => "grid",
            Self::List => "list",
            Self::Automatic => "automatic",
        }
    }
}

/// The settings of the stack: `[Dock]` keys `Stack`, `StackSort`, `StackDisplay`, `StackView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stack {
    /// In the dock: « Retirer du Dock » takes it out, the dock's own menu brings it back.
    pub shown: bool,
    pub sort: Sort,
    pub display: Display,
    pub view: View,
}

impl Default for Stack {
    fn default() -> Self {
        Self {
            shown: true,
            sort: Sort::default(),
            display: Display::default(),
            view: View::default(),
        }
    }
}
