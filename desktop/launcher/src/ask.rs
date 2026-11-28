//! « Demander à Sherlock »: always the last entry when the query is not empty. Chosen, or Tab
//! pressed in the field, the query goes to the panel « Demande à Sherlock » (lot A-19,
//! `sherlock.rs`), which hands it to `sherlockd --ask` on its standard input: never in the
//! arguments, which every local account reads (`ps`, `/proc/*/cmdline`).

use sherlock_ui::launcher::text;

use crate::source::{Action, Hit, Query, Source, Tile};

#[derive(Debug, Clone, Copy, Default)]
pub struct Ask;

impl Source for Ask {
    fn section(&self) -> &'static str {
        text::SHERLOCK
    }

    /// One entry, the query as typed.
    fn search(&self, query: &Query, limit: usize) -> Vec<Hit> {
        if query.terms.is_empty() || limit == 0 {
            return Vec::new();
        }
        vec![Hit {
            title: text::ASK.to_owned(),
            subtitle: Some(text::ASK_LINE.to_owned()),
            tile: Tile::Sherlock,
            app: None,
            action: Action::Ask(query.raw.trim().to_owned()),
        }]
    }

    fn reserved(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_goes_to_sherlock_as_typed() {
        let hits = Ask.search(&Query::new(" range « mes » photos; rm -rf ~ "), 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, text::ASK);
        assert_eq!(hits[0].subtitle.as_deref(), Some(text::ASK_LINE));
        assert_eq!(
            hits[0].action,
            Action::Ask("range « mes » photos; rm -rf ~".into())
        );
        assert!(Ask.search(&Query::new("  "), 1).is_empty());
        assert!(Ask.search(&Query::new("x"), 0).is_empty());
    }
}
