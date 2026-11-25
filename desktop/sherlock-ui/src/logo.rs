//! Logo de l'OS, sans texte ni nom. Les SVG et leurs PNG sont dans `logo/`, écrits par
//! `logo/exporter.py` ; la construction est décrite dans `docs/sherlock-ui.md`.

/// Les trois versions du logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Logo {
    /// Anneau vert, point crème, carré bleu nuit.
    Color,
    /// Crème sur bleu nuit.
    Cream,
    /// Bleu nuit sur crème.
    Night,
}

impl Logo {
    pub const ALL: [Self; 3] = [Self::Color, Self::Cream, Self::Night];

    /// Le logo dans son carré, pour un affichage de `size` pixels de côté. À 32 px et moins, la
    /// version optique : anneau plus épais, symbole plus grand, queue plus courte.
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

    /// Le symbole seul, sans carré. Il remplit son cadre et se lit à toutes les tailles.
    pub const fn bare(self) -> &'static str {
        match self {
            Self::Color => include_str!("../logo/logo-transparent.svg"),
            Self::Cream => include_str!("../logo/logo-cream-transparent.svg"),
            Self::Night => include_str!("../logo/logo-night-transparent.svg"),
        }
    }

    /// Versions avec carré : grande, optique 32 px, optique 16 px.
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

    /// Le premier nombre qui suit `marker` dans `svg`.
    fn number_after(svg: &str, marker: &str) -> i32 {
        let start = svg.find(marker).expect("repère présent") + marker.len();
        let digits: String = svg[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().expect("nombre")
    }

    #[test]
    fn les_logos_n_emploient_que_les_couleurs_du_theme() {
        let allowed = [color::DESKTOP, color::ACCENT, color::CREAM].map(hex);
        for svg in all() {
            for value in svg.split('"').filter(|v| v.starts_with('#')) {
                assert!(
                    allowed.iter().any(|a| a == value),
                    "couleur hors thème : {value}"
                );
            }
            assert!(!svg.contains("<text"), "le logo n'a pas de texte");
        }
    }

    #[test]
    fn la_queue_part_de_l_anneau_dans_chaque_version() {
        for svg in all() {
            let r = number_after(svg, "r=\"");
            let x = number_after(svg, "d=\"M");
            let y = number_after(svg, &format!("d=\"M{x} "));
            assert_eq!((x - 256).pow(2) + (y - 256).pow(2), r.pow(2), "{svg}");
        }
    }

    #[test]
    fn les_petites_tailles_prennent_la_version_optique() {
        let [large, medium, small] = Logo::Color.squares();
        assert_eq!(Logo::Color.square(16.0), small);
        assert_eq!(Logo::Color.square(32.0), medium);
        assert_eq!(Logo::Color.square(64.0), large);
        // La version optique a un anneau plus épais que la grande.
        let width = |svg| number_after(svg, "stroke-width=\"");
        assert!(width(small) > width(medium) && width(medium) > width(large));
    }
}
