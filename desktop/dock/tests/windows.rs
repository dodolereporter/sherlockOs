//! The `zwlr_foreign_toplevel_manager_v1` source against a fake compositor: KWin does not offer
//! the protocol and our compositor not yet, so a small `wayland-server` plays the compositor.

use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use dock::windows::{self, Request, Source, SourceError, Wlr};
use wayland_client::Connection;
use wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::{
    self as handle, ZwlrForeignToplevelHandleV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::{
    self as manager, ZwlrForeignToplevelManagerV1,
};
use wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use wayland_server::protocol::wl_seat::WlSeat;
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, Resource,
};

/// The fake compositor: open windows (`app_id`, active), the handles sent to each client, and
/// what the clients asked of the windows.
struct Compositor {
    windows: Vec<(String, bool)>,
    handles: Vec<ZwlrForeignToplevelHandleV1>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ()> for Compositor {
    fn bind(
        state: &mut Self,
        display: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let manager = init.init(resource, ());
        for (app_id, active) in &state.windows {
            let Ok(toplevel) = client.create_resource::<ZwlrForeignToplevelHandleV1, String, Self>(
                display,
                manager.version(),
                app_id.clone(),
            ) else {
                continue;
            };
            manager.toplevel(&toplevel);
            toplevel.app_id(app_id.clone());
            let flags = if *active {
                (handle::State::Activated as u32).to_ne_bytes().to_vec()
            } else {
                Vec::new()
            };
            toplevel.state(flags);
            toplevel.done();
            state.handles.push(toplevel);
        }
    }
}

impl GlobalDispatch<WlSeat, ()> for Compositor {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<WlSeat>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<WlSeat, ()> for Compositor {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlSeat,
        _: wayland_server::protocol::wl_seat::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Compositor {
    fn request(
        _: &mut Self,
        _: &Client,
        resource: &ZwlrForeignToplevelManagerV1,
        request: manager::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let manager::Request::Stop = request {
            resource.finished();
        }
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, String> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwlrForeignToplevelHandleV1,
        request: handle::Request,
        app_id: &String,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let name = match request {
            handle::Request::Activate { .. } => "activate",
            handle::Request::SetMinimized => "minimize",
            handle::Request::Close => "close",
            _ => return,
        };
        if let Ok(mut requests) = state.requests.lock() {
            requests.push(format!("{name} {app_id}"));
        }
        if name == "close" {
            // The window goes: every client that knows it hears so.
            state.windows.retain(|(id, _)| id != app_id);
            for toplevel in state
                .handles
                .iter()
                .filter(|h| h.data::<String>() == Some(app_id))
            {
                toplevel.closed();
            }
        }
    }
}

struct NoData;

impl ClientData for NoData {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

/// Runs the fake compositor on a thread; returns a connection for each of `clients`.
struct Fake {
    connections: Vec<Connection>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    fn start(
        windows: &[(&str, bool)],
        with_manager: bool,
        clients: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut display = Display::<Compositor>::new()?;
        let mut handle = display.handle();
        if with_manager {
            handle.create_global::<Compositor, ZwlrForeignToplevelManagerV1, ()>(3, ());
            handle.create_global::<Compositor, WlSeat, ()>(1, ());
        }
        let mut connections = Vec::new();
        for _ in 0..clients {
            let (ours, theirs) = UnixStream::pair()?;
            handle.insert_client(theirs, Arc::new(NoData))?;
            connections.push(Connection::from_socket(ours)?);
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut state = Compositor {
            windows: windows.iter().map(|(a, b)| (a.to_string(), *b)).collect(),
            handles: Vec::new(),
            requests: Arc::clone(&requests),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let _ = display.dispatch_clients(&mut state);
                let _ = display.flush_clients();
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Ok(Self {
            connections,
            requests,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn the_windows_are_read_and_acted_on() {
    let windows = [("org.example.terminal", true), ("ExampleBrowser", false)];
    let mut fake = Fake::start(&windows, true, 4).expect("fake compositor");
    let mut watcher = Wlr::with_connection(fake.connections.remove(0)).expect("manager bound");
    let windows = watcher.read().expect("read");
    assert!(windows.controllable);
    assert_eq!(windows.of(|id| id == "org.example.terminal"), (1, true));
    assert_eq!(windows.of(|id| id == "ExampleBrowser"), (1, false));

    // The clicks of the dock: the browser, open behind, is brought forward; the terminal, in
    // front, is minimized. Each request goes on a connection of its own.
    let browser = windows::click(Some(windows.of(|id| id == "ExampleBrowser")), true);
    assert_eq!(browser, windows::Click::Activate);
    let sent = windows::act(fake.connections.remove(0), Request::Activate, &|id| {
        id == "ExampleBrowser"
    });
    assert_eq!(sent.expect("activated"), 1);
    let terminal = windows::click(Some(windows.of(|id| id == "org.example.terminal")), true);
    assert_eq!(terminal, windows::Click::Minimize);
    let sent = windows::act(fake.connections.remove(0), Request::Minimize, &|id| {
        id == "org.example.terminal"
    });
    assert_eq!(sent.expect("minimized"), 1);

    // « Quitter »: the compositor gets the request, the watcher sees the window go.
    let sent = windows::act(fake.connections.remove(0), Request::Close, &|id| {
        id.eq_ignore_ascii_case("ORG.EXAMPLE.TERMINAL")
    });
    assert_eq!(sent.expect("closed"), 1);
    assert!(watcher.wait(), "a change came");
    let windows = watcher.read().expect("read");
    assert_eq!(windows.list.len(), 1);
    assert_eq!(windows.of(|id| id == "org.example.terminal"), (0, false));
    assert_eq!(
        fake.requests.lock().map(|c| c.clone()).unwrap_or_default(),
        [
            "activate ExampleBrowser",
            "minimize org.example.terminal",
            "close org.example.terminal"
        ]
    );
}

#[test]
fn a_compositor_without_the_protocol_is_reported() {
    let mut fake = Fake::start(&[], false, 1).expect("fake compositor");
    let result = Wlr::with_connection(fake.connections.remove(0));
    assert!(matches!(result, Err(SourceError::Unsupported(_))));
}
