//! Couleurs du thème, tirées de `docs/contexte.md`.

/// Une couleur sRGB, avec son opacité entre 0 et 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Color {
    /// Couleur opaque, comme `#RRGGBB`.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Couleur translucide, comme `rgba(r, g, b, a)` en CSS.
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Composantes entre 0 et 1, multipliées par l'opacité : la forme qu'attendent les moteurs
    /// de rendu pour mélanger les couleurs.
    pub fn premultiplied(self) -> [f32; 4] {
        let channel = |c: u8| f32::from(c) / 255.0 * self.a;
        [channel(self.r), channel(self.g), channel(self.b), self.a]
    }
}

/// Vert de l'écharpe : action principale, état actif, réussite.
pub const ACCENT: Color = Color::rgb(0x85, 0xC5, 0x95);

/// Fond de l'entrée active d'une navigation.
pub const ACTIVE_ENTRY: Color = Color::rgba(133, 197, 149, 0.16);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_couleur_opaque_garde_ses_composantes() {
        assert_eq!(Color::rgb(255, 0, 51).premultiplied(), [1.0, 0.0, 0.2, 1.0]);
    }

    #[test]
    fn une_couleur_translucide_est_multipliee_par_son_opacite() {
        let [r, g, b, a] = Color::rgba(255, 255, 0, 0.5).premultiplied();
        assert_eq!((r, g, b, a), (0.5, 0.5, 0.0, 0.5));
    }
}
