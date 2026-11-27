//! A fullscreen window covers the top layer of its screen, as on macOS (lot D-21e): the bar,
//! the dock and the notifications go behind it. Only the overlay layer (on-screen displays,
//! urgent notifications, the lock screen) stays in front. The pointer resting against an edge
//! for `EDGE_REVEAL_DELAY` brings the surfaces anchored along that edge back over the window:
//! they slide in, and slide out once the pointer has left them and their menus are closed.
//!
//! A surface partly beyond its screen hides by itself (the hidden dock of lot D-18c2): the band
//! it keeps at the edge still takes the pointer over the window, and its client brings it back
//! with its own slide, which the reveal does not add to.

use std::time::Duration;

use sherlock_ui::motion::{self, Fade};
use smithay::{
    desktop::{
        LayerMap, LayerSurface, PopupManager, Window, WindowSurfaceType, layer_map_for_output,
    },
    output::Output,
    reexports::{
        calloop::{
            RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, Size},
    wayland::shell::wlr_layer::{Anchor, Layer},
};

use crate::{State, handlers::is_fullscreen, input::Hit};

/// An edge of a screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    const ALL: [Self; 4] = [Self::Top, Self::Bottom, Self::Left, Self::Right];

    /// The edge a surface anchored by `anchor` lies along: anchored to it and not to the
    /// opposite one, and to both sides across it or to neither. A corner surface (the
    /// notifications) or one stretched between two opposite edges has none.
    pub fn of(anchor: Anchor) -> Option<Self> {
        Self::ALL.into_iter().find(|edge| {
            let (own, opposite, across) = edge.anchors();
            let across = anchor & across;
            anchor.contains(own)
                && !anchor.intersects(opposite)
                && (across.is_empty() || across == edge.anchors().2)
        })
    }

    /// Its anchor, the opposite one, and the two across it.
    fn anchors(self) -> (Anchor, Anchor, Anchor) {
        let (horizontal, vertical) = (Anchor::LEFT | Anchor::RIGHT, Anchor::TOP | Anchor::BOTTOM);
        match self {
            Self::Top => (Anchor::TOP, Anchor::BOTTOM, horizontal),
            Self::Bottom => (Anchor::BOTTOM, Anchor::TOP, horizontal),
            Self::Left => (Anchor::LEFT, Anchor::RIGHT, vertical),
            Self::Right => (Anchor::RIGHT, Anchor::LEFT, vertical),
        }
    }

    /// The pointer at `at`, in the coordinates of a screen of `size`, is against this edge:
    /// on its last row or column of points.
    pub fn touched(self, at: Point<f64, Logical>, size: Size<i32, Logical>) -> bool {
        let (w, h) = (f64::from(size.w), f64::from(size.h));
        match self {
            Self::Top => at.y < 1.0,
            Self::Bottom => at.y >= h - 1.0,
            Self::Left => at.x < 1.0,
            Self::Right => at.x >= w - 1.0,
        }
    }

    /// What a surface at `rect` covers from this edge of a screen of `size`: the band from the
    /// edge to its far side, as wide as the surface. The pointer there keeps it out.
    pub fn reach(
        self,
        rect: Rectangle<i32, Logical>,
        size: Size<i32, Logical>,
    ) -> Rectangle<i32, Logical> {
        let (loc, end) = (rect.loc, rect.loc + rect.size);
        match self {
            Self::Top => Rectangle::new((loc.x, 0).into(), (rect.size.w, end.y).into()),
            Self::Bottom => {
                Rectangle::new((loc.x, loc.y).into(), (rect.size.w, size.h - loc.y).into())
            }
            Self::Left => Rectangle::new((0, loc.y).into(), (end.x, rect.size.h).into()),
            Self::Right => {
                Rectangle::new((loc.x, loc.y).into(), (size.w - loc.x, rect.size.h).into())
            }
        }
    }

    /// The shift of a surface whose reach is `reach`, shown at `shown` (0 out of sight, 1 in
    /// place): out through this edge.
    fn shift(self, reach: Rectangle<i32, Logical>, shown: f64) -> Point<f64, Logical> {
        let hidden = 1.0 - shown.clamp(0.0, 1.0);
        let (w, h) = (f64::from(reach.size.w), f64::from(reach.size.h));
        match self {
            Self::Top => (0.0, -h * hidden),
            Self::Bottom => (0.0, h * hidden),
            Self::Left => (-w * hidden, 0.0),
            Self::Right => (w * hidden, 0.0),
        }
        .into()
    }
}

/// The surfaces along an edge, brought back over a fullscreen window.
#[derive(Debug)]
struct Reveal {
    output: Output,
    edge: Edge,
    /// Those the reveal slides: the ones fully on the screen when it started.
    sliding: Vec<WlSurface>,
    /// From 0, out of sight, to 1, in place; back to 0 as they leave.
    shown: Fade,
}

impl Reveal {
    fn leaving(&self) -> bool {
        self.shown.target() == 0.0
    }
}

/// The edge where the pointer rests and the reveals in progress.
#[derive(Debug, Default)]
pub struct Reveals {
    /// The screen and edge where the pointer rests, since when, and the timer that ends the
    /// wait (`None` once it has run).
    resting: Option<(Output, Edge, Duration)>,
    timer: Option<RegistrationToken>,
    shown: Vec<Reveal>,
}

impl State {
    /// The frontmost fullscreen window of `output`, which covers its top layer. None while
    /// Mission Control is open: it shows the bar over its windows.
    pub fn fullscreen_on(&self, output: &Output) -> Option<Window> {
        if self.overview_open() {
            return None;
        }
        self.space
            .elements()
            .rev()
            .filter(|w| is_fullscreen(w) && self.is_visible(w))
            .find(|w| self.window_output(w).as_ref() == Some(output))
            .cloned()
    }

    /// Where `surface`, of the top layer of `output` (whose layers are `map`), goes: in front
    /// of the windows, shifted by its slide (`Some`), or behind the fullscreen window that
    /// covers the screen (`None`, `covered`). `input`: for the pointer, which the band of a
    /// surface hiding by itself takes in front all the same.
    pub(crate) fn top_shift(
        &self,
        (output, map, covered): (&Output, &LayerMap, bool),
        surface: &LayerSurface,
        input: bool,
        now: Duration,
    ) -> Option<Point<f64, Logical>> {
        if !covered {
            return Some(Point::default());
        }
        let along = along(map, surface);
        let size = self.space.output_geometry(output).map(|g| g.size);
        let reveal = along.and_then(|(edge, _)| {
            self.reveals
                .shown
                .iter()
                .find(|r| &r.output == output && r.edge == edge)
        });
        match (reveal, along, size) {
            (Some(reveal), Some((edge, rect)), Some(size)) => {
                let slides = reveal.sliding.contains(surface.wl_surface());
                let shown = if slides {
                    f64::from(reveal.shown.value(now))
                } else {
                    1.0
                };
                Some(edge.shift(edge.reach(rect, size), shown))
            }
            (None, Some((_, rect)), Some(size)) if input && !on_screen(rect, size) => {
                Some(Point::default())
            }
            _ => None,
        }
    }

    /// The surface of the top layer under `pos`: among those in front of the windows
    /// (`front`), or those behind a fullscreen window.
    pub(crate) fn top_hit(&self, pos: Point<f64, Logical>, front: bool) -> Option<Hit> {
        let output = self.space.output_under(pos).next()?;
        let origin = self.space.output_geometry(output)?.loc;
        let covered = self.fullscreen_on(output).is_some();
        let (map, now) = (layer_map_for_output(output), self.now());
        let local = pos - origin.to_f64();
        map.layers_on(Layer::Top).rev().find_map(|surface| {
            let shift = self.top_shift((output, &map, covered), surface, true, now);
            if shift.is_some() != front {
                return None;
            }
            let geometry = map.layer_geometry(surface)?;
            let loc = (geometry.loc - surface.bbox().loc).to_f64() + shift.unwrap_or_default();
            let (hit, offset) = surface.surface_under(local - loc, WindowSurfaceType::ALL)?;
            Some(Hit::Layer(
                surface.wl_surface().clone(),
                hit,
                origin.to_f64() + loc + offset.to_f64(),
            ))
        })
    }

    /// A reveal slides on `output` at `now`: a frame is due.
    pub(crate) fn reveal_sliding(&self, output: &Output, now: Duration) -> bool {
        self.reveals
            .shown
            .iter()
            .any(|r| &r.output == output && !r.shown.is_done(now))
    }

    /// The edge of a covered screen the pointer at `pos` is against, with surfaces along it.
    fn touched_edge(&self, pos: Point<f64, Logical>) -> Option<(Output, Edge)> {
        let output = self.space.output_under(pos).next()?;
        let geometry = self.space.output_geometry(output)?;
        self.fullscreen_on(output)?;
        let map = layer_map_for_output(output);
        let edges: Vec<_> = map
            .layers_on(Layer::Top)
            .filter_map(|s| Edge::of(s.cached_state().anchor))
            .collect();
        let local = pos - geometry.loc.to_f64();
        Edge::ALL
            .into_iter()
            .find(|e| edges.contains(e) && e.touched(local, geometry.size))
            .map(|edge| (output.clone(), edge))
    }

    /// Whether the surfaces of `reveal` stay out: their screen is still covered, and the
    /// pointer at `pos` is against their edge or over them, or one of them has a menu open.
    fn keeps(&self, reveal: &Reveal, pos: Point<f64, Logical>) -> bool {
        let Some(geometry) = self.space.output_geometry(&reveal.output) else {
            return false;
        };
        if self.fullscreen_on(&reveal.output).is_none() {
            return false;
        }
        let local = pos - geometry.loc.to_f64();
        if geometry.to_f64().contains(pos) && reveal.edge.touched(local, geometry.size) {
            return true;
        }
        let map = layer_map_for_output(&reveal.output);
        map.layers_on(Layer::Top).any(|surface| {
            along(&map, surface).is_some_and(|(edge, rect)| {
                edge == reveal.edge
                    && (edge.reach(rect, geometry.size).to_f64().contains(local)
                        || PopupManager::popups_for_surface(surface.wl_surface())
                            .next()
                            .is_some())
            })
        })
    }

    /// Follows the pointer against the edges of covered screens: once at each turn of the
    /// loop, and when the wait at an edge ends.
    pub(crate) fn refresh_reveals(&mut self) {
        let Some(pos) = self.seat.get_pointer().map(|p| p.current_location()) else {
            return;
        };
        let now = self.now();
        let mut changed = false;
        // The pointer comes against an edge, or leaves it: the wait starts again.
        let touched = self.touched_edge(pos);
        let resting = self
            .reveals
            .resting
            .as_ref()
            .map(|(o, e, _)| (o.clone(), *e));
        if resting != touched {
            if let Some(token) = self.reveals.timer.take() {
                self.loop_handle.remove(token);
            }
            let rests = touched.is_some();
            self.reveals.resting = touched.map(|(o, e)| (o, e, now));
            if rests {
                let timer = Timer::from_duration(motion::EDGE_REVEAL_DELAY);
                let token = self.loop_handle.insert_source(timer, |_, _, state| {
                    state.reveals.timer = None;
                    state.refresh_reveals();
                    TimeoutAction::Drop
                });
                self.reveals.timer = token.ok();
            }
        }
        // The rest is long enough: the surfaces of the edge come, or come back.
        if let Some((output, edge, since)) = self.reveals.resting.clone()
            && now >= since + motion::EDGE_REVEAL_DELAY
        {
            changed |= self.reveal(output, edge, now);
        }
        // Those the pointer left go.
        let leave: Vec<_> = (0..self.reveals.shown.len())
            .filter(|&i| {
                let reveal = &self.reveals.shown[i];
                !reveal.leaving() && !self.keeps(reveal, pos)
            })
            .collect();
        for i in leave {
            let reveal = &mut self.reveals.shown[i];
            reveal.shown.retarget(now, 0.0, motion::EDGE_REVEAL);
            let (output, edge) = (reveal.output.clone(), reveal.edge);
            self.give_keyboard_back(&output, edge);
            changed = true;
        }
        // Gone, or their screen no longer covered: forgotten.
        let before = self.reveals.shown.len();
        let covered: Vec<bool> = (self.reveals.shown.iter())
            .map(|r| self.fullscreen_on(&r.output).is_some())
            .collect();
        let mut covered = covered.into_iter();
        self.reveals
            .shown
            .retain(|r| covered.next().unwrap_or(false) && !(r.leaving() && r.shown.is_done(now)));
        changed |= self.reveals.shown.len() != before;
        if changed {
            self.queue_redraw();
        }
    }

    /// Brings the surfaces along `edge` of `output` out, or back if they were leaving. Whether
    /// anything changed.
    fn reveal(&mut self, output: Output, edge: Edge, now: Duration) -> bool {
        if let Some(reveal) =
            (self.reveals.shown.iter_mut()).find(|r| r.output == output && r.edge == edge)
        {
            if !reveal.leaving() {
                return false;
            }
            reveal.shown.retarget(now, 1.0, motion::EDGE_REVEAL);
            return true;
        }
        let size = self.space.output_geometry(&output).map(|g| g.size);
        let map = layer_map_for_output(&output);
        let sliding = map
            .layers_on(Layer::Top)
            .filter(|s| {
                along(&map, s).is_some_and(|(e, rect)| {
                    e == edge && size.is_some_and(|size| on_screen(rect, size))
                })
            })
            .map(|s| s.wl_surface().clone())
            .collect();
        drop(map);
        tracing::debug!(
            ?edge,
            "the pointer rests at the edge: its surfaces come out"
        );
        self.reveals.shown.push(Reveal {
            output,
            edge,
            sliding,
            shown: Fade::new(0.0, 1.0, now, motion::EDGE_REVEAL),
        });
        true
    }

    /// The keyboard was on a surface along `edge` of `output`, which goes back behind the
    /// fullscreen window: the window takes it back.
    fn give_keyboard_back(&mut self, output: &Output, edge: Edge) {
        let Some(focus) = self.keyboard_focus() else {
            return;
        };
        let map = layer_map_for_output(output);
        let along = map
            .layer_for_surface(&focus, WindowSurfaceType::TOPLEVEL)
            .is_some_and(|s| {
                s.layer() == Layer::Top && along(&map, s).is_some_and(|(e, _)| e == edge)
            });
        drop(map);
        if along {
            self.focus(self.top_window());
        }
    }
}

/// The edge `surface` lies along, and where it is on its screen, whose layers are `map`.
fn along(map: &LayerMap, surface: &LayerSurface) -> Option<(Edge, Rectangle<i32, Logical>)> {
    let geometry = map.layer_geometry(surface)?;
    Some((Edge::of(surface.cached_state().anchor)?, geometry))
}

/// `rect` lies within a screen of `size`, in its coordinates.
fn on_screen(rect: Rectangle<i32, Logical>, size: Size<i32, Logical>) -> bool {
    Rectangle::from_size(size).contains_rect(rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_surface_lies_along_an_edge_unless_in_a_corner() {
        let bar = Anchor::TOP | Anchor::LEFT | Anchor::RIGHT;
        assert_eq!(Edge::of(bar), Some(Edge::Top));
        assert_eq!(Edge::of(Anchor::BOTTOM), Some(Edge::Bottom));
        assert_eq!(Edge::of(Anchor::LEFT), Some(Edge::Left));
        // The notifications, in a corner, and a surface across the whole height.
        assert_eq!(Edge::of(Anchor::TOP | Anchor::RIGHT), None);
        assert_eq!(Edge::of(Anchor::TOP | Anchor::BOTTOM), None);
        assert_eq!(Edge::of(Anchor::empty()), None);
    }

    #[test]
    fn a_surface_slides_out_through_its_edge_by_its_reach() {
        let screen = Size::from((800, 600));
        // The bar: 36 high, 10 below the top edge.
        let bar = Rectangle::new((12, 10).into(), (776, 36).into());
        let reach = Edge::Top.reach(bar, screen);
        assert_eq!(reach, Rectangle::new((12, 0).into(), (776, 46).into()));
        assert_eq!(Edge::Top.shift(reach, 0.0), Point::from((0.0, -46.0)));
        assert_eq!(Edge::Top.shift(reach, 1.0), Point::from((0.0, 0.0)));
        // The dock, 76 high at the bottom, slides down.
        let dock = Rectangle::new((300, 524).into(), (200, 76).into());
        let reach = Edge::Bottom.reach(dock, screen);
        assert_eq!(Edge::Bottom.shift(reach, 0.5), Point::from((0.0, 38.0)));
        assert!(Edge::Bottom.touched((400.0, 599.5).into(), screen));
        assert!(!Edge::Bottom.touched((400.0, 590.0).into(), screen));
        assert!(Edge::Top.touched((400.0, 0.0).into(), screen));
    }
}
