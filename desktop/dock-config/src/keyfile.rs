//! Key files, the format of desktop entries, icon themes and the dock configuration: groups
//! of `key=value` lines. Comments and blank lines are kept when a file is written back, and
//! values are escaped on writing: a value can never add a line of its own.

/// Languages tried for a localized key, before the key without language: the interface is in
/// French.
const LANGUAGES: [&str; 2] = ["fr_FR", "fr"];

/// A line of a group: a key and its value as written (escapes kept), or a line kept as it is
/// (comment, blank line, anything that is not `key=value`).
#[derive(Debug, Clone, PartialEq)]
enum Line {
    Pair(String, String),
    Other(String),
}

/// A key file: groups of `key=value` lines.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyFile {
    /// Lines before the first group.
    preamble: Vec<String>,
    groups: Vec<(String, Vec<Line>)>,
}

impl KeyFile {
    /// Reads a key file. Comments, blank lines and other lines are kept for writing back.
    pub fn parse(text: &str) -> Self {
        let mut file = Self::default();
        for original in text.lines() {
            let line = original.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                file.groups.push((name.to_owned(), Vec::new()));
                continue;
            }
            let pair = (!line.starts_with('#'))
                .then(|| line.split_once('='))
                .flatten()
                .map(|(key, value)| Line::Pair(key.trim().to_owned(), value.trim().to_owned()));
            match file.groups.last_mut() {
                Some((_, lines)) => {
                    lines.push(pair.unwrap_or_else(|| Line::Other(original.to_owned())));
                }
                None => file.preamble.push(original.to_owned()),
            }
        }
        file
    }

    fn pairs(&self, group: &str) -> impl Iterator<Item = (&str, &str)> {
        self.groups
            .iter()
            .filter(move |(name, _)| name == group)
            .take(1)
            .flat_map(|(_, lines)| lines)
            .filter_map(|line| match line {
                Line::Pair(key, value) => Some((key.as_str(), value.as_str())),
                Line::Other(_) => None,
            })
    }

    /// Raw value of `key` in `group`, escapes kept. The first one wins.
    fn raw(&self, group: &str, key: &str) -> Option<&str> {
        self.pairs(group).find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    /// String value of `key` in `group`, escapes (`\s`, `\n`, `\t`, `\r`, `\\`) resolved.
    pub fn get(&self, group: &str, key: &str) -> Option<String> {
        self.raw(group, key).map(unescape)
    }

    /// Localized string: `key[fr_FR]`, then `key[fr]`, then `key`.
    pub fn localized(&self, group: &str, key: &str) -> Option<String> {
        LANGUAGES
            .iter()
            .find_map(|lang| self.get(group, &format!("{key}[{lang}]")))
            .or_else(|| self.get(group, key))
    }

    /// List value, separated by `;`. A `;` inside an item is written `\;`, as the desktop entry
    /// specification says (lot D-45c): an item that holds one stays whole.
    pub fn list(&self, group: &str, key: &str) -> Vec<String> {
        self.raw(group, key).map(split_list).unwrap_or_default()
    }

    /// Sets `key` in `group` to the list `items`, each escaped, a `;` inside one as `\;`: what
    /// `list` reads back.
    pub fn set_list(&mut self, group: &str, key: &str, items: &[String]) {
        let raw: Vec<String> = items
            .iter()
            .map(|item| escape(item).replace(';', "\\;"))
            .collect();
        self.set_raw(group, key, raw.join(";"));
    }

    /// List value separated by `separator`: icon themes use commas.
    pub fn split(&self, group: &str, key: &str, separator: char) -> Vec<String> {
        self.get(group, key)
            .map(|v| {
                v.split(separator)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Boolean value: `true` only when written `true`.
    pub fn flag(&self, group: &str, key: &str) -> bool {
        self.raw(group, key) == Some("true")
    }

    /// Sets `key` in `group` to `value`, escaped, adding the group if needed. A line break in
    /// `value` is written `\n`: it cannot start a line of its own.
    pub fn set(&mut self, group: &str, key: &str, value: &str) {
        self.set_raw(group, key, escape(value));
    }

    /// Sets `key` in `group` to `escaped`, already escaped.
    fn set_raw(&mut self, group: &str, key: &str, escaped: String) {
        let index = match self.groups.iter().position(|(name, _)| name == group) {
            Some(index) => index,
            None => {
                self.groups.push((group.to_owned(), Vec::new()));
                self.groups.len() - 1
            }
        };
        let lines = &mut self.groups[index].1;
        let existing = lines
            .iter_mut()
            .find(|line| matches!(line, Line::Pair(k, _) if k == key));
        match existing {
            Some(line) => *line = Line::Pair(key.to_owned(), escaped),
            None => {
                // After the last pair, before the blank lines that end the group.
                let at = lines
                    .iter()
                    .rposition(|l| matches!(l, Line::Pair(..)))
                    .map_or(0, |i| i + 1);
                lines.insert(at, Line::Pair(key.to_owned(), escaped));
            }
        }
    }

    /// Writes the file back: its lines in their order, the values set since in place.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        for line in &self.preamble {
            text.push_str(line);
            text.push('\n');
        }
        for (name, lines) in &self.groups {
            text.push_str(&format!("[{name}]\n"));
            for line in lines {
                match line {
                    Line::Pair(key, value) => text.push_str(&format!("{key}={value}\n")),
                    Line::Other(other) => {
                        text.push_str(other);
                        text.push('\n');
                    }
                }
            }
        }
        text
    }
}

/// Escapes a value for writing: backslash, line breaks, tabs, and a leading space.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for (i, c) in value.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ' ' if i == 0 => out.push_str("\\s"),
            c => out.push(c),
        }
    }
    out
}

/// The items of a raw list value: split at each `;` that is not escaped, `\;` turned into `;`,
/// then each item unescaped and trimmed. Empty items are dropped.
fn split_list(raw: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut item = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(';') => item.push(';'),
                Some(next) => {
                    item.push('\\');
                    item.push(next);
                }
                None => item.push('\\'),
            },
            ';' => items.push(std::mem::take(&mut item)),
            c => item.push(c),
        }
    }
    items.push(item);
    items
        .iter()
        .map(|item| unescape(item).trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => {
                // `\;` stays escaped: it only matters when splitting a list.
                if other == ';' {
                    out.push('\\');
                }
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_item_keeps_an_escaped_semicolon() {
        let file = KeyFile::parse("[A]\nL=a\\;b;c;\\\\;d\\s;\\x\nOld=discord;steam;\n");
        // `\;` inside an item, `\\` before a separator, `\s` and an unknown escape as before.
        assert_eq!(file.list("A", "L"), ["a;b", "c", "\\", "d", "x"]);
        assert_eq!(file.list("A", "Old"), ["discord", "steam"]);
        let items: Vec<String> = ["chat;work", "a\\b", " lead", "plain", "line\nbreak"]
            .map(String::from)
            .to_vec();
        let mut file = KeyFile::default();
        file.set_list("A", "L", &items);
        let text = file.to_text();
        assert!(text.contains("L=chat\\;work;a\\\\b;"), "{text}");
        assert_eq!(text.lines().count(), 2, "no line of its own: {text}");
        let back = KeyFile::parse(&text).list("A", "L");
        // Trimmed, as every list item is.
        assert_eq!(back, ["chat;work", "a\\b", "lead", "plain", "line\nbreak"]);
    }

    #[test]
    fn escapes_and_localized_keys() {
        let file = KeyFile::parse("[A]\nName=Files\nName[fr]=Fichiers\nComment=a\\sb\\\\c\n");
        assert_eq!(file.localized("A", "Name").as_deref(), Some("Fichiers"));
        assert_eq!(file.get("A", "Comment").as_deref(), Some("a b\\c"));
        assert_eq!(file.localized("A", "Missing"), None);
    }

    #[test]
    fn a_key_file_writes_back_what_it_read() {
        let text = "# Written by hand.\n[Dock]\n# Left to right.\nPinned=a;b;\n\n[Other]\nX=y\n";
        let mut file = KeyFile::parse(text);
        assert_eq!(file.to_text(), text, "comments and blank lines kept");
        file.set("Dock", "Pinned", "a;");
        file.set("Dock", "IconTheme", "hicolor");
        assert_eq!(
            file.to_text(),
            "# Written by hand.\n[Dock]\n# Left to right.\nPinned=a;\nIconTheme=hicolor\n\n\
             [Other]\nX=y\n"
        );
        assert_eq!(KeyFile::parse(&file.to_text()), file);
    }

    #[test]
    fn a_value_cannot_add_a_line() {
        let mut file = KeyFile::default();
        file.set("Dock", "IconTheme", "evil\n[Dock]\nPosition=left");
        file.set("Dock", "Comment", " lead\\tab\t");
        let text = file.to_text();
        assert_eq!(text.lines().count(), 3, "{text}");
        let read = KeyFile::parse(&text);
        assert_eq!(
            read.get("Dock", "IconTheme").as_deref(),
            Some("evil\n[Dock]\nPosition=left")
        );
        assert_eq!(read.get("Dock", "Position"), None);
        assert_eq!(read.get("Dock", "Comment").as_deref(), Some(" lead\\tab\t"));
    }
}
