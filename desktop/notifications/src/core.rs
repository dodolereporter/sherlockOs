//! The state of the daemon, without D-Bus nor screen: the open notifications, which of them
//! show, their timers, Do not disturb and the history. The server feeds it, the bubbles show
//! what it sends (`Event`) and report the clicks back.
//!
//! Everything is bounded: at most `MAX_VISIBLE` shown and `MAX_WAITING` waiting notifications
//! are open, and the history holds `HISTORY`. Every walk over them is therefore short.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use notifications_config::Follower;
use sherlock_ui::notifications as ui;
use zbus::blocking::Connection;

use crate::history::{self, Entry};
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
    /// The pointer is over its bubble: its timer stays held.
    held: bool,
    timer: Timer,
}

impl Open {
    fn start(&mut self, now: Instant) {
        if let Timer::Waiting(left) = self.timer
            && !self.held
        {
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
    /// Closed by the daemon itself, their signal not sent yet: the expiry thread sends it, out
    /// of the method call that closed them.
    dropped: Vec<u32>,
    /// The history changed since `Changed` was last sent: the expiry thread sends it, out of
    /// the method call that changed it.
    changed: bool,
}

impl State {
    /// Who sent the notification `id`, while it is open or in the history.
    fn owner(&self, id: u32) -> Option<&str> {
        let open = self.open.iter().map(|o| &o.note);
        let mut all = open.chain(self.history.iter());
        all.find(|n| n.id == id).map(|n| n.sender.as_str())
    }

    fn do_not_disturb(&mut self) -> bool {
        let Some((config, problem)) = self.dnd.as_mut().map(Follower::current) else {
            return false;
        };
        if let Some(problem) = problem {
            tracing::warn!(%problem, "Do not disturb setting ignored");
        }
        // An end of Do not disturb that passed lets the notifications show (D-23e).
        config.active_now()
    }
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

    /// `Notify`: the id of the notification. `replaces_id` is kept when it names a notification
    /// of the same sender, open or in the history; any other gets a new id, so that no
    /// application can take over the bubble, and the action keys, of another.
    pub fn notify(&self, request: Request) -> u32 {
        let replaces = request.replaces_id;
        // Read and bounded before taking the lock: files are read there.
        let mut note = Notification::new(0, request);
        let mut events = Vec::new();
        let mut state = self.lock();
        note.id = match state.owner(replaces) {
            Some(owner) if replaces != 0 && owner == note.sender => replaces,
            _ => {
                state.last_id = state.last_id.checked_add(1).unwrap_or(1);
                state.last_id
            }
        };
        let id = note.id;
        let note = Arc::new(note);
        let hidden = note.urgency != Urgency::Critical && state.do_not_disturb();
        state.history.retain(|old| old.id != id);
        state.history.push_front(Arc::clone(&note));
        state.history.truncate(ui::HISTORY);
        let timer = note.timeout.map_or(Timer::Never, Timer::Waiting);
        let now = Instant::now();
        match state.open.iter_mut().find(|o| o.note.id == id) {
            // Already on screen: updated in place, even in Do not disturb.
            Some(open) => {
                open.note = Arc::clone(&note);
                open.timer = timer;
                if open.shown {
                    open.start(now);
                    events.push(Event::Show(note));
                }
            }
            // Do not disturb: the history only, and closed at once.
            None if hidden => state.dropped.push(id),
            None => {
                state.open.push(Open {
                    note,
                    shown: false,
                    held: false,
                    timer,
                });
                overflow(&mut state);
            }
        }
        promote(&mut state, now, &mut events);
        state.changed = true;
        drop(state);
        self.tick.notify_all();
        self.send(events);
        id
    }

    /// Closes a notification. Returns whether it was open; the caller sends the signal.
    pub fn close(&self, id: u32) -> bool {
        self.close_if(id, |_| true)
    }

    /// `CloseNotification` from `sender`: only its own notifications.
    pub fn close_from(&self, id: u32, sender: &str) -> bool {
        self.close_if(id, |note| note.sender == sender)
    }

    fn close_if(&self, id: u32, allowed: impl Fn(&Notification) -> bool) -> bool {
        let mut events = Vec::new();
        let mut state = self.lock();
        let found = state.open.iter().position(|o| o.note.id == id);
        let Some(index) = found.filter(|&i| allowed(&state.open[i].note)) else {
            return false;
        };
        if state.open.remove(index).shown {
            events.push(Event::Close(id));
        }
        promote(&mut state, Instant::now(), &mut events);
        // Its actions are gone from the notification centre.
        state.changed = true;
        drop(state);
        self.tick.notify_all();
        self.send(events);
        true
    }
    /// A click on the bubble: its default action, if it has one, then it closes.
    pub fn click(&self, id: u32) {
        self.click_activated(id, None);
    }

    /// `click`, the action preceded by the activation `token` of the click when one came.
    pub fn click_activated(&self, id: u32, token: Option<&str>) {
        let default = self.open(id).is_some_and(|note| note.has_default());
        if default {
            self.invoke_activated(id, DEFAULT_ACTION, token);
        } else if self.close(id) {
            self.signal_closed(id, Reason::Dismissed);
        }
    }

    /// A button of the bubble, or of the notification centre: `ActionInvoked`, then it closes
    /// unless it is resident. Returns whether the notification was open with that action.
    pub fn invoke(&self, id: u32, key: &str) -> bool {
        self.invoke_activated(id, key, None)
    }

    /// `invoke`, preceded by `ActivationToken` with the activation `token` of the click when
    /// one came (version 1.2 of the specification, lot Q-52): the application brings its
    /// window forward with it.
    pub fn invoke_activated(&self, id: u32, key: &str, token: Option<&str>) -> bool {
        let Some(note) = self.open(id) else {
            return false;
        };
        if !note.actions.iter().any(|(k, _)| k == key) {
            return false;
        }
        if let Some(token) = token {
            self.signal_activation_token(id, token);
        }
        self.signal_action(id, key);
        if !note.resident && self.close(id) {
            self.signal_closed(id, Reason::Dismissed);
        }
        true
    }

    /// The pointer is over the bubble (`held`) or left it: its timer stops, then goes on with
    /// the time it had left.
    pub fn hold(&self, id: u32, held: bool) {
        let mut state = self.lock();
        let now = Instant::now();
        if let Some(open) = state.open.iter_mut().find(|o| o.note.id == id && o.shown) {
            open.held = held;
            match open.timer {
                Timer::Running(at) if held => {
                    open.timer = Timer::Waiting(at.saturating_duration_since(now));
                }
                Timer::Waiting(_) => open.start(now),
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

    /// The history as `org.sherlockos.Notifications.History` lists it, newest first, each entry
    /// telling whether its notification is still open.
    pub fn history_entries(&self) -> Vec<Entry> {
        let state = self.lock();
        let open = |id: u32| state.open.iter().any(|o| o.note.id == id);
        state
            .history
            .iter()
            .map(|note| history::entry(note, open(note.id)))
            .collect()
    }

    /// Empties the history (« Tout effacer » of the centre).
    pub fn clear_history(&self) {
        let mut state = self.lock();
        state.history.clear();
        state.changed = true;
        drop(state);
        self.tick.notify_all();
    }

    /// Closes the notifications whose time is up, and returns how long until the next one.
    /// Also sends the signals of the notifications the daemon closed itself, and `Changed`
    /// of the history.
    pub fn expire(&self, now: Instant) -> Option<Duration> {
        let (expired, dropped, changed): (Vec<u32>, Vec<u32>, bool) = {
            let mut state = self.lock();
            let due = |o: &&Open| matches!(o.timer, Timer::Running(at) if at <= now);
            let expired = state.open.iter().filter(due).map(|o| o.note.id).collect();
            let changed = std::mem::take(&mut state.changed);
            (expired, std::mem::take(&mut state.dropped), changed)
        };
        if changed {
            self.signal_changed();
        }
        for id in dropped {
            self.signal_closed(id, Reason::Undefined);
        }
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
            // A signal still to send, set after `expire` looked: no waiting.
            if state.changed || !state.dropped.is_empty() {
                continue;
            }
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

    /// `Changed` of the history interface: the notification centre reads the list again.
    fn signal_changed(&self) {
        if let Some(bus) = self.bus.get() {
            let sent = bus.emit_signal(
                None::<&str>,
                history::PATH,
                history::INTERFACE,
                "Changed",
                &(),
            );
            if let Err(error) = sent {
                tracing::warn!(%error, "cannot send Changed");
            }
        }
    }

    fn signal_activation_token(&self, id: u32, token: &str) {
        if let Some(bus) = self.bus.get() {
            let sent = bus.emit_signal(
                None::<&str>,
                PATH,
                INTERFACE,
                "ActivationToken",
                &(id, token),
            );
            if let Err(error) = sent {
                tracing::warn!(%error, "cannot send ActivationToken");
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

/// Beyond `MAX_WAITING` notifications waiting for room, the oldest one closes (critical ones
/// last); it stays in the history.
fn overflow(state: &mut State) {
    let waiting = state.open.iter().filter(|o| !o.shown).count();
    if waiting <= ui::MAX_WAITING {
        return;
    }
    let waiting = |o: &&Open| !o.shown;
    let oldest = state
        .open
        .iter()
        .position(|o| waiting(&o) && o.note.urgency != Urgency::Critical)
        .or_else(|| state.open.iter().position(|o| waiting(&o)));
    if let Some(index) = oldest {
        let id = state.open.remove(index).note.id;
        state.dropped.push(id);
    }
}

/// Shows the next waiting notifications, in the order they came, while there is room.
fn promote(state: &mut State, now: Instant, events: &mut Vec<Event>) {
    let mut room = ui::MAX_VISIBLE.saturating_sub(state.open.iter().filter(|o| o.shown).count());
    for open in state.open.iter_mut().filter(|o| !o.shown) {
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

    fn urgent() -> zbus::zvariant::OwnedValue {
        zbus::zvariant::OwnedValue::try_from(zbus::zvariant::Value::U8(2)).unwrap()
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
    fn waiting_notifications_are_bounded() {
        let (core, _events) = core();
        let critical = core.notify(Request {
            hints: [(String::from("urgency"), urgent())].into(),
            ..request("critique", 0)
        });
        for i in 0..(ui::MAX_VISIBLE + ui::MAX_WAITING + 20) {
            core.notify(request(&i.to_string(), 0));
        }
        let state = core.lock();
        assert_eq!(state.open.len(), ui::MAX_VISIBLE + ui::MAX_WAITING);
        // The oldest waiting ones closed, the critical one shows; all stay in the history.
        assert!(state.open.iter().any(|o| o.note.id == critical && o.shown));
        assert_eq!(state.dropped.len(), 21);
        assert_eq!(state.dropped[0], ui::MAX_VISIBLE as u32 + 1);
        drop(state);
        assert_eq!(core.expire(Instant::now()), None);
        assert!(core.lock().dropped.is_empty());
    }

    #[test]
    fn a_replacement_under_the_pointer_keeps_its_timer_held() {
        let (core, _events) = core();
        let id = core.notify(request("a", 1000));
        core.hold(id, true);
        core.notify(Request {
            replaces_id: id,
            ..request("b", 1000)
        });
        assert_eq!(core.expire(Instant::now() + Duration::from_secs(5)), None);
        assert!(core.open(id).is_some());
    }

    #[test]
    fn only_the_sender_replaces_or_closes() {
        let (core, _events) = core();
        let from = |sender: &str, replaces_id| Request {
            sender: sender.to_owned(),
            replaces_id,
            ..request("x", 0)
        };
        let id = core.notify(from(":1.1", 0));
        assert_eq!(core.notify(from(":1.1", id)), id);
        let other = core.notify(from(":1.2", id));
        assert_ne!(other, id);
        assert!(!core.close_from(id, ":1.2"));
        assert!(core.close_from(id, ":1.1"));
        // Closed, it stays in the history: its sender may still reuse its id.
        assert_eq!(core.notify(from(":1.1", id)), id);
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
