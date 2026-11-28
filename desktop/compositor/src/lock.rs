//! Session lock (lot D-25a, ADR 0023): `ext-session-lock-v1`, applied strictly.
//!
//! Once the session is locked, only the lock surfaces are drawn and get input. A screen without
//! one shows the plain background of the system screens; no window, anchored surface, menu or
//! thumbnail is drawn, in a frame or in a screenshot. Only the console switch, Ctrl + Alt +
//! Backspace, and the volume and brightness keys act. The lock ends only on `unlock_and_destroy`
//! from the lock that holds it, once `locked` was sent: smithay would end it on the request of
//! any lock object, so the requests of `ext_session_lock_v1` are checked here first.
//!
//! If the lock program dies, the session stays locked: the compositor starts it again; after
//! `CLIENT_ATTEMPTS` failures, the same program in its rescue mode (`--rescue`, lot D-25c: a
//! plain field checked by PAM, in software), `RESCUE_ATTEMPTS` times; then `swaylock` if it is
//! installed; and only then draws a screen of its own, which cannot unlock. Only the lock
//! program the compositor started, through a connection it opened itself (`WAYLAND_SOCKET`),
//! may lock, the session locked or not; its identity is forgotten when it dies. See "Session
//! lock" in `docs/compositor.md`.
//!
//! A lock program that hangs gets the same treatment (lot D-25f): after an input, the
//! compositor pings it (`xdg_wm_base.ping`, answered by its event loop); without an answer
//! within `PING_TIMEOUT`, it is killed and started again, and after `CLIENT_ATTEMPTS` the rescue
//! mode follows. The log is written to disk at each lock and unlock, and every `LOG_SYNC` while
//! locked, so that its end survives a forced restart.

use std::{
    ffi::OsString,
    os::{
        fd::{AsRawFd, BorrowedFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::Arc,
    time::{Duration, Instant},
};

use sherlock_ui::{color, lock as ui};
use smithay::{
    desktop::{
        WindowSurfaceType,
        utils::{send_frames_surface_tree, under_from_surface_tree},
    },
    input::pointer::CursorImageStatus,
    output::Output,
    reexports::{
        calloop::{
            RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        rustix,
        wayland_protocols::ext::session_lock::v1::server::{
            ext_session_lock_manager_v1::ExtSessionLockManagerV1,
            ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
            ext_session_lock_v1::{self, ExtSessionLockV1},
        },
        wayland_protocols::xdg::shell::server::xdg_wm_base::XdgWmBase,
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            backend::{ClientId, DisconnectReason},
            delegate_dispatch, delegate_global_dispatch,
            protocol::{wl_output::WlOutput, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Size, Transform},
    wayland::{
        compositor::{TraversalAction, send_surface_state, with_surface_tree_downward},
        fractional_scale::with_fractional_scale,
        session_lock::{
            ExtLockSurfaceUserData, LockSurface, SessionLockHandler, SessionLockManagerGlobalData,
            SessionLockManagerState, SessionLockState, SessionLocker,
        },
        shell::xdg::{ShellClient, XdgShellState},
    },
};

use crate::{
    ClientState, State,
    damage::{Key, Part},
    focus::KeyboardFocus,
    launcher::BIN_VAR,
    layer::has_content,
    mission_control::{Label, label_width},
    output::View,
    render::Item,
};

/// Name of the lock program, beside the session's programs or in the `PATH`.
pub const PROGRAM: &str = "sherlockos-lock";
/// Starts of the lock program in a row before its rescue mode.
pub const CLIENT_ATTEMPTS: u32 = 3;
/// Starts of the rescue mode in a row before `swaylock`.
pub const RESCUE_ATTEMPTS: u32 = 3;
/// The argument of the rescue mode of the lock program.
pub const RESCUE_ARGUMENT: &str = "--rescue";
/// Time a lock program has to take the lock before it counts as a failure.
pub const START_TIMEOUT: Duration = Duration::from_secs(10);
/// Pause before starting a lock program again.
pub const RELAUNCH_DELAY: Duration = Duration::from_millis(500);
/// How often the lock program is checked on while the session is locked.
const WATCH: Duration = Duration::from_millis(100);
/// Time the lock program has to answer a ping sent after an input, before it counts as hung
/// (lot D-25f).
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);
/// At most one ping in this time: input comes in bursts.
const PING_EVERY: Duration = Duration::from_secs(1);
/// How often the log is written to disk while locked.
pub const LOG_SYNC: Duration = Duration::from_secs(5);

/// The programs that show the lock screen: ours, its rescue mode, then `swaylock`.
#[derive(Debug, Clone)]
pub struct Programs {
    pub client: Vec<OsString>,
    pub rescue: Option<Vec<OsString>>,
    pub swaylock: Option<Vec<OsString>>,
}

/// What shows the lock screen now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Client,
    Rescue,
    Swaylock,
    /// The compositor's own screen: every program failed.
    Screen,
}

impl Stage {
    /// The stage of the `attempt`-th start (from 0).
    pub fn of(attempt: u32) -> Self {
        match attempt {
            n if n < CLIENT_ATTEMPTS => Self::Client,
            n if n < CLIENT_ATTEMPTS + RESCUE_ATTEMPTS => Self::Rescue,
            n if n == CLIENT_ATTEMPTS + RESCUE_ATTEMPTS => Self::Swaylock,
            _ => Self::Screen,
        }
    }
}

impl Programs {
    /// `PROGRAM` beside the session's programs (`SHERLOCKOS_BIN`) if it is there, else in the
    /// `PATH`; the same with `--rescue`; `swaylock` on the background of the system screens if
    /// it is in the `PATH`.
    pub fn of(var: impl Fn(&str) -> Option<OsString>) -> Self {
        let client = var(BIN_VAR)
            .map(|dir| Path::new(&dir).join(PROGRAM))
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from(PROGRAM))
            .into_os_string();
        let background = ui::Colors::of(color::palette()).background;
        let swaylock = var("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("swaylock"))
                .find(|file| file.is_file())
        });
        Self {
            rescue: Some(vec![client.clone(), RESCUE_ARGUMENT.into()]),
            client: vec![client],
            swaylock: swaylock.map(|program| {
                vec![
                    program.into_os_string(),
                    "--color".into(),
                    ui::hex(background).into(),
                ]
            }),
        }
    }

    fn command(&self, stage: Stage) -> Option<&Vec<OsString>> {
        match stage {
            Stage::Client => Some(&self.client),
            Stage::Rescue => self.rescue.as_ref(),
            Stage::Swaylock => self.swaylock.as_ref(),
            Stage::Screen => None,
        }
    }
}

struct Surface {
    surface: LockSurface,
    output: Output,
    /// The size last configured.
    size: Size<i32, Logical>,
}

/// The lock, held by the state.
pub(crate) struct SessionLock {
    manager: SessionLockManagerState,
    locked: bool,
    /// The lock that may end it: the one accepted last.
    owner: Option<ExtSessionLockV1>,
    /// `locked`, sent once every screen has shown a locked frame.
    confirm: Option<SessionLocker>,
    shown: Vec<Output>,
    surfaces: Vec<Surface>,
    /// The lock whose request is being read: a new surface belongs to it.
    reading: Option<ExtSessionLockV1>,
    /// The connection of the lock program alive, which the compositor opened itself: the only
    /// client that may lock. Never a pid, which a process of the session could take over once
    /// the program is dead.
    program: Option<ClientId>,
    /// Lock programs started since the session was locked.
    attempts: u32,
    /// What was started last.
    stage: Stage,
    child: Option<(Child, Instant)>,
    next_launch: Option<Instant>,
    /// Every lock program failed: the compositor draws the rescue screen.
    rescue_screen: bool,
    programs: Option<Programs>,
    /// The timer of `watch_lock_program`, while locked.
    watch: Option<RegistrationToken>,
    /// The client binding `xdg_wm_base` now.
    binding: Option<ClientId>,
    /// The shell client of the lock program: what the pings go to.
    shell: Option<ShellClient>,
    /// A ping sent and not answered yet, since then; the last ping sent.
    ping: Option<Instant>,
    pinged: Option<Instant>,
    ping_timeout: Duration,
    /// The last time the log was written to disk.
    synced: Option<Instant>,
}

impl SessionLock {
    pub(crate) fn new(dh: &DisplayHandle) -> Self {
        Self {
            manager: SessionLockManagerState::new::<State, _>(dh, |_| true),
            locked: false,
            owner: None,
            confirm: None,
            shown: Vec::new(),
            surfaces: Vec::new(),
            reading: None,
            program: None,
            attempts: 0,
            child: None,
            next_launch: None,
            rescue_screen: false,
            stage: Stage::Client,
            programs: None,
            watch: None,
            binding: None,
            shell: None,
            ping: None,
            pinged: None,
            ping_timeout: PING_TIMEOUT,
            synced: None,
        }
    }
}

impl State {
    /// The session is locked.
    pub fn is_locked(&self) -> bool {
        self.lock.locked
    }

    /// `locked` was sent to the lock that holds the session.
    pub fn lock_confirmed(&self) -> bool {
        self.lock.locked && self.lock.owner.is_some() && self.lock.confirm.is_none()
    }

    /// Every screen has shown a frame built while locked.
    pub fn lock_shown_everywhere(&self) -> bool {
        self.lock.locked
            && self
                .screens
                .iter()
                .all(|s| self.lock.shown.contains(&s.output))
    }

    /// What shows the lock screen, or was started last.
    pub fn lock_stage(&self) -> Stage {
        self.lock.stage
    }

    /// Every lock program failed: the compositor shows its rescue screen.
    pub fn lock_rescue_shown(&self) -> bool {
        self.lock.locked && self.lock.rescue_screen
    }

    /// Replaces the time a lock program has to answer a ping (the tests).
    pub fn set_lock_ping_timeout(&mut self, timeout: Duration) {
        self.lock.ping_timeout = timeout;
    }

    /// The connection of the lock program is open.
    pub fn lock_program_connected(&self) -> bool {
        self.lock.program.is_some()
    }

    /// Requests to write the log to disk so far.
    pub fn log_sync_requests(&self) -> u64 {
        self.log_sync.requests()
    }

    /// Replaces the console switch (the tests).
    pub fn set_switch_vt(&mut self, switch: Box<dyn FnMut(i32)>) {
        self.switch_vt = Some(switch);
    }

    /// Lock programs started since the session was locked.
    pub fn lock_attempts(&self) -> u32 {
        self.lock.attempts
    }

    /// A client bound `xdg_wm_base`: the lock program's is kept for the pings.
    pub(crate) fn shell_client_bound(&mut self, shell: ShellClient) {
        if self.lock.binding.is_some() && self.lock.binding == self.lock.program {
            self.lock.shell = Some(shell);
        }
    }

    /// A client answered a ping.
    pub(crate) fn shell_client_pong(&mut self, shell: &ShellClient) {
        if self.lock.shell.as_ref() == Some(shell) {
            self.lock.ping = None;
        }
    }

    /// An input while locked: the lock program holding the lock is pinged, which only an event
    /// loop alive answers (lot D-25f).
    pub(crate) fn ping_lock_program(&mut self) {
        let lock = &mut self.lock;
        let recent = lock.pinged.is_some_and(|at| at.elapsed() < PING_EVERY);
        if !lock.locked || lock.owner.is_none() || lock.ping.is_some() || recent {
            return;
        }
        let Some(shell) = lock.shell.as_ref().filter(|s| s.alive()) else {
            return;
        };
        let now = Instant::now();
        lock.pinged = Some(now);
        match shell.send_ping(SERIAL_COUNTER.next_serial()) {
            Ok(()) => lock.ping = Some(now),
            Err(err) => tracing::debug!("lock program: no ping: {err:?}"),
        }
    }

    /// Replaces the lock programs (the tests).
    pub fn set_lock_programs(&mut self, programs: Programs) {
        self.lock.programs = Some(programs);
    }

    /// Plugs in `stream` as the connection of the lock program, the only client that may lock:
    /// what `launch_lock_program` does with the socket it gives the program (the tests, whose
    /// client runs in their own process).
    pub fn insert_lock_program(&mut self, stream: UnixStream) {
        let data = Arc::new(ClientState::default());
        match self.display_handle.insert_client(stream, data) {
            Ok(client) => self.lock.program = Some(client.id()),
            Err(err) => tracing::warn!("lock program refused: {err}"),
        }
    }

    /// Locks the session now, and starts the lock program if no lock is held (Super + L).
    pub fn lock_session(&mut self) {
        if !self.lock.locked {
            self.enter_lock();
        }
        if self.lock.owner.is_none() && self.lock.child.is_none() {
            self.lock.attempts = 0;
            self.lock.rescue_screen = false;
            self.launch_lock_program();
        }
    }

    /// The screen `output` has shown a frame built while locked.
    pub fn lock_frame_shown(&mut self, output: &Output) {
        if self.lock.locked && !self.lock.shown.contains(output) {
            self.lock.shown.push(output.clone());
        }
        self.confirm_lock();
    }

    /// Frame callbacks of the lock surface of `output`.
    pub fn send_lock_frames(&self, output: &Output, time: Duration) {
        for s in self.lock.surfaces.iter().filter(|s| &s.output == output) {
            let surface = s.surface.wl_surface();
            send_frames_surface_tree(surface, output, time, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        }
    }

    fn enter_lock(&mut self) {
        tracing::info!("session locked");
        self.sync_log();
        let lock = &mut self.lock;
        (lock.locked, lock.rescue_screen) = (true, false);
        lock.shown.clear();
        // Nothing started before keeps the input: gestures, menus, the area of a screenshot.
        self.gesture = None;
        self.cancel_gestures();
        self.close_overview();
        self.cancel_area();
        self.dismiss_popups();
        let time = self.now().as_millis() as u32;
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), time);
            self.pointer_motion(pointer.current_location(), time);
        }
        self.set_keyboard_focus(None);
        // No shape asked for by a window stays: the lock surface names its own.
        self.cursor_status = CursorImageStatus::default_named();
        let watch = self
            .loop_handle
            .insert_source(Timer::from_duration(WATCH), |_, _, state| {
                state.watch_lock_program()
            });
        match watch {
            Ok(token) => self.lock.watch = Some(token),
            Err(err) => tracing::warn!("lock: cannot watch the lock program: {}", err.error),
        }
        self.queue_redraw();
    }

    fn end_lock(&mut self) {
        tracing::info!("session unlocked");
        self.sync_log();
        (self.lock.ping, self.lock.pinged) = (None, None);
        let lock = &mut self.lock;
        (lock.locked, lock.rescue_screen) = (false, false);
        (lock.owner, lock.confirm, lock.next_launch) = (None, None, None);
        lock.surfaces.clear();
        lock.program = None;
        lock.attempts = 0;
        if let Some((mut child, _)) = lock.child.take() {
            // It leaves by itself once unlocked: a thread waits for it, so that it does not
            // stay a zombie.
            let _ = std::thread::Builder::new()
                .name("lock".into())
                .spawn(move || child.wait());
        }
        if let Some(token) = lock.watch.take() {
            self.loop_handle.remove(token);
        }
        self.note_input();
        let top = self.top_window();
        self.focus(top);
        if let Some(pointer) = self.seat.get_pointer() {
            self.pointer_motion(pointer.current_location(), self.now().as_millis() as u32);
        }
        self.queue_redraw();
    }

    /// Sends `locked` once every screen has shown a locked frame.
    fn confirm_lock(&mut self) {
        let all = self
            .screens
            .iter()
            .all(|s| self.lock.shown.contains(&s.output));
        if self.lock.locked
            && all
            && let Some(confirm) = self.lock.confirm.take()
        {
            tracing::info!("lock confirmed: every screen shows the lock");
            confirm.lock();
        }
    }

    fn launch_lock_program(&mut self) {
        let programs = self
            .lock
            .programs
            .clone()
            .unwrap_or_else(|| Programs::of(|key| std::env::var_os(key)));
        // The next stage that has a program: no rescue mode or no `swaylock` skips it.
        let mut command = None;
        while command.is_none() && Stage::of(self.lock.attempts) != Stage::Screen {
            let stage = Stage::of(self.lock.attempts);
            command = programs.command(stage).cloned();
            if command.is_none() {
                self.lock.attempts = match stage {
                    Stage::Rescue => CLIENT_ATTEMPTS + RESCUE_ATTEMPTS,
                    _ => self.lock.attempts + 1,
                };
            }
        }
        self.lock.stage = Stage::of(self.lock.attempts);
        self.lock.attempts += 1;
        let Some((program, arguments)) = command.as_ref().and_then(|c| c.split_first()) else {
            tracing::warn!("lock: every lock program failed, the compositor shows the rescue");
            self.lock.rescue_screen = true;
            self.queue_redraw();
            return;
        };
        // The compositor opens the program's connection itself and hands it over through
        // `WAYLAND_SOCKET`: that connection, and no other, may lock (ADR 0023).
        let (ours, theirs) = match UnixStream::pair() {
            Ok(pair) => pair,
            Err(err) => {
                tracing::warn!("lock program: no socket: {err}");
                self.lock_program_failed();
                return;
            }
        };
        self.insert_lock_program(ours);
        let fd = theirs.as_raw_fd();
        let mut child = Command::new(program);
        child
            .args(arguments)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env("WAYLAND_SOCKET", fd.to_string());
        // SAFETY: between fork and exec, only `fcntl` runs, which is async-signal-safe: it lets
        // the child keep its end of the socket, which only it gets (the parent's copy keeps
        // close-on-exec, so no other program inherits it).
        unsafe {
            child.pre_exec(move || {
                let fd = BorrowedFd::borrow_raw(fd);
                rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::empty())
                    .map_err(std::io::Error::from)
            });
        }
        let spawned = child.spawn();
        drop(theirs);
        match spawned {
            Ok(child) => {
                tracing::info!(program = ?program, pid = child.id(), "lock program started");
                self.lock.child = Some((child, Instant::now()));
            }
            Err(err) => {
                tracing::warn!(program = ?program, "lock program: {err}");
                self.lock_program_failed();
            }
        }
    }

    /// The lock program died, gave up its lock, or did not lock in time: the next one is
    /// started after a pause. The session stays locked.
    fn lock_program_failed(&mut self) {
        if let Some((mut child, _)) = self.lock.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.forget_lock_program();
        if self.lock.locked && self.lock.owner.is_none() {
            self.lock.next_launch = Some(Instant::now() + RELAUNCH_DELAY);
        }
    }

    /// The lock program is gone: its connection is closed, and nothing may lock until the next
    /// one starts.
    fn forget_lock_program(&mut self) {
        (self.lock.shell, self.lock.ping) = (None, None);
        if let Some(id) = self.lock.program.take() {
            self.display_handle
                .backend_handle()
                .kill_client(id, DisconnectReason::ConnectionClosed);
        }
    }

    /// Checks the lock program while locked: whether it ended, took too long to lock, or
    /// whether the next one is due.
    fn watch_lock_program(&mut self) -> TimeoutAction {
        if !self.lock.locked {
            self.lock.watch = None;
            return TimeoutAction::Drop;
        }
        if self.lock.synced.is_none_or(|at| at.elapsed() >= LOG_SYNC) {
            self.sync_log();
        }
        if self
            .lock
            .ping
            .is_some_and(|at| at.elapsed() > self.lock.ping_timeout)
        {
            tracing::warn!(
                timeout = ?self.lock.ping_timeout,
                "the lock program does not answer: killed, the session stays locked"
            );
            self.sync_log();
            // As if its lock went without an unlock, without waiting for the connection's end.
            (self.lock.owner, self.lock.confirm) = (None, None);
            self.lock.surfaces.clear();
            self.lock_program_failed();
            self.set_keyboard_focus(None);
            self.queue_redraw();
        }
        let owned = self.lock.owner.is_some();
        if let Some((child, started)) = self.lock.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) if !owned => {
                    tracing::warn!(%status, "lock program ended without a lock");
                    self.lock.child = None;
                    self.lock_program_failed();
                }
                Ok(Some(status)) => {
                    tracing::info!(%status, "lock program ended");
                    self.lock.child = None;
                    self.forget_lock_program();
                }
                Ok(None) if !owned && started.elapsed() > START_TIMEOUT => {
                    tracing::warn!("lock program: no lock after {START_TIMEOUT:?}");
                    self.lock_program_failed();
                }
                Ok(None) => {}
                Err(err) => tracing::warn!("lock program: {err}"),
            }
        }
        let due = self.lock.next_launch.is_some_and(|at| at <= Instant::now());
        if due && !owned && self.lock.child.is_none() {
            self.lock.next_launch = None;
            self.launch_lock_program();
        }
        TimeoutAction::ToDuration(WATCH)
    }

    /// Asks for the log to be written to disk.
    fn sync_log(&mut self) {
        self.lock.synced = Some(Instant::now());
        self.log_sync.request();
    }

    /// Enter on the rescue screen: the lock programs are tried again from the start.
    pub(crate) fn retry_lock_program(&mut self) {
        if self.lock_rescue_shown() && self.lock.owner.is_none() && self.lock.child.is_none() {
            (self.lock.attempts, self.lock.rescue_screen) = (0, false);
            self.launch_lock_program();
            self.queue_redraw();
        }
    }

    /// A lock object was destroyed. The lock that held the session, without unlocking it: the
    /// session stays locked, and a lock program is started again.
    fn lock_destroyed(&mut self, lock: &ExtSessionLockV1) {
        if self.lock.owner.as_ref() != Some(lock) {
            return;
        }
        tracing::warn!("the lock program left without unlocking: the session stays locked");
        (self.lock.owner, self.lock.confirm) = (None, None);
        // Its surfaces go with it, alive or not: none is drawn nor gets the keyboard any more.
        self.lock.surfaces.clear();
        self.lock_program_failed();
        self.set_keyboard_focus(None);
        self.queue_redraw();
    }

    /// Cleanup of each loop iteration: dead lock surfaces, their size and scale, and the
    /// confirmation when no screen is left to show a frame.
    pub(crate) fn refresh_lock(&mut self) {
        if !self.lock.locked {
            return;
        }
        self.lock.surfaces.retain(|s| s.surface.alive());
        for s in &mut self.lock.surfaces {
            let Some(area) = self.space.output_geometry(&s.output) else {
                continue;
            };
            if area.size != s.size {
                s.size = area.size;
                configure(&s.surface, area.size);
            }
            let scale = s.output.current_scale();
            with_surface_tree_downward(
                s.surface.wl_surface(),
                (),
                |_, _, _| TraversalAction::DoChildren(()),
                |surface, states, _| {
                    with_fractional_scale(states, |fractional| {
                        fractional.set_preferred_scale(scale.fractional_scale());
                    });
                    send_surface_state(surface, states, scale.integer_scale(), Transform::Normal);
                },
                |_, _, _| true,
            );
        }
        self.confirm_lock();
    }

    /// The lock surface that has the keyboard: the one of the screen under the pointer, or
    /// the first one.
    pub(crate) fn lock_focus(&self) -> Option<KeyboardFocus> {
        let output = self.pointer_output();
        let surfaces = &self.lock.surfaces;
        surfaces
            .iter()
            .find(|s| Some(&s.output) == output.as_ref())
            .or(surfaces.first())
            .map(|s| KeyboardFocus::Wayland(s.surface.wl_surface().clone()))
    }

    /// The lock surface under `pos`, and its origin in the space.
    pub(crate) fn lock_hit(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.space.output_under(pos).next()?;
        let origin = self.space.output_geometry(output)?.loc;
        let s = self.lock.surfaces.iter().find(|s| &s.output == output)?;
        let surface = s.surface.wl_surface();
        let (hit, at) = under_from_surface_tree(surface, pos, origin, WindowSurfaceType::ALL)?;
        Some((hit, at.to_f64()))
    }

    fn accept_lock(&mut self, locker: SessionLocker) {
        let lock = locker.ext_session_lock().clone();
        let client = lock.client().map(|c| c.id());
        let held = self.lock.owner.as_ref().is_some_and(Resource::is_alive);
        let program = client.is_some() && client == self.lock.program;
        if held || !program {
            // Dropping the locker sends `finished`. Only the lock program the compositor
            // started may lock, the session locked or not (ADR 0023).
            tracing::warn!(held, program, "lock refused");
            return;
        }
        tracing::info!("lock accepted");
        if !self.lock.locked {
            self.enter_lock();
        }
        self.lock.rescue_screen = false;
        self.lock.owner = Some(lock);
        self.lock.confirm = Some(locker);
        self.confirm_lock();
    }
}

fn configure(surface: &LockSurface, size: Size<i32, Logical>) {
    let size = (size.w.max(1) as u32, size.h.max(1) as u32);
    surface.with_pending_state(|state| state.size = Some(size.into()));
    surface.send_configure();
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.lock.manager
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        self.accept_lock(confirmation);
    }

    /// Reached only through `Dispatch<ExtSessionLockV1>` below, which checks the lock first.
    fn unlock(&mut self) {
        if self.lock.locked {
            self.end_lock();
        }
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let ours = self.lock.reading.is_some() && self.lock.reading == self.lock.owner;
        let output = Output::from_resource(&output);
        let area = output.as_ref().and_then(|o| self.space.output_geometry(o));
        let (Some(output), Some(area), true) = (output, area, ours) else {
            // A lock refused, or a screen gone: the surface is never shown.
            tracing::warn!("lock surface ignored");
            return;
        };
        // smithay sends the first configure after this, with this size.
        let size = (area.size.w.max(1) as u32, area.size.h.max(1) as u32);
        surface.with_pending_state(|state| state.size = Some(size.into()));
        output.enter(surface.wl_surface());
        self.lock.surfaces.retain(|s| s.output != output);
        self.lock.surfaces.push(Surface {
            surface,
            output,
            size: area.size,
        });
        self.set_keyboard_focus(None);
        self.queue_redraw();
    }
}

delegate_global_dispatch!(State: [ExtSessionLockManagerV1: SessionLockManagerGlobalData] => SessionLockManagerState);
delegate_dispatch!(State: [ExtSessionLockManagerV1: ()] => SessionLockManagerState);
delegate_dispatch!(State: [ExtSessionLockSurfaceV1: ExtLockSurfaceUserData] => SessionLockManagerState);

/// The binding of `xdg_wm_base` goes to smithay, the client known meanwhile: `new_client`
/// names the shell client only, and the lock program's is the one pinged (lot D-25f).
impl GlobalDispatch<XdgWmBase, ()> for State {
    fn bind(
        state: &mut Self,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<XdgWmBase>,
        global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.lock.binding = Some(client.id());
        <XdgShellState as GlobalDispatch<XdgWmBase, (), Self>>::bind(
            state,
            dh,
            client,
            resource,
            global_data,
            data_init,
        );
        state.lock.binding = None;
    }
}

/// Requests of a lock go to smithay, as `delegate_session_lock!` would send them, after two
/// checks smithay does not make: `unlock_and_destroy` ends the lock only from the lock that
/// holds it, once `locked` was sent (smithay unlocks for any lock object, even after an
/// error), and a surface of a lock refused is never shown.
impl Dispatch<ExtSessionLockV1, SessionLockState> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &ExtSessionLockV1,
        request: ext_session_lock_v1::Request,
        data: &SessionLockState,
        dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_session_lock_v1::Request::UnlockAndDestroy = request {
            let holds = state.lock.owner.as_ref() == Some(resource);
            if !holds || !state.lock_confirmed() {
                tracing::warn!(holds, "unlock refused");
                resource.post_error(
                    ext_session_lock_v1::Error::InvalidUnlock,
                    "this lock does not hold the session",
                );
                return;
            }
        }
        state.lock.reading = Some(resource.clone());
        <SessionLockManagerState as Dispatch<_, _, Self>>::request(
            state, client, resource, request, data, dh, data_init,
        );
        state.lock.reading = None;
    }

    fn destroyed(
        state: &mut Self,
        client: ClientId,
        resource: &ExtSessionLockV1,
        data: &SessionLockState,
    ) {
        <SessionLockManagerState as Dispatch<_, _, Self>>::destroyed(state, client, resource, data);
        state.lock_destroyed(resource);
    }
}

/// What a screen shows while locked, from front to back: the on-screen display of the media
/// keys, the lock surface of the screen, the plain background, and the rescue screen when no
/// lock program could start.
pub fn items(state: &State, (output, view): (&Output, &View)) -> (Vec<Item>, bool) {
    let mut items = Vec::new();
    let animating = crate::osd::push_item(&mut items, state, (output, view));
    let colors = ui::Colors::of(color::palette());
    let surface = state
        .lock
        .surfaces
        .iter()
        .find(|s| &s.output == output)
        .map(|s| s.surface.wl_surface())
        .filter(|surface| has_content(surface));
    let scale = view.scale as f32;
    if let Some(surface) = surface {
        items.push(Item::Surface {
            surface: surface.clone(),
            origin: view.frame.loc,
            scale: view.scale,
            alpha: 1.0,
        });
    } else if state.lock.rescue_screen {
        let height = f64::from(ui::RESCUE_LABEL_HEIGHT);
        let gap = f64::from(ui::RESCUE_GAP);
        let center = view.area.loc.to_f64() + view.area.size.to_f64().downscale(2.0).to_point();
        for (i, text) in [ui::RESCUE_TITLE, ui::RESCUE_HINT].into_iter().enumerate() {
            let width = f64::from(label_width(text));
            let y = center.y - height - gap / 2.0 + i as f64 * (height + gap);
            let loc = (center.x - width / 2.0, y);
            let rect = view.rect(Rectangle::new(loc.into(), (width, height).into()));
            items.push(Item::Label {
                key: Key::of(None, Part::LockLabel(i)),
                rect,
                label: Label {
                    text: text.to_owned(),
                    size: (rect.size.w.max(1) as u32, rect.size.h.max(1) as u32),
                    scale,
                    background: colors.label,
                    color: colors.label_text,
                    icon: None,
                },
                alpha: 1.0,
            });
        }
    }
    let fill = colors.background.premultiplied();
    items.push(Item::Shape {
        key: Key::of(None, Part::LockBackground),
        rect: view.frame,
        fill,
        border: fill,
        border_width: 0.0,
        radius: 0.0,
        alpha: 1.0,
    });
    (items, animating)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_program_is_found_beside_the_programs_then_in_the_path() {
        let dir = std::env::temp_dir().join(format!("lock-bin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let var = |dir: &Path| {
            let dir = dir.as_os_str().to_owned();
            move |key: &str| (key == BIN_VAR).then(|| dir.clone())
        };
        assert_eq!(Programs::of(var(&dir)).client, [OsString::from(PROGRAM)]);
        std::fs::write(dir.join(PROGRAM), b"").unwrap();
        let programs = Programs::of(var(&dir));
        assert_eq!(programs.client, [dir.join(PROGRAM).into_os_string()]);
        let rescue = programs.rescue.unwrap();
        assert_eq!(
            rescue,
            [dir.join(PROGRAM).into_os_string(), RESCUE_ARGUMENT.into()]
        );
        // swaylock only when it is installed.
        assert_eq!(programs.swaylock, None);
        std::fs::write(dir.join("swaylock"), b"").unwrap();
        let path = dir.as_os_str().to_owned();
        let with_path = |key: &str| (key == "PATH").then(|| path.clone());
        let swaylock = Programs::of(with_path).swaylock.unwrap();
        assert_eq!(swaylock[0], dir.join("swaylock").into_os_string());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_stages_follow_each_other() {
        let stages: Vec<_> = (0..8).map(Stage::of).collect();
        use Stage::*;
        assert_eq!(
            stages,
            [
                Client, Client, Client, Rescue, Rescue, Rescue, Swaylock, Screen
            ]
        );
    }
}
