//! Drag and drop from an X11 window to a Wayland client (lot D-29c).
//!
//! An X11 application drags with XDND: it owns `XdndSelection`, holds the pointer (its own X11
//! grab, over the press that XWayland got) and sends its messages to the X11 window under it.
//! When the pointer, button held from an X11 window, reaches a Wayland surface while an X11
//! client owns `XdndSelection`, the compositor takes the drag over: it reads the types the
//! application offers (`XdndTypeList`) and starts a drag of its own on the Wayland side
//! (smithay's `start_dnd`), whose data it reads from `XdndSelection` when the Wayland client
//! asks (`x11_paste`). Once the Wayland client is done, or the drag is cancelled, the X11
//! application's button goes up (XTEST, inside XWayland only): it ends its own drag.

use std::time::Duration;

use smithay::{
    input::{Seat, pointer::GrabStartData},
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::{Resource, protocol::wl_data_device_manager::DndAction},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::selection::data_device::{ServerDndGrabHandler, SourceMetadata, start_dnd},
    xwayland::XWaylandClientData,
};

use crate::{State, input::Hit};

/// Longest wait for the Wayland client dropped on to take the data and say so.
const DROP_PATIENCE: Duration = Duration::from_secs(5);

/// An X11 drag carried on the Wayland side.
pub(crate) struct FromX11 {
    /// The X11 button held (1 left, 2 middle, 3 right).
    button: u8,
    dropped: bool,
}

/// The X11 button of a Linux button code.
fn x11_button(code: u32) -> Option<u8> {
    match code {
        0x110 => Some(1),
        0x112 => Some(2),
        0x111 => Some(3),
        _ => None,
    }
}

impl State {
    /// The pointer moved to `location`, over `hit`, without a drag of the compositor's: if a
    /// button held from an X11 window brings an X11 drag over a Wayland surface, it is taken
    /// over.
    pub(crate) fn x11_drag_motion(&mut self, hit: Option<&Hit>, location: Point<f64, Logical>) {
        if self.dnd.from_x11.is_some() || self.dragging() || self.dnd.x11.is_none() {
            return;
        }
        let over_wayland = match hit {
            Some(Hit::Surface(window, ..)) => window.x11_surface().is_none(),
            Some(Hit::Layer(..)) => true,
            _ => false,
        };
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let Some(start) = pointer.grab_start_data() else {
            return;
        };
        let from_x11 = start.focus.as_ref().is_some_and(|(surface, _)| {
            surface
                .client()
                .is_some_and(|c| c.get_data::<XWaylandClientData>().is_some())
        });
        if !over_wayland || !from_x11 {
            return;
        }
        let Some(button) = x11_button(start.button) else {
            return;
        };
        let Some(side) = self.dnd.x11.as_mut() else {
            return;
        };
        let types = match side.xdnd.foreign_drag() {
            Ok(Some(types)) if !types.is_empty() => types,
            Ok(_) => return,
            Err(err) => {
                tracing::debug!("X11 drag not read: {err}");
                return;
            }
        };
        tracing::debug!(?types, "X11 drag taken over on the Wayland side");
        self.dnd.from_x11 = Some(FromX11 {
            button,
            dropped: false,
        });
        let start = GrabStartData {
            focus: None,
            button: start.button,
            location,
        };
        let metadata = SourceMetadata {
            mime_types: types,
            dnd_action: DndAction::Copy | DndAction::Move,
        };
        let (dh, seat) = (self.display_handle.clone(), self.seat.clone());
        start_dnd(
            &dh,
            &seat,
            self,
            SERIAL_COUNTER.next_serial(),
            Some(start),
            None,
            metadata,
        );
    }

    /// A drag of an X11 application is carried on the Wayland side.
    pub(crate) fn dragging_from_x11(&self) -> bool {
        self.dnd.from_x11.is_some()
    }

    /// Escape during an X11 drag carried here: it is cancelled. `false`: none.
    pub(crate) fn cancel_x11_drag(&mut self, time: u32) -> bool {
        if self.dnd.from_x11.as_ref().is_none_or(|d| d.dropped) {
            return self.dnd.from_x11.is_some();
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return true;
        };
        let serial = SERIAL_COUNTER.next_serial();
        let location = pointer.current_location();
        let event = smithay::input::pointer::MotionEvent {
            location,
            serial,
            time,
        };
        pointer.motion(self, None, &event);
        pointer.unset_grab(self, serial, time);
        pointer.frame(self);
        true
    }

    /// The X11 application's button goes up: it ends its own drag.
    fn end_x11_drag(&mut self) {
        let Some(drag) = self.dnd.from_x11.take() else {
            return;
        };
        if let Some(side) = self.dnd.x11.as_mut()
            && let Err(err) = side.xdnd.release_button(drag.button)
        {
            tracing::debug!("X11 button not released: {err}");
        }
        self.queue_redraw();
    }
}

impl ServerDndGrabHandler for State {
    /// The Wayland client reads the drag: from `XdndSelection`, in a thread.
    fn send(&mut self, mime_type: String, fd: std::os::fd::OwnedFd, _seat: Seat<Self>) {
        let (Some(display), Some(_)) = (self.x_display, self.dnd.from_x11.as_ref()) else {
            return;
        };
        crate::x11_paste::paste_drag(display, mime_type, fd);
    }

    fn dropped(&mut self, _seat: Seat<Self>) {
        let Some(drag) = self.dnd.from_x11.as_mut() else {
            return;
        };
        drag.dropped = true;
        // A client that never finishes: the X11 application is released all the same.
        let waited =
            self.loop_handle
                .insert_source(Timer::from_duration(DROP_PATIENCE), |_, _, state| {
                    if state.dnd.from_x11.as_ref().is_some_and(|d| d.dropped) {
                        state.end_x11_drag();
                    }
                    TimeoutAction::Drop
                });
        if let Err(err) = waited {
            tracing::warn!("drop wait: {err}");
        }
    }

    fn finished(&mut self, _seat: Seat<Self>) {
        self.end_x11_drag();
    }

    fn cancelled(&mut self, _seat: Seat<Self>) {
        self.end_x11_drag();
    }
}
