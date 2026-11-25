//! Anchored surfaces (`zwlr_layer_shell_v1`): the bar, the dock, the launcher and the
//! notifications are separate programs anchored to a screen edge (ADR 0005).
//!
//! Each screen has its layer map (smithay's `LayerMap`), which places its surfaces from their
//! anchors and margins and keeps what their exclusive zones leave to windows: the work area.
//! Background and bottom surfaces are drawn under the windows, top and overlay ones above.
//!
//! Values from clients are checked before smithay places anything: a surface whose size,
//! margins or exclusive zone do not fit its screen is closed (see `check_reserves`).

use std::cell::Cell;

use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    desktop::{LayerSurface, PopupKind, WindowSurfaceType, layer_map_for_output},
    output::Output,
    reexports::{
        wayland_protocols_wlr::layer_shell::v1::server::{
            zwlr_layer_shell_v1::ZwlrLayerShellV1,
            zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, Resource,
            backend::ClientId,
            delegate_dispatch, delegate_global_dispatch,
            protocol::{wl_output::WlOutput, wl_surface::WlSurface},
        },
    },
    utils::IsAlive,
    utils::{Logical, Point, Rectangle, Size},
    wayland::shell::{
        wlr_layer::{
            Anchor, ExclusiveZone, KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface,
            LayerSurfaceCachedState, WlrLayerShellGlobalData, WlrLayerShellHandler,
            WlrLayerShellState, WlrLayerSurfaceUserData,
        },
        xdg::PopupSurface,
    },
};

use crate::{State, focus::KeyboardFocus, input::Hit, window_data};

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    /// A surface without a screen of its own goes to the one under the pointer. It is placed
    /// at its first commit, when its anchors and size are known.
    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let Some(output) = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.pointer_output())
        else {
            surface.send_close();
            return;
        };
        tracing::debug!(namespace, output = output.name(), "anchored surface");
        let mapped =
            layer_map_for_output(&output).map_layer(&LayerSurface::new(surface, namespace));
        if let Err(err) = mapped {
            tracing::warn!("anchored surface refused: {err}");
        }
    }

    /// A menu of an anchored surface (the menus of the bar). It is kept inside the screen.
    fn new_popup(&mut self, parent: WlrLayerSurface, popup: PopupSurface) {
        let Some((output, layer)) = self.layer_for(parent.wl_surface()) else {
            return;
        };
        let Some(area) = self.space.output_geometry(&output) else {
            return;
        };
        // The target is the screen, seen from the parent surface.
        let target = Rectangle::new(area.loc - layer, area.size);
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let surface = surface.wl_surface();
        self.unmapped_layers
            .retain(|(l, _)| l.wl_surface() != surface);
        if let Some((output, _)) = self.layer_for(surface) {
            let mut map = layer_map_for_output(&output);
            let layer = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .cloned();
            if let Some(layer) = layer {
                map.unmap_layer(&layer);
            }
            drop(map);
            self.reflow(&output);
        }
        // The keyboard goes back to a window if this surface had it.
        let had_keyboard = self
            .keyboard_focus()
            .is_none_or(|focus| !focus.alive() || &focus == surface);
        if had_keyboard {
            self.focus(self.top_window());
        }
    }
}

impl State {
    /// Screen of an anchored surface, and its place in the space.
    pub(crate) fn layer_for(&self, surface: &WlSurface) -> Option<(Output, Point<i32, Logical>)> {
        self.space.outputs().find_map(|output| {
            let map = layer_map_for_output(output);
            let layer = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)?;
            let loc = map.layer_geometry(layer)?.loc - layer.bbox().loc;
            Some((
                output.clone(),
                loc + self.space.output_geometry(output)?.loc,
            ))
        })
    }

    /// An anchored surface committed. Its values are checked, then its screen places it again
    /// and it gets its first configure. A null buffer unmaps it until its next commit, which
    /// maps it again with a new configure. When the exclusive zone changes, maximized and
    /// snapped windows follow, and the keyboard follows what the surface accepts.
    pub(crate) fn layer_commit(&mut self, surface: &WlSurface) -> bool {
        let (output, layer, remapped) = match self.layer_for(surface) {
            Some((output, _)) => {
                let map = layer_map_for_output(&output);
                let layer = map
                    .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                    .cloned();
                drop(map);
                let Some(layer) = layer else {
                    return false;
                };
                (output, layer, false)
            }
            None => {
                let Some(i) = self
                    .unmapped_layers
                    .iter()
                    .position(|(l, _)| l.wl_surface() == surface)
                else {
                    return false;
                };
                let (layer, output) = self.unmapped_layers.swap_remove(i);
                (output, layer, true)
            }
        };
        let Some(screen) = self.space.output_geometry(&output) else {
            return true;
        };
        let mut map = layer_map_for_output(&output);
        let before = map.non_exclusive_zone();
        let others = map
            .layers()
            .filter(|l| *l != &layer)
            .map(LayerSurface::cached_state);
        let checked = check_reserves(others.chain([layer.cached_state()]), screen.size);
        let shown = layer.user_data().get_or_insert(|| Cell::new(false));
        let content = has_content(surface);
        if let Err(reason) = checked {
            tracing::warn!(
                namespace = layer.namespace(),
                reason,
                "anchored surface refused"
            );
            map.unmap_layer(&layer);
            layer.layer_surface().send_close();
        } else if shown.get() && !content {
            // A null buffer: unmapped, until its next commit.
            shown.set(false);
            map.unmap_layer(&layer);
            self.unmapped_layers.push((layer.clone(), output.clone()));
        } else {
            shown.set(content);
            if remapped && let Err(err) = map.map_layer(&layer) {
                tracing::warn!("anchored surface: {err}");
            }
            map.arrange();
            if remapped {
                layer.layer_surface().send_configure();
            } else {
                layer.layer_surface().send_pending_configure();
            }
        }
        let zone_changed = before != map.non_exclusive_zone();
        drop(map);
        if zone_changed {
            self.reflow(&output);
        }
        // An exclusive keyboard surface takes the keyboard, a surface that no longer wants it
        // (or is gone) gives it back.
        let focused = self.keyboard_focus().as_ref() == Some(surface);
        if (self.exclusive_layer().as_ref() == Some(surface) && !focused)
            || (focused && !self.layer_wants_keyboard(surface))
        {
            self.focus(self.focused_window().or_else(|| self.top_window()));
        }
        true
    }

    /// Maximized, fullscreen and snapped windows of `output` take its new geometry, Wayland
    /// or X11, including those still waiting for their first content: a window maximized at
    /// startup, before the bar arrives, must not stay under it.
    pub(crate) fn reflow(&mut self, output: &Output) {
        let windows: Vec<_> = self
            .space
            .elements()
            .filter(|w| window_data(w).restore.get().is_some())
            .filter(|w| self.window_output(w).as_ref() == Some(output))
            .cloned()
            .collect();
        for window in windows {
            self.rearrange_on(&window, Some(output.clone()), |_, _| {});
        }
    }

    /// The anchored surface that keeps the keyboard: exclusive keyboard, on the overlay or top
    /// layer, with content.
    pub(crate) fn exclusive_layer(&self) -> Option<WlSurface> {
        self.space.outputs().find_map(|output| {
            let map = layer_map_for_output(output);
            [Layer::Overlay, Layer::Top].into_iter().find_map(|layer| {
                map.layers_on(layer)
                    .rev()
                    .find(|l| {
                        l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive
                            && has_content(l.wl_surface())
                    })
                    .map(|l| l.wl_surface().clone())
            })
        })
    }

    /// The anchored surface accepts the keyboard, exclusive or on demand.
    fn layer_wants_keyboard(&self, surface: &WlSurface) -> bool {
        self.space.outputs().any(|output| {
            layer_map_for_output(output)
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .is_some_and(LayerSurface::can_receive_keyboard_focus)
        })
    }

    /// A click on an anchored surface: it gets the keyboard if it accepts it, unless another
    /// one keeps it.
    pub(crate) fn click_layer(&mut self, surface: &WlSurface) {
        if !self.layer_wants_keyboard(surface) || self.exclusive_layer().is_some() {
            return;
        }
        self.set_keyboard_focus(Some(KeyboardFocus::Wayland(surface.clone())));
    }

    /// Anchored surface of `layers` under `pos`, with the surface hit and its origin.
    pub(crate) fn layer_hit(&self, pos: Point<f64, Logical>, layers: &[Layer]) -> Option<Hit> {
        let output = self.space.output_under(pos).next()?;
        let origin = self.space.output_geometry(output)?.loc;
        let map = layer_map_for_output(output);
        let local = pos - origin.to_f64();
        layers.iter().find_map(|layer| {
            let surface = map.layer_under(*layer, local)?;
            let loc = map.layer_geometry(surface)?.loc - surface.bbox().loc;
            let (hit, offset) =
                surface.surface_under(local - loc.to_f64(), WindowSurfaceType::ALL)?;
            Some(Hit::Layer(
                surface.wl_surface().clone(),
                hit,
                (origin + loc + offset).to_f64(),
            ))
        })
    }
}

/// Checks the anchored surfaces of a screen of `screen` points before smithay places them:
/// no size beyond 2^31 (it would turn negative), no margin larger than the screen, and the
/// exclusive zones with their margins, summed along each axis, fit in the screen. Everything
/// smithay then computes stays within a few times the screen size: nothing can overflow.
fn check_reserves(
    states: impl Iterator<Item = LayerSurfaceCachedState>,
    screen: Size<i32, Logical>,
) -> Result<(), &'static str> {
    let (width, height) = (i64::from(screen.w), i64::from(screen.h));
    let (mut across, mut down) = (0_i64, 0_i64);
    for state in states {
        if state.size.w < 0 || state.size.h < 0 {
            return Err("size out of bounds");
        }
        let m = state.margin;
        let margin = |side: Anchor, value: i32| {
            if state.anchor.contains(side) {
                i64::from(value).abs()
            } else {
                0
            }
        };
        let all = [m.top, m.right, m.bottom, m.left];
        if all.iter().any(|v| i64::from(*v).abs() > width.max(height)) {
            return Err("margin larger than the screen");
        }
        let ExclusiveZone::Exclusive(amount) = state.exclusive_zone else {
            continue;
        };
        let amount = i64::from(amount);
        let a = state.anchor;
        let (left, right) = (margin(Anchor::LEFT, m.left), margin(Anchor::RIGHT, m.right));
        let (top, bottom) = (margin(Anchor::TOP, m.top), margin(Anchor::BOTTOM, m.bottom));
        // The same cases, in the same order, as smithay's `LayerMap::arrange`.
        if a.contains(Anchor::TOP | Anchor::BOTTOM)
            || a.contains(Anchor::LEFT) != a.contains(Anchor::RIGHT)
        {
            across += amount + left + right;
        } else if a.contains(Anchor::LEFT | Anchor::RIGHT)
            || a.intersects(Anchor::TOP | Anchor::BOTTOM)
        {
            down += amount + top + bottom;
        }
    }
    if across > width || down > height {
        return Err("exclusive zone larger than the screen");
    }
    Ok(())
}

/// The surface has a buffer: it shows something.
fn has_content(surface: &WlSurface) -> bool {
    with_renderer_surface_state(surface, |state| state.buffer().is_some()).unwrap_or(false)
}

/// Menu with an outside click pending: the root surface it belongs to, and its first popup.
/// A click on a surface of another client closes the whole chain (`xdg_popup.popup_done`).
pub(crate) struct PopupGrab {
    pub root: WlSurface,
    pub popup: PopupKind,
}

delegate_dispatch!(State: [ZwlrLayerShellV1: ()] => WlrLayerShellState);
delegate_global_dispatch!(State: [ZwlrLayerShellV1: WlrLayerShellGlobalData] => WlrLayerShellState);

/// Requests of a layer surface go to smithay, as `delegate_layer_shell!` would send them, but
/// a size beyond 2^31 is refused first: smithay turns it into a negative size and asserts
/// against it. The protocol has an error for it (`invalid_size`), which ends the client.
impl Dispatch<ZwlrLayerSurfaceV1, WlrLayerSurfaceUserData> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &ZwlrLayerSurfaceV1,
        request: zwlr_layer_surface_v1::Request,
        data: &WlrLayerSurfaceUserData,
        dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let zwlr_layer_surface_v1::Request::SetSize { width, height } = &request
            && (i32::try_from(*width).is_err() || i32::try_from(*height).is_err())
        {
            tracing::warn!(width, height, "anchored surface: size out of bounds");
            resource.post_error(
                zwlr_layer_surface_v1::Error::InvalidSize,
                "size beyond 2^31",
            );
            return;
        }
        <WlrLayerShellState as Dispatch<_, _, Self>>::request(
            state, client, resource, request, data, dh, data_init,
        );
    }

    fn destroyed(
        state: &mut Self,
        client: ClientId,
        resource: &ZwlrLayerSurfaceV1,
        data: &WlrLayerSurfaceUserData,
    ) {
        <WlrLayerShellState as Dispatch<_, _, Self>>::destroyed(state, client, resource, data);
    }
}
