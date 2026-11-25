//! Backend fenêtré : le compositeur s'affiche dans une fenêtre d'une session Wayland ou X11.

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

use crate::{Error, State, anim::PreviewMotion, render::Scene};

/// Ouvre la fenêtre, crée un écran par échelle de `scales`, côte à côte dans la fenêtre, et
/// branche le rendu sur la boucle.
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
                // Un client peut changer de curseur sans que le pointeur bouge.
                update_cursor(&backend, &mut cursor, state.cursor_icon());
                if let Err(err) = render(&mut backend, &scene, &mut preview, state) {
                    tracing::warn!("rendu : {err}");
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

/// Une image par image de la fenêtre hôte, pour chaque surface : fenêtres et surfaces ancrées.
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

/// Donne au curseur de la fenêtre hôte la forme voulue, s'il en change.
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
    scene: &Scene,
    preview: &mut PreviewMotion,
    state: &mut State,
) -> Result<(), Box<dyn std::error::Error>> {
    let now = state.start_time.elapsed();
    let preview = preview.frame(now, state.snap_preview());
    let size = backend.window_size();
    let (animating, render_time) = {
        let (renderer, mut framebuffer) = backend.bind()?;
        // Compté après `bind`, qui peut attendre qu'un tampon de la fenêtre hôte se libère.
        let started = Instant::now();
        let mut animating = false;
        let mut screens = Vec::new();
        for (output, view) in state.views() {
            let (elements, moving) = scene.elements(renderer, state, &output, &view, preview, now);
            animating |= moving;
            screens.push((view.frame, elements));
        }
        // GLES dessine la tête en bas : l'image est retournée pour compenser. Redessinée en
        // entier à chaque fois.
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
        // En mesure, on attend que la puce graphique ait fini : le temps compté est celui du
        // rendu entier, pas seulement celui de l'envoi des commandes.
        if state.frame_stats.is_some() {
            sync.wait()?;
        }
        (animating, started.elapsed())
    };
    backend.submit(None)?;
    if let Some(stats) = state.frame_stats.as_mut()
        && let Some(period) = stats.record(state.start_time.elapsed(), render_time, animating)
    {
        tracing::info!(animation = animating, "mesure : {period}");
    }

    Ok(())
}
