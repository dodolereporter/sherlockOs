//! Screens: their place in the space and in the host frame, their scale, and the scale each
//! client surface is told to render at (`wp_fractional_scale_v1`, `wl_surface.preferred_buffer_scale`).
//!
//! In nested mode, every screen is a slice of the host window, side by side: the frame is the
//! whole window, each screen draws into its own rectangle of it. In the space, screens sit side
//! by side too, each with its logical size (physical size divided by its scale).

use smithay::{
    delegate_fractional_scale, delegate_viewporter,
    desktop::{Window, layer_map_for_output},
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Rectangle, Size, Transform},
    wayland::{
        compositor::{SurfaceData, send_surface_state},
        fractional_scale::{FractionalScaleHandler, with_fractional_scale},
    },
};

use sherlock_ui::window as ui;

use crate::{State, layout};

/// A screen and the rectangle of the host frame it draws into.
#[derive(Debug, Clone)]
pub struct Screen {
    pub output: Output,
    pub frame: Rectangle<i32, Physical>,
}

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
        let name = format!("virtuel-{}", self.screens.len() + 1);
        let properties = PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "compositeur".into(),
            model: "virtuel".into(),
        };
        let output = Output::new(name, properties);
        output.create_global::<State>(&self.display_handle);
        output.change_current_state(
            None,
            Some(Transform::Normal),
            Some(Scale::Fractional(scale)),
            None,
        );
        self.screens.push(Screen {
            output: output.clone(),
            frame: Rectangle::from_size(size),
        });
        self.place_screens();
        output
    }

    /// The host frame is now `size`: it is shared equally between the screens.
    pub fn resize_frame(&mut self, size: Size<i32, Physical>) {
        let count = self.screens.len() as i32;
        for (i, screen) in self.screens.iter_mut().enumerate() {
            let w = size.w / count.max(1);
            let last = i as i32 == count - 1;
            screen.frame.size = (if last { size.w - w * (count - 1) } else { w }, size.h).into();
        }
        self.place_screens();
    }

    /// Lays the screens side by side, in the frame and in the space, from their frame size.
    /// Anchored surfaces, and windows that fill a zone of a screen, follow.
    fn place_screens(&mut self) {
        let (mut frame_x, mut space_x) = (0, 0);
        for screen in &mut self.screens {
            screen.frame.loc = (frame_x, 0).into();
            frame_x += screen.frame.size.w;
            let mode = Mode {
                size: screen.frame.size,
                refresh: 60_000,
            };
            let output = &screen.output;
            output.change_current_state(Some(mode), None, None, Some((space_x, 0).into()));
            output.set_preferred(mode);
            self.space.map_output(output, (space_x, 0));
            space_x += self.space.output_geometry(output).map_or(0, |g| g.size.w);
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
        let center = geometry.loc.to_f64() + geometry.size.to_f64().downscale(2.0).to_point();
        self.space
            .output_under(center)
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

impl FractionalScaleHandler for State {}

delegate_fractional_scale!(State);
delegate_viewporter!(State);
