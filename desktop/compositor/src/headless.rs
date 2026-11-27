//! Without a screen (`--headless`, lot D-36): one virtual screen of 1440 × 900 per scale, and
//! nothing drawn. Real applications run against the compositor and its protocols, to check
//! what they ask for, without any window showing up anywhere. The windows still receive their
//! frame callbacks, at 60 per second, so that a toolkit that waits for them goes on.

use std::time::Duration;

use smithay::{
    desktop::layer_map_for_output,
    reexports::calloop::{
        EventLoop,
        timer::{TimeoutAction, Timer},
    },
};

use crate::{Error, State};

/// Size of each virtual screen, the reference screen of the boards.
const SCREEN: (i32, i32) = (1440, 900);
/// Time between two frame callbacks.
const FRAME: Duration = Duration::from_millis(16);

/// Creates one screen per scale in `scales`, and sends the frame callbacks.
pub fn init(
    event_loop: &EventLoop<'static, State>,
    state: &mut State,
    scales: &[f64],
) -> Result<(), Error> {
    for scale in scales {
        let size = (f64::from(SCREEN.0) * scale, f64::from(SCREEN.1) * scale);
        state.add_output(
            (size.0.round() as i32, size.1.round() as i32).into(),
            *scale,
        );
    }
    event_loop
        .handle()
        .insert_source(Timer::from_duration(FRAME), |_, _, state| {
            let time = state.now();
            let outputs: Vec<_> = state.space.outputs().cloned().collect();
            for output in &outputs {
                let windows: Vec<_> = state.space.elements_for_output(output).cloned().collect();
                for window in &windows {
                    state.send_window_frame(window, output, time);
                }
                for layer in layer_map_for_output(output).layers() {
                    layer.send_frame(output, time, Some(Duration::ZERO), |_, _| {
                        Some(output.clone())
                    });
                }
            }
            TimeoutAction::ToDuration(FRAME)
        })
        .map_err(|err| err.error)?;
    tracing::info!(screens = scales.len(), "without a screen: nothing is drawn");
    Ok(())
}
