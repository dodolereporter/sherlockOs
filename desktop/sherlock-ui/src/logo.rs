//! Logo of the OS, with no text nor name. The SVGs and their PNGs are in `logo/`, written by
//! `logo/export.py`; the construction is described in `docs/sherlock-ui.md`.

/// The three versions of the logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Logo {
    /// Green ring, cream dot, night blue square.
    Color,
    /// Cream on night blue.
    Cream,
    /// Night blue on cream.
    Night,
}

impl Logo {
    pub const ALL: [Self; 3] = [Self::Color, Self::Cream, Self::Night];

    /// The logo in its square, for a display of `size` pixels per side. At 32 px and below, the
    /// optical version: thicker ring, larger symbol, shorter tail.
    pub const fn square(self, size: f32) -> &'static str {
        let [large, medium, small] = self.squares();
        if size <= 16.0 {
            small
        } else if size <= 32.0 {
            medium
        } else {
            large
        }
    }

    /// The symbol alone, without a square. It fills its frame and reads at every size.
    pub const fn bare(self) -> &'static str {
        match self {
            Self::Color => include_str!("../logo/logo-transparent.svg"),
            Self::Cream => include_str!("../logo/logo-cream-transparent.svg"),
            Self::Night => include_str!("../logo/logo-night-transparent.svg"),
        }
    }

    /// Versions with a square: large, optical 32 px, optical 16 px.
    const fn squares(self) -> [&'static str; 3] {
        match self {
            Self::Color => [
                include_str!("../logo/logo.svg"),
                include_str!("../logo/logo-32.svg"),
                include_str!("../logo/logo-16.svg"),
            ],
            Self::Cream => [
                include_str!("../logo/logo-cream.svg"),
                include_str!("../logo/logo-cream-32.svg"),
                include_str!("../logo/logo-cream-16.svg"),
            ],
            Self::Night => [
                include_str!("../logo/logo-night.svg"),
                include_str!("../logo/logo-night-32.svg"),
                include_str!("../logo/logo-night-16.svg"),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{self, Color};

    fn hex(c: Color) -> String {
        format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
    }

    fn all() -> Vec<&'static str> {
        Logo::ALL
            .iter()
            .flat_map(|l| l.squares().into_iter().chain([l.bare()]))
            .collect()
    }

    /// The first number that follows `marker` in `svg`.
    fn number_after(svg: &str, marker: &str) -> i32 {
        let start = svg.find(marker).expect("marker present") + marker.len();
        let digits: String = svg[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().expect("number")
    }

    #[test]
    fn logos_use_only_theme_colors() {
        let allowed = [color::DESKTOP, color::ACCENT, color::CREAM].map(hex);
        for svg in all() {
            for value in svg.split('"').filter(|v| v.starts_with('#')) {
                assert!(
                    allowed.iter().any(|a| a == value),
                    "color outside the theme: {value}"
                );
            }
            assert!(!svg.contains("<text"), "the logo has no text");
        }
    }

    #[test]
    fn the_tail_starts_on_the_ring_in_every_version() {
        for svg in all() {
            let r = number_after(svg, "r=\"");
            let x = number_after(svg, "d=\"M");
            let y = number_after(svg, &format!("d=\"M{x} "));
            assert_eq!((x - 256).pow(2) + (y - 256).pow(2), r.pow(2), "{svg}");
        }
    }

    #[test]
    fn small_sizes_take_the_optical_version() {
        let [large, medium, small] = Logo::Color.squares();
        assert_eq!(Logo::Color.square(16.0), small);
        assert_eq!(Logo::Color.square(32.0), medium);
        assert_eq!(Logo::Color.square(64.0), large);
        // The optical version has a thicker ring than the large one.
        let width = |svg| number_after(svg, "stroke-width=\"");
        assert!(width(small) > width(medium) && width(medium) > width(large));
    }
}
