//! Logo de l'OS, sans texte ni nom. Les SVG et leurs PNG sont dans `logo/`, écrits par
//! `logo/exporter.py` ; la construction est décrite dans `docs/sherlock-ui.md`.

/// Version couleur : anneau vert, point crème, carré bleu nuit.
pub const COLOR: &str = include_str!("../logo/logo.svg");
/// Version couleur sans carré, pour un fond sombre.
pub const COLOR_BARE: &str = include_str!("../logo/logo-sans-fond.svg");
/// Crème sur bleu nuit.
pub const CREAM: &str = include_str!("../logo/logo-creme.svg");
/// Crème sans carré, pour un fond sombre.
pub const CREAM_BARE: &str = include_str!("../logo/logo-creme-sans-fond.svg");
/// Bleu nuit sur crème.
pub const NIGHT: &str = include_str!("../logo/logo-nuit.svg");
/// Bleu nuit sans carré, pour un fond clair.
pub const NIGHT_BARE: &str = include_str!("../logo/logo-nuit-sans-fond.svg");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{self, Color};

    fn hex(c: Color) -> String {
        format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
    }

    #[test]
    fn les_logos_n_emploient_que_les_couleurs_du_theme() {
        let allowed = [color::DESKTOP, color::ACCENT, color::CREAM].map(hex);
        for svg in [COLOR, COLOR_BARE, CREAM, CREAM_BARE, NIGHT, NIGHT_BARE] {
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
    fn la_queue_part_de_l_anneau() {
        // Départ de la queue (340, 368) sur l'anneau de rayon 140 centré en (256, 256).
        assert!(COLOR.contains("r=\"140\"") && COLOR.contains("M340 368"));
        assert_eq!(
            (340 - 256_i32).pow(2) + (368 - 256_i32).pow(2),
            140_i32.pow(2)
        );
    }
}
