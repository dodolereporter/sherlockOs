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
        if let Some(label) = self.view_label(&layout) {
            layers.push(label);
        }
        // The pointer is tracked over the whole surface: `mouse_area` also reports where the
        // pointer enters, which the bare enter event does not carry.
        let layers = stack(layers).width(Length::Fill).height(Length::Fill);
        mouse_area(layers)
            .on_move(|point| Message::Pointer(Pointer::Moved(point)))
            .on_exit(Message::Pointer(Pointer::Left))
            .on_press(Message::Pointer(Pointer::Pressed(mouse::Button::Left)))
            .on_release(Message::Pointer(Pointer::Released))
            .on_right_press(Message::Pointer(Pointer::Pressed(mouse::Button::Right)))
            .interaction(self.interaction())
            .on_middle_press(Message::Pointer(Pointer::Pressed(mouse::Button::Middle)))
            .into()
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
            let index = self.index_of(&menu.target);
            let name = index
                .and_then(|i| self.items.get(i))
                .map_or("", |i| i.name.as_str());
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
            ui::menu(entries, dock::MENU_WIDTH).height(Length::Fill)
        });
        // It unfolds from its icon: from the side facing the dock.
        let origin = match self.geometry.position {
            Edge::Bottom => (0.5, 1.0),
            Edge::Top => (0.5, 0.0),
            Edge::Left => (0.0, 0.5),
            Edge::Right => (1.0, 0.5),
        };
        reveal(panel)
            .opacity(opacity)
            .scale(menu.unfold.scale(self.now), origin)
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
        // Smooth edges on the GPU, at rest and magnified, rasterized once per step (UI-05).
        Glyph::File(handle) => sherlock_ui::raster::icon(handle, side)
            .opacity(opacity)
            .into(),
        Glyph::Letter(letter) => {
            let letter = text(letter.to_string())
                .size(side * dock::LETTER_RATIO)
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
