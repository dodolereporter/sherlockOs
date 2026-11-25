//! Theme colors, named by role. Values from `docs/contexte.md`, `docs/design/THEME-CLAIR.md`
//! and the design boards. A program sets the active theme (`set_active`), and every color is
//! read from its palette (`palette`): no component looks at the theme itself.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use crate::motion::{self, Fade};

/// An sRGB color, with its opacity between 0 and 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Color {
    /// Opaque color, like `#RRGGBB`.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Translucent color, like `rgba(r, g, b, a)` in CSS.
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Channels between 0 and 1, multiplied by the opacity: the form renderers expect to blend
    /// colors.
    pub fn premultiplied(self) -> [f32; 4] {
        let channel = |c: u8| f32::from(c) / 255.0 * self.a;
        [channel(self.r), channel(self.g), channel(self.b), self.a]
    }

    /// Opaque mix: `t` is 0 for `self`, 1 for `other`. Used for the hover shades.
    pub const fn mix(self, other: Self, t: f32) -> Self {
        const fn channel(a: u8, b: u8, t: f32) -> u8 {
            (a as f32 + (b as f32 - a as f32) * t + 0.5) as u8
        }
        Self::rgb(
            channel(self.r, other.r, t),
            channel(self.g, other.g, t),
            channel(self.b, other.b, t),
        )
    }

    /// Mix of every channel and of the opacity: `t` is 0 for `self`, 1 for `other`. Used for
    /// the fade between the two palettes.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let channel =
            |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Self {
            r: channel(self.r, other.r),
            g: channel(self.g, other.g),
            b: channel(self.b, other.b),
            a: self.a + (other.a - self.a) * t,
        }
    }

    /// `self` laid over `below`, blended in sRGB as the browser and tiny-skia do.
    pub fn over(self, below: Self) -> Self {
        let a = self.a + below.a * (1.0 - self.a);
        if a <= 0.0 {
            return Self::rgba(0, 0, 0, 0.0);
        }
        let channel = |top: u8, bottom: u8| {
            let mixed =
                (f32::from(top) * self.a + f32::from(bottom) * below.a * (1.0 - self.a)) / a;
            mixed.round().clamp(0.0, 255.0) as u8
        };
        Self::rgba(
            channel(self.r, below.r),
            channel(self.g, below.g),
            channel(self.b, below.b),
            a,
        )
    }

    /// Relative luminance as defined by WCAG 2, opacity ignored.
    pub fn luminance(self) -> f32 {
        let linear = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }

    /// WCAG contrast ratio between two opaque colors, from 1 to 21.
    pub fn contrast(self, other: Self) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

#[cfg(feature = "iced")]
impl From<Color> for iced::Color {
    fn from(c: Color) -> Self {
        Self::from_rgba8(c.r, c.g, c.b, c.a)
    }
}

#[cfg(feature = "iced")]
impl From<iced::Color> for Color {
    fn from(c: iced::Color) -> Self {
        let [r, g, b, _] = c.into_rgba8();
        Self::rgba(r, g, b, c.a)
    }
}

// Backgrounds.

/// Desktop wallpaper.
/// Dark value only: use `palette()`.
pub const DESKTOP: Color = Color::rgb(0x0E, 0x1A, 0x2E);
/// The three large discs of the wallpaper, from darkest to lightest.
/// Dark value only: use `palette()`.
pub const DESKTOP_DISCS: [Color; 3] = [
    Color::rgb(0x13, 0x2A, 0x49),
    Color::rgb(0x16, 0x31, 0x4F),
    Color::rgb(0x1B, 0x3F, 0x63),
];
/// Background of the system screens: login, boot, installer.
/// Dark value only: use `palette()`.
pub const SYSTEM_SCREEN: Color = Color::rgb(0x0A, 0x14, 0x24);
/// Darkest background: terminal, code areas. Also used as text on the green.
/// Dark value only: use `palette()`.
pub const DEEPEST: Color = Color::rgb(0x0B, 0x12, 0x20);
/// Body of opaque windows.
/// Dark value only: use `palette()`.
pub const WINDOW: Color = Color::rgb(0x0F, 0x18, 0x28);
/// Title bar of windows.
/// Dark value only: use `palette()`.
pub const TITLE_BAR: Color = Color::rgb(0x13, 0x1E, 0x32);
/// Side panel: navigation on the left, preview on the right.
/// Dark value only: use `palette()`.
pub const SIDEBAR: Color = Color::rgb(0x11, 0x1B, 0x2E);
/// Card inside a window.
/// Dark value only: use `palette()`.
pub const CARD: Color = Color::rgb(0x15, 0x22, 0x38);
/// Command block of the terminal.
/// Dark value only: use `palette()`.
pub const TERMINAL_BLOCK: Color = Color::rgb(0x10, 0x1B, 0x2E);

// Glass. The blur is the compositor's business: here, only the tint.

/// Floating panels (24 px blur).
/// Dark value only: use `palette()`.
pub const GLASS: Color = Color::rgba(16, 26, 44, 0.82);
/// Menus and submenus (28 px blur).
/// Dark value only: use `palette()`.
pub const MENU_GLASS: Color = Color::rgba(22, 34, 56, 0.96);
/// Dock (30 px blur).
/// Dark value only: use `palette()`.
pub const DOCK_GLASS: Color = Color::rgba(244, 235, 227, 0.17);
/// Top bar (24 px blur). The Main design board makes it slightly lighter than `GLASS`.
/// Dark value only: use `palette()`.
pub const TOP_BAR_GLASS: Color = Color::rgba(16, 26, 44, 0.78);

// Lines and veils.

/// Border of a glass panel and of a window, menu separator.
/// Dark value only: use `palette()`.
pub const BORDER: Color = Color::rgba(255, 255, 255, 0.10);
/// Border of a menu (Menus design board).
/// Dark value only: use `palette()`.
pub const MENU_BORDER: Color = Color::rgba(255, 255, 255, 0.14);
/// Border of a card (« Réglages » design board).
/// Dark value only: use `palette()`.
pub const CARD_BORDER: Color = Color::rgba(255, 255, 255, 0.09);
/// Border of the top bar (Main design board).
/// Dark value only: use `palette()`.
pub const TOP_BAR_BORDER: Color = Color::rgba(255, 255, 255, 0.09);
/// Chip of an inactive workspace in the top bar, at rest then hovered.
/// Dark value only: use `palette()`.
pub const CHIP: Color = Color::rgba(255, 255, 255, 0.07);
/// Dark value only: use `palette()`.
pub const CHIP_HOVER: Color = Color::rgba(255, 255, 255, 0.12);
/// Veil of a top bar button whose menu is open (the open « Fenêtre » button of the Menus
/// board).
/// Dark value only: use `palette()`.
pub const TOP_BAR_BUTTON_OPEN: Color = Color::rgba(255, 255, 255, 0.14);
/// Border of the dock.
/// Dark value only: use `palette()`.
pub const DOCK_BORDER: Color = Color::rgba(255, 255, 255, 0.26);
/// Border of a secondary button.
/// Dark value only: use `palette()`.
pub const SECONDARY_BORDER: Color = Color::rgba(255, 255, 255, 0.18);
/// Line between two rows of a card, bottom of the title bar.
/// Dark value only: use `palette()`.
pub const DIVIDER: Color = Color::rgba(255, 255, 255, 0.07);
/// Background of a text field.
/// Dark value only: use `palette()`.
pub const FIELD: Color = Color::rgba(255, 255, 255, 0.07);
/// Hover veil on an element without background.
/// Dark value only: use `palette()`.
pub const HOVER: Color = Color::rgba(255, 255, 255, 0.07);
/// Press veil on an element without background.
/// Dark value only: use `palette()`.
pub const PRESSED: Color = Color::rgba(255, 255, 255, 0.12);
/// Track of a toggle that is off, at rest and hovered. The design boards only show toggles
/// that are on: the off track reuses the border of a secondary button.
/// Dark value only: use `palette()`.
pub const TOGGLE_OFF: Color = Color::rgba(255, 255, 255, 0.18);
/// Dark value only: use `palette()`.
pub const TOGGLE_OFF_HOVER: Color = Color::rgba(255, 255, 255, 0.26);
/// Inset area of a toolbar: the breadcrumb (Evidence mock-up).
/// Dark value only: use `palette()`.
pub const INSET: Color = Color::rgba(255, 255, 255, 0.05);
/// Pill behind the window buttons.
/// Dark value only: use `palette()`.
pub const CONTROLS: Color = Color::rgba(255, 255, 255, 0.06);
/// Veil laid over a window behind a question awaiting an answer.
/// Dark value only: use `palette()`.
pub const SCRIM: Color = Color::rgba(3, 8, 18, 0.45);
/// Background of a status badge: warning, success, neutral (git status in the Evidence
/// mock-up), and border of a green badge without background (git branch).
/// Dark value only: use `palette()`.
pub const AMBER_SOFT: Color = Color::rgba(240, 196, 138, 0.18);
/// Dark value only: use `palette()`.
pub const ACCENT_SOFT: Color = Color::rgba(133, 197, 149, 0.18);
/// Dark value only: use `palette()`.
pub const NEUTRAL_SOFT: Color = Color::rgba(255, 255, 255, 0.08);
/// Dark value only: use `palette()`.
pub const ACCENT_BORDER: Color = Color::rgba(133, 197, 149, 0.5);
/// Background of a disabled element.
/// Dark value only: use `palette()`.
pub const DISABLED: Color = Color::rgba(255, 255, 255, 0.05);
/// Colour of the crisp shadows of floating surfaces (`surface::Shadowed`), the same blue
/// black as `SHADOW` and `MENU_SHADOW`; each step of the shadow sets its own opacity.
/// Dark value only: use `palette()`.
pub const SHADOW_BASE: Color = Color::rgb(3, 8, 18);
/// Shadow of windows and panels.
/// Dark value only: use `palette()`.
pub const SHADOW: Color = Color::rgba(3, 8, 18, 0.55);
/// Shadow of menus.
/// Dark value only: use `palette()`.
pub const MENU_SHADOW: Color = Color::rgba(3, 8, 18, 0.6);

// Texts.

/// Titles and content.
/// Dark value only: use `palette()`.
pub const TEXT_PRIMARY: Color = Color::rgb(0xEE, 0xF2, 0xF7);
/// Descriptions, values.
/// Dark value only: use `palette()`.
pub const TEXT_SECONDARY: Color = Color::rgb(0xC5, 0xCE, 0xDB);
/// Labels, shortcuts, text of a disabled element.
/// Dark value only: use `palette()`.
pub const TEXT_TERTIARY: Color = Color::rgb(0xA9, 0xB4, 0xC6);
/// Text of a code snippet or of a file preview (Evidence mock-up).
/// Dark value only: use `palette()`.
pub const TEXT_CODE: Color = Color::rgb(0xDC, 0xE4, 0xEE);
/// Line numbers.
/// Dark value only: use `palette()`.
pub const TEXT_LINE_NUMBER: Color = Color::rgb(0x84, 0x92, 0xA8);
/// Text laid on the green.
/// Dark value only: use `palette()`.
pub const TEXT_ON_ACCENT: Color = DEEPEST;
/// Text laid on the cream.
/// Dark value only: use `palette()`.
pub const TEXT_ON_CREAM: Color = Color::rgb(0x14, 0x23, 0x3B);

// Accents.

/// Green of the scarf: main action, active state, success.
/// Dark value only: use `palette()`.
pub const ACCENT: Color = Color::rgb(0x85, 0xC5, 0x95);
/// Hovered green.
/// Dark value only: use `palette()`.
pub const ACCENT_HOVER: Color = ACCENT.mix(TEXT_PRIMARY, 0.18);
/// Pressed green.
/// Dark value only: use `palette()`.
pub const ACCENT_PRESSED: Color = ACCENT.mix(DEEPEST, 0.14);
/// Blue of icons and folders, the darkest.
/// Dark value only: use `palette()`.
pub const BLUE_DARK: Color = Color::rgb(0x2B, 0x5F, 0x86);
/// Blue of icons and folders.
/// Dark value only: use `palette()`.
pub const BLUE: Color = Color::rgb(0x32, 0x6D, 0x93);
/// Blue of the selection and of code keywords.
/// Dark value only: use `palette()`.
pub const BLUE_BRIGHT: Color = Color::rgb(0x5B, 0xA3, 0xD0);
/// Light blue of code keywords.
/// Dark value only: use `palette()`.
pub const BLUE_LIGHT: Color = Color::rgb(0x8F, 0xC1, 0xE8);
/// What comes from Sherlock or from the trust system.
/// Dark value only: use `palette()`.
pub const CREAM: Color = Color::rgb(0xF4, 0xEB, 0xE3);
/// Awaiting a consent, warning.
/// Dark value only: use `palette()`.
pub const AMBER: Color = Color::rgb(0xF0, 0xC4, 0x8A);
/// Error, destructive action.
/// Dark value only: use `palette()`.
pub const SALMON: Color = Color::rgb(0xF6, 0xB5, 0xA6);

/// Fully transparent: the background of a shell surface that draws its own shapes.
pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0.0);

/// Selected row of a list or of a menu, hovered menu entry.
/// Dark value only: use `palette()`.
pub const SELECTED_ROW: Color = Color::rgba(91, 163, 208, 0.26);
/// Background of the active entry of a navigation.
/// Dark value only: use `palette()`.
pub const ACTIVE_ENTRY: Color = Color::rgba(133, 197, 149, 0.16);
/// Keyboard focus ring. The design boards show none: light blue, visible on every background
/// and distinct from the green of the active state.
/// Dark value only: use `palette()`.
pub const FOCUS_RING: Color = BLUE_LIGHT;

/// The two themes. They share sizes and spacings: only the palette changes
/// (`docs/design/THEME-CLAIR.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub const ALL: [Self; 2] = [Self::Dark, Self::Light];

    pub const fn palette(self) -> &'static Palette {
        match self {
            Self::Dark => &Palette::DARK,
            Self::Light => &Palette::LIGHT,
        }
    }

    /// Its step in a fade between the palettes.
    const fn step(self) -> u8 {
        match self {
            Self::Dark => 0,
            Self::Light => MIX_STEPS,
        }
    }
}

/// Steps of a fade between the two palettes: enough for a smooth fade at 60 frames a second,
/// few enough to build each palette once.
pub const MIX_STEPS: u8 = 32;

/// Where the program stands between the dark palette (0) and the light one (`MIX_STEPS`).
static MIX: AtomicU8 = AtomicU8::new(0);

/// Makes `theme` the active theme of the program. The program then redraws: the iced theme
/// (`theme::theme`) and every component follow it.
pub fn set_active(theme: Theme) {
    MIX.store(theme.step(), Ordering::Relaxed);
}

/// Shows a mix of the two palettes, from 0 (dark) to 1 (light), during a fade from one theme
/// to the other (`ThemeFade`). It is rounded to a step.
pub fn set_mix(light: f32) {
    let step = (light.clamp(0.0, 1.0) * f32::from(MIX_STEPS)).round() as u8;
    MIX.store(step, Ordering::Relaxed);
}

/// The step of the palette shown, from 0 (dark) to `MIX_STEPS` (light).
pub fn mix_step() -> u8 {
    MIX.load(Ordering::Relaxed)
}

/// The active theme, dark until the program sets one. During a fade, the closer one.
pub fn active() -> Theme {
    if mix_step() * 2 > MIX_STEPS {
        Theme::Light
    } else {
        Theme::Dark
    }
}

/// The palette shown: that of the active theme, or a mix of both during a fade.
pub fn palette() -> &'static Palette {
    Palette::at_step(mix_step())
}

/// The theme a program shows, followed with a fade (`motion::THEME_FADE`). The program calls
/// `set` with each theme of the appearance file, and `tick` on each frame while `running`.
#[derive(Debug, Clone, Default)]
pub struct ThemeFade {
    target: Option<Theme>,
    /// The fade under way, timed from its instant.
    fade: Option<(Instant, Fade)>,
}

impl ThemeFade {
    /// Heads for `theme`: at once the first time, the theme the program starts in, and with a
    /// fade afterwards. A fade turned back halfway comes back in half the time.
    pub fn set(&mut self, theme: Theme, now: Instant) {
        match self.target {
            None => set_active(theme),
            Some(target) if target == theme => return,
            Some(_) => {
                let from = f32::from(mix_step()) / f32::from(MIX_STEPS);
                let to = f32::from(theme.step()) / f32::from(MIX_STEPS);
                let duration = motion::THEME_FADE.mul_f32((to - from).abs());
                let fade = Fade::new(from, to, std::time::Duration::ZERO, duration);
                self.fade = Some((now, fade));
            }
        }
        self.target = Some(theme);
    }

    /// Shows the fade at `now`. Returns whether it still runs.
    pub fn tick(&mut self, now: Instant) -> bool {
        let Some((start, fade)) = self.fade else {
            return false;
        };
        let at = now.saturating_duration_since(start);
        set_mix(fade.value(at));
        if fade.is_done(at) {
            self.fade = None;
        }
        self.fade.is_some()
    }

    /// A fade is under way: the program asks for frames.
    pub fn running(&self) -> bool {
        self.fade.is_some()
    }
}

/// Every color of a theme, named by its role. The module constants above are the dark values,
/// kept for the compositor, which does not follow the theme yet (lot D-05c): new code reads
/// `palette()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub desktop: Color,
    pub desktop_discs: [Color; 3],
    pub system_screen: Color,
    /// Discs of the system screens (login, boot, installer).
    pub system_discs: [Color; 3],
    /// Background of the game mode.
    pub game: Color,
    /// Code areas, the knob of a toggle that is on.
    pub deepest: Color,
    pub window: Color,
    pub title_bar: Color,
    pub sidebar: Color,
    pub card: Color,
    pub terminal_block: Color,
    pub glass: Color,
    pub menu_glass: Color,
    pub dock_glass: Color,
    pub top_bar_glass: Color,
    pub border: Color,
    pub menu_border: Color,
    pub card_border: Color,
    pub top_bar_border: Color,
    pub chip: Color,
    pub chip_hover: Color,
    pub top_bar_button_open: Color,
    pub dock_border: Color,
    pub secondary_border: Color,
    pub divider: Color,
    pub field: Color,
    pub hover: Color,
    pub pressed: Color,
    pub toggle_off: Color,
    pub toggle_off_hover: Color,
    pub inset: Color,
    pub controls: Color,
    pub scrim: Color,
    pub amber_soft: Color,
    pub accent_soft: Color,
    pub neutral_soft: Color,
    pub accent_border: Color,
    pub disabled: Color,
    /// Colour of the crisp shadow steps. Its opacity scales the opacity of every step.
    pub shadow_base: Color,
    pub shadow: Color,
    pub menu_shadow: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub text_code: Color,
    /// Types in code.
    pub text_code_type: Color,
    pub text_line_number: Color,
    pub text_on_accent: Color,
    pub text_on_cream: Color,
    /// The green as a fill: buttons, toggles, progress bars, chips.
    pub accent: Color,
    /// The green when it colours a text or an icon.
    pub accent_text: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub blue_dark: Color,
    pub blue: Color,
    pub blue_bright: Color,
    /// Code keywords and folders.
    pub blue_light: Color,
    pub cream: Color,
    /// The amber as a fill, then when it colours a text.
    pub amber: Color,
    pub amber_text: Color,
    /// The salmon as a fill, then when it colours a text.
    pub salmon: Color,
    pub salmon_text: Color,
    pub selected_row: Color,
    pub active_entry: Color,
    pub focus_ring: Color,
    /// Ring of the color logo in the top bar.
    pub logo_ring: Color,
    /// Dot of the color logo in the top bar, and dot under an open application in the dock:
    /// cream on the dark glass, night blue on the light one, where cream would not show.
    pub logo_dot: Color,
    /// Separator of the dock, before the open applications that are not pinned.
    pub dock_separator: Color,
}

/// Tint and light border of the light theme: the night blue of the text on cream.
const fn navy(a: f32) -> Color {
    Color::rgba(20, 35, 59, a)
}

/// Glass of the light theme.
const fn white(a: f32) -> Color {
    Color::rgba(255, 255, 255, a)
}

impl Palette {
    pub const DARK: Self = Self {
        desktop: DESKTOP,
        desktop_discs: DESKTOP_DISCS,
        system_screen: SYSTEM_SCREEN,
        system_discs: [
            Color::rgb(0x0F, 0x20, 0x38),
            Color::rgb(0x11, 0x26, 0x3E),
            Color::rgb(0x14, 0x30, 0x4C),
        ],
        game: Color::rgb(0x08, 0x0F, 0x1C),
        deepest: DEEPEST,
        window: WINDOW,
        title_bar: TITLE_BAR,
        sidebar: SIDEBAR,
        card: CARD,
        terminal_block: TERMINAL_BLOCK,
        glass: GLASS,
        menu_glass: MENU_GLASS,
        dock_glass: DOCK_GLASS,
        top_bar_glass: TOP_BAR_GLASS,
        border: BORDER,
        menu_border: MENU_BORDER,
        card_border: CARD_BORDER,
        top_bar_border: TOP_BAR_BORDER,
        chip: CHIP,
        chip_hover: CHIP_HOVER,
        top_bar_button_open: TOP_BAR_BUTTON_OPEN,
        dock_border: DOCK_BORDER,
        secondary_border: SECONDARY_BORDER,
        divider: DIVIDER,
        field: FIELD,
        hover: HOVER,
        pressed: PRESSED,
        toggle_off: TOGGLE_OFF,
        toggle_off_hover: TOGGLE_OFF_HOVER,
        inset: INSET,
        controls: CONTROLS,
        scrim: SCRIM,
        amber_soft: AMBER_SOFT,
        accent_soft: ACCENT_SOFT,
        neutral_soft: NEUTRAL_SOFT,
        accent_border: ACCENT_BORDER,
        disabled: DISABLED,
        shadow_base: SHADOW_BASE,
        shadow: SHADOW,
        menu_shadow: MENU_SHADOW,
        text_primary: TEXT_PRIMARY,
        text_secondary: TEXT_SECONDARY,
        text_tertiary: TEXT_TERTIARY,
        text_code: TEXT_CODE,
        text_code_type: Color::rgb(0xE9, 0xDC, 0xC9),
        text_line_number: TEXT_LINE_NUMBER,
        text_on_accent: TEXT_ON_ACCENT,
        text_on_cream: TEXT_ON_CREAM,
        accent: ACCENT,
        accent_text: ACCENT,
        accent_hover: ACCENT_HOVER,
        accent_pressed: ACCENT_PRESSED,
        blue_dark: BLUE_DARK,
        blue: BLUE,
        blue_bright: BLUE_BRIGHT,
        blue_light: BLUE_LIGHT,
        cream: CREAM,
        amber: AMBER,
        amber_text: AMBER,
        salmon: SALMON,
        salmon_text: SALMON,
        selected_row: SELECTED_ROW,
        active_entry: ACTIVE_ENTRY,
        focus_ring: FOCUS_RING,
        logo_ring: ACCENT,
        logo_dot: CREAM,
        dock_separator: Color::rgba(255, 255, 255, 0.28),
    };

    /// Values of `docs/design/THEME-CLAIR.md`. Its rules give the veils and the shadows: a
    /// white veil becomes night blue at the same opacity, capped at 0.16; a shadow keeps a
    /// third of its opacity (× 0.33). The fills of the green, amber, salmon, cream and blues,
    /// the selection and the soft badges do not change, as in the light boards. Five text
    /// colors replace those of the table, on the author's decision (`docs/appearance.md`).
    pub const LIGHT: Self = Self {
        desktop: Color::rgb(0xE9, 0xEE, 0xF4),
        desktop_discs: [
            Color::rgb(0xDD, 0xE6, 0xF0),
            Color::rgb(0xD4, 0xE0, 0xEC),
            Color::rgb(0xC8, 0xD8, 0xE9),
        ],
        system_screen: Color::rgb(0xEE, 0xF2, 0xF6),
        system_discs: [
            Color::rgb(0xE2, 0xE9, 0xF1),
            Color::rgb(0xD9, 0xE3, 0xEE),
            Color::rgb(0xD0, 0xDD, 0xEA),
        ],
        game: Color::rgb(0xE7, 0xED, 0xF4),
        deepest: Color::rgb(0xF6, 0xF8, 0xFB),
        window: Color::rgb(0xFF, 0xFF, 0xFF),
        title_bar: Color::rgb(0xF3, 0xF5, 0xF9),
        sidebar: Color::rgb(0xF6, 0xF8, 0xFB),
        card: Color::rgb(0xF3, 0xF6, 0xFA),
        terminal_block: Color::rgb(0xFF, 0xFF, 0xFF),
        glass: white(0.82),
        menu_glass: white(0.97),
        dock_glass: white(0.62),
        top_bar_glass: white(0.78),
        border: navy(0.10),
        menu_border: navy(0.14),
        card_border: navy(0.09),
        top_bar_border: navy(0.09),
        chip: navy(0.07),
        chip_hover: navy(0.12),
        top_bar_button_open: navy(0.14),
        dock_border: navy(0.16),
        secondary_border: navy(0.16),
        divider: navy(0.07),
        field: navy(0.07),
        hover: navy(0.07),
        pressed: navy(0.12),
        // Both dark values pass the 0.16 cap: capped, the hover would not show. The track at
        // rest takes the 0.12 of the pressed veil, a step below; hovered, the cap.
        toggle_off: navy(0.12),
        toggle_off_hover: navy(0.16),
        inset: navy(0.05),
        controls: navy(0.06),
        scrim: navy(0.45 * 0.33),
        amber_soft: AMBER_SOFT,
        accent_soft: ACCENT_SOFT,
        neutral_soft: navy(0.08),
        accent_border: ACCENT_BORDER,
        disabled: navy(0.05),
        shadow_base: navy(0.33),
        shadow: navy(0.55 * 0.33),
        menu_shadow: navy(0.6 * 0.33),
        text_primary: Color::rgb(0x14, 0x23, 0x3B),
        text_secondary: Color::rgb(0x46, 0x56, 0x6F),
        // This one and the four text colors below are darker than the table, to reach 4.5 to
        // 1 everywhere: the author's decision, see `docs/appearance.md`.
        text_tertiary: Color::rgb(0x4F, 0x5E, 0x75),
        text_code: Color::rgb(0x1E, 0x2B, 0x40),
        text_code_type: Color::rgb(0x7A, 0x4E, 0x12),
        text_line_number: Color::rgb(0x65, 0x73, 0x88),
        text_on_accent: TEXT_ON_ACCENT,
        text_on_cream: TEXT_ON_CREAM,
        accent: ACCENT,
        accent_text: Color::rgb(0x2C, 0x76, 0x4D),
        accent_hover: ACCENT_HOVER,
        accent_pressed: ACCENT_PRESSED,
        blue_dark: BLUE_DARK,
        blue: BLUE,
        blue_bright: BLUE_BRIGHT,
        blue_light: Color::rgb(0x1F, 0x64, 0xA0),
        cream: CREAM,
        amber: AMBER,
        amber_text: Color::rgb(0x93, 0x61, 0x14),
        salmon: SALMON,
        salmon_text: Color::rgb(0x92, 0x35, 0x2D),
        selected_row: SELECTED_ROW,
        active_entry: ACTIVE_ENTRY,
        // The light blue of the dark theme reads at 1.9 to 1 on white: the focus ring takes
        // the dark variant of the table, that of the keywords.
        focus_ring: Color::rgb(0x1F, 0x64, 0xA0),
        // The green of the table for icons: the fill green reads at 1.4 to 1 on the light bar.
        logo_ring: Color::rgb(0x2E, 0x7D, 0x4F),
        // Cream reads at 1.1 to 1 on the light bar and dock: the night blue of the light text.
        logo_dot: TEXT_ON_CREAM,
        dock_separator: navy(0.16),
    };

    /// The palette at `step` of a fade from dark (0) to light (`MIX_STEPS`).
    pub fn at_step(step: u8) -> &'static Self {
        static STEPS: OnceLock<Vec<Palette>> = OnceLock::new();
        match step {
            0 => &Self::DARK,
            s if s >= MIX_STEPS => &Self::LIGHT,
            s => &STEPS.get_or_init(|| {
                (0..=MIX_STEPS)
                    .map(|s| Self::DARK.mix(&Self::LIGHT, f32::from(s) / f32::from(MIX_STEPS)))
                    .collect()
            })[usize::from(s)],
        }
    }

    /// Mix of every color: `t` is 0 for `self`, 1 for `other`.
    pub fn mix(&self, other: &Self, t: f32) -> Self {
        let discs = |a: [Color; 3], b: [Color; 3]| [0, 1, 2].map(|i| a[i].lerp(b[i], t));
        macro_rules! mixed {
            ($($field:ident),+ $(,)?) => {
                Self {
                    desktop_discs: discs(self.desktop_discs, other.desktop_discs),
                    system_discs: discs(self.system_discs, other.system_discs),
                    $($field: self.$field.lerp(other.$field, t)),+
                }
            };
        }
        mixed!(
            desktop,
            system_screen,
            game,
            deepest,
            window,
            title_bar,
            sidebar,
            card,
            terminal_block,
            glass,
            menu_glass,
            dock_glass,
            top_bar_glass,
            border,
            menu_border,
            card_border,
            top_bar_border,
            chip,
            chip_hover,
            top_bar_button_open,
            dock_border,
            secondary_border,
            divider,
            field,
            hover,
            pressed,
            toggle_off,
            toggle_off_hover,
            inset,
            controls,
            scrim,
            amber_soft,
            accent_soft,
            neutral_soft,
            accent_border,
            disabled,
            shadow_base,
            shadow,
            menu_shadow,
            text_primary,
            text_secondary,
            text_tertiary,
            text_code,
            text_code_type,
            text_line_number,
            text_on_accent,
            text_on_cream,
            accent,
            accent_text,
            accent_hover,
            accent_pressed,
            blue_dark,
            blue,
            blue_bright,
            blue_light,
            cream,
            amber,
            amber_text,
            salmon,
            salmon_text,
            selected_row,
            active_entry,
            focus_ring,
            logo_ring,
            logo_dot,
            dock_separator,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opaque_color_keeps_its_channels() {
        assert_eq!(Color::rgb(255, 0, 51).premultiplied(), [1.0, 0.0, 0.2, 1.0]);
    }

    #[test]
    fn a_translucent_color_is_multiplied_by_its_opacity() {
        let [r, g, b, a] = Color::rgba(255, 255, 0, 0.5).premultiplied();
        assert_eq!((r, g, b, a), (0.5, 0.5, 0.0, 0.5));
    }

    #[test]
    fn contrast_follows_wcag() {
        let black = Color::rgb(0, 0, 0);
        let white = Color::rgb(255, 255, 255);
        assert!((black.contrast(white) - 21.0).abs() < 0.01);
        assert!((white.contrast(white) - 1.0).abs() < f32::EPSILON);
        // Reference value: #777777 on white gives 4.48.
        assert!((Color::rgb(0x77, 0x77, 0x77).contrast(white) - 4.48).abs() < 0.01);
    }

    #[test]
    fn a_veil_is_laid_as_in_the_browser() {
        let laid = Color::rgba(255, 255, 255, 0.5).over(Color::rgb(0, 0, 0));
        assert_eq!(laid, Color::rgb(128, 128, 128));
        assert_eq!(GLASS.over(DESKTOP).a, 1.0);
    }

    /// The palette table of the light theme design, as the author wrote it.
    const TABLE: &str = include_str!("../../../docs/design/THEME-CLAIR.md");

    /// Colors of a table cell, `#RRGGBB` or `rgba(r,g,b,a)`; an opacity written as a formula
    /// (`A`) is read as 1.
    fn colors_in(cell: &str) -> Vec<Color> {
        let mut found = Vec::new();
        for (i, _) in cell.match_indices('#') {
            let hex = |at: usize| u8::from_str_radix(&cell[i + at..i + at + 2], 16).unwrap();
            found.push(Color::rgb(hex(1), hex(3), hex(5)));
        }
        for (i, _) in cell.match_indices("rgba(") {
            let end = i + cell[i..].find(')').unwrap();
            let parts: Vec<&str> = cell[i + 5..end].split(',').map(str::trim).collect();
            let channel = |k: usize| parts[k].parse::<u8>().unwrap();
            let a = parts[3].parse::<f32>().unwrap_or(1.0);
            found.push(Color::rgba(channel(0), channel(1), channel(2), a));
        }
        found
    }

    /// Dark then light colors of the row of `role`.
    fn row(role: &str) -> (Vec<Color>, Vec<Color>) {
        let line = TABLE
            .lines()
            .find(|l| l.starts_with(&format!("| {role} |")))
            .unwrap_or_else(|| panic!("no row « {role} »"));
        let cells: Vec<&str> = line.split('|').collect();
        (colors_in(cells[2]), colors_in(cells[3]))
    }

    fn rgb(c: Color) -> (u8, u8, u8) {
        (c.r, c.g, c.b)
    }

    /// Rows of the table whose light value the author replaced, to reach 4.5 to 1: the value
    /// of the table, then the one kept.
    const OVERRIDES: [(&str, Color, Color); 5] = [
        (
            "Texte tertiaire",
            Color::rgb(0x5C, 0x6B, 0x82),
            Color::rgb(0x4F, 0x5E, 0x75),
        ),
        (
            "Numéros de ligne, texte fantôme",
            Color::rgb(0x7A, 0x88, 0x9D),
            Color::rgb(0x65, 0x73, 0x88),
        ),
        (
            "Vert, quand il colore du texte ou une icône",
            Color::rgb(0x2E, 0x7D, 0x4F),
            Color::rgb(0x2C, 0x76, 0x4D),
        ),
        (
            "Ambre, quand il colore du texte",
            Color::rgb(0x9A, 0x64, 0x12),
            Color::rgb(0x93, 0x61, 0x14),
        ),
        (
            "Saumon, quand il colore du texte",
            Color::rgb(0xB2, 0x3A, 0x2A),
            Color::rgb(0x92, 0x35, 0x2D),
        ),
    ];

    /// Rows of the table read token by token: the role, then the dark and light tokens.
    type Exact = (&'static str, Vec<Color>, Vec<Color>);

    fn exact() -> [Exact; 23] {
        let (d, l) = (&Palette::DARK, &Palette::LIGHT);
        let one = |dark: Color, light: Color| (vec![dark], vec![light]);
        let rows: [(&str, (Vec<Color>, Vec<Color>)); 23] = [
            ("Fond du bureau", one(d.desktop, l.desktop)),
            (
                "Disques du fond d'écran",
                (d.desktop_discs.to_vec(), l.desktop_discs.to_vec()),
            ),
            (
                "Fond des écrans système",
                one(d.system_screen, l.system_screen),
            ),
            (
                "Disques des écrans système",
                (d.system_discs.to_vec(), l.system_discs.to_vec()),
            ),
            ("Fond du mode jeu", one(d.game, l.game)),
            ("Terminal, zones de code", one(d.deepest, l.deepest)),
            ("Fenêtre", one(d.window, l.window)),
            ("Barre de titre", one(d.title_bar, l.title_bar)),
            ("Panneau latéral", one(d.sidebar, l.sidebar)),
            ("Carte", one(d.card, l.card)),
            (
                "Bloc du terminal, carte de l'installeur",
                one(d.terminal_block, l.terminal_block),
            ),
            ("Verre des menus", one(d.menu_glass, l.menu_glass)),
            ("Verre du dock", one(d.dock_glass, l.dock_glass)),
            ("Texte principal", one(d.text_primary, l.text_primary)),
            ("Texte secondaire", one(d.text_secondary, l.text_secondary)),
            ("Texte tertiaire", one(d.text_tertiary, l.text_tertiary)),
            (
                "Numéros de ligne, texte fantôme",
                one(d.text_line_number, l.text_line_number),
            ),
            ("Texte du code", one(d.text_code, l.text_code)),
            (
                "Mots-clés du code, dossiers",
                one(d.blue_light, l.blue_light),
            ),
            (
                "Types dans le code",
                one(d.text_code_type, l.text_code_type),
            ),
            (
                "Vert, quand il colore du texte ou une icône",
                one(d.accent_text, l.accent_text),
            ),
            (
                "Ambre, quand il colore du texte",
                one(d.amber_text, l.amber_text),
            ),
            (
                "Saumon, quand il colore du texte",
                one(d.salmon_text, l.salmon_text),
            ),
        ];
        rows.map(|(role, (dark, light))| (role, dark, light))
    }

    /// Rows given as a rule rather than as values.
    const RULES: [&str; 3] = [
        "Verre des panneaux",
        "Teintes et bordures légères",
        "Ombres",
    ];

    #[test]
    fn every_token_has_the_value_of_the_light_theme_table() {
        let (d, l) = (&Palette::DARK, &Palette::LIGHT);
        for (role, dark, light) in exact() {
            let (table_dark, mut table_light) = row(role);
            if let Some((_, from, to)) = OVERRIDES.iter().find(|(r, ..)| *r == role) {
                assert_eq!(table_light, [*from], "{role}: the table changed");
                table_light = vec![*to];
            }
            assert_eq!((table_dark, table_light), (dark, light), "{role}");
        }
        // Rows given as a rule: same tint, the opacity follows the dark one.
        let (dark, light) = row("Verre des panneaux");
        for (d, l) in [(d.glass, l.glass), (d.top_bar_glass, l.top_bar_glass)] {
            assert_eq!((rgb(d), rgb(l)), (rgb(dark[0]), rgb(light[0])));
            assert_eq!(l.a, d.a);
        }
        let (dark, light) = row("Teintes et bordures légères");
        for (d, l) in [
            (d.border, l.border),
            (d.menu_border, l.menu_border),
            (d.card_border, l.card_border),
            (d.top_bar_border, l.top_bar_border),
            (d.chip, l.chip),
            (d.chip_hover, l.chip_hover),
            (d.top_bar_button_open, l.top_bar_button_open),
            (d.dock_border, l.dock_border),
            (d.secondary_border, l.secondary_border),
            (d.divider, l.divider),
            (d.field, l.field),
            (d.hover, l.hover),
            (d.pressed, l.pressed),
            (d.toggle_off_hover, l.toggle_off_hover),
            (d.dock_separator, l.dock_separator),
            (d.inset, l.inset),
            (d.controls, l.controls),
            (d.neutral_soft, l.neutral_soft),
            (d.disabled, l.disabled),
        ] {
            assert_eq!((rgb(d), rgb(l)), (rgb(dark[0]), rgb(light[0])));
            // « opacité plafonnée à 0,16 ».
            assert_eq!(l.a, d.a.min(0.16), "{d:?}");
        }
        // The off toggle at rest, a step below its hover (`Palette::LIGHT`).
        assert_eq!(rgb(l.toggle_off), rgb(light[0]));
        assert!(l.toggle_off.a < l.toggle_off_hover.a && l.toggle_off.a < 0.16);
        let (dark, light) = row("Ombres");
        for (d, l) in [
            (d.shadow, l.shadow),
            (d.menu_shadow, l.menu_shadow),
            (d.scrim, l.scrim),
            (d.shadow_base, l.shadow_base),
        ] {
            assert_eq!((rgb(d), rgb(l)), (rgb(dark[0]), rgb(light[0])));
            assert!((l.a - d.a * 0.33).abs() < 1e-6, "{l:?}");
        }
    }

    #[test]
    fn every_row_of_the_table_is_checked() {
        // The rows read by `every_token_has_the_value_of_the_light_theme_table`: a row added
        // to the table later must be added there too.
        let mut checked: Vec<&str> = exact().iter().map(|(role, ..)| *role).collect();
        checked.extend(RULES);
        let rows: Vec<&str> = TABLE
            .lines()
            .filter(|l| l.starts_with("| ") && !l.starts_with("| ---") && !l.starts_with("| Rôle"))
            .filter_map(|l| l.split('|').nth(1).map(str::trim))
            .collect();
        let mut rows = rows;
        rows.sort_unstable();
        checked.sort_unstable();
        assert_eq!(rows, checked);
    }

    #[test]
    fn the_rings_take_colors_of_the_table() {
        let (d, l) = (&Palette::DARK, &Palette::LIGHT);
        // Focus ring: the keyword blue of each theme, the light one being the dark variant.
        assert_eq!(d.focus_ring, row("Mots-clés du code, dossiers").0[0]);
        assert_eq!(l.focus_ring, row("Mots-clés du code, dossiers").1[0]);
        // Logo ring: the green fill in dark; in light, the icon green as the table gives it,
        // before the author darkened the green of texts.
        assert_eq!(d.logo_ring, d.accent);
        let green = "Vert, quand il colore du texte ou une icône";
        assert_eq!(l.logo_ring, row(green).1[0]);
    }

    #[test]
    fn fills_of_sherlock_and_of_the_accents_do_not_change() {
        let (d, l) = (&Palette::DARK, &Palette::LIGHT);
        for (name, dark, light) in [
            ("accent", d.accent, l.accent),
            ("accent hover", d.accent_hover, l.accent_hover),
            ("accent pressed", d.accent_pressed, l.accent_pressed),
            ("amber", d.amber, l.amber),
            ("salmon", d.salmon, l.salmon),
            ("cream", d.cream, l.cream),
            ("text on cream", d.text_on_cream, l.text_on_cream),
            ("text on accent", d.text_on_accent, l.text_on_accent),
            ("blue dark", d.blue_dark, l.blue_dark),
            ("blue", d.blue, l.blue),
            ("blue bright", d.blue_bright, l.blue_bright),
            ("selected row", d.selected_row, l.selected_row),
            ("active entry", d.active_entry, l.active_entry),
            ("amber soft", d.amber_soft, l.amber_soft),
            ("accent soft", d.accent_soft, l.accent_soft),
            ("accent border", d.accent_border, l.accent_border),
        ] {
            assert_eq!(dark, light, "{name}");
        }
        assert_eq!(d.text_on_accent, Color::rgb(0x0B, 0x12, 0x20));
        assert_eq!(d.text_on_cream, Color::rgb(0x14, 0x23, 0x3B));
    }

    #[test]
    fn the_active_theme_gives_its_palette_and_fades_to_the_next() {
        // The only test of this crate that changes the active theme: no other one races it.
        assert_eq!(active(), Theme::Dark);
        set_active(Theme::Light);
        assert_eq!(palette(), &Palette::LIGHT);
        set_active(Theme::Dark);
        assert_eq!(palette(), &Palette::DARK);

        let start = Instant::now();
        let mut fade = ThemeFade::default();
        // The first theme comes at once: the one the program starts in.
        fade.set(Theme::Dark, start);
        assert!(!fade.running());
        fade.set(Theme::Light, start);
        assert!(fade.running());
        assert!(fade.tick(start + motion::THEME_FADE / 4));
        let between = palette();
        assert_ne!(between, &Palette::DARK);
        assert_ne!(between, &Palette::LIGHT);
        // Turned back halfway: it comes back from where it stands, in less than a full fade.
        let back = start + motion::THEME_FADE / 4;
        fade.set(Theme::Dark, back);
        assert!(fade.tick(back));
        assert!(!fade.tick(back + motion::THEME_FADE));
        assert_eq!(palette(), &Palette::DARK);
        fade.set(Theme::Light, back);
        assert!(!fade.tick(back + motion::THEME_FADE));
        assert_eq!((active(), palette()), (Theme::Light, &Palette::LIGHT));
        set_active(Theme::Dark);
    }

    #[test]
    fn the_steps_of_a_fade_go_from_one_palette_to_the_other() {
        assert_eq!(Palette::at_step(0), &Palette::DARK);
        assert_eq!(Palette::at_step(MIX_STEPS), &Palette::LIGHT);
        let (dark, light) = (Palette::DARK.text_primary, Palette::LIGHT.text_primary);
        let texts: Vec<Color> = (0..=MIX_STEPS)
            .map(|s| Palette::at_step(s).text_primary)
            .collect();
        // Each step darkens the text, so that each one is known by it (`theme::palette`).
        assert!(texts.windows(2).all(|w| w[1].r < w[0].r), "{texts:?}");
        assert_eq!((texts[0], texts[usize::from(MIX_STEPS)]), (dark, light));
        // Translucent colors mix their opacity too.
        let half = Palette::at_step(MIX_STEPS / 2).dock_glass;
        assert!(half.a > Palette::DARK.dock_glass.a && half.a < Palette::LIGHT.dock_glass.a);
    }

    #[test]
    fn shades_stay_close_to_the_green() {
        assert_eq!(ACCENT.mix(DEEPEST, 0.0), ACCENT);
        assert!(ACCENT_HOVER.luminance() > ACCENT.luminance());
        assert!(ACCENT_PRESSED.luminance() < ACCENT.luminance());
    }
}
