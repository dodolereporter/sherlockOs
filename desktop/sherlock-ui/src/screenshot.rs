//! Screenshots taken by the compositor (lot D-35c): their folder and file names, the
//! notification that confirms them, and the look of the area being chosen. No board draws
//! them: the look is that of the snap preview over a dimmed screen. See `docs/screenshots.md`.

use crate::color::{self, Color};

/// The pictures folder when the user has none set (`XDG_PICTURES_DIR`), in the home folder.
pub const PICTURES: &str = "Images";
/// The folder of the screenshots, inside the pictures folder.
pub const FOLDER: &str = "Captures d'écran";
/// The name the notification gives as its sender.
pub const APP: &str = "Capture d'écran";
/// The notification, once the file is written: its title, then the file name.
pub const SAVED: &str = "Capture d'écran enregistrée";
/// The body of the notification when the image is in the clipboard too.
pub fn saved_body(file: &str) -> String {
    format!("{file}, copiée dans le presse-papiers")
}

/// The name of a screenshot taken at that local date and time; `copy` from 2 when the name
/// is already taken.
pub fn file_name(date: (i32, u32, u32), time: (u32, u32, u32), copy: u32) -> String {
    let (y, mo, d) = date;
    let (h, mi, s) = time;
    let base = format!("Capture d'écran du {y:04}-{mo:02}-{d:02} à {h:02}.{mi:02}.{s:02}");
    if copy < 2 {
        format!("{base}.png")
    } else {
        format!("{base} ({copy}).png")
    }
}

/// Opacity of the dim laid over the screen while an area is chosen, in the scrim color.
pub const DIM: f32 = 0.6;

/// The rectangle being chosen: inside, and its border.
pub fn area_fill() -> Color {
    color::palette().active_entry
}

pub fn area_border() -> Color {
    color::palette().accent
}

/// Width of the border of the rectangle being chosen.
pub const AREA_BORDER_WIDTH: f32 = 2.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_carries_the_date_and_its_copy() {
        let name = file_name((2026, 11, 26), (9, 5, 7), 0);
        assert_eq!(name, "Capture d'écran du 2026-11-26 à 09.05.07.png");
        let again = file_name((2026, 11, 26), (9, 5, 7), 2);
        assert_eq!(again, "Capture d'écran du 2026-11-26 à 09.05.07 (2).png");
        assert!(!name.contains('/') && !name.contains(':'));
    }
}
