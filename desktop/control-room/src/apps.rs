//! The default applications, through `mime-apps`: what opens each kind of file or link, read
//! from the associations and written to `$XDG_CONFIG_HOME/mimeapps.list`; the terminal, to
//! `xdg-terminals.list`.

use std::io;

use mime_apps::{Catalog, Dirs, terminals};
use sherlock_ui::control_room::texts as t;

/// A kind of file or link the page offers a default for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Browser,
    Mail,
    Editor,
    Terminal,
    Files,
    Images,
    Video,
    Music,
}

impl Kind {
    pub const ALL: [Self; 8] = [
        Self::Browser,
        Self::Mail,
        Self::Editor,
        Self::Terminal,
        Self::Files,
        Self::Images,
        Self::Video,
        Self::Music,
    ];

    /// The types it covers, the first one choosing the applications offered. The terminal has
    /// none: its own list.
    pub fn types(self) -> &'static [&'static str] {
        match self {
            Self::Browser => &[
                "x-scheme-handler/https",
                "x-scheme-handler/http",
                "text/html",
            ],
            Self::Mail => &["x-scheme-handler/mailto"],
            Self::Editor => &["text/plain"],
            Self::Terminal => &[],
            Self::Files => &["inode/directory"],
            Self::Images => &["image/png", "image/jpeg", "image/gif", "image/webp"],
            Self::Video => &["video/mp4", "video/x-matroska", "video/webm"],
            Self::Music => &["audio/mpeg", "audio/flac", "audio/ogg"],
        }
    }

    /// Its name and the sentence under it.
    pub fn texts(self) -> (&'static str, &'static str) {
        match self {
            Self::Browser => (t::BROWSER, t::BROWSER_HELP),
            Self::Mail => (t::MAIL, t::MAIL_HELP),
            Self::Editor => (t::EDITOR, t::EDITOR_HELP),
            Self::Terminal => (t::TERMINAL, t::TERMINAL_HELP),
            Self::Files => (t::FILES, t::FILES_HELP),
            Self::Images => (t::IMAGES, t::IMAGES_HELP),
            Self::Video => (t::VIDEO, t::VIDEO_HELP),
            Self::Music => (t::MUSIC, t::MUSIC_HELP),
        }
    }
}

/// An application in a menu: its desktop id, shown by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub name: String,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// The applications offered for `kind`, and the default one.
pub fn choices(catalog: &Catalog, kind: Kind) -> (Vec<Choice>, Option<Choice>) {
    let choice = |app: &mime_apps::DesktopEntry| Choice {
        id: app.id.clone(),
        name: app.name.clone(),
    };
    match kind.types().first() {
        None => (
            terminals::installed(catalog)
                .into_iter()
                .map(choice)
                .collect(),
            terminals::default(catalog).map(choice),
        ),
        Some(first) => {
            let handlers = catalog.handlers(first);
            let default = handlers.first().map(|app| choice(app));
            let mut offered: Vec<Choice> = handlers.into_iter().map(choice).collect();
            offered.sort_by_key(|c| c.name.to_lowercase());
            (offered, default)
        }
    }
}

/// Writes of `mimeapps.list` and `xdg-terminals.list`, one at a time: two choices made in a
/// row each read, change and write the file, and neither loses the other.
static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Makes `id` the default for every type of `kind`.
pub fn set(dirs: &Dirs, desktops: &[String], kind: Kind, id: &str) -> io::Result<()> {
    let _one_at_a_time = WRITING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if kind == Kind::Terminal {
        return terminals::set_default(dirs, id);
    }
    for mime in kind.types() {
        mime_apps::set_default(dirs, desktops, mime, id)?;
    }
    Ok(())
}
