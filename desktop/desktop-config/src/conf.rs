//! The `key = value` files of the desktop (`touchpad.conf`, `pointer.conf`, `wallpaper.conf`,
//! `lock.conf`): their lines, and how one key is changed in their text without touching the
//! rest (lot D-26a). Comments, blank lines, unknown keys and their order stay as they are.

/// Each line of `text` that is neither blank nor a comment, with its number from 1: its key
/// and value trimmed, or why it is not `key = value`.
pub(crate) fn pairs(text: &str) -> impl Iterator<Item = (usize, Result<(&str, &str), String>)> {
    text.lines().enumerate().filter_map(|(i, line)| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let pair = line
            .split_once('=')
            .map(|(key, value)| (key.trim(), value.trim()))
            .ok_or_else(|| String::from("expected key = value"));
        Some((i + 1, pair))
    })
}

/// `text` with `key` set to `value`: its last line, the one that counts, is replaced; without
/// one, a line is added at the end. Every other line is kept as it was.
pub fn set(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key} = {value}");
    let mut lines: Vec<&str> = text.lines().collect();
    let found = pairs(text)
        .filter(|(_, pair)| matches!(pair, Ok((k, _)) if *k == key))
        .map(|(number, _)| number - 1)
        .last();
    match found {
        Some(at) => lines[at] = &line,
        None => lines.push(&line),
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// `text` with each key of `mine` whose value differs from the one of `read` (the same keys,
/// read from `text`) set again: only what changed is written. A value with a line break is
/// refused: it would end its line and add others, so the key keeps the value of the file.
pub(crate) fn update(text: &str, mine: &[(&str, String)], read: &[(&str, String)]) -> String {
    let mut text = text.to_owned();
    for ((key, value), (_, before)) in mine.iter().zip(read) {
        if value != before && !mime_apps::breaks_line(value) {
            text = set(&text, key, value);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_key_changes_and_nothing_else() {
        let text = "# mine\nspeed = 0.4\n\n# the corner\nclick = areas\nfuture-key = 3\n";
        assert_eq!(
            set(text, "click", "fingers"),
            "# mine\nspeed = 0.4\n\n# the corner\nclick = fingers\nfuture-key = 3\n"
        );
        assert_eq!(set("a = 1", "b", "2"), "a = 1\nb = 2\n");
        assert_eq!(set("", "b", "2"), "b = 2\n");
        // The last line of a key is the one that counts.
        assert_eq!(set("a = 1\na = 2\n", "a", "3"), "a = 1\na = 3\n");
        // A commented key is not the key.
        assert_eq!(set("# a = 1\n", "a", "3"), "# a = 1\na = 3\n");
    }

    #[test]
    fn a_value_with_a_line_break_is_refused() {
        let read = [("path", String::from("/a.png"))];
        let mine = [("path", String::from("/b.png\nfollow-theme = false"))];
        assert_eq!(update("path = /a.png\n", &mine, &read), "path = /a.png\n");
        let mine = [("path", String::from("/b.png"))];
        assert_eq!(update("path = /a.png\n", &mine, &read), "path = /b.png\n");
    }
}
