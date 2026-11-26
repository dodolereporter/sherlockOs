//! The body markup of the specification, reduced to what a bubble shows: bold and italic. Every
//! other tag is dropped and its text kept as plain text; entities are decoded into text. The
//! result is a list of styled spans, never markup: nothing a client sends is interpreted as
//! HTML. Control characters and the invisible bidirectional formatting ones, which can make a
//! text read differently from what it holds, are dropped.

/// Bytes of a text read at most: what lies beyond is never looked at.
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
/// Bytes a tag may span, from `<` to `>`: a longer one is text.
const MAX_TAG_BYTES: usize = 256;
/// Bytes an entity may span, from `&` to `;`.
const MAX_ENTITY_BYTES: usize = 12;

/// A run of text and its style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

/// Whether `c` is left out of every text: a control character but the line break, or a
/// bidirectional formatting character (marks, embeddings, overrides, isolates).
pub fn hidden(c: char) -> bool {
    (c.is_control() && c != '\n')
        || matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The first `MAX_TEXT_BYTES` of `text`, cut on a character.
fn head(text: &str) -> &str {
    &text[..text.floor_char_boundary(MAX_TEXT_BYTES)]
}

/// `text` on one line, without hidden characters, cut after `max_chars` characters with an
/// ellipsis: names, titles, labels.
pub fn clean(text: &str, max_chars: usize) -> String {
    let mut kept = head(text)
        .trim()
        .chars()
        .filter(|c| !hidden(*c))
        .map(|c| if c == '\n' { ' ' } else { c });
    let mut out: String = kept.by_ref().take(max_chars).collect();
    if kept.next().is_some() {
        out.push('…');
    }
    out
}

/// The position of `byte` in the first `window` bytes of `text`.
fn find(text: &str, byte: u8, window: usize) -> Option<usize> {
    text.as_bytes().iter().take(window).position(|b| *b == byte)
}

/// The spans of `markup`, cut after `max_chars` characters with an ellipsis. Linear: each tag
/// or entity is looked for within a few bytes, and only the head of the text is read.
pub fn parse(markup: &str, max_chars: usize) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let (mut bold, mut italic) = (0_u32, 0_u32);
    let mut count = 0;
    let mut rest = head(markup);
    while let Some(c) = rest.chars().next() {
        if count >= max_chars {
            push(&mut spans, "…", bold > 0, italic > 0);
            break;
        }
        let (piece, advance) = match c {
            // A tag starts with a letter or `/` and holds no other `<`: « a < b » is text.
            '<' => match find(rest, b'>', MAX_TAG_BYTES).filter(|end| {
                let inside = &rest[1..*end];
                inside.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/')
                    && !inside.contains('<')
            }) {
                Some(end) => {
                    let tag = rest[1..end].trim().to_ascii_lowercase();
                    let (closing, name) = match tag.strip_prefix('/') {
                        Some(name) => (true, name.trim()),
                        None => (false, tag.as_str()),
                    };
                    let name = name
                        .split([' ', '/', '\t', '\n'])
                        .next()
                        .unwrap_or_default();
                    let depth = match name {
                        "b" => Some(&mut bold),
                        "i" => Some(&mut italic),
                        _ => None,
                    };
                    match depth {
                        Some(depth) if closing => *depth = depth.saturating_sub(1),
                        Some(depth) => *depth += 1,
                        None if name == "br" => count += 1,
                        None => {}
                    }
                    let piece = if name == "br" { "\n" } else { "" };
                    (std::borrow::Cow::Borrowed(piece), end + 1)
                }
                None => (std::borrow::Cow::Borrowed("<"), 1),
            },
            '&' => match entity(rest) {
                Some((decoded, length)) => (std::borrow::Cow::Owned(decoded.to_string()), length),
                None => (std::borrow::Cow::Borrowed("&"), 1),
            },
            _ if hidden(c) => (std::borrow::Cow::Borrowed(""), c.len_utf8()),
            _ => (std::borrow::Cow::Owned(c.to_string()), c.len_utf8()),
        };
        if !piece.is_empty() && piece != "\n" {
            count += 1;
        }
        push(&mut spans, &piece, bold > 0, italic > 0);
        rest = &rest[advance..];
    }
    spans
}

/// Adds `text` to the last span when it has the same style, or as a new span.
fn push(spans: &mut Vec<Span>, text: &str, bold: bool, italic: bool) {
    if text.is_empty() {
        return;
    }
    match spans.last_mut() {
        Some(last) if last.bold == bold && last.italic == italic => last.text.push_str(text),
        _ => spans.push(Span {
            text: text.to_owned(),
            bold,
            italic,
        }),
    }
}

/// The character of the entity at the start of `text` (`&amp;`, `&#233;`, `&#xE9;`…) and its
/// length, or `None` when it is not one.
fn entity(text: &str) -> Option<(char, usize)> {
    let end = find(text, b';', MAX_ENTITY_BYTES)?;
    let name = &text[1..end];
    let decoded = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code).filter(|c| !hidden(*c))?
        }
    };
    Some((decoded, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, bold: bool, italic: bool) -> Span {
        Span {
            text: text.to_owned(),
            bold,
            italic,
        }
    }

    #[test]
    fn bold_and_italic_become_styles() {
        assert_eq!(
            parse("Build <b>terminé</b> en <i>3 <b>s</b></i>", 300),
            vec![
                span("Build ", false, false),
                span("terminé", true, false),
                span(" en ", false, false),
                span("3 ", false, true),
                span("s", true, true),
            ]
        );
    }

    #[test]
    fn injected_html_is_neutralized() {
        let body = "<script>alert(1)</script><b onclick=\"x()\">Hi</b><img src=x onerror=y>\
                    <a href=\"https://example.org\">lien</a> &lt;b&gt;pas gras&lt;/b&gt; &amp;";
        let spans = parse(body, 300);
        assert_eq!(
            spans,
            vec![
                span("alert(1)", false, false),
                span("Hi", true, false),
                span("lien <b>pas gras</b> &", false, false),
            ]
        );
        // What comes out is text only: the escaped tags are characters, not markup.
        assert!(
            spans
                .iter()
                .all(|s| !s.text.contains("<script") && !s.text.contains("href"))
        );
    }

    #[test]
    fn entities_breaks_and_stray_characters() {
        let spans = parse("a < b &#233;&#xE9; &bogus; &#0; c<br/>d", 300);
        assert_eq!(
            spans,
            vec![span("a < b éé &bogus; &#0; c\nd", false, false)]
        );
        // An unclosed tag keeps the style to the end, a stray closing tag changes nothing.
        assert_eq!(parse("</b><b>x", 300), vec![span("x", true, false)]);
    }

    #[test]
    fn bidirectional_overrides_and_controls_are_dropped() {
        let spans = parse("abc\u{202E}gpj.exe\u{2066}x&#x202E;\u{7}\ty", 300);
        assert_eq!(spans, vec![span("abcgpj.exex&#x202E;y", false, false)]);
        assert_eq!(clean(" a\u{200F}b\nc\u{2069} ", 10), "ab c");
        assert_eq!(clean("abcdef", 3), "abc…");
    }

    #[test]
    fn hostile_bodies_stay_linear() {
        // Megabytes of unclosed tags and entities: only the head is read, each `<` and `&`
        // looks a few bytes ahead.
        let body = "<".repeat(4 << 20) + &"&".repeat(4 << 20);
        let start = std::time::Instant::now();
        let spans = parse(&body, 300);
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
        assert_eq!(spans[0].text.chars().count(), 301);
        let long = format!("<b{}>x", " ".repeat(MAX_TAG_BYTES));
        assert!(!parse(&long, 300)[0].bold);
        assert_eq!(clean(&"é".repeat(1 << 22), 5), "ééééé…");
    }

    #[test]
    fn long_bodies_are_cut() {
        let spans = parse(&"é".repeat(500), 10);
        assert_eq!(
            spans,
            vec![span(&format!("{}…", "é".repeat(10)), false, false)]
        );
    }
}
