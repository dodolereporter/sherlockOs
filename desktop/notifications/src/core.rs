//! The state of the daemon, without D-Bus nor screen: the open notifications, which of them
//! show, their timers, Do not disturb and the history. The server feeds it, the bubbles show
//! what it sends (`Event`) and report the clicks back.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use notifications_config::Follower;
use sherlock_ui::notifications as ui;
use zbus::blocking::Connection;

use crate::model::{DEFAULT_ACTION, Notification, Reason, Request, Urgency};
use crate::server::{INTERFACE, PATH};

/// What the bubbles have to do.
#[derive(Debug, Clone)]
pub enum Event {
    /// Show a notification, or update the bubble with the same id.
    Show(Arc<Notification>),
    /// Take the bubble away.
    Close(u32),
}

/// Where the events go: the bubbles, or a test.
pub type Sink = Box<dyn Fn(Event) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Timer {
    Never,
    /// Not started yet, or held while the pointer is over the bubble.
    Waiting(Duration),
    Running(Instant),
}

struct Open {
    note: Arc<Notification>,
    shown: bool,
    /// Do not disturb was on when it came: it never shows.
    hidden: bool,
    timer: Timer,
}

impl Open {
    fn start(&mut self, now: Instant) {
        if let Timer::Waiting(left) = self.timer {
            self.timer = Timer::Running(now + left);
        }
    }
}

#[derive(Default)]
struct State {
    last_id: u32,
    /// In the order they came.
    open: Vec<Open>,
    /// The newest first.
    history: VecDeque<Arc<Notification>>,
    dnd: Option<Follower>,
}

pub struct Core {
    state: Mutex<State>,
    tick: Condvar,
    sink: Sink,
    bus: OnceLock<Connection>,
}

impl Core {
    /// A core that sends its events to `sink` and reads Do not disturb from `dnd`.
    pub fn new(sink: Sink, dnd: Option<Follower>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                dnd,
                ..State::default()
            }),
            tick: Condvar::new(),
            sink,
            bus: OnceLock::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The connection the signals of the clicks and of the timers go out on.
    pub fn attach(self: &Arc<Self>, bus: Connection) -> std::io::Result<()> {
        let _ = self.bus.set(bus);
        let core = Arc::clone(self);
        std::thread::Builder::new()
            .name(String::from("expiry"))
            .spawn(move || core.expire_forever())
            .map(|_| ())
    }

    /// `Notify`: the id of the notification, `replaces_id` when it names one this daemon gave.
    pub fn notify(&self, request: Request) -> u32 {
        let mut events = Vec::new();
        let mut state = self.lock();
        let replaces = request.replaces_id;
        let id = if replaces != 0 && replaces <= state.last_id {
            replaces
        } else {
            state.last_id = state.last_id.checked_add(1).unwrap_or(1);
            state.last_id
        };
        let note = Arc::new(Notification::new(id, request));
        let dnd = match state.dnd.as_mut().map(Follower::current) {
            Some((config, problem)) => {
                if let Some(problem) = problem {
                    tracing::warn!(%problem, "Do not disturb setting ignored");
                }
                config.do_not_disturb
            }
            None => false,
        };
        state.history.retain(|old| old.id != id);
        state.history.push_front(Arc::clone(&note));
        state.history.truncate(ui::HISTORY);
        let timer = note.timeout.map_or(Timer::Never, Timer::Waiting);
        let now = Instant::now();
        match state.open.iter_mut().find(|o| o.note.id == id) {
            Some(open) => {
                open.note = Arc::clone(&note);
                open.timer = timer;
                if open.shown || open.hidden {
                    open.start(now);
                }
                if open.shown {
                    events.push(Event::Show(note));
                }
            }
            None => {
                let hidden = dnd && note.urgency != Urgency::Critical;
                let mut open = Open {
                    note,
                    shown: false,
                    hidden,
                    timer,
                };
                // A hidden notification still expires, out of sight.
                if hidden {
                    open.start(now);
                }
                state.open.push(open);
            }
        }
        promote(&mut state, now, &mut events);
        drop(state);
        self.tick.notify_all();
        self.send(events);
        id
    }

    /// Closes a notification. Returns whether it was open; the caller sends the signal.
    pub fn close(&self, id: u32) -> bool {
        let mut events = Vec::new();
        let mut state = self.lock();
        let Some(index) = state.open.iter().position(|o| o.note.id == id) else {
            return false;
        };
        if state.open.remove(index).shown {
            events.push(Event::Close(id));
        }
        promote(&mut state, Instant::now(), &mut events);
        drop(state);
        self.tick.notify_all();
        self.send(events);
        true
    }

    /// A click on the bubble: its default action, if it has one, then it closes.
    pub fn click(&self, id: u32) {
        let default = self.open(id).is_some_and(|note| note.has_default());
        if default {
            self.invoke(id, DEFAULT_ACTION);
        } else if self.close(id) {
            self.signal_closed(id, Reason::Dismissed);
        }
    }

    /// A button of the bubble: `ActionInvoked`, then it closes unless it is resident.
    pub fn invoke(&self, id: u32, key: &str) {
        let Some(note) = self.open(id) else {
            return;
        };
        if !note.actions.iter().any(|(k, _)| k == key) {
            return;
        }
        self.signal_action(id, key);
        if !note.resident && self.close(id) {
            self.signal_closed(id, Reason::Dismissed);
        }
    }

    /// The pointer is over the bubble (`held`) or left it: its timer stops, then goes on with
    /// the time it had left.
    pub fn hold(&self, id: u32, held: bool) {
        let mut state = self.lock();
        let now = Instant::now();
        if let Some(open) = state.open.iter_mut().find(|o| o.note.id == id && o.shown) {
            match open.timer {
                Timer::Running(at) if held => {
                    open.timer = Timer::Waiting(at.saturating_duration_since(now));
                }
                Timer::Waiting(_) if !held => open.start(now),
                _ => {}
            }
        }
        drop(state);
        self.tick.notify_all();
    }

    /// The notification with this id, while it is open.
    pub fn open(&self, id: u32) -> Option<Arc<Notification>> {
        let state = self.lock();
        let open = state.open.iter().find(|o| o.note.id == id)?;
        Some(Arc::clone(&open.note))
    }

    /// The ids of the notifications that show, in the order they came.
    pub fn shown(&self) -> Vec<u32> {
        let state = self.lock();
        state
            .open
            .iter()
            .filter(|o| o.shown)
            .map(|o| o.note.id)
            .collect()
    }

    /// The history, newest first: for the notification centre of the panels.
    pub fn history(&self) -> Vec<Arc<Notification>> {
        self.lock().history.iter().cloned().collect()
    }

    /// Empties the history (« Tout effacer » of the centre).
    pub fn clear_history(&self) {
        self.lock().history.clear();
    }

    /// Closes the notifications whose time is up, and returns how long until the next one.
    pub fn expire(&self, now: Instant) -> Option<Duration> {
        let expired: Vec<u32> = {
            let state = self.lock();
            let due = |o: &&Open| matches!(o.timer, Timer::Running(at) if at <= now);
            state.open.iter().filter(due).map(|o| o.note.id).collect()
        };
        for id in expired {
            if self.close(id) {
                self.signal_closed(id, Reason::Expired);
            }
        }
        next_wait(&self.lock(), now)
    }

    fn expire_forever(&self) {
        loop {
            self.expire(Instant::now());
            // The wait is computed under the lock it then waits on: a change of the timers in
            // between wakes the thread up, and it computes the wait again.
            let state = self.lock();
            match next_wait(&state, Instant::now()) {
                Some(wait) => drop(self.tick.wait_timeout(state, wait)),
                None => drop(self.tick.wait(state)),
            }
        }
    }

    fn signal_closed(&self, id: u32, reason: Reason) {
        if let Some(bus) = self.bus.get() {
            let sent = bus.emit_signal(
                None::<&str>,
                PATH,
                INTERFACE,
                "NotificationClosed",
                &(id, reason as u32),
            );
            if let Err(error) = sent {
                tracing::warn!(%error, "cannot send NotificationClosed");
            }
        }
    }

    fn signal_action(&self, id: u32, key: &str) {
        if let Some(bus) = self.bus.get() {
            let sent = bus.emit_signal(None::<&str>, PATH, INTERFACE, "ActionInvoked", &(id, key));
            if let Err(error) = sent {
                tracing::warn!(%error, "cannot send ActionInvoked");
            }
        }
    }

    fn send(&self, events: Vec<Event>) {
        for event in events {
            (self.sink)(event);
        }
    }
}

/// How long until the next running timer is up.
fn next_wait(state: &State, now: Instant) -> Option<Duration> {
    state
        .open
        .iter()
        .filter_map(|o| match o.timer {
            Timer::Running(at) => Some(at.saturating_duration_since(now)),
            _ => None,
        })
        .min()
}

/// Shows the next waiting notifications, in the order they came, while there is room.
fn promote(state: &mut State, now: Instant, events: &mut Vec<Event>) {
    let mut room = ui::MAX_VISIBLE.saturating_sub(state.open.iter().filter(|o| o.shown).count());
    for open in state.open.iter_mut().filter(|o| !o.shown && !o.hidden) {
        if room == 0 {
            break;
        }
        room -= 1;
        open.shown = true;
        open.start(now);
        events.push(Event::Show(Arc::clone(&open.note)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn core() -> (Arc<Core>, mpsc::Receiver<Event>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        let sink: Sink = Box::new(move |event| {
            let _ = sender.lock().map(|s| s.send(event));
        });
        (Core::new(sink, None), receiver)
    }

    fn request(summary: &str, timeout: i32) -> Request {
        Request {
            summary: summary.to_owned(),
            expire_timeout: timeout,
            ..Request::default()
        }
    }

    #[test]
    fn only_a_few_show_at_once_and_the_others_wait() {
        let (core, events) = core();
        let ids: Vec<u32> = (0..6)
            .map(|i| core.notify(request(&i.to_string(), 0)))
            .collect();
        assert_eq!(ids, [1, 2, 3, 4, 5, 6]);
        assert_eq!(core.shown(), [1, 2, 3, 4]);
        assert_eq!(events.try_iter().count(), ui::MAX_VISIBLE);
        assert!(core.close(2));
        assert_eq!(core.shown(), [1, 3, 4, 5]);
        let shown: Vec<_> = events.try_iter().map(|e| format!("{e:?}")).collect();
        assert!(shown[0].starts_with("Close(2)") && shown[1].contains("id: 5"));
        assert!(!core.close(2));
    }

    #[test]
    fn hovering_holds_the_timer() {
        let (core, _events) = core();
        let id = core.notify(request("a", 1000));
        let start = Instant::now();
        core.hold(id, true);
        assert_eq!(core.expire(start + Duration::from_secs(5)), None);
        assert!(core.open(id).is_some());
        core.hold(id, false);
        let left = core.expire(Instant::now()).unwrap();
        assert!(left > Duration::from_millis(900) && left <= Duration::from_millis(1000));
        assert_eq!(core.expire(Instant::now() + Duration::from_secs(2)), None);
        assert!(core.open(id).is_none());
    }

    #[test]
    fn the_history_keeps_the_last_hundred() {
        let (core, _events) = core();
        for i in 0..(ui::HISTORY + 5) {
            core.notify(request(&i.to_string(), 0));
        }
        let history = core.history();
        assert_eq!(history.len(), ui::HISTORY);
        assert_eq!(history[0].summary, (ui::HISTORY + 4).to_string());
        // A replacement takes the place of the old entry.
        core.notify(Request {
            replaces_id: 50,
            ..request("remplacée", 0)
        });
        let history = core.history();
        assert_eq!((history[0].id, history.len()), (50, ui::HISTORY));
        assert_eq!(history.iter().filter(|n| n.id == 50).count(), 1);
        core.clear_history();
        assert!(core.history().is_empty());
    }
}
