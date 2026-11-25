//! The body markup of the specification, reduced to what a bubble shows: bold and italic. Every
//! other tag is dropped and its text kept as plain text; entities are decoded into text. The
//! result is a list of styled spans, never markup: nothing a client sends is interpreted as
//! HTML.

/// A run of text and its style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

/// The spans of `markup`, cut after `max_chars` characters with an ellipsis.
pub fn parse(markup: &str, max_chars: usize) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let (mut bold, mut italic) = (0_u32, 0_u32);
    let mut count = 0;
    let mut rest = markup;
    while let Some(c) = rest.chars().next() {
        if count >= max_chars {
            push(&mut spans, "…", bold > 0, italic > 0);
            break;
        }
        let (piece, advance) = match c {
            // A tag starts with a letter or `/` and holds no other `<`: « a < b » is text.
            '<' => match rest.find('>').filter(|end| {
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
    let end = text.get(..12).unwrap_or(text).find(';')?;
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
            char::from_u32(code).filter(|c| !c.is_control() || *c == '\n')?
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
    fn long_bodies_are_cut() {
        let spans = parse(&"é".repeat(500), 10);
        assert_eq!(
            spans,
            vec![span(&format!("{}…", "é".repeat(10)), false, false)]
        );
    }
}
