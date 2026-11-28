//! `org.freedesktop.impl.portal.ScreenCast`, the backend that `xdg-desktop-portal` calls when an
//! application (Discord, Chrome, Firefox) asks to share the screen. It asks the user what to
//! share in our picker, then shares it into PipeWire; the frontend hands the PipeWire node to
//! the application.
//!
//! A session lives at the path the frontend gives, with `org.freedesktop.impl.portal.Session`:
//! closing it, or the application leaving, stops its shares.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use zbus::{
    Connection,
    object_server::SignalEmitter,
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value},
};

use crate::{
    capture::{self, Share, Source},
    picker,
};

/// `AvailableSourceTypes`: a screen (`MONITOR`) or a window (`WINDOW`).
pub const MONITOR: u32 = 1;
pub const WINDOW: u32 = 2;
/// `AvailableCursorModes`: hidden or drawn into the frames, never as metadata.
const CURSOR_HIDDEN: u32 = 1;
const CURSOR_EMBEDDED: u32 = 2;
/// Answers of a request.
const SUCCESS: u32 = 0;
const CANCELLED: u32 = 1;
const FAILED: u32 = 2;

type Options = HashMap<String, OwnedValue>;

/// What the application asked for in `SelectSources`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub types: u32,
    pub multiple: bool,
    pub cursor: bool,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            types: MONITOR,
            multiple: false,
            cursor: true,
        }
    }
}

impl Selection {
    /// Reads the options of `SelectSources`, keeping what we can do.
    pub fn from_options(options: &Options) -> Self {
        let number = |key: &str| options.get(key).and_then(|v| u32::try_from(v).ok());
        let types = number("types").map_or(MONITOR, |t| t & (MONITOR | WINDOW));
        Self {
            types: if types == 0 { MONITOR } else { types },
            multiple: options
                .get("multiple")
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(false),
            cursor: number("cursor_mode").is_none_or(|m| m != CURSOR_HIDDEN),
        }
    }
}

/// A session: what it asked for, the application, and its shares once started.
#[derive(Default)]
pub struct Session {
    pub app_id: String,
    pub selection: Selection,
    pub shares: Vec<(Source, Share)>,
}

/// The sessions open, by path: shared with the indicator.
pub type Sessions = Arc<Mutex<HashMap<OwnedObjectPath, Session>>>;

pub struct ScreenCast {
    pub sessions: Sessions,
}

/// Where the bar follows the shares and stops them (`org.sherlockos.ScreenShare1`).
pub const SHARES_PATH: &str = "/org/sherlockos/ScreenShare";
pub const SHARES_INTERFACE: &str = "org.sherlockos.ScreenShare1";

/// The applications sharing now, one per session that shares.
fn sharing(sessions: &Sessions) -> Vec<String> {
    lock(sessions)
        .values()
        .filter(|s| !s.shares.is_empty())
        .map(|s| s.app_id.clone())
        .collect()
}

/// Tells the bar what is shared now.
async fn announce(bus: &Connection, sessions: &Sessions) {
    let apps = sharing(sessions);
    let emitted = (bus.emit_signal(
        None::<()>,
        SHARES_PATH,
        SHARES_INTERFACE,
        "Changed",
        &(apps,),
    ))
    .await;
    if let Err(err) = emitted {
        tracing::warn!("screen share indicator: {err}");
    }
}

/// Ends the session at `path`: its shares stop, the frontend is told (`Closed`), and its
/// object goes.
async fn end(bus: &Connection, sessions: &Sessions, path: &OwnedObjectPath) {
    let session = lock(sessions).remove(path);
    // The shares stop as they drop: their threads join, off the bus's thread.
    blocking::unblock(move || drop(session)).await;
    let server = bus.object_server();
    if let Ok(object) = server.interface::<_, SessionObject>(path).await {
        let _ = SessionObject::closed(object.signal_emitter()).await;
    }
    let _ = server.remove::<SessionObject, _>(path).await;
    announce(bus, sessions).await;
}

/// Ends the sessions whose shares all stopped: their screen or window went away, or PipeWire
/// stopped reading. The frontend tells the application; the bar's indicator goes.
pub async fn reap(bus: &Connection, sessions: &Sessions) {
    let over: Vec<OwnedObjectPath> = lock(sessions)
        .iter()
        .filter(|(_, s)| !s.shares.is_empty() && s.shares.iter().all(|(_, sh)| !sh.running()))
        .map(|(path, _)| path.clone())
        .collect();
    for path in over {
        tracing::info!(%path, "screen share over");
        end(bus, sessions, &path).await;
    }
}

/// `org.sherlockos.ScreenShare1`: the bar's indicator and its « Arrêter le partage ».
pub struct Shares {
    pub sessions: Sessions,
}

#[zbus::interface(name = "org.sherlockos.ScreenShare1")]
impl Shares {
    /// The applications sharing now.
    async fn sharing(&self) -> Vec<String> {
        sharing(&self.sessions)
    }

    /// Stops every share: the applications are told their session closed.
    async fn stop_all(&self, #[zbus(connection)] bus: &Connection) {
        let paths: Vec<OwnedObjectPath> = lock(&self.sessions).keys().cloned().collect();
        for path in paths {
            end(bus, &self.sessions, &path).await;
        }
    }
}

fn lock(sessions: &Sessions) -> std::sync::MutexGuard<'_, HashMap<OwnedObjectPath, Session>> {
    sessions.lock().unwrap_or_else(PoisonError::into_inner)
}

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    async fn create_session(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: Options,
        #[zbus(connection)] bus: &Connection,
    ) -> (u32, Options) {
        let path = OwnedObjectPath::from(session_handle);
        let object = SessionObject {
            path: path.clone(),
            sessions: Arc::clone(&self.sessions),
        };
        if let Err(err) = bus.object_server().at(&path, object).await {
            tracing::warn!("session {path}: {err}");
            return (FAILED, Options::new());
        }
        let session = Session {
            app_id,
            ..Session::default()
        };
        lock(&self.sessions).insert(path, session);
        (SUCCESS, Options::new())
    }

    async fn select_sources(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        options: Options,
    ) -> (u32, Options) {
        let path = OwnedObjectPath::from(session_handle);
        match lock(&self.sessions).get_mut(&path) {
            Some(session) => {
                session.selection = Selection::from_options(&options);
                (SUCCESS, Options::new())
            }
            None => (FAILED, Options::new()),
        }
    }

    async fn start(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _parent_window: String,
        _options: Options,
        #[zbus(connection)] bus: &Connection,
    ) -> (u32, Options) {
        let path = OwnedObjectPath::from(session_handle);
        let Some((app_id, selection)) = lock(&self.sessions)
            .get(&path)
            .map(|s| (s.app_id.clone(), s.selection.clone()))
        else {
            return (FAILED, Options::new());
        };
        // The picker waits for the user, the shares for PipeWire: off the bus's thread.
        let started = blocking::unblock(move || start(&app_id, &selection)).await;
        let shares = match started {
            Ok(Some(shares)) => shares,
            Ok(None) => return (CANCELLED, Options::new()),
            Err(err) => {
                tracing::warn!("screen share: {err}");
                return (FAILED, Options::new());
            }
        };
        let streams: Vec<(u32, HashMap<&str, Value<'_>>)> = shares
            .iter()
            .map(|(source, share)| (share.node, stream_properties(source, share.size)))
            .collect();
        let results = HashMap::from([("streams", Value::from(streams))]);
        let results: Options = results
            .into_iter()
            .filter_map(|(k, v)| Some((k.to_owned(), OwnedValue::try_from(v).ok()?)))
            .collect();
        match lock(&self.sessions).get_mut(&path) {
            Some(session) => session.shares = shares,
            // Closed while the user chose: the shares stop.
            None => return (CANCELLED, Options::new()),
        }
        announce(bus, &self.sessions).await;
        (SUCCESS, results)
    }

    #[zbus(property, name = "AvailableSourceTypes")]
    fn available_source_types(&self) -> u32 {
        MONITOR | WINDOW
    }

    #[zbus(property, name = "AvailableCursorModes")]
    fn available_cursor_modes(&self) -> u32 {
        CURSOR_HIDDEN | CURSOR_EMBEDDED
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        5
    }
}

/// What the frontend tells the application of a stream.
fn stream_properties(source: &Source, (w, h): (u32, u32)) -> HashMap<&'static str, Value<'static>> {
    let kind = match source {
        Source::Screen(_) => MONITOR,
        Source::Window(_) => WINDOW,
    };
    HashMap::from([
        ("source_type", Value::from(kind)),
        ("size", Value::from((w as i32, h as i32))),
    ])
}

/// Asks the user, then starts a share of each source chosen. `None`: cancelled.
fn start(
    app_id: &str,
    selection: &Selection,
) -> Result<Option<Vec<(Source, Share)>>, capture::Error> {
    let sources = capture::sources()?;
    let Some(chosen) = picker::pick(app_id, selection, &sources) else {
        return Ok(None);
    };
    let mut shares = Vec::new();
    for source in chosen {
        let share = capture::share(source.clone(), selection.cursor)?;
        shares.push((source, share));
    }
    Ok(Some(shares))
}

/// `org.freedesktop.impl.portal.Session` of a session.
struct SessionObject {
    path: OwnedObjectPath,
    sessions: Sessions,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl SessionObject {
    async fn close(&self, #[zbus(connection)] bus: &Connection) {
        end(bus, &self.sessions, &self.path).await;
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(pairs: &[(&str, Value<'_>)]) -> Options {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OwnedValue::try_from(v.clone()).unwrap()))
            .collect()
    }

    #[test]
    fn the_options_keep_what_can_be_shared() {
        assert_eq!(
            Selection::from_options(&Options::new()),
            Selection::default()
        );
        let asked = options(&[
            ("types", Value::from(MONITOR | WINDOW | 4)),
            ("multiple", Value::from(true)),
            ("cursor_mode", Value::from(CURSOR_HIDDEN)),
        ]);
        let selection = Selection::from_options(&asked);
        assert_eq!(
            selection,
            Selection {
                types: MONITOR | WINDOW,
                multiple: true,
                cursor: false,
            }
        );
        // Only a virtual screen, which we do not have: a real one instead.
        let virtual_only = options(&[("types", Value::from(4u32))]);
        assert_eq!(Selection::from_options(&virtual_only).types, MONITOR);
        // Metadata is not offered: drawn in.
        let metadata = options(&[("cursor_mode", Value::from(4u32))]);
        assert!(Selection::from_options(&metadata).cursor);
    }

    #[test]
    fn a_stream_says_its_kind_and_size() {
        let screen = Source::Screen(capture::Screen {
            name: String::from("eDP-1"),
            description: String::new(),
        });
        let properties = stream_properties(&screen, (1920, 1080));
        assert_eq!(properties["source_type"], Value::from(MONITOR));
        assert_eq!(properties["size"], Value::from((1920, 1080)));
    }
}
