//! The two views of the dock: the dock itself and the menu of an icon.

use dock_config::Position as Edge;
use iced::widget::{container, mouse_area, pin, row, space, stack, text};
use iced::window::Id;
use iced::{Alignment, Element, Length, Rectangle, mouse};
use sherlock_ui::animated::reveal;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::{color, dock, metrics, typography};

use crate::app::{Dock, Message, Pointer};
use crate::items::Glyph;
use crate::layout::{Geometry, Layout};
use crate::stack::Presentation;

impl Dock {
    pub fn view(&self, id: Id) -> Element<'_, Message> {
        if self.menu.as_ref().is_some_and(|m| m.id == id) {
            self.view_menu()
        } else {
            self.view_dock()
        }
    }

    pub fn view_dock(&self) -> Element<'_, Message> {
        let layout = self.layout(self.now);
        let edge = self.geometry.position;
        let bar = layout.bar;
        let glass = container(space())
            .width(bar.width)
            .height(bar.height)
            .style(dock::bar);
        // The blur behind the glass, over its rounded shape (lot D-37).
        let glass = sherlock_ui::glass::glass(glass, metrics::DOCK_RADIUS);
        let mut layers = vec![place(glass, bar.x, bar.y)];
        if let Some(separator) = layout.separator {
            let line = container(space())
                .width(separator.width)
                .height(separator.height)
                .style(dock::separator);
            layers.push(place(line, separator.x, separator.y));
        }
        let now = self.now;
        // Icons leaving: closing their room in the bar, or vanishing where they were let go.
        for (leaving, rect) in self.flowing().zip(&layout.leaving) {
            let track = &leaving.track;
            let (scale, opacity) = (track.drawn_scale(now), track.opacity.value(now));
            layers.push(drawn(&leaving.item.glyph, *rect, edge, scale, 0.0, opacity));
        }
        for leaving in &self.leaving {
            let Some(offset) = leaving.dropped else {
                continue;
            };
            let track = &leaving.track;
            let rect = layout.at_offset(offset, track.size.value(now) as f32);
            let (scale, opacity) = (track.drawn_scale(now), track.opacity.value(now));
            layers.push(drawn(&leaving.item.glyph, rect, edge, scale, 0.0, opacity));
        }
        let held = self.held();
        let mut on_top = None;
        for (index, (item, icon)) in self.items.iter().zip(&layout.icons).enumerate() {
            let track = &self.tracks[index];
            let (scale, opacity) = match held {
                Some((i, out)) if i == index => {
                    (1.0, if out { dock::DRAG_OUT_OPACITY } else { 1.0 })
                }
                _ => (track.drawn_scale(now), track.opacity.value(now)),
            };
            let lift = self.lift(index, now);
            let layer = drawn(&item.glyph, *icon, edge, scale, lift, opacity);
            if held.is_some_and(|(i, _)| i == index) {
                // Above the others, without its dot.
                on_top = Some(layer);
                continue;
            }
            layers.push(layer);
            if self.focus == Some(index) {
                let pad = metrics::FOCUS_RING_GAP + metrics::FOCUS_RING_WIDTH;
                let ring = container(space())
                    .width(icon.width + 2.0 * pad)
                    .height(icon.height + 2.0 * pad)
                    .style(dock::ring(icon.width));
                layers.push(place(ring, icon.x - pad, icon.y - pad));
            }
            // One dot for an open application, however many windows it has, as on macOS; it
            // fades in and out with the application's first and last window.
            let shown = track.dot.value(now) * opacity;
            if shown > 0.0
                && let Some(dot) = layout.dot(index)
            {
                let mark = container(space())
                    .width(dot.width)
                    .height(dot.height)
                    .style(dock::dot(track.dot_active, shown));
                layers.push(place(mark, dot.x, dot.y));
            }
        }
        layers.extend(on_top);
        // The smoke of an application let go out of the dock, over everything.
        for poof in &self.poofs {
            let Some((puffs, opacity)) = poof.puffs(now) else {
                continue;
            };
            for ((x, y), radius) in puffs {
                let puff = container(space())
                    .width(2.0 * radius)
                    .height(2.0 * radius)
                    .style(dock::poof(radius, opacity));
                layers.push(place(puff, x - radius, y - radius));
            }
        }
        if let Some(label) = self.view_label(&layout) {
            layers.push(label);
        }
        // The pointer is tracked over the whole surface: `mouse_area` also reports where the
        // pointer enters, which the bare enter event does not carry.
        let layers = stack(layers).width(Length::Fill).height(Length::Fill);
        let area = mouse_area(layers)
            .on_move(|point| Message::Pointer(Pointer::Moved(point)))
            .on_exit(Message::Pointer(Pointer::Left))
            .on_press(Message::Pointer(Pointer::Pressed(mouse::Button::Left)))
            .on_release(Message::Pointer(Pointer::Released))
            .on_right_press(Message::Pointer(Pointer::Pressed(mouse::Button::Right)))
            .on_middle_press(Message::Pointer(Pointer::Pressed(mouse::Button::Middle)))
            .on_scroll(Message::Scroll);
        // The closed hand while an icon is dragged, wherever the pointer goes.
        sherlock_ui::pointer::held(area, self.held().is_some()).into()
    }

    /// The name of the icon closest to the pointer, past the magnified icons (board: 108 above
    /// the bottom of the bar), centered on the icon; beside it on the left and right edges.
    fn view_label(&self, layout: &Layout) -> Option<Element<'_, Message>> {
        let opacity = self.label.value(self.now);
        let index = self.label_item?;
        if self.held().is_some() {
            return None;
        }
        if opacity <= 0.0 {
            return None;
        }
        let anchor = layout.label_anchor(index)?;
        let name = self.items.get(index)?.name.as_str();
        let (vertical, horizontal) = dock::LABEL_PADDING;
        let pill = container(
            text(name)
                .size(dock::LABEL_SIZE)
                .line_height(text::LineHeight::Relative(typography::LINE_HEIGHT))
                .font(typography::UI_SEMIBOLD),
        )
        .padding([vertical, horizontal])
        .style(dock::label(opacity));
        let height = Geometry::label_height();
        let surface = self.surface_size;
        // A box as wide as the surface allows around the anchor, the pill aligned in it.
        let (x, y, width, align) = match self.geometry.position {
            Edge::Bottom | Edge::Top => {
                let span = 2.0 * anchor.x.min(surface.width - anchor.x).max(0.0);
                let y = if self.geometry.position == Edge::Bottom {
                    anchor.y - height
                } else {
                    anchor.y
                };
                (anchor.x - span / 2.0, y, span, Alignment::Center)
            }
            Edge::Left => (
                anchor.x,
                anchor.y - height / 2.0,
                surface.width - anchor.x,
                Alignment::Start,
            ),
            Edge::Right => (0.0, anchor.y - height / 2.0, anchor.x, Alignment::End),
        };
        let aligned = container(pill).width(width.max(0.0)).align_x(align);
        Some(place(aligned, x, y))
    }

    pub fn view_menu(&self) -> Element<'_, Message> {
        let Some(menu) = &self.menu else {
            return space().into();
        };
        let opacity = menu.unfold.opacity(self.now);
        // Built inside the fade, for the colors its texts take when they are built.
        let panel = color::with_opacity(opacity, || {
            let index = menu.target.as_ref().and_then(|t| self.index_of(t));
            let name = match &menu.target {
                Some(_) => index
                    .and_then(|i| self.items.get(i))
                    .map_or("", |i| i.name.as_str()),
                None => dock::text::DOCK,
            };
            let mut header = row![
                ui::body(name).font(typography::UI_SEMIBOLD),
                space::horizontal()
            ]
            .align_y(Alignment::Center)
            .height(dock::MENU_HEADER_HEIGHT)
            .padding([0.0, metrics::MENU_ITEM_PADDING]);
            if let Some((count, _)) = index.and_then(|i| self.open_windows(i))
                && count > 0
            {
                let count = text(dock::text::windows(count))
                    .size(typography::SMALL_SIZE)
                    .color(color::palette().text_secondary);
                header = header.push(count);
            }
            if let Some((shown, count)) = menu.stack {
                let panel = match shown {
                    // The fan has no panel: its items rise over the dock (lot D-21k2).
                    Presentation::Fan => {
                        return self.view_fan(count, menu.unfold.progress(self.now));
                    }
                    Presentation::Grid => self.view_grid(header.into(), count),
                    Presentation::List => self.view_list(header.into(), count),
                };
                return sherlock_ui::glass::glass(panel, metrics::MENU_RADIUS).into();
            }
            let mut entries: Vec<Element<'_, Message>> = vec![header.into()];
            for (index, line) in menu.entries.iter().enumerate() {
                if line.starts_group {
                    entries.push(ui::menu_separator());
                }
                let on_press = line.enabled.then_some(Message::MenuPick(line.entry));
                let trailing = if line.checked {
                    Trailing::Checked
                } else {
                    Trailing::None
                };
                // A long window title ends with « … » (`menu_item`).
                let mut item = ui::menu_item(&line.label, trailing, on_press);
                if menu.cursor == Some(index) {
                    item = ui::open(item);
                }
                entries.push(mouse_area(item).on_enter(Message::MenuHover(index)).into());
            }
            let menu = ui::menu(entries, crate::menu::width(&menu.entries)).height(Length::Fill);
            // The blur behind its glass (lot D-37b).
            sherlock_ui::glass::glass(menu, metrics::MENU_RADIUS).into()
        });
        // It unfolds from its icon: from the side facing the dock.
        let origin = match self.geometry.position {
            Edge::Bottom => (0.5, 1.0),
            Edge::Top => (0.5, 0.0),
            Edge::Left => (0.0, 0.5),
            Edge::Right => (1.0, 0.5),
        };
        // The fan's items come out of the icon themselves; the panels grow from it.
        let fan = matches!(menu.stack, Some((Presentation::Fan, _)));
        let scale = if fan {
            1.0
        } else {
            menu.unfold.scale(self.now)
        };
        reveal(panel).opacity(opacity).scale(scale, origin).into()
    }
}

impl Dock {
    /// The grid of the downloads stack (lot D-21k): its newest items, `count` at most, four to
    /// a row, then « Ouvrir dans Evidence ».
    fn view_grid<'a>(&'a self, header: Element<'a, Message>, count: usize) -> Element<'a, Message> {
        let items = self.stack.as_ref().map_or(&[][..], |s| &s.items[..]);
        let cells: Vec<Element<'a, Message>> = items
            .iter()
            .take(count)
            .enumerate()
            .map(|(index, item)| {
                let picture =
                    ui::icon(item.icon, dock::STACK_ICON, color::palette().text_secondary);
                dock::stack_cell(picture, &item.name, Message::StackOpen(index))
            })
            .collect();
        let mut rows = iced::widget::Column::new().spacing(dock::STACK_GAP);
        if cells.is_empty() {
            let empty = container(ui::description(dock::text::STACK_EMPTY))
                .center_x(Length::Fill)
                .center_y(dock::STACK_CELL.1);
            rows = rows.push(empty);
        }
        let mut cells = cells.into_iter().peekable();
        while cells.peek().is_some() {
            let line: Vec<_> = cells.by_ref().take(dock::STACK_COLUMNS).collect();
            rows = rows.push(iced::widget::Row::from_vec(line).spacing(dock::STACK_GAP));
        }
        let footer = ui::menu_item(
            dock::text::OPEN_IN_EVIDENCE,
            Trailing::None,
            Some(Message::StackFolder),
        );
        // Past four rows, the grid scrolls (UI-11, lot D-21k2).
        let rows = sherlock_ui::scroll::scroll(rows).height(Length::Fill);
        let content = iced::widget::column![header, rows, footer]
            .width(Length::Fill)
            .height(Length::Fill);
        container(content)
            .padding(dock::STACK_PADDING)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(sherlock_ui::theme::menu)
            .into()
    }
}

impl Dock {
    /// The fan of the stack (lot D-21k2), as macOS's: « Ouvrir dans Evidence » at the top, then
    /// its items in an arc, the first at the bottom, over the icon; each its icon and its name in
    /// a pill on its left. They come out of the icon one after the other as `progress` goes to
    /// 1, on the menus' spring.
    fn view_fan(&self, count: usize, progress: f32) -> Element<'_, Message> {
        let items = self.stack.as_ref().map_or(&[][..], |s| &s.items[..]);
        let shown = &items[..count.min(items.len())];
        let rows = shown.len();
        let mut column = iced::widget::Column::new().width(Length::Fill);
        let footer = ui::secondary_button(dock::text::OPEN_IN_EVIDENCE, Some(Message::StackFolder));
        // Over the column, its right side on the top item's.
        let top =
            dock::FAN_HALF - dock::FAN_ICON / 2.0 - dock::fan_offset(rows.saturating_sub(1), rows);
        let footer = container(footer)
            .height(dock::STACK_FOOTER)
            .align_right(Length::Fill)
            .padding(iced::Padding::ZERO.right(top.max(0.0)));
        column = column.push(footer);
        // Top to bottom: the last of the items first, the first one nearest the icon.
        for (index, item) in shown.iter().enumerate().rev() {
            let picture = container(ui::icon(
                item.icon,
                dock::STACK_ICON,
                color::palette().text_secondary,
            ))
            .center(dock::FAN_ICON)
            .style(dock::fan_tile);
            let name = container(
                sherlock_ui::ellipsis::ellipsis(item.name.as_str()).size(dock::LABEL_SIZE),
            )
            .padding([dock::LABEL_PADDING.0, dock::LABEL_PADDING.1])
            .max_width(dock::FAN_LABEL_MAX)
            .style(dock::label(1.0));
            let row = row![name, picture]
                .spacing(dock::FAN_LABEL_GAP)
                .align_y(Alignment::Center);
            let button = iced::widget::button(row)
                .padding(0)
                .style(|_, _| iced::widget::button::Style::default())
                .on_press(Message::StackOpen(index));
            // Out of the icon: from the bottom, later for the higher rows; the spring overshoots
            // a little past 1, then settles on it.
            let delay = dock::FAN_STAGGER * index as f32;
            let t = ((progress - delay) / (1.0 - delay)).max(0.0);
            let rise = (index + 1) as f32 * dock::FAN_ROW * (1.0 - t);
            // The icons over the dock's icon, the higher ones leaning right.
            let right = dock::FAN_HALF - dock::FAN_ICON / 2.0 - dock::fan_offset(index, rows);
            let placed = container(reveal(button).offset(0.0, rise).opacity(t.min(1.0)))
                .height(dock::FAN_ROW)
                .align_right(Length::Fill)
                .align_y(Alignment::Center)
                .padding(iced::Padding::ZERO.right(right));
            column = column.push(placed);
        }
        container(column)
            .padding(iced::Padding::ZERO.bottom(dock::STACK_PADDING))
            .align_bottom(Length::Fill)
            .into()
    }

    /// The list of the stack (lot D-21k2), as a menu: its items with their glyph, a folder
    /// marked with a chevron and opened in Evidence, then « Ouvrir dans Evidence ».
    fn view_list<'a>(&'a self, header: Element<'a, Message>, count: usize) -> Element<'a, Message> {
        let items = self.stack.as_ref().map_or(&[][..], |s| &s.items[..]);
        let mut entries = vec![header];
        for (index, item) in items.iter().take(count).enumerate() {
            let glyph = ui::icon(
                item.icon,
                metrics::ICON_SIZE,
                color::palette().text_secondary,
            );
            let trailing = if item.folder {
                Trailing::Submenu
            } else {
                Trailing::None
            };
            let line = ui::menu_item_with(
                Some(glyph.into()),
                &item.name,
                trailing,
                Some(Message::StackOpen(index)),
            );
            entries.push(line.into());
        }
        entries.push(ui::menu_separator());
        entries.push(
            ui::menu_item(
                dock::text::OPEN_IN_EVIDENCE,
                Trailing::None,
                Some(Message::StackFolder),
            )
            .into(),
        );
        ui::menu(entries, dock::OWN_MENU_WIDTH)
            .height(Length::Fill)
            .into()
    }
}

fn place<'a>(content: impl Into<Element<'a, Message>>, x: f32, y: f32) -> Element<'a, Message> {
    pin(content).x(x).y(y).into()
}

/// An icon laid out at `rect`, drawn at `scale` from the middle of its side against the
/// `edge`, `lift` away from its place (a bounce), at `opacity`.
fn drawn(
    glyph_of: &Glyph,
    rect: Rectangle,
    edge: Edge,
    scale: f32,
    lift: f32,
    opacity: f32,
) -> Element<'static, Message> {
    let at = crate::layout::drawn(rect, edge, scale, lift);
    place(glyph(glyph_of, at.width, opacity), at.x, at.y)
}

/// An icon of side `side`, at `opacity`.
fn glyph(glyph: &Glyph, side: f32, opacity: f32) -> Element<'static, Message> {
    let faded = |c: color::Color| color::Color {
        a: c.a * opacity,
        ..c
    };
    match glyph {
        // The hole of an application dragged over the dock: room, nothing drawn (D-41d).
        Glyph::None => space().width(side).height(side).into(),
        // Smooth edges on the GPU, at rest and magnified, rasterized once per step (UI-05).
        Glyph::File(handle) => sherlock_ui::raster::icon(handle, side)
            .opacity(opacity)
            .into(),
        Glyph::Letter(letter) => {
            // At least a pixel: an icon that appears from nothing has no side yet, and text of
            // size 0 makes the text engine panic.
            let letter = text(letter.to_string())
                .size((side * dock::LETTER_RATIO).max(1.0))
                .font(typography::TITLE)
                .color(faded(dock::FALLBACK_LETTER));
            tile(letter, faded(dock::FALLBACK_TILE), side)
        }
    }
}

fn tile<'a>(
    content: impl Into<Element<'a, Message>>,
    tint: color::Color,
    side: f32,
) -> Element<'a, Message> {
    container(content)
        .width(side)
        .height(side)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(dock::tile(tint, side))
        .into()
}
