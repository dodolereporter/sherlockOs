//! Mouvements des fenêtres et de l'aperçu du calage : les ressorts et les fondus de
//! `sherlock_ui::motion` appliqués à des rectangles. Du calcul pur, sans rendu ni Wayland.

use std::time::Duration;

use sherlock_ui::motion::{self, Animation, Fade, Spring};
use smithay::utils::{Logical, Rectangle};

pub type RectF = Rectangle<f64, Logical>;

/// Ce qu'il faut dessiner d'une fenêtre ou de l'aperçu à un instant donné.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Visual {
    pub rect: RectF,
    pub opacity: f32,
}

/// `rect` mis à l'échelle `scale` autour de son centre.
pub fn scaled(rect: RectF, scale: f64) -> RectF {
    let size = rect.size.upscale(scale);
    let loc = rect.loc + (rect.size.to_point() - size.to_point()).downscale(2.0);
    Rectangle::new(loc, size)
}

fn parts(rect: RectF) -> [f64; 4] {
    [rect.loc.x, rect.loc.y, rect.size.w, rect.size.h]
}

/// Rectangle mené par quatre ressorts : position et taille.
#[derive(Debug, Clone, Copy)]
pub struct AnimatedRect([Animation; 4]);

impl AnimatedRect {
    pub fn new(spring: Spring, from: RectF, to: RectF, now: Duration) -> Self {
        let (from, to) = (parts(from), parts(to));
        Self(std::array::from_fn(|i| {
            Animation::new(spring, from[i], to[i], now)
        }))
    }

    /// Rectangle à l'instant `now`. Un côté posé est à sa cible exacte, sans reste de flottant.
    pub fn value(&self, now: Duration) -> RectF {
        let [x, y, w, h] = self.0.map(|a| {
            if a.is_done(now, motion::GEOMETRY_PRECISION) {
                a.target()
            } else {
                a.value(now)
            }
        });
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// Change de cible sans à-coup.
    pub fn retarget(&mut self, now: Duration, to: RectF) {
        for (animation, to) in self.0.iter_mut().zip(parts(to)) {
            if animation.target() != to {
                animation.retarget(now, to);
            }
        }
    }

    pub fn is_done(&self, now: Duration) -> bool {
        self.0
            .iter()
            .all(|a| a.is_done(now, motion::GEOMETRY_PRECISION))
    }
}

/// Mouvement d'une fenêtre : ouverture (échelle et opacité), puis changements de géométrie
/// annoncés par `animate_next_change`. Les autres changements, comme ceux d'un glisser ou
/// d'un redimensionnement, sont suivis sans délai.
#[derive(Debug, Default)]
pub struct WindowMotion {
    /// Ouverture : échelle et opacité. `None` une fois finie.
    open: Option<(Animation, Fade)>,
    opened: bool,
    /// Géométrie animée. `None` au repos : la fenêtre est dessinée à sa place réelle.
    geometry: Option<AnimatedRect>,
    /// Dernière géométrie dessinée, avant l'échelle de l'ouverture.
    last: Option<RectF>,
    animate_next: bool,
}

impl WindowMotion {
    /// Le prochain changement de géométrie sera animé : calage, agrandissement, retour.
    pub fn animate_next_change(&mut self) {
        self.animate_next = true;
    }

    /// Ce qu'il faut dessiner à l'instant `now` pour une fenêtre dont la géométrie réelle est
    /// `target`. Le premier appel lance l'ouverture.
    pub fn frame(&mut self, now: Duration, target: Rectangle<i32, Logical>) -> Visual {
        let target = target.to_f64();
        if !self.opened {
            self.opened = true;
            self.open = Some((
                Animation::new(motion::OPEN_SPRING, motion::OPEN_SCALE, 1.0, now),
                Fade::new(0.0, 1.0, now, motion::FADE),
            ));
        }
        match (&mut self.geometry, self.last) {
            (Some(geometry), _) => geometry.retarget(now, target),
            (None, Some(last)) if self.animate_next && last != target => {
                self.geometry = Some(AnimatedRect::new(motion::SNAP_SPRING, last, target, now));
                self.animate_next = false;
            }
            // Jamais dessinée : rien d'où partir, elle apparaîtra à sa place.
            (None, None) => self.animate_next = false,
            _ => {}
        }
        let rect = match self.geometry {
            Some(geometry) if !geometry.is_done(now) => geometry.value(now),
            _ => {
                self.geometry = None;
                target
            }
        };
        self.last = Some(rect);

        let (scale, opacity) = match self.open {
            Some((scale, fade))
                if !(scale.is_done(now, motion::SCALE_PRECISION) && fade.is_done(now)) =>
            {
                (scale.value(now), fade.value(now))
            }
            _ => {
                self.open = None;
                (1.0, 1.0)
            }
        };
        Visual {
            rect: scaled(rect, scale),
            opacity,
        }
    }

    /// Une animation est en cours : la géométrie dessinée n'est pas la géométrie réelle.
    pub fn is_animating(&self) -> bool {
        self.open.is_some() || self.geometry.is_some()
    }
}

/// Mouvement de l'aperçu du calage : il grandit en apparaissant, glisse d'une zone à l'autre
/// et s'efface à sa dernière place.
#[derive(Debug, Default)]
pub struct PreviewMotion {
    shown: Option<(AnimatedRect, Fade)>,
}

impl PreviewMotion {
    /// Ce qu'il faut dessiner à l'instant `now`, `zone` étant la zone visée s'il y en a une.
    pub fn frame(
        &mut self,
        now: Duration,
        zone: Option<Rectangle<i32, Logical>>,
    ) -> Option<Visual> {
        match (&mut self.shown, zone) {
            (Some((rect, fade)), Some(zone)) => {
                rect.retarget(now, zone.to_f64());
                if fade.target() < 1.0 {
                    fade.retarget(now, 1.0, motion::FADE);
                }
            }
            (None, Some(zone)) => {
                let zone = zone.to_f64();
                let from = scaled(zone, motion::OPEN_SCALE);
                self.shown = Some((
                    AnimatedRect::new(motion::PREVIEW_SPRING, from, zone, now),
                    Fade::new(0.0, 1.0, now, motion::FADE),
                ));
            }
            (Some((_, fade)), None) if fade.target() > 0.0 => {
                fade.retarget(now, 0.0, motion::FADE);
            }
            _ => {}
        }
        let (rect, fade) = self.shown?;
        if fade.target() <= 0.0 && fade.is_done(now) {
            self.shown = None;
            return None;
        }
        Some(Visual {
            rect: rect.value(now),
            opacity: fade.value(now),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn une_fenetre_s_ouvre_en_grandissant_et_en_apparaissant() {
        let mut motion = WindowMotion::default();
        let place = rect(100, 100, 400, 300);
        let first = motion.frame(ms(0), place);
        assert_eq!(first.opacity, 0.0);
        let expected = scaled(place.to_f64(), motion::OPEN_SCALE);
        assert!((first.rect.size.w - expected.size.w).abs() < 1e-9);
        assert!((first.rect.loc.y - expected.loc.y).abs() < 1e-9);
        // Centrée sur sa place pendant toute l'ouverture.
        let midway = motion.frame(ms(60), place);
        assert!(midway.opacity > 0.0 && midway.opacity < 1.0);
        let center = |r: RectF| r.loc + r.size.to_point().downscale(2.0);
        assert!((center(midway.rect) - center(place.to_f64())).x.abs() < 1e-9);
        assert!(motion.is_animating());
        let end = motion.frame(ms(1000), place);
        assert_eq!(end.rect, place.to_f64());
        assert_eq!(end.opacity, 1.0);
        assert!(!motion.is_animating());
    }

    #[test]
    fn un_calage_s_anime_et_un_glisser_suit_sans_delai() {
        let mut motion = WindowMotion::default();
        let (floating, left) = (rect(300, 200, 400, 300), rect(0, 0, 720, 900));
        motion.frame(ms(0), floating);
        motion.frame(ms(1000), floating);
        // Glisser : la fenêtre suit le pointeur.
        let dragged = rect(310, 205, 400, 300);
        assert_eq!(motion.frame(ms(1010), dragged).rect, dragged.to_f64());
        // Calage : elle part de sa place et rejoint la zone, d'abord la position, puis la
        // taille quand le client répond.
        motion.animate_next_change();
        let start = motion.frame(ms(1020), rect(0, 0, 400, 300)).rect;
        assert_eq!(start, dragged.to_f64());
        motion.frame(ms(1060), left);
        let moving = motion.frame(ms(1100), left).rect;
        assert!(moving.loc.x > 0.0 && moving.loc.x < 310.0);
        assert!(moving.size.w > 400.0 && moving.size.w < 720.0);
        assert_eq!(motion.frame(ms(2000), left).rect, left.to_f64());
        assert!(!motion.is_animating());
        // Le calage fini, le glisser suivant n'est plus animé.
        assert_eq!(motion.frame(ms(2010), floating).rect, floating.to_f64());
    }

    #[test]
    fn l_apercu_apparait_glisse_et_s_efface() {
        let mut preview = PreviewMotion::default();
        let (left, top) = (rect(0, 0, 720, 900), rect(0, 0, 1440, 900));
        assert_eq!(preview.frame(ms(0), None), None);
        let first = preview.frame(ms(0), Some(left)).unwrap();
        assert_eq!(first.opacity, 0.0);
        assert!(first.rect.size.w < 720.0);
        assert_eq!(
            preview.frame(ms(500), Some(left)).unwrap().rect,
            left.to_f64()
        );
        let sliding = preview.frame(ms(550), Some(top)).unwrap();
        assert!(sliding.rect.size.w > 720.0 && sliding.rect.size.w < 1440.0);
        assert_eq!(sliding.opacity, 1.0);
        let fading = preview.frame(ms(600), None).unwrap();
        assert_eq!(fading.opacity, 1.0);
        assert!(preview.frame(ms(650), None).unwrap().opacity < 1.0);
        assert_eq!(preview.frame(ms(600) + motion::FADE, None), None);
    }
}
