//! Backend fenêtré : le compositeur s'affiche dans une fenêtre d'une session Wayland ou X11.

use std::time::{Duration, Instant};

use sherlock_ui::color;
use smithay::{
    backend::{
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    input::pointer::CursorIcon,
    output::{Mode, Output},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

use crate::{Error, State, anim::PreviewMotion, render::Scene};

/// Ouvre la fenêtre, crée l'écran qui lui correspond et branche le rendu sur la boucle.
pub fn init(event_loop: &EventLoop<'static, State>, state: &mut State) -> Result<(), Error> {
    let (mut backend, winit) = winit::init::<GlesRenderer>()?;
    state.nested = true;
    // GLES dessine la tête en bas : l'écran est retourné pour compenser.
    let output = state.add_output(backend.window_size(), Transform::Flipped180);
    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let scene = Scene::new(backend.renderer())?;
    let mut preview = PreviewMotion::default();
    let mut cursor = Some(CursorIcon::Default);

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                let mode = Mode {
                    size,
                    refresh: 60_000,
                };
                output.change_current_state(Some(mode), None, None, None);
                output.set_preferred(mode);
            }
            WinitEvent::Input(event) => {
                state.process_input_event(event);
                update_cursor(&backend, &mut cursor, state.cursor_icon());
            }
            WinitEvent::Redraw => {
                // Un client peut changer de curseur sans que le pointeur bouge.
                update_cursor(&backend, &mut cursor, state.cursor_icon());
                let rendered = render(
                    &mut backend,
                    &output,
                    &mut damage_tracker,
                    &scene,
                    &mut preview,
                    state,
                );
                if let Err(err) = rendered {
                    tracing::warn!("rendu : {err}");
                }
                let time = state.start_time.elapsed();
                for window in state.space.elements() {
                    window.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                        Some(output.clone())
                    });
                }
                backend.window().request_redraw();
            }
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            WinitEvent::Focus(false) => state.forget_intercepted_keys(),
            WinitEvent::Focus(true) => {}
        })
        .map_err(|err| err.error)?;
    Ok(())
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

fn render(
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    scene: &Scene,
    preview: &mut PreviewMotion,
    state: &mut State,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(area) = state.space.output_geometry(output) else {
        return Ok(());
    };
    let now = state.start_time.elapsed();
    let zone = state
        .snap_preview()
        .map(|rect| Rectangle::new(rect.loc - area.loc, rect.size));
    let preview = preview.frame(now, zone);
    let (animating, render_time) = {
        let (renderer, mut framebuffer) = backend.bind()?;
        // Compté après `bind`, qui peut attendre qu'un tampon de la fenêtre hôte se libère.
        let started = Instant::now();
        let (elements, animating) = scene.elements(renderer, state, area, preview, now);
        // Âge 0 : l'image est redessinée en entier à chaque fois.
        let result = damage_tracker.render_output(
            renderer,
            &mut framebuffer,
            0,
            &elements,
            color::DESKTOP.premultiplied(),
        )?;
        // En mesure, on attend que la puce graphique ait fini : le temps compté est celui du
        // rendu entier, pas seulement celui de l'envoi des commandes.
        if state.frame_stats.is_some() {
            result.sync.wait()?;
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
