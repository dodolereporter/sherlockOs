//! Le pont entre Slint et Wayland : surface layer-shell de la barre, popups du menu,
//! pointeur, rappels d'image et tampons `wl_shm`.
//!
//! L'échelle est fixée à 1 : la mise à l'échelle fractionnaire demanderait de brancher
//! `wp_fractional_scale` et `wp_viewporter`, ce que iced_layershell fait déjà de son côté.

use std::error::Error;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor};
use slint::platform::{PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_output, wl_pointer, wl_seat, wl_shm, wl_surface,
};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::xdg::XdgPositioner;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::popup::{Popup, PopupConfigure, PopupHandler};
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowHandler};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};

use crate::{Command, Commands, Theme, Ui};

/// Bouton gauche de la souris, dans la numérotation du noyau (BTN_LEFT).
const BTN_LEFT: u32 = 0x110;

/// Une surface Wayland et la fenêtre Slint qui la dessine.
struct Target {
    window: Rc<MinimalSoftwareWindow>,
    size: (u32, u32),
    configured: bool,
    frame_pending: bool,
    pixels: Vec<PremultipliedRgbaColor>,
}

impl Target {
    fn new(window: Rc<MinimalSoftwareWindow>) -> Self {
        Self {
            window,
            size: (0, 0),
            configured: false,
            frame_pending: false,
            pixels: Vec::new(),
        }
    }

    fn configure(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        self.configured = true;
        self.window.set_size(PhysicalSize::new(width, height));
        self.window.request_redraw();
    }

    fn animating(&self) -> bool {
        self.configured && self.window.window().has_active_animations()
    }

    /// Dessine la fenêtre si Slint le demande et si le compositeur attend une image.
    fn render(
        &mut self,
        surface: &wl_surface::WlSurface,
        pool: &mut SlotPool,
        qh: &QueueHandle<App>,
    ) -> Result<(), Box<dyn Error>> {
        if !self.configured || self.frame_pending {
            return Ok(());
        }
        if self.window.window().has_active_animations() {
            self.window.request_redraw();
        }
        let (width, height) = self.size;
        let Target { window, pixels, .. } = self;
        let drawn = window.draw_if_needed(|renderer| {
            pixels.clear();
            pixels.resize((width * height) as usize, PremultipliedRgbaColor::default());
            renderer.render(pixels, width as usize);
        });
        if !drawn {
            return Ok(());
        }
        let (buffer, canvas) = pool.create_buffer(
            width as i32,
            height as i32,
            width as i32 * 4,
            wl_shm::Format::Argb8888,
        )?;
        // wl_shm attend de l'ARGB prémultiplié, rangé en mémoire en petit-boutiste : B, G, R, A.
        let (canvas, _) = canvas.as_chunks_mut::<4>();
        for (dst, p) in canvas.iter_mut().zip(pixels.iter()) {
            *dst = [p.blue, p.green, p.red, p.alpha];
        }
        surface.damage_buffer(0, 0, width as i32, height as i32);
        surface.frame(qh, FrameCallbackData(surface.clone()));
        buffer.attach_to(surface)?;
        surface.commit();
        self.frame_pending = true;
        Ok(())
    }
}

struct PopupSurface {
    popup: Popup,
    target: Target,
}

pub struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    xdg_shell: XdgShell,
    pool: SlotPool,
    layer: LayerSurface,
    bar: Target,
    menu: Option<PopupSurface>,
    submenu: Option<PopupSurface>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    /// Numéro du dernier clic : le compositeur l'exige pour qu'un menu capture le pointeur.
    last_press_serial: Option<u32>,
    ui: Ui,
    commands: Commands,
    exit: bool,
}

pub fn run(ui: Ui, commands: Commands) -> Result<(), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let xdg_shell = XdgShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let theme = ui.bar.global::<Theme>();
    let bar_height = theme.get_bar_height() as u32;
    let surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Top, Some("essai-barre-slint"), None);
    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, bar_height);
    layer.set_exclusive_zone(bar_height as i32);
    let side = theme.get_bar_margin_side() as i32;
    layer.set_margin(theme.get_bar_margin_top() as i32, side, 0, side);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    // Premier commit sans tampon : le compositeur répond par la taille de la barre.
    layer.commit();

    let pool = SlotPool::new(1440 * bar_height as usize * 4, &shm)?;
    let bar = Target::new(ui.bar_window.clone());

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        shm,
        xdg_shell,
        pool,
        layer,
        bar,
        menu: None,
        submenu: None,
        seat: None,
        pointer: None,
        last_press_serial: None,
        ui,
        commands,
        exit: false,
    };
    app.prerender_popups();

    let mut event_loop: EventLoop<App> = EventLoop::try_new()?;
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .map_err(|e| format!("source Wayland : {e}"))?;

    loop {
        slint::platform::update_timers_and_animations();
        app.process_commands(&qh)?;
        if app.exit {
            return Ok(());
        }
        app.render_all(&qh)?;
        // Au repos, la boucle dort jusqu'au prochain minuteur de Slint (l'horloge, chaque
        // seconde). Pendant une animation, les rappels d'image du compositeur la réveillent.
        let timeout = if app.animating() {
            Some(Duration::from_millis(4))
        } else {
            slint::platform::duration_until_next_timer_update()
        };
        event_loop.dispatch(timeout, &mut app)?;
    }
}

impl App {
    /// Slint n'anime pas une propriété lue pour la première fois : sans ce rendu à blanc au
    /// démarrage, la première ouverture du menu apparaît d'un coup, sans fondu.
    fn prerender_popups(&self) {
        let (menu_size, _) = self.menu_geometry();
        let (submenu_size, _) = self.submenu_geometry();
        for (window, (width, height)) in [
            (&self.ui.menu_window, menu_size),
            (&self.ui.submenu_window, submenu_size),
        ] {
            window.set_size(PhysicalSize::new(width, height));
            window.request_redraw();
            window.draw_if_needed(|renderer| {
                let mut pixels = vec![PremultipliedRgbaColor::default(); (width * height) as usize];
                renderer.render(&mut pixels, width as usize);
            });
        }
    }

    fn animating(&self) -> bool {
        self.bar.animating()
            || self.menu.as_ref().is_some_and(|m| m.target.animating())
            || self.submenu.as_ref().is_some_and(|m| m.target.animating())
    }

    fn render_all(&mut self, qh: &QueueHandle<App>) -> Result<(), Box<dyn Error>> {
        self.bar
            .render(self.layer.wl_surface(), &mut self.pool, qh)?;
        for popup in [&mut self.menu, &mut self.submenu].into_iter().flatten() {
            let surface = popup.popup.wl_surface().clone();
            popup.target.render(&surface, &mut self.pool, qh)?;
        }
        Ok(())
    }

    fn process_commands(&mut self, qh: &QueueHandle<App>) -> Result<(), Box<dyn Error>> {
        loop {
            let next = self.commands.borrow_mut().pop_front();
            let Some(command) = next else {
                return Ok(());
            };
            match command {
                Command::ToggleMenu if self.menu.is_some() => self.close_menus(),
                Command::ToggleMenu => self.open_menu(qh)?,
                Command::OpenSubMenu => self.open_submenu(qh)?,
                Command::CloseSubMenu => self.close_submenu(),
                Command::CloseMenus => self.close_menus(),
                Command::SelectDesktop(i) => {
                    self.ui.bar.set_active_desktop(i);
                    self.ui.submenu.set_active_desktop(i);
                }
                Command::Exit => self.exit = true,
            }
        }
    }

    fn positioner(
        &self,
        size: (u32, u32),
        anchor_rect: (i32, i32, i32, i32),
        anchor: xdg_positioner::Anchor,
    ) -> Result<XdgPositioner, Box<dyn Error>> {
        let positioner = XdgPositioner::new(&self.xdg_shell)?;
        positioner.set_size(size.0 as i32, size.1 as i32);
        let (x, y, w, h) = anchor_rect;
        positioner.set_anchor_rect(x, y, w, h);
        positioner.set_anchor(anchor);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        positioner.set_constraint_adjustment(
            xdg_positioner::ConstraintAdjustment::FlipX
                | xdg_positioner::ConstraintAdjustment::FlipY
                | xdg_positioner::ConstraintAdjustment::SlideX
                | xdg_positioner::ConstraintAdjustment::SlideY,
        );
        Ok(positioner)
    }

    /// Les tailles et les positions se calculent à partir de `Theme`, la même source que
    /// l'interface Slint.
    fn menu_geometry(&self) -> ((u32, u32), (i32, i32, i32, i32)) {
        let t = self.ui.bar.global::<Theme>();
        let height = t.get_menu_gap()
            + 2.0
            + 2.0 * t.get_menu_padding()
            + 5.0 * t.get_menu_item_h()
            + 1.0
            + 2.0 * t.get_separator_margin_y();
        let x = t.get_bar_pad_left()
            + t.get_sherlock_w()
            + t.get_left_gap()
            + t.get_app_name_w()
            + t.get_left_gap();
        (
            (t.get_menu_w() as u32, height as u32),
            (
                x as i32,
                0,
                t.get_menu_button_w() as i32,
                t.get_bar_height() as i32,
            ),
        )
    }

    fn submenu_geometry(&self) -> ((u32, u32), (i32, i32, i32, i32)) {
        let t = self.ui.bar.global::<Theme>();
        let height = 2.0 + 2.0 * t.get_menu_padding() + 3.0 * t.get_menu_item_h();
        let width = t.get_submenu_gap() + t.get_submenu_w();
        // L'entrée à sous-menu est la quatrième : le sous-menu s'aligne sur elle.
        let y = t.get_menu_gap() + 3.0 * t.get_menu_item_h();
        (
            (width as u32, height as u32),
            (
                0,
                y as i32,
                t.get_menu_w() as i32,
                t.get_menu_item_h() as i32,
            ),
        )
    }

    fn new_popup(
        &mut self,
        qh: &QueueHandle<App>,
        parent: Option<&Popup>,
        size: (u32, u32),
        anchor_rect: (i32, i32, i32, i32),
        anchor: xdg_positioner::Anchor,
    ) -> Result<Popup, Box<dyn Error>> {
        let positioner = self.positioner(size, anchor_rect, anchor)?;
        let surface = self.compositor.create_surface(qh);
        let popup = Popup::from_surface(
            parent.map(Popup::xdg_surface),
            &positioner,
            qh,
            surface,
            &self.xdg_shell,
        )?;
        if parent.is_none() {
            self.layer.get_popup(popup.xdg_popup());
        }
        // Sans clic (mode automatique), pas de capture : le menu ne se fermera pas en
        // cliquant ailleurs, mais il s'ouvre quand même.
        if let (Some(seat), Some(serial)) = (&self.seat, self.last_press_serial) {
            popup.xdg_popup().grab(seat, serial);
        }
        popup.wl_surface().commit();
        Ok(popup)
    }

    fn open_menu(&mut self, qh: &QueueHandle<App>) -> Result<(), Box<dyn Error>> {
        let (size, rect) = self.menu_geometry();
        let popup = self.new_popup(qh, None, size, rect, xdg_positioner::Anchor::BottomLeft)?;
        self.ui.menu.set_shown(false);
        self.ui.menu.set_shown(true);
        self.ui.bar.set_menu_open(true);
        self.menu = Some(PopupSurface {
            popup,
            target: Target::new(self.ui.menu_window.clone()),
        });
        Ok(())
    }

    fn open_submenu(&mut self, qh: &QueueHandle<App>) -> Result<(), Box<dyn Error>> {
        if self.submenu.is_some() {
            return Ok(());
        }
        let Some(parent) = self.menu.as_ref().map(|m| m.popup.clone()) else {
            return Ok(());
        };
        let (size, rect) = self.submenu_geometry();
        let popup = self.new_popup(
            qh,
            Some(&parent),
            size,
            rect,
            xdg_positioner::Anchor::TopRight,
        )?;
        self.ui.menu.set_submenu_open(true);
        self.submenu = Some(PopupSurface {
            popup,
            target: Target::new(self.ui.submenu_window.clone()),
        });
        Ok(())
    }

    fn close_submenu(&mut self) {
        if self.submenu.take().is_some() {
            self.ui.menu.set_submenu_open(false);
            self.ui
                .submenu_window
                .window()
                .dispatch_event(WindowEvent::PointerExited);
        }
    }

    fn close_menus(&mut self) {
        // Le sous-menu se détruit avant son parent, comme l'exige xdg_popup.
        self.close_submenu();
        if self.menu.take().is_some() {
            self.ui.menu.set_shown(false);
            self.ui
                .menu_window
                .window()
                .dispatch_event(WindowEvent::PointerExited);
        }
        self.ui.bar.set_menu_open(false);
    }

    fn target_for(&mut self, surface: &wl_surface::WlSurface) -> Option<&mut Target> {
        if self.layer.wl_surface() == surface {
            return Some(&mut self.bar);
        }
        [&mut self.menu, &mut self.submenu]
            .into_iter()
            .flatten()
            .find(|p| p.popup.wl_surface() == surface)
            .map(|p| &mut p.target)
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if let Some(target) = self.target_for(surface) {
            target.frame_pending = false;
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (width, height) = configure.new_size;
        self.bar.configure(width, height);
    }
}

impl PopupHandler for App {
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        popup: &Popup,
        config: PopupConfigure,
    ) {
        let (width, height) = (config.width.max(1) as u32, config.height.max(1) as u32);
        if let Some(target) = self.target_for(popup.wl_surface()) {
            target.configure(width, height);
        }
    }

    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Popup) {
        // Clic hors du menu : le compositeur retire la capture, on ferme tout.
        self.commands.borrow_mut().push_back(Command::CloseMenus);
    }
}

/// Exigé par `XdgShell::bind`, qui gère aussi les fenêtres ordinaires. La barre n'en crée aucune.
impl WindowHandler for App {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {}
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        _: WindowConfigure,
        _: u32,
    ) {
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.seat.get_or_insert(seat);
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        self.seat.get_or_insert(seat);
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Some(pointer) = self.pointer.take()
        {
            pointer.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if let PointerEventKind::Press { serial, .. } = event.kind {
                self.last_press_serial = Some(serial);
            }
            let Some(target) = self.target_for(&event.surface) else {
                continue;
            };
            let position = LogicalPosition::new(event.position.0 as f32, event.position.1 as f32);
            let slint_event = match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    WindowEvent::PointerMoved { position }
                }
                PointerEventKind::Leave { .. } => WindowEvent::PointerExited,
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    WindowEvent::PointerPressed {
                        position,
                        button: PointerEventButton::Left,
                    }
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    WindowEvent::PointerReleased {
                        position,
                        button: PointerEventButton::Left,
                    }
                }
                _ => continue,
            };
            target.window.window().dispatch_event(slint_event);
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(App);
delegate_dispatch2!(App);
