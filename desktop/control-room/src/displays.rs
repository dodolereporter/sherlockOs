//! The screens, through `wlr-output-management` (ADR 0025, lot D-26b): a Wayland connection of
//! its own, in a thread of its own while the « Écrans » page is open. It hands the page every
//! state the compositor announces (`Screens`), and applies the configurations the page sends
//! (`Request`), answering whether the compositor took them.
//!
//! What the compositor sends is bounded here too: at most `MAX_HEADS` heads of `MAX_MODES`
//! modes each, names cut to `MAX_NAME` characters.

use std::sync::mpsc;
use std::time::Duration;

use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child,
    protocol::{wl_output::Transform, wl_registry},
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self as configuration, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self as head, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self as manager, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self as mode, ZwlrOutputModeV1},
};

pub const MAX_HEADS: usize = 16;
pub const MAX_MODES: usize = 256;
pub const MAX_NAME: usize = 64;

/// How often the thread looks for a request of the page, between two reads of the compositor.
const POLL: Duration = Duration::from_millis(40);

/// A mode: size in pixels and refresh in millihertz (0: not given).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    pub refresh: i32,
}

/// A screen, as the compositor describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    /// The connector: `eDP-1`, `HDMI-A-1`…
    pub name: String,
    pub make: String,
    pub model: String,
    /// The preferred mode first.
    pub modes: Vec<Mode>,
    /// On, at this mode, place, rotation and scale; `None` when off.
    pub on: Option<Lit>,
}

/// What a screen that is on shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lit {
    pub mode: Mode,
    pub position: (i32, i32),
    /// `wl_output.transform`: 0 normal, 1 to 3 a quarter turn each, clockwise.
    pub transform: u32,
    pub scale: f64,
}

impl Lit {
    /// Its size in the space: the mode, turned, divided by the scale.
    pub fn logical_size(&self) -> (f64, f64) {
        let (w, h) = (f64::from(self.mode.width), f64::from(self.mode.height));
        let (w, h) = if self.transform % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        };
        (w / self.scale, h / self.scale)
    }
}

/// What the thread tells the page.
#[derive(Debug, Clone)]
pub enum Event {
    /// The requests go there.
    Ready(mpsc::Sender<Request>),
    /// The screens, after each change.
    Screens(Vec<Screen>),
    /// The answer to the last configuration.
    Answer(Answer),
    /// No connection, or the compositor does not offer the protocol.
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Applied,
    Refused,
    /// The screens changed meanwhile: the page sends it again on the new state.
    Outdated,
}

/// A whole configuration, one entry per screen by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Request(pub Vec<(String, Option<Lit>)>);

/// What the client knows of a head.
#[derive(Default)]
struct Head {
    proxy: Option<ZwlrOutputHeadV1>,
    name: String,
    make: String,
    model: String,
    modes: Vec<(ZwlrOutputModeV1, Mode, bool)>,
    enabled: bool,
    current: Option<ZwlrOutputModeV1>,
    position: (i32, i32),
    transform: u32,
    scale: f64,
}

#[derive(Default)]
struct State {
    manager: Option<(u32, u32)>,
    heads: Vec<Head>,
    serial: Option<u32>,
    /// A `done` came since the screens were last handed over.
    changed: bool,
    answer: Option<Answer>,
}

fn bounded(text: String) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect()
}

impl State {
    fn screens(&self) -> Vec<Screen> {
        self.heads
            .iter()
            .map(|h| {
                let mut modes: Vec<Mode> = Vec::new();
                let preferred = h.modes.iter().filter(|(_, _, p)| *p);
                for (_, mode, _) in preferred.chain(h.modes.iter()) {
                    if !modes.contains(mode) {
                        modes.push(*mode);
                    }
                }
                let current = h
                    .current
                    .as_ref()
                    .and_then(|c| h.modes.iter().find(|(m, _, _)| m == c))
                    .map(|(_, mode, _)| *mode);
                let on = current.filter(|_| h.enabled).map(|mode| Lit {
                    mode,
                    position: h.position,
                    transform: h.transform,
                    scale: h.scale,
                });
                Screen {
                    name: h.name.clone(),
                    make: h.make.clone(),
                    model: h.model.clone(),
                    modes,
                    on,
                }
            })
            .collect()
    }

    fn head(&mut self, proxy: &ZwlrOutputHeadV1) -> Option<&mut Head> {
        self.heads
            .iter_mut()
            .find(|h| h.proxy.as_ref() == Some(proxy))
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == ZwlrOutputManagerV1::interface().name
        {
            state.manager = Some((name, version.min(3)));
        }
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager::Event::Head { head } => {
                if state.heads.len() < MAX_HEADS {
                    state.heads.push(Head {
                        proxy: Some(head),
                        scale: 1.0,
                        ..Head::default()
                    });
                }
            }
            manager::Event::Done { serial } => {
                state.serial = Some(serial);
                state.changed = true;
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwlrOutputManagerV1, [
        manager::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: head::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let head::Event::Finished = event {
            state.heads.retain(|h| h.proxy.as_ref() != Some(proxy));
            return;
        }
        let Some(head) = state.head(proxy) else {
            return;
        };
        match event {
            head::Event::Name { name } => head.name = bounded(name),
            head::Event::Make { make } => head.make = bounded(make),
            head::Event::Model { model } => head.model = bounded(model),
            head::Event::Mode { mode } => {
                if head.modes.len() < MAX_MODES {
                    let blank = Mode {
                        width: 0,
                        height: 0,
                        refresh: 0,
                    };
                    head.modes.push((mode, blank, false));
                }
            }
            head::Event::Enabled { enabled } => head.enabled = enabled != 0,
            head::Event::CurrentMode { mode } => head.current = Some(mode),
            head::Event::Position { x, y } => head.position = (x, y),
            head::Event::Transform { transform } => {
                head.transform = match transform {
                    WEnum::Value(t) => u32::from(t),
                    WEnum::Unknown(_) => 0,
                };
            }
            head::Event::Scale { scale } if scale.is_finite() && scale > 0.0 => {
                head.scale = scale;
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwlrOutputHeadV1, [
        head::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: mode::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        for head in &mut state.heads {
            if let mode::Event::Finished = event {
                head.modes.retain(|(m, _, _)| m != proxy);
                continue;
            }
            let Some((_, mode, preferred)) = head.modes.iter_mut().find(|(m, _, _)| m == proxy)
            else {
                continue;
            };
            match event {
                mode::Event::Size { width, height } => {
                    (mode.width, mode.height) = (width.clamp(0, 65_535), height.clamp(0, 65_535));
                }
                mode::Event::Refresh { refresh } => mode.refresh = refresh.clamp(0, 10_000_000),
                mode::Event::Preferred => *preferred = true,
                _ => {}
            }
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputConfigurationV1,
        event: configuration::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.answer = Some(match event {
            configuration::Event::Succeeded => Answer::Applied,
            configuration::Event::Failed => Answer::Refused,
            _ => Answer::Outdated,
        });
        proxy.destroy();
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationHeadV1,
        _: <ZwlrOutputConfigurationHeadV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Talks to the compositor until `send` fails (the page closed) or the connection ends.
/// `send` returns `false` once nobody listens.
pub fn run(send: impl Fn(Event) -> bool) {
    let Ok(conn) = Connection::connect_to_env() else {
        tracing::warn!("screens: no Wayland connection");
        send(Event::Unavailable);
        return;
    };
    let mut queue: EventQueue<State> = conn.new_event_queue();
    let qh = queue.handle();
    let registry = conn.display().get_registry(&qh, ());
    let mut state = State::default();
    if queue.roundtrip(&mut state).is_err() {
        send(Event::Unavailable);
        return;
    }
    let Some((name, version)) = state.manager else {
        tracing::warn!("screens: the compositor does not offer wlr-output-management");
        send(Event::Unavailable);
        return;
    };
    let manager: ZwlrOutputManagerV1 = registry.bind(name, version, &qh, ());
    let (requests, incoming) = mpsc::channel();
    if !send(Event::Ready(requests)) {
        return;
    }
    loop {
        if conn.flush().is_err() {
            break;
        }
        if let Some(guard) = queue.prepare_read() {
            // Non-blocking: nothing to read is not an error.
            let _ = guard.read();
        }
        if queue.dispatch_pending(&mut state).is_err() {
            break;
        }
        if let Some(answer) = state.answer.take()
            && !send(Event::Answer(answer))
        {
            break;
        }
        if std::mem::take(&mut state.changed) && !send(Event::Screens(state.screens())) {
            break;
        }
        match incoming.recv_timeout(POLL) {
            Ok(request) => configure(&manager, &state, &qh, &request),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    manager.stop();
    let _ = conn.flush();
    send(Event::Unavailable);
}

/// Sends `request` as a configuration on the last serial; a screen it does not name stays
/// as it is.
fn configure(
    manager: &ZwlrOutputManagerV1,
    state: &State,
    qh: &QueueHandle<State>,
    request: &Request,
) {
    let Some(serial) = state.serial else {
        return;
    };
    let config = manager.create_configuration(serial, qh, ());
    for head in &state.heads {
        let Some(proxy) = head.proxy.as_ref() else {
            continue;
        };
        let wanted = request.0.iter().find(|(name, _)| *name == head.name);
        let lit = match wanted {
            Some((_, lit)) => *lit,
            None => state
                .screens()
                .into_iter()
                .find(|s| s.name == head.name)
                .and_then(|s| s.on),
        };
        let Some(lit) = lit else {
            config.disable_head(proxy);
            continue;
        };
        let c = config.enable_head(proxy, qh, ());
        let mode = head.modes.iter().find(|(_, m, _)| *m == lit.mode);
        match mode {
            Some((proxy, _, _)) => c.set_mode(proxy),
            None => c.set_custom_mode(lit.mode.width, lit.mode.height, lit.mode.refresh),
        }
        c.set_position(lit.position.0, lit.position.1);
        if let Ok(transform) = Transform::try_from(lit.transform) {
            c.set_transform(transform);
        }
        c.set_scale(lit.scale);
    }
    config.apply();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turned_screen_swaps_its_sides_in_the_space() {
        let lit = Lit {
            mode: Mode {
                width: 1920,
                height: 1080,
                refresh: 60_000,
            },
            position: (0, 0),
            transform: 1,
            scale: 1.5,
        };
        assert_eq!(lit.logical_size(), (720.0, 1280.0));
        assert_eq!(
            Lit {
                transform: 2,
                ..lit
            }
            .logical_size(),
            (1280.0, 720.0)
        );
    }

    #[test]
    fn names_are_bounded() {
        assert_eq!(bounded("a\u{1}b".into()), "ab");
        assert_eq!(bounded("x".repeat(500)).len(), MAX_NAME);
    }
}
