//! Mesure des images (`--mesurer`) : cadence, temps de rendu, images en retard. Les temps
//! sont des `Duration` comptées depuis le démarrage, pour que le calcul se teste sans horloge.

use std::{fmt, time::Duration};

use sherlock_ui::motion;

/// Durée entre deux bilans du journal.
const PERIOD: Duration = Duration::from_secs(1);

/// Bilan d'une suite d'images.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Tally {
    pub frames: u32,
    /// Temps passé de la première à la dernière image.
    pub span: Duration,
    pub render_total: Duration,
    pub render_max: Duration,
    /// Plus long intervalle entre deux images.
    pub interval_max: Duration,
    /// Images arrivées plus d'une période de la cadence visée après la précédente.
    pub late: u32,
}

impl Tally {
    fn add(&mut self, interval: Option<Duration>, render: Duration) {
        self.frames += 1;
        self.render_total += render;
        self.render_max = self.render_max.max(render);
        if let Some(interval) = interval {
            self.span += interval;
            self.interval_max = self.interval_max.max(interval);
            self.late += u32::from(interval > frame_budget());
        }
    }

    /// Images par seconde sur la période.
    pub fn fps(&self) -> f64 {
        let intervals = self.frames.saturating_sub(1);
        if self.span.is_zero() {
            return 0.0;
        }
        f64::from(intervals) / self.span.as_secs_f64()
    }
}

/// Intervalle le plus long entre deux images pour tenir la cadence visée.
pub fn frame_budget() -> Duration {
    Duration::from_secs(1) / motion::FRAME_RATE
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

impl fmt::Display for Tally {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let average = self
            .render_total
            .checked_div(self.frames)
            .unwrap_or_default();
        write!(
            f,
            "{} images, {:.1} images/s, rendu moyen {:.2} ms, rendu max {:.2} ms, \
             intervalle max {:.2} ms, {} en retard (> {:.2} ms)",
            self.frames,
            self.fps(),
            ms(average),
            ms(self.render_max),
            ms(self.interval_max),
            self.late,
            ms(frame_budget()),
        )
    }
}

/// Statistiques tenues pendant toute la session.
#[derive(Debug, Default)]
pub struct FrameStats {
    last: Option<Duration>,
    period_start: Option<Duration>,
    period: Tally,
    /// Images dessinées pendant une animation, sur toute la session.
    pub animated: Tally,
    /// Toutes les images de la session.
    pub all: Tally,
}

impl FrameStats {
    /// Note une image finie à l'instant `now`, dessinée en `render`. Rend le bilan de la
    /// période écoulée, une fois par période.
    pub fn record(&mut self, now: Duration, render: Duration, animating: bool) -> Option<Tally> {
        let interval = self.last.map(|last| now.saturating_sub(last));
        self.last = Some(now);
        self.all.add(interval, render);
        if animating {
            self.animated.add(interval, render);
        }
        let start = *self.period_start.get_or_insert(now);
        self.period
            .add(interval.filter(|_| self.period.frames > 0), render);
        if now.saturating_sub(start) < PERIOD {
            return None;
        }
        self.period_start = Some(now);
        Some(std::mem::take(&mut self.period))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn soixante_images_regulieres_tiennent_la_cadence() {
        let mut stats = FrameStats::default();
        let step = Duration::from_micros(16_000);
        let mut report = None;
        for i in 0..=63 {
            report = report.or(stats.record(step * i, ms(2), true));
        }
        let report = report.unwrap();
        assert_eq!(report.late, 0);
        assert!((report.fps() - 62.5).abs() < 0.01, "{report}");
        assert_eq!(report.render_max, ms(2));
        assert_eq!(stats.animated.late, 0);
        assert_eq!(stats.animated.frames, 64);
    }

    #[test]
    fn une_image_manquee_est_comptee_en_retard() {
        let mut stats = FrameStats::default();
        for t in [0, 16, 32, 66, 82] {
            stats.record(ms(t), ms(1), t < 70);
        }
        assert_eq!(stats.all.late, 1);
        assert_eq!(stats.all.interval_max, ms(34));
        // La dernière image, sans animation, ne compte pas parmi les images animées.
        assert_eq!(stats.animated.frames, 4);
        assert_eq!(stats.animated.late, 1);
    }
}
