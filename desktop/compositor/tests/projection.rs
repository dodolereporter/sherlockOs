//! Super + P (lot D-39) on two virtual screens, without a backend: the chooser opens on the
//! choice after the current one, Super + P again and the arrows move it, Enter applies it; each
//! of the four choices gives its configuration (Duplicate scales the second screen to show the
//! whole of the first); the second screen is projected on as last time when it comes back; Do
//! not disturb is switched for the time of the projection; unplugged while the laptop's screen
//! is off, the laptop's lights again.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use compositor::{State, render};
    use desktop_config::displays::Projection;
    use smithay::{
        backend::input::KeyState,
        input::keyboard::Keycode,
        output::Output,
        reexports::calloop::EventLoop,
        utils::{Logical, Rectangle},
    };

    const KEY_SUPER: u32 = 125 + 8;
    const KEY_P: u32 = 25 + 8;
    const KEY_RIGHT: u32 = 106 + 8;
    const KEY_ENTER: u32 = 28 + 8;
    const KEY_ESCAPE: u32 = 1 + 8;
    const KEY_SPACE: u32 = 57 + 8;

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        dir: PathBuf,
        time: u32,
    }

    impl Harness {
        /// The laptop's screen, 1920 × 1200, and a projector of 1920 × 1080 to its right.
        fn start(name: &str) -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((1920, 1200).into(), 1.0);
            state.add_output((1920, 1080).into(), 1.0);
            let dir =
                std::env::temp_dir().join(format!("projection-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self {
                event_loop,
                state,
                dir,
                time: 0,
            }
        }

        fn turn(&mut self) {
            for _ in 0..3 {
                self.event_loop
                    .dispatch(Some(Duration::from_millis(1)), &mut self.state)
                    .unwrap();
            }
        }

        fn key(&mut self, code: u32, pressed: bool) {
            self.time += 1;
            let state = if pressed {
                KeyState::Pressed
            } else {
                KeyState::Released
            };
            self.state
                .keyboard_key(Keycode::new(code), state, self.time);
        }

        fn tap(&mut self, code: u32) {
            self.key(code, true);
            self.key(code, false);
        }

        fn outputs(&self) -> Vec<Output> {
            self.state.space.outputs().cloned().collect()
        }

        fn geometry(&self, name: &str) -> Option<Rectangle<i32, Logical>> {
            let output = self.outputs().into_iter().find(|o| o.name() == name)?;
            self.state.space.output_geometry(&output)
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn super_p_opens_moves_and_applies_the_choice() {
        let mut h = Harness::start("keys");
        assert_eq!(h.state.projection_now(), Some(Projection::Extend));
        // Super + P: the choice after the current one.
        h.key(KEY_SUPER, true);
        h.tap(KEY_P);
        assert_eq!(h.state.projection_selected(), Some(Projection::Second));
        // Again, Super held: around to the laptop's screen alone; the arrow, to Duplicate.
        h.tap(KEY_P);
        h.key(KEY_SUPER, false);
        assert_eq!(h.state.projection_selected(), Some(Projection::Laptop));
        h.tap(KEY_RIGHT);
        assert_eq!(h.state.projection_selected(), Some(Projection::Duplicate));
        // Drawn on each screen, its four choices named.
        let output = h.outputs()[0].clone();
        let views = h.state.views();
        let (items, animating) =
            render::items(&h.state, (&output, &views[0].1), None, false, h.state.now());
        let drawn = format!("{items:?}");
        for name in [
            "Écran du portable seul",
            "Dupliquer",
            "Étendre",
            "Second écran seul",
        ] {
            assert!(drawn.contains(name), "{name}");
        }
        assert!(animating, "the highlight slides");
        // Enter: duplicated; the projector, at 0.75, shows the whole of the laptop's screen.
        h.tap(KEY_ENTER);
        assert!(!h.state.projection_open());
        assert_eq!(h.state.projection_now(), Some(Projection::Duplicate));
        assert_eq!(
            h.geometry("virtual-1"),
            Some(Rectangle::from_size((1920, 1200).into()))
        );
        let projector = h.geometry("virtual-2").unwrap();
        assert_eq!(projector.size, (2560, 1440).into());
        assert_eq!(projector.loc, (-320, -120).into());
        // Escape closes without a change.
        h.key(KEY_SUPER, true);
        h.tap(KEY_P);
        h.key(KEY_SUPER, false);
        h.tap(KEY_ESCAPE);
        assert!(!h.state.projection_open());
        assert_eq!(h.state.projection_now(), Some(Projection::Duplicate));
    }

    #[test]
    fn each_choice_gives_its_screens_and_the_laptop_comes_back() {
        let mut h = Harness::start("choices");
        h.state.project(Projection::Laptop, false);
        assert_eq!(h.outputs().len(), 1);
        assert_eq!(h.state.projection_now(), Some(Projection::Laptop));
        h.state.project(Projection::Extend, false);
        assert_eq!(h.geometry("virtual-2").unwrap().loc, (1920, 0).into());
        h.state.project(Projection::Second, false);
        let lit: Vec<String> = h.outputs().iter().map(Output::name).collect();
        assert_eq!(lit, ["virtual-2"]);
        // The projector unplugged while the laptop's screen is off: it lights again.
        let projector = h.outputs()[0].clone();
        h.state.remove_screen(&projector);
        h.turn();
        let lit: Vec<String> = h.outputs().iter().map(Output::name).collect();
        assert_eq!(lit, ["virtual-1"], "never black");
        // A single screen: Super + P opens nothing.
        h.state.toggle_projection();
        assert!(!h.state.projection_open());
    }

    #[test]
    fn the_projector_is_projected_on_as_last_time_and_quiet_meanwhile() {
        let mut h = Harness::start("saved");
        let displays = h.dir.join("displays.conf");
        let notifications = h.dir.join("notifications.conf");
        h.state.remember_screens(Some(displays.clone()));
        h.state
            .set_projection_notifications(Some(notifications.clone()));
        // Through the panel, Do not disturb proposed: Space would turn it off, twice keeps it.
        h.state.toggle_projection();
        h.tap(KEY_SPACE);
        h.tap(KEY_SPACE);
        while h.state.projection_selected() != Some(Projection::Duplicate) {
            h.tap(KEY_RIGHT);
        }
        h.tap(KEY_ENTER);
        let quiet = std::fs::read_to_string(&notifications).unwrap();
        assert!(quiet.contains("DoNotDisturb=true"), "{quiet}");
        h.state.save_screens();
        h.turn();
        std::thread::sleep(Duration::from_millis(100));
        let saved = std::fs::read_to_string(&displays).unwrap();
        assert!(saved.contains("projection = duplicate"), "{saved}");
        // Unplugged, plugged again: duplicated again, without a key.
        let projector = h.outputs()[1].clone();
        h.state.remove_screen(&projector);
        h.state.add_output((1920, 1080).into(), 1.0);
        h.turn();
        assert_eq!(h.state.projection_now(), Some(Projection::Duplicate));
        // Back to the laptop's screen alone: Do not disturb goes off again.
        h.state.project(Projection::Laptop, true);
        let quiet = std::fs::read_to_string(&notifications).unwrap();
        assert!(quiet.contains("DoNotDisturb=false"), "{quiet}");
    }
}
