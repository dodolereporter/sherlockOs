//! Anchored surfaces (`zwlr_layer_shell_v1`): the bar, the dock, the launcher and the
//! notifications are separate programs anchored to a screen edge (ADR 0005).
//!
//! Each screen has its layer map (smithay's `LayerMap`), which places its surfaces from their
//! anchors and margins and keeps what their exclusive zones leave to windows: the work area.
//! Background and bottom surfaces are drawn under the windows, top and overlay ones above.

use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    delegate_layer_shell,
    desktop::{LayerSurface, PopupKind, WindowSurfaceType, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle},
    wayland::shell::{
        wlr_layer::{
            KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler,
            WlrLayerShellState,
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
        tracing::debug!(namespace, output = output.name(), "surface ancrée");
        let mapped =
            layer_map_for_output(&output).map_layer(&LayerSurface::new(surface, namespace));
        if let Err(err) = mapped {
            tracing::warn!("surface ancrée refusée : {err}");
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
        let Some((output, _)) = self.layer_for(surface.wl_surface()) else {
            return;
        };
        {
            let mut map = layer_map_for_output(&output);
            let layer = map
                .layer_for_surface(surface.wl_surface(), WindowSurfaceType::TOPLEVEL)
                .cloned();
            if let Some(layer) = layer {
                map.unmap_layer(&layer);
            }
        }
        self.reflow(&output);
        self.focus(self.focused_window().or_else(|| self.top_window()));
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

    /// An anchored surface committed: its screen places it again, and it gets its first
    /// configure. When its exclusive zone changes, maximized and snapped windows follow.
    pub(crate) fn layer_commit(&mut self, surface: &WlSurface) -> bool {
        let Some((output, _)) = self.layer_for(surface) else {
            return false;
        };
        let zone_changed = {
            let mut map = layer_map_for_output(&output);
            let before = map.non_exclusive_zone();
            map.arrange();
            if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
                layer.layer_surface().send_pending_configure();
            }
            before != map.non_exclusive_zone()
        };
        if zone_changed {
            self.reflow(&output);
        }
        // An exclusive keyboard surface takes the keyboard, a surface that no longer wants it
        // gives it back.
        let focused = self.keyboard_focus().as_ref() == Some(surface);
        let exclusive = self.exclusive_layer();
        if exclusive.as_ref() == Some(surface) && !focused
            || focused && exclusive.is_none() && !self.layer_wants_keyboard(surface)
        {
            self.focus(self.focused_window().or_else(|| self.top_window()));
        }
        true
    }

    /// Maximized, fullscreen and snapped windows of `output` take its new geometry.
    pub(crate) fn reflow(&mut self, output: &Output) {
        let windows: Vec<_> = self
            .space
            .elements()
            .filter(|w| {
                let data = window_data(w);
                data.shown.get() && data.restore.get().is_some()
            })
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

delegate_layer_shell!(State);
