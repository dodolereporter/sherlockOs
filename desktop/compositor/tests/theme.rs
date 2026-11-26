//! The theme of the appearance file, followed live (lot D-05c): rewriting `Theme=` changes the
//! palette and the default wallpaper without restarting the compositor. Its own process: the
//! palette shown is global to it.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };

    use compositor::{State, theme, wallpaper::Wallpaper};
    use sherlock_ui::color::{self, Palette, Theme};
    use smithay::reexports::calloop::EventLoop;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("theme-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes the appearance file as a settings program does: a new file renamed over it.
    fn write_theme(path: &Path, theme: &str) {
        let new = path.with_extension("new");
        fs::write(&new, format!("[Appearance]\nTheme={theme}\n")).unwrap();
        fs::rename(&new, path).unwrap();
    }

    /// Runs the loop until `done`, five seconds at most.
    fn run_until(
        event_loop: &mut EventLoop<'static, State>,
        state: &mut State,
        what: &str,
        done: impl Fn(&mut State) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            event_loop
                .dispatch(Some(Duration::from_millis(10)), state)
                .unwrap();
            if done(state) {
                return;
            }
        }
        panic!("timed out waiting for: {what}");
    }

    #[test]
    fn a_new_theme_in_the_file_applies_without_restarting() {
        let dir = scratch("live");
        let file = dir.join("appearance.conf");
        write_theme(&file, "dark");
        // The default pair, installed with one variant each (their content is not read here).
        let data = dir.join("data");
        for id in ["first-clue", "first-clue-light"] {
            let folder = data.join("wallpapers").join(id);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join("1920x1080.png"), b"").unwrap();
        }
        let mut event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.add_output((1440, 900).into(), 1.0);
        let handle = event_loop.handle();
        theme::follow_at(&handle, &mut state, file.clone());
        state.wallpaper = Wallpaper::start(&handle, None, vec![data.clone()]).unwrap();
        assert_eq!(state.theme(), Theme::Dark);
        assert_eq!(color::palette(), &Palette::DARK);
        let screen = (1440, 900).into();
        let dark = data.join("wallpapers/first-clue/1920x1080.png");
        assert_eq!(state.wallpaper.image_for(screen), Some(dark.clone()));

        // Light: the colors fade to the light palette, the wallpaper changes to its pendant.
        let epoch = state.redraw_epoch();
        write_theme(&file, "light");
        run_until(&mut event_loop, &mut state, "the light theme", |s| {
            s.theme() == Theme::Light
        });
        assert!(state.redraw_epoch() > epoch, "a frame is asked for");
        let light = data.join("wallpapers/first-clue-light/1920x1080.png");
        assert_eq!(state.wallpaper.image_for(screen), Some(light));
        assert!(state.tick_theme(), "the palettes fade");
        run_until(&mut event_loop, &mut state, "the end of the fade", |s| {
            !s.tick_theme()
        });
        assert_eq!(color::palette(), &Palette::LIGHT);

        // And back to dark.
        write_theme(&file, "dark");
        run_until(&mut event_loop, &mut state, "the dark theme", |s| {
            s.theme() == Theme::Dark && !s.tick_theme()
        });
        assert_eq!(color::palette(), &Palette::DARK);
        assert_eq!(state.wallpaper.image_for(screen), Some(dark));
        state.shutdown();
        fs::remove_dir_all(&dir).unwrap();
    }
}
