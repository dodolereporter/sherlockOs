//! Ressorts et fondus : la physique des animations du desktop, en calcul pur.
//!
//! Le compositeur et le shell partagent ces fonctions et ces jetons, sans iced. Le temps est
//! une `Duration` comptée depuis une origine que l'appelant choisit : une animation se calcule
//! à n'importe quel instant, sans état qui avance image par image.
//!
//! `docs/contexte.md` demande des ressorts avec un léger dépassement et des survols de 110 à
//! 150 ms. Les raideurs et les amortissements ci-dessous sont des choix du lot D-11 : un
//! dépassement de 2 à 3 %, posé en moins de 300 ms. Voir « Mouvement » dans
//! `docs/sherlock-ui.md`, et « Animations » dans `docs/compositeur.md` pour leur usage.

use std::time::Duration;

/// Ressort amorti, de masse 1 : `x'' = -raideur × (x - cible) - amortissement × x'`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub stiffness: f64,
    pub damping: f64,
}

impl Spring {
    pub const fn new(stiffness: f64, damping: f64) -> Self {
        Self { stiffness, damping }
    }

    /// Taux d'amortissement : sous 1, le ressort dépasse sa cible avant de s'y poser.
    pub fn damping_ratio(self) -> f64 {
        self.damping / (2.0 * self.stiffness.sqrt())
    }

    /// Écart à la cible et vitesse au bout de `t` secondes, en partant de l'écart `offset`
    /// avec la vitesse `velocity`. Solution exacte de l'équation, sans pas de temps.
    pub fn solve(self, offset: f64, velocity: f64, t: f64) -> (f64, f64) {
        let omega = self.stiffness.sqrt();
        let zeta = self.damping_ratio();
        if zeta < 1.0 {
            let decay = zeta * omega;
            let omega_d = omega * (1.0 - zeta * zeta).sqrt();
            let b = (velocity + decay * offset) / omega_d;
            let (sin, cos) = (omega_d * t).sin_cos();
            let e = (-decay * t).exp();
            let position = e * (offset * cos + b * sin);
            let speed =
                e * (velocity * cos - (decay * velocity + omega * omega * offset) / omega_d * sin);
            (position, speed)
        } else if (zeta - 1.0).abs() < 1e-9 {
            let b = velocity + omega * offset;
            let e = (-omega * t).exp();
            (e * (offset + b * t), e * (velocity - omega * b * t))
        } else {
            let root = omega * (zeta * zeta - 1.0).sqrt();
            let (r1, r2) = (-zeta * omega + root, -zeta * omega - root);
            let c2 = (velocity - r1 * offset) / (r2 - r1);
            let c1 = offset - c2;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        }
    }
}

/// Une valeur menée par un ressort vers sa cible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animation {
    spring: Spring,
    from: f64,
    to: f64,
    velocity: f64,
    start: Duration,
}

impl Animation {
    /// Part de `from` au repos, à l'instant `start`, vers `to`.
    pub fn new(spring: Spring, from: f64, to: f64, start: Duration) -> Self {
        Self {
            spring,
            from,
            to,
            velocity: 0.0,
            start,
        }
    }

    fn solve(&self, now: Duration) -> (f64, f64) {
        let t = now.saturating_sub(self.start).as_secs_f64();
        let (offset, speed) = self.spring.solve(self.from - self.to, self.velocity, t);
        (self.to + offset, speed)
    }

    /// Valeur à l'instant `now`.
    pub fn value(&self, now: Duration) -> f64 {
        self.solve(now).0
    }

    /// Cible.
    pub fn target(&self) -> f64 {
        self.to
    }

    /// Change de cible sans à-coup : la valeur et la vitesse de `now` sont gardées.
    pub fn retarget(&mut self, now: Duration, to: f64) {
        let (value, velocity) = self.solve(now);
        *self = Self {
            from: value,
            to,
            velocity,
            start: now,
            ..*self
        };
    }

    /// La valeur est à moins de `precision` de sa cible et n'en bouge presque plus : elle
    /// parcourrait moins de `precision` en un dixième de seconde.
    pub fn is_done(&self, now: Duration, precision: f64) -> bool {
        let (value, speed) = self.solve(now);
        (value - self.to).abs() < precision && speed.abs() * 0.1 < precision
    }
}

/// Fondu d'une valeur, le plus souvent une opacité, sur une durée fixe, avec un départ vif et
/// une arrivée douce (cubique).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
    from: f32,
    to: f32,
    start: Duration,
    duration: Duration,
}

impl Fade {
    pub fn new(from: f32, to: f32, start: Duration, duration: Duration) -> Self {
        Self {
            from,
            to,
            start,
            duration,
        }
    }

    /// Avancement de 0 à 1.
    fn progress(&self, now: Duration) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let t = now.saturating_sub(self.start).as_secs_f32() / self.duration.as_secs_f32();
        1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
    }

    /// Valeur à l'instant `now`.
    pub fn value(&self, now: Duration) -> f32 {
        self.from + (self.to - self.from) * self.progress(now)
    }

    /// Cible.
    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn is_done(&self, now: Duration) -> bool {
        now >= self.start + self.duration
    }

    /// Repart de la valeur de `now` vers `to`. La durée est celle du trajet entier, réduite à
    /// la part qui reste à parcourir : un fondu interrompu à mi-course revient en deux fois
    /// moins de temps, et rien ne saute.
    pub fn retarget(&mut self, now: Duration, to: f32, full: Duration) {
        let from = self.value(now);
        let span = (self.to - self.from).abs().max(f32::EPSILON);
        let share = ((to - from).abs() / span).clamp(0.0, 1.0);
        *self = Self::new(from, to, now, full.mul_f32(share));
    }
}

// Jetons. Le taux d'amortissement de chaque ressort est noté : sous 1, il dépasse sa cible.

/// Calage, agrandissement et retour d'une fenêtre (taux 0,75, dépassement de 3 %).
pub const SNAP_SPRING: Spring = Spring::new(500.0, 33.5);
/// Ouverture d'une fenêtre : son échelle (taux 0,80, dépassement de 2 %).
pub const OPEN_SPRING: Spring = Spring::new(450.0, 34.0);
/// Aperçu du calage, plus vif que la fenêtre qu'il annonce (taux 0,79).
pub const PREVIEW_SPRING: Spring = Spring::new(700.0, 42.0);

/// Échelle d'une fenêtre au début de son ouverture.
pub const OPEN_SCALE: f64 = 0.92;
/// Fondu d'ouverture d'une fenêtre, et d'apparition de l'aperçu du calage : la durée d'un
/// survol, dans la gamme de 110 à 150 ms.
pub const FADE: Duration = Duration::from_millis(crate::metrics::HOVER_MS.1);

/// Cadence que le desktop doit tenir, animations comprises (`docs/contexte.md`).
pub const FRAME_RATE: u32 = 60;

/// Précision sous laquelle une géométrie animée est posée, en pixels.
pub const GEOMETRY_PRECISION: f64 = 0.5;
/// Précision sous laquelle une échelle animée est posée.
pub const SCALE_PRECISION: f64 = 0.001;

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Spring; 3] = [SNAP_SPRING, OPEN_SPRING, PREVIEW_SPRING];

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Valeurs d'un trajet de 0 à 1, toutes les millisecondes pendant `total` ms.
    fn course(spring: Spring, total: u64) -> Vec<f64> {
        let animation = Animation::new(spring, 0.0, 1.0, Duration::ZERO);
        (0..=total).map(|t| animation.value(ms(t))).collect()
    }

    #[test]
    fn chaque_ressort_converge_vers_sa_cible() {
        for spring in ALL {
            let animation = Animation::new(spring, 10.0, 250.0, ms(40));
            assert_eq!(animation.value(ms(40)), 10.0);
            assert!(
                (animation.value(ms(2040)) - 250.0).abs() < 1e-6,
                "{spring:?}"
            );
            assert!(animation.is_done(ms(2040), GEOMETRY_PRECISION));
            assert!(!animation.is_done(ms(60), GEOMETRY_PRECISION));
        }
    }

    #[test]
    fn le_depassement_est_leger_et_borne() {
        for spring in ALL {
            let peak = course(spring, 2000).into_iter().fold(0.0, f64::max);
            // Léger : visible, mais sous 4 %.
            assert!(peak > 1.005 && peak < 1.04, "{spring:?} culmine à {peak}");
            assert!(spring.damping_ratio() > 0.7 && spring.damping_ratio() < 0.85);
        }
    }

    #[test]
    fn les_ressorts_repondent_vite() {
        for spring in ALL {
            let values = course(spring, 1000);
            // 90 % du trajet en moins de 150 ms, posé à 1 % près en moins de 300 ms.
            let first_90 = values.iter().position(|v| *v >= 0.9).unwrap();
            assert!(first_90 < 150, "{spring:?} : 90 % à {first_90} ms");
            let settled = values.iter().rposition(|v| (v - 1.0).abs() > 0.01).unwrap() + 1;
            assert!(settled < 300, "{spring:?} : posé à {settled} ms");
        }
    }

    #[test]
    fn un_changement_de_cible_garde_la_valeur_et_la_vitesse() {
        let mut animation = Animation::new(SNAP_SPRING, 0.0, 100.0, Duration::ZERO);
        let before = (animation.value(ms(50)), animation.value(ms(51)));
        animation.retarget(ms(50), -40.0);
        assert!((animation.value(ms(50)) - before.0).abs() < 1e-9);
        // La vitesse est continue : la première milliseconde va encore dans le même sens.
        assert!(animation.value(ms(51)) - before.0 > 0.0);
        assert!((animation.value(ms(51)) - before.1).abs() < 0.1);
        assert!((animation.value(ms(3000)) + 40.0).abs() < 1e-6);
    }

    #[test]
    fn les_ressorts_amortis_ou_critiques_ne_depassent_pas() {
        for spring in [Spring::new(400.0, 40.0), Spring::new(400.0, 80.0)] {
            let values = course(spring, 3000);
            assert!(values.iter().all(|v| *v <= 1.0 + 1e-9), "{spring:?}");
            assert!((values[3000] - 1.0).abs() < 1e-3, "{spring:?}");
        }
    }

    #[test]
    fn le_fondu_dure_un_survol_et_ne_saute_pas() {
        let (low, high) = crate::metrics::HOVER_MS;
        assert!(FADE >= ms(low) && FADE <= ms(high));
        let mut fade = Fade::new(0.0, 1.0, ms(10), FADE);
        assert_eq!(fade.value(ms(10)), 0.0);
        assert!(fade.value(ms(80)) > 0.5 && fade.value(ms(80)) < 1.0);
        assert_eq!(fade.value(ms(10) + FADE), 1.0);
        assert!(fade.is_done(ms(10) + FADE));
        // Interrompu, il repart de sa valeur et revient en moins de temps.
        let midway = fade.value(ms(40));
        fade.retarget(ms(40), 0.0, FADE);
        assert_eq!(fade.value(ms(40)), midway);
        assert!(fade.is_done(ms(40) + FADE.mul_f32(midway)));
        assert_eq!(fade.value(ms(1000)), 0.0);
    }
}
