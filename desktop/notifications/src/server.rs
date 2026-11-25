//! The `org.freedesktop.Notifications` server, on the session bus (Desktop Notifications
//! Specification 1.2), with zbus.

use std::collections::HashMap;
use std::sync::Arc;

use zbus::blocking::{Connection, connection};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

use crate::core::Core;
use crate::model::{Reason, Request};

/// The well-known name, the object path and the interface of the specification.
pub const NAME: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/freedesktop/Notifications";
pub const INTERFACE: &str = "org.freedesktop.Notifications";

/// What the server can do: actions as buttons, a body with bold and italic, icons that do
/// not move.
const CAPABILITIES: [&str; 4] = ["actions", "body", "body-markup", "icon-static"];

struct Server {
    core: Arc<Core>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        self.core.notify(Request {
            app_name,
            replaces_id,
            app_icon,
            summary,
            body,
            actions,
            hints,
            expire_timeout,
        })
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        if self.core.close(id) {
            Self::notification_closed(&emitter, id, Reason::Closed as u32).await?;
        }
        Ok(())
    }

    async fn get_capabilities(&self) -> Vec<String> {
        CAPABILITIES.map(String::from).to_vec()
    }

    /// Name, vendor, version, version of the specification. The OS has no name yet.
    #[zbus(out_args("name", "vendor", "version", "spec_version"))]
    async fn get_server_information(&self) -> (String, String, String, String) {
        (
            String::from(env!("CARGO_PKG_NAME")),
            String::from("NOM_OS"),
            String::from(env!("CARGO_PKG_VERSION")),
            String::from("1.2"),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

/// Serves `core` on the bus `builder` connects to. `Ok(None)` when another daemon already owns
/// the name (Plasma, another notification daemon): this one must not run.
pub fn start(
    builder: connection::Builder<'_>,
    core: &Arc<Core>,
) -> zbus::Result<Option<Connection>> {
    let server = Server {
        core: Arc::clone(core),
    };
    let bus = builder.serve_at(PATH, server)?.build()?;
    match bus.request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into()) {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
        Ok(_) | Err(zbus::Error::NameTaken) => return Ok(None),
        Err(error) => return Err(error),
    }
    core.attach(bus.clone())
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    Ok(Some(bus))
}
