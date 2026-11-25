//! Prise en main des fenêtres par le compositeur : saisie des bords, calage contre l'écran.
//!
//! Les tailles sont en pixels logiques. `docs/contexte.md` ne les fixe pas : ce sont des choix
//! du lot D-17, à revoir avec le design.

use crate::color::{self, Color};

/// Largeur de la bande de saisie, à l'extérieur de chaque côté d'une fenêtre.
pub const RESIZE_BAND: i32 = 8;

/// Taille de la saisie d'un coin : le long des côtés depuis le coin, et en diagonale hors de
/// la fenêtre. Plus large que la bande, pour qu'un coin se trouve sans viser au pixel.
pub const RESIZE_CORNER: i32 = 20;

/// Distance au bord de l'écran sous laquelle une fenêtre lâchée se cale.
pub const SNAP_EDGE: i32 = 8;

/// Distance à un coin de l'écran, le long d'un bord, sous laquelle une fenêtre lâchée prend
/// ce quart de l'écran.
pub const SNAP_CORNER: i32 = 64;

/// Fond de l'aperçu de calage : celui d'une entrée active.
pub const SNAP_PREVIEW_FILL: Color = color::ACTIVE_ENTRY;

/// Bordure de l'aperçu de calage.
pub const SNAP_PREVIEW_BORDER: Color = color::ACCENT;

/// Épaisseur de la bordure de l'aperçu de calage.
pub const SNAP_PREVIEW_BORDER_WIDTH: i32 = 2;
