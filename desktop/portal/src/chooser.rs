//! The picker's window (`sherlock-portal --pick`, lot D-28d): over the whole screen, a veil and
//! a glass panel where the user chooses the screen or the window to share, each with a
//! thumbnail, as macOS asks it. It grows in and fades in, and shrinks and fades out once the
//! user shares or cancels; the program then writes its answer (`picker.rs`) and ends.
//!
//! Keys: the arrows move the choice, Tab goes from screens to windows, Enter shares, Escape
//! cancels. A click beside the panel cancels too.

use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant};

use iced::keyboard::{self, key::Named};
use iced::widget::{Space, column, container, image, mouse_area, row, scrollable, stack, text};
use iced::window::{self, Id};
use iced::{Alignment, Element, Event, Length, Size, Subscription, Task, event};
use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use iced_exwlshell::to_layer_message;
use sherlock_ui::animated::reveal;
use sherlock_ui::component::{self as ui};
use sherlock_ui::ellipsis::ellipsis;
use sherlock_ui::motion::Animation;
use sherlock_ui::screen_share::{self as look, text as texts};
use sherlock_ui::{color, metrics, theme, typography};

use crate::capture::{self, Picture, Source};
use crate::picker::Request;
use crate::screencast::{MONITOR, WINDOW};

/// Name of the layer-shell surface, readable by the compositor.
pub const NAMESPACE: &str = "sherlock-portal-picker";

/// How long the thumbnails may take, all together: the panel shows without those late.
const THUMBNAILS: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Screens,
    Windows,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Surface(Id),
    Resized(Size),
    Frame(Instant),
    Kind(Kind),
    /// A tile clicked, by its source's number in the request.
    Choose(usize),
    Share,
    Cancel,
    /// A press on the panel, beside its controls: it stays.
    Inside,
    Key(Named),
}

pub struct Picker {
    request: Request,
    /// The thumbnail of each source, by its number, if it came in time.
    thumbnails: Vec<Option<image::Handle>>,
    kind: Kind,
    chosen: Vec<usize>,
    size: Size,
    presence: Animation,
    origin: Instant,
    now: Duration,
    /// The answer, once the user shared or cancelled: the panel fades out, then it is written.
    answer: Option<Vec<usize>>,
}

/// The thumbnails of `sources`, taken side by side; those not there after `THUMBNAILS` are
/// left out.
pub fn thumbnails(sources: &[Source]) -> Vec<Option<image::Handle>> {
    let stop = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = std::sync::mpsc::channel();
    for (i, source) in sources.iter().enumerate() {
        let (sender, source, stop) = (sender.clone(), source.clone(), Arc::clone(&stop));
        let spawned = std::thread::Builder::new()
            .name(String::from("thumbnail"))
            .spawn(move || {
                let picture = capture::picture(&source, &stop).ok();
                let _ = sender.send((i, picture.map(|p| thumbnail(&p))));
            });
        if let Err(err) = spawned {
            tracing::warn!("thumbnail: {err}");
        }
    }
    drop(sender);
    let mut thumbnails = vec![None; sources.len()];
    let until = Instant::now() + THUMBNAILS;
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(left) {
            Ok((i, handle)) => thumbnails[i] = handle,
            Err(_) => break,
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    thumbnails
}

/// `picture` shrunk to `look::THUMB_PIXELS` wide at most, by averaging boxes of pixels, as
/// RGBA.
pub fn thumbnail(picture: &Picture) -> image::Handle {
    let (w, h) = (picture.width.max(1), picture.height.max(1));
    let step = w.div_ceil(look::THUMB_PIXELS).max(1);
    let (tw, th) = ((w / step).max(1), (h / step).max(1));
    let mut rgba = Vec::with_capacity((tw * th * 4) as usize);
    for ty in 0..th {
        for tx in 0..tw {
            let mut sum = [0u32; 3];
            for y in ty * step..(ty + 1) * step {
                for x in tx * step..(tx + 1) * step {
                    let at = ((y * w + x) * 4) as usize;
                    let Some(&[b, g, r, _]) = picture
                        .bgrx
                        .get(at..at + 4)
                        .and_then(|p| <&[u8; 4]>::try_from(p).ok())
                    else {
                        continue;
                    };
                    sum[0] += u32::from(r);
                    sum[1] += u32::from(g);
                    sum[2] += u32::from(b);
                }
            }
            let n = step * step;
            rgba.extend([
                (sum[0] / n) as u8,
                (sum[1] / n) as u8,
                (sum[2] / n) as u8,
                255,
            ]);
        }
    }
    image::Handle::from_rgba(tw, th, rgba)
}

impl Picker {
    pub fn new(request: Request, thumbnails: Vec<Option<image::Handle>>) -> Self {
        let screens = request.selection.types & MONITOR != 0;
        let kind = if screens {
            Kind::Screens
        } else {
            Kind::Windows
        };
        let mut picker = Self {
            request,
            thumbnails,
            kind,
            chosen: Vec::new(),
            size: Size::new(1920.0, 1080.0),
            presence: Animation::new(look::OPEN_SPRING, 0.0, 0.0, Duration::ZERO),
            origin: Instant::now(),
            now: Duration::ZERO,
            answer: None,
        };
        picker.choose_first();
        picker
    }

    /// The sources of the kind shown, by number.
    fn shown(&self) -> Vec<usize> {
        let kind = self.kind;
        (self.request.offered())
            .filter(|(_, s)| {
                matches!(
                    (kind, s),
                    (Kind::Screens, Source::Screen(_)) | (Kind::Windows, Source::Window(_))
                )
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// One screen alone is chosen already: sharing it is one press of Enter.
    fn choose_first(&mut self) {
        let shown = self.shown();
        self.chosen = match (self.kind, shown.as_slice()) {
            (Kind::Screens, [only]) => vec![*only],
            _ => Vec::new(),
        };
    }

    /// Both kinds may be shared: the switch between them shows.
    fn both_kinds(&self) -> bool {
        self.request.selection.types & (MONITOR | WINDOW) == MONITOR | WINDOW
    }

    pub fn presence(&self) -> f64 {
        self.presence.value(self.now)
    }

    fn animating(&self) -> bool {
        !self.presence.is_done(self.now, look::OPEN_PRECISION)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let frames = match self.animating() {
            true => window::frames().map(Message::Frame),
            false => Subscription::none(),
        };
        Subscription::batch([event::listen_with(on_event), frames])
    }

    fn clock(&mut self) -> Duration {
        self.now = self.now.max(self.origin.elapsed());
        self.now
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        if self.answer.is_some() && !matches!(message, Message::Frame(_) | Message::Resized(_)) {
            return Task::none();
        }
        match message {
            Message::Surface(id) => {
                let now = self.clock();
                self.presence = Animation::new(look::OPEN_SPRING, 0.0, 1.0, now);
                return window::size(id).map(Message::Resized);
            }
            Message::Resized(size) => self.size = size,
            Message::Frame(at) => {
                self.now = self.now.max(at.saturating_duration_since(self.origin));
                if let Some(answer) = &self.answer
                    && !self.animating()
                {
                    write_answer(answer);
                    return iced::exit();
                }
            }
            Message::Kind(kind) => {
                self.kind = kind;
                self.choose_first();
            }
            Message::Choose(i) => self.choose(i),
            Message::Share if !self.chosen.is_empty() => self.close(self.chosen.clone()),
            Message::Share => {}
            Message::Cancel => self.close(Vec::new()),
            Message::Key(key) => return self.key(key),
            _ => {}
        }
        Task::none()
    }

    fn choose(&mut self, i: usize) {
        if self.request.selection.multiple {
            match self.chosen.iter().position(|c| *c == i) {
                Some(at) => {
                    self.chosen.remove(at);
                }
                None => self.chosen.push(i),
            }
        } else {
            self.chosen = vec![i];
        }
    }

    fn key(&mut self, key: Named) -> Task<Message> {
        match key {
            Named::Escape => self.close(Vec::new()),
            Named::Enter => return Task::done(Message::Share),
            Named::Tab if self.both_kinds() => {
                let kind = match self.kind {
                    Kind::Screens => Kind::Windows,
                    Kind::Windows => Kind::Screens,
                };
                return Task::done(Message::Kind(kind));
            }
            Named::ArrowLeft | Named::ArrowRight | Named::ArrowUp | Named::ArrowDown => {
                let shown = self.shown();
                let columns = look::columns(look::width(self.size.width)) as isize;
                let step = match key {
                    Named::ArrowLeft => -1,
                    Named::ArrowRight => 1,
                    Named::ArrowUp => -columns,
                    _ => columns,
                };
                let at = (self.chosen.last())
                    .and_then(|c| shown.iter().position(|s| s == c))
                    .map_or(0, |p| {
                        (p as isize + step).clamp(0, shown.len() as isize - 1)
                    });
                if let Some(i) = shown.get(at as usize) {
                    self.chosen = vec![*i];
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// Fades the panel out; the answer is written at the last frame.
    fn close(&mut self, answer: Vec<usize>) {
        let now = self.clock();
        self.presence.retarget(now, 0.0);
        self.answer = Some(answer);
    }

    pub fn view(&self, _id: Id) -> Element<'_, Message> {
        let shown = self.presence() as f32;
        let opacity = shown.clamp(0.0, 1.0);
        let scale = look::OPEN_SCALE + (1.0 - look::OPEN_SCALE) * shown;
        let width = look::width(self.size.width);
        let panel = mouse_area(self.view_panel(width)).on_press(Message::Inside);
        let panel = reveal(panel).scale(scale, (0.5, 0.5));
        let veil = mouse_area(
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::scrim),
        )
        .on_press(Message::Cancel);
        let centred = container(panel).center(Length::Fill);
        reveal(stack![veil, centred]).opacity(opacity).into()
    }

    fn view_panel(&self, width: f32) -> Element<'_, Message> {
        let app = look::app_name(&self.request.app_id).unwrap_or(texts::SOME_APP);
        let title = text(texts::title(app))
            .font(typography::UI_SEMIBOLD)
            .size(typography::TITLE_SMALL_SIZE);
        let mut parts: Vec<Element<'_, Message>> = vec![title.into()];
        if self.both_kinds() {
            let choice = |label, kind| (label, self.kind == kind, Some(Message::Kind(kind)));
            let switch = ui::segmented(vec![
                choice(texts::SCREENS, Kind::Screens),
                choice(texts::WINDOWS, Kind::Windows),
            ]);
            parts.push(container(switch).width(look::THUMB_WIDTH * 1.5).into());
        }
        parts.push(self.view_grid(width));
        let cancel = ui::secondary_button(texts::CANCEL, Some(Message::Cancel));
        let share = (!self.chosen.is_empty()).then_some(Message::Share);
        let share = ui::primary_button(texts::SHARE, share);
        let buttons = row![Space::new().width(Length::Fill), cancel, share]
            .spacing(metrics::GAP_INLINE)
            .align_y(Alignment::Center);
        parts.push(buttons.into());
        let content = iced::widget::Column::with_children(parts)
            .spacing(look::GAP)
            .align_x(Alignment::Center)
            .padding(look::PADDING);
        let panel = container(content).width(width).style(theme::glass);
        let tall = self.size.height * look::HEIGHT_SHARE;
        container(panel).max_height(tall).into()
    }

    fn view_grid(&self, width: f32) -> Element<'_, Message> {
        let shown = self.shown();
        if shown.is_empty() {
            return container(text(texts::NOTHING).style(sherlock_ui::panels::secondary_text))
                .center_x(Length::Fill)
                .padding(look::PADDING)
                .into();
        }
        let columns = look::columns(width);
        let rows = shown.chunks(columns).map(|line| {
            let tiles = line.iter().map(|i| self.view_tile(*i));
            iced::widget::Row::with_children(tiles)
                .spacing(look::TILE_GAP)
                .into()
        });
        let grid = iced::widget::Column::with_children(rows).spacing(look::TILE_GAP);
        scrollable(container(grid).center_x(Length::Fill))
            .height(Length::Shrink)
            .into()
    }

    fn view_tile(&self, i: usize) -> Element<'_, Message> {
        let chosen = self.chosen.contains(&i);
        let source = &self.request.sources[i];
        let picture: Element<'_, Message> = match self.thumbnails.get(i).cloned().flatten() {
            Some(handle) => image(handle)
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => Space::new().into(),
        };
        let frame = container(picture)
            .width(look::THUMB_WIDTH)
            .height(look::THUMB_HEIGHT)
            .padding(look::RING)
            .style(theme::picture_ring(chosen, look::THUMB_RADIUS));
        let (name, detail) = label(source, i, &self.request.sources);
        let name = ellipsis(name).width(look::THUMB_WIDTH);
        let mut lines = column![frame, name]
            .spacing(metrics::GAP_TIGHT)
            .align_x(Alignment::Center);
        if let Some(detail) = detail {
            let detail = ellipsis(detail)
                .size(typography::SMALL_SIZE)
                .width(look::THUMB_WIDTH)
                .style(sherlock_ui::panels::secondary_text);
            lines = lines.push(detail);
        }
        ui::picture_choice(lines, look::THUMB_RADIUS, Some(Message::Choose(i))).into()
    }
}

/// The name and the detail under a tile: a screen's description, or its number; a window's
/// title, and its application.
fn label(source: &Source, i: usize, all: &[Source]) -> (String, Option<String>) {
    match source {
        Source::Screen(screen) => {
            let number = all[..=i]
                .iter()
                .filter(|s| matches!(s, Source::Screen(_)))
                .count();
            let name = match screen.description.is_empty() {
                true => texts::screen(number),
                false => screen.description.clone(),
            };
            (name, Some(screen.name.clone()))
        }
        Source::Window(window) => {
            let title = match window.title.is_empty() {
                true => String::from(texts::UNTITLED),
                false => window.title.clone(),
            };
            let app = look::app_name(&window.app_id).map(str::to_owned);
            (title, app)
        }
    }
}

/// The chosen numbers on the standard output, one a line: the portal reads them.
fn write_answer(answer: &[usize]) {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    for i in answer {
        let _ = writeln!(out, "{i}");
    }
    let _ = out.flush();
}

pub fn on_event(event: Event, _: event::Status, _: Id) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(key),
            ..
        }) => Some(Message::Key(key)),
        Event::Window(window::Event::Resized(size)) => Some(Message::Resized(size)),
        _ => None,
    }
}

/// Runs the picker for `request` until the user shares or cancels.
pub fn run(request: Request) -> Result<(), iced_exwlshell::Error> {
    apply_theme();
    let sources = request.sources.clone();
    let thumbnails = thumbnails(&sources);
    let first = std::sync::Mutex::new(Some(Picker::new(request, thumbnails)));
    let boot = move || {
        let picker = first.lock().ok().and_then(|mut p| p.take());
        let picker =
            picker.unwrap_or_else(|| Picker::new(crate::picker::Request::decode(""), Vec::new()));
        (picker, Task::none())
    };
    let layer = LayerShellSettings {
        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
        layer: Layer::Overlay,
        size: LayerSize::FILL,
        exclusive_zone: -1,
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        start_mode: StartMode::Active,
        ..LayerShellSettings::default()
    };
    iced_exwlshell::daemon(boot, namespace, Picker::update, Picker::view)
        .theme(|_: &Picker, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: color::TRANSPARENT.into(),
            text_color: color::palette().text_primary.into(),
        })
        .subscription(Picker::subscription)
        .on_new_shell(|info: ShellInfo| {
            (info.shell == ShellType::LayerShell).then_some(Message::Surface(info.window))
        })
        .settings(iced::Settings {
            fonts: typography::FONTS.map(std::borrow::Cow::Borrowed).to_vec(),
            default_font: typography::UI,
            default_text_size: typography::UI_SIZE.into(),
            ..iced::Settings::default()
        })
        .wl_settings(ExWlSettings {
            layer_settings: layer,
            ..ExWlSettings::default()
        })
        .run()
}

/// Makes the theme of the appearance file the active one, as the launcher does.
fn apply_theme() {
    let Some(path) = appearance_config::path() else {
        return;
    };
    match appearance_config::AppearanceConfig::load(&path) {
        Ok((config, _)) => {
            let now = appearance_config::TimeOfDay::at(appearance_config::epoch_now());
            if let Ok(now) = now {
                color::set_active(config.theme_at(now).0);
            }
        }
        Err(error) => tracing::warn!(%error, "cannot read the theme"),
    }
}

fn namespace() -> String {
    String::from(NAMESPACE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{Screen, Window};
    use crate::screencast::Selection;

    fn request(types: u32, multiple: bool) -> Request {
        let screen = |name: &str| {
            Source::Screen(Screen {
                name: name.to_owned(),
                description: String::new(),
            })
        };
        Request {
            app_id: String::from("com.discordapp.Discord"),
            selection: Selection {
                types,
                multiple,
                cursor: true,
            },
            sources: vec![
                screen("eDP-1"),
                Source::Window(Window {
                    identifier: String::from("w1"),
                    app_id: String::from("firefox"),
                    title: String::new(),
                }),
                screen("HDMI-A-1"),
            ],
        }
    }

    #[test]
    fn the_screens_show_first_and_the_windows_after_tab() {
        let mut picker = Picker::new(request(MONITOR | WINDOW, false), Vec::new());
        assert_eq!(picker.shown(), vec![0, 2]);
        assert!(picker.chosen.is_empty(), "two screens: none chosen yet");
        let _ = picker.update(Message::Key(Named::Tab));
        let _ = picker.update(Message::Kind(Kind::Windows));
        assert_eq!(picker.shown(), vec![1]);
        // Windows only asked for: no switch, the windows at once.
        let picker = Picker::new(request(WINDOW, false), Vec::new());
        assert!(!picker.both_kinds());
        assert_eq!(picker.shown(), vec![1]);
    }

    #[test]
    fn one_choice_or_several_and_the_arrows_move_it() {
        let mut picker = Picker::new(request(MONITOR, false), Vec::new());
        let _ = picker.update(Message::Choose(0));
        let _ = picker.update(Message::Choose(2));
        assert_eq!(picker.chosen, vec![2]);
        let _ = picker.update(Message::Key(Named::ArrowLeft));
        assert_eq!(picker.chosen, vec![0]);
        // At the edge, it stays.
        let _ = picker.update(Message::Key(Named::ArrowLeft));
        assert_eq!(picker.chosen, vec![0]);
        let mut several = Picker::new(request(MONITOR, true), Vec::new());
        let _ = several.update(Message::Choose(0));
        let _ = several.update(Message::Choose(2));
        let _ = several.update(Message::Choose(0));
        assert_eq!(several.chosen, vec![2]);
    }

    #[test]
    fn sharing_needs_a_choice_and_closing_ignores_what_follows() {
        let mut picker = Picker::new(request(MONITOR, false), Vec::new());
        let _ = picker.update(Message::Share);
        assert_eq!(picker.answer, None, "nothing chosen");
        let _ = picker.update(Message::Choose(2));
        let _ = picker.update(Message::Share);
        assert_eq!(picker.answer, Some(vec![2]));
        let _ = picker.update(Message::Cancel);
        assert_eq!(picker.answer, Some(vec![2]), "the first answer stays");
    }

    #[test]
    fn a_window_without_title_says_so_and_the_screens_are_numbered() {
        let request = request(MONITOR | WINDOW, false);
        let (name, detail) = label(&request.sources[1], 1, &request.sources);
        assert_eq!(
            (name.as_str(), detail.as_deref()),
            (texts::UNTITLED, Some("firefox"))
        );
        let (name, _) = label(&request.sources[2], 2, &request.sources);
        assert_eq!(name, texts::screen(2));
    }

    #[test]
    fn a_thumbnail_is_the_picture_shrunk_in_rgba() {
        let picture = Picture {
            width: 4,
            height: 2,
            bgrx: [[10, 20, 30, 0]; 8].concat(),
        };
        // Already small: the same size, red and blue swapped back.
        let handle = thumbnail(&picture);
        let image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } = handle
        else {
            panic!("RGBA");
        };
        assert_eq!((width, height), (4, 2));
        assert_eq!(&pixels[..4], &[30, 20, 10, 255]);
    }
}
