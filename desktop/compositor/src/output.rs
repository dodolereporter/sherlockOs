//! Screens: their place in the space and in the host frame, their scale, and the scale each
//! client surface is told to render at (`wp_fractional_scale_v1`, `wl_surface.preferred_buffer_scale`).
//!
//! In nested mode, every screen is a slice of the host window, side by side: the frame is the
//! whole window, each screen draws into its own rectangle of it. In the space, screens sit side
//! by side too, each with its logical size (physical size divided by its scale).

use std::time::Duration;

use smithay::{
    delegate_fractional_scale, delegate_viewporter,
    desktop::{Window, layer_map_for_output},
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::{DisplayHandle, backend::GlobalId, protocol::wl_surface::WlSurface},
    },
    utils::{Logical, Physical, Point, Rectangle, Size, Transform},
    wayland::{
        compositor::{SurfaceData, send_surface_state},
        fractional_scale::{FractionalScaleHandler, with_fractional_scale},
    },
};

use desktop_config::displays::Identity;
use sherlock_ui::window as ui;

use crate::{State, layout};

/// A screen and the rectangle of the host frame it draws into.
#[derive(Debug, Clone)]
pub struct Screen {
    pub output: Output,
    pub frame: Rectangle<i32, Physical>,
    /// The `wl_output` global, removed with the screen; none while it is switched off.
    global: Option<GlobalId>,
    /// What can be changed of it, and who it is (lot D-26b).
    pub details: Details,
    /// Its place in the space, chosen by a client or saved; `None`: right of the others.
    pub position: Option<Point<i32, Logical>>,
}

impl Screen {
    /// The screen without its `wl_output` global, which `unplug` removes.
    pub(crate) fn switched_off(mut self) -> Self {
        self.global = None;
        self
    }

    /// Offers its `wl_output` global again.
    pub(crate) fn relight(&mut self, display: &DisplayHandle) {
        self.global = Some(self.output.create_global::<State>(display));
    }
}

/// Who a screen is and what it can do: its EDID names, its modes and whether it can turn
/// (ADR 0025).
#[derive(Debug, Clone, Default)]
pub struct Details {
    pub identity: Identity,
    /// The modes it can take, the preferred one first. Empty: only its current mode (a slice
    /// of the host window).
    pub modes: Vec<Mode>,
    /// It can be rotated: never a slice of the host window.
    pub rotates: bool,
}

/// Modes of a virtual screen without a host window (`--headless` and the tests).
const VIRTUAL_MODES: [(i32, i32); 4] = [(1280, 800), (1440, 900), (1920, 1080), (2560, 1440)];

/// Where a screen draws: its area of the space, its rectangle of the frame, its scale, and the
/// size of the whole frame. Everything the renderer needs to turn space coordinates into frame
/// pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub area: Rectangle<i32, Logical>,
    pub frame: Rectangle<i32, Physical>,
    pub scale: f64,
    pub frame_size: Size<i32, Physical>,
}

impl View {
    /// Frame pixel of a point of the space.
    pub fn point(&self, p: Point<f64, Logical>) -> Point<f64, Physical> {
        (p - self.area.loc.to_f64()).to_physical(self.scale) + self.frame.loc.to_f64()
    }

    /// Frame pixels of a rectangle of the space. Both corners are rounded, so that two
    /// rectangles that touch in the space still touch in the frame.
    pub fn rect(&self, r: Rectangle<f64, Logical>) -> Rectangle<i32, Physical> {
        let loc = self.point(r.loc).to_i32_round();
        let end = self.point(r.loc + r.size.to_point()).to_i32_round();
        Rectangle::new(loc, (end - loc).to_size())
    }
}

impl State {
    /// Adds a screen of `size` pixels at `scale`, to the right of the existing ones, both in the
    /// space and in the frame.
    pub fn add_output(&mut self, size: Size<i32, Physical>, scale: f64) -> Output {
        let name = format!("virtual-{}", self.screens.len() + 1);
        let properties = PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "compositor".into(),
            model: "virtual".into(),
        };
        let output = Output::new(name, properties);
        let mode = |(w, h)| Mode {
            size: (w, h).into(),
            refresh: 60_000,
        };
        // A slice of the host window has the size of the slice; other virtual screens can
        // take any of a few modes, and turn.
        let details = match self.nested {
            true => Details::default(),
            false => Details {
                identity: Identity::new("compositor", "virtual", &self.screens.len().to_string()),
                modes: std::iter::once(size.into())
                    .chain(VIRTUAL_MODES)
                    .map(mode)
                    .fold(Vec::new(), |mut modes, m| {
                        if !modes.contains(&m) {
                            modes.push(m);
                        }
                        modes
                    }),
                rotates: true,
            },
        };
        output.change_current_state(Some(mode(size.into())), None, None, None);
        self.plug_screen(output.clone(), size, scale, details);
        output
    }

    /// Adds `output`, a screen of `size` pixels at `scale`, to the right of the existing ones.
    /// Its refresh rate is its current mode's, if it has one.
    pub fn add_screen(&mut self, output: Output, size: Size<i32, Physical>, scale: f64) {
        self.plug_screen(output, size, scale, Details::default());
    }

    /// Adds `output`, a screen whose mode is `size` pixels, with what was saved of it: its
    /// scale, rotation and place, or `scale` and the right of the others (ADR 0025).
    pub fn plug_screen(
        &mut self,
        output: Output,
        size: Size<i32, Physical>,
        scale: f64,
        details: Details,
    ) {
        self.queue_redraw();
        let saved = self.saved_screen(&details.identity);
        let scale = saved.as_ref().map_or(scale, |s| s.scale);
        let transform = saved
            .as_ref()
            .filter(|_| details.rotates)
            .map_or(Transform::Normal, |s| {
                crate::displays::transform(s.transform)
            });
        let position = saved.as_ref().map(|s| Point::from(s.position));
        output.change_current_state(None, Some(transform), Some(Scale::Fractional(scale)), None);
        let global = output.create_global::<State>(&self.display_handle);
        self.screens.push(Screen {
            output,
            frame: Rectangle::from_size(transform.transform_size(size)),
            global: Some(global),
            details,
            position,
        });
        self.place_screens();
    }

    /// Removes a screen that was unplugged. Its anchored surfaces are closed (their program
    /// opens new ones on the screens left), the others close ranks from the left, and the
    /// windows no screen shows any more come to the screen under the pointer: snapped and
    /// maximized ones in its work area, the others centered in it.
    pub fn remove_screen(&mut self, output: &Output) {
        self.queue_redraw();
        let Some(i) = self.screens.iter().position(|s| &s.output == output) else {
            // Unplugged while switched off.
            self.displays.forget(output);
            return;
        };
        let screen = self.screens.remove(i);
        self.unplug(screen);
    }

    /// Takes `screen` out of the space and of the clients' view: switched off, or unplugged.
    pub(crate) fn unplug(&mut self, screen: Screen) {
        let output = &screen.output;
        for layer in layer_map_for_output(output).layers() {
            layer.layer_surface().send_close();
        }
        self.unmapped_layers.retain(|(_, o)| o != output);
        self.space.unmap_output(output);
        // Clients that bind the global right now must not get a protocol error: it is hidden
        // first, and destroyed a little later.
        if let Some(global) = screen.global {
            let display = self.display_handle.clone();
            display.disable_global::<State>(global.clone());
            let timer = Timer::from_duration(Duration::from_secs(5));
            let removed = self.loop_handle.insert_source(timer, move |_, _, _| {
                display.remove_global::<State>(global.clone());
                TimeoutAction::Drop
            });
            if let Err(err) = removed {
                tracing::warn!("removing the screen's global later: {err}");
            }
        }
        self.place_screens();
        // The pointer comes back on a screen before windows go to the one under it.
        let pointer = self.seat.get_pointer().map(|p| p.current_location());
        let first = self.space.outputs().next().cloned();
        if pointer.is_some_and(|p| self.space.output_under(p).next().is_none())
            && let Some(area) = first.and_then(|o| self.space.output_geometry(&o))
        {
            self.pointer_motion(center(area), 0);
        }
        let Some(output) = self.pointer_output() else {
            return;
        };
        // Snapped and maximized windows without a screen: `window_output` gives them the
        // pointer's.
        self.reflow(&output);
        let orphans: Vec<Window> = self
            .space
            .elements()
            .filter(|w| {
                let center = self.space.element_geometry(w).map(center);
                center.is_some_and(|c| self.space.output_under(c).next().is_none())
            })
            .cloned()
            .collect();
        let Some(area) = self.work_area(&output) else {
            return;
        };
        for window in orphans {
            let size = self
                .space
                .element_geometry(&window)
                .unwrap_or_default()
                .size;
            let loc = crate::decorations::centered(&window, area, size);
            self.move_window(&window, loc, false);
        }
    }

    /// The host frame is now `size`: it is shared equally between the screens.
    pub fn resize_frame(&mut self, size: Size<i32, Physical>) {
        self.queue_redraw();
        let count = self.screens.len() as i32;
        for (i, screen) in self.screens.iter_mut().enumerate() {
            let w = size.w / count.max(1);
            let last = i as i32 == count - 1;
            screen.frame.size = (if last { size.w - w * (count - 1) } else { w }, size.h).into();
        }
        self.place_screens();
    }

    /// Lays the screens side by side in the frame, from their frame size, and in the space
    /// where they were put, the others to the right of them all. Anchored surfaces, and
    /// windows that fill a zone of a screen, follow.
    pub(crate) fn place_screens(&mut self) {
        let mut frame_x = 0;
        let mut space_x = self
            .screens
            .iter()
            .filter_map(|s| {
                let size = s.output.current_mode()?.size;
                let size = s.output.current_transform().transform_size(size);
                let width = size
                    .to_f64()
                    .to_logical(s.output.current_scale().fractional_scale());
                Some(s.position?.x + width.w.round() as i32)
            })
            .max()
            .unwrap_or(0);
        for screen in &mut self.screens {
            screen.frame.loc = (frame_x, 0).into();
            frame_x += screen.frame.size.w;
            let refresh = screen.output.current_mode().map_or(60_000, |m| m.refresh);
            let output = &screen.output;
            // The frame is the screen turned; its mode, the screen as it is.
            let mode = Mode {
                size: output.current_transform().transform_size(screen.frame.size),
                refresh,
            };
            let at = screen.position.unwrap_or((space_x, 0).into());
            output.change_current_state(Some(mode), None, None, Some(at));
            if screen.details.modes.is_empty() {
                output.set_preferred(mode);
            }
            self.space.map_output(output, at);
            if screen.position.is_none() {
                space_x += self.space.output_geometry(output).map_or(0, |g| g.size.w);
            }
        }
        for output in self.outputs() {
            layer_map_for_output(&output).arrange();
            self.reflow(&output);
        }
    }

    /// The screens, in order.
    pub fn outputs(&self) -> Vec<Output> {
        self.screens.iter().map(|s| s.output.clone()).collect()
    }

    /// Size of the host frame that all screens share.
    pub fn frame_size(&self) -> Size<i32, Physical> {
        let frame = self.screens.iter().map(|s| s.frame);
        frame.reduce(|a, b| a.merge(b)).unwrap_or_default().size
    }

    /// How each screen draws into the frame, for the renderer.
    pub fn views(&self) -> Vec<(Output, View)> {
        let frame_size = self.frame_size();
        self.screens
            .iter()
            .filter_map(|s| {
                let view = View {
                    area: self.space.output_geometry(&s.output)?,
                    frame: s.frame,
                    scale: s.output.current_scale().fractional_scale(),
                    frame_size,
                };
                Some((s.output.clone(), view))
            })
            .collect()
    }

    /// Point of the space under the frame pixel `p`. Out of every screen, the nearest screen
    /// along the horizontal axis takes it.
    pub fn frame_to_space(&self, p: Point<f64, Physical>) -> Option<Point<f64, Logical>> {
        let screen = self
            .screens
            .iter()
            .find(|s| p.x < f64::from(s.frame.loc.x + s.frame.size.w))
            .or(self.screens.last())?;
        let area = self.space.output_geometry(&screen.output)?;
        let scale = screen.output.current_scale().fractional_scale();
        Some((p - screen.frame.loc.to_f64()).to_logical(scale) + area.loc.to_f64())
    }

    /// Screen under the pointer, or the first one. New windows, snapping and anchored surfaces
    /// without a screen of their own go there.
    pub fn pointer_output(&self) -> Option<Output> {
        let pointer = self.seat.get_pointer()?.current_location();
        self.space
            .output_under(pointer)
            .next()
            .or_else(|| self.space.outputs().next())
            .cloned()
    }

    /// Screen of a window: the one under its center, or the pointer's.
    pub fn window_output(&self, window: &Window) -> Option<Output> {
        let geometry = self.space.element_geometry(window)?;
        self.space
            .output_under(center(geometry))
            .next()
            .cloned()
            .or_else(|| self.pointer_output())
    }

    /// Part of a screen left to windows once anchored surfaces took their exclusive zone, in
    /// the space: where windows are centered, maximized and snapped. Never outside the screen,
    /// never smaller than `MIN_WORK_AREA` (see `layout::work_area`).
    pub fn work_area(&self, output: &Output) -> Option<Rectangle<i32, Logical>> {
        let geometry = self.space.output_geometry(output)?;
        let mut zone = layer_map_for_output(output).non_exclusive_zone();
        zone.loc += geometry.loc;
        Some(layout::work_area(geometry, zone, ui::MIN_WORK_AREA))
    }

    /// Work area of the screen under the pointer.
    pub fn output_area(&self) -> Option<Rectangle<i32, Logical>> {
        self.work_area(&self.pointer_output()?)
    }

    /// Tells every surface the scale of its screen: fractional for clients that know
    /// `wp_fractional_scale_v1`, rounded up for the others. Smithay only sends what changed.
    pub(crate) fn update_scales(&self) {
        let tell = |output: &Output| {
            let scale = output.current_scale();
            move |surface: &WlSurface, states: &SurfaceData| {
                with_fractional_scale(states, |fractional| {
                    fractional.set_preferred_scale(scale.fractional_scale());
                });
                send_surface_state(surface, states, scale.integer_scale(), Transform::Normal);
            }
        };
        for window in self.space.elements() {
            if let Some(output) = self.window_output(window) {
                window.with_surfaces(tell(&output));
            }
        }
        for output in self.space.outputs() {
            for layer in layer_map_for_output(output).layers() {
                layer.with_surfaces(tell(output));
            }
        }
    }
}

/// Center of a rectangle of the space.
fn center(r: Rectangle<i32, Logical>) -> Point<f64, Logical> {
    r.loc.to_f64() + r.size.to_f64().downscale(2.0).to_point()
}

impl FractionalScaleHandler for State {}

delegate_fractional_scale!(State);
delegate_viewporter!(State);
