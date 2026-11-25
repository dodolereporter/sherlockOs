//! Backend fenêtré : le compositeur s'affiche dans une fenêtre d'une session Wayland ou X11.

use std::time::Duration;

use smithay::{
    backend::{
        renderer::{
            Color32F, damage::OutputDamageTracker, element::surface::WaylandSurfaceRenderElement,
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    desktop::space::render_output,
    output::{Mode, Output},
    reexports::calloop::EventLoop,
    utils::Transform,
};

use crate::{Error, State};

/// Ouvre la fenêtre, crée l'écran qui lui correspond et branche le rendu sur la boucle.
pub fn init(event_loop: &EventLoop<'static, State>, state: &mut State) -> Result<(), Error> {
    let (mut backend, winit) = winit::init::<GlesRenderer>()?;
    state.nested = true;
    // GLES dessine la tête en bas : l'écran est retourné pour compenser.
    let output = state.add_output(backend.window_size(), Transform::Flipped180);
    let mut damage_tracker = OutputDamageTracker::from_output(&output);

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
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => {
                if let Err(err) = render(&mut backend, &output, &mut damage_tracker, state) {
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
            WinitEvent::Focus(_) => {}
        })
        .map_err(|err| err.error)?;
    Ok(())
}

fn render(
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    state: &State,
) -> Result<(), Box<dyn std::error::Error>> {
    {
        let (renderer, mut framebuffer) = backend.bind()?;
        // Fond noir provisoire : la couleur du bureau viendra de sherlock-ui (lot D-01).
        render_output::<_, WaylandSurfaceRenderElement<GlesRenderer>, _, _>(
            output,
            renderer,
            &mut framebuffer,
            1.0,
            0,
            [&state.space],
            &[],
            damage_tracker,
            Color32F::BLACK,
        )?;
    }
    backend.submit(None)?;
    Ok(())
}
