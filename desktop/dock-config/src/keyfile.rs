//! Key files, the format of desktop entries, icon themes and the dock configuration: groups
//! of `key=value` lines. Comments are not kept when a file is written back.

/// Languages tried for a localized key, before the key without language: the interface is in
/// French.
const LANGUAGES: [&str; 2] = ["fr_FR", "fr"];

/// A key file: groups of `key=value` lines.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyFile {
    groups: Vec<(String, Vec<(String, String)>)>,
}

impl KeyFile {
    /// Reads a key file. Comments, blank lines and lines outside a group are skipped.
    pub fn parse(text: &str) -> Self {
        let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                groups.push((name.to_owned(), Vec::new()));
            } else if let (Some((key, value)), Some((_, entries))) =
                (line.split_once('='), groups.last_mut())
            {
                entries.push((key.trim().to_owned(), value.trim().to_owned()));
            }
        }
        Self { groups }
    }

    /// Raw value of `key` in `group`, escapes kept. The first one wins.
    fn raw(&self, group: &str, key: &str) -> Option<&str> {
        let (_, entries) = self.groups.iter().find(|(name, _)| name == group)?;
        entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
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

    /// List value, separated by `;`.
    pub fn list(&self, group: &str, key: &str) -> Vec<String> {
        self.split(group, key, ';')
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

    /// Sets `key` in `group`, adding the group if needed.
    pub fn set(&mut self, group: &str, key: &str, value: String) {
        let index = match self.groups.iter().position(|(name, _)| name == group) {
            Some(index) => index,
            None => {
                self.groups.push((group.to_owned(), Vec::new()));
                self.groups.len() - 1
            }
        };
        let entries = &mut self.groups[index].1;
        match entries.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value,
            None => entries.push((key.to_owned(), value)),
        }
    }

    /// Writes the file back, group after group.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        for (name, entries) in &self.groups {
            text.push_str(&format!("[{name}]\n"));
            for (key, value) in entries {
                text.push_str(&format!("{key}={value}\n"));
            }
        }
        text
    }
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
    fn escapes_and_localized_keys() {
        let file = KeyFile::parse("[A]\nName=Files\nName[fr]=Fichiers\nComment=a\\sb\\\\c\n");
        assert_eq!(file.localized("A", "Name").as_deref(), Some("Fichiers"));
        assert_eq!(file.get("A", "Comment").as_deref(), Some("a b\\c"));
        assert_eq!(file.localized("A", "Missing"), None);
    }

    #[test]
    fn a_key_file_writes_back_what_it_read() {
        let mut file = KeyFile::parse("# comment\n[Dock]\nPinned=a;b;\n");
        file.set("Dock", "Pinned", String::from("a;"));
        file.set("Dock", "IconTheme", String::from("hicolor"));
        assert_eq!(file.to_text(), "[Dock]\nPinned=a;\nIconTheme=hicolor\n");
        assert_eq!(KeyFile::parse(&file.to_text()), file);
    }
}
