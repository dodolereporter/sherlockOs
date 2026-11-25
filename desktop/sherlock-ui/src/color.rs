//! Theme colors, named by role. Values from `docs/contexte.md` and the design boards.

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
pub const DESKTOP: Color = Color::rgb(0x0E, 0x1A, 0x2E);
/// The three large discs of the wallpaper, from darkest to lightest.
pub const DESKTOP_DISCS: [Color; 3] = [
    Color::rgb(0x13, 0x2A, 0x49),
    Color::rgb(0x16, 0x31, 0x4F),
    Color::rgb(0x1B, 0x3F, 0x63),
];
/// Background of the system screens: login, boot, installer.
pub const SYSTEM_SCREEN: Color = Color::rgb(0x0A, 0x14, 0x24);
/// Darkest background: terminal, code areas. Also used as text on the green.
pub const DEEPEST: Color = Color::rgb(0x0B, 0x12, 0x20);
/// Body of opaque windows.
pub const WINDOW: Color = Color::rgb(0x0F, 0x18, 0x28);
/// Title bar of windows.
pub const TITLE_BAR: Color = Color::rgb(0x13, 0x1E, 0x32);
/// Side panel: navigation on the left, preview on the right.
pub const SIDEBAR: Color = Color::rgb(0x11, 0x1B, 0x2E);
/// Card inside a window.
pub const CARD: Color = Color::rgb(0x15, 0x22, 0x38);
/// Command block of the terminal.
pub const TERMINAL_BLOCK: Color = Color::rgb(0x10, 0x1B, 0x2E);

// Glass. The blur is the compositor's business: here, only the tint.

/// Floating panels (24 px blur).
pub const GLASS: Color = Color::rgba(16, 26, 44, 0.82);
/// Menus and submenus (28 px blur).
pub const MENU_GLASS: Color = Color::rgba(22, 34, 56, 0.96);
/// Dock (30 px blur).
pub const DOCK_GLASS: Color = Color::rgba(244, 235, 227, 0.17);
/// Top bar (24 px blur). The Main design board makes it slightly lighter than `GLASS`.
pub const TOP_BAR_GLASS: Color = Color::rgba(16, 26, 44, 0.78);

// Lines and veils.

/// Border of a glass panel and of a window, menu separator.
pub const BORDER: Color = Color::rgba(255, 255, 255, 0.10);
/// Border of a menu (Menus design board).
pub const MENU_BORDER: Color = Color::rgba(255, 255, 255, 0.14);
/// Border of a card (« Réglages » design board).
pub const CARD_BORDER: Color = Color::rgba(255, 255, 255, 0.09);
/// Border of the top bar (Main design board).
pub const TOP_BAR_BORDER: Color = Color::rgba(255, 255, 255, 0.09);
/// Chip of an inactive workspace in the top bar, at rest then hovered.
pub const CHIP: Color = Color::rgba(255, 255, 255, 0.07);
pub const CHIP_HOVER: Color = Color::rgba(255, 255, 255, 0.12);
/// Veil of a top bar button whose menu is open (the open « Fenêtre » button of the Menus
/// board).
pub const TOP_BAR_BUTTON_OPEN: Color = Color::rgba(255, 255, 255, 0.14);
/// Border of the dock.
pub const DOCK_BORDER: Color = Color::rgba(255, 255, 255, 0.26);
/// Border of a secondary button.
pub const SECONDARY_BORDER: Color = Color::rgba(255, 255, 255, 0.18);
/// Line between two rows of a card, bottom of the title bar.
pub const DIVIDER: Color = Color::rgba(255, 255, 255, 0.07);
/// Background of a text field.
pub const FIELD: Color = Color::rgba(255, 255, 255, 0.07);
/// Hover veil on an element without background.
pub const HOVER: Color = Color::rgba(255, 255, 255, 0.07);
/// Press veil on an element without background.
pub const PRESSED: Color = Color::rgba(255, 255, 255, 0.12);
/// Track of a toggle that is off, at rest and hovered. The design boards only show toggles
/// that are on: the off track reuses the border of a secondary button.
pub const TOGGLE_OFF: Color = Color::rgba(255, 255, 255, 0.18);
pub const TOGGLE_OFF_HOVER: Color = Color::rgba(255, 255, 255, 0.26);
/// Inset area of a toolbar: the breadcrumb (Evidence mock-up).
pub const INSET: Color = Color::rgba(255, 255, 255, 0.05);
/// Pill behind the window buttons.
pub const CONTROLS: Color = Color::rgba(255, 255, 255, 0.06);
/// Veil laid over a window behind a question awaiting an answer.
pub const SCRIM: Color = Color::rgba(3, 8, 18, 0.45);
/// Background of a status badge: warning, success, neutral (git status in the Evidence
/// mock-up), and border of a green badge without background (git branch).
pub const AMBER_SOFT: Color = Color::rgba(240, 196, 138, 0.18);
pub const ACCENT_SOFT: Color = Color::rgba(133, 197, 149, 0.18);
pub const NEUTRAL_SOFT: Color = Color::rgba(255, 255, 255, 0.08);
pub const ACCENT_BORDER: Color = Color::rgba(133, 197, 149, 0.5);
/// Background of a disabled element.
pub const DISABLED: Color = Color::rgba(255, 255, 255, 0.05);
/// Colour of the crisp shadows of floating surfaces (`surface::Shadowed`), the same blue
/// black as `SHADOW` and `MENU_SHADOW`; each step of the shadow sets its own opacity.
pub const SHADOW_BASE: Color = Color::rgb(3, 8, 18);
/// Shadow of windows and panels.
pub const SHADOW: Color = Color::rgba(3, 8, 18, 0.55);
/// Shadow of menus.
pub const MENU_SHADOW: Color = Color::rgba(3, 8, 18, 0.6);

// Texts.

/// Titles and content.
pub const TEXT_PRIMARY: Color = Color::rgb(0xEE, 0xF2, 0xF7);
/// Descriptions, values.
pub const TEXT_SECONDARY: Color = Color::rgb(0xC5, 0xCE, 0xDB);
/// Labels, shortcuts, text of a disabled element.
pub const TEXT_TERTIARY: Color = Color::rgb(0xA9, 0xB4, 0xC6);
/// Text of a code snippet or of a file preview (Evidence mock-up).
pub const TEXT_CODE: Color = Color::rgb(0xDC, 0xE4, 0xEE);
/// Line numbers.
pub const TEXT_LINE_NUMBER: Color = Color::rgb(0x84, 0x92, 0xA8);
/// Text laid on the green.
pub const TEXT_ON_ACCENT: Color = DEEPEST;
/// Text laid on the cream.
pub const TEXT_ON_CREAM: Color = Color::rgb(0x14, 0x23, 0x3B);

// Accents.

/// Green of the scarf: main action, active state, success.
pub const ACCENT: Color = Color::rgb(0x85, 0xC5, 0x95);
/// Hovered green.
pub const ACCENT_HOVER: Color = ACCENT.mix(TEXT_PRIMARY, 0.18);
/// Pressed green.
pub const ACCENT_PRESSED: Color = ACCENT.mix(DEEPEST, 0.14);
/// Blue of icons and folders, the darkest.
pub const BLUE_DARK: Color = Color::rgb(0x2B, 0x5F, 0x86);
/// Blue of icons and folders.
pub const BLUE: Color = Color::rgb(0x32, 0x6D, 0x93);
/// Blue of the selection and of code keywords.
pub const BLUE_BRIGHT: Color = Color::rgb(0x5B, 0xA3, 0xD0);
/// Light blue of code keywords.
pub const BLUE_LIGHT: Color = Color::rgb(0x8F, 0xC1, 0xE8);
/// What comes from Sherlock or from the trust system.
pub const CREAM: Color = Color::rgb(0xF4, 0xEB, 0xE3);
/// Awaiting a consent, warning.
pub const AMBER: Color = Color::rgb(0xF0, 0xC4, 0x8A);
/// Error, destructive action.
pub const SALMON: Color = Color::rgb(0xF6, 0xB5, 0xA6);

/// Fully transparent: the background of a shell surface that draws its own shapes.
pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0.0);

/// Selected row of a list or of a menu, hovered menu entry.
pub const SELECTED_ROW: Color = Color::rgba(91, 163, 208, 0.26);
/// Background of the active entry of a navigation.
pub const ACTIVE_ENTRY: Color = Color::rgba(133, 197, 149, 0.16);
/// Keyboard focus ring. The design boards show none: light blue, visible on every background
/// and distinct from the green of the active state.
pub const FOCUS_RING: Color = BLUE_LIGHT;

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

    #[test]
    fn shades_stay_close_to_the_green() {
        assert_eq!(ACCENT.mix(DEEPEST, 0.0), ACCENT);
        assert!(ACCENT_HOVER.luminance() > ACCENT.luminance());
        assert!(ACCENT_PRESSED.luminance() < ACCENT.luminance());
    }
}
