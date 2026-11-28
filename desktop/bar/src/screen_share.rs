//! The screen share indicator (lot D-28d): while an application shares a screen or a window
//! through our portal, the bar shows a pill « Partage d'écran en cours »; a click on it stops
//! every share, as « Arrêter le partage » does on macOS. The portal announces the applications
//! sharing on the session bus (`org.sherlockos.ScreenShare1`, `Changed`); the bar never starts
//! it, and forgets the shares when it leaves the bus.

use iced::futures::StreamExt;
use iced::futures::stream::{self, BoxStream};
use zbus::Connection;

/// The portal's name, path and interface (`desktop/portal`).
pub const NAME: &str = "org.freedesktop.impl.portal.desktop.sherlockos";
pub const PATH: &str = "/org/sherlockos/ScreenShare";
pub const INTERFACE: &str = "org.sherlockos.ScreenShare1";
/// The longest list of applications kept from a signal.
const MAX_APPS: usize = 16;

/// The applications sharing at each change: from `Changed`, or none once the portal leaves.
async fn changes(bus: &Connection) -> zbus::Result<BoxStream<'static, Vec<String>>> {
    let changed = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(NAME)?
        .interface(INTERFACE)?
        .member("Changed")?
        .path(PATH)?
        .build();
    let left = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, NAME)?
        .build();
    let changed = zbus::MessageStream::for_match_rule(changed, bus, None).await?;
    let left = zbus::MessageStream::for_match_rule(left, bus, None).await?;
    let changed = changed.filter_map(|m| async move {
        let apps: Vec<String> = m.ok()?.body().deserialize::<(Vec<String>,)>().ok()?.0;
        Some(apps.into_iter().take(MAX_APPS).collect())
    });
    let left = left.filter_map(|m| async move {
        let (_, _, owner): (String, String, String) = m.ok()?.body().deserialize().ok()?;
        owner.is_empty().then(Vec::new)
    });
    Ok(stream::select(changed, left).boxed())
}

/// The applications sharing, for an iced subscription: nothing when the bus cannot be reached.
pub fn watch() -> BoxStream<'static, Vec<String>> {
    stream::once(async {
        let watched = match crate::bus::session().await {
            Ok(bus) => changes(&bus).await,
            Err(error) => Err(error),
        };
        watched.unwrap_or_else(|error| {
            tracing::warn!(%error, "the bar will not show the screen shares");
            stream::empty().boxed()
        })
    })
    .flatten()
    .boxed()
}

/// « Arrêter le partage »: every share stops. Never starts the portal.
pub async fn stop_all() -> Result<(), String> {
    let bus = crate::bus::session().await.map_err(|e| e.to_string())?;
    let call = zbus::message::Message::method_call(PATH, "StopAll")
        .and_then(|b| b.destination(NAME))
        .and_then(|b| b.interface(INTERFACE))
        .and_then(|b| b.with_flags(zbus::message::Flags::NoAutoStart))
        .and_then(|b| b.build(&()))
        .map_err(|e| e.to_string())?;
    // Not waited for: the portal tells the new state with `Changed`.
    bus.send(&call).await.map_err(|e| e.to_string())
}
