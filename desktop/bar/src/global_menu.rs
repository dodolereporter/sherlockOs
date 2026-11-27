//! The global menu (lot D-36): the menus of the active application in the bar, after its name,
//! as on a Mac. Their tree comes from the application over D-Bus (`dbusmenu`); this module
//! keeps it, places the menus in the bar, and walks them: which menu is open, which entry is
//! highlighted, which submenus show.
//!
//! The menus that do not fit in the left half of the bar go into « » », a menu whose entries
//! open them.

use iced::Padding;
use iced::window::Id;
use sherlock_ui::surface::Elevation;
use sherlock_ui::{metrics, panels, strings, typography};

use crate::dbusmenu::Item;
use crate::menu::Key;
use crate::source::workspaces::AppMenu;
use crate::unfold::Unfold;

/// Id of the « » » menu: dbusmenu ids are never negative.
pub const MORE_ID: i32 = -1;

/// A menu shown in a popup: the entry it opens, and its own popup.
#[derive(Debug, Clone)]
pub struct Level {
    /// Id of the entry whose children it shows (a menu of the bar, « » », or a submenu).
    pub item: i32,
    pub popup: Id,
    /// Highlighted entry, as an index among the entries shown.
    pub cursor: Option<usize>,
    /// Size of the popup's surface, its margins included.
    pub size: (u32, u32),
    /// Width of the menu itself.
    pub width: f32,
    pub unfold: Unfold,
}

/// The menus of the active application.
#[derive(Debug, Default)]
pub struct GlobalMenu {
    /// Where the application exports them.
    pub address: Option<AppMenu>,
    /// Their tree, once read: the root, whose children are the menus of the bar.
    pub root: Option<Item>,
    /// The menus open: the one of the bar first, then each submenu.
    pub levels: Vec<Level>,
    /// The menu being read before it opens: its id, and whether the keyboard opens it. A click
    /// elsewhere meanwhile replaces it.
    pub opening: Option<(i32, bool)>,
}

/// How a label is measured: its width in the font of the bar, bold or not.
pub type Measure<'a> = &'a dyn Fn(&str, bool) -> f32;

/// The width of `label` in the interface font of the bar.
pub fn measure(label: &str, bold: bool) -> f32 {
    let font = if bold {
        typography::UI_SEMIBOLD
    } else {
        typography::UI
    };
    sherlock_ui::ellipsis::width(label, font, typography::UI_SIZE)
}

/// Where the bar's buttons of the global menu sit: x and width of each, in the bar surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Width given to the application name, its padding included.
    pub name: f32,
    /// Each menu shown in the bar: its id, x and width.
    pub menus: Vec<(i32, f32, f32)>,
    /// « » », when some menus do not fit: x and width.
    pub more: Option<(f32, f32)>,
}

impl GlobalMenu {
    /// The application's menus, as the bar lists them: the visible children of the root.
    pub fn tops(&self) -> Vec<&Item> {
        self.root
            .as_ref()
            .map(|root| root.shown().filter(|i| !i.separator).collect())
            .unwrap_or_default()
    }

    /// The entries the menu `id` shows: those of a menu or a submenu, or for « » » the menus
    /// that do not fit in the bar.
    pub fn entries(&self, id: i32, placement: &Placement) -> Vec<&Item> {
        if id == MORE_ID {
            let kept: Vec<i32> = placement.menus.iter().map(|m| m.0).collect();
            let tops = self.tops().into_iter();
            return tops.filter(|i| !kept.contains(&i.id)).collect();
        }
        let item = self.root.as_ref().and_then(|root| root.find(id));
        item.map(|i| i.shown().collect()).unwrap_or_default()
    }

    /// The label of the menu `id` of the bar.
    pub fn label(&self, id: i32) -> &str {
        if id == MORE_ID {
            return strings::top_bar::MORE_MENUS;
        }
        let item = self.root.as_ref().and_then(|root| root.find(id));
        item.map_or("", |i| i.label.as_str())
    }

    /// The menu of the bar that is open, if any.
    pub fn open_top(&self) -> Option<i32> {
        self.levels.first().map(|l| l.item)
    }

    /// The level shown in the popup `popup`.
    pub fn level_of(&self, popup: Id) -> Option<usize> {
        self.levels.iter().position(|l| l.popup == popup)
    }

    /// Something moves in the open menus.
    pub fn running(&self) -> bool {
        self.levels.iter().any(|l| l.unfold.running())
    }

    /// The menus of the bar placed from the left of the bar surface, from `left` (where the
    /// application name starts), within `room`: the application name first, at most
    /// `APP_NAME_MAX`, then each menu while it fits, then « » » for the rest.
    pub fn place(&self, app: &str, left: f32, room: f32, measure: Measure<'_>) -> Placement {
        let (pad_left, pad_right) = metrics::APP_NAME_PADDING;
        let name = (measure(app, true) + pad_left + pad_right).min(metrics::APP_NAME_MAX);
        let button = |label: &str| measure(label, false) + 2.0 * panels::BAR_BUTTON_PADDING;
        let gap = metrics::TOP_BAR_LEFT_GAP;
        let tops = self.tops();
        let widths: Vec<f32> = tops.iter().map(|i| button(&i.label)).collect();
        let more = button(strings::top_bar::MORE_MENUS);
        let end = left + room;
        let mut x = left + name + gap;
        let mut menus = Vec::new();
        for (i, (item, width)) in tops.iter().zip(&widths).enumerate() {
            // The last menu needs no room for « » » after it.
            let after = if i + 1 == tops.len() { 0.0 } else { gap + more };
            if x + width + after > end {
                return Placement {
                    name,
                    menus,
                    more: Some((x, more)),
                };
            }
            menus.push((item.id, x, *width));
            x += width + gap;
        }
        Placement {
            name,
            menus,
            more: None,
        }
    }

    /// The x and width of the bar button that opens `id`.
    pub fn button(placement: &Placement, id: i32) -> Option<(f32, f32)> {
        if id == MORE_ID {
            return placement.more;
        }
        let menu = placement.menus.iter().find(|m| m.0 == id)?;
        Some((menu.1, menu.2))
    }

    /// The ids of the bar's buttons, in order, « » » last.
    pub fn buttons(placement: &Placement) -> Vec<i32> {
        let menus = placement.menus.iter().map(|m| m.0);
        menus.chain(placement.more.map(|_| MORE_ID)).collect()
    }
}

/// Room around a menu in its surface. In software, only the gap below the bar for a menu of
/// the bar, none for a submenu: the surface is opaque and holds no shadow. On the GPU, the
/// blurred shadow of the Menus board.
pub fn margins(gpu: bool, submenu: bool) -> Padding {
    if gpu {
        return Elevation::Menu.margins();
    }
    let top = if submenu {
        0.0
    } else {
        metrics::TOP_BAR_MENU_GAP
    };
    Padding {
        top,
        ..Padding::ZERO
    }
}

/// Height of a menu of `entries`, its padding included.
pub fn height(entries: &[&Item]) -> f32 {
    let separator = metrics::BORDER_WIDTH + 2.0 * metrics::MENU_SEPARATOR_MARGIN.0;
    let rows = entries.iter().map(|e| {
        if e.separator {
            separator
        } else {
            metrics::MENU_ITEM_HEIGHT
        }
    });
    rows.sum::<f32>() + 2.0 * metrics::MENU_PADDING
}

/// Top of the entry at `index` in a menu, from the top of its first entry: where a submenu's
/// glass starts, so that its first entry is level with this one.
pub fn entry_top(entries: &[&Item], index: usize) -> f32 {
    let above = &entries[..index.min(entries.len())];
    height(above) - 2.0 * metrics::MENU_PADDING
}

/// Width of a menu of `entries`: the widest label, its mark and shortcut or chevron, between
/// `APP_MENU_MIN_WIDTH` and `APP_MENU_MAX_WIDTH`.
pub fn width(entries: &[&Item], measure: Measure<'_>) -> f32 {
    let marks = entries
        .iter()
        .any(|e| e.toggle != crate::dbusmenu::Toggle::None);
    let mark = if marks {
        metrics::MENU_MARK_COLUMN + metrics::GAP_ICON
    } else {
        0.0
    };
    let widest = entries
        .iter()
        .filter(|e| !e.separator)
        .map(|e| {
            // The gap before what follows the label is there even when nothing does.
            let end = match &e.keys {
                Some(keys) => measure(keys, false),
                None if e.opens() => metrics::ICON_SIZE_SMALL,
                None => 0.0,
            };
            measure(&e.label, false) + metrics::GAP_INLINE + end
        })
        .fold(0.0, f32::max);
    let inner = widest + mark + 2.0 * metrics::MENU_ITEM_PADDING + 2.0 * metrics::MENU_PADDING;
    // A pixel more than the text: shaping rounds differently from one renderer to the other.
    (inner + 2.0)
        .ceil()
        .clamp(metrics::APP_MENU_MIN_WIDTH, metrics::APP_MENU_MAX_WIDTH)
}

/// The highlighted entry after `key` among `entries`: separators and disabled entries are
/// skipped, arrows wrap around.
pub fn move_cursor(entries: &[&Item], cursor: Option<usize>, key: Key) -> Option<usize> {
    let usable: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.separator && e.enabled)
        .map(|(i, _)| i)
        .collect();
    let (first, last) = (*usable.first()?, *usable.last()?);
    let position = cursor.and_then(|c| usable.iter().position(|u| *u == c));
    Some(match (key, position) {
        (Key::First, _) | (Key::Next, None) => first,
        (Key::Last, _) | (Key::Previous, None) => last,
        (Key::Next, Some(p)) => usable.get(p + 1).copied().unwrap_or(first),
        (Key::Previous, Some(p)) => match p {
            0 => last,
            p => usable[p - 1],
        },
        (_, Some(p)) => usable[p],
        (_, None) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbusmenu::Toggle;

    fn entry(id: i32, label: &str) -> Item {
        Item {
            id,
            label: label.to_owned(),
            ..Item::default()
        }
    }

    /// Kate's menus, each with two entries.
    fn kate() -> GlobalMenu {
        let names = [
            "Fichier",
            "Édition",
            "Affichage",
            "Sélection",
            "Aller",
            "Outils",
            "Sessions",
            "Configuration",
            "Aide",
        ];
        let tops = names.iter().enumerate().map(|(i, name)| {
            let id = (i as i32 + 1) * 10;
            Item {
                children: vec![entry(id + 1, "Nouveau"), entry(id + 2, "Ouvrir")],
                submenu: true,
                ..entry(id, name)
            }
        });
        GlobalMenu {
            root: Some(Item {
                children: tops.collect(),
                ..entry(0, "")
            }),
            ..GlobalMenu::default()
        }
    }

    /// Seven pixels per character, as a rough Inter 13.
    fn chars(label: &str, _: bool) -> f32 {
        label.chars().count() as f32 * 7.0
    }

    #[test]
    fn the_menus_follow_the_name_and_the_rest_goes_into_more() {
        let menu = kate();
        let wide = menu.place("Kate", 46.0, 900.0, &chars);
        assert_eq!(wide.menus.len(), 9);
        assert_eq!(wide.more, None);
        // Each button after the name, 4 apart, in order.
        let (_, first_x, first_w) = wide.menus[0];
        assert_eq!(first_x, 46.0 + wide.name + metrics::TOP_BAR_LEFT_GAP);
        assert_eq!(
            wide.menus[1].1,
            first_x + first_w + metrics::TOP_BAR_LEFT_GAP
        );
        // A small screen: what does not fit goes into « » », which fits.
        let narrow = menu.place("Kate", 46.0, 400.0, &chars);
        assert!(narrow.menus.len() < 9 && !narrow.menus.is_empty());
        let (x, width) = narrow.more.unwrap();
        assert!(x + width <= 46.0 + 400.0);
        let more = menu.entries(MORE_ID, &narrow);
        let hidden: Vec<_> = more.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(hidden.len(), 9 - narrow.menus.len());
        assert_eq!(hidden.last(), Some(&"Aide"));
        assert!(more.iter().all(|i| i.opens()));
        assert_eq!(menu.label(MORE_ID), "»");
        assert_eq!(menu.entries(10, &narrow).len(), 2);
        // A long name stops at APP_NAME_MAX.
        let long = menu.place(&"Kate ".repeat(40), 46.0, 2000.0, &chars);
        assert_eq!(long.name, metrics::APP_NAME_MAX);
    }

    #[test]
    fn the_cursor_skips_separators_and_disabled_entries() {
        let separator = Item {
            separator: true,
            ..entry(2, "")
        };
        let disabled = Item {
            enabled: false,
            ..entry(3, "Coller")
        };
        let items = [entry(1, "Copier"), separator, disabled, entry(4, "Tout")];
        let entries: Vec<&Item> = items.iter().collect();
        assert_eq!(move_cursor(&entries, None, Key::Next), Some(0));
        assert_eq!(move_cursor(&entries, Some(0), Key::Next), Some(3));
        assert_eq!(move_cursor(&entries, Some(3), Key::Next), Some(0));
        assert_eq!(move_cursor(&entries, Some(0), Key::Previous), Some(3));
        assert_eq!(move_cursor(&entries, None, Key::Last), Some(3));
        assert_eq!(move_cursor(&[], None, Key::Next), None);
    }

    #[test]
    fn a_menu_is_as_wide_as_its_widest_entry_within_bounds() {
        let save = Item {
            keys: Some(String::from("Ctrl+S")),
            ..entry(1, "Enregistrer")
        };
        let items = [save];
        let entries: Vec<&Item> = items.iter().collect();
        assert_eq!(width(&entries, &chars), metrics::APP_MENU_MIN_WIDTH);
        let long = Item {
            toggle: Toggle::Check(true),
            ..entry(2, &"Afficher ".repeat(10))
        };
        let items = [long];
        let entries: Vec<&Item> = items.iter().collect();
        assert_eq!(width(&entries, &chars), metrics::APP_MENU_MAX_WIDTH);
        // Height: two entries and a separator, the padding around.
        let items = [
            entry(1, "a"),
            Item {
                separator: true,
                ..entry(2, "")
            },
            entry(3, "b"),
        ];
        let entries: Vec<&Item> = items.iter().collect();
        let separator = metrics::BORDER_WIDTH + 2.0 * metrics::MENU_SEPARATOR_MARGIN.0;
        let expected = 2.0 * metrics::MENU_ITEM_HEIGHT + separator + 2.0 * metrics::MENU_PADDING;
        assert_eq!(height(&entries), expected);
        assert_eq!(
            entry_top(&entries, 2),
            metrics::MENU_ITEM_HEIGHT + separator
        );
    }
}
