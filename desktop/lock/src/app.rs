//! The lock screen: one lock surface per screen. At rest, every screen shows the background,
//! the date and the time. A key, a click or the pointer bring the form, on the screen of the
//! pointer (the one whose lock surface the compositor gives the keyboard to): Sherlock in 3D in
//! his bubble, the user's name, the password pill and the machine buttons (`form`). Every size
//! follows the height of its screen (`sherlock_ui::lock::screen`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iced::futures::channel::mpsc;
use iced::widget::{Space, button, column, container, image, row, stack, svg, text, text_input};
use iced::{
    Alignment, Background, Border, Color as IcedColor, ContentFit, Element, Event, Font, Length,
    Padding, Point, Size, Subscription, Task, keyboard, mouse, window,
};
use iced_exwlshell::to_exwlshell_message;
use sherlock_3d::{Model, Sherlock3d};
use sherlock_ui::color::{self, Color};
use sherlock_ui::component;
use sherlock_ui::icon::Icon;
use sherlock_ui::lock::screen::{self as ui, Colors};
use sherlock_ui::motion::Animation;
use sherlock_ui::{metrics, motion, strings, typography};
use system_services::secret::Secret;

use crate::background::{Picture, Pictures};
use crate::form::{self, Form, Input, Outcome};
use crate::machine::{Armed, Machine, Runner};
use crate::pam::{AuthError, Authenticator};

/// The still picture of Sherlock, until his model is read or without it.
const SHERLOCK: &[u8] = include_bytes!("../../../docs/design/images/sherlock-neutre.png");

/// The model of Sherlock, once read (it is not part of `Message`'s `Debug`).
#[derive(Clone)]
pub struct Mascot(pub Arc<Model>);

impl std::fmt::Debug for Mascot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Mascot")
    }
}

/// What the text field holds in place of the `i`-th character of the hidden password (lot
/// D-25m): a character of the private use area, which nobody types, different at each place so
/// that an erase in the middle is found where it was; the field draws dots anyway.
fn mask(i: usize) -> char {
    char::from_u32(0xE000 + (i % 6400) as u32).unwrap_or('\u{E000}')
}

/// The masks of a hidden password of `length` characters.
fn masks(length: usize) -> String {
    (0..length).map(mask).collect()
}

/// The password after the field went from `shown` (what it held: `password`, or its masks) to
/// `new`: the characters `new` kept at both ends stand for the password's, the ones between are
/// what was typed or pasted (lot D-25m). Built in one allocation, never grown, so that no copy is
/// left behind; the caller's `new` is a `Secret` too.
pub(crate) fn edited(password: &Secret, shown: &Secret, new: &str) -> Secret {
    let (old, shown) = (password.as_str(), shown.as_str());
    let length = shown.chars().count();
    let prefix = shown
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let rest = length.min(new.chars().count()) - prefix;
    let suffix = shown
        .chars()
        .rev()
        .zip(new.chars().rev())
        .take(rest)
        .take_while(|(a, b)| a == b)
        .count();
    let byte = |text: &str, chars: usize| {
        text.char_indices()
            .nth(chars)
            .map_or(text.len(), |(i, _)| i)
    };
    let new_end = new.chars().count() - suffix;
    let mut text = String::with_capacity(old.len() + new.len());
    text.push_str(&old[..byte(old, prefix)]);
    text.push_str(&new[byte(new, prefix)..byte(new, new_end)]);
    text.push_str(&old[byte(old, length - suffix)..]);
    Secret::new(text)
}

/// The id of the password field.
pub fn field() -> iced::widget::Id {
    iced::widget::Id::new("password")
}

#[to_exwlshell_message]
#[derive(Debug, Clone)]
pub enum Message {
    /// A lock surface, on one screen.
    Surface(window::Id),
    /// Its size, in logical pixels.
    Resized(window::Id, Size),
    /// The pointer or the keyboard is on that surface's screen.
    Here(window::Id),
    /// A key, a click or the pointer on that surface: it may bring the form.
    Input(window::Id, Input),
    Typed(Secret),
    Submit,
    Checked(Result<(), AuthError>),
    /// The eye: shows or hides the password.
    ShowPassword,
    Machine(Machine),
    Clock(Clock),
    /// Every second while the form shows: it may leave.
    /// with the time since boot, suspension included (`CLOCK_BOOTTIME`): a jump in it is a
    /// sleep the monotonic clock did not see.
    Tick(Instant, Duration),
    Frame(Instant),
    /// The model of Sherlock, read on a thread at start; `None` without it (the still picture).
    Mascot(Option<Mascot>),
    /// The pictures behind the screen, dark and light, read on a thread once the size of the
    /// screens is known.
    Background(Pictures),
    /// The scale of a surface's screen.
    Rescaled(f32),
    /// The theme of the appearance file.
    Theme(color::Theme),
}

/// The time shown, to the minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Clock {
    pub weekday: u8,
    pub day: u8,
    pub month: u8,
    pub hour: u8,
    pub minute: u8,
}

impl Clock {
    /// The local time now, from the C library like the bar.
    pub fn now() -> Option<Self> {
        let epoch = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        let tm = appearance_config::local(i64::try_from(epoch).ok()?).ok()?;
        let field = |v: i32| u8::try_from(v).ok();
        Some(Self {
            // Monday is 1 and Sunday 7.
            weekday: field(if tm.tm_wday == 0 { 7 } else { tm.tm_wday })?,
            day: field(tm.tm_mday)?,
            month: field(tm.tm_mon + 1)?,
            hour: field(tm.tm_hour)?,
            minute: field(tm.tm_min)?,
        })
    }
}

/// Where the password stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Typing,
    /// PAM is checking it, since then.
    Checking(Duration),
    /// It was refused, then: the pill shakes and empties.
    Refused(Duration, Refusal),
    /// Unlocked: the screen leaves, since then.
    Leaving(Duration),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Wrong,
    TooMany,
    Unavailable,
    Busy,
}

pub struct Lock {
    user: String,
    password: Secret,
    reveal: bool,
    phase: Phase,
    surfaces: Vec<window::Id>,
    sizes: HashMap<window::Id, Size>,
    primary: Option<window::Id>,
    clock: Clock,
    armed: Armed,
    /// When the screen came: the clock of the animations.
    start: Instant,
    now: Instant,
    /// The time of the world, `Instant::now` but in the tests: at rest, neither frames nor
    /// ticks come, so `now` is stale when an input brings the form (`input`).
    wall: fn() -> Instant,
    /// A key ended the entrance of what shows.
    entered: bool,
    /// The form, shown on demand.
    form: Form,
    /// Something was typed since the form came.
    typed: bool,
    /// « Veille » was confirmed: Sherlock sleeps until the next input.
    asleep: bool,
    mascot: Option<Mascot>,
    /// The time since boot at the last tick.
    last_boot: Option<Duration>,
    /// The rescue mode (`rescue`).
    rescue: bool,
    /// The dark and light pictures behind the screen.
    background: Option<(image::Handle, image::Handle)>,
    /// The size, in pixels, the background was asked for.
    asked: Option<(u32, u32)>,
    /// The largest scale of the screens.
    scale: f32,
    theme: color::ThemeFade,
    authenticator: Arc<dyn Authenticator>,
    /// A check of PAM holds its thread (lot D-25m).
    checking: Arc<AtomicBool>,
    runner: Arc<dyn Runner>,
}

impl Lock {
    pub fn new(
        user: String,
        authenticator: Arc<dyn Authenticator>,
        runner: Arc<dyn Runner>,
    ) -> Self {
        let now = Instant::now();
        Self {
            user,
            password: Secret::default(),
            reveal: false,
            phase: Phase::Typing,
            surfaces: Vec::new(),
            sizes: HashMap::new(),
            primary: None,
            clock: Clock::now().unwrap_or_default(),
            armed: Armed::default(),
            start: now,
            now,
            wall: Instant::now,
            entered: false,
            form: Form::default(),
            typed: false,
            asleep: false,
            mascot: None,
            last_boot: None,
            rescue: false,
            background: None,
            asked: None,
            scale: 1.0,
            theme: color::ThemeFade::default(),
            authenticator,
            checking: Arc::default(),
            runner,
        }
    }

    /// The rescue mode (`--rescue`, lot D-25c): what the compositor starts once the lock screen
    /// failed three times. In software, without Sherlock, background or animation: the user's
    /// name, the password field, checked by PAM exactly as in the normal mode, and a line that
    /// says it is the rescue lock screen. The field always shows.
    pub fn rescue(mut self) -> Self {
        self.rescue = true;
        self.entered = true;
        self.form.show(Duration::ZERO);
        self
    }

    /// The form shows.
    pub fn form_shown(&self) -> bool {
        self.form.shown()
    }

    /// What Sherlock shows now.
    pub fn mood(&self) -> sherlock_3d::Mood {
        form::mood(self.field(), self.elapsed())
    }

    fn field(&self) -> form::Field {
        form::Field {
            phase: self.phase,
            empty: self.password.is_empty(),
            peek: self.reveal,
            typed: self.typed,
            asleep: self.asleep,
        }
    }

    /// The form left: the field empties, the eye closes.
    fn clear(&mut self) {
        self.password = Secret::default();
        (self.reveal, self.typed) = (false, false);
        self.armed.disarm();
        if matches!(self.phase, Phase::Refused(..)) {
            self.phase = Phase::Typing;
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn primary(&self) -> Option<window::Id> {
        self.primary
    }

    pub fn armed(&self) -> Option<Machine> {
        self.armed.get()
    }

    /// What the text field holds (lot D-25m): the password itself only while the eye shows it;
    /// otherwise one mask per character, which the field draws as dots, so that iced's own
    /// copies of its value (rebuilt at every frame, freed without being wiped) never hold the
    /// password.
    fn shown(&self) -> Secret {
        match self.reveal {
            true => Secret::new(self.password.as_str().to_owned()),
            false => Secret::new(masks(self.password_len())),
        }
    }

    pub fn password_len(&self) -> usize {
        self.password.as_str().chars().count()
    }

    /// The clock of the animations, set by the tests.
    pub fn set_now(&mut self, now: Instant) {
        self.now = now;
    }

    /// What gives the time of the world, set by the tests.
    pub fn set_wall(&mut self, wall: fn() -> Instant) {
        self.wall = wall;
    }

    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
    }

    fn elapsed(&self) -> Duration {
        self.now.saturating_duration_since(self.start)
    }

    fn animating(&self) -> bool {
        let t = self.elapsed();
        let entrance = !self.entered && t < Duration::from_millis(1200);
        let form = self
            .form
            .changed()
            .is_some_and(|at| t < at + Duration::from_millis(1200));
        let phase = match self.phase {
            Phase::Typing => false,
            Phase::Checking(_) => true,
            Phase::Refused(at, _) => t < at + ui::SHAKE + Duration::from_millis(400),
            Phase::Leaving(_) => true,
        };
        entrance || form || phase || self.theme.running()
    }

    /// Reads the background once the screens are known, or again for a larger screen: at the
    /// size of the largest one, in pixels.
    fn ask_background(&mut self) -> Task<Message> {
        if self.rescue {
            return Task::none();
        }
        let (w, h) = self.sizes.values().fold((0.0f32, 0.0f32), |(w, h), size| {
            (w.max(size.width), h.max(size.height))
        });
        let needed = (
            (w * self.scale).ceil() as u32,
            (h * self.scale).ceil() as u32,
        );
        let covered = self
            .asked
            .is_some_and(|(aw, ah)| aw >= needed.0 && ah >= needed.1);
        if needed.0 == 0 || needed.1 == 0 || covered {
            return Task::none();
        }
        self.asked = Some(needed);
        crate::in_background(
            "background",
            move || crate::background::load(|key| std::env::var_os(key), needed),
            |pictures| pictures.map(Message::Background),
        )
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let frames = if self.animating() {
            window::frames().map(Message::Frame)
        } else {
            Subscription::none()
        };
        let ticks = if self.form.shown() {
            Subscription::run(ticks).map(|(now, boot)| Message::Tick(now, boot))
        } else {
            Subscription::none()
        };
        let appearance = match appearance_config::path() {
            Some(path) => Subscription::run_with(path, |path| appearance(path)).map(Message::Theme),
            None => Subscription::none(),
        };
        Subscription::batch([
            frames,
            ticks,
            appearance,
            Subscription::run(clock).map(Message::Clock),
            iced::event::listen_with(on_event),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) => {
                self.surfaces.push(id);
                if self.primary.is_none() {
                    self.primary = Some(id);
                }
                return iced::widget::operation::focus(field());
            }
            Message::Resized(id, size) => {
                self.sizes.insert(id, size);
                return self.ask_background();
            }
            Message::Rescaled(scale) => {
                self.scale = self.scale.max(scale);
                return self.ask_background();
            }
            Message::Theme(theme) => self.theme.set(theme, Instant::now()),
            Message::Here(id) => {
                if self.primary != Some(id) && self.surfaces.contains(&id) {
                    self.primary = Some(id);
                    return iced::widget::operation::focus(field());
                }
            }
            Message::Input(id, input) => return self.input(id, input),
            Message::Typed(typed) => {
                if self.form.shown() && matches!(self.phase, Phase::Typing | Phase::Refused(..)) {
                    self.typed |= !typed.is_empty();
                    self.password = edited(&self.password, &self.shown(), typed.as_str());
                    self.phase = Phase::Typing;
                    self.armed.disarm();
                }
            }
            Message::ShowPassword => self.reveal = !self.reveal,
            Message::Submit => return self.submit(),
            Message::Checked(Ok(())) => {
                tracing::info!("unlocked");
                self.phase = Phase::Leaving(self.elapsed());
            }
            Message::Checked(Err(err)) => {
                tracing::warn!("authentication: {err}");
                let refusal = match err {
                    AuthError::Refused => Refusal::Wrong,
                    AuthError::TooManyTries => Refusal::TooMany,
                    AuthError::Unavailable(_) => Refusal::Unavailable,
                    AuthError::Busy => Refusal::Busy,
                };
                self.phase = Phase::Refused(self.elapsed(), refusal);
                return iced::widget::operation::focus(field());
            }
            Message::Machine(machine) => {
                let now = Instant::now();
                if self.armed.click(machine, now) {
                    tracing::info!(verb = machine.verb(), "machine command");
                    if machine == Machine::Sleep {
                        // Nothing typed waits for whoever opens the lid.
                        self.form.hide(self.elapsed());
                        self.clear();
                        self.asleep = true;
                    }
                    self.runner.run(machine);
                }
            }
            Message::Clock(clock) => {
                self.clock = clock;
                self.armed.expire(Instant::now());
            }
            Message::Tick(now, boot) => {
                self.now = now;
                // A jump of the time since boot between two ticks: the machine slept (the lid,
                // or « Veille »). The form leaves and the field empties.
                let slept = self
                    .last_boot
                    .is_some_and(|last| boot.saturating_sub(last) > form::WAKE_GAP);
                self.last_boot = Some(boot);
                if slept {
                    tracing::info!("woken up: the form leaves, the field empties");
                    if !self.rescue {
                        self.form.hide(self.elapsed());
                    }
                    self.clear();
                    return Task::none();
                }
                let busy = matches!(self.phase, Phase::Checking(_) | Phase::Leaving(_));
                // The rescue screen keeps its field: it has nothing else to show.
                if !self.rescue && self.form.tick(self.elapsed(), busy) {
                    self.clear();
                }
            }
            Message::Mascot(mascot) => self.mascot = mascot,
            Message::Background(Pictures { dark, light }) => {
                let handle =
                    |Picture {
                         width,
                         height,
                         rgba,
                     }| image::Handle::from_rgba(width, height, rgba);
                self.background = Some((handle(dark), handle(light)));
            }
            Message::Frame(now) => {
                self.now = now;
                self.theme.tick(now);
                if let Phase::Leaving(at) = self.phase
                    && self.elapsed() >= at + ui::EXIT
                {
                    // The lock ends, then the program. THE ONLY CLEAN EXIT, AND IT UNLOCKS: the
                    // toolkit (exwlshellev) sends `unlock_and_destroy` on every clean exit while
                    // the lock is held, so `iced::exit()` and `UnLock` must never be reached but
                    // after PAM accepted the password (`Checked(Ok)`). An error ends the program
                    // without a clean exit (`main` aborts on a panic): the compositor keeps the
                    // session locked. `tests/render.rs` checks it.
                    return Task::done(Message::UnLock).chain(iced::exit());
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// A key, a click or the pointer on the surface `id`.
    fn input(&mut self, id: window::Id, input: Input) -> Task<Message> {
        // At rest, the clock of the animations stopped with the last frame: an input counts
        // from the time of the world, or the form would come in the past, skip its entrance and
        // leave at the next tick, as if `FORM_TIMEOUT` had passed (Q-44).
        self.now = self.now.max((self.wall)());
        self.asleep = false;
        let key = matches!(input, Input::Key { .. });
        // The rescue screen always shows its field: the keys go there, the pointer picks it.
        if self.rescue {
            return match key {
                true => Task::none(),
                false => self.update(Message::Here(id)),
            };
        }
        let mut task = Task::none();
        if !key {
            task = self.update(Message::Here(id));
        }
        if !self.form.shown() {
            // Nothing of what is typed: only that an input came to the lock at rest (D-25g).
            tracing::info!(key, "input on the lock at rest");
        }
        match self.form.input(self.elapsed(), input) {
            Outcome::Shown { typed } => {
                // Nothing of what is typed: only that the form came (lot D-25g).
                tracing::info!(key, "form shown");
                self.entered = false;
                self.typed = false;
                // The ticks start again with the form: no jump to compare with yet.
                self.last_boot = None;
                if let Some(typed) = typed {
                    // The key that brought the form starts the password.
                    let old = std::mem::take(&mut self.password);
                    let mut text = String::with_capacity(old.as_str().len() + typed.as_str().len());
                    text.push_str(old.as_str());
                    text.push_str(typed.as_str());
                    self.password = Secret::new(text);
                    self.typed = true;
                }
                return Task::batch([task, iced::widget::operation::focus(field())]);
            }
            Outcome::Hidden => self.clear(),
            // A key once the form shows ends its entrance.
            Outcome::Nothing => self.entered |= key,
        }
        task
    }

    fn submit(&mut self) -> Task<Message> {
        if !self.form.shown() || self.password.is_empty() || !matches!(self.phase, Phase::Typing) {
            return Task::none();
        }
        self.entered = true;
        self.phase = Phase::Checking(self.elapsed());
        // The field empties now: the password goes to PAM, and nowhere else.
        let password = std::mem::take(&mut self.password);
        tracing::info!("authentication: checking");
        // On a thread of its own, with a limit: the interface never waits for PAM (D-25f).
        let receiver = crate::pam::check_in_background(
            self.authenticator.clone(),
            &self.checking,
            self.user.clone(),
            password,
            crate::pam::TIMEOUT,
        );
        Task::perform(receiver, |result| {
            Message::Checked(
                result.unwrap_or_else(|_| Err(AuthError::Unavailable("no answer".into()))),
            )
        })
    }

    /// Progress of element `index` (0 for the time, then the bubble, the name, the pill and the
    /// machine buttons), from 0 to 1, times the exit. The time enters with the screen; the
    /// others come with the form, 70 ms apart, and leave with it in reverse order. A key ends
    /// the entrance. Unclamped: the spring overshoots a little.
    fn progress(&self, index: u32) -> f32 {
        let t = self.elapsed();
        let entrance = match (index, self.form.changed()) {
            (0, _) if self.entered => 1.0,
            (0, _) => Animation::new(ui::SPRING, 0.0, 1.0, Duration::ZERO).value(t),
            (_, None) => 0.0,
            (_, Some(_)) if self.form.shown() && self.entered => 1.0,
            (_, Some(at)) if self.form.shown() => {
                Animation::new(ui::SPRING, 0.0, 1.0, at + ui::STAGGER * (index - 1)).value(t)
            }
            (_, Some(at)) => {
                Animation::new(ui::SPRING, 1.0, 0.0, at + ui::STAGGER * (4 - index.min(4))).value(t)
            }
        };
        let exit = match self.phase {
            Phase::Leaving(at) => {
                let behind = ui::STAGGER * (4 - index.min(4));
                let t = t.saturating_sub(at + behind / 4).as_secs_f64();
                1.0 - (t / ui::EXIT.as_secs_f64()).clamp(0.0, 1.0)
            }
            _ => 1.0,
        };
        (entrance * exit) as f32
    }

    /// `progress`, between 0 and 1: an opacity (the tests read it).
    pub fn presence(&self, index: u32) -> f32 {
        self.progress(index).clamp(0.0, 1.0)
    }

    pub fn view(&self, id: window::Id) -> Element<'_, Message> {
        let colors = Colors::of(color::palette());
        if self.rescue {
            return self.rescue_view(id, &colors);
        }
        let size = self
            .sizes
            .get(&id)
            .copied()
            .unwrap_or(Size::new(1920.0, 1080.0));
        let height = size.height;
        let primary = self.primary.is_none_or(|p| p == id);
        let time = rise(
            column![
                text(
                    ui::date(self.clock.weekday, self.clock.day, self.clock.month)
                        .unwrap_or_default()
                )
                .size(ui::DATE_SIZE)
                .color(fade(colors.text, self.presence(0))),
                text(strings::clock(self.clock.hour, self.clock.minute))
                    .size(ui::clock_size(height))
                    .color(fade(colors.text, self.presence(0))),
            ]
            .align_x(Alignment::Center),
            self.presence(0),
        );
        let mut page = column![
            Space::new().height(ui::top(height)),
            time,
            Space::new().height(Length::Fill)
        ]
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .height(Length::Fill);
        let form = self.form.shown() || self.presence(1) > 0.01 || self.presence(4) > 0.01;
        if primary && form {
            page = page
                .push(rise(self.bubble(height, &colors), self.presence(1)))
                .push(Space::new().height(ui::BUBBLE_GAP))
                .push(rise(
                    text(self.user.as_str())
                        .size(ui::NAME_SIZE)
                        .font(Font {
                            weight: iced::font::Weight::Semibold,
                            ..typography::UI
                        })
                        .color(fade(colors.text, self.presence(2))),
                    self.presence(2),
                ))
                .push(Space::new().height(ui::NAME_GAP))
                .push(rise(self.pill(&colors), self.presence(3)))
                .push(Space::new().height(ui::HINT_GAP))
                .push(self.hint(&colors))
                .push(Space::new().height(ui::machine_gap(height)))
                .push(rise(self.machines(&colors), self.presence(4)))
                .push(Space::new().height(ui::bottom(height)));
        }
        let fill = colors.background;
        let plain = container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(fill.into())),
                ..container::Style::default()
            });
        let mut layers = stack![plain];
        // The picture of the dark theme, and that of the light one over it, as opaque as the
        // light palette is mixed in: the fade of the theme fades them too.
        if let Some((dark, light)) = &self.background {
            let light_share = f32::from(color::mix_step()) / f32::from(color::MIX_STEPS);
            let cover = |handle: &image::Handle, opacity: f32| {
                image(handle.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(ContentFit::Cover)
                    .opacity(opacity)
            };
            if light_share < 1.0 {
                layers = layers.push(cover(dark, 1.0));
            }
            if light_share > 0.0 {
                layers = layers.push(cover(light, light_share));
            }
        }
        layers
            .push(page)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// The rescue screen: its line, and on the screen of the pointer the name and the field, in
    /// the middle of the plain background.
    fn rescue_view(&self, id: window::Id, colors: &Colors) -> Element<'_, Message> {
        let mut page = column![text(ui::RESCUE).size(ui::NAME_SIZE).color(colors.secondary)]
            .align_x(Alignment::Center)
            .spacing(ui::NAME_GAP);
        if self.primary.is_none_or(|p| p == id) {
            page = page
                .push(
                    text(self.user.as_str())
                        .size(ui::NAME_SIZE)
                        .font(Font {
                            weight: iced::font::Weight::Semibold,
                            ..typography::UI
                        })
                        .color(colors.text),
                )
                .push(self.pill(colors))
                .push(self.hint(colors));
        }
        let fill = colors.background;
        container(page)
            .center(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(fill.into())),
                ..container::Style::default()
            })
            .into()
    }

    /// Sherlock in his bubble: in 3D on the GPU, the still picture in software or without the
    /// model. The bubble grows from `BUBBLE_FROM` of its size on a spring, in a box of
    /// constant size.
    fn bubble(&self, height: f32, colors: &Colors) -> Element<'_, Message> {
        let diameter = ui::bubble(height);
        let grow = ui::BUBBLE_FROM + (1.0 - ui::BUBBLE_FROM) * self.progress(1).max(0.0);
        let side = (diameter * grow).max(1.0);
        let fill = fade(colors.bubble, self.presence(1));
        let edge = fade(colors.bubble_border, self.presence(1));
        let field = self.field();
        // The field, under the bubble, the name and their gaps.
        let below = Point::new(
            diameter / 2.0,
            diameter + ui::BUBBLE_GAP + ui::NAME_SIZE + ui::NAME_GAP + 2.0 * ui::RISE,
        );
        let gaze = form::gaze(field, self.elapsed()).then_some(below);
        // Without the model, the still picture of the boards, its top in the bubble. With it,
        // the 3D widget, which falls back to the still picture itself in software.
        let mascot: Element<'_, Message> = match &self.mascot {
            Some(Mascot(model)) => Sherlock3d::new(Some(model.clone()))
                .mood(self.mood())
                .look_at(gaze)
                .background(fill)
                .paused(!self.form.shown())
                .width(side)
                .height(side)
                .into(),
            None => container(
                image(image::Handle::from_bytes(SHERLOCK))
                    .height(side * 1.15)
                    .opacity(self.presence(1)),
            )
            .padding(Padding::default().top(side * 0.1))
            .width(side)
            .align_x(Alignment::Center)
            .into(),
        };
        // The round bubble of the theme (night blue glass, or pale blue-grey: lot D-25d),
        // clipped: the still picture of software fills it like the 3D. It follows the fade of
        // the theme with the palette.
        let bubble = container(mascot)
            .width(side)
            .height(side)
            .clip(true)
            .style(move |_| container::Style {
                background: Some(Background::Color(fill)),
                border: Border::default().rounded(side / 2.0).width(1.0).color(edge),
                ..container::Style::default()
            });
        container(bubble).center(diameter).into()
    }

    /// The pill: the field, the eye and the arrow; a circle with the spinner while PAM checks.
    fn pill(&self, colors: &Colors) -> Element<'_, Message> {
        let t = self.elapsed();
        let presence = self.presence(3);
        let (fill, edge) = (
            fade(colors.field, presence),
            fade(colors.field_border, presence),
        );
        let style = move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(fill)),
            border: Border::default()
                .rounded(ui::FIELD_RADIUS)
                .width(1.0)
                .color(edge),
            ..container::Style::default()
        };
        let shrink = match self.phase {
            Phase::Checking(at) => Animation::new(ui::SPRING, 0.0, 1.0, at).value(t) as f32,
            _ => 0.0,
        };
        let width = ui::FIELD_WIDTH + (ui::FIELD_HEIGHT - ui::FIELD_WIDTH) * shrink.clamp(0.0, 1.0);
        let content: Element<'_, Message> = if let Phase::Checking(at) = self.phase {
            spinner(t.saturating_sub(at), colors.text)
        } else {
            let text_color = fade(colors.text, presence);
            let placeholder = fade(colors.placeholder, presence);
            // Read-only while the form leaves: no key goes into a field on its way out.
            let input = text_input(ui::PASSWORD, self.shown().as_str())
                .id(field())
                .secure(!self.reveal)
                .on_input_maybe(
                    self.form
                        .shown()
                        .then_some(|typed| Message::Typed(Secret::new(typed))),
                )
                .on_submit(Message::Submit)
                .size(ui::FIELD_TEXT_SIZE)
                .padding(Padding::default().left(ui::FIELD_RADIUS - 4.0))
                .style(move |_, _| text_input::Style {
                    background: Background::Color(IcedColor::TRANSPARENT),
                    border: Border::default(),
                    icon: placeholder,
                    placeholder,
                    value: text_color,
                    selection: fade(colors_selection(), presence),
                });
            let eye = if self.reveal { Icon::EyeOff } else { Icon::Eye };
            row![
                input.width(Length::Fill),
                self.field_button(eye, Message::ShowPassword, colors),
                self.field_button(Icon::ArrowRight, Message::Submit, colors),
            ]
            .align_y(Alignment::Center)
            .spacing(2)
            .padding(Padding::default().right(4.0))
            .into()
        };
        let pill = container(content)
            .width(width)
            .height(ui::FIELD_HEIGHT)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(style);
        // The shake of a refusal: three swings that die out.
        let dx = match self.phase {
            Phase::Refused(at, _) => {
                let p = (t.saturating_sub(at)).as_secs_f32() / ui::SHAKE.as_secs_f32();
                if p < 1.0 {
                    ui::SHAKE_AMPLITUDE * (p * ui::SHAKES * std::f32::consts::TAU).sin() * (1.0 - p)
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        row![
            Space::new().width(ui::SHAKE_AMPLITUDE + dx),
            pill,
            Space::new().width(ui::SHAKE_AMPLITUDE - dx),
        ]
        .into()
    }

    fn field_button(&self, icon: Icon, message: Message, colors: &Colors) -> Element<'_, Message> {
        let tint = fade(colors.text, self.presence(3));
        let hover = colors.button_hover;
        button(
            container(component::icon(icon, ui::FIELD_ICON, color_of(tint)))
                .center(ui::FIELD_BUTTON),
        )
        .on_press(message)
        .padding(0)
        .style(move |_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then(|| Background::Color(hover.into())),
            border: Border::default().rounded(ui::FIELD_BUTTON / 2.0),
            ..button::Style::default()
        })
        .into()
    }

    /// The line under the pill: why the password was refused.
    fn hint(&self, colors: &Colors) -> Element<'_, Message> {
        let words = match self.phase {
            Phase::Refused(_, Refusal::Wrong) => ui::WRONG,
            Phase::Refused(_, Refusal::TooMany) => ui::TOO_MANY,
            Phase::Refused(_, Refusal::Unavailable) => ui::UNAVAILABLE,
            Phase::Refused(_, Refusal::Busy) => ui::BUSY,
            _ => "",
        };
        text(words)
            .size(ui::HINT_SIZE)
            .height(ui::HINT_SIZE * typography::LINE_HEIGHT)
            .color(colors.error)
            .into()
    }

    fn machines(&self, colors: &Colors) -> Element<'_, Message> {
        let presence = self.presence(4);
        let buttons = Machine::ALL.map(|machine| {
            let armed = self.armed.get() == Some(machine);
            let (icon, label) = match machine {
                Machine::Sleep => (Icon::Sleep, ui::SLEEP),
                Machine::Reboot => (Icon::Restart, ui::REBOOT),
                Machine::PowerOff => (Icon::Power, ui::POWER_OFF),
            };
            let label = if armed { ui::CONFIRM } else { label };
            let (fill, hover, edge) = (
                fade(if armed { colors.armed } else { colors.button }, presence),
                fade(colors.button_hover, presence),
                fade(colors.field_border, presence),
            );
            let round = button(
                container(component::icon(
                    icon,
                    ui::MACHINE_ICON,
                    color_of(fade(colors.text, presence)),
                ))
                .center(ui::MACHINE_BUTTON),
            )
            .on_press(Message::Machine(machine))
            .padding(0)
            .style(move |_, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered | button::Status::Pressed => hover,
                    _ => fill,
                })),
                border: Border::default()
                    .rounded(ui::MACHINE_BUTTON / 2.0)
                    .width(1.0)
                    .color(edge),
                ..button::Style::default()
            });
            column![
                round,
                text(label)
                    .size(ui::MACHINE_LABEL_SIZE)
                    .color(fade(colors.secondary, presence)),
            ]
            .spacing(6)
            .align_x(Alignment::Center)
            .into()
        });
        row(buttons).spacing(ui::MACHINE_GAP).into()
    }
}

/// An element that rises into place as `presence` goes from 0 to 1, in a box of constant
/// height so that nothing around it moves.
fn rise<'a>(element: impl Into<Element<'a, Message>>, presence: f32) -> Element<'a, Message> {
    let offset = ui::RISE * (1.0 - presence.clamp(0.0, 1.0));
    container(element)
        .padding(Padding::default().top(offset).bottom(ui::RISE - offset))
        .into()
}

/// The spinner, `elapsed` after it started: the arc turned by one of `SPIN_STEPS` steps, drawn
/// into the SVG itself (one document per step, which iced keeps), since the rotation of the
/// SVG widget draws nothing with tiny-skia.
fn spinner<'a>(elapsed: Duration, tint: Color) -> Element<'a, Message> {
    let steps = motion::SPIN_STEPS;
    let turn = elapsed.as_secs_f32() / ui::SPIN.as_secs_f32();
    let step = (turn.fract() * steps as f32) as u32 % steps;
    let angle = step as f32 * 360.0 / steps as f32;
    let center = metrics::ICON_GRID / 2.0;
    let document = Icon::Spinner.svg().replace(
        "<path ",
        &format!("<path transform=\"rotate({angle} {center} {center})\" "),
    );
    svg(svg::Handle::from_memory(document.into_bytes()))
        .width(ui::FIELD_ICON)
        .height(ui::FIELD_ICON)
        .style(move |_, _| svg::Style {
            color: Some(tint.into()),
        })
        // The renderers ignore the alpha of an SVG's color: the opacity carries it (UI-04).
        .opacity(color::icon_opacity(tint))
        .into()
}

/// A color of the palette, its opacity times `presence`.
fn fade(color: Color, presence: f32) -> IcedColor {
    let mut color: IcedColor = color.into();
    color.a *= presence;
    color
}

fn color_of(color: IcedColor) -> Color {
    Color {
        r: (color.r * 255.0).round() as u8,
        g: (color.g * 255.0).round() as u8,
        b: (color.b * 255.0).round() as u8,
        a: color.a,
    }
}

fn colors_selection() -> Color {
    color::palette().selected_row
}

fn on_event(event: Event, _status: iced::event::Status, id: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorEntered) => Some(Message::Here(id)),
        Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::ButtonPressed(_)) => {
            Some(Message::Input(id, Input::Pointer))
        }
        Event::Window(window::Event::Focused) => Some(Message::Here(id)),
        Event::Window(window::Event::Rescaled(scale)) => Some(Message::Rescaled(scale)),
        Event::Window(window::Event::Resized(size) | window::Event::Opened { size, .. }) => {
            Some(Message::Resized(id, size))
        }
        Event::Keyboard(keyboard::Event::KeyPressed { key, text, .. }) => {
            let key = match key {
                keyboard::Key::Named(keyboard::key::Named::Space) => form::Key::Space,
                keyboard::Key::Named(keyboard::key::Named::Enter) => form::Key::Enter,
                keyboard::Key::Named(keyboard::key::Named::Escape) => form::Key::Escape,
                _ => form::Key::Other,
            };
            let text = text.map(|t| Secret::new(t.to_string()));
            Some(Message::Input(id, Input::Key { key, text }))
        }
        _ => None,
    }
}

/// The themes of the appearance file, followed on a thread of its own.
fn appearance(path: &std::path::Path) -> mpsc::UnboundedReceiver<color::Theme> {
    let (sender, receiver) = mpsc::unbounded();
    let follow = appearance_config::follow_themes(path.to_path_buf(), move |theme| {
        sender.unbounded_send(theme).is_ok()
    });
    if let Err(error) = follow {
        tracing::warn!(%error, "the lock screen will not follow the appearance file");
    }
    receiver
}

/// The time since boot, suspension included (`CLOCK_BOOTTIME`); zero if it cannot be read.
fn since_boot() -> Duration {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` writes into `time`, a valid timespec owned here.
    let status = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) };
    if status != 0 {
        return Duration::ZERO;
    }
    let secs = u64::try_from(time.tv_sec).unwrap_or(0);
    let nanos = u32::try_from(time.tv_nsec).unwrap_or(0);
    Duration::new(secs, nanos)
}

/// A tick every second, with the time since boot, from a thread of its own, while the
/// subscription lives.
fn ticks() -> mpsc::UnboundedReceiver<(Instant, Duration)> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("ticks".into())
        .spawn(move || {
            while sender
                .unbounded_send((Instant::now(), since_boot()))
                .is_ok()
            {
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("ticks: {err}");
    }
    receiver
}

/// The time, read again every few seconds on a thread of its own: sent when the minute changes.
fn clock() -> mpsc::UnboundedReceiver<Clock> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("clock".into())
        .spawn(move || {
            let mut last = None;
            loop {
                let now = Clock::now();
                if now.is_some() && now != last {
                    last = now;
                    if let Some(now) = now
                        && sender.unbounded_send(now).is_err()
                    {
                        return;
                    }
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("clock: {err}");
    }
    receiver
}

#[cfg(test)]
mod field_tests {
    use super::*;

    /// The field holds masks only; its edits land on the password (lot D-25m).
    #[test]
    fn the_hidden_field_holds_masks_and_its_edits_reach_the_password() {
        let password = Secret::new("abc".into());
        let shown = Secret::new(masks(3));
        let edit = |new: String| edited(&password, &shown, &new).as_str().to_owned();
        // Typed at the end, at the start, in the middle; several characters pasted.
        assert_eq!(edit(format!("{}d", masks(3))), "abcd");
        assert_eq!(edit(format!("é{}", masks(3))), "éabc");
        let m = masks(3);
        let m: Vec<char> = m.chars().collect();
        assert_eq!(edit(format!("{}X{}{}", m[0], m[1], m[2])), "aXbc");
        assert_eq!(edit(format!("{}{}€€{}", m[0], m[1], m[2])), "ab€€c");
        // Erased at the end, at the start, in the middle, all of it; a selection replaced.
        assert_eq!(edit(masks(2)), "ab");
        assert_eq!(edit(format!("{}{}", m[1], m[2])), "bc");
        assert_eq!(edit(format!("{}{}", m[0], m[2])), "ac");
        assert_eq!(edit(String::new()), "");
        assert_eq!(edit(format!("{}z", m[0])), "az");
        // From an empty field.
        let empty = Secret::default();
        assert_eq!(edited(&empty, &empty, "pw").as_str(), "pw");
    }

    #[test]
    fn the_revealed_field_edits_the_password_itself() {
        let password = Secret::new("motdepasse".into());
        let shown = Secret::new("motdepasse".into());
        assert_eq!(edited(&password, &shown, "motdepase").as_str(), "motdepase");
        assert_eq!(
            edited(&password, &shown, "motdepasse!").as_str(),
            "motdepasse!"
        );
    }
}
