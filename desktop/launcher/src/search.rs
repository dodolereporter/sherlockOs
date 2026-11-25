//! Matching a query against what a result is called: fuzzy on the name, by words on the
//! generic name and the keywords, ignoring case and accents. Every source of results uses it.

/// Scores of a term found in a text, best first.
const EXACT: u32 = 1000;
const PREFIX: u32 = 900;
pub const WORD: u32 = 800;
pub const SUBSTRING: u32 = 600;
/// The letters of the term in order, apart: lose `GAP` per letter skipped, `FUZZY_FLOOR` at
/// least, and win `WORD_START` when the first one starts a word.
const FUZZY: u32 = 400;
const GAP: u32 = 10;
const FUZZY_FLOOR: u32 = 100;
const WORD_START: u32 = 50;
/// What a match in the generic name, then in a keyword, gives up against the same match in
/// the name.
const GENERIC_PENALTY: u32 = 100;
const KEYWORD_PENALTY: u32 = 200;

/// `text` ready to compare: lower case, accents removed (« Éditeur » gives « editeur »).
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        let plain = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'ď' => 'd',
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
            'ł' => 'l',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
            'ř' => 'r',
            'ś' | 'š' => 's',
            'ť' => 't',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' => 'u',
            'ý' | 'ÿ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            'œ' => {
                out.push_str("oe");
                continue;
            }
            'æ' => {
                out.push_str("ae");
                continue;
            }
            'ß' => {
                out.push_str("ss");
                continue;
            }
            c => c,
        };
        out.push(plain);
    }
    out
}

/// The words of a query, folded. Every one must match for a result to count.
pub fn terms(query: &str) -> Vec<String> {
    fold(query).split_whitespace().map(str::to_owned).collect()
}

fn starts_word(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric())
}

/// Score of `term` in `text`, both folded; `None` when it is not there.
pub fn score(term: &str, text: &str) -> Option<u32> {
    if term.is_empty() {
        return None;
    }
    if text == term {
        return Some(EXACT);
    }
    if text.starts_with(term) {
        return Some(PREFIX);
    }
    let mut found = text.match_indices(term).map(|(at, _)| at).peekable();
    if found.peek().is_some() {
        let tier = if found.any(|at| starts_word(text, at)) {
            WORD
        } else {
            SUBSTRING
        };
        return Some(tier);
    }
    // The letters in order, each taken as early as possible.
    let mut letters = term.chars();
    let mut wanted = letters.next();
    let (mut first, mut last, mut count) = (None, 0, 0u32);
    for (index, (at, c)) in text.char_indices().enumerate() {
        if Some(c) == wanted {
            first.get_or_insert((index, at));
            last = index;
            count += 1;
            wanted = letters.next();
            if wanted.is_none() {
                break;
            }
        }
    }
    let (start, at) = first.filter(|_| wanted.is_none())?;
    let skipped = u32::try_from(last + 1 - start).unwrap_or(u32::MAX) - count;
    let bonus = if starts_word(text, at) { WORD_START } else { 0 };
    Some(
        FUZZY
            .saturating_sub(GAP.saturating_mul(skipped))
            .max(FUZZY_FLOOR)
            + bonus,
    )
}

/// What a result is called, folded once when the source loads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Haystack {
    name: String,
    generic: Option<String>,
    keywords: Vec<String>,
}

impl Haystack {
    pub fn new(name: &str, generic: Option<&str>, keywords: &[String]) -> Self {
        Self {
            name: fold(name),
            generic: generic.map(fold),
            keywords: keywords.iter().map(|k| fold(k)).collect(),
        }
    }

    /// The folded name, to order results of equal score.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Score of a query given as `terms`: the sum over its terms of the best place each is
    /// found. The generic name and the keywords count when a term starts one of their words,
    /// never for scattered letters. `None` when a term is found nowhere, or without terms.
    pub fn score(&self, terms: &[String]) -> Option<u32> {
        self.score_from(terms, 0)
    }

    /// Same as `score`, a term counting in the name only from `floor` up: `SUBSTRING` leaves
    /// out scattered letters, `WORD` keeps only the starts of words.
    pub fn score_from(&self, terms: &[String], floor: u32) -> Option<u32> {
        if terms.is_empty() {
            return None;
        }
        terms.iter().try_fold(0u32, |total, term| {
            let by_words = |text: &str, penalty: u32| {
                score(term, text)
                    .filter(|s| *s >= WORD)
                    .map(|s| s - penalty)
            };
            let generic = self
                .generic
                .as_deref()
                .and_then(|g| by_words(g, GENERIC_PENALTY));
            let keyword = self
                .keywords
                .iter()
                .filter_map(|k| by_words(k, KEYWORD_PENALTY))
                .max();
            let name = score(term, &self.name).filter(|s| *s >= floor);
            let best = [name, generic, keyword].into_iter().flatten().max()?;
            Some(total + best)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_and_accents_do_not_count() {
        assert_eq!(fold("Éditeur de TEXTE"), "editeur de texte");
        assert_eq!(fold("Cœur Straße"), "coeur strasse");
        assert_eq!(terms("  Navigateur  WEB "), ["navigateur", "web"]);
        assert_eq!(score("edi", &fold("Éditeur")), Some(PREFIX));
    }

    #[test]
    fn closer_matches_score_higher() {
        let exact = score("kate", "kate");
        let prefix = score("ka", "kate");
        let word = score("te", "kate text");
        let inside = score("at", "kate");
        let scattered = score("kt", "kate");
        let far = score("ke", "kitchen sink");
        assert!(exact > prefix && prefix > word && word > inside && inside > scattered);
        assert!(scattered > far && far >= Some(FUZZY_FLOOR));
        assert_eq!(score("xyz", "kate"), None);
        assert_eq!(score("etak", "kate"), None, "letters out of order");
        assert_eq!(score("", "kate"), None);
    }

    #[test]
    fn the_name_comes_before_the_generic_name_and_the_keywords() {
        let firefox = Haystack::new("Firefox", Some("Navigateur web"), &["Internet".into()]);
        let web = Haystack::new("Web", None, &[]);
        assert!(web.score(&terms("web")) > firefox.score(&terms("web")));
        assert!(firefox.score(&terms("navig")).is_some());
        assert!(firefox.score(&terms("inter")).is_some());
        // Scattered letters count in the name only.
        assert!(firefox.score(&terms("ngw")).is_none());
        assert!(firefox.score(&terms("ffx")).is_some());
        // Every term must be found.
        assert!(firefox.score(&terms("fire web")).is_some());
        assert!(firefox.score(&terms("fire mail")).is_none());
        assert!(firefox.score(&[]).is_none());
        // A floor leaves out the weaker places in the name.
        assert!(firefox.score_from(&terms("ffx"), SUBSTRING).is_none());
        assert!(firefox.score_from(&terms("refo"), SUBSTRING).is_some());
        assert!(firefox.score_from(&terms("refo"), WORD).is_none());
        assert!(firefox.score_from(&terms("fox"), WORD).is_none());
        assert!(firefox.score_from(&terms("fire"), WORD).is_some());
    }
}
