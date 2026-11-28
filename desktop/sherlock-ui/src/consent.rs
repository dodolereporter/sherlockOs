//! The consent window of Sherlock (lot A-11b, ADR 0031): drawn by the compositor itself, above
//! every surface, when Sherlock asks to run a tool that is not a read. No board draws it yet: the
//! measures are choices of the lot, to review with the design.

use std::time::Duration;

use crate::color::{Color, Palette};

/// The panel, centered on each screen: its width, padding, radius, the height of a line of text,
/// the gap between lines.
pub const WIDTH: f32 = 480.0;
pub const PADDING: f32 = 24.0;
pub const RADIUS: f32 = 16.0;
pub const LINE_HEIGHT: f32 = 24.0;
pub const LINE_GAP: f32 = 4.0;
/// The two buttons: their size, the gap between them, their radius.
pub const BUTTON_WIDTH: f32 = 140.0;
pub const BUTTON_HEIGHT: f32 = 36.0;
pub const BUTTON_GAP: f32 = 12.0;
pub const BUTTON_RADIUS: f32 = 10.0;
/// Characters of the arguments per line, and lines of them at most: the rest ends with « … ».
pub const ARGUMENT_COLUMNS: usize = 56;
pub const ARGUMENT_LINES: usize = 4;

/// The buttons do nothing this long after the window appears: a click or a key already on its
/// way grants nothing and refuses nothing.
pub const ARMING: Duration = Duration::from_millis(600);
/// How long « Autoriser » is held for a destructive tool (the reinforced consent).
pub const HOLD: Duration = Duration::from_millis(1500);

pub mod text {
    pub const TITLE: &str = "Sherlock demande ton accord";
    pub const ACTION: &str = "Cette action modifie ton système.";
    pub const DESTRUCTIVE: &str = "Cette action ne peut pas être annulée.";
    pub const HOLD: &str = "Maintiens « Autoriser » pour confirmer.";
    pub const ALLOW: &str = "Autoriser";
    pub const DENY: &str = "Refuser";

    /// The line that names the tool.
    pub fn tool(name: &str) -> String {
        format!("Outil : {name}")
    }
}

/// The colors of the window in a palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub scrim: Color,
    pub panel: Color,
    pub border: Color,
    pub text: Color,
    pub secondary: Color,
    pub deny: Color,
    pub allow: Color,
    /// « Autoriser » of a destructive tool, and the ring of its hold.
    pub danger: Color,
    pub on_button: Color,
}

impl Colors {
    pub const fn of(palette: &Palette) -> Self {
        Self {
            scrim: palette.scrim,
            panel: palette.window,
            border: palette.menu_border,
            text: palette.text_primary,
            secondary: palette.text_secondary,
            deny: palette.card,
            allow: palette.accent,
            danger: palette.salmon,
            on_button: palette.text_on_accent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_reads_in_both_themes() {
        for p in [&Palette::DARK, &Palette::LIGHT] {
            let c = Colors::of(p);
            assert!(c.text.contrast(c.panel) >= 4.5);
            assert!(c.secondary.contrast(c.panel) >= 4.5);
            assert!(c.on_button.contrast(c.allow) >= 4.5, "{p:?}");
            assert!(c.on_button.contrast(c.danger) >= 4.5, "danger {p:?}");
            assert!(
                c.panel.a >= 1.0,
                "an opaque panel: nothing shows through it"
            );
        }
    }
}
