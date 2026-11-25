//! Windowed backend: the compositor shows up in a window of a Wayland or X11 session.

use std::time::{Duration, Instant};

use sherlock_ui::color;
use smithay::{
    backend::{
        renderer::{
            Frame as _, Renderer as _, element::Element as _, element::RenderElement as _,
            gles::GlesRenderer,
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
    render::Scene,
    wallpaper::{Textures, Wallpaper},
};

/// Opens the window, creates one screen per scale in `scales`, side by side in the window, and
/// plugs rendering into the loop.
pub fn init(
    event_loop: &EventLoop<'static, State>,
    state: &mut State,
    scales: &[f64],
) -> Result<(), Error> {
    let (mut backend, winit) = winit::init::<GlesRenderer>()?;
    state.nested = true;
    for scale in scales {
        state.add_output(backend.window_size(), *scale);
    }
    state.resize_frame(backend.window_size());
    let scene = Scene::new(backend.renderer())?;
    let var = |key: &str| std::env::var_os(key);
    state.wallpaper = Wallpaper::start(
        &event_loop.handle(),
        desktop_config::wallpaper::config_file(var),
        desktop_config::data_dirs(var),
    )?;
    let mut textures = Textures::default();
    let mut preview = PreviewMotion::default();
    let mut cursor = Some(CursorIcon::Default);

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => state.resize_frame(size),
            WinitEvent::Input(event) => {
                state.process_input_event(event);
                update_cursor(&backend, &mut cursor, state.cursor_icon());
            }
            WinitEvent::Redraw => {
                // A client may change its cursor without the pointer moving.
                update_cursor(&backend, &mut cursor, state.cursor_icon());
                let frame = (&scene, &mut textures, &mut preview);
                if let Err(err) = render(&mut backend, frame, state) {
                    tracing::warn!("render: {err}");
                }
                send_frames(state);
                backend.window().request_redraw();
            }
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            WinitEvent::Focus(false) => state.forget_intercepted_keys(),
            WinitEvent::Focus(true) => {}
        })
        .map_err(|err| err.error)?;
    Ok(())
}

/// One frame per frame of the host window, for each surface: windows and anchored surfaces.
fn send_frames(state: &State) {
    let time = state.start_time.elapsed();
    let outputs = state.outputs();
    let throttle = Some(Duration::ZERO);
    if let Some(output) = outputs.first() {
        for window in state.space.elements() {
            window.send_frame(output, time, throttle, |_, _| Some(output.clone()));
        }
    }
    for output in &outputs {
        for layer in layer_map_for_output(output).layers() {
            layer.send_frame(output, time, throttle, |_, _| Some(output.clone()));
        }
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
    (scene, textures, preview): (&Scene, &mut Textures, &mut PreviewMotion),
    state: &mut State,
) -> Result<(), Box<dyn std::error::Error>> {
    let now = state.start_time.elapsed();
    let preview = preview.frame(now, state.snap_preview());
    let size = backend.window_size();
    let (animating, render_time) = {
        let (renderer, mut framebuffer) = backend.bind()?;
        // Counted after `bind`, which may wait for a buffer of the host window to be released.
        let started = Instant::now();
        let mut animating = false;
        let mut screens = Vec::new();
        // Each screen's wallpaper, by its size in pixels: a texture per variant, uploaded once.
        textures.update(renderer, &mut state.wallpaper);
        let mut used = Vec::new();
        for (output, view) in state.views() {
            let image = state.wallpaper.image_for(view.frame.size);
            let wallpaper = image.as_deref().and_then(|path| textures.get(path));
            let screen = (&output, &view);
            let (elements, moving) =
                scene.elements(renderer, state, screen, preview, wallpaper, now);
            animating |= moving;
            screens.push((view.frame, elements));
            used.extend(image);
        }
        textures.retain(&used);
        // GLES draws upside down: the frame is flipped to make up for it. Redrawn in full every
        // time.
        let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
        frame.clear(
            color::DESKTOP.premultiplied().into(),
            &[Rectangle::from_size(size)],
        )?;
        for (slice, elements) in &screens {
            for element in elements.iter().rev() {
                let dst = element.geometry(Scale::from(1.0));
                let Some(visible) = dst.intersection(*slice) else {
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
