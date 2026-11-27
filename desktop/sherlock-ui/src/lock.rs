//! The session lock (lot D-25a, ADR 0023). The lock screen itself is a program of its own,
//! `sherlockos-lock`; what lives here is what the compositor draws when no lock program shows
//! anything: the plain background of the system screens, and, once every lock program failed,
//! the rescue screen, two pills of glass in the middle of each screen. No board draws it: the
//! measures below are choices of the lot, taken from the Mission Control labels. See "Session
//! lock" in `docs/compositor.md`.

use std::time::Duration;

use crate::color::{Color, Palette};
use crate::mission_control;
use crate::motion::Ease;

/// What the rescue screen says.
pub const RESCUE_TITLE: &str = "Verrouillé";
pub const RESCUE_HINT: &str = "Appuie sur Entrée pour réessayer";

/// Height of a pill of the rescue screen, and the gap between the two.
pub const RESCUE_LABEL_HEIGHT: f32 = mission_control::LABEL_HEIGHT;
pub const RESCUE_GAP: f32 = 12.0;

/// The curtain (lot D-24d): the picture of the lock, which the compositor lowers from the top
/// over the last frame of the desktop when the session locks, and raises off the live desktop
/// once it is unlocked. The lock screen starts its entrance once it is down.
pub const CURTAIN: Duration = Duration::from_millis(400);
pub const CURTAIN_EASE: Ease = Ease::OutCubic;

/// Between two compositors (the login screen's, then the session's) the screen goes black: the
/// curtain fades to black in `CURTAIN_BLACK` before the login screen's compositor stops, and
/// comes back from black as long when the session's starts. Then it fades out over the desktop
/// in `CURTAIN_REVEAL`, and the bar and the dock arrive (lot D-24e).
pub const CURTAIN_BLACK: Duration = Duration::from_millis(250);
pub const CURTAIN_REVEAL: Duration = Duration::from_millis(600);
pub const CURTAIN_FADE_EASE: Ease = Ease::OutCubic;

/// How far the curtain is down `elapsed` after it started to move, from 0 (up, out of the
/// screen) to 1 (down, covering it): lowering when `down`, raising otherwise.
pub fn curtain(elapsed: Duration, down: bool) -> f32 {
    let t = elapsed.as_secs_f32() / CURTAIN.as_secs_f32();
    let way = CURTAIN_EASE.apply(t);
    if down { way } else { 1.0 - way }
}

/// The colors of the lock in a palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    /// Behind the lock surfaces, and alone on a screen without one.
    pub background: Color,
    pub label: Color,
    pub label_text: Color,
}

impl Colors {
    pub const fn of(palette: &Palette) -> Self {
        Self {
            background: palette.system_screen,
            label: palette.menu_glass,
            label_text: palette.text_primary,
        }
    }
}

/// `#RRGGBB` of a color without its `#`, as `swaylock --color` takes it.
pub fn hex(color: Color) -> String {
    format!("{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

/// The lock screen, `sherlockos-lock` (lot D-25a): macOS-like, as the author asked (no board
/// draws it, the board of the login screen inspires it). The time and the date at the top, then at the
/// bottom middle the bubble of Sherlock, the name of the user and the password pill, and the
/// machine buttons under them. Every size is in logical pixels and follows the screen: the
/// `fit` helpers keep them between a minimum and a maximum, from a 1366 × 768 laptop to a 4K
/// screen at scale 2. See `docs/lock.md`.
pub mod screen {
    use std::time::Duration;

    use crate::color::{Color, Palette};
    use crate::motion::{self, Ease, Spring};

    /// A length proportional to the screen's height, kept between `min` and `max`.
    pub fn fit(height: f32, ratio: f32, min: f32, max: f32) -> f32 {
        (height * ratio).clamp(min, max)
    }

    /// The time: size and room above it, from the screen's height.
    pub fn clock_size(height: f32) -> f32 {
        fit(height, 0.11, 64.0, 132.0)
    }
    pub fn top(height: f32) -> f32 {
        fit(height, 0.08, 28.0, 112.0)
    }
    /// The date, above the time. Both take the regular weight of Inter: iced 0.14 draws the
    /// variable font at its default instance only, and a thin or medium weight falls back to
    /// another font of the system.
    pub const DATE_SIZE: f32 = 20.0;

    /// The bubble of Sherlock: its diameter, the gap under it, the name under that.
    pub fn bubble(height: f32) -> f32 {
        fit(height, 0.1, 64.0, 112.0)
    }
    pub const BUBBLE_GAP: f32 = 14.0;
    pub const NAME_SIZE: f32 = 17.0;
    pub const NAME_GAP: f32 = 14.0;

    /// The password pill: width, height (fully round ends), the room of each icon button
    /// inside it, and the size of its icons.
    pub const FIELD_WIDTH: f32 = 260.0;
    pub const FIELD_HEIGHT: f32 = 36.0;
    pub const FIELD_RADIUS: f32 = FIELD_HEIGHT / 2.0;
    pub const FIELD_BUTTON: f32 = 28.0;
    pub const FIELD_ICON: f32 = 16.0;
    pub const FIELD_TEXT_SIZE: f32 = 14.0;
    /// The line under the pill: the error, or the caps lock.
    pub const HINT_SIZE: f32 = 13.0;
    pub const HINT_GAP: f32 = 10.0;

    /// The machine buttons: a round glass button and its label, side by side.
    pub const MACHINE_BUTTON: f32 = 40.0;
    pub const MACHINE_ICON: f32 = 18.0;
    pub const MACHINE_LABEL_SIZE: f32 = 12.0;
    pub const MACHINE_GAP: f32 = 28.0;
    /// Room under the machine buttons, and between the pill and them.
    pub fn bottom(height: f32) -> f32 {
        fit(height, 0.05, 20.0, 72.0)
    }
    pub fn machine_gap(height: f32) -> f32 {
        fit(height, 0.07, 24.0, 96.0)
    }

    /// Entrance: each element rises by `RISE` and fades in, `STAGGER` after the one before,
    /// on the spring of a window that opens. The first key ends every animation at once.
    pub const RISE: f32 = 16.0;
    pub const STAGGER: Duration = Duration::from_millis(70);
    pub const SPRING: Spring = motion::OPEN_SPRING;
    /// Exit, once unlocked or logged in: each element fades out in `EXIT`, `STAGGER` after
    /// the one below it (the machine buttons first, the time last); the background stays.
    pub const EXIT: Duration = Duration::from_millis(220);
    pub const EXIT_EASE: Ease = Ease::InQuad;
    /// A wrong password: the pill shakes three times, then empties.
    pub const SHAKE: Duration = Duration::from_millis(450);
    pub const SHAKES: f32 = 3.0;
    pub const SHAKE_AMPLITUDE: f32 = 12.0;
    /// One turn of the spinner.
    pub const SPIN: Duration = motion::SPIN;

    /// The form (bubble, name, pill, machine buttons) shows only on demand, as on macOS: a key,
    /// a click or the pointer bring it, and it leaves again after `FORM_TIMEOUT` without input.
    pub const FORM_TIMEOUT: Duration = Duration::from_secs(20);
    /// The bubble grows from this share of its size, on `SPRING`.
    pub const BUBBLE_FROM: f32 = 0.6;
    /// How long Sherlock is sorry after a refusal, before he rests.
    pub const SORRY: Duration = Duration::from_millis(2500);
    /// While a key comes in the field, and this long after the last one, Sherlock looks down
    /// at it, head and eyes, whatever his mood (lot D-24d).
    pub const TYPING_GAZE: Duration = Duration::from_millis(1500);
    /// The picture behind the screen fades in once read, over the same picture the compositor
    /// shows (lot D-24d): nothing jumps when it is ready.
    pub const PICTURE_FADE: Duration = motion::THEME_FADE;

    /// How much of element `index` is left `elapsed` after the screen started to leave, from 1
    /// to 0: the last element (`last`, the machine buttons) leaves first, the others follow
    /// `STAGGER` apart, the time (0) last.
    pub fn exit(index: u32, last: u32, elapsed: Duration) -> f32 {
        let behind = STAGGER * last.saturating_sub(index);
        let t = elapsed.saturating_sub(behind).as_secs_f32() / EXIT.as_secs_f32();
        1.0 - EXIT_EASE.apply(t)
    }

    /// How long the exit of `last + 1` elements takes.
    pub fn exit_length(last: u32) -> Duration {
        STAGGER * last + EXIT
    }

    /// The picture behind the lock screen, blurred once at start: reduced to this width, blurred
    /// with this deviation (in pixels of the reduced picture), then darkened by `DIM`. Drawn
    /// back at the screen's size, the reduction blurs it further for free.
    pub const BACKGROUND_WIDTH: u32 = 320;
    pub const BLUR_SIGMA: f32 = 4.0;
    pub const DIM: f32 = 0.55;

    /// The colors of the screen in a palette.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Colors {
        pub background: Color,
        pub text: Color,
        pub secondary: Color,
        pub field: Color,
        pub field_border: Color,
        pub placeholder: Color,
        pub bubble: Color,
        pub bubble_border: Color,
        pub button: Color,
        pub button_hover: Color,
        pub armed: Color,
        pub error: Color,
    }

    impl Colors {
        pub const fn of(palette: &Palette) -> Self {
            Self {
                background: palette.system_screen,
                text: palette.text_primary,
                secondary: palette.text_secondary,
                field: palette.glass,
                field_border: palette.border,
                placeholder: palette.text_tertiary,
                bubble: palette.mascot_bubble,
                bubble_border: palette.mascot_bubble_border,
                button: palette.glass,
                button_hover: palette.hover,
                armed: palette.amber_soft,
                error: palette.salmon_text,
            }
        }
    }

    /// The texts of the screen.
    pub const PASSWORD: &str = "Mot de passe";
    pub const SHOW: &str = "Afficher le mot de passe";
    pub const HIDE: &str = "Masquer le mot de passe";
    pub const UNLOCK: &str = "Déverrouiller";
    pub const WRONG: &str = "Mot de passe incorrect";
    pub const TOO_MANY: &str = "Trop d'essais. Patiente un peu avant de réessayer.";
    pub const UNAVAILABLE: &str = "Vérification impossible. Réessaie.";
    pub const CAPS_LOCK: &str = "Verr. Maj activé";
    pub const SLEEP: &str = "Veille";
    pub const REBOOT: &str = "Redémarrer";
    pub const POWER_OFF: &str = "Éteindre";
    pub const CONFIRM: &str = "Reclique pour confirmer";
    /// The line of the rescue mode (lot D-25c).
    pub const RESCUE: &str = "Écran de verrouillage de secours";

    /// Days of the week, Monday to Sunday, and months, for the date of the screen.
    const WEEKDAYS: [&str; 7] = [
        "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
    ];
    const MONTHS: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];

    /// « jeudi 26 novembre »: `weekday` from 1 (Monday) to 7, `month` from 1 to 12.
    pub fn date(weekday: u8, day: u8, month: u8) -> Option<String> {
        let weekday = WEEKDAYS.get(usize::from(weekday).checked_sub(1)?)?;
        let month = MONTHS.get(usize::from(month).checked_sub(1)?)?;
        Some(format!("{weekday} {day} {month}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_background_is_the_one_of_the_system_screens() {
        let colors = Colors::of(&Palette::DARK);
        assert_eq!(colors.background, Palette::DARK.system_screen);
        assert_eq!(hex(Palette::DARK.system_screen), "0a1424");
    }

    #[test]
    fn the_curtain_comes_down_then_goes_up() {
        assert_eq!(curtain(Duration::ZERO, true), 0.0);
        let half = curtain(CURTAIN / 2, true);
        assert!(half > 0.5 && half < 1.0, "{half}");
        assert_eq!(curtain(CURTAIN, true), 1.0);
        assert_eq!(curtain(CURTAIN * 2, true), 1.0);
        assert_eq!(curtain(Duration::ZERO, false), 1.0);
        assert_eq!(curtain(CURTAIN, false), 0.0);
    }

    #[test]
    fn the_elements_leave_in_reverse_order() {
        let at = |ms| Duration::from_millis(ms);
        // The buttons (4) go first, the time (0) last.
        assert!(screen::exit(4, 4, at(100)) < 1.0);
        assert_eq!(screen::exit(0, 4, at(100)), 1.0);
        assert!(screen::exit(4, 4, at(100)) < screen::exit(2, 4, at(100)));
        assert_eq!(screen::exit(4, 4, screen::EXIT), 0.0);
        assert_eq!(screen::exit(0, 4, screen::exit_length(4)), 0.0);
    }

    #[test]
    fn the_date_is_written_in_full() {
        assert_eq!(
            screen::date(4, 26, 11).as_deref(),
            Some("jeudi 26 novembre")
        );
        assert_eq!(screen::date(0, 1, 1), None);
        assert_eq!(screen::date(7, 1, 13), None);
    }

    #[test]
    fn the_sizes_follow_the_screen_within_bounds() {
        // 1366 × 768 at 1, 1920 × 1080 at 1, 2560 × 1600 at 1.5, 3840 × 2160 at 2: logical
        // heights of 768, 1080, 1067 and 1080.
        for height in [768.0, 1080.0, 1067.0, 2160.0] {
            let clock = screen::clock_size(height);
            assert!((64.0..=132.0).contains(&clock));
            let used = screen::top(height)
                + screen::DATE_SIZE
                + clock
                + screen::bubble(height)
                + screen::BUBBLE_GAP
                + screen::NAME_SIZE
                + screen::NAME_GAP
                + screen::FIELD_HEIGHT
                + screen::machine_gap(height)
                + screen::MACHINE_BUTTON
                + screen::bottom(height);
            assert!(used < height * 0.8, "{height}: {used}");
        }
    }

    /// Relative luminance of a color laid over `under` (WCAG).
    fn luminance(color: Color, under: Color) -> f64 {
        let channel = |c: u8, u: u8| {
            let a = f64::from(color.a);
            let v = (f64::from(c) * a + f64::from(u) * (1.0 - a)) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r, under.r)
            + 0.7152 * channel(color.g, under.g)
            + 0.0722 * channel(color.b, under.b)
    }

    fn contrast(a: f64, b: f64) -> f64 {
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn sherlock_stands_out_of_his_bubble_in_both_themes() {
        for (palette, least) in [(Palette::DARK, 3.0), (Palette::LIGHT, 1.5)] {
            let colors = screen::Colors::of(&palette);
            let bubble = luminance(colors.bubble, palette.system_screen);
            let sherlock = luminance(palette.cream, palette.cream);
            let ratio = contrast(bubble, sherlock);
            assert!(ratio >= least, "{ratio}");
        }
        // Not the cream of the boards any more: on it, his beige did not show.
        assert_ne!(
            screen::Colors::of(&Palette::LIGHT).bubble,
            Palette::LIGHT.cream
        );
    }
}
