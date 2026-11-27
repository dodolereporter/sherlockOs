//! Windowed backend: the compositor shows up in a window of a Wayland or X11 session.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use sherlock_ui::color;
use smithay::{
    backend::{
        renderer::{
            Frame as _, ImportDma as _, Renderer as _, element::Element as _,
            element::RenderElement as _, gles::GlesRenderer,
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    desktop::layer_map_for_output,
    input::pointer::CursorIcon,
    reexports::calloop::EventLoop,
    utils::{Rectangle, Scale, Transform},
};

use crate::{
    Error, State,
    anim::PreviewMotion,
    damage::Ids,
    dmabuf,
    render::Scene,
    wallpaper::{Textures, Wallpaper},
    window_anim,
};

/// Opens the window, creates one screen per scale in `scales`, side by side in the window, and
/// plugs rendering into the loop.
pub fn init(
    event_loop: &EventLoop<'static, State>,
    state: &mut State,
    scales: &[f64],
) -> Result<(), Error> {
    let (backend, winit) = winit::init::<GlesRenderer>()?;
    // Shared with the dmabuf importer, which State calls while it reads the clients: never
    // during a frame, since the loop runs one source at a time.
    let backend = Rc::new(RefCell::new(backend));
    enable_dmabuf(state, &backend);
    state.nested = true;
    let scene = {
        let mut backend = backend.borrow_mut();
        for scale in scales {
            state.add_output(backend.window_size(), *scale);
        }
        state.resize_frame(backend.window_size());
        Rc::new(Scene::new(backend.renderer())?)
    };
    // The last image of a window that closes (lot D-34): drawn while the clients are read,
    // never during a frame.
    let (weak_backend, weak_scene) = (Rc::downgrade(&backend), Rc::downgrade(&scene));
    state.set_snapshotter(Box::new(move |state, window| {
        let (backend, scene) = (weak_backend.upgrade()?, weak_scene.upgrade()?);
        let mut backend = backend.try_borrow_mut().ok()?;
        window_anim::snapshot(&scene, backend.renderer(), state, window)
    }));
    // The last image of a menu of the shell that closes (lot UI-06), the same way.
    let (weak_backend, weak_scene) = (Rc::downgrade(&backend), Rc::downgrade(&scene));
    state.set_popup_snapshotter(Box::new(move |state, surface, size, scale| {
        let (backend, scene) = (weak_backend.upgrade()?, weak_scene.upgrade()?);
        let mut backend = backend.try_borrow_mut().ok()?;
        crate::popup_anim::snapshot(&scene, backend.renderer(), state, surface, size, scale)
    }));
    let var = |key: &str| std::env::var_os(key);
    state.wallpaper = Wallpaper::start(
        &event_loop.handle(),
        desktop_config::wallpaper::config_file(var),
        desktop_config::wallpaper::dirs(var),
    )?;
    // The rings of the clicks shown; the host draws the pointer (lot C-01).
    state.watch_pointer(
        &event_loop.handle(),
        desktop_config::pointer::config_file(var),
    )?;
    // The layout and the repeat of Control Room's keyboard page (lot D-26c).
    state.watch_keyboard(
        &event_loop.handle(),
        desktop_config::keyboard::config_file(var),
    )?;
    // What is copied, for Super + V, and its setting (lot D-42).
    state.watch_clipboard_settings(
        &event_loop.handle(),
        desktop_config::clipboard::config_file(var),
    )?;
    // Shared with the screenshot tool (lot D-35e), which draws between two frames: nested,
    // Super + Shift + S is Alt + Shift + S, and Print Screen stays with the host session.
    let textures = Rc::new(RefCell::new(Textures::default()));
    state.enable_screenshots(crate::capture::Settings::default());
    let (weak_backend, weak_scene) = (Rc::downgrade(&backend), Rc::downgrade(&scene));
    let weak_textures = Rc::downgrade(&textures);
    state.set_capturer(Box::new(move |state, target| {
        let (backend, scene) = (weak_backend.upgrade()?, weak_scene.upgrade()?);
        let textures = weak_textures.upgrade()?;
        let mut backend = backend.try_borrow_mut().ok()?;
        let mut textures = textures.try_borrow_mut().ok()?;
        crate::capture::image(&scene, backend.renderer(), &mut textures, state, target)
    }));
    // The host frame is drawn in full every time: the identities only matter to the real
    // session's damage trackers.
    let mut ids = Ids::default();
    let mut preview = PreviewMotion::default();
    let mut cursor = Some(CursorIcon::Default);

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| {
            let mut backend = backend.borrow_mut();
            match event {
                WinitEvent::Resized { size, .. } => state.resize_frame(size),
                WinitEvent::Input(event) => {
                    state.process_input_event(event);
                    update_cursor(&backend, &mut cursor, state.cursor_icon());
                }
                WinitEvent::Redraw => {
                    // A client may change its cursor without the pointer moving.
                    update_cursor(&backend, &mut cursor, state.cursor_icon());
                    let mut textures = textures.borrow_mut();
                    let frame = (&*scene, &mut ids, &mut *textures, &mut preview);
                    let locked = state.is_locked();
                    match render(&mut backend, frame, state) {
                        // The host window shows every screen at once (lot D-25a).
                        Ok(()) if locked => {
                            for output in state.outputs() {
                                state.lock_frame_shown(&output);
                            }
                        }
                        Ok(()) => {}
                        Err(err) => tracing::warn!("render: {err}"),
                    }
                    send_frames(state);
                    backend.window().request_redraw();
                }
                WinitEvent::CloseRequested => state.loop_signal.stop(),
                WinitEvent::Focus(false) => state.forget_intercepted_keys(),
                WinitEvent::Focus(true) => {}
            }
        })
        .map_err(|err| err.error)?;
    Ok(())
}

/// Offers linux-dmabuf to clients, with the formats the renderer imports, on the chip it draws on.
/// Without a GPU (llvmpipe), or if EGL does not name its device, clients keep `wl_shm`.
fn enable_dmabuf(state: &mut State, backend: &Rc<RefCell<WinitGraphicsBackend<GlesRenderer>>>) {
    let (node, formats) = {
        let mut backend = backend.borrow_mut();
        let renderer = backend.renderer();
        match dmabuf::render_node(renderer) {
            Ok(node) => (node, renderer.dmabuf_formats()),
            Err(err) => {
                tracing::warn!("no linux-dmabuf, GPU clients will draw in software: {err}");
                return;
            }
        }
    };
    let codes: std::collections::BTreeSet<_> = formats.iter().map(|f| f.code.to_string()).collect();
    tracing::info!(
        node = %node.path.display(),
        driver = dmabuf::driver(node.dev).as_deref().unwrap_or("unknown"),
        formats = formats.iter().count(),
        codes = ?codes,
        "linux-dmabuf offered"
    );
    for format in formats.iter() {
        tracing::debug!(code = ?format.code, modifier = ?format.modifier, "dmabuf format");
    }
    let backend = Rc::clone(backend);
    let import: dmabuf::Importer = Box::new(move |buffer| {
        let mut backend = backend.try_borrow_mut()?;
        backend.renderer().import_dmabuf(buffer, None)?;
        Ok(())
    });
    if let Err(err) = state.enable_dmabuf(node.dev, formats, import) {
        tracing::warn!("no linux-dmabuf, the feedback table cannot be created: {err}");
    }
}

/// One frame per frame of the host window, for each surface: windows, anchored surfaces and
/// lock surfaces.
fn send_frames(state: &State) {
    let time = state.start_time.elapsed();
    let outputs = state.outputs();
    let throttle = Some(Duration::ZERO);
    if let Some(output) = outputs.first() {
        for window in state.space.elements() {
            state.send_window_frame(window, output, time);
        }
    }
    for output in &outputs {
        for layer in layer_map_for_output(output).layers() {
            layer.send_frame(output, time, throttle, |_, _| Some(output.clone()));
        }
        state.send_lock_frames(output, time);
        state.send_drag_frames(output, time);
    }
}

/// Gives the host window's cursor the wanted shape, if it changes.
fn update_cursor(
    backend: &WinitGraphicsBackend<GlesRenderer>,
    current: &mut Option<CursorIcon>,
    wanted: Option<CursorIcon>,
) {
    if *current == wanted {
        return;
    }
    let window = backend.window();
    match wanted {
        Some(icon) => {
            window.set_cursor(icon);
            window.set_cursor_visible(true);
        }
        None => window.set_cursor_visible(false),
    }
    *current = wanted;
}

/// Draws every screen into its slice of the host frame. Each element is drawn only inside its
/// screen's slice: a window across two screens is drawn twice, once at each scale.
fn render(
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    (scene, ids, textures, preview): (&Scene, &mut Ids, &mut Textures, &mut PreviewMotion),
    state: &mut State,
) -> Result<(), Box<dyn std::error::Error>> {
    let now = state.start_time.elapsed();
    let preview = preview.frame(now, state.snap_preview());
    let size = backend.window_size();
    let (animating, render_time) = {
        let (renderer, mut framebuffer) = backend.bind()?;
        // Counted after `bind`, which may wait for a buffer of the host window to be released.
        let started = Instant::now();
        let views = state.views();
        let (screens, animating) =
            scene.screens(renderer, ids, state, textures, &views, preview, now);
        // GLES draws upside down: the frame is flipped to make up for it. Redrawn in full every
        // time.
        let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
        frame.clear(
            color::palette().desktop.premultiplied().into(),
            &[Rectangle::from_size(size)],
        )?;
        for ((_, view), elements) in views.iter().zip(&screens) {
            for element in elements.iter().rev() {
                let dst = element.geometry(Scale::from(1.0));
                let Some(visible) = dst.intersection(view.frame) else {
                    continue;
                };
                let damage = [Rectangle::new(visible.loc - dst.loc, visible.size)];
                element.draw(&mut frame, element.src(), dst, &damage, &[])?;
            }
        }
        let sync = frame.finish()?;
        // When measuring, wait for the GPU to finish: the time counted is that of the whole
        // render, not only of sending the commands.
        if state.frame_stats.is_some() {
            sync.wait()?;
        }
        (animating, started.elapsed())
    };
    backend.submit(None)?;
    if let Some(stats) = state.frame_stats.as_mut()
        && let Some(period) = stats.record(state.start_time.elapsed(), render_time, animating)
    {
        tracing::info!(animation = animating, "measure: {period}");
    }

    Ok(())
}
