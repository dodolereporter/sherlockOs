//! Scène de mesure du rendu : dix fenêtres s'ouvrent, puis la plus haute s'agrandit et revient
//! toutes les 700 ms, douze fois, et la scène se termine. Chaque fenêtre est un aplat aux
//! couleurs du thème : barre de titre, puis corps. Avec `--mesurer`, le compositeur s'arrête
//! avec elle et écrit son bilan :
//!
//! ```sh
//! cargo build -p compositeur --example scene
//! cargo run -p compositeur -- --imbrique --mesurer target/debug/examples/scene
//! ```
//!
//! `scene planche` n'ouvre qu'une fenêtre, de la taille de l'éditeur de la planche Main
//! (836 × 652), et la garde 20 secondes : de quoi comparer les coins, la bordure et l'ombre.

use std::{
    error::Error,
    fs::{File, OpenOptions},
    os::{fd::AsFd, unix::fs::FileExt},
    time::{Duration, Instant},
};

use sherlock_ui::{
    color::{self, Color},
    metrics,
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    protocol::{
        wl_buffer::WlBuffer,
        wl_compositor::WlCompositor,
        wl_registry,
        wl_shm::{Format, WlShm},
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
    },
};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::{self, XdgSurface},
    xdg_toplevel::{self, XdgToplevel},
    xdg_wm_base::{self, XdgWmBase},
};

const WINDOWS: usize = 10;
const TOGGLES: u32 = 12;
const TOGGLE_EVERY: Duration = Duration::from_millis(700);
/// Fenêtre de l'éditeur de code de la planche Main, et durée pendant laquelle elle reste.
const PLATE_WINDOW: (i32, i32) = (836, 652);
const PLATE_HOLD: Duration = Duration::from_secs(20);
/// Valeur de l'état `maximized` dans une configuration xdg_toplevel.
const MAXIMIZED: u32 = 1;

struct Win {
    surface: WlSurface,
    toplevel: XdgToplevel,
    /// Taille choisie par le client quand le compositeur le laisse faire.
    default: (i32, i32),
    pending: (i32, i32),
    maximized: bool,
    buffer: Option<(WlBuffer, File)>,
}

#[derive(Default)]
struct App {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    wm_base: Option<XdgWmBase>,
    windows: Vec<Win>,
    error: Option<String>,
}

/// Une ligne de `width` pixels de la couleur `color`, en ARGB8888 petit-boutiste.
fn row(color: Color, width: i32) -> Vec<u8> {
    let alpha = (color.a * 255.0).round() as u8;
    [color.b, color.g, color.r, alpha].repeat(width.max(0) as usize)
}

/// Mémoire partagée de `len` octets, sans fichier qui reste sur le disque.
fn shm_file(len: u64) -> std::io::Result<File> {
    let path = std::env::temp_dir().join(format!("compositeur-scene-{}", std::process::id()));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;
    std::fs::remove_file(&path)?;
    file.set_len(len)?;
    Ok(file)
}

impl App {
    /// Dessine la fenêtre `index` à sa taille demandée et l'envoie.
    fn draw(&mut self, index: usize, qh: &QueueHandle<Self>) -> Result<(), Box<dyn Error>> {
        let shm = self.shm.as_ref().ok_or("pas de wl_shm")?;
        let window = self.windows.get_mut(index).ok_or("fenêtre inconnue")?;
        let (width, height) = window.pending;
        let title = metrics::TITLE_BAR_HEIGHT.round() as i32;
        let (bar, body) = (row(color::TITLE_BAR, width), row(color::WINDOW, width));
        let mut pixels = Vec::with_capacity(body.len() * height.max(0) as usize);
        for y in 0..height {
            pixels.extend_from_slice(if y < title { &bar } else { &body });
        }
        let file = shm_file(pixels.len() as u64)?;
        file.write_all_at(&pixels, 0)?;
        let pool = shm.create_pool(file.as_fd(), i32::try_from(pixels.len())?, qh, ());
        let buffer = pool.create_buffer(0, width, height, width * 4, Format::Argb8888, qh, ());
        pool.destroy();
        window.surface.attach(Some(&buffer), 0, 0);
        window.surface.damage_buffer(0, 0, width, height);
        window.surface.commit();
        if let Some((old, _)) = window.buffer.replace((buffer, file)) {
            old.destroy();
        }
        Ok(())
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        app: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => app.compositor = Some(registry.bind(name, 4, qh, ())),
                "wl_shm" => app.shm = Some(registry.bind(name, 1, qh, ())),
                "xdg_wm_base" => app.wm_base = Some(registry.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<XdgToplevel, usize> for App {
    fn event(
        app: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let (
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            },
            Some(window),
        ) = (event, app.windows.get_mut(*index))
        {
            window.pending = if width > 0 && height > 0 {
                (width, height)
            } else {
                window.default
            };
            window.maximized = states
                .as_chunks::<4>()
                .0
                .iter()
                .any(|s| u32::from_ne_bytes(*s) == MAXIMIZED);
        }
    }
}

impl Dispatch<XdgSurface, usize> for App {
    fn event(
        app: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        index: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            if let Err(err) = app.draw(*index, qh) {
                app.error.get_or_insert(err.to_string());
            }
        }
    }
}

delegate_noop!(App: WlCompositor);
delegate_noop!(App: WlShmPool);
delegate_noop!(App: ignore WlShm);
delegate_noop!(App: ignore WlSurface);
delegate_noop!(App: ignore WlBuffer);

fn main() -> Result<(), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut app = App::default();
    queue.roundtrip(&mut app)?;
    let (Some(compositor), Some(wm_base)) = (app.compositor.clone(), app.wm_base.clone()) else {
        return Err("le compositeur n'annonce pas wl_compositor et xdg_wm_base".into());
    };
    let plate = std::env::args().nth(1).as_deref() == Some("planche");
    let count = if plate { 1 } else { WINDOWS };
    // Des fenêtres de plus en plus petites, centrées l'une sur l'autre par le compositeur.
    for index in 0..count {
        let step = index as i32;
        let surface = compositor.create_surface(&qh, ());
        let toplevel = wm_base
            .get_xdg_surface(&surface, &qh, index)
            .get_toplevel(&qh, index);
        surface.commit();
        let default = if plate {
            PLATE_WINDOW
        } else {
            (960 - 48 * step, 640 - 32 * step)
        };
        app.windows.push(Win {
            surface,
            toplevel,
            default,
            pending: default,
            maximized: false,
            buffer: None,
        });
    }
    let start = Instant::now();
    let mut toggles = 0;
    // Une période de plus après le dernier changement, pour que son animation finisse.
    while toggles <= TOGGLES {
        queue.roundtrip(&mut app)?;
        if let Some(err) = app.error.take() {
            return Err(err.into());
        }
        if plate {
            toggles = if start.elapsed() < PLATE_HOLD {
                0
            } else {
                TOGGLES + 1
            };
        } else if start.elapsed() >= TOGGLE_EVERY * (toggles + 1) {
            if let (true, Some(top)) = (toggles < TOGGLES, app.windows.last()) {
                if top.maximized {
                    top.toplevel.unset_maximized();
                } else {
                    top.toplevel.set_maximized();
                }
            }
            toggles += 1;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
