//! The login screen: one overlay surface per screen. Every screen shows the background, the date
//! and the time; the screen of the pointer (the first one at start) also shows the form, from
//! the top: Sherlock in 3D in his bubble, the only avatar, the name of the user and, when there
//! are several, a discreet row of pills to choose another one (lot D-24d), the password pill,
//! the choice of the session and the machine buttons. The look and the motions are those of the
//! lock screen (`lock`), whose pieces it reuses; its own sizes and texts are in
//! `sherlock_ui::greeter`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::futures::channel::oneshot;
use iced::widget::{Space, button, column, container, image, pin, row, stack, text, text_input};
use iced::{
    Alignment, Background, Border, Color as IcedColor, Element, Event, Length, Padding, Point,
    Size, Subscription, Task, keyboard, mouse, window,
};
use iced_exwlshell::to_exwlshell_message;
use lock::app::{Clock, Mascot, Phase, Refusal};
use lock::background::{Picture, Pictures};
use lock::machine::{Armed, Machine, Runner};
use lock::view::{self, Avatar, color_of, fade, rise, spinner};
use sherlock_ui::color;
use sherlock_ui::component;
use sherlock_ui::ellipsis::ellipsis;
use sherlock_ui::focus::Focusable;
use sherlock_ui::greeter as ui;
use sherlock_ui::icon::Icon;
use sherlock_ui::lock::screen::{self as screen, Colors};
use sherlock_ui::motion::Animation;
use sherlock_ui::{strings, typography};
use system_services::secret::Secret;

use crate::login::{Outcome, Report};
use crate::memory::Memory;
use crate::sessions::{SHERLOCKOS, Session};
use crate::users::{Account, User};

/// The elements of the screen, from the time (0) to the machine buttons.
const LAST: u32 = 5;

/// What logs a user in: greetd (`crate::Greetd`), or a fake in the tests.
pub trait Login: Send + Sync + 'static {
    fn log_in(&self, user: &str, password: Secret, session: &Session) -> Result<Report, String>;
}

/// Where the screen writes what it remembers: the file, or nothing in the tests.
pub trait Remember: Send + Sync + 'static {
    fn save(&self, memory: &Memory);
}

/// The ids of the two fields.
pub fn password_field() -> iced::widget::Id {
    iced::widget::Id::new("password")
}
pub fn name_field() -> iced::widget::Id {
    iced::widget::Id::new("name")
}

#[to_exwlshell_message]
#[derive(Debug, Clone)]
pub enum Message {
    /// An overlay surface, on one screen.
    Surface(window::Id),
    /// A click on that surface: the field keeps the keyboard.
    Pressed(window::Id),
    Resized(window::Id, Size),
    Rescaled(f32),
    /// The pointer or the keyboard is on that surface's screen.
    Here(window::Id),
    /// A key: it ends the entrance; the arrows choose the user while nothing is typed.
    Key(Option<Step>),
    /// A user of the pills; the one past the last is « Autre… ».
    Select(usize),
    OtherName(String),
    Typed(Secret),
    Submit,
    LoggedIn(Result<Report, String>),
    ShowPassword,
    /// A session of the list.
    Session(usize),
    /// Shows or hides the other sessions.
    Others,
    Machine(Machine),
    Clock(Clock),
    Frame(Instant),
    Mascot(Option<Mascot>),
    Background(Pictures),
    Theme(color::Theme),
    /// What AccountsService says, once it answered.
    Accounts(Vec<Account>),
}

/// A key the screen treats apart: the arrows choose the user, Tab moves the focus between the
/// field and the buttons (backward with Shift), Escape empties the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Left,
    Right,
    Tab { back: bool },
    Escape,
}

pub struct Greeter {
    users: Vec<User>,
    /// The index of the selected user; `users.len()` is « Autre… ».
    selected: usize,
    /// When the selection last changed.
    switched: Option<Duration>,
    other_name: String,
    sessions: Vec<Session>,
    session: Option<usize>,
    /// When the session last changed, and the place of the thumb before.
    session_switched: Option<(Duration, f32)>,
    show_others: bool,
    memory: Memory,
    password: Secret,
    reveal: bool,
    typed: bool,
    phase: Phase,
    /// What greetd or PAM said, shown under the pill.
    note: Option<String>,
    surfaces: Vec<window::Id>,
    sizes: HashMap<window::Id, Size>,
    primary: Option<window::Id>,
    clock: Clock,
    armed: Armed,
    start: Instant,
    now: Instant,
    /// A key ended the entrance.
    entered: bool,
    mascot: Option<Mascot>,
    /// The dark and light pictures behind the screen, and when they were read.
    background: Option<(image::Handle, image::Handle)>,
    /// `None` when read before the first frame: shown at once.
    background_at: Option<Duration>,
    /// When the last key came in a field: Sherlock looks at it.
    last_key: Option<Duration>,
    /// The first frame was drawn: the entrance starts from it.
    drawn: bool,
    /// How many times the field was given the keyboard (the tests).
    focus_requests: u32,
    asked: Option<(u32, u32)>,
    scale: f32,
    theme: color::ThemeFade,
    login: Arc<dyn Login>,
    remember: Arc<dyn Remember>,
    runner: Arc<dyn Runner>,
}

impl Greeter {
    /// The screen for `users` and `sessions`, the last user of `memory` selected.
    pub fn new(
        users: Vec<User>,
        sessions: Vec<Session>,
        memory: Memory,
        login: Arc<dyn Login>,
        remember: Arc<dyn Remember>,
        runner: Arc<dyn Runner>,
    ) -> Self {
        let now = Instant::now();
        let selected = memory
            .last_user
            .as_deref()
            .and_then(|last| users.iter().position(|u| u.name == last))
            .unwrap_or(0);
        let mut greeter = Self {
            users,
            selected,
            switched: None,
            other_name: String::new(),
            sessions,
            session: None,
            session_switched: None,
            show_others: false,
            memory,
            password: Secret::default(),
            reveal: false,
            typed: false,
            phase: Phase::Typing,
            note: None,
            surfaces: Vec::new(),
            sizes: HashMap::new(),
            primary: None,
            clock: Clock::now().unwrap_or_default(),
            armed: Armed::default(),
            start: now,
            now,
            entered: false,
            mascot: None,
            background: None,
            background_at: None,
            last_key: None,
            drawn: false,
            focus_requests: 0,
            asked: None,
            scale: 1.0,
            theme: color::ThemeFade::default(),
            login,
            remember,
            runner,
        };
        greeter.session = greeter.remembered_session();
        greeter
    }

    /// The session the selected user chose last time, else SherlockOS, else the first.
    fn remembered_session(&self) -> Option<usize> {
        let user = self.users.get(self.selected).map(|u| u.name.as_str());
        let remembered = user.and_then(|u| self.memory.session(u));
        let index = |id: &str| self.sessions.iter().position(|s| s.id == id);
        remembered
            .and_then(index)
            .or_else(|| index(SHERLOCKOS))
            .or((!self.sessions.is_empty()).then_some(0))
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn session(&self) -> Option<&Session> {
        self.sessions.get(self.session?)
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    pub fn password_len(&self) -> usize {
        self.password.as_str().chars().count()
    }

    pub fn set_now(&mut self, now: Instant) {
        self.now = now;
    }

    /// The clock of the animations at `elapsed` from the start of the screen (the tests).
    pub fn set_elapsed(&mut self, elapsed: Duration) {
        self.now = self.start + elapsed;
    }

    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
    }

    /// « Autre… » is selected: a name is typed.
    fn other(&self) -> bool {
        self.selected >= self.users.len()
    }

    /// The login name that will be sent.
    fn user_name(&self) -> &str {
        match self.users.get(self.selected) {
            Some(user) => &user.name,
            None => self.other_name.trim(),
        }
    }

    fn elapsed(&self) -> Duration {
        self.now.saturating_duration_since(self.start)
    }

    /// Sherlock looks down at the field now.
    pub fn looks_at_field(&self) -> bool {
        lock::form::gaze(self.field(), self.elapsed())
    }

    /// What Sherlock shows now: the moods of the lock screen.
    pub fn mood(&self) -> sherlock_3d::Mood {
        lock::form::mood(self.field(), self.elapsed())
    }

    fn field(&self) -> lock::form::Field {
        lock::form::Field {
            phase: self.phase,
            empty: self.password.is_empty(),
            peek: self.reveal,
            typed: self.typed,
            asleep: false,
            last_key: self.last_key,
        }
    }

    /// The last of the choices the arrows go through: the users and « Autre… » when there are
    /// several users; with a single one, only them.
    fn last_choice(&self) -> usize {
        if self.users.len() > 1 {
            self.users.len()
        } else {
            0
        }
    }

    fn animating(&self) -> bool {
        let t = self.elapsed();
        let recent = |at: Option<Duration>, length: Duration| {
            at.is_some_and(|at| t < at + length + Duration::from_millis(100))
        };
        let phase = match self.phase {
            Phase::Typing => false,
            Phase::Checking(_) | Phase::Leaving(_) => true,
            Phase::Refused(at, _) => t < at + screen::SHAKE + Duration::from_millis(400),
        };
        let picture = self.background_at;
        (!self.entered && t < Duration::from_millis(1400))
            || recent(self.switched, ui::LOOK_AT_USER)
            || recent(
                self.session_switched.map(|s| s.0),
                Duration::from_millis(1100),
            )
            || recent(self.last_key, screen::TYPING_GAZE)
            || recent(picture, screen::PICTURE_FADE)
            || phase
            || self.theme.running()
    }

    fn ask_background(&mut self) -> Task<Message> {
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
        lock::in_background(
            "background",
            move || lock::background::load(|key| std::env::var_os(key), needed),
            |pictures| pictures.map(Message::Background),
        )
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let frames = if self.animating() {
            window::frames().map(Message::Frame)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            frames,
            Subscription::run(lock::app::clock).map(Message::Clock),
            iced::event::listen_with(on_event),
        ])
    }

    /// The field that takes the keys: the name for « Autre… » until one is typed, else the
    /// password.
    /// Gives the keyboard to the field: whatever took it (a click elsewhere, Escape in the
    /// field, the spinner in its place), the next key goes there.
    fn focus(&mut self) -> Task<Message> {
        self.focus_requests += 1;
        let id = if self.other() && self.other_name.trim().is_empty() {
            name_field()
        } else {
            password_field()
        };
        iced::widget::operation::focus(id)
    }

    /// How many times the field was given the keyboard (the tests).
    pub fn focus_requests(&self) -> u32 {
        self.focus_requests
    }

    /// The real names of AccountsService, and its system accounts left out. The selection
    /// stays on the same user, or on « Autre… ». While a login is under way, the list does not
    /// lose the user being logged in.
    pub fn take_accounts(&mut self, accounts: &[Account]) {
        let name = self.users.get(self.selected).map(|u| u.name.clone());
        let users = crate::users::merge(self.users.clone(), accounts);
        let place = name
            .as_ref()
            .map(|name| users.iter().position(|u| &u.name == name));
        let busy = matches!(self.phase, Phase::Checking(_) | Phase::Leaving(_));
        if busy && place == Some(None) {
            return;
        }
        self.selected = match place {
            Some(Some(index)) => index,
            // The selected user was a system account.
            Some(None) => 0,
            None => users.len(),
        };
        self.users = users;
    }

    fn select(&mut self, index: usize) -> Task<Message> {
        let index = index.min(self.users.len());
        let busy = matches!(self.phase, Phase::Checking(_) | Phase::Leaving(_));
        if busy {
            return Task::none();
        }
        if index == self.selected {
            return self.focus();
        }
        self.selected = index;
        self.switched = Some(self.elapsed());
        self.session = self.remembered_session().or(self.session);
        self.password = Secret::default();
        (self.reveal, self.typed, self.note) = (false, false, None);
        self.phase = Phase::Typing;
        self.focus()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) => {
                self.surfaces.push(id);
                if self.primary.is_none() {
                    self.primary = Some(id);
                }
                return self.focus();
            }
            Message::Resized(id, size) => {
                // The first size of the screen of the form: its widgets exist from now on, so
                // the field takes the keyboard at once.
                let first = self.sizes.insert(id, size).is_none();
                let background = self.ask_background();
                if first && self.primary == Some(id) {
                    return Task::batch([background, self.focus()]);
                }
                return background;
            }
            Message::Pressed(id) => {
                let here = self.update(Message::Here(id));
                if matches!(self.phase, Phase::Typing | Phase::Refused(..)) {
                    return Task::batch([here, self.focus()]);
                }
                return here;
            }
            Message::Rescaled(scale) => {
                self.scale = self.scale.max(scale);
                return self.ask_background();
            }
            Message::Here(id) => {
                if self.primary != Some(id) && self.surfaces.contains(&id) {
                    self.primary = Some(id);
                    return self.focus();
                }
            }
            Message::Key(step) => {
                self.entered = true;
                // The arrows go through the users while nothing is typed.
                if self.password.is_empty() && !(self.other() && !self.other_name.is_empty()) {
                    let last = self.last_choice();
                    match step {
                        Some(Step::Left) => return self.select(self.selected.saturating_sub(1)),
                        Some(Step::Right) => return self.select((self.selected + 1).min(last)),
                        _ => {}
                    }
                }
                match step {
                    Some(Step::Tab { back }) => {
                        return sherlock_ui::focus::advance(match back {
                            true => sherlock_ui::focus::Direction::Previous,
                            false => sherlock_ui::focus::Direction::Next,
                        });
                    }
                    // Escape empties the field and keeps the keyboard in it.
                    Some(Step::Escape)
                        if matches!(self.phase, Phase::Typing | Phase::Refused(..)) =>
                    {
                        self.password = Secret::default();
                        self.phase = Phase::Typing;
                        return self.focus();
                    }
                    _ => {}
                }
            }
            Message::Select(index) => return self.select(index),
            Message::OtherName(name) => {
                if matches!(self.phase, Phase::Typing | Phase::Refused(..)) {
                    self.other_name = name;
                    self.phase = Phase::Typing;
                    self.last_key = Some(self.elapsed());
                }
            }
            Message::Typed(typed) => {
                if matches!(self.phase, Phase::Typing | Phase::Refused(..)) {
                    self.typed |= !typed.is_empty();
                    self.last_key = Some(self.elapsed());
                    self.password = typed;
                    self.phase = Phase::Typing;
                    self.armed.disarm();
                }
            }
            Message::ShowPassword => {
                self.reveal = !self.reveal;
                return self.focus();
            }
            Message::Submit => return self.submit(),
            Message::LoggedIn(result) => return self.logged_in(result),
            Message::Session(index) => {
                if index < self.sessions.len() && Some(index) != self.session {
                    self.session_switched = Some((self.elapsed(), self.thumb()));
                    self.session = Some(index);
                }
                return self.focus();
            }
            Message::Others => {
                self.show_others = !self.show_others;
                return self.focus();
            }
            Message::Machine(machine) => {
                if self.armed.click(machine, Instant::now()) {
                    tracing::info!(verb = machine.verb(), "machine command");
                    self.runner.run(machine);
                }
                return self.focus();
            }
            Message::Clock(clock) => {
                self.clock = clock;
                self.armed.expire(Instant::now());
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
                self.background_at = self.drawn.then(|| self.elapsed());
            }
            Message::Theme(theme) => self.theme.set(theme, Instant::now()),
            Message::Accounts(accounts) => self.take_accounts(&accounts),
            Message::Frame(now) => {
                // The entrance starts from the first frame: it is seen whole.
                if !std::mem::replace(&mut self.drawn, true) {
                    if !self.entered {
                        self.start = now;
                    }
                    self.now = now;
                    // The field takes the keyboard with the first frame, whatever came before.
                    return self.focus();
                }
                self.now = now;
                self.theme.tick(now);
                if let Phase::Leaving(at) = self.phase
                    && self.elapsed() >= at + ui::SUCCESS_HOLD + screen::exit_length(LAST)
                {
                    // The session starts once this program has left (greetd-ipc(7)).
                    return iced::exit();
                }
            }
            _ => {}
        }
        Task::none()
    }

    fn submit(&mut self) -> Task<Message> {
        if !matches!(self.phase, Phase::Typing) || self.password.is_empty() {
            return self.focus();
        }
        let user = self.user_name().to_owned();
        if user.is_empty() {
            self.note = Some(ui::NO_USER.into());
            return self.focus();
        }
        let Some(session) = self.session().cloned() else {
            self.note = Some(ui::NO_SESSION.into());
            return Task::none();
        };
        self.entered = true;
        self.note = None;
        self.phase = Phase::Checking(self.elapsed());
        // The field empties now: the password goes to greetd, and nowhere else.
        let password = std::mem::take(&mut self.password);
        let login = self.login.clone();
        let (sender, receiver) = oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("greetd".into())
            .spawn(move || {
                let _ = sender.send(login.log_in(&user, password, &session));
            });
        if let Err(err) = spawned {
            return Task::done(Message::LoggedIn(Err(err.to_string())));
        }
        Task::perform(receiver, |result| {
            Message::LoggedIn(result.unwrap_or_else(|_| Err("no answer".into())))
        })
    }

    fn logged_in(&mut self, result: Result<Report, String>) -> Task<Message> {
        let now = self.elapsed();
        let report = match result {
            Ok(report) => report,
            Err(err) => {
                tracing::warn!("greetd: {err}");
                self.note = Some(ui::FAILED.into());
                self.phase = Phase::Refused(now, Refusal::Unavailable);
                return self.focus();
            }
        };
        // What PAM said on the way: its last message.
        self.note = report.notes.last().map(|n| n.text.clone());
        match report.outcome {
            Outcome::Started => {
                let user = self.user_name().to_owned();
                tracing::info!(user, "logged in: the session starts once the screen leaves");
                if let Some(session) = self.session() {
                    let id = session.id.clone();
                    self.memory.logged_in(&user, &id);
                    self.remember.save(&self.memory);
                }
                self.phase = Phase::Leaving(now);
                return Task::none();
            }
            Outcome::Refused => self.phase = Phase::Refused(now, Refusal::Wrong),
            Outcome::Unsupported(question) => {
                tracing::warn!(question, "PAM asked what the screen cannot answer");
                self.note = Some(ui::UNSUPPORTED.into());
                self.phase = Phase::Refused(now, Refusal::Unavailable);
            }
            Outcome::Failed(reason) => {
                tracing::warn!(reason, "greetd could not log in");
                self.note.get_or_insert_with(|| ui::FAILED.into());
                self.phase = Phase::Refused(now, Refusal::Unavailable);
            }
        }
        self.focus()
    }

    /// Progress of element `index` (0 the time, 1 the bubble, 2 the name and the pills, 3 the
    /// pill, 4 the sessions, 5 the machine buttons), from 0 to 1, times the exit: they rise in
    /// turn, then leave in reverse order once Sherlock was happy for `SUCCESS_HOLD`, the bubble
    /// shrinking. The background stays: the session starts from the same picture.
    fn progress(&self, index: u32) -> f32 {
        let t = self.elapsed();
        let entrance = if self.entered {
            1.0
        } else {
            Animation::new(screen::SPRING, 0.0, 1.0, screen::STAGGER * index).value(t)
        };
        let exit = match self.phase {
            Phase::Leaving(at) => {
                screen::exit(index, LAST, t.saturating_sub(at + ui::SUCCESS_HOLD))
            }
            _ => 1.0,
        };
        entrance as f32 * exit
    }

    fn presence(&self, index: u32) -> f32 {
        self.progress(index).clamp(0.0, 1.0)
    }

    /// The height of the pills and their gap under the name, when they show.
    fn chips_height(&self) -> f32 {
        if self.users.len() > 1 {
            ui::CHIPS_GAP + ui::CHIP_HEIGHT
        } else {
            0.0
        }
    }

    /// Where the thumb of the sessions stands: 0 on « Bureau », 1 on « Mode jeu ».
    fn thumb(&self) -> f32 {
        let target = match self.session() {
            Some(session) if session.id == SHERLOCKOS => 0.0,
            _ => -1.0,
        };
        match self.session_switched {
            Some((at, from)) => {
                Animation::new(ui::SEGMENT_SPRING, f64::from(from), f64::from(target), at)
                    .value(self.elapsed()) as f32
            }
            None => target,
        }
    }

    pub fn view(&self, id: window::Id) -> Element<'_, Message> {
        let colors = Colors::of(color::palette());
        let size = self
            .sizes
            .get(&id)
            .copied()
            .unwrap_or(Size::new(1920.0, 1080.0));
        let height = size.height;
        let time = rise(
            column![
                text(
                    screen::date(self.clock.weekday, self.clock.day, self.clock.month)
                        .unwrap_or_default()
                )
                .size(screen::DATE_SIZE)
                .color(fade(colors.text, self.presence(0))),
                text(strings::clock(self.clock.hour, self.clock.minute))
                    .size(screen::clock_size(height))
                    .color(fade(colors.text, self.presence(0))),
            ]
            .align_x(Alignment::Center),
            self.presence(0),
        );
        let mut page = column![
            Space::new().height(screen::top(height)),
            time,
            Space::new().height(Length::Fill)
        ]
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .height(Length::Fill);
        if self.primary.is_none_or(|p| p == id) {
            page = page
                .push(rise(self.bubble(size, &colors), self.presence(1)))
                .push(Space::new().height(screen::BUBBLE_GAP))
                .push(rise(self.identity(&colors), self.presence(2)))
                .push(Space::new().height(screen::NAME_GAP))
                .push(rise(self.pill(&colors), self.presence(3)))
                .push(Space::new().height(screen::HINT_GAP))
                .push(self.hint(&colors))
                .push(Space::new().height(ui::SESSION_GAP))
                .push(rise(self.sessions(&colors), self.presence(4)))
                .push(Space::new().height(screen::machine_gap(height)))
                .push(rise(self.machines(&colors), self.presence(5)))
                .push(Space::new().height(screen::bottom(height)));
        }
        // The pictures, faded in over the compositor's own; never a plain background.
        let opacity = self.background_at.map_or(1.0, |at| {
            view::picture_opacity(self.elapsed().saturating_sub(at))
        });
        stack![view::backdrop(self.background.as_ref(), opacity), page]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Sherlock in his bubble, the same avatar as on the lock screen (`lock::view::bubble`). He
    /// looks at the pills when a user was just chosen, at the field while keys come in it.
    fn bubble(&self, size: Size, colors: &Colors) -> Element<'_, Message> {
        let diameter = screen::bubble(size.height);
        let t = self.elapsed();
        let chips = Point::new(
            diameter / 2.0,
            diameter
                + screen::BUBBLE_GAP
                + screen::NAME_SIZE
                + ui::CHIPS_GAP
                + ui::CHIP_HEIGHT / 2.0
                + screen::RISE,
        );
        let gaze = match self.switched {
            Some(at) if t < at + ui::LOOK_AT_USER => Some(chips),
            _ => self
                .looks_at_field()
                .then(|| view::field_point(diameter, self.chips_height())),
        };
        let avatar = Avatar {
            mascot: self.mascot.as_ref(),
            mood: self.mood(),
            gaze,
            paused: false,
        };
        view::bubble(avatar, diameter, self.progress(1), colors)
    }

    /// The name, then the pills to choose another user when there are several.
    fn identity(&self, colors: &Colors) -> Element<'_, Message> {
        let name = self.name(colors);
        if self.users.len() < 2 {
            return name;
        }
        column![name, Space::new().height(ui::CHIPS_GAP), self.chips(colors)]
            .align_x(Alignment::Center)
            .into()
    }

    /// One small pill per user, by the name shown (« … » when too long), « Autre… » last; the
    /// selected one is of glass, the others show their glass on hover.
    fn chips(&self, colors: &Colors) -> Element<'_, Message> {
        let presence = self.presence(2);
        let labels = self
            .users
            .iter()
            .map(|user| user.shown().to_owned())
            .chain([ui::OTHER.to_owned()]);
        let chips = labels.enumerate().map(|(index, label)| {
            let selected = index == self.selected;
            let tint = fade(
                if selected {
                    colors.text
                } else {
                    colors.secondary
                },
                presence,
            );
            let (fill, edge, hover) = (
                fade(colors.field, presence),
                fade(colors.field_border, presence),
                fade(colors.button_hover, presence),
            );
            let label = ellipsis(label)
                .size(ui::CHIP_TEXT_SIZE)
                .color(color_of(tint));
            button(
                container(label)
                    .max_width(ui::CHIP_MAX_WIDTH - 2.0 * ui::CHIP_PADDING)
                    .center_y(ui::CHIP_HEIGHT),
            )
            .on_press(Message::Select(index))
            .padding(Padding::from([0.0, ui::CHIP_PADDING]))
            .style(move |_, status| button::Style {
                background: match (selected, status) {
                    (true, _) => Some(Background::Color(fill)),
                    (_, button::Status::Hovered | button::Status::Pressed) => {
                        Some(Background::Color(hover))
                    }
                    _ => None,
                },
                border: Border::default()
                    .rounded(ui::CHIP_HEIGHT / 2.0)
                    .width(if selected { 1.0 } else { 0.0 })
                    .color(edge),
                ..button::Style::default()
            })
            .into()
        });
        row(chips)
            .spacing(ui::CHIP_GAP)
            .align_y(Alignment::Center)
            .into()
    }

    /// The name of the selected user, or the field of a name for « Autre… ».
    fn name(&self, colors: &Colors) -> Element<'_, Message> {
        let presence = self.presence(2);
        if self.other() {
            let editable = matches!(self.phase, Phase::Typing | Phase::Refused(..));
            let input = text_input(ui::USER_NAME, &self.other_name)
                .id(name_field())
                .on_input_maybe(editable.then_some(Message::OtherName))
                .on_submit(Message::Submit)
                .size(screen::FIELD_TEXT_SIZE)
                .padding(Padding::from([0.0, screen::FIELD_RADIUS]))
                .width(screen::FIELD_WIDTH)
                .style(field_style(colors, presence));
            return container(input)
                .height(screen::FIELD_HEIGHT)
                .align_y(Alignment::Center)
                .style(pill_style(colors, presence))
                .into();
        }
        let shown = self
            .users
            .get(self.selected)
            .map(User::shown)
            .unwrap_or_default();
        // The name of a user just chosen fades in.
        let swap = match self.switched {
            Some(at) => Animation::new(ui::SWAP_SPRING, 0.0, 1.0, at).value(self.elapsed()) as f32,
            None => 1.0,
        };
        view::name(shown, fade(colors.text, presence * swap.clamp(0.0, 1.0)))
    }

    /// The password pill, as on the lock screen: a circle with the spinner while greetd checks.
    fn pill(&self, colors: &Colors) -> Element<'_, Message> {
        let t = self.elapsed();
        let presence = self.presence(3);
        let shrink = match self.phase {
            Phase::Checking(at) => Animation::new(screen::SPRING, 0.0, 1.0, at).value(t) as f32,
            Phase::Leaving(_) => 1.0,
            _ => 0.0,
        };
        let width = screen::FIELD_WIDTH
            + (screen::FIELD_HEIGHT - screen::FIELD_WIDTH) * shrink.clamp(0.0, 1.0);
        let content: Element<'_, Message> = match self.phase {
            Phase::Checking(at) => spinner(t.saturating_sub(at), colors.text),
            Phase::Leaving(_) => component::icon(
                Icon::Check,
                screen::FIELD_ICON,
                color_of(fade(colors.text, presence)),
            )
            .into(),
            _ => {
                let input = text_input(screen::PASSWORD, self.password.as_str())
                    .id(password_field())
                    .secure(!self.reveal)
                    .on_input(|typed| Message::Typed(Secret::new(typed)))
                    .on_submit(Message::Submit)
                    .size(screen::FIELD_TEXT_SIZE)
                    .padding(Padding::default().left(screen::FIELD_RADIUS - 4.0))
                    .style(field_style(colors, presence));
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
            }
        };
        let pill = container(content)
            .width(width)
            .height(screen::FIELD_HEIGHT)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(pill_style(colors, presence));
        let dx = match self.phase {
            Phase::Refused(at, _) => {
                let p = t.saturating_sub(at).as_secs_f32() / screen::SHAKE.as_secs_f32();
                if p < 1.0 {
                    screen::SHAKE_AMPLITUDE
                        * (p * screen::SHAKES * std::f32::consts::TAU).sin()
                        * (1.0 - p)
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        row![
            Space::new().width(screen::SHAKE_AMPLITUDE + dx),
            pill,
            Space::new().width(screen::SHAKE_AMPLITUDE - dx),
        ]
        .into()
    }

    fn field_button(&self, icon: Icon, message: Message, colors: &Colors) -> Element<'_, Message> {
        let tint = fade(colors.text, self.presence(3));
        let hover = colors.button_hover;
        // Tab reaches it; Enter or Space presses it (`sherlock_ui::focus`).
        let pressed = button(
            container(component::icon(icon, screen::FIELD_ICON, color_of(tint)))
                .center(screen::FIELD_BUTTON),
        )
        .on_press(message.clone())
        .padding(0)
        .style(move |_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then(|| Background::Color(hover.into())),
            border: Border::default().rounded(screen::FIELD_BUTTON / 2.0),
            ..button::Style::default()
        });
        Focusable::new(pressed, Some(message), screen::FIELD_BUTTON / 2.0).into()
    }

    /// The line under the pill: why the login failed, or what PAM said.
    fn hint(&self, colors: &Colors) -> Element<'_, Message> {
        let (words, tint) = match (self.phase, &self.note) {
            (Phase::Refused(_, Refusal::Wrong), _) => (screen::WRONG, colors.error),
            (Phase::Refused(..), Some(note)) => (note.as_str(), colors.error),
            (_, Some(note)) => (note.as_str(), colors.secondary),
            _ => ("", colors.secondary),
        };
        text(words)
            .size(screen::HINT_SIZE)
            .height(screen::HINT_SIZE * typography::LINE_HEIGHT)
            .color(tint)
            .into()
    }

    /// « Bureau » or « Mode jeu » (not there yet: grayed), a segmented control whose thumb
    /// slides; under it, « Autres sessions » opens the list of the others.
    fn sessions(&self, colors: &Colors) -> Element<'_, Message> {
        let presence = self.presence(4);
        let ours = self.sessions.iter().position(|s| s.id == SHERLOCKOS);
        let thumb = self.thumb();
        let segment = |label: &'static str, message: Option<Message>, index: f32| {
            let enabled = message.is_some();
            let on = (thumb - index).abs() < 0.5;
            let tint = match (enabled, on) {
                (false, _) => fade(colors.secondary, presence * ui::DISABLED_OPACITY),
                (true, true) => fade(colors.text, presence),
                (true, false) => fade(colors.secondary, presence),
            };
            let hover = fade(colors.button_hover, presence);
            button(
                container(text(label).size(ui::SEGMENT_TEXT_SIZE).color(tint))
                    .center_x(ui::SEGMENT_WIDTH)
                    .center_y(ui::SEGMENT_HEIGHT - 2.0 * ui::SEGMENT_PADDING),
            )
            .on_press_maybe(message)
            .padding(0)
            .style(move |_, status| button::Style {
                background: (enabled
                    && !on
                    && matches!(status, button::Status::Hovered | button::Status::Pressed))
                .then_some(Background::Color(hover)),
                border: Border::default().rounded(ui::SEGMENT_HEIGHT / 2.0),
                ..button::Style::default()
            })
        };
        let segments = row![
            segment(ui::DESKTOP, ours.map(Message::Session), 0.0),
            segment(ui::GAME, None, 1.0),
        ];
        let thumb_fill = fade(
            colors.button_hover,
            presence * thumb.clamp(-1.0, 0.0).mul_add(1.0, 1.0),
        );
        let inner = ui::SEGMENT_HEIGHT - 2.0 * ui::SEGMENT_PADDING;
        let thumb_box = container(Space::new())
            .width(ui::SEGMENT_WIDTH)
            .height(inner)
            .style(move |_| container::Style {
                background: Some(Background::Color(thumb_fill)),
                border: Border::default().rounded(inner / 2.0),
                ..container::Style::default()
            });
        let control = container(
            stack![
                pin(thumb_box).x(ui::SEGMENT_WIDTH * thumb.max(0.0)),
                segments
            ]
            .width(2.0 * ui::SEGMENT_WIDTH)
            .height(inner),
        )
        .padding(ui::SEGMENT_PADDING)
        .style(pill_style(colors, presence));
        let mut part = column![control]
            .spacing(ui::OTHERS_GAP)
            .align_x(Alignment::Center);
        let others: Vec<(usize, &Session)> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.id != SHERLOCKOS)
            .collect();
        if !others.is_empty() {
            let current = self.session().filter(|s| s.id != SHERLOCKOS);
            let label = match (self.show_others, current) {
                (true, _) => ui::HIDE_SESSIONS.to_owned(),
                (false, Some(session)) => ui::session_line(&session.name),
                (false, None) => ui::OTHER_SESSIONS.to_owned(),
            };
            part = part.push(link(label, Message::Others, colors, presence));
            if self.show_others {
                let list = others.into_iter().map(|(index, session)| {
                    link(
                        session.name.clone(),
                        Message::Session(index),
                        colors,
                        presence,
                    )
                });
                part = part.push(row(list).spacing(ui::OTHERS_ROW_GAP));
            }
        }
        part.into()
    }

    fn machines(&self, colors: &Colors) -> Element<'_, Message> {
        let presence = self.presence(5);
        let buttons = Machine::ALL.map(|machine| {
            let armed = self.armed.get() == Some(machine);
            let (icon, label) = match machine {
                Machine::Sleep => (Icon::Sleep, screen::SLEEP),
                Machine::Reboot => (Icon::Restart, screen::REBOOT),
                Machine::PowerOff => (Icon::Power, screen::POWER_OFF),
            };
            let label = if armed { screen::CONFIRM } else { label };
            let (fill, hover, edge) = (
                fade(if armed { colors.armed } else { colors.button }, presence),
                fade(colors.button_hover, presence),
                fade(colors.field_border, presence),
            );
            let round = button(
                container(component::icon(
                    icon,
                    screen::MACHINE_ICON,
                    color_of(fade(colors.text, presence)),
                ))
                .center(screen::MACHINE_BUTTON),
            )
            .on_press(Message::Machine(machine))
            .padding(0)
            .style(move |_, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered | button::Status::Pressed => hover,
                    _ => fill,
                })),
                border: Border::default()
                    .rounded(screen::MACHINE_BUTTON / 2.0)
                    .width(1.0)
                    .color(edge),
                ..button::Style::default()
            });
            let round = Focusable::new(
                round,
                Some(Message::Machine(machine)),
                screen::MACHINE_BUTTON / 2.0,
            );
            column![
                round,
                text(label)
                    .size(screen::MACHINE_LABEL_SIZE)
                    .color(fade(colors.secondary, presence)),
            ]
            .spacing(6)
            .align_x(Alignment::Center)
            .into()
        });
        row(buttons).spacing(screen::MACHINE_GAP).into()
    }
}

/// A discreet text button.
fn link<'a>(
    label: String,
    message: Message,
    colors: &Colors,
    presence: f32,
) -> Element<'a, Message> {
    let (rest, hover) = (
        fade(colors.secondary, presence),
        fade(colors.text, presence),
    );
    button(text(label).size(ui::OTHERS_TEXT_SIZE))
        .on_press(message)
        .padding(0)
        .style(move |_, status| button::Style {
            text_color: match status {
                button::Status::Hovered | button::Status::Pressed => hover,
                _ => rest,
            },
            ..button::Style::default()
        })
        .into()
}

fn pill_style(
    colors: &Colors,
    presence: f32,
) -> impl Fn(&iced::Theme) -> container::Style + 'static {
    let (fill, edge) = (
        fade(colors.field, presence),
        fade(colors.field_border, presence),
    );
    move |_| container::Style {
        background: Some(Background::Color(fill)),
        border: Border::default()
            .rounded(screen::FIELD_RADIUS)
            .width(1.0)
            .color(edge),
        ..container::Style::default()
    }
}

fn field_style(
    colors: &Colors,
    presence: f32,
) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style + 'static {
    let (value, placeholder) = (
        fade(colors.text, presence),
        fade(colors.placeholder, presence),
    );
    let selection = fade(color::palette().selected_row, presence);
    move |_, _| text_input::Style {
        background: Background::Color(IcedColor::TRANSPARENT),
        border: Border::default(),
        icon: placeholder,
        placeholder,
        value,
        selection,
    }
}

fn on_event(event: Event, _status: iced::event::Status, id: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorEntered) => Some(Message::Here(id)),
        Event::Mouse(mouse::Event::ButtonPressed(_)) => Some(Message::Pressed(id)),
        Event::Window(window::Event::Focused) => Some(Message::Here(id)),
        Event::Window(window::Event::Rescaled(scale)) => Some(Message::Rescaled(scale)),
        Event::Window(window::Event::Resized(size) | window::Event::Opened { size, .. }) => {
            Some(Message::Resized(id, size))
        }
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            Some(Message::Key(match key {
                keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => Some(Step::Left),
                keyboard::Key::Named(keyboard::key::Named::ArrowRight) => Some(Step::Right),
                keyboard::Key::Named(keyboard::key::Named::Tab) => Some(Step::Tab {
                    back: modifiers.shift(),
                }),
                keyboard::Key::Named(keyboard::key::Named::Escape) => Some(Step::Escape),
                _ => None,
            }))
        }
        _ => None,
    }
}
