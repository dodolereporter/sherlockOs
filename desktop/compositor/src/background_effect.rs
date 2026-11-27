//! What clients ask to have blurred behind their surfaces (lot D-37, ADR 0027):
//! `ext_background_effect_v1`. The region a surface gives, in its own coordinates, applies at
//! its commit; `blur.rs` draws the blur behind it. See "Blur behind glass" in
//! `docs/compositor.md`.

use std::sync::atomic::{AtomicBool, Ordering};

use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    reexports::{
        wayland_protocols::ext::background_effect::v1::server::{
            ext_background_effect_manager_v1::{self as manager, ExtBackgroundEffectManagerV1},
            ext_background_effect_surface_v1::{self as effect, ExtBackgroundEffectSurfaceV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            backend::ClientId, protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Rectangle},
    wayland::compositor::{
        Cacheable, RectangleKind, RegionAttributes, get_region_attributes, with_states,
    },
};

use crate::State;

pub(crate) fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, ExtBackgroundEffectManagerV1, ()>(1, ());
}

/// The blur region of a surface, double-buffered: the pending one applies at its commit.
#[derive(Debug, Clone, Default)]
struct BlurRegion(Option<RegionAttributes>);

impl Cacheable for BlurRegion {
    fn commit(&mut self, _: &DisplayHandle) -> Self {
        self.clone()
    }

    fn merge_into(self, into: &mut Self, _: &DisplayHandle) {
        *into = self;
    }
}

/// The surface has its `ext_background_effect_surface_v1`.
#[derive(Default)]
struct HasEffect(AtomicBool);

fn set_pending(surface: &WlSurface, region: Option<RegionAttributes>) {
    with_states(surface, |states| {
        *states.cached_state.get::<BlurRegion>().pending() = BlurRegion(region);
    });
}

/// The region of `surface` to blur, in its own coordinates: disjoint rectangles inside it.
pub fn region(surface: &WlSurface) -> Vec<Rectangle<i32, Logical>> {
    let size = with_renderer_surface_state(surface, |s| s.surface_size()).flatten();
    let Some(size) = size else {
        return Vec::new();
    };
    let current = with_states(surface, |states| {
        states.cached_state.get::<BlurRegion>().current().0.clone()
    });
    current.map_or_else(Vec::new, |r| resolve(&r, Rectangle::from_size(size)))
}

/// The rectangles added and taken away by a region, as disjoint rectangles inside `bounds`.
fn resolve(
    region: &RegionAttributes,
    bounds: Rectangle<i32, Logical>,
) -> Vec<Rectangle<i32, Logical>> {
    let mut rects: Vec<Rectangle<i32, Logical>> = Vec::new();
    for (kind, rect) in &region.rects {
        let Some(rect) = rect.intersection(bounds) else {
            continue;
        };
        rects = match kind {
            RectangleKind::Add => {
                let new = rect.subtract_rects(rects.iter().copied());
                rects.into_iter().chain(new).collect()
            }
            RectangleKind::Subtract => rects
                .into_iter()
                .flat_map(|r| r.subtract_rect(rect))
                .collect(),
        };
    }
    rects
}

impl GlobalDispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ExtBackgroundEffectManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        // Blur is always offered: where it cannot be drawn, the veil stands for it.
        let manager = data_init.init(resource, ());
        manager.capabilities(manager::Capability::Blur);
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        manager: &ExtBackgroundEffectManagerV1,
        request: manager::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let manager::Request::GetBackgroundEffect { id, surface } = request {
            let taken = with_states(&surface, |states| {
                let has = states.data_map.get_or_insert_threadsafe(HasEffect::default);
                has.0.swap(true, Ordering::Relaxed)
            });
            if taken {
                let message = "the surface already has a background effect";
                manager.post_error(manager::Error::BackgroundEffectExists, message);
                return;
            }
            data_init.init(id, surface);
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, WlSurface> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        resource: &ExtBackgroundEffectSurfaceV1,
        request: effect::Request,
        surface: &WlSurface,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            effect::Request::SetBlurRegion { region } if surface.is_alive() => {
                set_pending(surface, region.as_ref().map(get_region_attributes));
            }
            effect::Request::SetBlurRegion { .. } => {
                let message = "the surface was destroyed";
                resource.post_error(effect::Error::SurfaceDestroyed, message);
            }
            // The region goes at the next commit.
            effect::Request::Destroy if surface.is_alive() => set_pending(surface, None),
            _ => {}
        }
    }

    fn destroyed(_: &mut Self, _: ClientId, _: &ExtBackgroundEffectSurfaceV1, surface: &WlSurface) {
        if surface.is_alive() {
            with_states(surface, |states| {
                if let Some(has) = states.data_map.get::<HasEffect>() {
                    has.0.store(false, Ordering::Relaxed);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_region_is_cut_to_the_surface_and_its_holes_taken_away() {
        let region = RegionAttributes {
            rects: vec![
                (RectangleKind::Add, rect(0, 0, 100, 40)),
                (RectangleKind::Add, rect(50, 20, 100, 40)),
                (RectangleKind::Subtract, rect(10, 10, 10, 10)),
            ],
        };
        let rects = resolve(&region, rect(0, 0, 120, 50));
        let area: i32 = rects.iter().map(|r| r.size.w * r.size.h).sum();
        // 100 × 40, then what the second adds inside the surface, minus the hole.
        assert_eq!(area, 100 * 40 + 20 * 30 + 50 * 10 - 10 * 10);
        assert!(rects.iter().all(|r| rect(0, 0, 120, 50).contains_rect(*r)));
        assert!(!rects.iter().any(|r| r.contains((15, 15))));
    }
}
