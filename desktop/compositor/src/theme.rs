//! The theme of the appearance file, followed live (lots D-08 and D-05c): the palette of
//! everything the compositor draws fades to the new theme, the default wallpaper crossfades to
//! its pendant (`wallpaper`), and the real session's cursor takes the house theme of the new
//! theme (`cursor`). See "Theme" in `docs/compositor.md`.

use std::{path::PathBuf, time::Instant};

use sherlock_ui::{
    color::{Theme, ThemeFade},
    wallpapers::ThemeVariant,
};
use smithay::reexports::calloop::{LoopHandle, channel};

use crate::State;

/// The theme the compositor shows, and the fade towards it.
#[derive(Debug, Default)]
pub struct Themes {
    /// The theme of the appearance file, dark until it is read.
    target: Theme,
    fade: ThemeFade,
}

/// The wallpapers made for `theme`.
pub fn variant(theme: Theme) -> ThemeVariant {
    match theme {
        Theme::Dark => ThemeVariant::Dark,
        Theme::Light => ThemeVariant::Light,
    }
}

/// Makes the theme of the appearance file active at once, then follows it. Without the file
/// (no `XDG_CONFIG_HOME` nor `HOME`), the dark theme.
pub fn follow(handle: &LoopHandle<'static, State>, state: &mut State) {
    if let Some(path) = appearance_config::path() {
        follow_at(handle, state, path);
    }
}

/// `follow`, for the appearance file at `path`: the tests give a file of their own.
pub fn follow_at(handle: &LoopHandle<'static, State>, state: &mut State, path: PathBuf) {
    state.set_theme(appearance_config::theme_now(&path));
    let (sender, themes) = channel::channel();
    let source = handle.insert_source(themes, |event, _, state| {
        if let channel::Event::Msg(theme) = event {
            state.set_theme(theme);
            // The theme comes again after a change of the Accessibility settings, already
            // applied to the process (lot D-26p): what the compositor draws follows them.
            state.queue_redraw();
        }
    });
    if let Err(err) = source {
        tracing::warn!("following the theme: {}", err.error);
        return;
    }
    let follow = appearance_config::follow_themes(path, move |theme| sender.send(theme).is_ok());
    if let Err(err) = follow {
        tracing::warn!("following the theme: {err}");
    }
}

impl State {
    /// Heads for `theme`: at once the first time, with a fade of `motion::THEME_FADE` after.
    /// The wallpaper that follows the theme changes to its pendant.
    pub fn set_theme(&mut self, theme: Theme) {
        if self.themes.target != theme {
            tracing::info!(?theme, "theme");
        }
        self.themes.target = theme;
        self.themes.fade.set(theme, Instant::now());
        self.wallpaper.set_theme(variant(theme));
        self.queue_redraw();
    }

    /// The theme the compositor heads for: the cursor's.
    pub fn theme(&self) -> Theme {
        self.themes.target
    }

    /// Shows the fade between the palettes at this instant; called before each frame is built.
    /// Returns whether it still runs: a frame is then due at the next blank. Once it is over,
    /// nothing asks for one.
    pub fn tick_theme(&mut self) -> bool {
        self.themes.fade.tick(Instant::now())
    }
}
