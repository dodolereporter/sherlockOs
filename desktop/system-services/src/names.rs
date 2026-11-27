//! Names chosen far away, shown as they are by the shell: the name of a Bluetooth device, the
//! description of a sound output (often a Bluetooth device's name too). Anyone near can name a
//! device « Clavier de Bob » with a right-to-left override, line breaks or a page of text, and
//! make a row look like another one: « Oublier » or « Confirmer » would then act on the wrong
//! device. What is shown is cleaned here.

/// The longest name shown, in characters; past it, « … ».
pub const MAX_NAME_CHARS: usize = 64;

/// The bytes read of a name at most, before cleaning.
const MAX_NAME_BYTES: usize = 1024;

/// Whether `c` is left out of a name: a control character (line breaks too) or a bidirectional
/// formatting character (marks, embeddings, overrides, isolates), as the notifications do.
pub fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// `text` as shown: on one line, without hidden characters, trimmed, cut after `MAX_NAME_CHARS`
/// characters with « … ».
pub fn shown(text: &str) -> String {
    let text = &text[..text.floor_char_boundary(MAX_NAME_BYTES)];
    let mut kept = text.trim().chars().filter(|c| !hidden(*c));
    let mut out: String = kept.by_ref().take(MAX_NAME_CHARS).collect();
    if kept.next().is_some() {
        out.push('…');
    }
    out
}

/// A name given as bytes (a Wi-Fi network's): its UTF-8 parts as they are, each other byte as
/// `\xNN`, so that two names that differ never show the same; then cleaned as `shown`.
pub fn shown_bytes(bytes: &[u8]) -> String {
    let bytes = &bytes[..bytes.len().min(MAX_NAME_BYTES)];
    let mut text = String::new();
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        for byte in chunk.invalid() {
            text.push_str(&format!("\\x{byte:02X}"));
        }
    }
    shown(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_far_away_are_cleaned() {
        assert_eq!(shown("  Casque  "), "Casque");
        // An override that would show « Bob » as « boB », a line that would add a row.
        assert_eq!(shown("Clavier \u{202E}boB\u{202C}"), "Clavier boB");
        assert_eq!(shown("Souris\nConfirmer"), "SourisConfirmer");
        assert_eq!(shown("a\u{0}b\u{1b}[31m"), "ab[31m");
        let long = shown(&"x".repeat(500));
        assert_eq!(long.chars().count(), MAX_NAME_CHARS + 1);
        assert!(long.ends_with('…'));
        assert_eq!(shown_bytes(b"Caf\xe9"), "Caf\\xE9");
        assert_ne!(shown_bytes(b"Caf\xe9"), shown_bytes(b"Caf\xe8"));
        assert_eq!(shown_bytes("Café".as_bytes()), "Café");
    }
}
