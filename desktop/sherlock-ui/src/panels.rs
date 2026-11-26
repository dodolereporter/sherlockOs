//! Panels of the top bar (batch D-20b): the notification centre and the quick settings of the
//! Panels design board, `docs/design/planches/Panneaux.dc.html` (and its light version), and
//! the calendar under the clock, which no board draws: it follows the same panel glass and
//! the rules of the visual identity. Tokens, colors, styles and the French texts.

use crate::color::{Color, Palette};
use crate::{metrics, motion, typography};

// Geometry, in logical pixels.

/// Distance between the area the top bar reserves and the panels: the board puts them at 54,
/// the bar ends at 46.
pub const BELOW_BAR: f32 = 8.0;
/// Inner padding and radius of a panel (the board's `padding: 14px`, `border-radius: 20px`).
pub const PADDING: f32 = metrics::PANEL_PADDING;
pub const RADIUS: f32 = metrics::PANEL_RADIUS;
/// Gap between the notification centre and the quick settings on the board (756 + 320 + 16 =
/// 1092).
pub const PANEL_GAP: f32 = 16.0;

/// Notification centre: width, gap between its rows, title size, « Tout effacer » button.
pub const CENTER_WIDTH: f32 = 320.0;
pub const CENTER_GAP: f32 = 8.0;
pub const TITLE_SIZE: f32 = typography::TITLE_SMALL_SIZE;
pub const CLEAR_HEIGHT: f32 = 26.0;
pub const CLEAR_RADIUS: f32 = 8.0;
pub const CLEAR_PADDING: f32 = 10.0;
/// Card of a group of notifications: padding, gap between its rows, radius, the application
/// tile and its initial, gap in the header, and the gap between two notifications of a group.
pub const CARD_PADDING: f32 = 12.0;
pub const CARD_GAP: f32 = 6.0;
pub const CARD_RADIUS: f32 = metrics::CARD_RADIUS;
pub const TILE: f32 = 20.0;
pub const TILE_RADIUS: f32 = 6.0;
pub const TILE_LETTER_SIZE: f32 = 11.0;
pub const HEADER_GAP: f32 = 8.0;
pub const ENTRY_GAP: f32 = 10.0;
/// Action buttons of an open notification: compact, as on the board.
pub const ACTION_HEIGHT: f32 = metrics::BUTTON_COMPACT_HEIGHT;
pub const ACTION_RADIUS: f32 = metrics::BUTTON_COMPACT_RADIUS;
pub const ACTION_PADDING: f32 = 12.0;
pub const ACTION_GAP: f32 = 6.0;
/// Notifications of one application shown in its card, the newest ones; the others are
/// counted below them.
pub const GROUP_MAX: usize = 3;

/// Quick settings: width. Its height sets the height of the whole control centre, the
/// notification centre included (see `CONTROL_HEIGHT`).
pub const QUICK_WIDTH: f32 = 336.0;
/// Height of the control centre panels: the board's quick settings (a grid of two rows of 56
/// and 8, the volume and brightness rows, the two segmented groups, the user row, 14 between
/// each), plus the line under the volume that names the output.
pub const CONTROL_HEIGHT: f32 = 2.0 * (PADDING + metrics::BORDER_WIDTH)
    + (2.0 * 56.0 + 8.0)
    + 5.0 * metrics::GAP_SECTION
    + (28.0 + 4.0 + 17.0)
    + 28.0
    + 2.0 * (17.0 + 6.0 + 36.0)
    + (metrics::BORDER_WIDTH + 12.0 + 34.0);

/// The notification centre under the calendar, in the panel of the clock: its height. The
/// list scrolls inside it.
pub const CENTER_HEIGHT: f32 = 280.0;

// Motion (D-20c): the panels unfold from the bar like a window opens, and fold back the same
// way.

/// Opening and closing of a panel: the spring of a window opening, from 96 % of its size and 8
/// higher, around its top right corner, under the bar's end; the fade of a hover.
pub const OPEN_SPRING: motion::Spring = motion::OPEN_SPRING;
pub const OPEN_SCALE: f32 = 0.96;
pub const OPEN_RISE: f32 = 8.0;
pub const OPEN_FADE: std::time::Duration = motion::FADE;
/// A page that replaces another in a panel (the Wi-Fi networks): it slides in from 24 on its
/// side, on the same spring and fade.
pub const PAGE_SLIDE: f32 = 24.0;
/// The control centre turning into the calendar or back (UI-01): each slides by this much, one
/// out, the other in, while a mask goes from the height of one to the other's. 12, the room
/// between the panels and the edge of the screen: nothing leaves the surface.
pub const SWITCH_SLIDE: f32 = metrics::TOP_BAR_MARGIN_SIDE;
/// A tile switched on or off: its colors mix over a hover's duration.
pub const TILE_FADE: std::time::Duration = motion::FADE;

/// Calendar: one day cell, the radius of the mark of today, gap below the month row and
/// below the weekday row, the agenda slot.
pub const DAY_CELL: f32 = 36.0;
pub const DAY_MARK: f32 = 30.0;
pub const MONTH_ROW: f32 = 32.0;
pub const WEEKDAY_ROW: f32 = 22.0;
pub const CALENDAR_GAP: f32 = 8.0;
pub const AGENDA_HEIGHT: f32 = 56.0;
/// Weeks shown: always six, so that the panel keeps its height from one month to the next.
pub const WEEKS: usize = 6;
pub const CALENDAR_WIDTH: f32 = 2.0 * (PADDING + metrics::BORDER_WIDTH) + 7.0 * DAY_CELL;
pub const CALENDAR_HEIGHT: f32 = 2.0 * (PADDING + metrics::BORDER_WIDTH)
    + MONTH_ROW
    + CALENDAR_GAP
    + WEEKDAY_ROW
    + WEEKS as f32 * DAY_CELL
    + metrics::GAP_SECTION
    + metrics::BORDER_WIDTH
    + AGENDA_HEIGHT;

/// Quick settings: a tile of the grid (height, radius, gap, side padding, glyph, gap between
/// the glyph and its texts), the chevron of the Wi-Fi tile. The padding and the gap are 2 under
/// the board's, so that « Ne pas déranger » fits in Inter as iced lays it out.
pub const TILE_HEIGHT: f32 = 56.0;
pub const QUICK_TILE_RADIUS: f32 = 14.0;
pub const TILE_GAP: f32 = 8.0;
pub const TILE_PADDING: f32 = 10.0;
pub const GLYPH: f32 = 18.0;
/// The glyph of a tile: 2 under the board's 18, for the same room as its title.
pub const TILE_GLYPH: f32 = 16.0;
pub const TILE_ICON_GAP: f32 = 8.0;
pub const CHEVRON: f32 = 28.0;
/// A slider row: the button or icon on its left, the gap, the rail and handle; the room under
/// the volume for the name of the output.
pub const ROW_BUTTON: f32 = 28.0;
pub const SLIDER_GAP: f32 = 10.0;
pub const SLIDER_INSET: f32 = ROW_BUTTON + SLIDER_GAP;
pub const RAIL: f32 = 4.0;
pub const HANDLE: f32 = 8.0;
pub const CAPTION_GAP: f32 = 4.0;
/// A labelled group: gap under its label; the segmented control (its padding, gap, segment
/// height and radius, its own radius).
pub const GROUP_GAP: f32 = 6.0;
pub const SEGMENTED_PADDING: f32 = 3.0;
pub const SEGMENT_GAP: f32 = 2.0;
pub const SEGMENT_HEIGHT: f32 = 30.0;
pub const SEGMENT_RADIUS: f32 = 8.0;
pub const SEGMENTED_RADIUS: f32 = 11.0;
/// The code a Bluetooth device pairs with, in the question of the agent (D-20d).
pub const CODE_SIZE: f32 = typography::TITLE_SMALL_SIZE + 5.0;

/// The user row: the room under the line above it, the round buttons and their gap.
pub const FOOTER_GAP: f32 = 12.0;
pub const ROUND_BUTTON: f32 = 34.0;
pub const ROUND_RADIUS: f32 = 10.0;
pub const ROUND_GAP: f32 = 4.0;
pub const ROUND_GLYPH: f32 = 16.0;

/// The line icons of the quick settings, on the grid and stroke of `icon::Icon`. All from the
/// Panels board but the padlock of a protected network and the three session commands, which
/// are the launcher's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Wifi,
    Bluetooth,
    Moon,
    Gamepad,
    Volume,
    Muted,
    Sun,
    User,
    Lock,
    LogOut,
    Reboot,
    PowerOff,
    Padlock,
    /// The kinds of Bluetooth devices (D-20d).
    Headphones,
    Mouse,
    Keyboard,
    Phone,
    Computer,
}

impl Glyph {
    pub const ALL: [Self; 18] = [
        Self::Wifi,
        Self::Bluetooth,
        Self::Moon,
        Self::Gamepad,
        Self::Volume,
        Self::Muted,
        Self::Sun,
        Self::User,
        Self::Lock,
        Self::LogOut,
        Self::Reboot,
        Self::PowerOff,
        Self::Padlock,
        Self::Headphones,
        Self::Mouse,
        Self::Keyboard,
        Self::Phone,
        Self::Computer,
    ];

    /// Path on the 24 grid.
    pub const fn path(self) -> &'static str {
        use crate::icon::Icon;
        use crate::launcher::Glyph as Launcher;
        match self {
            Self::Wifi => Icon::Network.path(),
            Self::Bluetooth => "M7 7l10 10-5 4V3l5 4L7 17",
            Self::Moon => "M19.5 14.5A8 8 0 0 1 9.5 4.5a8 8 0 1 0 10 10z",
            Self::Gamepad => {
                "M7.5 8h9a4.5 4.5 0 0 1 4.4 5.4l-.6 3a2.2 2.2 0 0 1-3.9.9L15 15.5H9l-1.4 1.8a2.2 2.2 0 0 1-3.9-.9l-.6-3A4.5 4.5 0 0 1 7.5 8zM8 10.5v3M6.5 12h3M15.5 11h.01M17.5 13h.01"
            }
            Self::Volume => Icon::Sound.path(),
            Self::Muted => Icon::SoundMuted.path(),
            Self::Sun => {
                "M12 8a4 4 0 1 0 0 8a4 4 0 0 0 0-8zM12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5L19 19M5 19l1.5-1.5M17.5 6.5L19 5"
            }
            Self::User => "M12 4a4 4 0 1 0 0 8a4 4 0 0 0 0-8zM4 20a8 8 0 0 1 16 0",
            Self::Lock => "M7 11V8a5 5 0 0 1 10 0v3M6 11h12v9H6z",
            Self::LogOut => Launcher::LogOut.path(),
            Self::Reboot => Launcher::Reboot.path(),
            Self::PowerOff => Launcher::PowerOff.path(),
            Self::Padlock => Icon::Lock.path(),
            Self::Headphones => {
                "M4 15v-3a8 8 0 0 1 16 0v3M4 15h3v5H5a1 1 0 0 1-1-1zM20 15h-3v5h2a1 1 0 0 0 1-1z"
            }
            Self::Mouse => "M12 3a6 6 0 0 0-6 6v6a6 6 0 0 0 12 0V9a6 6 0 0 0-6-6zM12 7v3",
            Self::Keyboard => {
                "M3 7h18v10H3zM6.5 10h.01M9.5 10h.01M12.5 10h.01M15.5 10h.01M17.5 10h.01M8 14h8"
            }
            Self::Phone => {
                "M8 3h8a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM11 18h2"
            }
            Self::Computer => "M4 5h16v10H4zM2 19h20",
        }
    }

    /// Full SVG document, stroked in black: the display recolors it.
    pub fn svg(self) -> String {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {grid} {grid}\" fill=\"none\" \
             stroke=\"#000\" stroke-width=\"{stroke}\" stroke-linecap=\"round\" \
             stroke-linejoin=\"round\"><path d=\"{path}\"/></svg>",
            grid = metrics::ICON_GRID,
            stroke = metrics::ICON_STROKE,
            path = self.path(),
        )
    }
}

/// Ghost button around the clock and around each indicator of the bar: its side padding and
/// height. The bar takes the padding back from its gaps. UI-01 makes the hit area wider and
/// taller than D-20's (6 and 28): the whole height of the bar but the border and 2 around.
pub const BAR_BUTTON_PADDING: f32 = 7.0;
pub const BAR_BUTTON_HEIGHT: f32 = 30.0;
/// A bar button pressed shrinks a little, around its centre.
pub const BAR_PRESS_SCALE: f32 = 0.94;

// Colors.

/// The panels' own colors, for one theme; the rest comes from the palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    /// Card of a notification: the board's `rgba(255,255,255,0.07)`.
    pub card: Color,
    /// « Tout effacer » and the tiles that are off: `0.08`.
    pub control: Color,
    /// The line under the title of a tile that is on: the board's `#1E3A2A`.
    pub tile_on_line: Color,
}

impl Colors {
    pub const DARK: Self = Self {
        card: Color::rgba(255, 255, 255, 0.07),
        control: Color::rgba(255, 255, 255, 0.08),
        tile_on_line: Color::rgb(0x1E, 0x3A, 0x2A),
    };

    /// The light board: the same veils, in night blue.
    pub const LIGHT: Self = Self {
        card: Color::rgba(20, 35, 59, 0.07),
        control: Color::rgba(20, 35, 59, 0.08),
        tile_on_line: Color::rgb(0x1E, 0x3A, 0x2A),
    };

    /// The colors that go with `palette`.
    pub fn of(palette: &Palette) -> &'static Self {
        if *palette == Palette::LIGHT {
            &Self::LIGHT
        } else {
            &Self::DARK
        }
    }
}

/// Panel texts, in French.
pub mod text {
    /// Notification centre.
    pub const NOTIFICATIONS: &str = "Notifications";
    pub const CLEAR_ALL: &str = "Tout effacer";
    pub const NO_NOTIFICATION: &str = "Aucune notification";
    pub const CENTER_UNAVAILABLE: &str = "Le service des notifications ne répond pas.";
    /// Notifications of an application beyond those shown in its card.
    pub fn more(count: usize) -> String {
        if count == 1 {
            String::from("1 autre notification")
        } else {
            format!("{count} autres notifications")
        }
    }

    /// Calendar.
    pub const MONTHS: [&str; 12] = [
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
    /// Heads of the columns, Monday first.
    pub const WEEKDAYS: [&str; 7] = ["lu", "ma", "me", "je", "ve", "sa", "di"];
    pub const PREVIOUS_MONTH: &str = "Mois précédent";
    pub const NEXT_MONTH: &str = "Mois suivant";
    pub const AGENDA: &str = "Agenda";
    pub const AGENDA_SOON: &str = "Bientôt : tes rendez-vous du jour ici.";

    /// Quick settings.
    pub const WIFI: &str = "Wi-Fi";
    pub const BLUETOOTH: &str = "Bluetooth";
    pub const DO_NOT_DISTURB: &str = "Ne pas déranger";
    pub const GAME_MODE: &str = "Mode jeu";
    pub const ON: &str = "Activé";
    pub const OFF: &str = "Désactivé";
    pub const LOADING: &str = "…";
    pub const SOON: &str = "Bientôt";
    pub const UNAVAILABLE: &str = "Indisponible";
    pub const REFUSED: &str = "Refusé";
    pub const FAILED: &str = "Échec";
    pub const NOT_CONNECTED: &str = "Non connecté";
    pub const CONNECTING: &str = "Connexion…";
    pub const NO_WIFI_DEVICE: &str = "Aucune carte";
    pub const BLOCKED: &str = "Bloqué";
    pub const NETWORKS: &str = "Réseaux Wi-Fi";
    pub const BACK: &str = "Retour";
    pub const PASSWORD: &str = "Mot de passe";
    pub const JOIN: &str = "Rejoindre";
    pub const WIFI_OFF_LINE: &str = "Le Wi-Fi est coupé.";
    pub const NO_NETWORK: &str = "Aucun réseau trouvé.";
    pub const ENTERPRISE: &str = "Ce réseau se rejoint depuis les Réglages.";
    pub const MUTE: &str = "Couper le son";
    pub const UNMUTE: &str = "Rétablir le son";
    pub const NO_OUTPUT: &str = "Aucune sortie son";
    pub const GPU: &str = "Carte graphique";
    pub const GPU_CHOICES: [&str; 3] = ["Auto", "AMD", "NVIDIA"];
    pub const ENERGY: &str = "Énergie";
    pub const PROFILES: [&str; 3] = ["Économie", "Équilibré", "Performance"];
    pub const ENERGY_ABSENT: &str = "Service absent";
    pub const LOCK: &str = "Verrouiller";
    pub const LOG_OUT: &str = "Se déconnecter";
    pub const REBOOT: &str = "Redémarrer";
    pub const POWER_OFF: &str = "Éteindre";
    pub const CONFIRM: &str = "Reclique pour confirmer";

    /// What the battery is doing, for `battery`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Charging {
        /// On its own, `Some` minutes left.
        No(Option<u32>),
        /// Charging, full in `Some` minutes.
        Yes(Option<u32>),
        /// Plugged in, full or not charging.
        Plugged,
    }

    /// The battery next to « Énergie »: « 82 % · 3 h 12 restantes », « 54 % · pleine dans
    /// 45 min », « 100 % · branchée ».
    pub fn battery(percent: u8, charging: Charging) -> String {
        let value = crate::strings::percent(percent);
        match charging {
            Charging::No(Some(left)) => format!("{value} · {} restantes", duration(left)),
            Charging::Yes(Some(left)) => format!("{value} · pleine dans {}", duration(left)),
            Charging::Yes(None) => format!("{value} · en charge"),
            Charging::Plugged => format!("{value} · branchée"),
            Charging::No(None) => value,
        }
    }

    /// « 45 min », « 3 h 12 », « 2 h ».
    fn duration(minutes: u32) -> String {
        match (minutes / 60, minutes % 60) {
            (0, m) => format!("{m} min"),
            (h, 0) => format!("{h} h"),
            (h, m) => format!("{h} h {m:02}"),
        }
    }

    // The Bluetooth devices (D-20d).
    pub const BLUETOOTH_DEVICES: &str = "Appareils Bluetooth";
    pub const MY_DEVICES: &str = "Mes appareils";
    pub const OTHER_DEVICES: &str = "Autres appareils";
    pub const SEARCHING: &str = "Recherche…";
    pub const NO_DEVICE: &str = "Aucun appareil à proximité.";
    pub const BLUETOOTH_OFF_LINE: &str = "Le Bluetooth est coupé.";
    pub const CONNECTED: &str = "Connecté";
    pub const PAIRING: &str = "Appairage…";
    pub const CONFIRM_BUTTON: &str = "Confirmer";
    pub const CANCEL: &str = "Annuler";
    pub const FORGET: &str = "Oublier";

    /// The battery of a device, after its state: « Connecté · 80 % ».
    pub fn device_line(connected: bool, battery: Option<u8>) -> String {
        let state = if connected { CONNECTED } else { "" };
        match battery {
            Some(level) if connected => format!("{state} · {}", crate::strings::percent(level)),
            Some(level) => crate::strings::percent(level),
            None => state.to_owned(),
        }
    }

    /// What the panel says while a device pairs, and the code it shows.
    pub fn confirm_code(name: &str) -> String {
        format!("{name} affiche-t-il ce code ?")
    }

    pub fn type_code(name: &str) -> String {
        format!("Tape ce code sur {name}, puis Entrée.")
    }

    pub fn forget_question(name: &str) -> String {
        format!("Oublier {name} ? Il faudra l’appairer de nouveau.")
    }

    /// Bluetooth devices connected.
    pub fn devices(count: usize) -> String {
        if count == 1 {
            String::from("1 appareil")
        } else {
            format!("{count} appareils")
        }
    }

    /// Title of the month shown, such as « novembre 2026 ». `month` goes from 1 to 12.
    pub fn month_title(month: u8, year: i32) -> String {
        let name = usize::from(month)
            .checked_sub(1)
            .and_then(|i| MONTHS.get(i))
            .copied()
            .unwrap_or_default();
        format!("{name} {year}")
    }
}

#[cfg(feature = "iced")]
pub use styles::*;

#[cfg(feature = "iced")]
mod styles {
    use iced::widget::{button, container, text};
    use iced::{Background, Element, Length, Theme, border};

    use super::*;
    use crate::component::Control;
    use crate::ellipsis::ellipsis;
    use crate::theme;

    fn fill(c: Color) -> Option<Background> {
        Some(Background::Color(c.into()))
    }

    /// A panel: the glass and border of the board, radius 20. Its shadow is drawn by
    /// `surface::Shadowed`.
    pub fn panel(theme: &Theme) -> container::Style {
        let p = theme::palette(theme);
        container::Style {
            background: fill(p.glass),
            border: border::width(metrics::BORDER_WIDTH)
                .color(p.border)
                .rounded(RADIUS),
            text_color: Some(p.text_primary.into()),
            ..container::Style::default()
        }
    }

    /// Card of a group of notifications.
    pub fn card(theme: &Theme) -> container::Style {
        let p = theme::palette(theme);
        container::Style {
            background: fill(Colors::of(p).card),
            border: border::rounded(CARD_RADIUS),
            text_color: Some(p.text_primary.into()),
            ..container::Style::default()
        }
    }

    /// Tile of an application in a card, with its initial: the bubbles' tile.
    pub fn tile(theme: &Theme) -> container::Style {
        crate::notifications::tile(theme)
    }

    /// Secondary text: application name, time, body.
    pub fn secondary_text(theme: &Theme) -> text::Style {
        text::Style {
            color: Some(theme::palette(theme).text_secondary.into()),
        }
    }

    /// « Tout effacer »: a veiled compact button with secondary text.
    fn clear_style(theme: &Theme, status: button::Status) -> button::Style {
        let p = theme::palette(theme);
        let veil = Colors::of(p).control;
        let (background, text) = match status {
            button::Status::Active => (veil, p.text_secondary),
            button::Status::Hovered => (p.chip_hover, p.text_primary),
            button::Status::Pressed => (p.pressed, p.text_primary),
            button::Status::Disabled => (p.disabled, p.text_tertiary),
        };
        button::Style {
            background: fill(background),
            text_color: text.into(),
            border: border::rounded(CLEAR_RADIUS),
            ..button::Style::default()
        }
    }

    /// The « Tout effacer » button of the notification centre.
    pub fn clear_button<'a, Message: Clone + 'a>(
        label: &'a str,
        on_press: Option<Message>,
    ) -> Control<'a, Message> {
        let label = text(label)
            .font(typography::UI)
            .size(typography::SMALL_SIZE);
        let content = container(label).center_y(Length::Fill);
        let mut control = Control::new(content, clear_style, on_press, CLEAR_RADIUS);
        control.button = control
            .button
            .height(CLEAR_HEIGHT)
            .padding([0.0, CLEAR_PADDING]);
        control
    }

    /// An action of an open notification: the first one primary, the others secondary.
    pub fn action_button<'a, Message: Clone + 'a>(
        label: &'a str,
        primary: bool,
        on_press: Option<Message>,
    ) -> Control<'a, Message> {
        let font = if primary {
            typography::UI_SEMIBOLD
        } else {
            typography::UI
        };
        let content = container(crate::component::body(label).font(font)).center_y(Length::Fill);
        let style = crate::notifications::action(primary);
        let mut control = Control::new(content, style, on_press, ACTION_RADIUS);
        control.button = control
            .button
            .height(ACTION_HEIGHT)
            .padding([0.0, ACTION_PADDING]);
        control
    }

    /// Ghost button of the top bar around the clock or an indicator: a pill fades in on
    /// hover and deepens on press, the content shrinks a little while pressed, and while its
    /// panel is open it keeps a veil and a green mark under it, which fade in and out.
    pub fn bar_button<'a, Message: Clone + 'a>(
        content: impl Into<Element<'a, Message>>,
        open: bool,
        on_press: Option<Message>,
    ) -> Control<'a, Message> {
        let content = container(content).center_y(Length::Fill);
        let mut control = Control::new(
            content,
            theme::sherlock_button(false),
            on_press,
            metrics::SHERLOCK_BUTTON_RADIUS,
        );
        control.button = control
            .button
            .height(BAR_BUTTON_HEIGHT)
            .padding([0.0, BAR_BUTTON_PADDING]);
        control.open = Some(open);
        control.press_scale = BAR_PRESS_SCALE;
        control
    }

    /// Mark of today in the calendar: a green disc, dark text.
    pub fn today(theme: &Theme) -> container::Style {
        let p = theme::palette(theme);
        container::Style {
            background: fill(p.accent),
            border: border::rounded(DAY_MARK / 2.0),
            text_color: Some(p.text_on_accent.into()),
            ..container::Style::default()
        }
    }

    /// A day of the month shown, or of a month next to it (faded).
    pub fn day_text(in_month: bool) -> impl Fn(&Theme) -> text::Style {
        move |theme| {
            let p = theme::palette(theme);
            let c = if in_month {
                p.text_primary
            } else {
                p.text_tertiary
            };
            text::Style {
                color: Some(c.into()),
            }
        }
    }

    /// Heads of the weekday columns.
    pub fn weekday_text(theme: &Theme) -> text::Style {
        text::Style {
            color: Some(theme::palette(theme).text_tertiary.into()),
        }
    }

    /// The SVG of a glyph, built once.
    fn handle(glyph: Glyph) -> iced::widget::svg::Handle {
        static HANDLES: std::sync::OnceLock<Vec<iced::widget::svg::Handle>> =
            std::sync::OnceLock::new();
        let all = HANDLES.get_or_init(|| {
            Glyph::ALL
                .iter()
                .map(|g| iced::widget::svg::Handle::from_memory(g.svg().into_bytes()))
                .collect()
        });
        Glyph::ALL
            .iter()
            .position(|g| *g == glyph)
            .and_then(|i| all.get(i))
            .cloned()
            .unwrap_or_else(|| iced::widget::svg::Handle::from_memory(glyph.svg().into_bytes()))
    }

    /// A glyph of the quick settings, in `tint`, at `size`.
    fn glyph_sized<'a>(glyph: Glyph, tint: Color, size: f32) -> iced::widget::Svg<'a> {
        iced::widget::svg(handle(glyph))
            .width(size)
            .height(size)
            .style(move |_, _| iced::widget::svg::Style {
                color: Some(tint.into()),
            })
    }

    /// A glyph of the quick settings, 18 px, in `tint`.
    pub fn glyph<'a>(glyph: Glyph, tint: Color) -> iced::widget::Svg<'a> {
        glyph_sized(glyph, tint, GLYPH)
    }

    /// Background of a tile: green when on (`lit` 1), veiled when off, fainter when it can do
    /// nothing.
    fn tile_back(lit: f32, enabled: bool) -> impl Fn(&Theme) -> container::Style {
        move |theme| {
            let p = theme::palette(theme);
            let back = if enabled {
                Colors::of(p).control.lerp(p.accent, lit)
            } else {
                p.disabled
            };
            container::Style {
                background: fill(back),
                border: border::rounded(QUICK_TILE_RADIUS),
                ..container::Style::default()
            }
        }
    }

    /// The press area of a tile, over its background: a veil on hover and press.
    fn tile_press(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
        move |theme, status| {
            let p = theme::palette(theme);
            let veil = match (on, status) {
                (true, button::Status::Hovered) => Some(p.accent_hover),
                (true, button::Status::Pressed) => Some(p.accent_pressed),
                (false, button::Status::Hovered) => Some(p.hover),
                (false, button::Status::Pressed) => Some(p.pressed),
                _ => None,
            };
            button::Style {
                background: veil.and_then(fill),
                border: border::rounded(QUICK_TILE_RADIUS),
                ..button::Style::default()
            }
        }
    }

    /// A tile of the grid: its glyph, its title and the line under it. `on_press` switches it;
    /// without it the tile is faded and does nothing. `more`: a chevron on its right, with its
    /// label, which opens a page of its own.
    ///
    /// `lit` is how much it is on, from 0 to 1: a tile switched goes from one to the other
    /// over `TILE_FADE`, its colors mixed.
    pub fn quick_tile<'a, Message: Clone + 'a>(
        glyph_of: Glyph,
        title: &'a str,
        line: String,
        lit: f32,
        on_press: Option<Message>,
        more: Option<(&'a str, Message)>,
    ) -> Element<'a, Message> {
        let p = crate::color::palette();
        let enabled = on_press.is_some();
        let lit = if enabled { lit.clamp(0.0, 1.0) } else { 0.0 };
        let on = lit >= 0.5;
        let (ink, under) = if enabled {
            (
                p.text_primary.lerp(p.text_on_accent, lit),
                p.text_secondary.lerp(Colors::of(p).tile_on_line, lit),
            )
        } else {
            (p.text_tertiary, p.text_tertiary)
        };
        // A long network name ends with « … » rather than being cut.
        let texts = iced::widget::column![
            ellipsis(title)
                .font(typography::UI_SEMIBOLD)
                .color(ink)
                .width(Length::Fill),
            ellipsis(line)
                .size(typography::SMALL_SIZE)
                .color(under)
                .width(Length::Fill),
        ];
        let icon = glyph_sized(glyph_of, ink, TILE_GLYPH);
        let content = iced::widget::row![icon, texts]
            .spacing(TILE_ICON_GAP)
            .align_y(iced::Alignment::Center)
            .height(Length::Fill);
        let mut main = Control::new(content, tile_press(on), on_press, QUICK_TILE_RADIUS);
        main.button = main
            .button
            .width(Length::Fill)
            .height(TILE_HEIGHT)
            .padding([0.0, TILE_PADDING]);
        let mut line = iced::widget::Row::new().push(main);
        if let Some((label, message)) = more {
            let chevron =
                crate::component::icon(crate::icon::Icon::Chevron, metrics::ICON_SIZE, ink);
            let chevron = container(chevron).center(Length::Fill);
            let mut open = Control::new(chevron, tile_press(on), Some(message), QUICK_TILE_RADIUS);
            open.button = open.button.width(CHEVRON).height(TILE_HEIGHT).padding(0);
            open.label = Some(label);
            line = line.push(open);
        }
        container(line)
            .width(Length::Fill)
            .height(TILE_HEIGHT)
            .style(tile_back(lit, enabled))
            .into()
    }

    /// A segmented control of the board: see `component::segmented`.
    pub fn segmented<'a, Message: Clone + 'a>(
        choices: Vec<(&'a str, bool, Option<Message>)>,
    ) -> Element<'a, Message> {
        crate::component::segmented(choices)
    }

    /// A row of the board: its button or icon, then a slider from 0 to 100. Without
    /// `on_change`, a faded rail that does nothing.
    pub fn slider_row<'a, Message: Clone + 'a>(
        leading: impl Into<Element<'a, Message>>,
        value: u8,
        on_change: Option<fn(u8) -> Message>,
    ) -> Element<'a, Message> {
        let control = crate::component::slider(value, on_change);
        iced::widget::row![leading.into(), control]
            .spacing(SLIDER_GAP)
            .align_y(iced::Alignment::Center)
            .height(ROW_BUTTON)
            .into()
    }

    /// A ghost button of 28 holding a glyph: the mute switch. `label` names it.
    pub fn glyph_button<'a, Message: Clone + 'a>(
        glyph_of: Glyph,
        label: &'a str,
        on_press: Option<Message>,
    ) -> Control<'a, Message> {
        let p = crate::color::palette();
        let tint = if on_press.is_some() {
            p.text_secondary
        } else {
            p.text_tertiary
        };
        let content = container(glyph(glyph_of, tint)).center(Length::Fill);
        let radius = metrics::ICON_BUTTON_RADIUS;
        let mut control = Control::new(content, theme::icon_button, on_press, radius);
        control.button = control
            .button
            .width(ROW_BUTTON)
            .height(ROW_BUTTON)
            .padding(0);
        control.label = Some(label);
        control
    }

    /// A round button of the user row: veiled; amber while its session command waits for a
    /// second click.
    fn round_style(armed: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
        move |theme, status| {
            let p = theme::palette(theme);
            let back = match (armed, status) {
                (true, _) => p.amber_soft,
                (false, button::Status::Hovered) => p.chip_hover,
                (false, button::Status::Pressed) => p.pressed,
                (false, _) => Colors::of(p).control,
            };
            button::Style {
                background: fill(back),
                border: border::rounded(ROUND_RADIUS),
                ..button::Style::default()
            }
        }
    }

    /// A Bluetooth device of the list (D-20d), as a menu entry: its glyph and its name, its
    /// state on the right (« Connecté · 80 % »), then `more` (« Oublier »). `on_press`
    /// connects, disconnects or pairs it.
    pub fn device_row<'a, Message: Clone + 'a>(
        glyph_of: Glyph,
        name: &'a str,
        line: String,
        on_press: Option<Message>,
        more: Option<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let p = crate::color::palette();
        let start = iced::widget::row![
            glyph(glyph_of, p.text_secondary),
            ellipsis(name).width(Length::Fill),
        ]
        .spacing(metrics::GAP_ICON)
        .align_y(iced::Alignment::Center);
        let end = ellipsis(line)
            .size(typography::SMALL_SIZE)
            .color(p.text_tertiary);
        let content = iced::widget::row![container(start).width(Length::Fill), end]
            .spacing(metrics::GAP_INLINE)
            .align_y(iced::Alignment::Center)
            .height(Length::Fill);
        let mut entry = Control::new(
            content,
            theme::menu_item,
            on_press,
            metrics::MENU_ITEM_RADIUS,
        );
        entry.button = entry
            .button
            .width(Length::Fill)
            .height(metrics::MENU_ITEM_HEIGHT)
            .padding([0.0, metrics::MENU_ITEM_PADDING]);
        let mut line = iced::widget::Row::new()
            .push(entry)
            .spacing(metrics::GAP_TIGHT)
            .align_y(iced::Alignment::Center);
        if let Some(more) = more {
            line = line.push(more);
        }
        line.into()
    }

    /// A button of the user row (« Verrouiller », « Éteindre »…). `label` names it.
    pub fn round_button<'a, Message: Clone + 'a>(
        glyph_of: Glyph,
        label: &'a str,
        armed: bool,
        on_press: Option<Message>,
    ) -> Control<'a, Message> {
        let p = crate::color::palette();
        let tint = if armed { p.amber_text } else { p.text_primary };
        let content = container(glyph_sized(glyph_of, tint, ROUND_GLYPH)).center(Length::Fill);
        let mut control = Control::new(content, round_style(armed), on_press, ROUND_RADIUS);
        control.button = control
            .button
            .width(ROUND_BUTTON)
            .height(ROUND_BUTTON)
            .padding(0);
        control.label = Some(label);
        control
    }

    /// Error line of a panel: the salmon as text.
    pub fn error_text(theme: &Theme) -> text::Style {
        text::Style {
            color: Some(theme::palette(theme).salmon_text.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panels_sit_where_the_board_puts_them() {
        let bar = metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT;
        assert_eq!(bar + BELOW_BAR, 54.0);
        // The quick settings at 1092, 12 from the right edge.
        let right = metrics::REFERENCE_SCREEN.0 - metrics::TOP_BAR_MARGIN_SIDE;
        assert_eq!(right - QUICK_WIDTH, 1092.0);
        // The control centre, and the calendar over the notifications, stay clear of the dock
        // on the reference screen.
        let bottom = bar + BELOW_BAR + CONTROL_HEIGHT;
        assert!(bottom < metrics::REFERENCE_SCREEN.1 - 150.0, "{bottom}");
        let clock = bar + BELOW_BAR + CALENDAR_HEIGHT + PANEL_GAP + CENTER_HEIGHT;
        assert!(clock < metrics::REFERENCE_SCREEN.1 - 150.0, "{clock}");
        const { assert!(CALENDAR_WIDTH <= CENTER_WIDTH) };
        const { assert!(CALENDAR_HEIGHT < CONTROL_HEIGHT) };
    }

    #[test]
    fn texts_read_on_the_panels_in_both_themes() {
        for palette in [&Palette::DARK, &Palette::LIGHT] {
            let colors = Colors::of(palette);
            for below in palette.desktop_discs {
                let glass = palette.glass.over(below);
                let card = colors.card.over(glass);
                for back in [glass, card, colors.control.over(glass)] {
                    for text in [palette.text_primary, palette.text_secondary] {
                        assert!(text.contrast(back) >= 4.5, "{text:?} on {back:?}");
                    }
                    assert!(palette.salmon_text.contrast(back) >= 4.5);
                }
                assert!(palette.text_on_accent.contrast(palette.accent) >= 4.5);
            }
        }
    }

    #[test]
    fn the_line_of_a_tile_that_is_on_reads_on_the_green() {
        for palette in [&Palette::DARK, &Palette::LIGHT] {
            let line = Colors::of(palette).tile_on_line;
            assert!(line.contrast(palette.accent) >= 4.5, "{line:?}");
        }
    }

    #[test]
    fn every_glyph_stays_in_the_grid() {
        for glyph in Glyph::ALL {
            let numbers = glyph
                .path()
                .split(|c: char| c.is_ascii_alphabetic() || c == ' ' || c == ',')
                .flat_map(|chunk| chunk.split_inclusive('-'))
                .filter_map(|n| n.trim_end_matches('-').parse::<f32>().ok());
            for n in numbers {
                assert!(n.abs() <= metrics::ICON_GRID, "{glyph:?}: {n}");
            }
        }
    }

    #[test]
    fn the_month_title_is_in_french() {
        assert_eq!(text::month_title(11, 2026), "novembre 2026");
        assert_eq!(text::month_title(2, 2028), "février 2028");
        assert_eq!(text::month_title(13, 2028), " 2028");
        assert_eq!(text::more(1), "1 autre notification");
        assert_eq!(text::more(4), "4 autres notifications");
        assert_eq!(text::devices(1), "1 appareil");
        assert_eq!(text::devices(2), "2 appareils");
    }

    #[test]
    fn the_battery_says_its_level_and_its_time() {
        use text::Charging;
        assert_eq!(
            text::battery(82, Charging::No(Some(192))),
            "82\u{202F}% · 3 h 12 restantes"
        );
        assert_eq!(
            text::battery(54, Charging::Yes(Some(45))),
            "54\u{202F}% · pleine dans 45 min"
        );
        assert!(text::battery(54, Charging::Yes(Some(120))).ends_with("dans 2 h"));
        assert_eq!(
            text::battery(54, Charging::Yes(None)),
            "54\u{202F}% · en charge"
        );
        assert_eq!(
            text::battery(100, Charging::Plugged),
            "100\u{202F}% · branchée"
        );
        assert_eq!(text::battery(7, Charging::No(None)), "7\u{202F}%");
    }
}
