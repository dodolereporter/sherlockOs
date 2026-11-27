//! The clipboard history itself, without Wayland: the items, newest first, at most
//! [`MAX_ENTRIES`] and [`MAX_BYTES`] of content, a copy already there moved back to the top,
//! the pinned items kept, and what changed since the shell was last told.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

/// Items kept at most, pinned ones included.
pub const MAX_ENTRIES: usize = 50;

/// Content kept at most, all items together: past it, the oldest unpinned items go.
pub const MAX_BYTES: usize = 128 << 20;

/// What an item is, as the panel shows it. The values are those of the shell protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Text = 0,
    Link = 1,
    Files = 2,
    Image = 3,
}

/// The same content under each type it is offered as, for the compositor to offer it again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Held {
    types: Vec<(String, Arc<[u8]>)>,
}

impl Held {
    /// `bytes` offered under each of `types`.
    pub fn new(types: &[&str], bytes: Arc<[u8]>) -> Self {
        Self {
            types: types
                .iter()
                .map(|t| ((*t).to_owned(), bytes.clone()))
                .collect(),
        }
    }

    /// A screenshot, or any PNG picture.
    pub fn png(bytes: Arc<[u8]>) -> Self {
        Self::new(&["image/png"], bytes)
    }

    /// The same content offered under one more type, with bytes of its own.
    pub fn with(mut self, mime: &str, bytes: Arc<[u8]>) -> Self {
        self.types.push((mime.to_owned(), bytes));
        self
    }

    /// Each type with its bytes.
    pub fn entries(&self) -> &[(String, Arc<[u8]>)] {
        &self.types
    }

    /// The types offered, in order of preference.
    pub fn types(&self) -> Vec<String> {
        self.types.iter().map(|(t, _)| t.clone()).collect()
    }

    /// The bytes offered as `mime`.
    pub fn bytes(&self, mime: &str) -> Option<Arc<[u8]>> {
        self.types
            .iter()
            .find(|(t, _)| t == mime)
            .map(|(_, b)| b.clone())
    }

    /// Bytes held, each buffer counted once.
    fn size(&self) -> usize {
        let mut seen: Vec<*const u8> = Vec::new();
        let mut size = 0;
        for (_, bytes) in &self.types {
            if !seen.contains(&bytes.as_ptr()) {
                seen.push(bytes.as_ptr());
                size += bytes.len();
            }
        }
        size
    }
}

/// What was copied, read and sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
    pub kind: Kind,
    pub held: Held,
    /// A few lines to show, a few hundred characters at most.
    pub preview: String,
    /// A small PNG of a picture.
    pub thumbnail: Option<Arc<[u8]>>,
    digest: u64,
}

impl Content {
    /// `held`, of kind `kind`, told apart from the others by `key`, its main bytes.
    pub fn new(kind: Kind, held: Held, key: &[u8], preview: String) -> Self {
        let mut hasher = DefaultHasher::new();
        kind.hash(&mut hasher);
        key.hash(&mut hasher);
        Self {
            kind,
            held,
            preview,
            thumbnail: None,
            digest: hasher.finish(),
        }
    }

    pub fn with_thumbnail(mut self, png: Arc<[u8]>) -> Self {
        self.thumbnail = Some(png);
        self
    }

    fn same(&self, other: &Self) -> bool {
        self.digest == other.digest && self.kind == other.kind && self.held == other.held
    }

    fn size(&self) -> usize {
        self.held.size() + self.thumbnail.as_ref().map_or(0, |t| t.len()) + self.preview.len()
    }
}

/// One item of the history.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Never given to another item while the compositor runs.
    pub id: u32,
    pub content: Content,
    /// The application it was copied from: its app_id or X11 class, empty when unknown.
    pub app: String,
    /// When it was copied, last: seconds since the Unix epoch.
    pub time: u64,
    pub pinned: bool,
}

/// What changed since the shell was last told.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// New or changed items, by id.
    pub updated: Vec<u32>,
    /// Items gone, by id.
    pub removed: Vec<u32>,
}

/// The items, newest first.
#[derive(Debug)]
pub struct History {
    entries: Vec<Entry>,
    next_id: u32,
    changes: Changes,
}

impl Default for History {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 1,
            changes: Changes::default(),
        }
    }
}

impl History {
    /// The items, newest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: u32) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Adds what was just copied, from `app` at `time`. The same content already there goes
    /// back to the top instead, and keeps its id and its pin. Returns its id, or `None` when
    /// nothing unpinned could make room for it.
    pub fn push(&mut self, content: Content, app: String, time: u64) -> Option<u32> {
        if let Some(at) = self.entries.iter().position(|e| e.content.same(&content)) {
            let mut entry = self.entries.remove(at);
            entry.app = app;
            entry.time = time;
            let id = entry.id;
            self.entries.insert(0, entry);
            self.updated(id);
            return Some(id);
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.entries.insert(
            0,
            Entry {
                id,
                content,
                app,
                time,
                pinned: false,
            },
        );
        self.updated(id);
        self.trim();
        self.get(id).map(|e| e.id)
    }

    /// An item restored from disk, as it was, at the end: older than what is there.
    pub fn restore(&mut self, content: Content, app: String, time: u64, pinned: bool) {
        if self.entries.iter().any(|e| e.content.same(&content)) {
            return;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.entries.push(Entry {
            id,
            content,
            app,
            time,
            pinned,
        });
        self.updated(id);
        self.trim();
    }

    /// Moves item `id` to the top, copied again now.
    pub fn promote(&mut self, id: u32, time: u64) {
        if let Some(at) = self.entries.iter().position(|e| e.id == id) {
            let mut entry = self.entries.remove(at);
            entry.time = time;
            self.entries.insert(0, entry);
            self.updated(id);
        }
    }

    pub fn set_pinned(&mut self, id: u32, pinned: bool) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id)
            && entry.pinned != pinned
        {
            entry.pinned = pinned;
            self.updated(id);
        }
    }

    pub fn remove(&mut self, id: u32) {
        if let Some(at) = self.entries.iter().position(|e| e.id == id) {
            self.entries.remove(at);
            self.removed(id);
        }
    }

    /// « Tout effacer »: every item but the pinned ones.
    pub fn clear(&mut self) {
        let (kept, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|e| e.pinned);
        self.entries = kept;
        gone.iter().for_each(|e| self.removed(e.id));
    }

    /// Everything, pinned items included: the history was turned off.
    pub fn forget(&mut self) {
        let gone = std::mem::take(&mut self.entries);
        gone.iter().for_each(|e| self.removed(e.id));
    }

    /// What changed since the last call.
    pub fn take_changes(&mut self) -> Changes {
        std::mem::take(&mut self.changes)
    }

    fn updated(&mut self, id: u32) {
        if !self.changes.updated.contains(&id) {
            self.changes.updated.push(id);
        }
    }

    fn removed(&mut self, id: u32) {
        self.changes.updated.retain(|u| *u != id);
        self.changes.removed.push(id);
    }

    /// The oldest unpinned items go while there are too many, or too many bytes.
    fn trim(&mut self) {
        loop {
            let size: usize = self.entries.iter().map(|e| e.content.size()).sum();
            if self.entries.len() <= MAX_ENTRIES && size <= MAX_BYTES {
                return;
            }
            let Some(at) = self.entries.iter().rposition(|e| !e.pinned) else {
                return;
            };
            let id = self.entries.remove(at).id;
            self.removed(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Content {
        let bytes: Arc<[u8]> = Arc::from(s.as_bytes());
        Content::new(
            Kind::Text,
            Held::new(&["text/plain;charset=utf-8"], bytes),
            s.as_bytes(),
            s.into(),
        )
    }

    fn previews(history: &History) -> Vec<&str> {
        history
            .entries()
            .iter()
            .map(|e| e.content.preview.as_str())
            .collect()
    }

    #[test]
    fn a_copy_already_there_goes_back_to_the_top() {
        let mut history = History::default();
        let first = history.push(text("a"), "kate".into(), 1).unwrap();
        history.push(text("b"), "kate".into(), 2);
        history.take_changes();
        let again = history.push(text("a"), "firefox".into(), 3).unwrap();
        assert_eq!(first, again);
        assert_eq!(previews(&history), ["a", "b"]);
        let top = &history.entries()[0];
        assert_eq!((top.app.as_str(), top.time), ("firefox", 3));
        assert_eq!(history.take_changes().updated, [first]);
    }

    #[test]
    fn fifty_items_at_most_and_the_pinned_ones_stay() {
        let mut history = History::default();
        let pinned = history.push(text("keep"), String::new(), 0).unwrap();
        history.set_pinned(pinned, true);
        for i in 0..80 {
            history.push(text(&i.to_string()), String::new(), i);
        }
        assert_eq!(history.entries().len(), MAX_ENTRIES);
        assert!(history.get(pinned).is_some());
        assert_eq!(history.entries()[0].content.preview, "79");
        // The oldest unpinned ones went.
        assert!(history.entries().iter().all(|e| e.content.preview != "30"));
    }

    #[test]
    fn too_many_bytes_make_the_oldest_go() {
        let mut history = History::default();
        let big = |seed: u8| {
            let bytes: Arc<[u8]> = vec![seed; 50 << 20].into();
            Content::new(Kind::Image, Held::png(bytes), &[seed], String::new())
        };
        for seed in 0..4 {
            history.push(big(seed), String::new(), 0);
        }
        assert_eq!(history.entries().len(), 2);
    }

    #[test]
    fn clearing_keeps_the_pinned_items_and_forgetting_does_not() {
        let mut history = History::default();
        let a = history.push(text("a"), String::new(), 0).unwrap();
        let b = history.push(text("b"), String::new(), 0).unwrap();
        history.set_pinned(a, true);
        history.take_changes();
        history.clear();
        assert_eq!(previews(&history), ["a"]);
        assert_eq!(history.take_changes().removed, [b]);
        history.forget();
        assert!(history.entries().is_empty());
        assert_eq!(history.take_changes().removed, [a]);
    }

    #[test]
    fn an_item_removed_or_promoted() {
        let mut history = History::default();
        let a = history.push(text("a"), String::new(), 0).unwrap();
        let b = history.push(text("b"), String::new(), 0).unwrap();
        history.promote(a, 9);
        assert_eq!(previews(&history), ["a", "b"]);
        history.remove(b);
        history.remove(b);
        assert_eq!(previews(&history), ["a"]);
    }

    #[test]
    fn held_content_is_offered_under_each_type() {
        let bytes: Arc<[u8]> = Arc::from(&b"x"[..]);
        let held = Held::new(&["text/plain", "text/plain;charset=utf-8"], bytes);
        assert_eq!(held.types(), ["text/plain", "text/plain;charset=utf-8"]);
        assert_eq!(held.bytes("text/plain").as_deref(), Some(&b"x"[..]));
        assert_eq!(held.bytes("image/png"), None);
        assert_eq!(held.size(), 1);
    }
}
