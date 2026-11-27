//! The items of the history as the panel holds them: in the order it shows them, and those a
//! query finds.

use launcher::search::fold;

/// What an item is: the `kind` enum of the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Link,
    Files,
    Image,
}

impl Kind {
    /// The protocol's value; an unknown one reads as text.
    pub fn from_wire(value: u32) -> Self {
        match value {
            1 => Self::Link,
            2 => Self::Files,
            3 => Self::Image,
            _ => Self::Text,
        }
    }
}

/// One item of the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: u32,
    pub kind: Kind,
    pub pinned: bool,
    /// When it was copied, in seconds since the Unix epoch.
    pub time: u64,
    /// The app_id or X11 class of its application; empty when unknown.
    pub app: String,
    pub preview: String,
    /// Its thumbnail, a PNG, for a picture.
    pub thumbnail: Option<Vec<u8>>,
    /// `preview` and `app` folded, for the search.
    folded: String,
}

impl Item {
    pub fn new(id: u32, kind: Kind, pinned: bool, time: u64, app: String, preview: String) -> Self {
        let folded = fold(&format!("{preview}\n{app}"));
        Self {
            id,
            kind,
            pinned,
            time,
            app,
            preview,
            thumbnail: None,
            folded,
        }
    }

    /// Every word of `query` is in its preview or its application.
    pub fn matches(&self, query: &str) -> bool {
        fold(query)
            .split_whitespace()
            .all(|word| self.folded.contains(word))
    }

    /// Its application, as a name: the last part of an app_id (`org.kde.kate` gives
    /// « Kate »), with a capital.
    pub fn app_name(&self) -> String {
        let last = self.app.rsplit('.').next().unwrap_or_default();
        let mut chars = last.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        }
    }
}

/// The items in the panel's order: pinned first, then newest first.
pub fn sort(items: &mut [Item]) {
    items.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.time.cmp(&a.time))
            .then(b.id.cmp(&a.id))
    });
}

/// The items `query` finds, in order; all of them for an empty query.
pub fn found<'a>(items: &'a [Item], query: &str) -> Vec<&'a Item> {
    items.iter().filter(|i| i.matches(query)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u32, pinned: bool, time: u64, preview: &str) -> Item {
        Item::new(
            id,
            Kind::Text,
            pinned,
            time,
            "org.kde.kate".into(),
            preview.into(),
        )
    }

    #[test]
    fn pinned_items_come_first_then_the_newest() {
        let mut items = vec![
            item(1, false, 10, "a"),
            item(2, true, 1, "b"),
            item(3, false, 20, "c"),
            item(4, false, 20, "d"),
        ];
        sort(&mut items);
        let ids: Vec<u32> = items.iter().map(|i| i.id).collect();
        assert_eq!(ids, [2, 4, 3, 1]);
    }

    #[test]
    fn the_search_ignores_case_accents_and_order() {
        let items = vec![
            item(1, false, 0, "Déjà vu au café"),
            item(2, false, 0, "other"),
        ];
        let ids = |q: &str| found(&items, q).iter().map(|i| i.id).collect::<Vec<_>>();
        assert_eq!(ids("CAFE deja"), [1]);
        assert_eq!(ids(""), [1, 2]);
        assert_eq!(ids("kate"), [1, 2]);
        assert!(ids("nothing").is_empty());
    }

    #[test]
    fn the_application_reads_as_a_name() {
        assert_eq!(item(1, false, 0, "").app_name(), "Kate");
        let mut x11 = item(1, false, 0, "");
        x11.app = "firefox".into();
        assert_eq!(x11.app_name(), "Firefox");
        x11.app.clear();
        assert_eq!(x11.app_name(), "");
        assert_eq!(Kind::from_wire(3), Kind::Image);
        assert_eq!(Kind::from_wire(99), Kind::Text);
    }
}
