//! Disposition des fenêtres : où les poser, comment les déplacer, les redimensionner et les caler.
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

/// Bords saisis dans la bande qui entoure une fenêtre, à l'extérieur de sa géométrie `window`.
///
/// La bande fait `band` pixels le long des côtés. Aux coins, elle s'élargit à `corner` pixels,
/// en diagonale hors de la fenêtre comme le long des côtés : un appui près d'un coin tire le coin.
pub fn border_edges(
    window: Rectangle<i32, Logical>,
    pointer: Point<f64, Logical>,
    band: i32,
    corner: i32,
) -> Option<Edges> {
    let (x0, y0) = (f64::from(window.loc.x), f64::from(window.loc.y));
    let (w, h) = (f64::from(window.size.w), f64::from(window.size.h));
    let (x1, y1) = (x0 + w, y0 + h);
    // Distance hors de la fenêtre sur chaque axe, nulle en face d'elle.
    let dx = (x0 - pointer.x).max(pointer.x - x1).max(0.0);
    let dy = (y0 - pointer.y).max(pointer.y - y1).max(0.0);
    let inside = (x0..x1).contains(&pointer.x) && (y0..y1).contains(&pointer.y);
    let reach = if dx > 0.0 && dy > 0.0 { corner } else { band };
    if inside || dx.max(dy) > f64::from(reach) {
        return None;
    }
    // Une fenêtre étroite partage ses côtés entre les deux coins.
    let (kx, ky) = (
        f64::from(corner).min(w / 2.0),
        f64::from(corner).min(h / 2.0),
    );
    let left = pointer.x < x0 + kx;
    let top = pointer.y < y0 + ky;
    Some(Edges {
        top,
        bottom: !top && pointer.y >= y1 - ky,
        left,
        right: !left && pointer.x >= x1 - kx,
    })
}

/// Zone de l'écran où caler une fenêtre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// Tout l'écran : la fenêtre est agrandie.
    Maximized,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Zone {
    /// Zone visée par une fenêtre lâchée avec le pointeur en `pointer` : contre le bord haut,
    /// gauche ou droit de l'écran (à moins de `edge` pixels), ou dans un coin (à moins de
    /// `corner` pixels de ce coin, le long d'un bord). Le bord bas seul ne cale rien.
    pub fn under(
        area: Rectangle<i32, Logical>,
        pointer: Point<f64, Logical>,
        edge: i32,
        corner: i32,
    ) -> Option<Self> {
        let x = pointer.x - f64::from(area.loc.x);
        let y = pointer.y - f64::from(area.loc.y);
        let (w, h) = (f64::from(area.size.w), f64::from(area.size.h));
        let near = |limit: i32| {
            let limit = f64::from(limit);
            (x < limit, x >= w - limit, y < limit, y >= h - limit)
        };
        let (left, right, top, bottom) = near(edge);
        let (c_left, c_right, c_top, c_bottom) = near(corner);
        Some(if (left && c_top) || (top && c_left) {
            Self::TopLeft
        } else if (right && c_top) || (top && c_right) {
            Self::TopRight
        } else if (left && c_bottom) || (bottom && c_left) {
            Self::BottomLeft
        } else if (right && c_bottom) || (bottom && c_right) {
            Self::BottomRight
        } else if top {
            Self::Maximized
        } else if left {
            Self::Left
        } else if right {
            Self::Right
        } else {
            return None;
        })
    }

    /// Rectangle que prend une fenêtre calée dans cette zone de `area`. Un pixel impair va à
    /// droite ou en bas.
    pub fn geometry(self, area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        let Point { x, y, .. } = area.loc;
        let Size { w, h, .. } = area.size;
        let (half_w, half_h) = (w / 2, h / 2);
        let (loc, size) = match self {
            Self::Maximized => ((x, y), (w, h)),
            Self::Left => ((x, y), (half_w, h)),
            Self::Right => ((x + half_w, y), (w - half_w, h)),
            Self::TopLeft => ((x, y), (half_w, half_h)),
            Self::TopRight => ((x + half_w, y), (w - half_w, half_h)),
            Self::BottomLeft => ((x, y + half_h), (half_w, h - half_h)),
            Self::BottomRight => ((x + half_w, y + half_h), (w - half_w, h - half_h)),
        };
        Rectangle::new(loc.into(), size.into())
    }
}

/// Position d'une fenêtre qu'on détache de sa zone en la déplaçant : elle reprend la taille
/// `size` sous le pointeur, saisie à la même proportion de sa largeur et à la même distance de
/// son bord haut (dans la limite de sa nouvelle hauteur).
pub fn unsnapped(
    current: Rectangle<i32, Logical>,
    size: Size<i32, Logical>,
    pointer: Point<f64, Logical>,
) -> Point<i32, Logical> {
    let ratio = if current.size.w > 0 {
        ((pointer.x - f64::from(current.loc.x)) / f64::from(current.size.w)).clamp(0.0, 1.0)
    } else {
        0.5
    };
    let from_top = (pointer.y - f64::from(current.loc.y)).clamp(0.0, f64::from(size.h.max(0)));
    Point::<f64, Logical>::from((pointer.x - ratio * f64::from(size.w), pointer.y - from_top))
        .to_i32_round()
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

/// Redimensionnement d'une fenêtre, du premier mouvement jusqu'au dernier envoi du client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resize {
    pub edges: Edges,
    /// Géométrie de la fenêtre au début du geste.
    pub initial: Rectangle<i32, Logical>,
    /// Le bouton est relâché : le prochain envoi du client est le dernier du geste.
    pub done: bool,
}

impl Resize {
    /// À chaque nouveau contenu du client : position à donner à la fenêtre, et ce qu'il faut
    /// garder pour l'envoi suivant.
    pub fn on_commit(self, size: Size<i32, Logical>) -> (Point<i32, Logical>, Option<Self>) {
        let next = (!self.done).then_some(self);
        (anchored(self.initial, self.edges, size), next)
    }
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

    fn edges(top: bool, bottom: bool, left: bool, right: bool) -> Option<Edges> {
        Some(Edges {
            top,
            bottom,
            left,
            right,
        })
    }

    #[test]
    fn la_bande_autour_de_la_fenetre_tire_chaque_bord() {
        let window = rect(100, 100, 200, 100);
        let at = |x: f64, y: f64| border_edges(window, (x, y).into(), 8, 20);
        assert_eq!(at(95.0, 150.0), edges(false, false, true, false), "gauche");
        assert_eq!(at(305.0, 150.0), edges(false, false, false, true), "droite");
        assert_eq!(at(200.0, 93.0), edges(true, false, false, false), "haut");
        assert_eq!(at(200.0, 200.0), edges(false, true, false, false), "bas");
        // Le long d'un côté, près d'un coin : c'est le coin.
        assert_eq!(
            at(95.0, 110.0),
            edges(true, false, true, false),
            "haut gauche"
        );
        assert_eq!(
            at(290.0, 95.0),
            edges(true, false, false, true),
            "haut droit"
        );
        assert_eq!(
            at(95.0, 190.0),
            edges(false, true, true, false),
            "bas gauche"
        );
        assert_eq!(
            at(304.0, 199.0),
            edges(false, true, false, true),
            "bas droit"
        );
    }

    #[test]
    fn la_bande_est_plus_large_aux_coins() {
        let window = rect(100, 100, 200, 100);
        let at = |x: f64, y: f64| border_edges(window, (x, y).into(), 8, 20);
        // 15 pixels hors de la fenêtre : trop loin sur un côté, encore pris en diagonale.
        assert_eq!(at(85.0, 150.0), None);
        assert_eq!(at(85.0, 85.0), edges(true, false, true, false));
        assert_eq!(at(315.0, 215.0), edges(false, true, false, true));
        assert_eq!(at(79.0, 79.0), None);
    }

    #[test]
    fn la_bande_ne_couvre_pas_l_interieur() {
        let window = rect(100, 100, 200, 100);
        let at = |x: f64, y: f64| border_edges(window, (x, y).into(), 8, 20);
        assert_eq!(at(100.0, 100.0), None);
        assert_eq!(at(299.5, 199.5), None);
        assert_eq!(at(200.0, 150.0), None);
        assert_eq!(at(400.0, 150.0), None);
    }

    #[test]
    fn une_fenetre_etroite_partage_ses_cotes_entre_les_coins() {
        let window = rect(0, 0, 30, 30);
        let right = border_edges(window, (20.0, -3.0).into(), 8, 20);
        assert_eq!(right, edges(true, false, false, true));
    }

    fn zone(x: f64, y: f64) -> Option<Zone> {
        Zone::under(rect(0, 0, 1440, 900), (x, y).into(), 8, 64)
    }

    #[test]
    fn lachee_contre_un_bord_la_fenetre_se_cale() {
        assert_eq!(zone(700.0, 0.0), Some(Zone::Maximized));
        assert_eq!(zone(700.0, 7.5), Some(Zone::Maximized));
        assert_eq!(zone(0.0, 450.0), Some(Zone::Left));
        assert_eq!(zone(1439.0, 450.0), Some(Zone::Right));
        // Le pointeur peut sortir de l'écran pendant le geste : ça compte comme le bord.
        assert_eq!(zone(-20.0, 450.0), Some(Zone::Left));
        assert_eq!(zone(700.0, 450.0), None);
        assert_eq!(zone(700.0, 8.0), None);
        assert_eq!(zone(700.0, 899.0), None, "le bord bas seul ne cale rien");
    }

    #[test]
    fn lachee_dans_un_coin_la_fenetre_prend_un_quart() {
        assert_eq!(zone(0.0, 0.0), Some(Zone::TopLeft));
        assert_eq!(
            zone(3.0, 60.0),
            Some(Zone::TopLeft),
            "le long du bord gauche"
        );
        assert_eq!(zone(60.0, 3.0), Some(Zone::TopLeft), "le long du bord haut");
        assert_eq!(zone(1439.0, 10.0), Some(Zone::TopRight));
        assert_eq!(zone(2.0, 899.0), Some(Zone::BottomLeft));
        assert_eq!(zone(1400.0, 899.0), Some(Zone::BottomRight));
        assert_eq!(zone(3.0, 70.0), Some(Zone::Left), "hors du coin");
    }

    #[test]
    fn geometrie_de_chaque_zone() {
        let area = rect(10, 20, 1001, 801);
        let cases = [
            (Zone::Maximized, rect(10, 20, 1001, 801)),
            (Zone::Left, rect(10, 20, 500, 801)),
            (Zone::Right, rect(510, 20, 501, 801)),
            (Zone::TopLeft, rect(10, 20, 500, 400)),
            (Zone::TopRight, rect(510, 20, 501, 400)),
            (Zone::BottomLeft, rect(10, 420, 500, 401)),
            (Zone::BottomRight, rect(510, 420, 501, 401)),
        ];
        for (zone, expected) in cases {
            assert_eq!(zone.geometry(area), expected, "{zone:?}");
        }
    }

    #[test]
    fn detachee_la_fenetre_reprend_sa_taille_sous_le_pointeur() {
        let maximized = rect(0, 0, 1440, 900);
        // Saisie aux trois quarts de la largeur, à 12 pixels du haut.
        let loc = unsnapped(maximized, (400, 300).into(), (1080.0, 12.0).into());
        assert_eq!(loc, (780, 0).into());
        // Le pointeur reste aux trois quarts de la fenêtre rendue à sa taille.
        assert_eq!(f64::from(1080 - loc.x) / 400.0, 0.75);
        // Saisie plus bas que la nouvelle hauteur : le pointeur reste dans la fenêtre.
        let half = rect(720, 0, 720, 900);
        let loc = unsnapped(half, (400, 300).into(), (900.0, 800.0).into());
        assert_eq!(loc, (800, 500).into());
    }

    #[test]
    fn le_coin_oppose_reste_en_place_pendant_le_geste() {
        let initial = rect(100, 100, 200, 100);
        let top_left = Edges::from_xdg_bits(5);
        let during = Resize {
            edges: top_left,
            initial,
            done: false,
        };
        // Pendant le geste, chaque envoi recale la fenêtre et l'état reste.
        let (loc, next) = during.on_commit((260, 140).into());
        assert_eq!(loc, (40, 60).into());
        assert_eq!(next, Some(during));
        // Après le relâchement, le dernier envoi recale encore, puis l'état disparaît.
        let (loc, next) = Resize {
            done: true,
            ..during
        }
        .on_commit((250, 130).into());
        assert_eq!(loc, (50, 70).into());
        assert_eq!(next, None);
    }
}
