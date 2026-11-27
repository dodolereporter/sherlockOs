//! The screens changed as a whole configuration (ADR 0025, lot D-26b): checked before anything
//! changes, applied, saved per screen and taken again when the screen comes back. Two virtual
//! screens without a host window, which can take several modes and turn.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use compositor::{
        State,
        displays::{Refused, Setting, Wanted},
    };
    use desktop_config::displays::{Config, Identity};
    use smithay::{
        output::Mode,
        reexports::calloop::EventLoop,
        utils::{Point, Transform},
    };

    struct Harness {
        _event_loop: EventLoop<'static, State>,
        state: State,
    }

    impl Harness {
        /// Two virtual screens of 1440 × 900.
        fn start() -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((1440, 900).into(), 1.0);
            state.add_output((1440, 900).into(), 1.0);
            Self {
                _event_loop: event_loop,
                state,
            }
        }
    }

    #[test]
    fn a_screen_switched_off_leaves_the_space_and_comes_back() {
        let mut h = Harness::start();
        let second = h.state.outputs()[1].clone();
        let off = Wanted {
            output: second.clone(),
            on: None,
        };
        h.state.apply_screens(&[off]).unwrap();
        assert_eq!(h.state.outputs().len(), 1);
        let heads = h.state.heads();
        assert_eq!(heads.len(), 2);
        assert!(heads[1].1.is_none());
        let on = Wanted {
            output: second,
            on: Some(Setting::default()),
        };
        h.state.apply_screens(&[on]).unwrap();
        assert_eq!(h.state.outputs().len(), 2);
    }

    #[test]
    fn a_wrong_configuration_is_refused_and_changes_nothing() {
        let mut h = Harness::start();
        let [first, second] = [0, 1].map(|i| h.state.outputs()[i].clone());
        let on = |output: &smithay::output::Output, setting: Setting| Wanted {
            output: output.clone(),
            on: Some(setting),
        };
        let off = |output: &smithay::output::Output| Wanted {
            output: output.clone(),
            on: None,
        };
        let odd = Setting {
            mode: Some(Mode {
                size: (1234, 567).into(),
                refresh: 60_000,
            }),
            ..Setting::default()
        };
        assert!(matches!(
            h.state.apply_screens(&[on(&first, odd)]),
            Err(Refused::Mode(_))
        ));
        assert_eq!(
            h.state.apply_screens(&[off(&first), off(&second)]),
            Err(Refused::AllOff)
        );
        let huge = Setting {
            scale: Some(9.0),
            ..Setting::default()
        };
        assert_eq!(
            h.state.apply_screens(&[on(&first, huge)]),
            Err(Refused::Scale(9.0))
        );
        assert_eq!(first.current_scale().fractional_scale(), 1.0);
        assert_eq!(h.state.outputs().len(), 2);
    }

    #[test]
    fn a_turned_screen_swaps_its_sides() {
        let mut h = Harness::start();
        let first = h.state.outputs()[0].clone();
        let turned = Wanted {
            output: first.clone(),
            on: Some(Setting {
                transform: Some(Transform::_90),
                position: Some(Point::from((0, 0))),
                ..Setting::default()
            }),
        };
        h.state.apply_screens(&[turned]).unwrap();
        let area = h.state.space.output_geometry(&first).unwrap();
        assert_eq!(area.size, (900, 1440).into());
        // The other screen did not move: it was placed by its first configuration.
        let second = h.state.outputs()[1].clone();
        assert_eq!(h.state.space.output_geometry(&second).unwrap().loc.x, 1440);
    }

    #[test]
    fn the_setting_is_saved_per_screen_and_taken_back() {
        let dir = std::env::temp_dir().join(format!("displays-{}", std::process::id()));
        let path = dir.join("displays.conf");
        let mut h = Harness::start();
        h.state.remember_screens(Some(path.clone()));
        let first = h.state.outputs()[0].clone();
        let wanted = Wanted {
            output: first.clone(),
            on: Some(Setting {
                scale: Some(1.5),
                position: Some(Point::from((100, 50))),
                ..Setting::default()
            }),
        };
        h.state.apply_screens(&[wanted]).unwrap();
        h.state.save_screens();
        // Written off the loop.
        let mut saved = None;
        for _ in 0..200 {
            if let Ok((config, _)) = Config::read(&path)
                && !config.screens.is_empty()
            {
                saved = Some(config);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let saved = saved.expect("displays.conf written");
        let identity = Identity::new("compositor", "virtual", "0");
        let screen = saved.get(&identity).unwrap();
        assert_eq!((screen.scale, screen.position), (1.5, (100, 50)));

        // A new session finds it for the same screen.
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.remember_screens(Some(path));
        let output = state.add_output((1440, 900).into(), 1.0);
        assert_eq!(output.current_scale().fractional_scale(), 1.5);
        let area = state.space.output_geometry(&output).unwrap();
        assert_eq!(area.loc, (100, 50).into());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
