//! Frame pacing and damage of the real session, without a screen (lot P-01). A client shows
//! the typical load: ten windows, the bar and the dock. A fake screen of 1440 × 900 runs the
//! real session's frame cycle at 60 Hz: `Pacing` says when a frame is due, `render::items`
//! builds the scene, the elements the compositor draws keep their identity through `Ids`, and
//! smithay's damage tracker, the one inside each `DrmCompositor`, says what to redraw. Only the
//! drawing itself is left out: `DummyRenderer` stands for the GPU.
//!
//! `bench_ten_windows_bar_and_animated_dock` prints the figures of the merge request, before
//! and after the lot:
//! `cargo test --release -p compositor --test perf -- --ignored --nocapture`.

#[cfg(test)]
mod tests {
    use std::{
        fs::{File, OpenOptions},
        os::{fd::AsFd, unix::net::UnixStream},
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };

    use compositor::{
        State,
        damage::{Ids, Stamped},
        pacing::Pacing,
        render::{Item, items, rounded_opaque},
    };
    use smithay::{
        backend::renderer::{
            damage::OutputDamageTracker,
            element::{
                Element, Id, Kind,
                solid::SolidColorRenderElement,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
                utils::RescaleRenderElement,
            },
            test::DummyRenderer,
            utils::{CommitCounter, DamageSet, OpaqueRegions},
        },
        desktop::layer_map_for_output,
        reexports::{calloop::EventLoop, wayland_server::Resource as _},
        utils::{Buffer, Physical, Point, Rectangle, Scale, Transform},
        wayland::seat::WaylandFocus as _,
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy as _, QueueHandle, delegate_noop,
        protocol::{
            wl_buffer::WlBuffer,
            wl_callback::{self, WlCallback},
            wl_compositor::WlCompositor,
            wl_region::WlRegion,
            wl_registry,
            wl_shm::{Format, WlShm},
            wl_shm_pool::WlShmPool,
            wl_surface::WlSurface,
        },
    };
    use wayland_protocols::xdg::decoration::zv1::client::{
        zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
        zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
    };
    use wayland_protocols::xdg::shell::client::{
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::XdgToplevel,
        xdg_wm_base::{self, XdgWmBase},
    };
    use wayland_protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
        zwlr_layer_surface_v1::{self, Anchor, ZwlrLayerSurfaceV1},
    };

    const SCREEN: (i32, i32) = (1440, 900);
    const WINDOWS: usize = 10;
    const WINDOW: (i32, i32) = (800, 600);
    const BAR: (i32, i32) = (1440, 36);
    const DOCK: (i32, i32) = (700, 76);
    /// One vertical blank at 60 Hz.
    const PERIOD: Duration = Duration::from_micros(16_667);

    #[derive(Default)]
    struct Client {
        globals: Vec<(u32, String, u32)>,
        configures: usize,
        layer_configures: usize,
        /// Frame callbacks received.
        frames: usize,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Client {
        fn event(
            client: &mut Self,
            _: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            {
                client.globals.push((name, interface, version));
            }
        }
    }

    impl Dispatch<XdgWmBase, ()> for Client {
        fn event(
            _: &mut Self,
            base: &XdgWmBase,
            event: xdg_wm_base::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_wm_base::Event::Ping { serial } = event {
                base.pong(serial);
            }
        }
    }

    impl Dispatch<XdgSurface, ()> for Client {
        fn event(
            client: &mut Self,
            surface: &XdgSurface,
            event: xdg_surface::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial } = event {
                surface.ack_configure(serial);
                client.configures += 1;
            }
        }
    }

    impl Dispatch<ZwlrLayerSurfaceV1, ()> for Client {
        fn event(
            client: &mut Self,
            surface: &ZwlrLayerSurfaceV1,
            event: zwlr_layer_surface_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
                surface.ack_configure(serial);
                client.layer_configures += 1;
            }
        }
    }

    impl Dispatch<WlCallback, ()> for Client {
        fn event(
            client: &mut Self,
            _: &WlCallback,
            event: wl_callback::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_callback::Event::Done { .. } = event {
                client.frames += 1;
            }
        }
    }

    delegate_noop!(Client: ZxdgDecorationManagerV1);
    delegate_noop!(Client: ignore ZxdgToplevelDecorationV1);
    delegate_noop!(Client: WlCompositor);
    delegate_noop!(Client: WlRegion);
    delegate_noop!(Client: ignore WlSurface);
    delegate_noop!(Client: ignore WlShm);
    delegate_noop!(Client: WlShmPool);
    delegate_noop!(Client: ignore WlBuffer);
    delegate_noop!(Client: ignore XdgToplevel);
    delegate_noop!(Client: ZwlrLayerShellV1);

    /// What the fake screen draws: the client surfaces as the scene builds them, and the
    /// compositor's own shapes as plain rectangles under their stable identity. Only what the
    /// damage tracker reads: nothing is drawn.
    enum Probe {
        Surface(Box<RescaleRenderElement<WaylandSurfaceRenderElement<DummyRenderer>>>),
        /// A window's surface: its opaque region loses the rounded corners, as in
        /// `RoundedSurface`.
        Window(
            Box<RescaleRenderElement<WaylandSurfaceRenderElement<DummyRenderer>>>,
            Rectangle<f64, Physical>,
            f32,
            /// Its opaque region counts: not before lot P-01.
            bool,
        ),
        Paint(Stamped<SolidColorRenderElement>),
    }

    impl Probe {
        fn inner(&self) -> &dyn Element {
            match self {
                Probe::Surface(e) | Probe::Window(e, ..) => e.as_ref(),
                Probe::Paint(e) => e,
            }
        }
    }

    impl Element for Probe {
        fn id(&self) -> &Id {
            self.inner().id()
        }

        fn current_commit(&self) -> CommitCounter {
            self.inner().current_commit()
        }

        fn src(&self) -> Rectangle<f64, Buffer> {
            self.inner().src()
        }

        fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
            self.inner().geometry(scale)
        }

        fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
            self.inner().location(scale)
        }

        fn transform(&self) -> Transform {
            self.inner().transform()
        }

        fn damage_since(
            &self,
            scale: Scale<f64>,
            commit: Option<CommitCounter>,
        ) -> DamageSet<i32, Physical> {
            self.inner().damage_since(scale, commit)
        }

        fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
            let regions = self.inner().opaque_regions(scale);
            match self {
                Probe::Window(e, clip, radius, true) => {
                    rounded_opaque(regions, e.geometry(scale).loc, *clip, *radius)
                }
                Probe::Window(..) => OpaqueRegions::default(),
                _ => regions,
            }
        }

        fn alpha(&self) -> f32 {
            self.inner().alpha()
        }
    }

    /// Shared memory for buffers, without a file left on disk.
    fn shm_file(size: u64) -> File {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("compositor-perf-{}-{n}", std::process::id());
        let path = std::env::temp_dir().join(name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        file.set_len(size).unwrap();
        file
    }

    /// A surface of the client and its buffer.
    struct Shown {
        surface: WlSurface,
        buffer: WlBuffer,
        size: (i32, i32),
        _file: File,
    }

    impl Shown {
        /// Draws again, all of it, and asks for a frame callback.
        fn redraw(&self, qh: &QueueHandle<Client>) {
            self.surface.attach(Some(&self.buffer), 0, 0);
            self.surface.damage(0, 0, self.size.0, self.size.1);
            self.surface.frame(qh, ());
            self.surface.commit();
        }
    }

    /// A frame built by the fake screen.
    struct Frame {
        damage: Vec<Rectangle<i32, Physical>>,
        /// Pixels drawn: each element, inside the damage.
        drawn: u64,
        /// Time spent building the scene and its damage.
        cpu: Duration,
    }

    struct Screen {
        pacing: Pacing,
        ids: Ids,
        tracker: OutputDamageTracker,
        renderer: DummyRenderer,
        /// As before lot P-01: a frame at every blank, new identities in every frame.
        before: bool,
    }

    struct Bench {
        event_loop: EventLoop<'static, State>,
        state: State,
        conn: Connection,
        queue: EventQueue<Client>,
        client: Client,
        screen: Screen,
        windows: Vec<Shown>,
        dock: Option<Shown>,
        _bar: Option<Shown>,
    }

    impl Bench {
        fn start(before: bool) -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output(SCREEN.into(), 1.0);
            let (server_side, client_side) = UnixStream::pair().unwrap();
            state.insert_client(server_side);
            let conn = Connection::from_socket(client_side).unwrap();
            let queue = conn.new_event_queue();
            let mut bench = Self {
                event_loop,
                state,
                conn,
                queue,
                client: Client::default(),
                screen: Screen {
                    pacing: Pacing::default(),
                    ids: Ids::default(),
                    tracker: OutputDamageTracker::new(SCREEN, 1.0, Transform::Normal),
                    renderer: DummyRenderer,
                    before,
                },
                windows: Vec::new(),
                dock: None,
                _bar: None,
            };
            bench.show_desktop();
            bench
        }

        /// Runs the compositor, then the client, once, without waiting.
        fn pump(&mut self) {
            for _ in 0..2 {
                self.conn.flush().unwrap();
                self.event_loop
                    .dispatch(Some(Duration::ZERO), &mut self.state)
                    .unwrap();
                self.state.refresh();
                if let Some(guard) = self.queue.prepare_read() {
                    let _ = guard.read();
                }
                self.queue.dispatch_pending(&mut self.client).unwrap();
            }
        }

        fn pump_until(&mut self, what: &str, done: impl Fn(&Client, &State) -> bool) {
            for _ in 0..200 {
                self.pump();
                if done(&self.client, &self.state) {
                    return;
                }
            }
            panic!("timed out waiting for: {what}");
        }

        fn buffer(&self, size: (i32, i32)) -> (WlBuffer, File) {
            let qh = self.queue.handle();
            let shm: WlShm = self.bind();
            let bytes = size.0 * size.1 * 4;
            let file = shm_file(bytes as u64);
            let pool = shm.create_pool(file.as_fd(), bytes, &qh, ());
            let buffer =
                pool.create_buffer(0, size.0, size.1, size.0 * 4, Format::Argb8888, &qh, ());
            (buffer, file)
        }

        fn bind<I: wayland_client::Proxy + 'static>(&self) -> I
        where
            Client: Dispatch<I, ()>,
        {
            let registry = self.conn.display().get_registry(&self.queue.handle(), ());
            let name = I::interface().name;
            let (id, _, version) = self.client.globals.iter().find(|g| g.1 == name).unwrap();
            registry.bind(
                *id,
                (*version).min(I::interface().version),
                &self.queue.handle(),
                (),
            )
        }

        /// Ten windows of 800 × 600 with the title bar of the compositor (lot D-08), the bar at
        /// the top, the dock at the bottom.
        fn show_desktop(&mut self) {
            let qh = self.queue.handle();
            self.conn.display().get_registry(&qh, ());
            self.pump_until("the globals", |c, _| {
                c.globals.iter().any(|g| g.1 == "zwlr_layer_shell_v1")
            });
            let compositor: WlCompositor = self.bind();
            let wm_base: XdgWmBase = self.bind();
            let layer_shell: ZwlrLayerShellV1 = self.bind();
            let decorations: ZxdgDecorationManagerV1 = self.bind();
            for i in 0..WINDOWS {
                let surface = compositor.create_surface(&qh, ());
                let toplevel = wm_base
                    .get_xdg_surface(&surface, &qh, ())
                    .get_toplevel(&qh, ());
                toplevel.set_title(format!("window {i}"));
                decorations.get_toplevel_decoration(&toplevel, &qh, ());
                surface.commit();
                self.pump_until("the configure", |c, _| c.configures > i);
                let (buffer, file) = self.buffer(WINDOW);
                surface.attach(Some(&buffer), 0, 0);
                // Opaque, as toolkits declare their windows.
                let region = compositor.create_region(&qh, ());
                region.add(0, 0, WINDOW.0, WINDOW.1);
                surface.set_opaque_region(Some(&region));
                surface.commit();
                self.windows.push(Shown {
                    surface,
                    buffer,
                    size: WINDOW,
                    _file: file,
                });
            }
            let mut layer = |size: (i32, i32), anchor: Anchor, layers: usize| {
                let surface = compositor.create_surface(&qh, ());
                let layer = layer_shell.get_layer_surface(
                    &surface,
                    None,
                    Layer::Top,
                    "shell".into(),
                    &qh,
                    (),
                );
                layer.set_size(size.0 as u32, size.1 as u32);
                layer.set_anchor(anchor);
                surface.commit();
                self.pump_until("the anchored surface", |c, _| c.layer_configures > layers);
                let (buffer, file) = self.buffer(size);
                surface.attach(Some(&buffer), 0, 0);
                surface.commit();
                drop(layer);
                Shown {
                    surface,
                    buffer,
                    size,
                    _file: file,
                }
            };
            let bar = layer(BAR, Anchor::Top | Anchor::Left | Anchor::Right, 0);
            let dock = layer(DOCK, Anchor::Bottom, 1);
            self._bar = Some(bar);
            self.dock = Some(dock);
            self.pump_until("every window", |_, s| s.space.elements().count() == WINDOWS);
        }

        /// One vertical blank: the clients answer, then the screen draws if a frame is due.
        fn tick(&mut self) -> Option<Frame> {
            self.tick_with(|_| {})
        }

        /// One vertical blank, where `client` acts once it has read what came at the previous
        /// one (its frame callbacks). The animations follow the compositor's clock: real time.
        fn tick_with(&mut self, client: impl FnOnce(&mut Self)) -> Option<Frame> {
            let started = Instant::now();
            // The blank of the previous frame, real or estimated.
            self.screen.pacing.presented();
            self.pump();
            client(self);
            self.pump();
            let frame = self.frame();
            if frame.is_some() {
                self.send_frames();
            }
            std::thread::sleep(PERIOD.saturating_sub(started.elapsed()));
            frame
        }

        /// What `udev::Backend::render` does, the drawing aside.
        fn frame(&mut self) -> Option<Frame> {
            let epoch = self.state.redraw_epoch();
            let screen = &mut self.screen;
            if !screen.before && !screen.pacing.wants_frame(epoch) {
                return None;
            }
            let started = Instant::now();
            if screen.before {
                screen.ids.clear();
            }
            let (output, view) = self.state.views().remove(0);
            let now = self.state.now();
            let (items, animating) = items(&self.state, (&output, &view), None, false, now);
            let elements: Vec<Probe> = items
                .into_iter()
                .flat_map(|item| probe(&mut screen.renderer, &mut screen.ids, item, !screen.before))
                .collect();
            screen.ids.sweep();
            let (damage, _) = screen.tracker.damage_output(1, &elements).unwrap();
            let damage = damage.cloned().unwrap_or_default();
            let cpu = started.elapsed();
            screen.pacing.drawn(epoch, !damage.is_empty(), animating);
            Some(Frame {
                drawn: drawn(&elements, &damage),
                damage,
                cpu,
            })
        }

        fn send_frames(&self) {
            let output = self.state.space.outputs().next().unwrap().clone();
            let time = self.state.now();
            for window in self.state.space.elements() {
                self.state.send_window_frame(window, &output, time);
            }
            for layer in layer_map_for_output(&output).layers() {
                layer.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                    Some(output.clone())
                });
            }
        }

        /// Runs until nothing is drawn for half a second: openings and the like are over.
        fn settle(&mut self) {
            // Before lot P-01, the desktop is never at rest: the openings last 0.6 s at most.
            if self.screen.before {
                (0..40).for_each(|_| drop(self.tick()));
                return;
            }
            let mut quiet = 0;
            for _ in 0..300 {
                let frame = self.tick();
                quiet = if frame.is_some_and(|f| !f.damage.is_empty()) {
                    0
                } else {
                    quiet + 1
                };
                if quiet == 30 {
                    return;
                }
            }
            panic!("the desktop never comes to rest");
        }

        /// `ticks` blanks with the dock drawing at each frame callback, like its magnification.
        fn animate_dock(&mut self, ticks: usize) -> Vec<Frame> {
            let qh = self.queue.handle();
            let mut seen = self.client.frames;
            self.dock.as_ref().unwrap().redraw(&qh);
            let mut frames = Vec::new();
            for _ in 0..ticks {
                frames.extend(self.tick_with(|bench| {
                    if bench.client.frames > seen {
                        seen = bench.client.frames;
                        bench.dock.as_ref().unwrap().redraw(&qh);
                    }
                }));
            }
            frames
        }

        /// Where window `index` is, in frame pixels (scale 1).
        fn rect_of_window(&self, index: usize) -> Rectangle<i32, Physical> {
            let id = self.windows[index].surface.id().protocol_id();
            let window = self
                .state
                .space
                .elements()
                .find(|w| w.wl_surface().is_some_and(|s| s.id().protocol_id() == id))
                .unwrap();
            let r = self.state.space.element_geometry(window).unwrap();
            Rectangle::new((r.loc.x, r.loc.y).into(), (r.size.w, r.size.h).into())
        }

        fn dock_rect(&self) -> Rectangle<i32, Physical> {
            let output = self.state.space.outputs().next().unwrap();
            let map = layer_map_for_output(output);
            let dock = map.layers().last().unwrap();
            let r = map.layer_geometry(dock).unwrap();
            Rectangle::new((r.loc.x, r.loc.y).into(), (r.size.w, r.size.h).into())
        }
    }

    /// The elements of one item, as `Scene::elements` makes them, without the GPU programs.
    fn probe(renderer: &mut DummyRenderer, ids: &mut Ids, item: Item, opaque: bool) -> Vec<Probe> {
        let params = item.params();
        let surfaces =
            |renderer: &mut DummyRenderer, surface, origin, at, scale: Scale<f64>, alpha| {
                let tree: Vec<WaylandSurfaceRenderElement<DummyRenderer>> =
                    render_elements_from_surface_tree(
                        renderer,
                        surface,
                        origin,
                        1.0,
                        alpha,
                        Kind::Unspecified,
                    );
                tree.into_iter()
                    .map(|e| {
                        Probe::Surface(Box::new(RescaleRenderElement::from_element(e, at, scale)))
                    })
                    .collect::<Vec<_>>()
            };
        // Translucent, like the shaders' elements, which declare no opaque region.
        let paint = |ids: &mut Ids, key, rect, alpha: f32| {
            let solid = SolidColorRenderElement::new(
                Id::new(),
                rect,
                CommitCounter::default(),
                [0.0, 0.0, 0.0, alpha / 2.0],
                Kind::Unspecified,
            );
            vec![Probe::Paint(ids.stamp(key, params, solid))]
        };
        match item {
            Item::Surface {
                surface,
                origin,
                scale,
                alpha,
            } => surfaces(renderer, &surface, origin, origin, scale.into(), alpha),
            Item::Window {
                surface,
                rect,
                clip,
                geometry,
                alpha,
                radius,
            } => {
                let stretch = Scale {
                    x: f64::from(rect.size.w) / f64::from(geometry.size.w),
                    y: f64::from(rect.size.h) / f64::from(geometry.size.h),
                };
                let origin = rect.loc - geometry.loc.to_physical(1);
                let clip = clip.to_f64();
                surfaces(renderer, &surface, origin, rect.loc, stretch, alpha)
                    .into_iter()
                    .map(|probe| match probe {
                        Probe::Surface(e) => Probe::Window(e, clip, radius, opaque),
                        other => other,
                    })
                    .collect()
            }
            Item::Shape {
                key, rect, alpha, ..
            } => paint(ids, key, rect, alpha),
            Item::Shadow {
                key, shadow, alpha, ..
            } => paint(ids, key, shadow.area, alpha),
            Item::Wallpaper => Vec::new(),
            Item::Decoration {
                key, rect, alpha, ..
            } => paint(ids, key, rect, alpha),
        }
    }

    /// Pixels drawn: the part of each element inside the damage and not hidden by the opaque
    /// regions of the elements in front of it, as smithay draws them.
    fn drawn(elements: &[Probe], damage: &[Rectangle<i32, Physical>]) -> u64 {
        let mut disjoint: Vec<Rectangle<i32, Physical>> = Vec::new();
        for rect in damage {
            let pieces = rect.subtract_rects(disjoint.iter().copied());
            disjoint.extend(pieces);
        }
        let scale = Scale::from(1.0);
        let mut opaque: Vec<Rectangle<i32, Physical>> = Vec::new();
        let mut total = 0;
        for element in elements {
            let geometry = element.geometry(scale);
            let damaged = disjoint.iter().filter_map(|d| d.intersection(geometry));
            let visible = Rectangle::subtract_rects_many(damaged, opaque.iter().copied());
            total += visible
                .iter()
                .map(|r| (r.size.w * r.size.h) as u64)
                .sum::<u64>();
            opaque.extend(element.opaque_regions(scale).into_iter().map(|mut r| {
                r.loc += geometry.loc;
                r
            }));
        }
        total
    }

    fn bounds(damage: &[Rectangle<i32, Physical>]) -> Option<Rectangle<i32, Physical>> {
        damage.iter().copied().reduce(|a, b| a.merge(b))
    }

    #[test]
    fn nothing_is_drawn_at_rest() {
        let mut bench = Bench::start(false);
        bench.settle();
        let epoch = bench.state.redraw_epoch();
        let built = (0..30).filter_map(|_| bench.tick()).count();
        assert_eq!(built, 0, "no frame is even built at rest");
        assert_eq!(bench.state.redraw_epoch(), epoch, "nothing changed");
    }

    #[test]
    fn a_window_that_draws_redraws_only_its_area() {
        let mut bench = Bench::start(false);
        bench.settle();
        let qh = bench.queue.handle();
        bench.windows[3].redraw(&qh);
        let frames: Vec<Frame> = (0..5).filter_map(|_| bench.tick()).collect();
        assert_eq!(frames.len(), 1, "one frame, then rest again");
        assert_eq!(bounds(&frames[0].damage), Some(bench.rect_of_window(3)));
    }

    #[test]
    fn a_title_bar_is_drawn_again_only_when_it_changes() {
        let mut bench = Bench::start(false);
        bench.settle();
        // The pointer comes on the close button of the window on top: its bar alone is drawn
        // again, with the button lit.
        let window = bench.state.space.elements().last().unwrap().clone();
        let r = bench.state.space.element_geometry(&window).unwrap();
        let close = (r.loc.x + r.size.w - 9 - 14, r.loc.y - 42 + 21);
        let close = (f64::from(close.0), f64::from(close.1));
        bench.state.pointer_motion(close.into(), 0);
        let frames: Vec<Frame> = (0..5).filter_map(|_| bench.tick()).collect();
        assert_eq!(frames.len(), 1, "one frame, then rest again");
        let bar = Rectangle::new((r.loc.x, r.loc.y - 42).into(), (r.size.w, 42).into());
        assert_eq!(bounds(&frames[0].damage), Some(bar));
        // Still on the button: a frame for the motion, nothing to draw.
        bench
            .state
            .pointer_motion((close.0 + 1.0, close.1).into(), 1);
        let frames: Vec<Frame> = (0..5).filter_map(|_| bench.tick()).collect();
        assert!(frames.iter().all(|f| f.damage.is_empty()));
    }

    #[test]
    fn the_animated_dock_redraws_only_the_dock() {
        let mut bench = Bench::start(false);
        bench.settle();
        let frames = bench.animate_dock(30);
        assert!(
            frames.len() >= 28,
            "a frame at every blank: {}",
            frames.len()
        );
        let dock = bench.dock_rect();
        for frame in &frames {
            assert_eq!(bounds(&frame.damage), Some(dock));
        }
    }

    #[test]
    fn a_workspace_switch_redraws_everything_while_it_slides() {
        let mut bench = Bench::start(false);
        bench.settle();
        bench.state.activate_workspace("2");
        let frames: Vec<Frame> = (0..60).filter_map(|_| bench.tick()).collect();
        // The windows slide away, then the background they leave is drawn again, whole.
        let screen = Rectangle::from_size(SCREEN.into());
        let damage: Vec<_> = frames.iter().flat_map(|f| f.damage.clone()).collect();
        assert_eq!(bounds(&damage), Some(screen), "the whole screen");
        let shown = frames.iter().filter(|f| !f.damage.is_empty()).count();
        assert!(shown > 5, "a frame at each blank while sliding: {shown}");
        assert!(frames.len() < 40, "then rest: {} frames", frames.len());
    }

    /// Figures over a run, per second of the screen.
    struct Run {
        built: usize,
        shown: usize,
        damaged: u64,
        drawn: u64,
        cpu: Duration,
        seconds: f64,
    }

    impl Run {
        fn of(frames: &[Frame], ticks: usize) -> Self {
            Self {
                built: frames.len(),
                shown: frames.iter().filter(|f| !f.damage.is_empty()).count(),
                damaged: frames
                    .iter()
                    .map(|f| {
                        f.damage
                            .iter()
                            .map(|r| (r.size.w * r.size.h) as u64)
                            .sum::<u64>()
                    })
                    .sum(),
                drawn: frames.iter().map(|f| f.drawn).sum(),
                cpu: frames.iter().map(|f| f.cpu).sum(),
                seconds: ticks as f64 * PERIOD.as_secs_f64(),
            }
        }

        fn line(&self, what: &str) -> String {
            let per_s = |n: f64| n / self.seconds;
            let mpx = |n: u64| per_s(n as f64) / 1e6;
            let cpu = self.cpu.as_secs_f64() * 1000.0 / self.built.max(1) as f64;
            format!(
                "{what:<34} {:>6.1} {:>6.1} {:>10.1} {:>10.1} {:>10.3}",
                per_s(self.built as f64),
                per_s(self.shown as f64),
                mpx(self.damaged),
                mpx(self.drawn),
                cpu,
            )
        }
    }

    #[test]
    #[ignore = "bench: prints the figures, run it with --release --ignored --nocapture"]
    fn bench_ten_windows_bar_and_animated_dock() {
        let screen = f64::from(SCREEN.0 * SCREEN.1) / 1e6;
        eprintln!(
            "screen 1440 × 900 ({screen:.2} Mpx), 10 windows of 800 × 600, bar, dock of \
             700 × 76; 60 Hz"
        );
        eprintln!(
            "{:<34} {:>6} {:>6} {:>10} {:>10} {:>10}",
            "", "built", "shown", "damage", "drawn", "cpu"
        );
        eprintln!(
            "{:<34} {:>6} {:>6} {:>10} {:>10} {:>10}",
            "", "/s", "/s", "Mpx/s", "Mpx/s", "ms/frame"
        );
        for before in [true, false] {
            let label = if before { "before" } else { "after" };
            let mut bench = Bench::start(before);
            bench.settle();
            let rest: Vec<Frame> = (0..120).filter_map(|_| bench.tick()).collect();
            eprintln!("{}", Run::of(&rest, 120).line(&format!("{label}, at rest")));
            let dock = bench.animate_dock(120);
            eprintln!(
                "{}",
                Run::of(&dock, 120).line(&format!("{label}, dock animated"))
            );
            let qh = bench.queue.handle();
            let mut windows = Vec::new();
            for _ in 0..60 {
                bench.windows[0].redraw(&qh);
                windows.extend(bench.tick());
            }
            eprintln!(
                "{}",
                Run::of(&windows, 60).line(&format!("{label}, a window drawing"))
            );
            bench.state.activate_workspace("2");
            let switch: Vec<Frame> = (0..60).filter_map(|_| bench.tick()).collect();
            eprintln!(
                "{}",
                Run::of(&switch, 60).line(&format!("{label}, workspace switch"))
            );
        }
    }
}
