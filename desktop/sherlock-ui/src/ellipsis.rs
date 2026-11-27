//! A line of text that never gets cut short: when it does not fit its width, it ends with
//! « … » at the last whole character that fits (lot UI-01). iced 0.14 can only clip a text,
//! which leaves half a letter at the edge.
//!
//! The text is shaped for real, by the renderer: the cut follows the width of each glyph, not
//! an average. The cut is computed again only when the text, its font or its width changes.

use std::borrow::Cow;

use iced::advanced::layout::{self, Layout};
use iced::advanced::text::{self as core_text, Paragraph as _, paragraph};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Widget, mouse, renderer};
use iced::widget::text::{self, LineHeight, Wrapping};
use iced::{Element, Font, Length, Pixels, Rectangle, Renderer, Size, Theme, alignment};

use crate::color::Color;
use crate::typography;

/// What closes a cut text.
pub const MARK: &str = "…";
/// A text wider than its room by less than this is left whole: the overflow is a fraction of a
/// pixel, which shaping rounds differently from one renderer to the other (« Ne pas déranger »
/// takes 105.3 of the 105 its tile gives).
pub const TOLERANCE: f32 = 1.0;

/// The longest start of `content` that fits `max` once « … » follows it, as `measure` gives
/// widths. The whole text when it fits; « … » alone when not even one character does.
pub fn fit<'a>(content: &'a str, max: f32, mut measure: impl FnMut(&str) -> f32) -> Cow<'a, str> {
    let max = max + TOLERANCE;
    if measure(content) <= max {
        return Cow::Borrowed(content);
    }
    let cut = |end: usize| format!("{}{MARK}", content[..end].trim_end());
    // The character boundaries, the end excluded: a cut keeps at least one character less.
    let ends: Vec<usize> = content.char_indices().map(|(i, _)| i).skip(1).collect();
    // The widths grow with the length: a binary search finds the longest start that fits.
    let (mut low, mut high) = (0, ends.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        if measure(&cut(ends[middle - 1])) <= max {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    match low {
        0 => Cow::Borrowed(MARK),
        n => Cow::Owned(cut(ends[n - 1])),
    }
}

type StyleFn<'a> = Box<dyn Fn(&Theme) -> text::Style + 'a>;

/// What a cut depends on: the text, the room, the font and the size.
type CutKey = (String, u32, Font, u32);

/// Cuts kept at most: beyond, the cache starts again empty.
const KEPT_CUTS: usize = 4096;

/// The cuts found by any `Ellipsis` of the program.
static CUTS: std::sync::Mutex<Option<std::collections::HashMap<CutKey, String>>> =
    std::sync::Mutex::new(None);

fn cut_found(key: &CutKey) -> Option<String> {
    let cuts = CUTS.lock().ok()?;
    cuts.as_ref()?.get(key).cloned()
}

fn keep_cut(key: CutKey, shown: String) {
    if let Ok(mut cuts) = CUTS.lock() {
        let cuts = cuts.get_or_insert_with(std::collections::HashMap::new);
        if cuts.len() >= KEPT_CUTS {
            cuts.clear();
        }
        cuts.insert(key, shown);
    }
}

/// A single line of text, cut with « … » when it is too long for its width.
pub struct Ellipsis<'a> {
    content: Cow<'a, str>,
    font: Font,
    size: f32,
    line_height: LineHeight,
    width: Length,
    style: Option<StyleFn<'a>>,
}

/// `content` in the interface font and size, on one line: see the setters.
pub fn ellipsis<'a>(content: impl Into<Cow<'a, str>>) -> Ellipsis<'a> {
    Ellipsis {
        content: content.into(),
        font: typography::UI,
        size: typography::UI_SIZE,
        line_height: LineHeight::default(),
        width: Length::Shrink,
        style: None,
    }
}

impl<'a> Ellipsis<'a> {
    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn line_height(mut self, line_height: impl Into<LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }

    /// `Shrink` by default: as wide as the text, at most the width it is given. `Fill` takes
    /// the width left by its neighbours in a row.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Its color, taken when it is built, like iced's `text(..).color(..)`: inside
    /// `color::with_opacity`, it is faded.
    pub fn color(mut self, tint: Color) -> Self {
        let tint: iced::Color = tint.into();
        self.style = Some(Box::new(move |_| text::Style { color: Some(tint) }));
        self
    }

    /// Its color from the theme, at each redraw.
    pub fn style(mut self, style: impl Fn(&Theme) -> text::Style + 'a) -> Self {
        self.style = Some(Box::new(style));
        self
    }

    fn text<'b>(&self, content: &'b str, bounds: Size) -> core_text::Text<&'b str, Font> {
        core_text::Text {
            content,
            bounds,
            size: Pixels(self.size),
            line_height: self.line_height,
            font: self.font,
            align_x: text::Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: text::Shaping::Auto,
            wrapping: Wrapping::None,
        }
    }
}

/// The line laid out, and what it was cut for.
#[derive(Default)]
struct State {
    paragraph: paragraph::Plain<<Renderer as core_text::Renderer>::Paragraph>,
    /// The text, the width, the font and the size of the last cut, and the line shown.
    cut_for: Option<(String, u32, Font, u32)>,
    shown: String,
}

impl<Message> Widget<Message, Theme, Renderer> for Ellipsis<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        // A fixed width is the room the text gets.
        let limits = limits.width(self.width);
        let max = limits.max();
        let key = (
            self.content.to_string(),
            max.width.to_bits(),
            self.font,
            self.size.to_bits(),
        );
        if state.cut_for.as_ref() != Some(&key) {
            // A surface opened again (a resident program, lot P-02) has new states: the cut
            // found before is kept for the whole program, so that it is not shaped again.
            state.shown = match cut_found(&key) {
                Some(shown) => shown,
                None => {
                    let unbounded = Size::new(f32::INFINITY, max.height);
                    let shown = fit(&self.content, max.width, |probe| {
                        <Renderer as core_text::Renderer>::Paragraph::with_text(
                            self.text(probe, unbounded),
                        )
                        .min_bounds()
                        .width
                    })
                    .into_owned();
                    keep_cut(key.clone(), shown.clone());
                    shown
                }
            };
            state.cut_for = Some(key);
        }
        let _ = state.paragraph.update(self.text(state.shown.as_str(), max));
        let natural = state.paragraph.min_bounds();
        let width = match self.width {
            Length::Shrink => natural.width.min(max.width),
            _ => max.width,
        };
        layout::Node::new(limits.resolve(
            self.width,
            Length::Shrink,
            Size::new(width, natural.height),
        ))
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let style = self
            .style
            .as_ref()
            .map_or_else(text::Style::default, |style| style(theme));
        iced::advanced::widget::text::draw(
            renderer,
            defaults,
            layout.bounds(),
            state.paragraph.raw(),
            style,
            viewport,
        );
    }

    /// Selectors and screen readers get the whole text, not the cut one.
    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.text(None, layout.bounds(), &self.content);
    }
}

impl<'a, Message: 'a> From<Ellipsis<'a>> for Element<'a, Message> {
    fn from(widget: Ellipsis<'a>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seven pixels per character, « … » included.
    fn width(s: &str) -> f32 {
        7.0 * s.chars().count() as f32
    }

    #[test]
    fn a_text_that_fits_is_left_whole() {
        assert_eq!(fit("Maison", 42.0, width), "Maison");
        // A fraction of a pixel too wide still shows whole.
        assert_eq!(fit("Maison", 41.5, width), "Maison");
        assert!(matches!(fit("Maison", 100.0, width), Cow::Borrowed(_)));
    }

    #[test]
    fn a_text_too_long_ends_with_an_ellipsis_that_fits() {
        let long = "Livebox-Wi-Fi-du-voisin-du-dessus";
        let cut = fit(long, 69.0, width);
        assert_eq!(cut, "Livebox-W…");
        assert!(width(&cut) <= 70.0);
        // One more character would not fit.
        assert!(width("Livebox-Wi…") > 70.0);
    }

    #[test]
    fn the_cut_never_splits_a_character_nor_ends_on_a_space() {
        let cut = fit("Écouteurs de Camille", 76.0, width);
        assert_eq!(cut, "Écouteurs…");
        let cut = fit("Né à Noël", 34.0, width);
        assert!(cut.ends_with(MARK) && !cut.contains(" …"), "{cut}");
    }

    #[test]
    fn nothing_fits_but_the_mark() {
        assert_eq!(fit("Casque", 5.0, width), MARK);
        assert_eq!(fit("", -1.0, width), "");
    }
}
