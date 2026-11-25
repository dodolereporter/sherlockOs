//! Mesure de fluidité, lancée par `fichiers --mesurer <dossier>` : ouverture du dossier, puis
//! trois secondes de défilement continu, puis trois secondes de sélection qui descend d'une
//! ligne toutes les quelques images. Chaque phase écrit son bilan dans le journal (`tracing`),
//! puis l'application se ferme. Le dossier n'est jamais modifié.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Durée de chaque phase mesurée.
const PHASE: Duration = Duration::from_secs(3);
/// Défilement à chaque image, en pixels.
pub const SCROLL_STEP: f32 = 24.0;
/// Images entre deux pas de la sélection.
pub const SELECT_EVERY: usize = 9;
/// Une image au-delà de cet intervalle manque les 60 images par seconde.
const LATE: Duration = Duration::from_micros(16_667);

/// Ce que l'application doit faire à cette image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Open,
    Wait,
    Scroll,
    Select,
    Idle,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    Opening,
    /// Le dossier est chargé : l'image suivante prouve qu'il a été dessiné.
    Drawn,
    Scrolling,
    Selecting,
    Done,
}

/// État de la mesure.
#[derive(Debug, Clone)]
pub struct Probe {
    pub target: PathBuf,
    phase: Phase,
    since: Option<Instant>,
    last: Option<Instant>,
    intervals: Vec<Duration>,
}

impl Probe {
    pub fn new(target: PathBuf) -> Self {
        Self {
            target,
            phase: Phase::Start,
            since: None,
            last: None,
            intervals: Vec::new(),
        }
    }

    pub fn active(&self) -> bool {
        self.phase != Phase::Done
    }

    /// Une image vient d'être demandée. `loaded` : le dossier mesuré est affiché.
    pub fn frame(&mut self, now: Instant, loaded: bool) -> Step {
        if let Some(last) = self.last.replace(now) {
            self.intervals.push(now - last);
        }
        let since = *self.since.get_or_insert(now);
        match self.phase {
            Phase::Start => {
                self.phase = Phase::Opening;
                self.since = Some(now);
                Step::Open
            }
            Phase::Opening if loaded => {
                self.phase = Phase::Drawn;
                Step::Wait
            }
            Phase::Opening => Step::Wait,
            Phase::Drawn => {
                tracing::info!(
                    "mesure : ouverture du dossier en {:.1} ms",
                    (now - since).as_secs_f64() * 1000.0
                );
                self.next(Phase::Scrolling, now);
                Step::Scroll
            }
            Phase::Scrolling if now - since >= PHASE => {
                self.report("défilement", now - since);
                self.next(Phase::Selecting, now);
                Step::Idle
            }
            Phase::Scrolling => Step::Scroll,
            Phase::Selecting if now - since >= PHASE => {
                self.report("sélection animée", now - since);
                self.phase = Phase::Done;
                Step::Exit
            }
            Phase::Selecting if self.intervals.len().is_multiple_of(SELECT_EVERY) => Step::Select,
            Phase::Selecting => Step::Idle,
            Phase::Done => Step::Exit,
        }
    }

    fn next(&mut self, phase: Phase, now: Instant) {
        self.phase = phase;
        self.since = Some(now);
        self.intervals.clear();
    }

    fn report(&self, what: &str, elapsed: Duration) {
        let mut sorted = self.intervals.clone();
        sorted.sort();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let count = sorted.len();
        let p95 = sorted.get(count * 95 / 100).copied().unwrap_or_default();
        let max = sorted.last().copied().unwrap_or_default();
        let late = sorted.iter().filter(|d| **d > LATE).count();
        tracing::info!(
            "mesure : {what} : {count} images en {:.2} s, {:.1} images/s, intervalle p95 {:.1} ms, max {:.1} ms, {late} au-delà de 16,7 ms",
            elapsed.as_secs_f64(),
            count as f64 / elapsed.as_secs_f64(),
            ms(p95),
            ms(max),
        );
    }
}
