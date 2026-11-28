//! The window, after the « Réglages » board: the title bar, the sidebar with the search on
//! top, the page on the right. In a narrow window the sidebar folds away; a button of the
//! title bar slides it back over the page.

use iced::widget::{
    Column, Row, column, container, mouse_area, opaque, responsive, rich_text, row, space, span,
    stack, svg,
};
use iced::{Alignment, Element, Length, Padding, Size};
use sherlock_ui::animated::reveal;
use sherlock_ui::app_icon::AppIcon;
use sherlock_ui::component as ui;
use sherlock_ui::control_room::{self as cr, texts as t};
use sherlock_ui::icon::Icon;
use sherlock_ui::scroll::scroll;
use sherlock_ui::{color, metrics, theme, typography};

use crate::app::{App, Message, SEARCH, WindowAction};
use crate::search::{Page, Setting};

/// `fill` with its opacity scaled by `share`.
fn faded(fill: color::Color, share: f32) -> color::Color {
    color::Color {
        a: fill.a * share,
        ..fill
    }
}

/// Vertical rule between the sidebar and the page.
fn vline<'a>() -> Element<'a, Message> {
    container(space())
        .width(metrics::BORDER_WIDTH)
        .height(Length::Fill)
        .style(theme::divider)
        .into()
}

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        responsive(move |size| self.window(size)).into()
    }

    fn window(&self, size: Size) -> Element<'_, Message> {
        let compact = size.width < cr::COMPACT_WIDTH;
        let body: Element<'_, Message> = if compact {
            let mut layers = vec![self.page_area(size.width)];
            let now = self.now();
            let opacity = self.sidebar.opacity(now);
            if opacity > 0.0 {
                let scrim = container(space().width(Length::Fill).height(Length::Fill))
                    .style(move |theme| theme::veil(faded(theme::palette(theme).scrim, opacity)));
                layers.push(opaque(mouse_area(scrim).on_press(Message::ToggleSidebar)));
                let hidden = 1.0 - self.sidebar.progress(now);
                let panel = color::with_opacity(opacity, || self.sidebar());
                let panel = reveal(row![panel, vline()])
                    .opacity(opacity)
                    .offset(-hidden * cr::SIDEBAR_WIDTH, 0.0);
                layers.push(opaque(panel));
            }
            stack(layers).into()
        } else {
            let page = size.width - cr::SIDEBAR_WIDTH - metrics::BORDER_WIDTH;
            row![self.sidebar(), vline(), self.page_area(page)].into()
        };
        column![
            self.title_bar(compact),
            ui::divider(),
            container(body).height(Length::Fill)
        ]
        .into()
    }

    fn title_bar(&self, compact: bool) -> Element<'_, Message> {
        let icon = AppIcon::ControlRoom.at(metrics::APP_BADGE, 1.0);
        let badge = svg(svg::Handle::from_memory(icon.as_bytes()))
            .width(metrics::APP_BADGE)
            .height(metrics::APP_BADGE);
        let mut line = Row::new()
            .spacing(metrics::GAP_ICON)
            .align_y(Alignment::Center);
        if compact {
            line = line.push(ui::icon_button(
                Icon::Sidebar,
                t::SHOW_SIDEBAR,
                Some(Message::ToggleSidebar),
            ));
        }
        let controls = ui::window_controls(
            [t::MINIMIZE, t::MAXIMIZE, t::CLOSE],
            [
                Message::Window(WindowAction::Minimize),
                Message::Window(WindowAction::Maximize),
                Message::Window(WindowAction::Close),
            ],
        );
        let line = line
            .push(badge)
            .push(ui::body(t::APP_NAME).font(typography::UI_SEMIBOLD))
            .push(ui::ellipsis(self.page.name()).color(color::palette().text_tertiary))
            .push(space::horizontal())
            .push(controls);
        let (left, right) = metrics::TITLE_BAR_PADDING;
        let bar = container(line)
            .width(Length::Fill)
            .height(metrics::TITLE_BAR_HEIGHT)
            .padding(Padding {
                left,
                right,
                ..Padding::ZERO
            })
            .align_y(Alignment::Center)
            .style(theme::title_bar);
        mouse_area(bar)
            .on_press(Message::Window(WindowAction::Drag))
            .on_double_click(Message::Window(WindowAction::Maximize))
            .into()
    }

    /// The search, then the pages, or the settings found while a query is typed.
    fn sidebar(&self) -> Element<'_, Message> {
        let mut nav = Column::new().spacing(cr::NAV_GAP).push(
            container(ui::search_field(
                t::SEARCH,
                &self.query,
                Some(Message::Search),
                SEARCH,
                cr::SIDEBAR_WIDTH - 2.0 * cr::SIDEBAR_PADDING.1,
            ))
            .padding(Padding {
                bottom: cr::SEARCH_GAP,
                ..Padding::ZERO
            }),
        );
        if self.query.trim().is_empty() {
            for page in Page::ALL {
                let go = Some(Message::Go(page));
                nav = nav.push(ui::nav_item(
                    page.icon(),
                    page.name(),
                    None,
                    page == self.page,
                    go,
                ));
            }
        } else if self.hits.is_empty() {
            nav = nav.push(
                container(ui::description(t::NO_RESULT)).padding([0.0, metrics::NAV_ITEM_PADDING]),
            );
        } else {
            // One after the other, each fading in and coming up a little.
            let now = self.now();
            for (i, hit) in self.hits.iter().enumerate() {
                let start = self.hits_since + cr::HIT_STAGGER * i as u32;
                let shown = now.saturating_sub(start).as_secs_f32() / cr::PAGE_FADE.as_secs_f32();
                let shown = shown.clamp(0.0, 1.0);
                let line = color::with_opacity(shown, || self.hit(hit));
                nav = nav.push(
                    reveal(line)
                        .opacity(shown)
                        .offset(0.0, (1.0 - shown) * cr::HIT_RISE),
                );
            }
        }
        let (vertical, side) = cr::SIDEBAR_PADDING;
        container(scroll(nav).height(Length::Fill))
            .width(cr::SIDEBAR_WIDTH)
            .height(Length::Fill)
            .padding([vertical, side])
            .style(theme::sidebar)
            .into()
    }

    /// A setting found: its name, the part the query matched in green, its page below.
    fn hit<'a>(&'a self, hit: &'a crate::search::Hit) -> Element<'a, Message> {
        let (page, name, _) = hit.setting.entry();
        let p = color::palette();
        let mut spans: Vec<iced::widget::text::Span<'a, Message>> = Vec::new();
        let mut at = 0;
        for mark in &hit.marks {
            if mark.start < at {
                continue;
            }
            spans.push(span(&name[at..mark.start]));
            spans.push(
                span(&name[mark.clone()])
                    .color(p.accent_text)
                    .font(typography::UI_SEMIBOLD),
            );
            at = mark.end;
        }
        spans.push(span(&name[at..]));
        // A name too long for the sidebar is cut with « … », without its highlight.
        let room = cr::SIDEBAR_WIDTH - 2.0 * (cr::SIDEBAR_PADDING.1 + metrics::NAV_ITEM_PADDING);
        let fits =
            sherlock_ui::ellipsis::width(name, typography::UI_MEDIUM, typography::UI_SIZE) <= room;
        let title: Element<'a, Message> = match fits {
            true => rich_text(spans)
                .font(typography::UI_MEDIUM)
                .size(typography::UI_SIZE)
                .color(p.text_primary)
                .into(),
            false => ui::ellipsis(name)
                .font(typography::UI_MEDIUM)
                .color(p.text_primary)
                .into(),
        };
        let detail = ui::ellipsis(page.name())
            .size(typography::SMALL_SIZE)
            .color(p.text_secondary);
        let lines = column![title, detail].spacing(metrics::GAP_TIGHT);
        ui::picture_choice(
            container(lines).padding([metrics::GAP_TIGHT * 2.0, metrics::NAV_ITEM_PADDING]),
            metrics::NAV_ITEM_RADIUS,
            Some(Message::Open(hit.setting)),
        )
        .width(Length::Fill)
        .into()
    }

    /// The page, in a column no wider than `CONTENT_MAX_WIDTH`, coming in when it changes;
    /// the page left fades out under it, rising a little, and takes no click.
    fn page_area(&self, width: f32) -> Element<'_, Message> {
        let now = self.now();
        let opacity = self.page_slide.opacity(now);
        let rise = (1.0 - self.page_slide.progress(now)) * cr::PAGE_RISE;
        let page = scroll(self.page_content(self.page, width, opacity, rise))
            .id(crate::scroll::PAGE)
            .height(Length::Fill);
        let mut layers: Vec<Element<'_, Message>> = Vec::new();
        if let Some((left, at)) = self.leaving {
            let gone =
                (now.saturating_sub(at).as_secs_f32() / cr::PAGE_FADE.as_secs_f32()).min(1.0);
            let content = self.page_content(left, width, 1.0 - gone, -gone * cr::PAGE_RISE);
            layers.push(scroll(content).height(Length::Fill).into());
            // Nothing reaches the page left.
            layers.push(opaque(space().width(Length::Fill).height(Length::Fill)));
        }
        layers.push(page.into());
        container(stack(layers))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::window)
            .into()
    }

    /// The title and cards of `page`, at `opacity`, moved down by `rise`.
    fn page_content(
        &self,
        page: Page,
        width: f32,
        opacity: f32,
        rise: f32,
    ) -> Element<'_, Message> {
        let (vertical, horizontal) = metrics::PAGE_PADDING;
        let inner = (width - 2.0 * horizontal).min(cr::CONTENT_MAX_WIDTH);
        let content = color::with_opacity(opacity, || {
            let heading = column![
                ui::title(page.name()),
                ui::body(page.about())
                    .size(cr::SUBTITLE_SIZE)
                    .color(color::palette().text_secondary),
            ]
            .spacing(cr::TITLE_GAP);
            let cards = match page {
                Page::Sound => self.sound(inner),
                Page::Bluetooth => self.bluetooth_page(inner),
                Page::Appearance => self.appearance(inner),
                Page::Keyboard => self.keyboard(inner),
                Page::Touchpad => self.touchpad(inner),
                Page::Lock => self.lock(inner),
                Page::Clipboard => self.clipboard(inner),
                Page::DefaultApps => self.default_apps(inner),
            };
            Column::new()
                .push(heading)
                .extend(cards)
                .spacing(cr::CARD_GAP)
                .max_width(cr::CONTENT_MAX_WIDTH)
        });
        let page = reveal(content).opacity(opacity).offset(0.0, rise);
        container(page)
            .center_x(Length::Fill)
            .padding([vertical, horizontal])
            .into()
    }

    /// A card: its title if its rows need one, then its rows, a line between two.
    pub(crate) fn card<'a>(
        &self,
        title: Option<&'a str>,
        rows: Vec<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let mut column = Column::new();
        if let Some(title) = title {
            column = column.push(
                ui::body(title)
                    .size(typography::CARD_TITLE_SIZE)
                    .font(typography::UI_SEMIBOLD),
            );
        }
        for (i, line) in rows.into_iter().enumerate() {
            if i > 0 {
                column = column.push(ui::divider());
            }
            column = column.push(line);
        }
        ui::card(column).width(Length::Fill).into()
    }

    /// A setting: its name and help on the left, its control on the right; in a narrow page,
    /// a wide control goes under them, full width. Lit when the search opened it.
    pub(crate) fn row<'a>(
        &self,
        setting: Setting,
        width: f32,
        help: &'a str,
        control: Element<'a, Message>,
        wide: bool,
    ) -> Element<'a, Message> {
        let name = match setting {
            // The names out of their card are longer: the dock's are short inside it.
            Setting::DockSize => t::DOCK_SIZE,
            Setting::MagnificationFactor => t::MAGNIFICATION_FACTOR,
            Setting::AutoHide => t::AUTO_HIDE,
            Setting::Position => t::POSITION,
            _ => setting.entry().1,
        };
        // A setting whose file cannot be changed says so, under its disabled control.
        let failed = matches!(setting, Setting::App(kind) if self.apps_failed.contains(&kind));
        let help = match setting.file().is_none_or(|file| self.can_change(file)) && !failed {
            true => help,
            false => t::LOCKED_FILE,
        };
        let label = column![
            ui::body(name).font(typography::UI_MEDIUM),
            ui::body(help)
                .size(typography::SMALL_SIZE)
                .color(color::palette().text_secondary),
        ]
        .width(Length::Fill);
        let stacked = wide && width < cr::STACK_WIDTH;
        let line: Element<'a, Message> = if stacked {
            column![label, control]
                .spacing(metrics::GAP_ICON)
                .padding([metrics::GAP_ICON, 0.0])
                .into()
        } else {
            let control = if wide {
                container(control).width(cr::CONTROL_WIDTH).into()
            } else {
                control
            };
            row![label, control]
                .spacing(metrics::GAP_ROW)
                .align_y(Alignment::Center)
                .into()
        };
        let lit = match &self.spotlight {
            Some((lit, fade)) if *lit == setting => fade.value(self.now()),
            _ => 0.0,
        };
        // A row is at least as high as the board's, taller when its texts wrap.
        let line =
            row![space().height(metrics::SETTING_ROW_HEIGHT), line].align_y(Alignment::Center);
        container(line)
            .id(crate::scroll::row(setting))
            .width(Length::Fill)
            .style(move |theme| {
                let fill = faded(theme::palette(theme).selected_row, lit);
                iced::widget::container::Style {
                    border: iced::border::rounded(metrics::LIST_ROW_RADIUS),
                    ..theme::veil(fill)
                }
            })
            .into()
    }
}
