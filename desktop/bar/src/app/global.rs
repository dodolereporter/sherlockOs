//! The global menu in the bar (lot D-36): the menus of the active application after its name,
//! their popups, the pointer and the keys. The tree and the placement are in `global_menu`, the
//! calls to the application in `dbusmenu`.

use iced::widget::{Column, container, mouse_area, opaque, scrollable};
use iced::window::Id;
use iced::{Element, Length, Task, window};
use iced_exwlshell::actions::{IcedNewPopupSettings, RegionActionCB};
use iced_exwlshell::reexport::{PixelSize, PopupAnchor, PopupGravity, Position};
use sherlock_ui::component::{self as ui, Mark, Trailing};
use sherlock_ui::surface::{self, Elevation};
use sherlock_ui::{animated, color, metrics, theme};

use super::{Bar, Message};
use crate::dbusmenu::{self, Item, Toggle};
use crate::global_menu::{self, Anchor, Level, MORE_ID, Placement};
use crate::menu::Key;
use crate::source::workspaces::AppMenu;
use crate::unfold::Unfold;

/// Room kept under the menus, above the bottom of the screen.
pub(super) const SCREEN_ROOM: f32 = 24.0;

impl Bar {
    /// The menus of the active application are open.
    pub fn app_menu_open(&self) -> bool {
        !self.global.levels.is_empty()
    }

    /// Where the application name and its menus sit in the bar: from the right of the Sherlock
    /// button to where the left half of the bar ends, less the workspaces of the middle.
    pub fn placement(&self) -> Placement {
        let app = self.app_name().unwrap_or_default();
        let (pad_left, pad_right) = metrics::TOP_BAR_PADDING;
        let left = pad_left + metrics::BORDER_WIDTH + metrics::SHERLOCK_BUTTON.0;
        let left = left + metrics::TOP_BAR_LEFT_GAP;
        let chips: f32 = self.group().map_or(0.0, |g| {
            let widths = g.workspaces.iter().enumerate().map(|(i, w)| {
                let label = sherlock_ui::strings::top_bar::workspace(i + 1, w.name.as_deref());
                global_menu::measure(&label, true) + 2.0 * metrics::CHIP_PADDING
            });
            let gaps = g.workspaces.len().saturating_sub(1) as f32 * metrics::CHIP_GAP;
            widths.sum::<f32>() + gaps
        });
        let inner = self.bar_width() - pad_left - pad_right - 2.0 * metrics::BORDER_WIDTH;
        let half = (inner - chips) / 2.0;
        let room = half - (left - pad_left - metrics::BORDER_WIDTH) - metrics::GAP_INLINE;
        self.global
            .place(&app, left, room.max(0.0), &global_menu::measure)
    }

    /// The name shown before the menus: the application's, else the window's title.
    pub fn app_name(&self) -> Option<String> {
        let desktop = self.desktop.as_ref()?;
        let named = desktop
            .app_id
            .as_ref()
            .and_then(|id| self.app_names.get(id));
        named.cloned().flatten().or_else(|| desktop.app.clone())
    }

    /// The active window changed: its menus, if it has others, replace those shown.
    pub(super) fn follow_app_menu(&mut self) -> Task<Message> {
        let menu = self.desktop.as_ref().and_then(|d| d.menu.clone());
        let mut tasks = Task::none();
        if let Some(id) = self.desktop.as_ref().and_then(|d| d.app_id.clone())
            && !self.app_names.contains_key(&id)
        {
            self.app_names.insert(id.clone(), None);
            tasks = Task::perform(
                async move {
                    let name = crate::names::of(&id);
                    (id, name)
                },
                |(id, name)| Message::AppName(id, name),
            );
        }
        if menu == self.global.address {
            return tasks;
        }
        let closed = self.drop_app_menus();
        self.global.address = menu.clone();
        self.global.root = None;
        self.global.opening = None;
        // The name and the menus are read side by side: neither waits for the other.
        match menu {
            Some(menu) => Task::batch([tasks, closed, read_menu(menu, 0)]),
            None => Task::batch([tasks, closed]),
        }
    }

    /// The tree of `menu` under `id`, read: the whole tree for 0, a menu about to open else.
    pub(super) fn app_menu_read(
        &mut self,
        menu: AppMenu,
        id: i32,
        read: Result<Item, String>,
    ) -> Task<Message> {
        if let Some(shown) = &mut self.global.item
            && shown.address == menu
        {
            match read {
                Ok(item) if id == 0 => shown.root = Some(item),
                Ok(item) => {
                    if let Some(root) = &mut shown.root {
                        root.graft(item);
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, service = menu.service, "the icon's menu does not answer");
                    if id == 0 {
                        return self.drop_app_menus();
                    }
                }
            }
            return self.opened_read(id);
        }
        if self.global.address.as_ref() != Some(&menu) {
            return Task::none();
        }
        match read {
            Ok(item) if id == 0 => {
                let menus = item.shown().filter(|i| !i.separator).count();
                tracing::debug!(
                    service = menu.service,
                    menus,
                    "the application's menus read"
                );
                self.global.root = Some(item);
            }
            Ok(item) => {
                if let Some(root) = &mut self.global.root {
                    root.graft(item);
                }
            }
            Err(error) => {
                tracing::warn!(%error, service = menu.service, "the application's menu does not answer");
                if id == 0 {
                    self.global.root = None;
                }
            }
        }
        self.opened_read(id)
    }

    /// The menu `id` was read: it opens when it was waited for, else the menus shown follow it.
    fn opened_read(&mut self, id: i32) -> Task<Message> {
        if let Some((opening, keyboard)) = self.global.opening
            && opening == id
        {
            self.global.opening = None;
            let cursor = keyboard.then(|| self.first_entry(id)).flatten();
            return self.open_level(0, id, cursor);
        }
        self.resize_levels()
    }

    /// Shows the menu at `address` of an icon of the notification area, at `anchor`, once the
    /// application has filled it (lot D-45a).
    pub(super) fn show_item_menu(&mut self, address: AppMenu, anchor: Anchor) -> Task<Message> {
        let closed = self.drop_app_menus();
        self.global.item = Some(global_menu::ItemMenu {
            address: address.clone(),
            root: None,
            anchor,
        });
        self.global.opening = Some((0, false));
        closed.chain(prepare(address, 0))
    }

    /// A menu of the bar clicked: it opens, or closes when it is the one open. While another
    /// one is open, this one replaces it at once.
    pub(super) fn toggle_app_menu(&mut self, id: i32) -> Task<Message> {
        if self.global.open_top() == Some(id) {
            return self.close_app_menus();
        }
        self.show_app_menu(id, false)
    }

    /// The first entry of the menu `id` that can be highlighted.
    fn first_entry(&self, id: i32) -> Option<usize> {
        global_menu_first(&self.global.entries(id, &self.placement()))
    }

    /// The pointer enters a menu of the bar: while one is open, this one opens instead, as on
    /// a Mac.
    pub(super) fn hover_app_menu(&mut self, id: i32) -> Task<Message> {
        match self.global.open_top() {
            Some(open) if open != id && !self.folding_app_menus() => self.show_app_menu(id, false),
            _ => Task::none(),
        }
    }

    fn folding_app_menus(&self) -> bool {
        self.global
            .levels
            .first()
            .is_some_and(|l| l.unfold.folding())
    }

    /// Opens the menu `id` of the bar, once the application has filled it (`AboutToShow`);
    /// opened from the keyboard, its first entry highlighted.
    ///
    /// From one menu to the next, the menu shown stays until the next one is read, then gives
    /// it its place: the bar never shows an empty or half sized popup meanwhile (D-36d).
    pub(super) fn show_app_menu(&mut self, id: i32, keyboard: bool) -> Task<Message> {
        let closed = self
            .drop_menu()
            .chain(self.drop_panel())
            .chain(self.drop_tray());
        // An icon's menu shown from the bar gives its place: its levels are not the
        // application's.
        let closed = if self.global.item.is_some() {
            closed.chain(self.drop_app_menus())
        } else {
            closed
        };
        let Some(menu) = self.global.address.clone() else {
            return closed.chain(self.drop_app_menus());
        };
        if id == MORE_ID {
            let cursor = keyboard.then(|| self.first_entry(id)).flatten();
            return closed.chain(self.open_level(0, id, cursor));
        }
        // A menu folding back goes at once; one shown stays, without its submenus.
        let shown = if self.folding_app_menus() {
            self.drop_app_menus()
        } else {
            self.drop_levels(1)
        };
        self.global.opening = Some((id, keyboard));
        closed.chain(shown).chain(prepare(menu, id))
    }

    /// Opens the popup of the menu `item` at `level`, its first entry highlighted with
    /// `cursor`. The levels from `level` on are replaced.
    pub(super) fn open_level(
        &mut self,
        level: usize,
        item: i32,
        cursor: Option<usize>,
    ) -> Task<Message> {
        let closed = self.drop_levels(level);
        let Some(surface) = self.surface else {
            return closed;
        };
        let placement = self.placement();
        let entries = self.global.entries(item, &placement);
        if entries.is_empty() {
            return closed;
        }
        let gpu = surface::gpu_drawn() == Some(true);
        let margins = global_menu::margins(gpu, level > 0);
        let width = global_menu::width(&entries, &global_menu::measure);
        let size = (
            (width + margins.left + margins.right).ceil() as u32,
            self.level_height(level, &entries),
        );
        let height = size.1 as f32 - margins.top - margins.bottom;
        let item_anchor = self.global.item.as_ref().map(|shown| shown.anchor);
        let settings = if let (0, Some(Anchor::Popup(parent, x, y))) = (level, item_anchor) {
            // An icon's menu under its cell, from the popup of the notification area.
            IcedNewPopupSettings::new(
                parent,
                PixelSize::px(size.0, size.1),
                Position::new((x - margins.left) as i32, (y - margins.top).max(0.0) as i32),
                PixelSize::px(1, 1),
            )
            .anchor(PopupAnchor::TopLeft)
            .gravity(PopupGravity::BottomRight)
        } else if level == 0 {
            let x = match item_anchor {
                Some(Anchor::Bar(x)) => Some(x),
                _ => global_menu::GlobalMenu::button(&placement, item).map(|(x, _)| x),
            };
            let Some(x) = x else {
                return closed;
            };
            // As the Sherlock menu: the popup grows down from the centre of its top edge, so
            // that its shadow may start left of the bar surface.
            let left = x - margins.left;
            let centre = left as i32 + (size.0 / 2) as i32;
            let top = metrics::TOP_BAR_HEIGHT + metrics::TOP_BAR_MENU_GAP - margins.top;
            IcedNewPopupSettings::new(
                surface,
                PixelSize::px(size.0, size.1),
                Position::new(centre - 1, 0),
                PixelSize::px(2, top.max(1.0) as u32),
            )
            .anchor(PopupAnchor::Bottom)
            .gravity(PopupGravity::Bottom)
        } else {
            let Some(parent) = self.global.levels.get(level - 1) else {
                return closed;
            };
            let parent_entries = self.global.entries(parent.item, &placement);
            let index = parent_entries
                .iter()
                .position(|e| e.id == item)
                .unwrap_or(0);
            let parent_margins = global_menu::margins(gpu, level > 1);
            let parent_right = parent_margins.left + parent.width;
            // The submenu's top entry level with the entry that opens it, a gap to its right.
            let x = parent_right + metrics::SUBMENU_GAP - margins.left;
            let x = x.min(parent.size.0 as f32 - 1.0).max(0.0);
            let y =
                parent_margins.top + global_menu::entry_top(&parent_entries, index) - margins.top;
            IcedNewPopupSettings::new(
                parent.popup,
                PixelSize::px(size.0, size.1),
                Position::new(x as i32, y.max(0.0) as i32),
                PixelSize::px(1, 1),
            )
            .anchor(PopupAnchor::TopLeft)
            .gravity(PopupGravity::BottomRight)
        };
        let id = Id::unique();
        let unfold = if self.animates() {
            Unfold::opening(animated::clock())
        } else {
            Unfold::default()
        };
        self.global.levels.push(Level {
            item,
            popup: id,
            cursor,
            size,
            width,
            unfold,
        });
        // Only the menu takes the pointer: the room for its shadow lets the clicks through.
        let (x, y) = (margins.left as i32, margins.top as i32);
        let region = RegionActionCB::new(move |region| {
            if let Err(error) = region.add(x, y, width.ceil() as i32, height.ceil() as i32) {
                tracing::warn!(%error, "cannot limit the menu's input region");
            }
        });
        let about = match (self.global.shown_address(), level) {
            (Some(menu), 1..) if item != MORE_ID => refresh(menu.clone(), item),
            _ => Task::none(),
        };
        closed
            .chain(Task::done(Message::NewPopUp { settings, id }))
            .chain(Task::done(Message::SetInputRegion {
                id,
                callback: region,
            }))
            .chain(about)
    }

    /// The height of the popup of the menu `item` at `level`, its margins included: as tall
    /// as its entries, within the screen.
    fn level_height(&self, level: usize, entries: &[&Item]) -> u32 {
        let gpu = surface::gpu_drawn() == Some(true);
        let margins = global_menu::margins(gpu, level > 0);
        let screen = self.screen_height().unwrap_or(metrics::REFERENCE_SCREEN.1);
        let top = metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT + metrics::TOP_BAR_MENU_GAP;
        let height = global_menu::height(entries).min(screen - top - SCREEN_ROOM);
        (height + margins.top + margins.bottom).ceil() as u32
    }

    /// A menu shown changed its number of entries: its popup opens again at the new size; a
    /// menu left without entries closes.
    fn resize_levels(&mut self) -> Task<Message> {
        let placement = self.placement();
        let changed = self
            .global
            .levels
            .iter()
            .enumerate()
            .find_map(|(level, shown)| {
                let entries = self.global.entries(shown.item, &placement);
                let same =
                    !entries.is_empty() && self.level_height(level, &entries) == shown.size.1;
                (!same).then_some((level, shown.item, shown.cursor, entries.is_empty()))
            });
        match changed {
            Some((level, _, _, true)) => self.drop_levels(level),
            Some((level, item, cursor, false)) => self.open_level(level, item, cursor),
            None => Task::none(),
        }
    }

    /// The pointer over the entry `index` of the menu at `level`: it is highlighted, and its
    /// submenu opens; a submenu of another entry closes.
    pub(super) fn hover_entry(&mut self, level: usize, index: usize) -> Task<Message> {
        let placement = self.placement();
        let Some(shown) = self.global.levels.get(level) else {
            return Task::none();
        };
        let entries = self.global.entries(shown.item, &placement);
        let Some(entry) = entries.get(index) else {
            return Task::none();
        };
        let usable = entry.enabled && !entry.separator;
        let (opens, id) = (entry.opens() && usable, entry.id);
        self.global.levels[level].cursor = usable.then_some(index);
        let child = self.global.levels.get(level + 1).map(|l| l.item);
        match (opens, child) {
            (true, Some(open)) if open == id => Task::none(),
            (true, _) => self.open_level(level + 1, id, None),
            (false, _) => self.drop_levels(level + 1),
        }
    }

    /// The entry `index` of the menu at `level` clicked: a submenu opens, an action runs and
    /// the menus close.
    pub(super) fn pick_entry(&mut self, level: usize, index: usize) -> Task<Message> {
        let placement = self.placement();
        let Some(shown) = self.global.levels.get(level) else {
            return Task::none();
        };
        let entries = self.global.entries(shown.item, &placement);
        let Some(entry) = entries.get(index) else {
            return Task::none();
        };
        if !entry.enabled || entry.separator {
            return Task::none();
        }
        let id = entry.id;
        if entry.opens() {
            let first = global_menu_first(&self.global.entries(id, &placement));
            return self.open_level(level + 1, id, first);
        }
        let Some((menu, id)) = self.picked(level, index) else {
            return Task::none();
        };
        tracing::debug!(id, "menu entry picked");
        self.close_app_menus().chain(Task::perform(
            async move {
                let bus = dbusmenu::bus().await?;
                dbusmenu::clicked(&bus, &menu, id).await
            },
            |sent| Message::EntrySent(sent.map_err(|e| e.to_string())),
        ))
    }

    /// What a click on the entry `index` of the menu at `level` runs: the application's menu
    /// and the entry's id, for `Event` « clicked ». `None` for a separator, a disabled entry or
    /// one that opens a submenu.
    pub fn picked(&self, level: usize, index: usize) -> Option<(AppMenu, i32)> {
        let placement = self.placement();
        let shown = self.global.levels.get(level)?;
        let entries = self.global.entries(shown.item, &placement);
        let entry = entries.get(index)?;
        let runs = entry.enabled && !entry.separator && !entry.opens();
        runs.then(|| (self.global.shown_address().cloned(), entry.id))
            .and_then(|(menu, id)| Some((menu?, id)))
    }

    /// A key while the menus are open.
    pub(super) fn app_menu_key(&mut self, key: Key) -> Task<Message> {
        let placement = self.placement();
        let deepest = self.global.levels.len().saturating_sub(1);
        let Some(shown) = self.global.levels.get(deepest) else {
            return Task::none();
        };
        let entries = self.global.entries(shown.item, &placement);
        let cursor = shown.cursor;
        let current = cursor
            .and_then(|c| entries.get(c))
            .map(|e| (e.id, e.opens()));
        match key {
            Key::Close if deepest > 0 => self.drop_levels(deepest),
            Key::Close => self.close_app_menus(),
            Key::Activate => match cursor {
                Some(index) => self.pick_entry(deepest, index),
                None => Task::none(),
            },
            Key::Right => match current {
                Some((id, true)) => {
                    let first = global_menu_first(&self.global.entries(id, &placement));
                    self.open_level(deepest + 1, id, first)
                }
                _ => self.next_top(&placement, 1),
            },
            Key::Left if deepest > 0 => self.drop_levels(deepest),
            Key::Left => self.next_top(&placement, -1),
            key => {
                let moved = global_menu::move_cursor(&entries, cursor, key);
                self.global.levels[deepest].cursor = moved;
                Task::none()
            }
        }
    }

    /// The menu of the bar `step` away from the open one, wrapping around, its first entry
    /// highlighted.
    fn next_top(&mut self, placement: &Placement, step: isize) -> Task<Message> {
        let buttons = global_menu::GlobalMenu::buttons(placement);
        let Some(open) = self.global.open_top() else {
            return Task::none();
        };
        let Some(at) = buttons.iter().position(|b| *b == open) else {
            return Task::none();
        };
        let count = buttons.len() as isize;
        let next = buttons[(at as isize + step).rem_euclid(count) as usize];
        self.show_app_menu(next, true)
    }

    /// Closes the menus: the submenus at once, the menu of the bar folding back first.
    pub(super) fn close_app_menus(&mut self) -> Task<Message> {
        let closed = self.drop_levels(1);
        self.global.opening = None;
        let animates = self.animates();
        match self.global.levels.first_mut() {
            Some(level) if animates => {
                level.cursor = None;
                level.unfold.fold(animated::clock(), false);
                closed
            }
            _ => closed.chain(self.drop_app_menus()),
        }
    }

    /// Closes every popup of the menus at once.
    pub(super) fn drop_app_menus(&mut self) -> Task<Message> {
        self.global.opening = None;
        self.global.item = None;
        self.drop_levels(0)
    }

    /// Closes the popups of the levels from `level` on, at once.
    fn drop_levels(&mut self, level: usize) -> Task<Message> {
        if level >= self.global.levels.len() {
            return Task::none();
        }
        let gone = self.global.levels.split_off(level);
        // The deepest first: a popup closes before its parent.
        Task::batch(gone.into_iter().rev().map(|l| window::close(l.popup)))
    }

    /// The compositor closed the popup `id` (a click elsewhere): it and the levels under it go.
    pub(super) fn app_menu_closed(&mut self, id: Id) {
        if let Some(level) = self.global.level_of(id) {
            self.global.levels.truncate(level);
            if level == 0 {
                self.global.item = None;
            }
        }
    }

    /// Next frame of the menus' motion; the menu of the bar goes once it has faded out.
    pub(super) fn tick_app_menus(&mut self, at: std::time::Duration) -> Task<Message> {
        for level in &mut self.global.levels {
            level.unfold.tick(at);
        }
        match self.global.levels.first() {
            Some(level) if level.unfold.folded() => self.drop_app_menus(),
            _ => Task::none(),
        }
    }

    /// The buttons of the menus, after the application name.
    pub(super) fn view_app_menus(&self, placement: &Placement) -> Vec<Element<'_, Message>> {
        let open = self.global.open_top();
        global_menu::GlobalMenu::buttons(placement)
            .into_iter()
            .map(|id| {
                let label = ui::body(self.global.label(id));
                let toggle = Some(Message::OpenAppMenu(id));
                let button = sherlock_ui::panels::bar_button(label, open == Some(id), toggle);
                let button = if self.focused() == Some(super::Target::AppMenu(id)) {
                    button.focus_ring()
                } else {
                    button
                };
                mouse_area(button)
                    .on_enter(Message::HoverAppMenu(id))
                    .into()
            })
            .collect()
    }

    /// The menu shown in the popup of `level`.
    pub(super) fn view_app_menu(&self, level: usize) -> Element<'_, Message> {
        let Some(shown) = self.global.levels.get(level) else {
            return iced::widget::space().into();
        };
        let placement = self.placement();
        let entries = self.global.entries(shown.item, &placement);
        let marks = entries.iter().any(|e| e.toggle != Toggle::None);
        let child = self.global.levels.get(level + 1).map(|l| l.item);
        let gpu = surface::gpu_drawn() == Some(true);
        let margins = global_menu::margins(gpu, level > 0);
        let panel = color::with_opacity(shown.unfold.opacity(), || {
            let rows = entries.iter().enumerate().map(|(i, entry)| {
                if entry.separator {
                    return ui::menu_separator();
                }
                let trailing = match (&entry.keys, entry.opens()) {
                    (_, true) => Trailing::Submenu,
                    (Some(keys), false) => Trailing::Shortcut(keys),
                    (None, false) => Trailing::None,
                };
                let mark = match entry.toggle {
                    Toggle::Check(true) => Mark::Check,
                    Toggle::Radio(true) => Mark::Radio,
                    _ => Mark::Blank,
                };
                let leading = marks.then(|| ui::menu_mark(mark, entry.enabled));
                let pick = entry.enabled.then_some(Message::PickEntry(level, i));
                let mut row = ui::menu_item_with(leading, &entry.label, trailing, pick);
                if shown.cursor == Some(i) || child == Some(entry.id) {
                    row = ui::open(row);
                }
                mouse_area(row)
                    .on_enter(Message::HoverEntry(level, i))
                    .into()
            });
            let list = scrollable(Column::with_children(rows));
            let panel = container(list)
                .width(shown.width)
                .height(Length::Fill)
                .padding(metrics::MENU_PADDING)
                .style(theme::menu);
            opaque(surface::shadowed(
                panel,
                metrics::MENU_RADIUS,
                Elevation::Menu,
            ))
        });
        let panel = shown.unfold.panel(panel, (0.0, 0.0));
        // A click on the shadow closes the menus, as a click elsewhere.
        mouse_area(container(panel).padding(margins))
            .on_press(Message::CloseAppMenus)
            .into()
    }
}

/// The first entry that can be highlighted.
fn global_menu_first(entries: &[&Item]) -> Option<usize> {
    global_menu::move_cursor(entries, None, Key::First)
}

/// Reads the tree of `menu` under `id`.
pub(super) fn read_menu(menu: AppMenu, id: i32) -> Task<Message> {
    let asked = menu.clone();
    Task::perform(
        async move {
            let bus = dbusmenu::bus().await?;
            dbusmenu::layout(&bus, &asked, id).await
        },
        move |read| Message::AppMenuRead(menu.clone(), id, read.map_err(|e| e.to_string())),
    )
}

/// Tells the application the menu `id` opens, then reads it: what it filled shows.
fn prepare(menu: AppMenu, id: i32) -> Task<Message> {
    let asked = menu.clone();
    Task::perform(
        async move {
            let bus = dbusmenu::bus().await?;
            // A menu that cannot say is read as it is.
            if let Err(error) = dbusmenu::about_to_show(&bus, &asked, id).await {
                tracing::debug!(%error, id, "AboutToShow not answered");
            }
            dbusmenu::layout(&bus, &asked, id).await
        },
        move |read| Message::AppMenuRead(menu.clone(), id, read.map_err(|e| e.to_string())),
    )
}

/// A submenu opens: the application may fill it, then it is read again.
fn refresh(menu: AppMenu, id: i32) -> Task<Message> {
    prepare(menu, id)
}
