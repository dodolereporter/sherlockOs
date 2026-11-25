//! Valeurs du design pour ce prototype.
//!
//! Dans le code durable, elles viendront de `desktop/sherlock-ui` (lot D-01). Ici, elles sont
//! regroupées en un seul endroit et recopiées de `docs/design/planches/Main.dc.html` et
//! `Menus.dc.html`. Le prototype Slint porte les mêmes valeurs dans `ui/barre.slint`.

use iced::Color;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba8(r, g, b, a)
}

// Couleurs
pub const TEXT: Color = rgb(0xEE, 0xF2, 0xF7);
pub const TEXT_2: Color = rgb(0xC5, 0xCE, 0xDB);
pub const TEXT_3: Color = rgb(0xA9, 0xB4, 0xC6);
pub const TEXT_ON_ACCENT: Color = rgb(0x0B, 0x12, 0x20);
pub const ACCENT: Color = rgb(0x85, 0xC5, 0x95);
pub const CREAM: Color = rgb(0xF4, 0xEB, 0xE3);
pub const BAR_GLASS: Color = rgba(16, 26, 44, 0.78);
pub const BAR_BORDER: Color = rgba(255, 255, 255, 0.09);
pub const MENU_GLASS: Color = rgba(22, 34, 56, 0.96);
pub const MENU_BORDER: Color = rgba(255, 255, 255, 0.14);
pub const MENU_BUTTON_OPEN: Color = rgba(255, 255, 255, 0.14);
pub const DESKTOP_IDLE: Color = rgba(255, 255, 255, 0.07);
pub const DESKTOP_HOVER: Color = rgba(255, 255, 255, 0.12);
pub const SELECTION: Color = rgba(91, 163, 208, 0.26);
pub const SEPARATOR: Color = rgba(255, 255, 255, 0.10);

// Typographie : Inter n'est pas installée sur le poste, les deux prototypes prennent Noto Sans.
pub const FONT_FAMILY: &str = "Noto Sans";
pub const FONT_SIZE: f32 = 13.0;
pub const SHORTCUT_SIZE: f32 = 12.0;

// Barre
pub const BAR_HEIGHT: u32 = 36;
pub const BAR_MARGIN_TOP: i32 = 10;
pub const BAR_MARGIN_SIDE: i32 = 12;
pub const BAR_RADIUS: f32 = 12.0;
pub const BAR_PAD_LEFT: f32 = 6.0;
pub const BAR_PAD_RIGHT: f32 = 10.0;
pub const LEFT_GAP: f32 = 4.0;
pub const RIGHT_GAP: f32 = 14.0;
pub const SHERLOCK_W: f32 = 30.0;
pub const SHERLOCK_H: f32 = 28.0;
pub const SHERLOCK_RADIUS: f32 = 8.0;
pub const APP_NAME_W: f32 = 70.0;
pub const MENU_BUTTON_W: f32 = 70.0;
pub const MENU_BUTTON_H: f32 = 26.0;
pub const MENU_BUTTON_RADIUS: f32 = 6.0;
pub const DESKTOP_H: f32 = 24.0;
pub const DESKTOP_PAD_X: f32 = 12.0;
pub const DESKTOP_RADIUS: f32 = 12.0;

// Menus
pub const MENU_W: u32 = 256;
pub const SUBMENU_W: u32 = 190;
/// Écart entre le bas de la barre et le haut du menu, et entre le menu et son sous-menu.
pub const MENU_GAP: f32 = 6.0;
pub const SUBMENU_GAP: f32 = 4.0;
pub const MENU_RADIUS: f32 = 12.0;
pub const MENU_PADDING: f32 = 6.0;
pub const MENU_ITEM_H: f32 = 30.0;
pub const MENU_ITEM_RADIUS: f32 = 7.0;
pub const MENU_ITEM_PAD_X: f32 = 10.0;
pub const MENU_ITEM_GAP: f32 = 12.0;
pub const SEPARATOR_MARGIN_Y: f32 = 4.0;
pub const SEPARATOR_MARGIN_X: f32 = 8.0;
/// Animation d'ouverture : fondu et glissé de 6 px, 180 ms.
pub const MENU_ANIM_MS: u64 = 180;
pub const MENU_SLIDE: f32 = 6.0;

// Textes d'interface
pub const APP_NAME: &str = "Éditeur";
pub const MENU_LABEL: &str = "Fenêtre";
pub const DESKTOPS: [&str; 3] = ["1 · Code", "2 · Web", "3 · Jeu"];

pub struct MenuItem {
    pub label: &'static str,
    pub shortcut: &'static str,
    pub submenu: bool,
}

pub const MENU_ITEMS: [MenuItem; 5] = [
    MenuItem {
        label: "Réduire",
        shortcut: "Super+M",
        submenu: false,
    },
    MenuItem {
        label: "Agrandir",
        shortcut: "Super+Haut",
        submenu: false,
    },
    MenuItem {
        label: "Moitié gauche",
        shortcut: "Super+Gauche",
        submenu: false,
    },
    MenuItem {
        label: "Déplacer vers le bureau",
        shortcut: "",
        submenu: true,
    },
    MenuItem {
        label: "Fermer la fenêtre",
        shortcut: "Super+Q",
        submenu: false,
    },
];
pub const MENU_SEPARATOR_BEFORE: usize = 4;
const SUBMENU_INDEX: usize = 3;

/// Taille de la surface du menu : l'écart sous la barre, la bordure, la marge, cinq entrées
/// et un séparateur.
pub fn menu_size() -> (u32, u32) {
    let h =
        MENU_GAP + 2.0 + 2.0 * MENU_PADDING + 5.0 * MENU_ITEM_H + 1.0 + 2.0 * SEPARATOR_MARGIN_Y;
    (MENU_W, h as u32)
}

pub fn submenu_size() -> (u32, u32) {
    let h = 2.0 + 2.0 * MENU_PADDING + 3.0 * MENU_ITEM_H;
    (SUBMENU_GAP as u32 + SUBMENU_W, h as u32)
}

/// Rectangle du bouton « Fenêtre » dans la barre. Les largeurs sont fixes, comme sur la
/// planche Menus, ce qui permet de placer le menu sans relire la mise en page.
pub fn menu_anchor_rect() -> (i32, i32, i32, i32) {
    let x = BAR_PAD_LEFT + SHERLOCK_W + LEFT_GAP + APP_NAME_W + LEFT_GAP;
    (x as i32, 0, MENU_BUTTON_W as i32, BAR_HEIGHT as i32)
}

/// Rectangle de l'entrée à sous-menu dans la surface du menu. Le sous-menu s'ouvre à droite,
/// son bord haut aligné sur celui du menu moins sa marge.
pub fn submenu_anchor_rect() -> (i32, i32, i32, i32) {
    let y = MENU_GAP + 1.0 + MENU_PADDING + SUBMENU_INDEX as f32 * MENU_ITEM_H - MENU_PADDING - 1.0;
    (0, y as i32, MENU_W as i32, MENU_ITEM_H as i32)
}

// Mode automatique pour les mesures : 4 s de repos, puis dix ouvertures du menu.
pub const AUTO_STEP_MS: u64 = 250;
pub const AUTO_IDLE_STEPS: u32 = 16;
pub const AUTO_CYCLES: u32 = 10;

const DAYS: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];
const MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

/// Heure et date au format de la planche : « 21:46 » et « mar. 24 nov. ».
pub fn now_strings() -> (String, String) {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    let clock = format!("{:02}:{:02}", now.hour(), now.minute());
    let day = DAYS[now.weekday().num_days_from_monday() as usize];
    let month = MONTHS[now.month0() as usize];
    (clock, format!("{day} {} {month}", now.day()))
}
