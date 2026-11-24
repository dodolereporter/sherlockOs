//! Disposition des fenêtres : où les poser, comment les déplacer et les redimensionner.
//!
//! Du calcul pur sur des rectangles, sans Wayland ni rendu, pour se tester seul.

use smithay::utils::{Logical, Point, Rectangle, Size};

/// Bords tirés pendant un redimensionnement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Edges {
    /// Lit les bits de `xdg_toplevel.resize_edge` : haut 1, bas 2, gauche 4, droite 8.
    pub fn from_xdg_bits(bits: u32) -> Self {
        Self {
            top: bits & 1 != 0,
            bottom: bits & 2 != 0,
            left: bits & 4 != 0,
            right: bits & 8 != 0,
        }
    }

    /// Bords les plus proches du pointeur : on tire le coin du quart de fenêtre où il se trouve.
    pub fn nearest(window: Rectangle<i32, Logical>, pointer: Point<f64, Logical>) -> Self {
        let center = window.loc.to_f64() + window.size.to_f64().downscale(2.0).to_point();
        Self {
            top: pointer.y < center.y,
            bottom: pointer.y >= center.y,
            left: pointer.x < center.x,
            right: pointer.x >= center.x,
        }
    }
}

/// Position qui centre une fenêtre dans la zone, sans que son coin haut gauche en sorte.
pub fn centered(area: Rectangle<i32, Logical>, size: Size<i32, Logical>) -> Point<i32, Logical> {
    let x = area.loc.x + ((area.size.w - size.w) / 2).max(0);
    let y = area.loc.y + ((area.size.h - size.h) / 2).max(0);
    (x, y).into()
}

/// Position d'une fenêtre déplacée : elle suit le pointeur depuis le début du geste.
pub fn moved(
    initial: Point<i32, Logical>,
    grab_start: Point<f64, Logical>,
    pointer: Point<f64, Logical>,
) -> Point<i32, Logical> {
    (initial.to_f64() + (pointer - grab_start)).to_i32_round()
}

/// Taille demandée pendant un redimensionnement, bornée par celles que la fenêtre accepte.
///
/// `min` et `max` viennent du client ; 0 veut dire « pas de borne ».
pub fn resized(
    initial: Rectangle<i32, Logical>,
    edges: Edges,
    delta: Point<f64, Logical>,
    min: Size<i32, Logical>,
    max: Size<i32, Logical>,
) -> Size<i32, Logical> {
    let grow = |size: i32, d: f64, start: bool, end: bool| -> i32 {
        match (start, end) {
            (true, _) => size - d.round() as i32,
            (false, true) => size + d.round() as i32,
            (false, false) => size,
        }
    };
    let bound = |value: i32, min: i32, max: i32| -> i32 {
        let max = if max <= 0 { i32::MAX } else { max };
        value.min(max).max(min.max(1))
    };
    let w = grow(initial.size.w, delta.x, edges.left, edges.right);
    let h = grow(initial.size.h, delta.y, edges.top, edges.bottom);
    (bound(w, min.w, max.w), bound(h, min.h, max.h)).into()
}

/// Position d'une fenêtre redimensionnée : le bord opposé à celui qu'on tire reste en place.
pub fn anchored(
    initial: Rectangle<i32, Logical>,
    edges: Edges,
    size: Size<i32, Logical>,
) -> Point<i32, Logical> {
    let mut loc = initial.loc;
    if edges.left {
        loc.x += initial.size.w - size.w;
    }
    if edges.top {
        loc.y += initial.size.h - size.h;
    }
    loc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn centre_la_fenetre_dans_la_zone() {
        assert_eq!(
            centered(rect(0, 0, 1000, 800), (400, 200).into()),
            (300, 300).into()
        );
        assert_eq!(
            centered(rect(100, 50, 1000, 800), (400, 200).into()),
            (400, 350).into()
        );
    }

    #[test]
    fn une_fenetre_trop_grande_reste_dans_la_zone() {
        assert_eq!(
            centered(rect(10, 20, 300, 200), (600, 400).into()),
            (10, 20).into()
        );
    }

    #[test]
    fn le_deplacement_suit_le_pointeur() {
        let start = Point::from((50.0, 50.0));
        assert_eq!(
            moved((100, 100).into(), start, (80.4, 20.6).into()),
            (130, 71).into()
        );
        assert_eq!(moved((100, 100).into(), start, start), (100, 100).into());
    }

    #[test]
    fn lit_les_bords_xdg() {
        let bottom_right = Edges::from_xdg_bits(10);
        assert!(bottom_right.bottom && bottom_right.right);
        assert!(!bottom_right.top && !bottom_right.left);
        assert_eq!(Edges::from_xdg_bits(0), Edges::default());
    }

    #[test]
    fn choisit_le_coin_le_plus_proche() {
        let window = rect(0, 0, 200, 100);
        let top_left = Edges::nearest(window, (10.0, 10.0).into());
        assert!(top_left.top && top_left.left && !top_left.bottom && !top_left.right);
        let bottom_right = Edges::nearest(window, (190.0, 90.0).into());
        assert!(bottom_right.bottom && bottom_right.right);
    }

    #[test]
    fn tirer_a_droite_et_en_bas_agrandit() {
        let edges = Edges::from_xdg_bits(10);
        let size = resized(
            rect(0, 0, 200, 100),
            edges,
            (30.0, -20.0).into(),
            (0, 0).into(),
            (0, 0).into(),
        );
        assert_eq!(size, (230, 80).into());
    }

    #[test]
    fn tirer_a_gauche_et_en_haut_garde_le_coin_oppose() {
        let initial = rect(100, 100, 200, 100);
        let edges = Edges::from_xdg_bits(5);
        let size = resized(
            initial,
            edges,
            (-50.0, 10.0).into(),
            (0, 0).into(),
            (0, 0).into(),
        );
        assert_eq!(size, (250, 90).into());
        assert_eq!(anchored(initial, edges, size), (50, 110).into());
        // Le bord bas droit n'a pas bougé.
        assert_eq!(
            anchored(initial, edges, size) + size.to_point(),
            (300, 200).into()
        );
    }

    #[test]
    fn respecte_les_tailles_du_client() {
        let edges = Edges::from_xdg_bits(10);
        let initial = rect(0, 0, 200, 100);
        let small = resized(
            initial,
            edges,
            (-500.0, -500.0).into(),
            (120, 60).into(),
            (0, 0).into(),
        );
        assert_eq!(small, (120, 60).into());
        let big = resized(
            initial,
            edges,
            (500.0, 500.0).into(),
            (0, 0).into(),
            (300, 150).into(),
        );
        assert_eq!(big, (300, 150).into());
        // Sans minimum, une fenêtre garde au moins un pixel.
        let tiny = resized(
            initial,
            edges,
            (-500.0, -500.0).into(),
            (0, 0).into(),
            (0, 0).into(),
        );
        assert_eq!(tiny, (1, 1).into());
    }

    #[test]
    fn un_bord_non_tire_ne_change_pas() {
        let edges = Edges {
            right: true,
            ..Edges::default()
        };
        let size = resized(
            rect(0, 0, 200, 100),
            edges,
            (10.0, 40.0).into(),
            (0, 0).into(),
            (0, 0).into(),
        );
        assert_eq!(size, (210, 100).into());
    }
}
