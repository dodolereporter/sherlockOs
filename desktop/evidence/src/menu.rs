//! The right-click menu coming and going, and where its submenu opens (lot E-01). The menu
//! unfolds from its corner like the bar's panels: a spring from 0 (folded) to 1 (in place) for
//! its scale, and a fade. It folds back the same way when it closes, its last entries drawn
//! until it has faded out. No iced here: the view gives these values to
//! `sherlock_ui::animated::Reveal`.

use std::time::Duration;

use iced::{Point, Size};
use sherlock_ui::motion::{Animation, Fade, SCALE_PRECISION};
use sherlock_ui::{metrics, panels};

use crate::app::{App, Menu, MenuItem, Sub};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuMotion {
    spring: Animation,
    fade: Fade,
}

impl Default for MenuMotion {
    /// In place, at rest.
    fn default() -> Self {
        Self {
            spring: Animation::new(panels::OPEN_SPRING, 1.0, 1.0, Duration::ZERO),
            fade: Fade::new(1.0, 1.0, Duration::ZERO, Duration::ZERO),
        }
    }
}

impl MenuMotion {
    /// Folded at `now`, unfolding.
    pub fn opening(now: Duration) -> Self {
        Self {
            spring: Animation::new(panels::OPEN_SPRING, 0.0, 1.0, now),
            fade: Fade::new(0.0, 1.0, now, panels::OPEN_FADE),
        }
    }

    /// Folds back from where it is.
    pub fn close(&mut self, now: Duration) {
        if self.fade.target() != 0.0 {
            self.spring.retarget(now, 0.0);
            self.fade.retarget(now, 0.0, panels::OPEN_FADE);
        }
    }

    /// Folded and faded out: a closing menu can go.
    pub fn gone(&self, now: Duration) -> bool {
        self.fade.target() == 0.0 && self.fade.is_done(now)
    }

    /// Frames are needed.
    pub fn running(&self, now: Duration) -> bool {
        let folding = self.fade.target() == 0.0;
        let moving = !self.spring.is_done(now, SCALE_PRECISION);
        !self.fade.is_done(now) || (moving && !folding)
    }

    pub fn opacity(&self, now: Duration) -> f32 {
        self.fade.value(now).clamp(0.0, 1.0)
    }

    /// The scale of the menu around its top left corner: `OPEN_SCALE` folded, 1 in place.
    pub fn scale(&self, now: Duration) -> f32 {
        let folded = 1.0 - self.spring.value(now) as f32;
        1.0 - folded * (1.0 - panels::OPEN_SCALE)
    }
}

/// A menu closing: what it showed, drawn while it fades out.
#[derive(Debug, Clone, PartialEq)]
pub struct Closing {
    pub menu: Menu,
    pub items: Vec<MenuItem>,
    pub sub: Vec<MenuItem>,
}

/// Width of a submenu: « Ouvrir avec » is wider, for application names and the switch.
pub fn submenu_width(sub: Option<Sub>) -> f32 {
    match sub {
        Some(Sub::OpenWith) => metrics::OPEN_WITH_WIDTH,
        _ => metrics::CONTEXT_SUBMENU_WIDTH,
    }
}

/// Top left corner of the submenu of `menu`, beside its entry: on the right, or on the left
/// when the window has no room there, and moved up to stay inside the window.
pub fn submenu_at(menu: &Menu, items: &[MenuItem], sub: &[MenuItem], window: Size) -> Point {
    let Some(entry) = menu
        .submenu
        .and_then(|s| items.iter().position(|i| *i == MenuItem::Submenu(s)))
    else {
        return menu.at;
    };
    let edge = 2.0 * (metrics::MENU_PADDING + metrics::BORDER_WIDTH);
    let y = menu.at.y + App::menu_height(&items[..entry]) - edge;
    let width = submenu_width(menu.submenu);
    let right = menu.at.x + metrics::CONTEXT_MENU_WIDTH + metrics::SUBMENU_GAP;
    let x = if right + width <= window.width {
        right
    } else {
        (menu.at.x - metrics::SUBMENU_GAP - width).max(0.0)
    };
    let y = y.min(window.height - App::menu_height(sub)).max(0.0);
    Point::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_menu_unfolds_then_folds_back_and_goes() {
        let mut motion = MenuMotion::opening(ms(100));
        assert_eq!(motion.opacity(ms(100)), 0.0);
        assert!((motion.scale(ms(100)) - panels::OPEN_SCALE).abs() < 1e-6);
        assert!(motion.running(ms(150)));
        let halfway = motion.opacity(ms(150));
        assert!(halfway > 0.0 && halfway < 1.0);
        assert!(!motion.running(ms(900)), "no frame at rest");
        assert_eq!(motion.opacity(ms(900)), 1.0);
        assert!((motion.scale(ms(900)) - 1.0).abs() < 1e-3);
        motion.close(ms(900));
        assert!(!motion.gone(ms(950)) && motion.running(ms(950)));
        assert!(motion.opacity(ms(950)) < 1.0);
        assert!(motion.gone(ms(900) + panels::OPEN_FADE));
        assert!(!MenuMotion::default().running(Duration::ZERO));
    }

    #[test]
    fn the_submenu_opens_on_the_left_near_the_right_edge() {
        let items = [
            MenuItem::Separator,
            MenuItem::Submenu(Sub::SortBy),
            MenuItem::Submenu(Sub::View),
        ];
        let sub = [MenuItem::Separator; 3];
        let mut menu = Menu {
            at: Point::new(100.0, 100.0),
            on_selection: false,
            submenu: Some(Sub::View),
            highlight: None,
        };
        let window = Size::new(1300.0, 676.0);
        let at = submenu_at(&menu, &items, &sub, window);
        assert_eq!(
            at.x,
            100.0 + metrics::CONTEXT_MENU_WIDTH + metrics::SUBMENU_GAP
        );
        // Level with its entry, the third: below a separator and an entry.
        let edge = 2.0 * (metrics::MENU_PADDING + metrics::BORDER_WIDTH);
        assert_eq!(at.y, 100.0 + App::menu_height(&items[..2]) - edge);
        menu.at.x = 1300.0 - metrics::CONTEXT_MENU_WIDTH;
        let at = submenu_at(&menu, &items, &sub, window);
        assert!(
            at.x + submenu_width(menu.submenu) <= menu.at.x,
            "on the left"
        );
        // Near the bottom, it moves up to stay inside.
        menu.at.y = 660.0;
        let at = submenu_at(&menu, &items, &sub, window);
        assert!(at.y + App::menu_height(&sub) <= 676.0);
    }
}
