//! The « Réseau » section of the sidebar (lot E-07): a click on a server mounts it through
//! gvfs (`smb`) off the interface thread and opens its folder; its eject button unmounts it.

use std::path::PathBuf;

use iced::Task;
use sherlock_ui::texts::evidence as texts;

use super::{App, Location, Message, Notice};
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
        Task::perform(async move { smb::mount(&uri, None) }, move |result| {
            Message::ServerMounted(host.clone(), result)
        })
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
