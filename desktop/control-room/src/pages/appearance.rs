//! The Appearance page: the theme and the hours of its automatic mode, the wallpaper, the dock.

use appearance_config::Mode;
use dock_config::{AutoHide, Position};
use iced::Element;
use iced::widget::{Column, Row, column, container, image, space};
use sherlock_ui::component as ui;
use sherlock_ui::control_room::{self as cr, texts as t};
use sherlock_ui::{color, metrics, theme, typography, wallpapers};

use crate::app::{App, Change, Message, hours};
use crate::search::Setting;
use crate::settings::File;

impl App {
    pub(crate) fn appearance(&self, width: f32) -> Vec<Element<'_, Message>> {
        let s = &self.settings;
        let set = |change| self.message(change);
        let mode = s.appearance.mode;
        let modes = [
            (t::DARK, Mode::Dark),
            (t::LIGHT, Mode::Light),
            (t::AUTO, Mode::Auto),
        ]
        .map(|(label, m)| (label, m == mode, set(Change::Mode(m))));
        let mut theme_rows = vec![self.row(
            Setting::Theme,
            width,
            t::THEME_HELP,
            ui::segmented(modes.to_vec()),
            true,
        )];
        if mode == Mode::Auto {
            let (light, dark) = (s.appearance.light_from, s.appearance.dark_from);
            let menu = |current, other, change: fn(_) -> Change| {
                let chosen = crate::app::Hour(appearance_config::TimeOfDay::minutes(current));
                if !self.can_change(File::Appearance) {
                    return ui::description(chosen.to_string()).into();
                }
                ui::dropdown(
                    hours(current, other),
                    Some(chosen),
                    move |hour| Message::Set(change(hour)),
                    cr::HOUR_WIDTH,
                )
            };
            let light_menu = menu(light, dark, Change::LightFrom);
            let dark_menu = menu(dark, light, Change::DarkFrom);
            theme_rows.push(self.row(
                Setting::LightFrom,
                width,
                t::LIGHT_FROM_HELP,
                light_menu,
                false,
            ));
            theme_rows.push(self.row(
                Setting::DarkFrom,
                width,
                t::DARK_FROM_HELP,
                dark_menu,
                false,
            ));
        }

        let follow = ui::toggle(
            s.wallpaper.follow_theme,
            self.enabled(File::Wallpaper, |on| Message::Set(Change::FollowTheme(on))),
        );
        let mut wallpaper_rows = Vec::new();
        if let Some(path) = &s.wallpaper.path {
            let own = column![
                ui::body(t::OWN_IMAGE).font(typography::UI_MEDIUM),
                ui::ellipsis(path.display().to_string())
                    .size(typography::SMALL_SIZE)
                    .color(color::palette().text_secondary),
                ui::body(t::OWN_IMAGE_HELP)
                    .size(typography::SMALL_SIZE)
                    .color(color::palette().text_secondary),
            ];
            wallpaper_rows.push(container(own).padding([metrics::GAP_ICON, 0.0]).into());
        }
        wallpaper_rows.push(self.wallpapers(width));
        wallpaper_rows.push(self.row(
            Setting::FollowTheme,
            width,
            t::FOLLOW_THEME_HELP,
            follow.into(),
            false,
        ));

        let d = &s.dock;
        let hide = [
            (t::HIDE_NEVER, AutoHide::Never),
            (t::HIDE_SMART, AutoHide::Smart),
            (t::HIDE_ALWAYS, AutoHide::Always),
        ]
        .map(|(label, h)| (label, h == d.auto_hide, set(Change::AutoHide(h))));
        let edges = [
            (t::BOTTOM, Position::Bottom),
            (t::LEFT, Position::Left),
            (t::RIGHT, Position::Right),
            (t::TOP, Position::Top),
        ]
        .map(|(label, p)| (label, p == d.position, set(Change::Position(p))));
        let size = ui::slider(
            self.dock_size(),
            self.enabled(File::Dock, |v| Message::Set(Change::DockSize(v))),
        );
        let magnify = ui::toggle(
            d.magnification,
            self.enabled(File::Dock, |on| Message::Set(Change::Magnification(on))),
        );
        let factor = ui::slider(
            self.magnification_factor(),
            self.enabled(File::Dock, |v| Message::Set(Change::MagnificationFactor(v)))
                .filter(|_| d.magnification),
        );
        let dock_rows = vec![
            self.row(Setting::DockSize, width, t::DOCK_SIZE_HELP, size, true),
            self.row(
                Setting::Magnification,
                width,
                t::MAGNIFICATION_HELP,
                magnify.into(),
                false,
            ),
            self.row(
                Setting::MagnificationFactor,
                width,
                t::MAGNIFICATION_FACTOR_HELP,
                factor,
                true,
            ),
            self.row(
                Setting::AutoHide,
                width,
                t::AUTO_HIDE_HELP,
                ui::segmented(hide.to_vec()),
                true,
            ),
            self.row(
                Setting::Position,
                width,
                t::POSITION_HELP,
                ui::segmented(edges.to_vec()),
                true,
            ),
        ];
        vec![
            self.card(None, theme_rows),
            self.card(Some(t::WALLPAPER), wallpaper_rows),
            self.card(Some(t::DOCK), dock_rows),
        ]
    }

    /// The installed wallpapers, as many to a line as the card holds; the chosen one ringed.
    fn wallpapers(&self, width: f32) -> Element<'_, Message> {
        // Found off the interface thread: never a look at the disk while drawing.
        let Some(installed) = self.installed.clone() else {
            return space().height(cr::WALLPAPER_TILE.1).into();
        };
        if installed.is_empty() {
            return container(ui::description(t::NO_WALLPAPER))
                .padding([metrics::GAP_ICON, 0.0])
                .into();
        }
        // As many to a line as the card holds at their smallest, then grown to fill it.
        let frame = cr::WALLPAPER_RING + cr::WALLPAPER_RING_GAP + metrics::GAP_TIGHT;
        let gap = cr::WALLPAPER_GAP;
        let inside = width - 2.0 * metrics::CARD_PADDING.1;
        let smallest = cr::WALLPAPER_TILE.0 + 2.0 * frame;
        let per_line = (((inside + gap) / (smallest + gap)) as usize).max(1);
        let share = (inside - gap * (per_line - 1) as f32) / per_line as f32 - 2.0 * frame;
        let tile = share.clamp(cr::WALLPAPER_TILE.0, cr::WALLPAPER_TILE_MAX);
        let chosen = self.wallpaper();
        let tiles: Vec<Element<'_, Message>> = installed
            .into_iter()
            .map(|w| self.wallpaper_tile(w, chosen == Some(w.id), tile))
            .collect();
        let mut lines = Column::new()
            .spacing(cr::WALLPAPER_GAP)
            .padding([metrics::GAP_ICON, 0.0]);
        let mut tiles = tiles.into_iter().peekable();
        while tiles.peek().is_some() {
            let line: Vec<_> = tiles.by_ref().take(per_line).collect();
            lines = lines.push(Row::with_children(line).spacing(cr::WALLPAPER_GAP));
        }
        lines.into()
    }

    fn wallpaper_tile(
        &self,
        wallpaper: &'static wallpapers::Wallpaper,
        chosen: bool,
        width: f32,
    ) -> Element<'_, Message> {
        let height = width * cr::WALLPAPER_TILE.1 / cr::WALLPAPER_TILE.0;
        let picture: Element<'_, Message> = match self.thumbnails.get(wallpaper.id) {
            Some(handle) => image(handle.clone())
                .width(width)
                .height(height)
                .opacity(color::opacity())
                .border_radius(cr::WALLPAPER_RADIUS)
                .into(),
            // Not read yet, or unreadable: the desktop's own color.
            None => container(space())
                .width(width)
                .height(height)
                .style(|theme| theme::veil(theme::palette(theme).desktop))
                .into(),
        };
        let ring = cr::WALLPAPER_RING + cr::WALLPAPER_RING_GAP;
        let ringed = container(picture)
            .padding(ring)
            .style(theme::picture_ring(chosen, cr::WALLPAPER_RADIUS + ring));
        let name = ui::ellipsis(wallpaper.name)
            .size(typography::SMALL_SIZE)
            .color(color::palette().text_secondary)
            .width(width + 2.0 * ring);
        let content = column![ringed, name].spacing(metrics::GAP_TIGHT);
        ui::picture_choice(
            content,
            cr::WALLPAPER_RADIUS,
            self.message(Change::Wallpaper(wallpaper.id)),
        )
        .into()
    }
}
