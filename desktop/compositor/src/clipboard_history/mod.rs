//! The clipboard history (lot D-42): what is copied, in Wayland and X11 applications, kept for
//! the Super + V panel. Every new CLIPBOARD selection passes through `State::selection_set`
//! (lot D-29); once it is in place, the compositor reads the one type worth keeping as any
//! client would, through a pipe, and a thread reads, bounds and sorts it (`read.rs`). The
//! history (`model.rs`) lives in memory; `clipboard.conf` turns it off.
//!
//! Never kept: anything while the session is locked, a secret a password manager copies
//! (`x-kde-passwordManagerHint`, the nspasteboard types, or the application itself), the
//! primary selection (every text selected), what the history itself puts back.

mod disk;
mod model;
mod read;

use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicUsize},
    time::{Duration, SystemTime},
};

use desktop_config::clipboard::Config;
use smithay::{
    reexports::calloop::{
        LoopHandle,
        channel::{self, Sender},
        timer::{TimeoutAction, Timer},
    },
    wayland::selection::{
        SelectionHandler, SelectionTarget,
        data_device::{
            current_data_device_selection_userdata, request_data_device_client_selection,
        },
    },
};

pub use model::{Changes, Content, Entry, Held, History, Kind, MAX_ENTRIES};
pub use read::{MAX_IMAGE, MAX_TEXT, THUMBNAIL, is_password_manager};

use crate::{State, handlers::SelectionOrigin};

/// How often the setting file is looked at.
const SETTINGS_CHECK: Duration = Duration::from_secs(1);

/// What the threads send back to the loop.
enum Back {
    /// A copy read and sorted.
    Captured {
        generation: u64,
        app: String,
        content: Option<Content>,
    },
    /// The history read from disk.
    Restored(Vec<disk::Restored>),
}

/// What the compositor keeps for the history.
#[derive(Default)]
pub(crate) struct ClipboardHistory {
    pub(crate) history: History,
    config: Config,
    /// The setting file, and its stamp when last read.
    file: Option<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
    /// Where the reading threads send what they read, once the loop listens.
    sender: Option<Sender<Back>>,
    /// Where the history goes when kept on disk, whether it was read from there, and the
    /// thread that writes it.
    disk: Option<PathBuf>,
    restored: bool,
    writer: Option<std::sync::mpsc::Sender<Option<Vec<Entry>>>>,
    /// The last copy asked for, and the last one kept: a slow read that ends after a newer one
    /// is dropped.
    generation: u64,
    kept: u64,
    reads: Arc<AtomicUsize>,
    /// The next selection is the history's own: an item put back.
    putting_back: bool,
    /// Whether the history is on, as the panels were last told.
    told_enabled: Option<bool>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl State {
    /// The clipboard history.
    pub fn clipboard_history(&self) -> &History {
        &self.clipboard.history
    }

    /// The history setting in use.
    pub fn clipboard_settings(&self) -> Config {
        self.clipboard.config
    }

    /// Applies `config`: the history turned off forgets everything at once.
    pub fn set_clipboard_settings(&mut self, config: Config) {
        if !config.history && self.clipboard.config.history {
            self.clipboard.history.forget();
            tracing::info!("clipboard history off: forgotten");
        }
        let kept = config.history && config.keep_on_disk;
        self.clipboard.config = config;
        match kept {
            true if !self.clipboard.restored => self.restore_clipboard(),
            true => self.save_clipboard(),
            // The file goes: the history lives in memory only.
            false => self.write_clipboard(None),
        }
        self.clipboard_changed();
    }

    /// Keeps the history on disk at `path` when the setting says so (the tests, or
    /// `watch_clipboard_settings`).
    pub fn set_clipboard_disk(&mut self, path: Option<PathBuf>) {
        self.clipboard.disk = path;
    }

    /// Reads the history kept on disk, in a thread.
    fn restore_clipboard(&mut self) {
        self.clipboard.restored = true;
        let (Some(path), Some(sender)) = (self.clipboard.disk.clone(), self.clipboard_sender())
        else {
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("clipboard-restore".into())
            .spawn(move || {
                let _ = sender.send(Back::Restored(disk::read(&path)));
            });
        if let Err(err) = spawned {
            tracing::warn!("clipboard history not read: {err}");
        }
    }

    /// Writes the history to disk, if it is kept there.
    pub(crate) fn save_clipboard(&mut self) {
        let config = self.clipboard.config;
        if config.history && config.keep_on_disk && self.clipboard.restored {
            let entries = self.clipboard.history.entries().to_vec();
            self.write_clipboard(Some(entries));
        }
    }

    /// Hands the writing thread the items to write, or `None` to remove the file. It writes
    /// the last it was given only.
    fn write_clipboard(&mut self, entries: Option<Vec<Entry>>) {
        let Some(path) = self.clipboard.disk.clone() else {
            return;
        };
        if self.clipboard.writer.is_none() {
            let (sender, receiver) = std::sync::mpsc::channel::<Option<Vec<Entry>>>();
            let spawned = std::thread::Builder::new()
                .name("clipboard-write".into())
                .spawn(move || {
                    while let Ok(mut latest) = receiver.recv() {
                        while let Ok(newer) = receiver.try_recv() {
                            latest = newer;
                        }
                        let done = match latest {
                            Some(entries) => disk::write(&path, &disk::encode(&entries)),
                            None => match std::fs::remove_file(&path) {
                                Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
                                _ => Ok(()),
                            },
                        };
                        if let Err(err) = done {
                            tracing::warn!("clipboard history on disk: {err}");
                        }
                    }
                });
            match spawned {
                Ok(_) => self.clipboard.writer = Some(sender),
                Err(err) => {
                    tracing::warn!("clipboard history not written: {err}");
                    return;
                }
            }
        }
        if let Some(writer) = &self.clipboard.writer {
            let _ = writer.send(entries);
        }
    }

    /// After each change of the history or of its setting: the disk and the panels follow.
    pub(crate) fn clipboard_changed(&mut self) {
        let changes = self.clipboard.history.take_changes();
        if !changes.updated.is_empty() || !changes.removed.is_empty() {
            self.save_clipboard();
        }
        self.tell_clipboard_clients(changes);
    }

    /// The history's state, if the panels have not been told it yet.
    pub(crate) fn clipboard_state_changed(&mut self) -> Option<bool> {
        let enabled = self.clipboard.config.history;
        let told = self.clipboard.told_enabled.replace(enabled);
        (told.is_some() && told != Some(enabled)).then_some(enabled)
    }

    /// Pins item `id`, or unpins it.
    pub fn pin_clipboard_item(&mut self, id: u32, pinned: bool) {
        self.clipboard.history.set_pinned(id, pinned);
        self.clipboard_changed();
    }

    /// Forgets item `id`.
    pub fn remove_clipboard_item(&mut self, id: u32) {
        self.clipboard.history.remove(id);
        self.clipboard_changed();
    }

    /// « Tout effacer »: every item but the pinned ones.
    pub fn clear_clipboard_history(&mut self) {
        self.clipboard.history.clear();
        self.clipboard_changed();
    }

    /// Reads the history setting of `file`, then again when it changes.
    pub fn watch_clipboard_settings(
        &mut self,
        handle: &LoopHandle<'static, State>,
        file: Option<PathBuf>,
    ) -> Result<(), crate::Error> {
        self.clipboard.file = file;
        if self.clipboard.disk.is_none() {
            let var = |key: &str| std::env::var_os(key);
            self.clipboard.disk = desktop_config::clipboard::history_file(var);
        }
        self.reload_clipboard_settings();
        handle
            .insert_source(Timer::from_duration(SETTINGS_CHECK), |_, _, state| {
                state.reload_clipboard_settings();
                TimeoutAction::ToDuration(SETTINGS_CHECK)
            })
            .map_err(|err| err.error)?;
        Ok(())
    }

    fn reload_clipboard_settings(&mut self) {
        let Some(file) = self.clipboard.file.clone() else {
            return;
        };
        let stamp = std::fs::metadata(&file)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        if stamp == self.clipboard.stamp {
            return;
        }
        self.clipboard.stamp = stamp;
        let config = Config::read(&file).unwrap_or_else(|err| {
            tracing::warn!("clipboard: {}: {err}", file.display());
            Config::default()
        });
        tracing::info!(?config, "clipboard history setting");
        self.set_clipboard_settings(config);
    }

    /// A new selection (`State::selection_set`): read once in place, if it is to be kept.
    pub(crate) fn clipboard_saw(
        &mut self,
        target: SelectionTarget,
        origin: &SelectionOrigin,
        mime_types: &[String],
    ) {
        if target != SelectionTarget::Clipboard {
            return;
        }
        if std::mem::take(&mut self.clipboard.putting_back) {
            return;
        }
        let app = match origin {
            SelectionOrigin::Compositor => String::new(),
            _ => self
                .focused_window()
                .map(|w| crate::shell::app_id(&w))
                .unwrap_or_default(),
        };
        let Some(mime) = read::choose(mime_types) else {
            return;
        };
        if !self.clipboard.config.history || self.is_locked() {
            return;
        }
        if crate::handlers::is_secret(mime_types) || is_password_manager(&app) {
            tracing::debug!(app, "clipboard history: a secret, not kept");
            return;
        }
        self.clipboard.generation += 1;
        let generation = self.clipboard.generation;
        // The selection is in place on the Wayland side after this call returns.
        self.loop_handle.insert_idle(move |state| {
            state.read_for_history(generation, app, mime);
        });
    }

    fn read_for_history(&mut self, generation: u64, app: String, mime: &'static str) {
        if self.is_locked() || generation != self.clipboard.generation {
            return;
        }
        let Some(sender) = self.clipboard_sender() else {
            return;
        };
        let (reader, writer) = match std::io::pipe() {
            Ok(pipe) => pipe,
            Err(err) => {
                tracing::warn!("clipboard history: no pipe: {err}");
                return;
            }
        };
        let fd = std::os::fd::OwnedFd::from(writer);
        // As a Wayland client would read it: the owner client, X11 or the compositor's own.
        let held = current_data_device_selection_userdata(&self.seat).map(|d| d.clone());
        match held {
            None => {
                if let Err(err) = request_data_device_client_selection(&self.seat, mime.into(), fd)
                {
                    tracing::debug!("clipboard history: selection gone: {err}");
                    return;
                }
            }
            Some(data) => {
                let seat = self.seat.clone();
                SelectionHandler::send_selection(
                    self,
                    SelectionTarget::Clipboard,
                    mime.into(),
                    fd,
                    seat,
                    &data,
                );
            }
        }
        read::spawn(reader, mime, self.clipboard.reads.clone(), move |content| {
            // The loop may have ended: nothing to keep then.
            let _ = sender.send(Back::Captured {
                generation,
                app,
                content,
            });
        });
    }

    /// The sender of the reading threads, its receiver put in the loop the first time.
    fn clipboard_sender(&mut self) -> Option<Sender<Back>> {
        if let Some(sender) = &self.clipboard.sender {
            return Some(sender.clone());
        }
        let (sender, receiver) = channel::channel();
        let inserted = self
            .loop_handle
            .insert_source(receiver, |event, _, state| match event {
                channel::Event::Msg(Back::Captured {
                    generation,
                    app,
                    content,
                }) => state.keep_captured(generation, app, content),
                channel::Event::Msg(Back::Restored(items)) => state.keep_restored(items),
                channel::Event::Closed => {}
            });
        if let Err(err) = inserted {
            tracing::warn!("clipboard history: {}", err.error);
            return None;
        }
        self.clipboard.sender = Some(sender.clone());
        Some(sender)
    }

    fn keep_restored(&mut self, items: Vec<disk::Restored>) {
        if !self.clipboard.config.history {
            return;
        }
        tracing::info!(items = items.len(), "clipboard history read from disk");
        for (content, app, time, pinned) in items {
            self.clipboard.history.restore(content, app, time, pinned);
        }
        self.clipboard_changed();
    }

    fn keep_captured(&mut self, generation: u64, app: String, content: Option<Content>) {
        let locked = self.is_locked();
        let clipboard = &mut self.clipboard;
        if generation <= clipboard.kept || !clipboard.config.history || locked {
            return;
        }
        let Some(content) = content else {
            return;
        };
        clipboard.kept = generation;
        let id = clipboard.history.push(content, app, now());
        tracing::debug!(?id, "clipboard history: kept");
        self.clipboard_changed();
    }

    /// Puts item `id` back on the clipboard, and moves it to the top. Only the shell's panel
    /// asks for it (lot D-42, the shell protocol).
    pub fn put_back(&mut self, id: u32) {
        if self.is_locked() {
            return;
        }
        let Some(held) = self
            .clipboard
            .history
            .get(id)
            .map(|e| e.content.held.clone())
        else {
            return;
        };
        self.clipboard.history.promote(id, now());
        self.clipboard.putting_back = true;
        self.hold_selection(held);
        // Taken by `clipboard_saw`, which every clipboard selection passes through.
        self.clipboard.putting_back = false;
        self.clipboard_changed();
    }
}
