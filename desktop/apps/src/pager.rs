//! The pages of the grid and how they slide. A wheel notch turns one page; a touchpad swipe
//! drags the pages under the fingers, then they settle on the nearest one, or the next when the
//! swipe was long or quick. Past the first and the last page the pages stretch like a rubber
//! band and come back: the elastic bounce. Everything is computed from the time.

use std::time::Duration;

use sherlock_ui::apps as look;
use sherlock_ui::motion::Animation;

/// Pages needed for `count` applications, `per_page` on each: one at least, even empty.
pub fn page_count(count: usize, per_page: usize) -> usize {
    count.div_ceil(per_page.max(1)).max(1)
}

/// A touchpad swipe under way.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Swipe {
    /// Where the pages were when it started, in pages.
    from: f64,
    /// How far the fingers moved the pages, in pixels: to the right is positive.
    moved: f32,
    /// Its speed, in pixels per second, and its last event.
    speed: f32,
    last: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pager {
    count: usize,
    /// The page shown, or the one the pages slide to.
    page: usize,
    /// Position of the pages, in pages: 1.5 is halfway between the second and the third.
    slide: Animation,
    swipe: Option<Swipe>,
    /// The last wheel notch that turned a page.
    notch: Option<Duration>,
}

impl Pager {
    /// `count` pages, the first one shown, still.
    pub fn new(count: usize) -> Self {
        Self {
            count: count.max(1),
            page: 0,
            slide: Animation::new(look::PAGE_SPRING, 0.0, 0.0, Duration::ZERO),
            swipe: None,
            notch: None,
        }
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn count(&self) -> usize {
        self.count
    }

    /// Another number of pages (a search): back to the first one, at once.
    pub fn reset(&mut self, count: usize) {
        *self = Self::new(count);
    }

    /// Slides to `page`, kept within the pages.
    pub fn go(&mut self, page: usize, now: Duration) {
        let page = page.min(self.count - 1);
        if page != self.page || self.swipe.is_some() {
            self.swipe = None;
            self.page = page;
            self.slide.retarget(now, page as f64);
        }
    }

    /// A wheel notch: `forward` for the next page. Past the ends, the pages bounce.
    pub fn notch(&mut self, forward: bool, now: Duration) {
        if self
            .notch
            .is_some_and(|at| now.saturating_sub(at) < look::WHEEL_PAUSE)
        {
            return;
        }
        self.notch = Some(now);
        let next = if forward {
            Some(self.page + 1).filter(|p| *p < self.count)
        } else {
            self.page.checked_sub(1)
        };
        match next {
            Some(page) => self.go(page, now),
            None => {
                let speed = if forward { 1.0 } else { -1.0 } * look::BOUNCE_SPEED;
                let from = self.slide.value(now);
                self.slide =
                    Animation::with_velocity(look::PAGE_SPRING, from, self.page as f64, speed, now);
            }
        }
    }

    /// The fingers moved the pages by `dx` pixels (to the right: towards the previous page), on
    /// a screen `width` wide.
    pub fn swipe(&mut self, dx: f32, width: f32, now: Duration) {
        let width = width.max(1.0);
        let from = self.position(now, width) as f64 / width as f64;
        let swipe = self.swipe.get_or_insert(Swipe {
            from,
            moved: 0.0,
            speed: 0.0,
            last: now,
        });
        let elapsed = now.saturating_sub(swipe.last).as_secs_f32();
        if elapsed > 0.0 {
            // The latest events weigh most: the speed at the moment the fingers lift.
            let instant = dx / elapsed;
            swipe.speed = 0.5 * swipe.speed + 0.5 * instant;
        }
        swipe.moved += dx;
        swipe.last = now;
    }

    /// At each frame: a swipe without events for `SWIPE_END` is over, and the pages settle.
    pub fn tick(&mut self, now: Duration, width: f32) {
        let Some(swipe) = self.swipe else {
            return;
        };
        if now.saturating_sub(swipe.last) < look::SWIPE_END {
            return;
        }
        let width = width.max(1.0);
        let at = self.position(now, width) / width;
        let start = swipe.from.round().max(0.0) as usize;
        let turned =
            swipe.moved.abs() > look::TURN_SHARE * width || swipe.speed.abs() > look::FLICK_SPEED;
        let page = match (turned, swipe.moved > 0.0) {
            (false, _) => start,
            (true, true) => start.saturating_sub(1),
            (true, false) => start + 1,
        }
        .min(self.count - 1);
        self.swipe = None;
        self.page = page;
        let speed = -f64::from(swipe.speed / width);
        self.slide =
            Animation::with_velocity(look::PAGE_SPRING, f64::from(at), page as f64, speed, now);
    }

    /// Where the pages are, in pixels from the first one, on a screen `width` wide.
    pub fn position(&self, now: Duration, width: f32) -> f32 {
        match self.swipe {
            Some(swipe) => rubber(
                swipe.from as f32 * width - swipe.moved,
                (self.count - 1) as f32 * width,
                width,
            ),
            None => self.slide.value(now) as f32 * width,
        }
    }

    /// Whether the pages still move, or a swipe is under way.
    pub fn moving(&self, now: Duration) -> bool {
        self.swipe.is_some() || !self.slide.is_done(now, look::PAGE_PRECISION)
    }
}

/// `x` kept between 0 and `end`, the excess past either one shrunk as a rubber band stretches.
fn rubber(x: f32, end: f32, width: f32) -> f32 {
    let stretch = |excess: f32| (1.0 - 1.0 / (excess * look::RUBBER / width + 1.0)) * width;
    if x < 0.0 {
        -stretch(-x)
    } else if x > end {
        end + stretch(x - end)
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: f32 = 1000.0;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// The pages after `pager` settles, the last event at `from`.
    fn settled(pager: &mut Pager, from: Duration) -> (usize, f32) {
        pager.tick(from + look::SWIPE_END, WIDTH);
        let end = from + ms(2000);
        assert!(!pager.moving(end), "settled");
        (pager.page(), pager.position(end, WIDTH).round())
    }

    #[test]
    fn the_pages_hold_every_application() {
        assert_eq!(page_count(0, 35), 1);
        assert_eq!(page_count(35, 35), 1);
        assert_eq!(page_count(36, 35), 2);
        assert_eq!(page_count(71, 35), 3);
        assert_eq!(page_count(3, 0), 3, "one per page at least");
    }

    #[test]
    fn a_notch_turns_one_page_and_bounces_at_the_ends() {
        let mut pager = Pager::new(3);
        assert!(!pager.moving(ms(0)), "still at first");
        pager.notch(true, ms(0));
        assert_eq!(pager.page(), 1);
        // A burst of notches turns one page only.
        pager.notch(true, ms(50));
        assert_eq!(pager.page(), 1);
        pager.notch(true, ms(400));
        assert_eq!(pager.page(), 2);
        assert_eq!(settled(&mut pager, ms(400)), (2, 2.0 * WIDTH));
        // Past the last page: the pages stretch beyond it, then come back.
        pager.notch(true, ms(3000));
        assert_eq!(pager.page(), 2);
        let peak = (1..40)
            .map(|i| pager.position(ms(3000 + 10 * i), WIDTH))
            .fold(f32::MIN, f32::max);
        assert!(peak > 2.0 * WIDTH + 20.0, "stretched to {peak}");
        assert!(peak < 2.5 * WIDTH, "a bounce, not a page: {peak}");
        assert_eq!(settled(&mut pager, ms(3000)), (2, 2.0 * WIDTH));
        // And before the first.
        pager.go(0, ms(6000));
        pager.notch(false, ms(9000));
        let low = (1..40)
            .map(|i| pager.position(ms(9000 + 10 * i), WIDTH))
            .fold(f32::MAX, f32::min);
        assert!(low < -20.0, "stretched to {low}");
    }

    #[test]
    fn a_swipe_follows_the_fingers_and_settles() {
        let mut pager = Pager::new(3);
        // A slow, short swipe to the left: the pages follow, then come back.
        for i in 1..=5 {
            pager.swipe(-20.0, WIDTH, ms(100 * i));
        }
        assert_eq!(pager.position(ms(500), WIDTH), 100.0);
        pager.tick(ms(550), WIDTH);
        assert!(pager.moving(ms(550)), "still under the fingers");
        assert_eq!(settled(&mut pager, ms(650)), (0, 0.0));
        // A long one turns the page.
        for i in 1..=10 {
            pager.swipe(-30.0, WIDTH, ms(3000 + 50 * i));
        }
        assert_eq!(settled(&mut pager, ms(3500)), (1, WIDTH));
        // A short flick too.
        pager.swipe(-30.0, WIDTH, ms(6000));
        pager.swipe(-30.0, WIDTH, ms(6010));
        assert_eq!(settled(&mut pager, ms(6010)), (2, 2.0 * WIDTH));
    }

    #[test]
    fn past_the_ends_the_pages_stretch_less_and_less() {
        let mut pager = Pager::new(1);
        pager.swipe(200.0, WIDTH, ms(0));
        let one = -pager.position(ms(0), WIDTH);
        pager.swipe(200.0, WIDTH, ms(10));
        let two = -pager.position(ms(10), WIDTH);
        assert!(one > 0.0 && one < 200.0, "{one}");
        assert!(two - one < one, "{one} then {two}");
        assert_eq!(settled(&mut pager, ms(10)), (0, 0.0));
    }

    #[test]
    fn a_new_search_starts_on_the_first_page() {
        let mut pager = Pager::new(4);
        pager.go(3, ms(0));
        pager.reset(2);
        assert_eq!((pager.page(), pager.count()), (0, 2));
        assert!(!pager.moving(ms(0)));
        pager.go(9, ms(0));
        assert_eq!(pager.page(), 1, "kept within the pages");
    }
}
