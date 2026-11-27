//! The path bar (lot E-04): names shown without the control characters they may hold, and
//! how many segments of the breadcrumb fold to fit. No iced here.

use std::borrow::Cow;

/// A character that must not reach the screen as it is: controls (a line feed, an escape),
/// and the marks that reverse the direction of a text (a name that shows « exe.txt » for an `.exe`).
fn unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// `name` safe to show: a control character becomes its picture (␊ for a line feed), a
/// direction mark becomes �.
pub fn visible(name: &str) -> Cow<'_, str> {
    if !name.chars().any(unsafe_char) {
        return Cow::Borrowed(name);
    }
    Cow::Owned(
        name.chars()
            .map(|c| match c {
                '\0'..='\u{1F}' => char::from_u32(0x2400 + c as u32).unwrap_or('\u{FFFD}'),
                '\u{7F}' => '\u{2421}',
                c if unsafe_char(c) => '\u{FFFD}',
                c => c,
            })
            .collect(),
    )
}

/// How many segments of the breadcrumb to fold behind « … », after the first, so that the
/// others fit `room`: `widths` of each segment, `chevron` between two, `fold` the width of
/// « … » and its chevron. The first and the last stay.
pub fn fold(widths: &[f32], chevron: f32, fold: f32, room: f32) -> usize {
    let total: f32 = widths.iter().sum::<f32>() + chevron * widths.len().saturating_sub(1) as f32;
    let mut width = total;
    let mut folded = 0;
    while width > room && folded + 2 < widths.len() {
        width -= widths[folded + 1] + chevron;
        if folded == 0 {
            width += fold;
        }
        folded += 1;
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_and_direction_marks_never_show() {
        assert_eq!(visible("notes.txt"), "notes.txt");
        assert_eq!(visible("a\nb\x1b[31m"), "a␊b␛[31m");
        assert_eq!(visible("photo\u{202E}gpj.exe"), "photo\u{FFFD}gpj.exe");
        assert_eq!(visible("del\x7f"), "del␡");
    }

    #[test]
    fn segments_fold_from_the_second_until_the_rest_fits() {
        let widths = [60.0, 80.0, 80.0, 80.0, 100.0];
        assert_eq!(fold(&widths, 10.0, 30.0, 500.0), 0);
        assert_eq!(fold(&widths, 10.0, 30.0, 400.0), 1);
        assert_eq!(fold(&widths, 10.0, 30.0, 250.0), 3);
        // The first and the last always stay.
        assert_eq!(fold(&widths, 10.0, 30.0, 10.0), 3);
    }
}
