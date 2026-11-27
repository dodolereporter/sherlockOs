//! What locks the session besides Super + L (lot D-25a3, ADR 0023):
//!
//! - **Inactivity:** no input for the delay of `lock.conf` (`desktop_config::lock`, 5 minutes by
//!   default, 0 for never), read again when it changes.
//! - **logind** (system bus): the `Lock` signal of our session (`loginctl lock-session`, which
//!   the bar and the launcher send), and the sleep: a `delay` inhibitor is held, and on
//!   `PrepareForSleep(true)` the session locks, then the inhibitor is released once every screen
//!   shows the lock (or after `SLEEP_GRACE`), so that the screen is locked at wake-up. logind's
//!   `Unlock` signal is ignored: only the lock screen ends the lock.
//! - **The lid**, closed (a libinput switch): when logind does not put the machine to sleep for
//!   it (docked, or `HandleLidSwitch=ignore`).
//!
//! The bus is read on a thread of its own; its events reach the loop through a channel. The
//! tests give it a private bus with a fake logind.

use std::{
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use desktop_config::lock::Config;
use smithay::reexports::calloop::{
    LoopHandle, channel,
    timer::{TimeoutAction, Timer},
};

use crate::State;

/// How often the inactivity is checked, and the setting read again.
const IDLE_CHECK: Duration = Duration::from_secs(1);
/// Longest wait for a locked frame before letting the machine sleep.
pub const SLEEP_GRACE: Duration = Duration::from_millis(1500);

const LOGIND: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";

/// What logind says.
#[derive(Debug)]
pub enum Logind {
    /// Lock our session.
    Lock,
    /// The machine goes to sleep (`true`) or wakes up.
    Sleep(bool),
    /// A new `delay` inhibitor of the sleep.
    Inhibitor(OwnedFd),
}

/// The triggers held by the state.
#[derive(Default)]
pub struct Triggers {
    config: Config,
    file: Option<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
    /// A delay set by the tests, over the file's.
    forced: Option<Option<Duration>>,
    /// The last input.
    last_input: Option<Instant>,
    inhibitor: Option<OwnedFd>,
    /// The machine is about to sleep, since then: the inhibitor goes once the lock shows.
    sleeping: Option<Instant>,
    /// The veil before the inactivity's lock or screens off, since then (lot D-38).
    dimmed: Option<Instant>,
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

impl Triggers {
    fn reload(&mut self) {
        let stamp = self.file.as_deref().and_then(stamp);
        if stamp == self.stamp {
            return;
        }
        self.stamp = stamp;
        self.config = match &self.file {
            Some(file) => match Config::read_lenient(file) {
                Ok((config, problems)) => {
                    for problem in problems {
                        tracing::warn!("lock: {}: {problem}, line ignored", file.display());
                    }
                    config
                }
                Err(err) => {
                    tracing::warn!("lock: {}: {err}", file.display());
                    Config::default()
                }
            },
            None => Config::default(),
        };
        tracing::info!(idle = ?self.config.idle(), "lock setting");
    }

    fn idle_delay(&self) -> Option<Duration> {
        self.forced.unwrap_or_else(|| self.config.idle())
    }
}

impl State {
    /// Reads the lock setting of `file`, then checks the inactivity and the file every
    /// `IDLE_CHECK`.
    pub fn watch_idle(
        &mut self,
        handle: &LoopHandle<'static, State>,
        file: Option<PathBuf>,
    ) -> Result<(), crate::Error> {
        self.triggers.file = file;
        self.triggers.reload();
        self.triggers.last_input = Some(Instant::now());
        handle
            .insert_source(Timer::from_duration(IDLE_CHECK), |_, _, state| {
                state.triggers.reload();
                state.check_idle();
                TimeoutAction::ToDuration(IDLE_CHECK)
            })
            .map_err(|err| err.error)?;
        Ok(())
    }

    /// The delay of inactivity before the lock, over the file's (the tests); `None`: never.
    pub fn set_idle_lock(&mut self, delay: Option<Duration>) {
        self.triggers.forced = Some(delay);
    }

    /// An input happened: the inactivity starts again.
    pub(crate) fn note_input(&mut self) {
        self.triggers.last_input = Some(Instant::now());
        // The veil lifts at once.
        if self.triggers.dimmed.take().is_some() {
            tracing::debug!("input: the veil lifts");
            self.queue_redraw();
        }
    }

    /// What is left before the inactivity locks the session or turns the screens off,
    /// whichever comes first; `None`: neither is coming.
    fn idle_left(&self) -> Option<Duration> {
        let last = self.triggers.last_input?;
        let lock = self.triggers.idle_delay().filter(|_| !self.is_locked());
        let off = self
            .screen_off_delay()
            .filter(|_| self.power_phase() == crate::power::Phase::On);
        let delay = match (lock, off) {
            (Some(a), Some(b)) => a.min(b),
            (a, b) => a.or(b)?,
        };
        Some(delay.saturating_sub(last.elapsed()))
    }

    /// The opacity of the veil over the screens, from 0 to `DIM`, and whether it still darkens.
    pub fn dim(&self) -> (f32, bool) {
        use sherlock_ui::screen_off as ui;
        let Some(since) = self.triggers.dimmed else {
            return (0.0, false);
        };
        let t = (since.elapsed().as_secs_f32() / ui::DIM_FADE.as_secs_f32()).min(1.0);
        // Ease in and out.
        let eased = t * t * (3.0 - 2.0 * t);
        (ui::DIM * eased, t < 1.0)
    }

    fn check_idle(&mut self) {
        // A visible video, a cookie on the bus, logind: the inactivity waits, and starts again
        // when the last one goes (lot D-43).
        if self.idle_inhibited() {
            self.note_input();
            return;
        }
        let Some(last) = self.triggers.last_input else {
            return;
        };
        // A few seconds before, the veil darkens the screens.
        if self.triggers.dimmed.is_none()
            && self
                .idle_left()
                .is_some_and(|left| left <= sherlock_ui::screen_off::DIM_LEAD)
        {
            tracing::debug!("no input: the veil darkens the screens");
            self.triggers.dimmed = Some(Instant::now());
            self.queue_redraw();
        }
        if let Some(delay) = self.triggers.idle_delay()
            && !self.is_locked()
            && last.elapsed() >= delay
        {
            tracing::info!(?delay, "no input: the session locks");
            self.triggers.dimmed = None;
            self.lock_session();
        }
        if let Some(delay) = self.screen_off_delay()
            && self.power_phase() == crate::power::Phase::On
            && last.elapsed() >= delay
        {
            tracing::info!(?delay, "no input: the screens go off");
            self.triggers.dimmed = None;
            self.screens_off(crate::power::Then::Stay);
        }
    }

    /// Listens to logind on the system bus (`address`, or the machine's), on a thread of its
    /// own.
    pub fn watch_logind(
        &mut self,
        handle: &LoopHandle<'static, State>,
        address: Option<String>,
    ) -> Result<(), crate::Error> {
        let (sender, receiver) = channel::channel();
        handle
            .insert_source(receiver, |event, _, state| {
                if let channel::Event::Msg(event) = event {
                    state.logind_event(event);
                }
            })
            .map_err(|err| err.error)?;
        let spawned = std::thread::Builder::new()
            .name("logind".into())
            .spawn(move || {
                if let Err(err) = listen(address, &sender) {
                    tracing::warn!("logind: {err}: no lock from logind, nor before the sleep");
                }
            });
        if let Err(err) = spawned {
            tracing::warn!("logind: no thread: {err}");
        }
        Ok(())
    }

    /// The sleep inhibitor is held.
    pub fn holds_sleep_inhibitor(&self) -> bool {
        self.triggers.inhibitor.is_some()
    }

    pub(crate) fn logind_event(&mut self, event: Logind) {
        match event {
            Logind::Lock => {
                tracing::info!("logind: lock");
                self.lock_session();
            }
            Logind::Sleep(true) => {
                tracing::info!("logind: the machine goes to sleep: the session locks");
                self.lock_session();
                self.screens_off(crate::power::Then::Sleep);
                self.triggers.sleeping = Some(Instant::now());
                let grace = self.loop_handle.insert_source(
                    Timer::from_duration(SLEEP_GRACE),
                    |_, _, state| {
                        state.release_sleep();
                        TimeoutAction::Drop
                    },
                );
                if let Err(err) = grace {
                    tracing::warn!("logind: {}", err.error);
                }
                self.release_sleep();
            }
            Logind::Sleep(false) => {
                self.triggers.sleeping = None;
                self.screens_on();
            }
            Logind::Inhibitor(fd) => self.triggers.inhibitor = Some(fd),
        }
    }

    /// Releases the sleep inhibitor once the session is locked and the screens are dark (the
    /// lock first, then the old television, lot D-38), or after `SLEEP_GRACE` in all.
    /// Called on the events above and after each loop iteration.
    pub(crate) fn release_sleep(&mut self) {
        let Some(since) = self.triggers.sleeping else {
            return;
        };
        let ready = self.is_locked() && self.screens_dark();
        if ready || since.elapsed() >= SLEEP_GRACE {
            tracing::info!(ready, "logind: the machine may sleep");
            self.triggers.sleeping = None;
            self.triggers.inhibitor = None;
        }
    }
}

/// Reads the signals of logind until the bus closes. A new inhibitor is taken at start and
/// after each wake-up.
fn listen(address: Option<String>, sender: &channel::Sender<Logind>) -> Result<(), zbus::Error> {
    use zbus::blocking::{Connection, MessageIterator, Proxy, connection::Builder};
    let connection = match address {
        Some(address) => Builder::address(address.as_str())?.build()?,
        None => Connection::system()?,
    };
    let manager = Proxy::new(&connection, LOGIND, MANAGER_PATH, MANAGER)?;
    let session: Option<zbus::zvariant::OwnedObjectPath> = manager
        .call("GetSessionByPID", &(std::process::id()))
        .inspect_err(|err| tracing::warn!("logind: no session for this process: {err}"))
        .ok();
    let inhibit = || -> Result<OwnedFd, zbus::Error> {
        let what = (
            "sleep",
            "SherlockOS",
            "Lock the session before sleep",
            "delay",
        );
        let fd: zbus::zvariant::OwnedFd = manager.call("Inhibit", &what)?;
        Ok(fd.into())
    };
    let send = |event| sender.send(event).is_ok();
    // Listening first: a `PrepareForSleep` sent right after the inhibitor is taken is not lost.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(LOGIND)?
        .build();
    let messages = MessageIterator::for_match_rule(rule, &connection, None)?;
    match inhibit() {
        Ok(fd) => {
            send(Logind::Inhibitor(fd));
        }
        Err(err) => tracing::warn!("logind: no sleep inhibitor: {err}"),
    }
    for message in messages {
        let message = message?;
        let header = message.header();
        let (Some(member), Some(path)) = (header.member(), header.path()) else {
            continue;
        };
        let event = match member.as_str() {
            "PrepareForSleep" if path.as_str() == MANAGER_PATH => {
                let going: bool = message.body().deserialize()?;
                if !going && let Ok(fd) = inhibit() {
                    send(Logind::Inhibitor(fd));
                }
                Logind::Sleep(going)
            }
            "Lock"
                if session
                    .as_ref()
                    .is_some_and(|s| s.as_str() == path.as_str()) =>
            {
                Logind::Lock
            }
            _ => continue,
        };
        if !send(event) {
            break;
        }
    }
    Ok(())
}
