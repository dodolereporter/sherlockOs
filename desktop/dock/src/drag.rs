//! A press on an icon: a click when the pointer stays, a drag of a pinned application or of
//! Sherlock when it goes past `dock::DRAG_THRESHOLD`. Dragged along the dock, the icon takes
//! the place nearest the pointer among the pinned ones; dragged out of the dock and let go, it
//! is unpinned.

use iced::{Event, Point, event, mouse, window};
use sherlock_ui::dock;

use crate::items::Target;

/// The left button is held since it went down on an icon.
#[derive(Debug, Clone, PartialEq)]
pub struct Press {
    /// The icon, by identity: its place changes while it is dragged.
    pub target: Target,
    /// Where the button went down.
    pub from: Point,
    pub pointer: Point,
    /// The icon follows the pointer.
    pub dragging: bool,
    /// The icon is out of the dock: letting it go unpins it.
    pub out: bool,
    /// The pinned list when the button went down.
    pub pinned: Vec<String>,
    /// The pinned applications shown, in order, when the button went down: the file is
    /// written only if this order changed.
    pub shown: Vec<String>,
}

impl Press {
    pub fn new(target: Target, at: Point, pinned: Vec<String>, shown: Vec<String>) -> Self {
        Self {
            target,
            from: at,
            pointer: at,
            dragging: false,
            out: false,
            pinned,
            shown,
        }
    }

    /// Whether the pointer went far enough from where the button went down to drag.
    pub fn past_threshold(&self) -> bool {
        self.from.distance(self.pointer) >= dock::DRAG_THRESHOLD
    }
}

/// What the pointer does while the button is held.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    Moved(Point),
    Released,
}

/// The pointer events of a press, wherever the pointer is: while the button is held, the
/// surface keeps the pointer past its edges.
pub fn gesture(event: Event, _: event::Status, _: window::Id) -> Option<Gesture> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Gesture::Moved(position)),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Gesture::Released),
        _ => None,
    }
}

/// The orders the first `pinned` icons can take with the icon at `from` moved to each place
/// among the pinned ones, Sherlock included, the `fixed` first icons (the « Applications »
/// button) staying first. Each order lists the current places of the icons.
pub fn orders(fixed: usize, pinned: usize, from: usize) -> Vec<Vec<usize>> {
    let others: Vec<usize> = (fixed..pinned).filter(|&i| i != from).collect();
    (0..=others.len())
        .map(|place| {
            let mut order = others.clone();
            order.insert(place, from);
            (0..fixed).chain(order).collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dragged_icon_takes_every_place() {
        // Files, Terminal, Sherlock, Editor; the terminal is dragged, past Sherlock too.
        assert_eq!(
            orders(0, 4, 1),
            [
                vec![1, 0, 2, 3],
                vec![0, 1, 2, 3],
                vec![0, 2, 1, 3],
                vec![0, 2, 3, 1],
            ]
        );
        // Sherlock moves like the others; the « Applications » button stays first.
        assert_eq!(
            orders(1, 4, 2),
            [vec![0, 2, 1, 3], vec![0, 1, 2, 3], vec![0, 1, 3, 2]]
        );
    }

    #[test]
    fn a_small_move_is_still_a_click() {
        let mut press = Press::new(
            Target::Sherlock,
            Point::new(10.0, 10.0),
            Vec::new(),
            Vec::new(),
        );
        press.pointer = Point::new(12.0, 12.0);
        assert!(!press.past_threshold());
        press.pointer = Point::new(10.0, 10.0 + dock::DRAG_THRESHOLD);
        assert!(press.past_threshold());
    }
}
