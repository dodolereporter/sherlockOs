//! The « Réseau » section of the sidebar (lot E-07): a click on a server mounts it through
//! gvfs (`smb`) off the interface thread and opens its folder; its eject button unmounts it.

use std::path::PathBuf;

use iced::Task;
use sherlock_ui::texts::evidence as texts;

use super::{App, Location, Message, Notice};
use sherlock_ui::motion::Fade;

/// The sheet that asks a server's name and password (lot E-07b).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Login {
    pub host: String,
    pub server: String,
    pub user: String,
    pub domain: String,
    pub password: String,
    /// Keep them in the keyring.
    pub remember: bool,
}

/// A field of the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    User,
    Domain,
    Password,
}
use crate::smb;

impl App {
    /// The server of this host, among those found.
    fn server(&self, host: &str) -> Option<&crate::network::Server> {
        self.servers.iter().find(|s| s.host == host)
    }

    /// Opens a server: its folder when it is connected, else it connects first, as a guest.
    pub(super) fn open_server(&mut self, host: &str) -> Task<Message> {
        if let Some(folder) = self.mounted.get(host).cloned() {
            return self.go(Location::Folder(folder));
        }
        let Some(server) = self.server(host).cloned() else {
            return Task::none();
        };
        self.busy += 1;
        self.notice = Some(Notice {
            text: texts::connecting(&server.name),
            error: false,
        });
        let uri = server.uri();
        let host = host.to_owned();
        let lookup = host.clone();
        // The name and the password kept in the keyring, if any; else as a guest.
        Task::perform(
            async move {
                let kept = crate::keyring::find(&lookup);
                smb::mount(&uri, kept.as_ref())
            },
            move |result| Message::ServerMounted(host.clone(), result),
        )
    }

    /// The connected server whose folder holds the location shown.
    pub(super) fn server_of_location(&self) -> Option<String> {
        let Location::Folder(dir) = &self.location else {
            return None;
        };
        self.mounted
            .iter()
            .find(|(_, folder)| dir.starts_with(folder))
            .map(|(host, _)| host.clone())
    }

    /// The sheet for `host`, its name prefilled from the last try.
    pub(super) fn ask_login(&mut self, host: String) {
        if self.login.as_ref().is_some_and(|l| l.host == host) {
            return;
        }
        let server = self
            .server(&host)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let user = self.login_user.clone();
        self.login = Some(Login {
            host,
            server,
            user,
            remember: true,
            ..Login::default()
        });
        self.login_fade = Some(Fade::new(0.0, 1.0, self.now(), sherlock_ui::motion::FADE));
    }

    pub(super) fn login_input(&mut self, field: Field, value: String) {
        if let Some(login) = &mut self.login {
            match field {
                Field::User => login.user = value,
                Field::Domain => login.domain = value,
                Field::Password => login.password = value,
            }
        }
    }

    pub(super) fn login_remember(&mut self, on: bool) {
        if let Some(login) = &mut self.login {
            login.remember = on;
        }
    }

    /// Connects with what the sheet holds, and keeps it in the keyring when asked and it worked.
    pub(super) fn login_submit(&mut self) -> Task<Message> {
        let Some(login) = self.login.take() else {
            return Task::none();
        };
        let Some(server) = self.server(&login.host).cloned() else {
            return Task::none();
        };
        self.busy += 1;
        self.notice = Some(Notice {
            text: texts::connecting(&server.name),
            error: false,
        });
        let credentials = smb::Credentials {
            user: login.user.clone(),
            domain: login.domain.clone(),
            password: login.password,
        };
        let (uri, host, remember) = (server.uri(), login.host.clone(), login.remember);
        // Connected already, as a guest or as someone else: gvfs would keep that connection
        // (« already mounted »), so it goes first (lot Q-56).
        let connected = self.mounted.remove(&login.host).is_some();
        let target = login.host;
        // The name is kept to prefill the sheet if it has to ask again.
        self.login_user = login.user;
        Task::perform(
            async move {
                if connected && let Err(error) = smb::unmount(&uri) {
                    tracing::info!(?error, "the guest connection stays");
                }
                let result = smb::mount(&uri, Some(&credentials));
                if result.is_ok() && remember && !crate::keyring::keep(&host, &credentials) {
                    tracing::info!("the password is not kept: no keyring, or a locked one");
                }
                result
            },
            move |result| Message::ServerMounted(target.clone(), result),
        )
    }

    pub(super) fn server_mounted(
        &mut self,
        host: String,
        result: Result<PathBuf, smb::Error>,
    ) -> Task<Message> {
        self.busy = self.busy.saturating_sub(1);
        match result {
            Ok(folder) => {
                self.notice = None;
                self.mounted.insert(host, folder.clone());
                self.go(Location::Folder(folder))
            }
            // The server wants a name and a password: the sheet asks for them.
            Err(smb::Error::Credentials) => {
                self.notice = None;
                self.ask_login(host);
                Task::none()
            }
            Err(error) => {
                tracing::warn!(?error, "server not mounted");
                self.notice = Some(Notice {
                    text: error.to_string(),
                    error: true,
                });
                Task::none()
            }
        }
    }

    /// Unmounts a connected server, leaving its folder first.
    pub(super) fn eject_server(&mut self, host: &str) -> Task<Message> {
        let (Some(server), Some(folder)) = (self.server(host).cloned(), self.mounted.get(host))
        else {
            return Task::none();
        };
        if let Location::Folder(dir) = &self.location
            && dir.starts_with(folder)
        {
            let home = self.config.home.clone();
            let _ = self.go(Location::Folder(home));
        }
        self.busy += 1;
        let uri = server.uri();
        let host = host.to_owned();
        Task::perform(async move { smb::unmount(&uri) }, move |result| {
            Message::ServerUnmounted(host.clone(), result)
        })
    }

    pub(super) fn server_unmounted(&mut self, host: &str, result: Result<(), smb::Error>) {
        self.busy = self.busy.saturating_sub(1);
        let (text, error) = match result {
            Ok(()) => {
                self.mounted.remove(host);
                (texts::DISCONNECTED.to_owned(), false)
            }
            Err(error) => (error.to_string(), true),
        };
        self.notice = Some(Notice { text, error });
    }
}
