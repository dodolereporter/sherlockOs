//! Radii, spacings and sizes, in logical pixels. Values from `docs/contexte.md` and from the
//! Menus and « Réglages » design boards.

/// Reference screen of the design boards.
pub const REFERENCE_SCREEN: (f32, f32) = (1440.0, 900.0);

/// The three discs of the wallpaper, on the reference screen: left, top and diameter of each,
/// in the order of `color::DESKTOP_DISCS` (Main design board). The last one is in front.
pub const DESKTOP_DISC_GEOMETRY: [(f32, f32, f32); 3] = [
    (-260.0, 340.0, 1100.0),
    (860.0, -420.0, 900.0),
    (980.0, 520.0, 520.0),
];

// Top bar.

/// Height of the top bar.
pub const TOP_BAR_HEIGHT: f32 = 36.0;
/// Side margins of the top bar.
pub const TOP_BAR_MARGIN_SIDE: f32 = 12.0;
/// Top margin of the top bar.
pub const TOP_BAR_MARGIN_TOP: f32 = 10.0;
/// Radius of the top bar.
pub const TOP_BAR_RADIUS: f32 = 12.0;
/// Inner padding of the top bar: left, right.
pub const TOP_BAR_PADDING: (f32, f32) = (6.0, 10.0);
/// Gap between the Sherlock button and the application name.
pub const TOP_BAR_LEFT_GAP: f32 = 4.0;
/// Sherlock button: width, height, radius, size of the logo symbol in it.
pub const SHERLOCK_BUTTON: (f32, f32) = (30.0, 28.0);
pub const SHERLOCK_BUTTON_RADIUS: f32 = 8.0;
pub const SHERLOCK_GLYPH: f32 = 22.0;
/// Active application name: left and right padding.
pub const APP_NAME_PADDING: (f32, f32) = (8.0, 10.0);
/// Workspace chip: height, side padding, radius, gap between two chips.
pub const CHIP_HEIGHT: f32 = 24.0;
pub const CHIP_PADDING: f32 = 12.0;
pub const CHIP_RADIUS: f32 = 12.0;
pub const CHIP_GAP: f32 = 4.0;
/// Right-hand indicators: gap between two indicators, between an icon and its value.
pub const STATUS_GAP: f32 = 14.0;
pub const STATUS_ICON_GAP: f32 = 6.0;
/// The icons of the bar's indicators (network, sound, battery). The board draws them at 16;
/// UI-01 enlarges them, found too small on the reference laptop.
pub const STATUS_ICON: f32 = 18.0;
/// Gap between the bottom of the bar and a menu it opens (Menus board: 46 then 52).
pub const TOP_BAR_MENU_GAP: f32 = 6.0;
/// Sherlock menu header: side of the avatar (the logo in its square), top, side and bottom
/// padding.
pub const MENU_AVATAR: f32 = 40.0;
pub const MENU_HEADER_PADDING: (f32, f32, f32) = (6.0, 8.0, 8.0);

// Windows and panels.

/// Radius of a window.
pub const WINDOW_RADIUS: f32 = 16.0;
/// Height of the title bar.
pub const TITLE_BAR_HEIGHT: f32 = 40.0;
/// Window button (minimize, maximize, close): width and height.
pub const WINDOW_BUTTON: (f32, f32) = (28.0, 24.0);
/// Width of the borders of windows, panels, menus and cards.
pub const BORDER_WIDTH: f32 = 1.0;
/// Shadow of a window or a panel: vertical offset and blur.
pub const SHADOW_OFFSET: f32 = 28.0;
pub const SHADOW_BLUR: f32 = 70.0;
/// Radius of a floating panel.
pub const PANEL_RADIUS: f32 = 20.0;
/// Inner padding of a floating panel.
pub const PANEL_PADDING: f32 = 14.0;

// Application windows (Evidence mock-up).

/// Application window of the Evidence mock-up, on the reference screen.
pub const REFERENCE_WINDOW: (f32, f32) = (1300.0, 676.0);

/// Title bar: left and right padding, application badge and its radius.
pub const TITLE_BAR_PADDING: (f32, f32) = (14.0, 8.0);
pub const APP_BADGE: f32 = 18.0;
pub const APP_BADGE_RADIUS: f32 = 5.0;
/// Pill behind the window buttons: padding, radius; radius of a button.
pub const WINDOW_CONTROLS_PADDING: f32 = 2.0;
pub const WINDOW_CONTROLS_RADIUS: f32 = 9.0;
pub const WINDOW_BUTTON_RADIUS: f32 = 7.0;
/// Toolbar under the title bar: height, side padding.
pub const TOOLBAR_HEIGHT: f32 = 48.0;
pub const TOOLBAR_PADDING: f32 = 12.0;
/// Breadcrumb: height, radius, padding, gap; segment: padding and radius.
pub const BREADCRUMB_HEIGHT: f32 = 32.0;
pub const BREADCRUMB_RADIUS: f32 = 8.0;
pub const BREADCRUMB_PADDING: f32 = 6.0;
pub const BREADCRUMB_GAP: f32 = 4.0;
/// Maximum breadcrumb segments shown: beyond that, the start folds up.
pub const BREADCRUMB_MAX: usize = 5;
pub const CRUMB_PADDING: (f32, f32) = (4.0, 8.0);
pub const CRUMB_RADIUS: f32 = 6.0;
/// Git branch badge: padding and radius.
pub const BRANCH_CHIP_PADDING: (f32, f32) = (3.0, 10.0);
pub const BRANCH_CHIP_RADIUS: f32 = 10.0;
/// Search field of a toolbar.
pub const SEARCH_WIDTH: f32 = 210.0;
/// Gap between the magnifier and the text of the search field.
pub const SEARCH_ICON_GAP: f32 = 8.0;
/// Status bar at the bottom of a window: height, side padding.
pub const STATUS_BAR_HEIGHT: f32 = 30.0;
pub const STATUS_BAR_PADDING: f32 = 14.0;
/// Navigation sidebar: width, top, side and bottom padding.
pub const SIDEBAR_WIDTH: f32 = 210.0;
pub const SIDEBAR_PADDING: (f32, f32, f32) = (4.0, 8.0, 12.0);
/// Section label of a navigation: top, side and bottom padding.
pub const NAV_SECTION_PADDING: (f32, f32, f32) = (12.0, 10.0, 6.0);
/// Navigation entry: height, radius, side padding.
pub const NAV_ITEM_HEIGHT: f32 = 32.0;
pub const NAV_ITEM_RADIUS: f32 = 8.0;
pub const NAV_ITEM_PADDING: f32 = 10.0;
/// Preview panel: width, padding, gap between its blocks; file tile and its icon.
pub const PREVIEW_WIDTH: f32 = 300.0;
pub const PREVIEW_PADDING: f32 = 16.0;
pub const PREVIEW_GAP: f32 = 12.0;
pub const PREVIEW_TILE: f32 = 44.0;
pub const PREVIEW_TILE_RADIUS: f32 = 11.0;
pub const PREVIEW_TILE_ICON: f32 = 22.0;
/// Text preview: maximum lines and columns shown. Beyond that, the panel cuts.
pub const PREVIEW_LINES: usize = 18;
/// The preview panel holds 33 characters of JetBrains Mono at 12 px (0.6 em each) inside
/// its padding and the code block's.
pub const PREVIEW_COLUMNS: usize = 33;
/// Largest side of a decoded preview image, in pixels: twice the panel's inner width, for
/// screens at scale 2.
pub const PREVIEW_IMAGE_PIXELS: u32 = 536;
/// Gap between two stacked buttons.
pub const BUTTON_STACK_GAP: f32 = 8.0;
/// Code snippet: vertical and horizontal padding, radius.
pub const CODE_BLOCK_PADDING: (f32, f32) = (10.0, 12.0);
pub const CODE_BLOCK_RADIUS: f32 = 10.0;

// Lists.

/// Side padding of the list, and gap above the first row.
pub const LIST_PADDING: f32 = 8.0;
pub const LIST_TOP_GAP: f32 = 4.0;
/// Column header and row: heights, radius of a row, side padding.
pub const LIST_HEADER_HEIGHT: f32 = 34.0;
pub const LIST_ROW_HEIGHT: f32 = 40.0;
pub const LIST_ROW_RADIUS: f32 = 9.0;
pub const LIST_ROW_PADDING: f32 = 12.0;
/// Gap between two columns, width of the columns after the name (date, size, kind, status).
pub const LIST_COLUMN_GAP: f32 = 12.0;
pub const LIST_COLUMNS: [f32; 4] = [150.0, 96.0, 120.0, 84.0];
/// File kind icon in a row.
pub const LIST_ICON: f32 = 18.0;
/// Extra rows drawn above and below the visible part of a long list.
pub const LIST_OVERSCAN: usize = 4;
/// Slide of the content of a folder that opens: it rises by this distance while fading in.
pub const LIST_SLIDE: f32 = 8.0;
/// Status badge: vertical and horizontal padding, radius.
pub const BADGE_PADDING: (f32, f32) = (1.0, 8.0);
pub const BADGE_RADIUS: f32 = 8.0;
/// Right-click menu of a list and its submenu (Evidence mock-up).
pub const CONTEXT_MENU_WIDTH: f32 = 236.0;
pub const CONTEXT_SUBMENU_WIDTH: f32 = 216.0;
/// « Ouvrir avec » submenu: wider than the design board's, for long application names and the
/// "always" switch; at most this many applications.
pub const OPEN_WITH_WIDTH: f32 = 280.0;
pub const OPEN_WITH_MAX: usize = 8;
/// Height of the list of the application chooser.
pub const CHOOSER_LIST_HEIGHT: f32 = 300.0;
/// Question asked inside a window: width.
pub const DIALOG_WIDTH: f32 = 380.0;

// Menus.

/// Radius of a menu.
pub const MENU_RADIUS: f32 = 12.0;
/// Inner padding of a menu.
pub const MENU_PADDING: f32 = 6.0;
/// Width of a menu and of a submenu.
pub const MENU_WIDTH: f32 = 256.0;
pub const SUBMENU_WIDTH: f32 = 190.0;
/// Gap between a menu and its submenu opened on the right (Menus board: 674 then 678).
pub const SUBMENU_GAP: f32 = 4.0;
/// Shadow of a menu: vertical offset and blur.
pub const MENU_SHADOW_OFFSET: f32 = 22.0;
pub const MENU_SHADOW_BLUR: f32 = 54.0;
/// Crisp shadow of a menu: steps of (vertical offset, spread, opacity), drawn as plain
/// rounded rectangles (see `surface`). They suggest the design board's `0 22px 54px
/// rgba(3,8,18,0.6)` without a blur, which the software renderer draws wrongly.
pub const MENU_SHADOW_LAYERS: [(f32, f32, f32); 3] =
    [(2.0, 1.0, 0.30), (8.0, 3.0, 0.16), (16.0, 6.0, 0.08)];
/// Crisp shadow of a floating panel or dialog, after `0 28px 70px rgba(3,8,18,0.55)`.
pub const PANEL_SHADOW_LAYERS: [(f32, f32, f32); 3] =
    [(3.0, 1.0, 0.28), (10.0, 4.0, 0.15), (20.0, 8.0, 0.07)];
/// Menu entry: height, radius, side padding.
pub const MENU_ITEM_HEIGHT: f32 = 30.0;
pub const MENU_ITEM_RADIUS: f32 = 7.0;
pub const MENU_ITEM_PADDING: f32 = 10.0;
/// Menu separator: top and side margins.
pub const MENU_SEPARATOR_MARGIN: (f32, f32) = (4.0, 8.0);

// Controls.

/// Button: height, radius, side padding. The range goes from 28 to 36 high.
pub const BUTTON_HEIGHT: f32 = 34.0;
pub const BUTTON_RADIUS: f32 = 9.0;
pub const BUTTON_PADDING: f32 = 14.0;
/// Compact button, in a panel or a notification.
pub const BUTTON_COMPACT_HEIGHT: f32 = 28.0;
pub const BUTTON_COMPACT_RADIUS: f32 = 8.0;
/// Icon-only button: side and radius.
pub const ICON_BUTTON: f32 = 32.0;
pub const ICON_BUTTON_RADIUS: f32 = 8.0;
/// Toggle: width, height, inset of the round knob.
pub const TOGGLE_WIDTH: f32 = 44.0;
pub const TOGGLE_HEIGHT: f32 = 26.0;
pub const TOGGLE_INSET: f32 = 3.0;
/// How much wider the knob grows while the toggle is pressed (UI-01).
pub const TOGGLE_KNOB_STRETCH: f32 = 4.0;
/// The mark under a bar button whose panel is open (UI-01): width and height, and its distance
/// to the bottom of the button.
pub const OPEN_MARK: (f32, f32) = (14.0, 2.0);
pub const OPEN_MARK_INSET: f32 = 2.0;
/// Check box and radio button (UI-01, no board draws them): side of the box, radius of the
/// box, width of its edge, diameter of the radio's dot.
pub const CHECKBOX: f32 = 18.0;
pub const CHECKBOX_RADIUS: f32 = 5.0;
pub const CHECKBOX_EDGE: f32 = 1.5;
pub const RADIO_DOT: f32 = 8.0;
/// A check box or radio button with its label: gap, height, padding and radius of the row.
pub const CHOICE_GAP: f32 = 10.0;
pub const CHOICE_HEIGHT: f32 = 30.0;
pub const CHOICE_PADDING: f32 = 6.0;
pub const CHOICE_RADIUS: f32 = 8.0;
/// Progress bar: the Demarrage board's 5 high, its track the pressed veil (`rgba(255,255,255,0.12)`).
pub const PROGRESS_HEIGHT: f32 = 5.0;
/// Loading indicator: its usual size, and the stroke of its arc (not an icon: bolder, to read
/// as moving).
pub const SPINNER: f32 = 20.0;
pub const SPINNER_STROKE: f32 = 2.4;
/// Entry of a list (UI-01): height with a second line, padding.
pub const LIST_ITEM_HEIGHT: f32 = 44.0;

/// Text field: height, radius, side padding.
pub const FIELD_HEIGHT: f32 = 32.0;
pub const FIELD_RADIUS: f32 = 9.0;
pub const FIELD_PADDING: f32 = 10.0;
/// Card: radius, vertical and horizontal padding.
pub const CARD_RADIUS: f32 = 14.0;
pub const CARD_PADDING: (f32, f32) = (16.0, 18.0);
/// Minimum height of a setting row in a card.
pub const SETTING_ROW_HEIGHT: f32 = 48.0;
/// Focus ring: width and gap to the element.
pub const FOCUS_RING_WIDTH: f32 = 2.0;
pub const FOCUS_RING_GAP: f32 = 2.0;

// Icons.

/// Drawing grid of the icons.
pub const ICON_GRID: f32 = 24.0;
/// Stroke width of the icons, taken in the 1.6 to 1.7 range.
pub const ICON_STROKE: f32 = 1.7;
/// Usual display size of an icon, and that of the menu chevrons.
pub const ICON_SIZE: f32 = 16.0;
pub const ICON_SIZE_SMALL: f32 = 14.0;
/// Breadcrumb chevrons and sort arrows.
pub const ICON_SIZE_TINY: f32 = 12.0;

// Dock.

/// Height and radius of the dock bar.
pub const DOCK_HEIGHT: f32 = 76.0;
pub const DOCK_RADIUS: f32 = 22.0;
/// Dock icon: side, radius as a share of the side, gap between two icons.
pub const DOCK_ICON: f32 = 52.0;
pub const DOCK_ICON_RADIUS_RATIO: f32 = 0.27;
pub const DOCK_ICON_GAP: f32 = 8.0;
/// Magnification: maximum factor and reach of the bell curve, in icons.
pub const DOCK_MAGNIFICATION: f32 = 1.7;
pub const DOCK_MAGNIFICATION_REACH: f32 = 1.2;

// Spacings, by role.

/// Between two entries of a navigation.
pub const GAP_TIGHT: f32 = 2.0;
/// Between an icon and its text.
pub const GAP_ICON: f32 = 10.0;
/// Between the text of an entry and its shortcut.
pub const GAP_INLINE: f32 = 12.0;
/// Between two blocks of a page, between cards.
pub const GAP_SECTION: f32 = 14.0;
/// Between a label and its control in a setting row.
pub const GAP_ROW: f32 = 16.0;
/// Padding of a window page: vertical and horizontal.
pub const PAGE_PADDING: (f32, f32) = (22.0, 28.0);

// Durations, in milliseconds.

/// Hover: from 110 to 150 ms.
pub const HOVER_MS: (u64, u64) = (110, 150);
/// Dock magnification.
pub const DOCK_TRANSITION_MS: u64 = 150;
/// Window in which a burst of system events (volume slider, network changes) is merged into
/// a single refresh of the shell.
pub const EVENT_COALESCE_MS: u64 = 150;
/// Longest such burst: a volume slider dragged for seconds still refreshes this often.
pub const EVENT_COALESCE_MAX_MS: u64 = 500;

// Token consistency, checked at compile time: a token out of its range does not compile.
const _: () = {
    assert!(BUTTON_COMPACT_HEIGHT >= 28.0 && BUTTON_HEIGHT <= 36.0);
    assert!(BUTTON_COMPACT_HEIGHT < BUTTON_HEIGHT);
    assert!(BUTTON_COMPACT_RADIUS >= 8.0 && BUTTON_RADIUS <= 10.0);
    assert!(ICON_STROKE >= 1.6 && ICON_STROKE <= 1.7);
    assert!(HOVER_MS.0 <= HOVER_MS.1 && DOCK_TRANSITION_MS <= HOVER_MS.1);
    assert!(EVENT_COALESCE_MS < EVENT_COALESCE_MAX_MS);
    // The round knob of the toggle is 20 and fits twice in its track.
    assert!(TOGGLE_HEIGHT - 2.0 * TOGGLE_INSET == 20.0);
    assert!(2.0 * (TOGGLE_HEIGHT - 2.0 * TOGGLE_INSET) < TOGGLE_WIDTH);
    // A menu entry fits in the menu, a submenu is narrower.
    assert!(MENU_ITEM_RADIUS < MENU_RADIUS && MENU_ITEM_HEIGHT < BUTTON_HEIGHT);
    assert!(SUBMENU_WIDTH < MENU_WIDTH);
    assert!(ICON_SIZE_SMALL < ICON_SIZE && ICON_SIZE < ICON_GRID);
    assert!(ICON_SIZE_TINY < ICON_SIZE_SMALL && LIST_ICON < ICON_GRID);
    // A navigation entry and a list row are taller than twice their radius.
    assert!(NAV_ITEM_RADIUS < NAV_ITEM_HEIGHT / 2.0 && LIST_ROW_RADIUS < LIST_ROW_HEIGHT / 2.0);
    assert!(BREADCRUMB_HEIGHT < TOOLBAR_HEIGHT && WINDOW_BUTTON.1 < TITLE_BAR_HEIGHT);
    // The bar's elements fit in its height.
    assert!(SHERLOCK_BUTTON.1 < TOP_BAR_HEIGHT && CHIP_HEIGHT < TOP_BAR_HEIGHT);
    assert!(SHERLOCK_GLYPH < SHERLOCK_BUTTON.1);
};
