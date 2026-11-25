//! Backend fenêtré : le compositeur s'affiche dans une fenêtre d'une session Wayland ou X11.

use std::time::Duration;

use sherlock_ui::window as ui;
use smithay::{
    backend::{
        renderer::{
            Color32F,
            damage::OutputDamageTracker,
            element::{
                Kind,
                solid::{SolidColorBuffer, SolidColorRenderElement},
            },
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    desktop::space::render_output,
    input::pointer::CursorIcon,
    output::{Mode, Output},
    reexports::calloop::EventLoop,
    utils::{Logical, Rectangle, Transform},
};

use crate::{Error, State};

/// Ouvre la fenêtre, crée l'écran qui lui correspond et branche le rendu sur la boucle.
pub fn init(event_loop: &EventLoop<'static, State>, state: &mut State) -> Result<(), Error> {
    let (mut backend, winit) = winit::init::<GlesRenderer>()?;
    state.nested = true;
    // GLES dessine la tête en bas : l'écran est retourné pour compenser.
    let output = state.add_output(backend.window_size(), Transform::Flipped180);
    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut preview = SnapPreview::default();
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
            WinitEvent::Focus(_) => {}
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

/// Aperçu du calage : un aplat et sa bordure. Les tampons restent d'une image à l'autre pour
/// que seul ce qui change soit redessiné.
#[derive(Default)]
struct SnapPreview {
    fill: SolidColorBuffer,
    borders: [SolidColorBuffer; 4],
}

impl SnapPreview {
    /// Éléments à dessiner pour l'aperçu `rect`, donné dans les coordonnées de l'écran.
    fn elements(&mut self, rect: Rectangle<i32, Logical>) -> Vec<SolidColorRenderElement> {
        let fill = Color32F::from(ui::SNAP_PREVIEW_FILL.premultiplied());
        let border = Color32F::from(ui::SNAP_PREVIEW_BORDER.premultiplied());
        let (x, y, w, h) = (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
        let b = ui::SNAP_PREVIEW_BORDER_WIDTH.min(w / 2).min(h / 2);
        // Haut, bas, gauche, droite : les côtés ne recouvrent pas les coins.
        let sides = [
            (x, y, w, b),
            (x, y + h - b, w, b),
            (x, y + b, b, h - 2 * b),
            (x + w - b, y + b, b, h - 2 * b),
        ];
        let parts = self
            .borders
            .iter_mut()
            .zip(sides)
            .map(|(buffer, side)| (buffer, side, border))
            .chain([(&mut self.fill, (x + b, y + b, w - 2 * b, h - 2 * b), fill)]);
        parts
            .map(|(buffer, (x, y, w, h), color)| {
                buffer.update((w, h), color);
                // L'écran est à l'échelle 1 : pixels logiques et physiques se confondent.
                SolidColorRenderElement::from_buffer(buffer, (x, y), 1.0, 1.0, Kind::Unspecified)
            })
            .collect()
    }
}

fn render(
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    preview: &mut SnapPreview,
    state: &State,
) -> Result<(), Box<dyn std::error::Error>> {
    let origin = state.space.output_geometry(output).map(|g| g.loc);
    let elements = match (state.snap_preview(), origin) {
        (Some(rect), Some(origin)) => {
            preview.elements(Rectangle::new(rect.loc - origin, rect.size))
        }
        _ => Vec::new(),
    };
    {
        let (renderer, mut framebuffer) = backend.bind()?;
        // Fond noir provisoire : la couleur du bureau viendra de sherlock-ui (lot D-01).
        // Les éléments ajoutés, ici l'aperçu du calage, passent devant les fenêtres.
        render_output::<_, SolidColorRenderElement, _, _>(
            output,
            renderer,
            &mut framebuffer,
            1.0,
            0,
            [&state.space],
            &elements,
            damage_tracker,
            Color32F::BLACK,
        )?;
    }
    backend.submit(None)?;
    Ok(())
}
