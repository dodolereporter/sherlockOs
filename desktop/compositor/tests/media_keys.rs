//! The media keys against fake services (lot D-35): a fake `pactl` that keeps its volume in a
//! file, and a private bus started by the test (its own `dbus-daemon`, never the machine's
//! buses) with a fake logind, whose `SetBrightness` writes a fake sysfs folder, and a fake media
//! player. The keys go through `State::keyboard_key` as the real session sends them; what they
//! changed shows in the on-screen display.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use compositor::{
        State,
        media_keys::{Controls, REPEAT_DELAY, REPEAT_INTERVAL},
        osd::Level,
    };
    use sherlock_ui::osd::Kind;
    use smithay::{
        backend::input::KeyState, input::keyboard::Keycode, reexports::calloop::EventLoop,
    };
    use system_services::audio::{Mixer, Pactl};

    // xkb codes: the evdev codes of linux/input-event-codes.h, plus 8.
    const VOLUME_UP: u32 = 115 + 8;
    const MUTE: u32 = 113 + 8;
    const MIC_MUTE: u32 = 248 + 8;
    const BRIGHTNESS_UP: u32 = 225 + 8;
    const PLAY_PAUSE: u32 = 164 + 8;

    /// A private bus: its own `dbus-daemon`, socket in the test folder, killed on drop.
    struct Bus {
        daemon: Child,
        address: String,
        served: Vec<zbus::blocking::Connection>,
    }

    impl Bus {
        fn start(dir: &Path) -> Self {
            let config = dir.join("bus.conf");
            std::fs::write(
                &config,
                format!(
                    "<busconfig><type>session</type><listen>unix:dir={}</listen>\
                     <auth>EXTERNAL</auth><policy context=\"default\">\
                     <allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/>\
                     <allow own=\"*\"/></policy></busconfig>",
                    dir.display()
                ),
            )
            .unwrap();
            let mut daemon = Command::new("dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
                .args(["--nofork", "--print-address"])
                .stdout(Stdio::piped())
                .spawn()
                .expect("dbus-daemon (package dbus-daemon) is needed by these tests");
            let mut address = String::new();
            let stdout = daemon.stdout.take().unwrap();
            BufReader::new(stdout).read_line(&mut address).unwrap();
            Self {
                daemon,
                address: address.trim().to_owned(),
                served: Vec::new(),
            }
        }

        fn serve<I: zbus::object_server::Interface>(&mut self, name: &str, path: &str, object: I) {
            let connection = zbus::blocking::connection::Builder::address(self.address.as_str())
                .unwrap()
                .serve_at(path, object)
                .unwrap()
                .name(name.to_owned())
                .unwrap()
                .build()
                .unwrap();
            self.served.push(connection);
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            self.served.clear();
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    type Calls = Arc<Mutex<Vec<String>>>;

    /// logind's session: `SetBrightness` writes the fake sysfs file.
    struct Session {
        sysfs: PathBuf,
        calls: Calls,
    }

    #[zbus::interface(name = "org.freedesktop.login1.Session")]
    impl Session {
        fn set_brightness(&self, subsystem: String, name: String, value: u32) {
            let file = self.sysfs.join(&name).join("brightness");
            std::fs::write(file, value.to_string()).unwrap();
            self.calls
                .lock()
                .unwrap()
                .push(format!("{subsystem} {name} {value}"));
        }
    }

    /// A media player that plays.
    struct Player {
        calls: Calls,
    }

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Player {
        fn play_pause(&self) {
            self.calls.lock().unwrap().push(String::from("PlayPause"));
        }

        #[zbus(property)]
        fn playback_status(&self) -> String {
            String::from("Playing")
        }
    }

    /// A fake `pactl` that keeps the volume and the mute switches in files of `dir`, and logs
    /// each call.
    fn fake_pactl(dir: &Path) -> Pactl {
        let d = dir.display();
        std::fs::write(dir.join("volume"), "52").unwrap();
        std::fs::write(dir.join("mute"), "no").unwrap();
        std::fs::write(dir.join("mic"), "no").unwrap();
        let script = dir.join("pactl");
        let body = format!(
            "#!/bin/sh\necho \"$@\" >> '{d}/pactl.log'\ncase \"$1\" in\n\
             get-sink-volume) echo \"Volume: front-left: 0 /  $(cat '{d}/volume')% / 0 dB\" ;;\n\
             get-source-volume) echo 'Volume: front-left: 0 /  80% / 0 dB' ;;\n\
             get-sink-mute) echo \"Mute: $(cat '{d}/mute')\" ;;\n\
             get-source-mute) echo \"Mute: $(cat '{d}/mic')\" ;;\n\
             set-sink-volume) echo \"${{3%\\%}}\" > '{d}/volume' ;;\n\
             set-sink-mute) if [ \"$3\" = 1 ]; then echo yes; else echo no; fi > '{d}/mute' ;;\n\
             set-source-mute) if [ \"$3\" = 1 ]; then echo yes; else echo no; fi > '{d}/mic' ;;\n\
             esac\n"
        );
        std::fs::write(&script, body).unwrap();
        let chmod = Command::new("chmod").arg("+x").arg(&script).status();
        assert!(chmod.unwrap().success());
        Pactl {
            program: script.into_os_string(),
        }
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        dir: PathBuf,
        brightness: Calls,
        player: Calls,
        _bus: Bus,
    }

    impl Harness {
        fn start(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("media-keys-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            // A raw backlight at 20 %.
            let sysfs = dir.join("backlight");
            let light = sysfs.join("amdgpu_bl1");
            std::fs::create_dir_all(&light).unwrap();
            std::fs::write(light.join("type"), "raw").unwrap();
            std::fs::write(light.join("brightness"), "51").unwrap();
            std::fs::write(light.join("max_brightness"), "255").unwrap();
            let mut bus = Bus::start(&dir);
            let (brightness, player) = (Calls::default(), Calls::default());
            let session = Session {
                sysfs: sysfs.clone(),
                calls: brightness.clone(),
            };
            let path = system_services::backlight::SESSION;
            bus.serve(system_services::backlight::SERVICE, path, session);
            let calls = player.clone();
            let name = "org.mpris.MediaPlayer2.test";
            bus.serve(name, "/org/mpris/MediaPlayer2", Player { calls });

            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((800, 600).into(), 1.0);
            state.enable_media_keys(Controls {
                mixer: Mixer::Pactl(fake_pactl(&dir)),
                backlights: sysfs,
                system_bus: Some(bus.address.clone()),
                session_bus: Some(bus.address.clone()),
            });
            Self {
                event_loop,
                state,
                dir,
                brightness,
                player,
                _bus: bus,
            }
        }

        fn key(&mut self, code: u32, state: KeyState) {
            self.state.keyboard_key(Keycode::new(code), state, 1);
        }

        fn tap(&mut self, code: u32) {
            self.key(code, KeyState::Pressed);
            self.key(code, KeyState::Released);
        }

        fn run_for(&mut self, time: Duration) {
            let end = Instant::now() + time;
            while Instant::now() < end {
                self.event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                    .unwrap();
            }
        }

        fn run_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
            let end = Instant::now() + Duration::from_secs(10);
            while !done(self) {
                assert!(Instant::now() < end, "timed out waiting for: {what}");
                self.run_for(Duration::from_millis(10));
            }
        }

        fn pactl_log(&self) -> String {
            std::fs::read_to_string(self.dir.join("pactl.log")).unwrap_or_default()
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn level(kind: Kind, percent: u16, muted: bool) -> Option<Level> {
        Some(Level {
            kind,
            percent,
            muted,
        })
    }

    #[test]
    fn the_sound_keys_step_mute_and_show_the_level() {
        let mut h = Harness::start("sound");
        h.tap(VOLUME_UP);
        let shown = level(Kind::Volume, 55, false);
        h.run_until("the volume up", |h| h.state.osd_level() == shown);
        assert!(h.pactl_log().contains("set-sink-volume @DEFAULT_SINK@ 55%"));
        h.tap(MUTE);
        let shown = level(Kind::Volume, 55, true);
        h.run_until("the sound muted", |h| h.state.osd_level() == shown);
        // A step unmutes.
        h.tap(VOLUME_UP);
        let shown = level(Kind::Volume, 60, false);
        h.run_until("the volume up again", |h| h.state.osd_level() == shown);
        h.tap(MIC_MUTE);
        let shown = level(Kind::Microphone, 80, true);
        h.run_until("the microphone muted", |h| h.state.osd_level() == shown);
        assert!(h.pactl_log().contains("set-source-mute @DEFAULT_SOURCE@ 1"));
        // The pill goes once the keys are left alone.
        h.run_until("the pill gone", |h| h.state.osd_level().is_none());
    }

    #[test]
    fn a_held_brightness_key_repeats_until_released() {
        let mut h = Harness::start("brightness");
        h.key(BRIGHTNESS_UP, KeyState::Pressed);
        // The first step, then one each interval after the delay: about six in all.
        h.run_for(REPEAT_DELAY + REPEAT_INTERVAL * 4 + REPEAT_INTERVAL / 2);
        h.key(BRIGHTNESS_UP, KeyState::Released);
        let count = |h: &Harness| h.brightness.lock().unwrap().len();
        h.run_until("the steps done", |h| count(h) >= 4);
        // The worker ends what it was given; then nothing more comes.
        h.run_for(REPEAT_INTERVAL * 3);
        let steps = count(&h);
        h.run_for(REPEAT_INTERVAL * 4);
        let calls = h.brightness.lock().unwrap().clone();
        assert_eq!(calls.len(), steps, "{calls:?}");
        assert!((4..=8).contains(&steps), "{calls:?}");
        // From 20 %, by steps of 5, through logind: never written by the compositor itself. A
        // repeat skipped while logind answers is not lost: the steps waiting go together.
        assert_eq!(calls[0], "backlight amdgpu_bl1 64");
        let raw = |percent: u16| {
            format!(
                "backlight amdgpu_bl1 {}",
                (255 * u32::from(percent) + 50) / 100
            )
        };
        let last = calls.last().unwrap();
        let percent = (40..=100).step_by(5).find(|p| raw(*p) == *last);
        let percent = percent.unwrap_or_else(|| panic!("{calls:?}"));
        let shown = level(Kind::Brightness, percent, false);
        h.run_until("the level shown", |h| h.state.osd_level() == shown);
    }

    #[test]
    fn a_key_held_against_a_hung_sound_server_piles_nothing_up() {
        let mut h = Harness::start("hung");
        // From now on `pactl` hangs, until it is killed after `audio::TIMEOUT`.
        let script = h.dir.join("pactl");
        let log = h.dir.join("pactl.log");
        let body = format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\nexec sleep 30\n",
            log.display()
        );
        std::fs::write(&script, body).unwrap();
        h.key(VOLUME_UP, KeyState::Pressed);
        h.run_for(Duration::from_secs(4));
        h.key(VOLUME_UP, KeyState::Released);
        let at_release = h.pactl_log().lines().count();
        // What runs at the release ends, plus one action at most, merged from what waited.
        h.run_for(system_services::audio::TIMEOUT * 2 + Duration::from_millis(500));
        let calls = h.pactl_log().lines().count();
        assert!(calls <= at_release + 1, "{at_release} then {calls}");
        assert!(calls <= 3, "{}", h.pactl_log());
        assert_eq!(h.state.osd_level(), None);
    }

    #[test]
    fn the_play_key_goes_to_the_player() {
        let mut h = Harness::start("player");
        h.tap(PLAY_PAUSE);
        h.run_until("the player told", |h| !h.player.lock().unwrap().is_empty());
        assert_eq!(*h.player.lock().unwrap(), ["PlayPause"]);
        // Nothing to show for a player.
        assert_eq!(h.state.osd_level(), None);
    }
}
