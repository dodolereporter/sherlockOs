//! The consent window of Sherlock (lot A-11b, ADR 0031): `sherlockd` asks, on a private socket
//! of the compositor's, whether a tool that is not a read may run; the compositor draws the
//! window itself and answers on that socket.
//!
//! - **Drawn by the compositor**, in front of everything, the overlay layer and the screenshot
//!   tool included: no client draws it, nothing draws above it. Its texts are ours; the tool's
//!   name and arguments are shown as text, cut, never as a client's surface.
//! - **Its input is reserved**: while it is shown, no client has the keyboard nor the pointer
//!   (as in Mission Control), popups are dismissed, and every key goes to it. A client cannot
//!   click it: Wayland has no request for that.
//! - **Refused by default**: Escape, « Refuser », the asking process gone (its socket closed)
//!   all refuse or withdraw. Its buttons do nothing for `consent::ARMING` after it appears.
//! - **The reinforced consent** of a destructive tool: « Autoriser » held for `consent::HOLD`;
//!   Enter does not grant it.
//!
//! Protocol: one JSON object per line. Asked: `{"id": 7, "tool": "empty_trash", "level":
//! "destructive", "arguments": "{…}"}`, `level` being `action` or `destructive`. Answered:
//! `{"id": 7, "decision": "granted"}` or `"denied"`. Anything else closes the connection.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sherlock_ui::color;
use sherlock_ui::consent::{self as ui, Colors, text};
use smithay::{
    input::keyboard::Keysym,
    output::Output,
    reexports::{
        calloop::{EventLoop, Interest, Mode, PostAction, RegistrationToken, generic::Generic},
        rustix,
        wayland_server::ListeningSocket,
    },
    utils::{Logical, Point, Rectangle},
};

use crate::{
    State,
    damage::{Key, Part},
    mission_control::{Label, RectF},
    output::View,
    render::Item,
};

/// Bytes of one request at most, a line: more closes the connection.
const MAX_LINE: usize = 8 * 1024;
/// Requests waiting, for one connection and in all: more are refused at once.
const MAX_PENDING: usize = 4;
const MAX_QUEUE: usize = 16;
/// Characters of a tool name shown.
const MAX_TOOL: usize = 128;

/// Who may ask, here and on the windows socket (lot A-12): `sherlockd` of this user (the
/// session), or any process of this user (tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Peers {
    #[default]
    Sherlockd,
    SameUser,
}

/// What a request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Action,
    Destructive,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    id: u64,
    tool: String,
    level: Level,
    arguments: String,
}

/// A request waiting for the user, and the connection to answer on.
#[derive(Debug)]
struct Pending {
    conn: u64,
    id: u64,
    tool: String,
    level: Level,
    arguments: String,
}

/// A button of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Allow,
    Deny,
}

/// The consent socket, its connections and the requests waiting, the first one shown.
#[derive(Default)]
pub(crate) struct Consents {
    peers: Peers,
    listener: Option<RegistrationToken>,
    conns: Vec<(u64, UnixStream, RegistrationToken)>,
    queue: VecDeque<Pending>,
    next_conn: u64,
    /// When the first request was shown, for `ARMING`.
    shown_at: Option<Duration>,
    /// « Autoriser » held down since.
    held: Option<Duration>,
    hovered: Option<Choice>,
}

/// The socket next to the Wayland socket at `socket`: `<name>-consent`.
pub fn socket_path(socket: &Path) -> PathBuf {
    let mut name = socket.file_name().unwrap_or_default().to_owned();
    name.push("-consent");
    socket.with_file_name(name)
}

/// Whether the process at the other end of `stream` may ask.
pub(crate) fn allowed(stream: &UnixStream, peers: Peers) -> bool {
    let Ok(peer) = rustix::net::sockopt::socket_peercred(stream) else {
        return false;
    };
    if peer.uid != rustix::process::getuid() {
        return false;
    }
    match peers {
        Peers::SameUser => true,
        Peers::Sherlockd => {
            let exe = format!("/proc/{}/exe", peer.pid.as_raw_nonzero());
            let program = std::fs::read_link(exe).ok();
            let name = program
                .as_deref()
                .and_then(Path::file_name)
                .and_then(|n| n.to_str());
            name.is_some_and(|n| n.strip_suffix(" (deleted)").unwrap_or(n) == "sherlockd")
        }
    }
}

/// `text` cut to `max` characters, « … » at the end when cut.
fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

/// The arguments on lines of `ARGUMENT_COLUMNS` characters, `ARGUMENT_LINES` at most.
fn argument_lines(arguments: &str) -> Vec<String> {
    let chars: Vec<char> = arguments.chars().filter(|c| !c.is_control()).collect();
    let mut lines: Vec<String> = chars
        .chunks(ui::ARGUMENT_COLUMNS)
        .map(|c| c.iter().collect())
        .collect();
    if lines.len() > ui::ARGUMENT_LINES {
        lines.truncate(ui::ARGUMENT_LINES);
        if let Some(last) = lines.last_mut() {
            last.pop();
            last.push('…');
        }
    }
    lines
}

impl State {
    /// Opens the consent socket next to the Wayland socket at `socket`, for `peers`.
    pub fn listen_consent_next_to(
        &mut self,
        socket: &Path,
        peers: Peers,
        event_loop: &EventLoop<'static, State>,
    ) -> Result<(), crate::Error> {
        let path = socket_path(socket);
        let listening = ListeningSocket::bind_absolute(path.clone())?;
        let source = Generic::new(listening, Interest::READ, Mode::Level);
        let token = event_loop
            .handle()
            .insert_source(source, |_, socket, state| {
                while let Some(stream) = socket.accept()? {
                    state.accept_consent(stream);
                }
                Ok(PostAction::Continue)
            })
            .map_err(|err| err.error)?;
        self.consent.peers = peers;
        self.consent.listener = Some(token);
        tracing::info!(?path, "consent socket open");
        Ok(())
    }

    /// Removes the consent socket and closes its connections: what waits is refused.
    pub(crate) fn stop_consent_socket(&mut self) {
        if let Some(token) = self.consent.listener.take() {
            self.loop_handle.remove(token);
        }
        for (_, _, token) in std::mem::take(&mut self.consent.conns) {
            self.loop_handle.remove(token);
        }
        self.consent.queue.clear();
    }

    fn accept_consent(&mut self, stream: UnixStream) {
        if !allowed(&stream, self.consent.peers) {
            tracing::warn!("consent socket: a process that is not sherlockd refused");
            return;
        }
        let (Ok(reader), Ok(())) = (stream.try_clone(), stream.set_nonblocking(true)) else {
            return;
        };
        let conn = self.consent.next_conn;
        self.consent.next_conn += 1;
        let mut buffer = Vec::new();
        let source = Generic::new(reader, Interest::READ, Mode::Level);
        let inserted = self
            .loop_handle
            .insert_source(source, move |_, reader, state| {
                let mut chunk = [0u8; 4096];
                // SAFETY: the stream is only read, never closed, while the source holds it.
                let reader = unsafe { reader.get_mut() };
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) => {
                            state.consent_gone(conn);
                            return Ok(PostAction::Remove);
                        }
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => {
                            state.consent_gone(conn);
                            return Ok(PostAction::Remove);
                        }
                    }
                }
                while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buffer.drain(..=end).collect();
                    if !state.consent_asked(conn, &line) {
                        state.consent_gone(conn);
                        return Ok(PostAction::Remove);
                    }
                }
                if buffer.len() > MAX_LINE {
                    state.consent_gone(conn);
                    return Ok(PostAction::Remove);
                }
                Ok(PostAction::Continue)
            });
        match inserted {
            Ok(token) => self.consent.conns.push((conn, stream, token)),
            Err(err) => tracing::warn!("consent connection: {}", err.error),
        }
    }

    /// A line from connection `conn`: a request, queued. `false` for anything malformed.
    fn consent_asked(&mut self, conn: u64, line: &[u8]) -> bool {
        if line.len() > MAX_LINE {
            return false;
        }
        let Ok(asked) = serde_json::from_slice::<Asked>(line) else {
            return false;
        };
        let of_conn = self.consent.queue.iter().filter(|p| p.conn == conn).count();
        if of_conn >= MAX_PENDING || self.consent.queue.len() >= MAX_QUEUE {
            self.consent_reply(conn, asked.id, false);
            return true;
        }
        let was_shown = self.consent_shown();
        self.consent.queue.push_back(Pending {
            conn,
            id: asked.id,
            // Shown as text, on one line: no control character from the asker.
            tool: cut(&asked.tool.replace(char::is_control, " "), MAX_TOOL),
            level: asked.level,
            arguments: asked.arguments,
        });
        if !was_shown {
            self.consent_appears();
        }
        true
    }

    /// The asking process is gone: its requests are withdrawn, unanswered.
    fn consent_gone(&mut self, conn: u64) {
        let front = self.consent.queue.front().map(|p| p.conn);
        self.consent.queue.retain(|p| p.conn != conn);
        if let Some(at) = self.consent.conns.iter().position(|c| c.0 == conn) {
            self.consent.conns.remove(at);
        }
        if front == Some(conn) {
            self.consent_next();
        }
    }

    fn consent_reply(&mut self, conn: u64, id: u64, granted: bool) {
        let decision = if granted { "granted" } else { "denied" };
        let line = format!(
            "{}\n",
            serde_json::json!({ "id": id, "decision": decision })
        );
        let written = self.consent.conns.iter_mut().find(|c| c.0 == conn);
        if let Some((_, stream, _)) = written
            && let Err(error) = stream.write_all(line.as_bytes())
        {
            tracing::warn!(%error, "consent answer not sent");
        }
    }

    /// The first request is answered: `granted` or not.
    fn consent_answer(&mut self, granted: bool) {
        let Some(pending) = self.consent.queue.pop_front() else {
            return;
        };
        tracing::info!(tool = pending.tool, granted, "consent answered");
        self.consent_reply(pending.conn, pending.id, granted);
        self.consent_next();
    }

    /// The next request comes, or the window goes and the keyboard goes back.
    fn consent_next(&mut self) {
        self.consent.held = None;
        self.consent.hovered = None;
        if self.consent.queue.is_empty() {
            self.consent.shown_at = None;
            self.queue_redraw();
            self.focus(self.top_window());
            self.refresh_pointer_focus();
        } else {
            self.consent_appears();
        }
    }

    /// The window appears: popups dismissed, the keyboard and the pointer taken from clients.
    fn consent_appears(&mut self) {
        self.consent.shown_at = Some(self.now());
        self.consent.held = None;
        self.queue_redraw();
        self.dismiss_popups();
        self.set_keyboard_focus(None);
        if let Some(pointer) = self.seat.get_pointer() {
            let time = self.now().as_millis() as u32;
            self.pointer_motion(pointer.current_location(), time);
        }
    }

    /// The consent window is shown: a request waits, and the session is not locked.
    pub fn consent_shown(&self) -> bool {
        !self.consent.queue.is_empty() && !self.is_locked()
    }

    /// The buttons act: past `ARMING` since the window appeared.
    fn consent_armed(&self) -> bool {
        let shown = self.consent.shown_at.unwrap_or_default();
        self.now().saturating_sub(shown) >= ui::ARMING
    }

    /// A key while the window is shown: Escape refuses, Enter grants an action-level request.
    pub(crate) fn consent_key(&mut self, sym: Keysym) {
        let level = self.consent.queue.front().map(|p| p.level);
        match sym {
            Keysym::Escape if self.consent_armed() => self.consent_answer(false),
            Keysym::Return | Keysym::KP_Enter
                if self.consent_armed() && level == Some(Level::Action) =>
            {
                self.consent_answer(true);
            }
            _ => {}
        }
    }

    /// The pointer moved to `at`: which button it is over.
    pub(crate) fn consent_motion(&mut self, at: Point<f64, Logical>) {
        let hovered = self.consent_target(at);
        if hovered != self.consent.hovered {
            self.consent.hovered = hovered;
            if hovered != Some(Choice::Allow) {
                self.consent.held = None;
            }
            self.queue_redraw();
        }
    }

    /// A button of the mouse at `at`.
    pub(crate) fn consent_button(&mut self, pressed: bool, at: Point<f64, Logical>) {
        let target = self.consent_target(at);
        let level = self.consent.queue.front().map(|p| p.level);
        let now = self.now();
        match (pressed, target, level) {
            (true, Some(Choice::Allow), Some(Level::Destructive)) if self.consent_armed() => {
                self.consent.held = Some(now);
            }
            (false, Some(Choice::Allow), Some(Level::Destructive)) => {
                let held = self.consent.held.take();
                if held.is_some_and(|since| now.saturating_sub(since) >= ui::HOLD) {
                    self.consent_answer(true);
                }
            }
            (false, Some(Choice::Allow), Some(Level::Action)) if self.consent_armed() => {
                self.consent_answer(true);
            }
            (false, Some(Choice::Deny), _) if self.consent_armed() => self.consent_answer(false),
            (false, ..) => self.consent.held = None,
            _ => {}
        }
        self.queue_redraw();
    }

    /// The button under `at`, on whichever screen.
    pub fn consent_target(&self, at: Point<f64, Logical>) -> Option<Choice> {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        outputs.iter().find_map(|output| {
            let layout = self.consent_layout(output)?;
            [(Choice::Allow, layout.allow), (Choice::Deny, layout.deny)]
                .into_iter()
                .find(|(_, rect)| rect.contains(at))
                .map(|(choice, _)| choice)
        })
    }

    /// Where the window and its parts are on `output`, in the space.
    pub fn consent_layout(&self, output: &Output) -> Option<ConsentLayout> {
        let pending = self
            .consent
            .queue
            .front()
            .filter(|_| self.consent_shown())?;
        let screen = self.space.output_geometry(output)?.to_f64();
        let mut lines = vec![text::TITLE.to_owned(), text::tool(&pending.tool)];
        lines.extend(argument_lines(&pending.arguments));
        lines.push(match pending.level {
            Level::Action => text::ACTION.to_owned(),
            Level::Destructive => format!("{} {}", text::DESTRUCTIVE, text::HOLD),
        });
        let line = f64::from(ui::LINE_HEIGHT + ui::LINE_GAP);
        let (padding, width) = (f64::from(ui::PADDING), f64::from(ui::WIDTH));
        let buttons = f64::from(ui::BUTTON_HEIGHT);
        let height = 2.0 * padding + lines.len() as f64 * line + padding + buttons;
        let x = screen.loc.x + (screen.size.w - width) / 2.0;
        let y = screen.loc.y + (screen.size.h - height) / 2.0;
        let panel = Rectangle::new((x, y).into(), (width, height).into());
        let (bw, gap) = (f64::from(ui::BUTTON_WIDTH), f64::from(ui::BUTTON_GAP));
        let by = y + height - padding - buttons;
        let allow = Rectangle::new((x + width - padding - bw, by).into(), (bw, buttons).into());
        let deny = Rectangle::new((allow.loc.x - gap - bw, by).into(), (bw, buttons).into());
        Some(ConsentLayout {
            screen,
            panel,
            lines,
            allow,
            deny,
            destructive: pending.level == Level::Destructive,
        })
    }
}

/// The window on one screen.
#[derive(Debug, Clone)]
pub struct ConsentLayout {
    pub screen: RectF,
    pub panel: RectF,
    pub lines: Vec<String>,
    pub allow: RectF,
    pub deny: RectF,
    pub destructive: bool,
}

/// The window on `output`, in front of everything, and whether it moves (a hold under way).
pub(crate) fn push_items(
    items: &mut Vec<Item>,
    state: &State,
    (output, view): (&Output, &View),
) -> bool {
    let Some(layout) = state.consent_layout(output) else {
        return false;
    };
    let colors = Colors::of(color::palette());
    let scale = view.scale as f32;
    let mut part = 0;
    let mut shape = |items: &mut Vec<Item>, rect: RectF, fill: color::Color, radius: f32| {
        items.push(Item::Shape {
            key: Key::of(None, Part::Consent(part)),
            rect: view.rect(rect),
            fill: fill.premultiplied(),
            border: fill.premultiplied(),
            border_width: 0.0,
            radius: radius * scale,
            alpha: 1.0,
        });
        part += 1;
    };
    let mut labels = 0;
    let mut label = |items: &mut Vec<Item>, rect: RectF, text: &str, bg, fg| {
        let at = view.rect(rect);
        items.push(Item::Label {
            key: Key::of(None, Part::ConsentLabel(labels)),
            rect: at,
            label: Label {
                text: text.to_owned(),
                size: (at.size.w.max(1) as u32, at.size.h.max(1) as u32),
                scale,
                background: bg,
                color: fg,
                icon: None,
            },
            alpha: 1.0,
        });
        labels += 1;
    };
    let held = state.consent.held.map(|since| {
        let ratio = state.now().saturating_sub(since).as_secs_f64() / ui::HOLD.as_secs_f64();
        ratio.min(1.0)
    });
    // Front to back: the buttons' texts, the hold's fill, the buttons, the lines, the panel,
    // the scrim over the whole screen.
    let allow_color = if layout.destructive {
        colors.danger
    } else {
        colors.allow
    };
    label(
        items,
        layout.allow,
        text::ALLOW,
        allow_color,
        colors.on_button,
    );
    label(items, layout.deny, text::DENY, colors.deny, colors.text);
    if let Some(ratio) = held {
        let mut fill = layout.allow;
        fill.size.h = 4.0;
        fill.loc.y += layout.allow.size.h - 4.0;
        fill.size.w *= ratio;
        shape(items, fill, colors.on_button, 2.0);
    }
    let (padding, line) = (
        f64::from(ui::PADDING),
        f64::from(ui::LINE_HEIGHT + ui::LINE_GAP),
    );
    for (i, text) in layout.lines.iter().enumerate() {
        let y = layout.panel.loc.y + padding + i as f64 * line;
        let rect = Rectangle::new(
            (layout.panel.loc.x + padding, y).into(),
            (
                layout.panel.size.w - 2.0 * padding,
                f64::from(ui::LINE_HEIGHT),
            )
                .into(),
        );
        let fg = if i == 0 {
            colors.text
        } else {
            colors.secondary
        };
        label(items, rect, text, colors.panel, fg);
    }
    shape(items, layout.panel, colors.panel, ui::RADIUS);
    shape(items, layout.screen, colors.scrim, 0.0);
    held.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arguments_are_cut_and_never_break_a_line() {
        let lines = argument_lines(&format!("{{\"a\":\"x\ny\"}}{}", "z".repeat(500)));
        assert_eq!(lines.len(), ui::ARGUMENT_LINES);
        assert!(lines.iter().all(|l| !l.contains('\n')));
        assert!(lines.last().unwrap().ends_with('…'));
        assert_eq!(
            cut("é".repeat(300).as_str(), MAX_TOOL).chars().count(),
            MAX_TOOL + 1
        );
        assert_eq!(
            socket_path(Path::new("/run/user/1000/wayland-1")),
            Path::new("/run/user/1000/wayland-1-consent")
        );
    }
}
