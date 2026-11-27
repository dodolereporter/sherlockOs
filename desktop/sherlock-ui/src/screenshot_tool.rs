//! The screenshot tool (lot D-35e), as Super + Shift + S opens it: a toolbar of glass at the
//! bottom of the screen (area, window or whole screen; timer; options), an area to draw and
//! adjust by its handles, the window under the pointer outlined, and a thumbnail of the image
//! taken in the bottom right corner. The compositor draws it. No board draws it: the measures
//! below are choices of the lot, taken from the panel, menu and button tokens, to review with
//! the design. Everything here is plain geometry in logical pixels, without a renderer, so that
//! it is tested at every screen size. See `docs/screenshots.md`.

use std::time::Duration;

use crate::color::{Color, Palette};
use crate::motion::{self, Ease, Spring};
use crate::{metrics, osd};

/// What is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Mode {
    #[default]
    Area,
    Window,
    Screen,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Area, Self::Window, Self::Screen];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Area => "Zone",
            Self::Window => "Fenêtre",
            Self::Screen => "Écran entier",
        }
    }
}

/// The timer: how long after the choice the image is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Delay {
    #[default]
    Off,
    Three,
    Ten,
}

impl Delay {
    pub const ALL: [Self; 3] = [Self::Off, Self::Three, Self::Ten];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Non",
            Self::Three => "3 s",
            Self::Ten => "10 s",
        }
    }

    pub const fn duration(self) -> Duration {
        match self {
            Self::Off => Duration::ZERO,
            Self::Three => Duration::from_secs(3),
            Self::Ten => Duration::from_secs(10),
        }
    }
}

/// Where the image goes. It always goes to the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Destination {
    CopyOnly,
    #[default]
    SaveAndCopy,
}

impl Destination {
    pub const ALL: [Self; 2] = [Self::CopyOnly, Self::SaveAndCopy];

    pub const fn label(self) -> &'static str {
        match self {
            Self::CopyOnly => "Copier seulement",
            Self::SaveAndCopy => "Enregistrer et copier",
        }
    }
}

/// The folder a screenshot is saved in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Folder {
    /// `Captures d'écran` in the pictures folder.
    #[default]
    Screenshots,
    Desktop,
    Documents,
}

impl Folder {
    pub const ALL: [Self; 3] = [Self::Screenshots, Self::Desktop, Self::Documents];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Screenshots => crate::screenshot::FOLDER,
            Self::Desktop => "Bureau",
            Self::Documents => "Documents",
        }
    }

    /// Its key in `user-dirs.dirs`, and the folder of the home folder used when it is not set.
    pub const fn user_dir(self) -> (&'static str, &'static str) {
        match self {
            Self::Screenshots => ("XDG_PICTURES_DIR", crate::screenshot::PICTURES),
            Self::Desktop => ("XDG_DESKTOP_DIR", "Bureau"),
            Self::Documents => ("XDG_DOCUMENTS_DIR", "Documents"),
        }
    }
}

/// The choices of the toolbar, kept from one opening to the next while the session lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Options {
    pub mode: Mode,
    pub delay: Delay,
    pub destination: Destination,
    pub folder: Folder,
    /// The cursor drawn in the image.
    pub cursor: bool,
}

/// Texts of the toolbar and of its options menu.
pub const TIMER: &str = "Minuteur";
pub const OPTIONS: &str = "Options";
pub const CANCEL: &str = "Annuler";
pub const CAPTURE: &str = "Capturer";
pub const AFTER: &str = "Après la capture";
pub const SAVE_IN: &str = "Enregistrer dans";
pub const SHOW_CURSOR: &str = "Montrer le curseur";
/// The notification when the image only went to the clipboard.
pub const COPIED: &str = "Capture d'écran copiée";
pub const COPIED_BODY: &str = "Elle est dans le presse-papiers.";

/// The size of the area, in pixels of the screen, as shown beside the pointer.
pub fn size_text(width: u32, height: u32) -> String {
    format!("{width} × {height}")
}

// Measures, on a screen 1920 logical pixels wide. Elsewhere the toolbar takes the same share of
// the width, never wider than the screen (`fit`).

/// Height of the toolbar, its radius (a floating panel's, rounder), and the space around its
/// buttons.
pub const BAR_HEIGHT: f32 = 48.0;
pub const BAR_RADIUS: f32 = 16.0;
pub const BAR_PADDING: f32 = 8.0;
/// A button of the toolbar: its height and radius (a button's), and the text padding.
pub const BUTTON_HEIGHT: f32 = 32.0;
pub const BUTTON_RADIUS: f32 = 9.0;
pub const BUTTON_PADDING: f32 = 12.0;
/// Between two buttons of a group, and between two groups, a thin divider in the middle.
pub const GAP: f32 = 2.0;
pub const GROUP_GAP: f32 = 17.0;
pub const DIVIDER_WIDTH: f32 = 1.0;
pub const DIVIDER_HEIGHT: f32 = 20.0;
/// From the toolbar to the edges of the screen, at least.
pub const MARGIN: f32 = 12.0;
/// The toolbar never grows past this scale, on any screen.
pub const FIT_MAX: f32 = 1.25;

/// The options menu, above its button: a menu's measures.
pub const MENU_PADDING: f32 = 6.0;
pub const MENU_ENTRY: f32 = 30.0;
pub const MENU_TITLE: f32 = 26.0;
pub const MENU_GAP: f32 = 8.0;
pub const MENU_RADIUS: f32 = metrics::PANEL_RADIUS - 8.0;
pub const MENU_ENTRY_RADIUS: f32 = 7.0;
/// Room on the left of an entry for the mark of the chosen one, and its diameter.
pub const MARK_ROOM: f32 = 22.0;
pub const MARK_SIZE: f32 = 6.0;

/// The area: its border, its handles (squares of `HANDLE` with a border), and how far from a
/// handle or an edge the pointer still takes it.
pub const AREA_BORDER: f32 = 1.5;
pub const HANDLE: f32 = 9.0;
pub const HANDLE_REACH: f64 = 10.0;
/// An area smaller than this, once released, is no area: a click.
pub const MIN_AREA: f64 = 4.0;
/// The window under the pointer, in window mode: its outline outside its frame.
pub const HIGHLIGHT_WIDTH: f32 = 3.0;
/// Opacity of the dim laid on the screen around the area.
pub const DIM: f32 = crate::screenshot::DIM;
/// The size label and the loupe, at this distance from the pointer.
pub const POINTER_GAP: f64 = 18.0;
pub const LABEL_HEIGHT: f32 = 24.0;
/// The loupe: pixels of the screen seen across (odd: the middle one is the pointer's), and
/// the size each one takes.
pub const LOUPE_PIXELS: u32 = 15;
pub const LOUPE_ZOOM: f32 = 8.0;
pub const LOUPE_BORDER: f32 = 2.0;
/// The countdown of the timer, in the middle of the screen.
pub const COUNTDOWN_SIZE: f32 = 96.0;

/// The thumbnail of the image taken: its longest side, its margin to the bottom right corner,
/// how long it stays, and how far a drag towards the edge must go to throw it away.
pub const THUMBNAIL: f32 = 180.0;
pub const THUMBNAIL_MARGIN: f32 = 20.0;
pub const THUMBNAIL_RADIUS: f32 = 10.0;
pub const THUMBNAIL_VISIBLE: Duration = Duration::from_secs(5);
pub const THUMBNAIL_DISMISS: f64 = 0.35;
/// A press that moves less than this is a click.
pub const CLICK_SLOP: f64 = 4.0;

/// Coming: the toolbar rises by `RISE` and fades in, as a window opens; going: the other way,
/// on the curve of a window that closes. The dim follows the same times. The thumbnail slides
/// in from the right edge.
pub const RISE: f64 = 16.0;
pub const ENTER: Duration = motion::WINDOW_OPEN;
pub const ENTER_EASE: Ease = motion::WINDOW_OPEN_EASE;
pub const LEAVE: Duration = motion::WINDOW_CLOSE;
pub const LEAVE_EASE: Ease = motion::WINDOW_CLOSE_EASE;
/// A hovered or pressed button changes color in the time of a hover.
pub const HOVER: Duration = motion::FADE;
/// The area going back inside the screen after a drag past its edge, and the thumbnail coming
/// back after a drag that did not throw it away.
pub const SETTLE_SPRING: Spring = motion::SNAP_SPRING;
/// Past an edge, the area moves less and less, as a rubber band (the formula of iOS).
pub const RUBBER: f64 = crate::apps::RUBBER as f64;

const _: () = {
    assert!(BUTTON_HEIGHT + 2.0 * BAR_PADDING == BAR_HEIGHT);
    assert!(LOUPE_PIXELS % 2 == 1);
    assert!(DIVIDER_HEIGHT < BUTTON_HEIGHT);
};

/// The colors of the tool in a palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub dim: Color,
    pub glass: Color,
    pub border: Color,
    pub divider: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub disabled: Color,
    pub hover: Color,
    pub pressed: Color,
    /// The chosen button of a group.
    pub chosen: Color,
    pub accent: Color,
    pub text_on_accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub area_fill: Color,
}

impl Colors {
    pub const fn of(palette: &Palette) -> Self {
        Self {
            dim: palette.scrim,
            glass: palette.menu_glass,
            border: palette.menu_border,
            divider: palette.divider,
            text: palette.text_primary,
            text_secondary: palette.text_tertiary,
            disabled: palette.disabled,
            hover: palette.hover,
            pressed: palette.pressed,
            chosen: palette.accent_soft,
            accent: palette.accent,
            text_on_accent: palette.text_on_accent,
            accent_hover: palette.accent_hover,
            accent_pressed: palette.accent_pressed,
            area_fill: Color::rgba(0, 0, 0, 0.0),
        }
    }
}

/// A rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    /// The rectangle between two corners, in any order.
    pub fn between(a: (f64, f64), b: (f64, f64)) -> Self {
        let (x, y) = (a.0.min(b.0), a.1.min(b.1));
        Self::new(x, y, (a.0 - b.0).abs(), (a.1 - b.1).abs())
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn contains(&self, (x, y): (f64, f64)) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// Moved by `(dx, dy)`.
    pub fn moved(&self, (dx, dy): (f64, f64)) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// The part inside `bounds`, of zero size when they do not meet.
    pub fn inside(&self, bounds: &Self) -> Self {
        let (x, y) = (self.x.max(bounds.x), self.y.max(bounds.y));
        let (r, b) = (
            self.right().min(bounds.right()),
            self.bottom().min(bounds.bottom()),
        );
        Self::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }

    /// Pushed back inside `bounds` without changing its size, unless it is larger.
    pub fn pushed_inside(&self, bounds: &Self) -> Self {
        let (w, h) = (self.w.min(bounds.w), self.h.min(bounds.h));
        let x = self.x.clamp(bounds.x, bounds.right() - w);
        let y = self.y.clamp(bounds.y, bounds.bottom() - h);
        Self::new(x, y, w, h)
    }
}

/// A handle of the area: its four corners and the middle of its four edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Right,
        Self::BottomRight,
        Self::Bottom,
        Self::BottomLeft,
        Self::Left,
    ];

    /// The edges it pulls: left, top, right, bottom.
    pub const fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Self::TopLeft => (true, true, false, false),
            Self::Top => (false, true, false, false),
            Self::TopRight => (false, true, true, false),
            Self::Right => (false, false, true, false),
            Self::BottomRight => (false, false, true, true),
            Self::Bottom => (false, false, false, true),
            Self::BottomLeft => (true, false, false, true),
            Self::Left => (true, false, false, false),
        }
    }

    /// Where it sits on `area`.
    pub fn center(self, area: &Rect) -> (f64, f64) {
        let (left, top, right, bottom) = self.edges();
        let x = match (left, right) {
            (true, _) => area.x,
            (_, true) => area.right(),
            _ => area.x + area.w / 2.0,
        };
        let y = match (top, bottom) {
            (true, _) => area.y,
            (_, true) => area.bottom(),
            _ => area.y + area.h / 2.0,
        };
        (x, y)
    }

    fn from_edges(edges: (bool, bool, bool, bool)) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.edges() == edges)
    }
}

/// What the pointer takes of the area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grip {
    /// A handle, or an edge anywhere along it.
    Handle(Handle),
    /// The inside: the area moves.
    Inside,
    /// Outside: a new area is drawn.
    Outside,
}

/// What the pointer at `p` takes of `area`.
pub fn grip(area: &Rect, p: (f64, f64)) -> Grip {
    let near = |a: f64, b: f64| (a - b).abs() <= HANDLE_REACH;
    let within = |v: f64, lo: f64, hi: f64| v >= lo - HANDLE_REACH && v <= hi + HANDLE_REACH;
    if !within(p.0, area.x, area.right()) || !within(p.1, area.y, area.bottom()) {
        return Grip::Outside;
    }
    // On a small area the edges are close: the nearest one wins.
    let left = near(p.0, area.x) && p.0 - area.x <= area.right() - p.0;
    let right = near(p.0, area.right()) && !left;
    let top = near(p.1, area.y) && p.1 - area.y <= area.bottom() - p.1;
    let bottom = near(p.1, area.bottom()) && !top;
    match Handle::from_edges((left, top, right, bottom)) {
        Some(handle) => Grip::Handle(handle),
        None if area.contains(p) => Grip::Inside,
        None => Grip::Outside,
    }
}

/// `area` with the edges of `handle` moved by `(dx, dy)`. Pulled past the opposite edge, it
/// turns over, as on macOS: the result is never of negative size.
pub fn resized(area: &Rect, handle: Handle, (dx, dy): (f64, f64)) -> Rect {
    let (left, top, right, bottom) = handle.edges();
    let mut l = area.x + if left { dx } else { 0.0 };
    let mut t = area.y + if top { dy } else { 0.0 };
    let mut r = area.right() + if right { dx } else { 0.0 };
    let mut b = area.bottom() + if bottom { dy } else { 0.0 };
    if l > r {
        std::mem::swap(&mut l, &mut r);
    }
    if t > b {
        std::mem::swap(&mut t, &mut b);
    }
    Rect::new(l, t, r - l, b - t)
}

/// `x` kept between `lo` and `hi`, the excess past either one shrunk as a rubber band stretches
/// over `span`.
pub fn rubber(x: f64, lo: f64, hi: f64, span: f64) -> f64 {
    let span = span.max(1.0);
    let stretch = |excess: f64| (1.0 - 1.0 / (excess * RUBBER / span + 1.0)) * span;
    if x < lo {
        lo - stretch(lo - x)
    } else if x > hi {
        hi + stretch(x - hi)
    } else {
        x
    }
}

/// The area as shown while it is dragged past the edges of `bounds`: moved as a whole when
/// `moving`, or edge by edge, each going past less and less.
pub fn elastic(area: &Rect, bounds: &Rect, moving: bool) -> Rect {
    let span = bounds.w.min(bounds.h);
    if moving {
        let settled = area.pushed_inside(bounds);
        let x = rubber(area.x, settled.x, settled.x, span);
        let y = rubber(area.y, settled.y, settled.y, span);
        return Rect::new(x, y, settled.w, settled.h);
    }
    let l = rubber(area.x, bounds.x, bounds.right(), span);
    let t = rubber(area.y, bounds.y, bounds.bottom(), span);
    let r = rubber(area.right(), bounds.x, bounds.right(), span);
    let b = rubber(area.bottom(), bounds.y, bounds.bottom(), span);
    Rect::new(l, t, (r - l).max(0.0), (b - t).max(0.0))
}

/// Where the area comes back to once let go: inside `bounds`.
pub fn settled(area: &Rect, bounds: &Rect, moving: bool) -> Rect {
    if moving {
        area.pushed_inside(bounds)
    } else {
        area.inside(bounds)
    }
}

/// The top left corner of a box of `size` beside the pointer at `p`: below and right of it,
/// or on the other side when it would leave `bounds`.
pub fn beside(p: (f64, f64), size: (f64, f64), bounds: &Rect) -> (f64, f64) {
    let x = if p.0 + POINTER_GAP + size.0 <= bounds.right() {
        p.0 + POINTER_GAP
    } else {
        p.0 - POINTER_GAP - size.0
    };
    let y = if p.1 + POINTER_GAP + size.1 <= bounds.bottom() {
        p.1 + POINTER_GAP
    } else {
        p.1 - POINTER_GAP - size.1
    };
    (x.max(bounds.x), y.max(bounds.y))
}

/// The scale of the toolbar on a screen `width` logical pixels wide whose toolbar would be
/// `natural` wide at scale 1: 1 on a screen 1920 wide, smaller on a narrow screen so that it
/// fits, a little larger on a wide one.
pub fn fit(width: f64, natural: f64) -> f64 {
    let share = (width / 1920.0).clamp(0.0, f64::from(FIT_MAX));
    let room = (width - 2.0 * f64::from(MARGIN)).max(1.0) / natural.max(1.0);
    share.max(1.0).min(room).min(f64::from(FIT_MAX))
}

/// A control of the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Control {
    Mode(Mode),
    /// The word « Minuteur » before the timer choices: not a button.
    Timer,
    Delay(Delay),
    Options,
    Cancel,
    Capture,
}

impl Control {
    /// The controls in their order, a new group after each `None`.
    pub const ROW: [Option<Self>; 13] = [
        Some(Self::Mode(Mode::Area)),
        Some(Self::Mode(Mode::Window)),
        Some(Self::Mode(Mode::Screen)),
        None,
        Some(Self::Timer),
        Some(Self::Delay(Delay::Off)),
        Some(Self::Delay(Delay::Three)),
        Some(Self::Delay(Delay::Ten)),
        None,
        Some(Self::Options),
        None,
        Some(Self::Cancel),
        Some(Self::Capture),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mode(mode) => mode.label(),
            Self::Timer => TIMER,
            Self::Delay(delay) => delay.label(),
            Self::Options => OPTIONS,
            Self::Cancel => CANCEL,
            Self::Capture => CAPTURE,
        }
    }

    /// A button, which the pointer may hover and press.
    pub fn is_button(self) -> bool {
        self != Self::Timer
    }

    /// Chosen with `options`: drawn in the chosen color.
    pub fn chosen(self, options: &Options) -> bool {
        match self {
            Self::Mode(mode) => options.mode == mode,
            Self::Delay(delay) => options.delay == delay,
            _ => false,
        }
    }
}

/// The toolbar laid out on a screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub rect: Rect,
    /// Its scale (`fit`): of its radius, its text and its measures.
    pub fit: f64,
    pub controls: Vec<(Control, Rect)>,
    pub dividers: Vec<Rect>,
}

impl Bar {
    /// The toolbar on `screen`, above the dock that reserves `reserved` at the bottom (0 when
    /// it reserves none), each text `measure(text)` wide at scale 1, padding not included.
    pub fn layout(screen: &Rect, reserved: f64, measure: impl Fn(&str) -> f64) -> Self {
        let (pad, gap, group) = (
            f64::from(BUTTON_PADDING),
            f64::from(GAP),
            f64::from(GROUP_GAP),
        );
        // At scale 1, from the left edge of the bar.
        let mut x = f64::from(BAR_PADDING);
        let mut placed = Vec::new();
        let mut dividers = Vec::new();
        for slot in Control::ROW {
            let Some(control) = slot else {
                dividers.push(x - gap + group / 2.0);
                x += group - gap;
                continue;
            };
            let w = measure(control.label()).ceil() + 2.0 * pad;
            placed.push((control, x, w));
            x += w + gap;
        }
        let natural = x - gap + f64::from(BAR_PADDING);
        let k = fit(screen.w, natural);
        let (w, h) = (natural * k, f64::from(BAR_HEIGHT) * k);
        let bottom = f64::from(osd::bottom(screen.h as f32, reserved as f32, k as f32))
            .min(screen.h - h - f64::from(MARGIN))
            .max(f64::from(MARGIN).min(screen.h - h));
        let rect = Rect::new(
            screen.x + (screen.w - w) / 2.0,
            screen.y + screen.h - bottom - h,
            w,
            h,
        );
        let button_y = rect.y + f64::from(BAR_PADDING) * k;
        let button_h = f64::from(BUTTON_HEIGHT) * k;
        let controls = placed
            .into_iter()
            .map(|(c, x, w)| (c, Rect::new(rect.x + x * k, button_y, w * k, button_h)))
            .collect();
        let divider_h = f64::from(DIVIDER_HEIGHT) * k;
        let dividers = dividers
            .into_iter()
            .map(|x| {
                Rect::new(
                    rect.x + x * k,
                    rect.y + (h - divider_h) / 2.0,
                    f64::from(DIVIDER_WIDTH) * k,
                    divider_h,
                )
            })
            .collect();
        Self {
            rect,
            fit: k,
            controls,
            dividers,
        }
    }

    /// The button under `p`.
    pub fn button_at(&self, p: (f64, f64)) -> Option<Control> {
        self.controls
            .iter()
            .find(|(c, r)| c.is_button() && r.contains(p))
            .map(|(c, _)| *c)
    }

    pub fn rect_of(&self, control: Control) -> Option<Rect> {
        self.controls
            .iter()
            .find(|(c, _)| *c == control)
            .map(|(_, r)| *r)
    }
}

/// An entry of the options menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Entry {
    /// A section title: not an entry one chooses.
    Title(&'static str),
    Destination(Destination),
    Folder(Folder),
    Cursor,
}

impl Entry {
    pub const ALL: [Self; 9] = [
        Self::Title(AFTER),
        Self::Destination(Destination::CopyOnly),
        Self::Destination(Destination::SaveAndCopy),
        Self::Title(SAVE_IN),
        Self::Folder(Folder::Screenshots),
        Self::Folder(Folder::Desktop),
        Self::Folder(Folder::Documents),
        Self::Title(""),
        Self::Cursor,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Title(title) => title,
            Self::Destination(d) => d.label(),
            Self::Folder(f) => f.label(),
            Self::Cursor => SHOW_CURSOR,
        }
    }

    /// Whether it bears the mark of a chosen entry.
    pub fn checked(self, options: &Options) -> bool {
        match self {
            Self::Title(_) => false,
            Self::Destination(d) => options.destination == d,
            Self::Folder(f) => options.folder == f,
            Self::Cursor => options.cursor,
        }
    }

    /// Whether it can be chosen: the folder only matters when the image is saved.
    pub fn enabled(self, options: &Options) -> bool {
        match self {
            Self::Title(_) => false,
            Self::Folder(_) => options.destination == Destination::SaveAndCopy,
            _ => true,
        }
    }

    /// `options` once it is chosen.
    pub fn choose(self, options: &mut Options) {
        match self {
            Self::Title(_) => {}
            Self::Destination(d) => options.destination = d,
            Self::Folder(f) => options.folder = f,
            Self::Cursor => options.cursor = !options.cursor,
        }
    }
}

/// The options menu laid out above its button.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub rect: Rect,
    pub entries: Vec<(Entry, Rect)>,
}

impl Menu {
    /// Above `button` of a toolbar at scale `k`, kept on `screen`, each text `measure(text)`
    /// wide at scale 1.
    pub fn layout(button: &Rect, k: f64, screen: &Rect, measure: impl Fn(&str) -> f64) -> Self {
        let pad = f64::from(MENU_PADDING);
        let text = Entry::ALL
            .iter()
            .map(|e| measure(e.label()))
            .fold(0.0, f64::max);
        let inner = f64::from(MARK_ROOM) + text.ceil() + f64::from(BUTTON_PADDING);
        let w = (inner + 2.0 * pad) * k;
        let height = |e: &Entry| match e {
            Entry::Title("") => f64::from(MENU_PADDING),
            Entry::Title(_) => f64::from(MENU_TITLE),
            _ => f64::from(MENU_ENTRY),
        };
        let h = (Entry::ALL.iter().map(height).sum::<f64>() + 2.0 * pad) * k;
        let x = (button.x + button.w / 2.0 - w / 2.0).clamp(
            screen.x + f64::from(MARGIN),
            screen.right() - f64::from(MARGIN) - w,
        );
        let y = (button.y - f64::from(MENU_GAP) * k - h).max(screen.y + f64::from(MARGIN));
        let rect = Rect::new(x, y, w, h);
        let mut top = y + pad * k;
        let entries = Entry::ALL
            .iter()
            .map(|e| {
                let r = Rect::new(x + pad * k, top, inner * k, height(e) * k);
                top += r.h;
                (*e, r)
            })
            .collect();
        Self { rect, entries }
    }

    /// The entry under `p`, if it can be chosen.
    pub fn entry_at(&self, p: (f64, f64), options: &Options) -> Option<Entry> {
        self.entries
            .iter()
            .find(|(e, r)| e.enabled(options) && r.contains(p))
            .map(|(e, _)| *e)
    }
}

/// The thumbnail of an image of `size` pixels, in the bottom right corner of `screen`, slid
/// right by `slide` logical pixels (0 in place).
pub fn thumbnail(screen: &Rect, size: (u32, u32), slide: f64) -> Rect {
    let (w, h) = (f64::from(size.0.max(1)), f64::from(size.1.max(1)));
    let k = f64::from(THUMBNAIL) / w.max(h);
    let (tw, th) = ((w * k).max(24.0), (h * k).max(24.0));
    let margin = f64::from(THUMBNAIL_MARGIN);
    Rect::new(
        screen.right() - margin - tw + slide,
        screen.bottom() - margin - th,
        tw,
        th,
    )
}

/// How far the thumbnail must be slid to be out of `screen`.
pub fn thumbnail_out(thumb: &Rect, screen: &Rect) -> f64 {
    screen.right() - thumb.x + metrics::SHADOW_BLUR as f64 / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Texts of 7 pixels a letter, as a stand-in for the font.
    fn measure(text: &str) -> f64 {
        text.chars().count() as f64 * 7.0
    }

    fn screen(w: f64, h: f64) -> Rect {
        Rect::new(0.0, 0.0, w, h)
    }

    #[test]
    fn the_toolbar_fits_every_screen_and_keeps_its_order() {
        // A small laptop, 1920 × 1080, 4K at scale 1.5 and 2 (in logical pixels), a projector.
        for (w, h) in [
            (1366.0, 768.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3840.0, 2160.0),
            (1024.0, 768.0),
            (800.0, 600.0),
            (640.0, 480.0),
        ] {
            let s = screen(w, h);
            let bar = Bar::layout(&s, 0.0, measure);
            assert!(
                bar.rect.x >= f64::from(MARGIN) - 1e-6,
                "{w}: {:?}",
                bar.rect
            );
            assert!(
                bar.rect.right() <= w - f64::from(MARGIN) + 1e-6,
                "{w}: {:?}",
                bar.rect
            );
            assert!(bar.rect.bottom() <= h, "{h}");
            assert!(
                (bar.rect.x + bar.rect.w / 2.0 - w / 2.0).abs() < 1e-6,
                "centered"
            );
            let xs: Vec<f64> = bar.controls.iter().map(|(_, r)| r.x).collect();
            assert!(xs.windows(2).all(|p| p[0] < p[1]));
            for (_, r) in &bar.controls {
                assert!(r.x >= bar.rect.x && r.right() <= bar.rect.right() + 1e-6);
                assert!(r.y >= bar.rect.y && r.bottom() <= bar.rect.bottom() + 1e-6);
            }
            assert_eq!(bar.dividers.len(), 3);
        }
        let full = Bar::layout(&screen(1920.0, 1080.0), 0.0, measure);
        assert!((full.fit - 1.0).abs() < 1e-9);
        assert!((full.rect.h - f64::from(BAR_HEIGHT)).abs() < 1e-9);
        // Above the dock.
        assert!(1080.0 - full.rect.bottom() >= f64::from(osd::DOCK_BAND));
        let narrow = Bar::layout(&screen(640.0, 480.0), 0.0, measure);
        assert!(narrow.fit < 1.0);
    }

    #[test]
    fn the_buttons_answer_but_not_the_timer_word() {
        let bar = Bar::layout(&screen(1920.0, 1080.0), 0.0, measure);
        let center = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let window = bar.rect_of(Control::Mode(Mode::Window)).unwrap();
        assert_eq!(
            bar.button_at(center(window)),
            Some(Control::Mode(Mode::Window))
        );
        let timer = bar.rect_of(Control::Timer).unwrap();
        assert_eq!(bar.button_at(center(timer)), None);
        assert_eq!(bar.button_at((0.0, 0.0)), None);
        let options = Options::default();
        assert!(Control::Mode(Mode::Area).chosen(&options));
        assert!(Control::Delay(Delay::Off).chosen(&options));
        assert!(!Control::Capture.chosen(&options));
    }

    #[test]
    fn the_menu_stays_on_screen_and_its_folder_waits_for_a_save() {
        let s = screen(1366.0, 768.0);
        let bar = Bar::layout(&s, 0.0, measure);
        let button = bar.rect_of(Control::Options).unwrap();
        let menu = Menu::layout(&button, bar.fit, &s, measure);
        assert!(menu.rect.bottom() <= button.y);
        assert!(menu.rect.x >= 0.0 && menu.rect.right() <= s.w);
        let mut options = Options::default();
        let (_, desktop) = menu.entries[5];
        let at = (desktop.x + 4.0, desktop.y + 4.0);
        assert_eq!(
            menu.entry_at(at, &options),
            Some(Entry::Folder(Folder::Desktop))
        );
        Entry::Destination(Destination::CopyOnly).choose(&mut options);
        assert_eq!(menu.entry_at(at, &options), None, "disabled");
        assert!(!Entry::Folder(Folder::Desktop).enabled(&options));
        Entry::Cursor.choose(&mut options);
        assert!(Entry::Cursor.checked(&options));
        let (_, title) = menu.entries[0];
        assert_eq!(
            menu.entry_at((title.x + 4.0, title.y + 4.0), &options),
            None
        );
    }

    #[test]
    fn handles_and_edges_are_taken_near_them() {
        let area = Rect::new(100.0, 100.0, 200.0, 100.0);
        assert_eq!(grip(&area, (101.0, 99.0)), Grip::Handle(Handle::TopLeft));
        assert_eq!(
            grip(&area, (305.0, 205.0)),
            Grip::Handle(Handle::BottomRight)
        );
        assert_eq!(grip(&area, (150.0, 102.0)), Grip::Handle(Handle::Top));
        assert_eq!(grip(&area, (295.0, 150.0)), Grip::Handle(Handle::Right));
        assert_eq!(grip(&area, (200.0, 150.0)), Grip::Inside);
        assert_eq!(grip(&area, (50.0, 150.0)), Grip::Outside);
        // A tiny area: the nearest edge wins, never both.
        let tiny = Rect::new(100.0, 100.0, 6.0, 6.0);
        assert_eq!(grip(&tiny, (101.0, 101.0)), Grip::Handle(Handle::TopLeft));
        assert_eq!(
            grip(&tiny, (105.0, 105.0)),
            Grip::Handle(Handle::BottomRight)
        );
    }

    #[test]
    fn a_handle_pulled_past_the_other_edge_turns_the_area_over() {
        let area = Rect::new(100.0, 100.0, 200.0, 100.0);
        let wider = resized(&area, Handle::Right, (50.0, 30.0));
        assert_eq!(wider, Rect::new(100.0, 100.0, 250.0, 100.0));
        let over = resized(&area, Handle::Left, (260.0, 0.0));
        assert_eq!(over, Rect::new(300.0, 100.0, 60.0, 100.0));
        let corner = resized(&area, Handle::TopLeft, (-10.0, -20.0));
        assert_eq!(corner, Rect::new(90.0, 80.0, 210.0, 120.0));
    }

    #[test]
    fn past_the_edge_the_area_stretches_then_comes_back() {
        let bounds = screen(1920.0, 1080.0);
        let area = Rect::new(-100.0, 10.0, 300.0, 200.0);
        let shown = elastic(&area, &bounds, true);
        // Goes past, but less than asked, and keeps its size.
        assert!(shown.x < 0.0 && shown.x > -100.0, "{shown:?}");
        assert_eq!((shown.w, shown.h), (300.0, 200.0));
        assert_eq!(settled(&area, &bounds, true).x, 0.0);
        // Further asked, further shown, never as far.
        let further = elastic(&area.moved((-200.0, 0.0)), &bounds, true);
        assert!(further.x < shown.x && further.x > -300.0);
        // Edge by edge: the right edge past the screen.
        let wide = Rect::new(1800.0, 10.0, 300.0, 200.0);
        let shown = elastic(&wide, &bounds, false);
        assert!(shown.right() > 1920.0 && shown.right() < 2100.0);
        assert_eq!(settled(&wide, &bounds, false).right(), 1920.0);
        // Inside, nothing changes.
        let inside = Rect::new(10.0, 10.0, 30.0, 30.0);
        assert_eq!(elastic(&inside, &bounds, true), inside);
        assert_eq!(elastic(&inside, &bounds, false), inside);
    }

    #[test]
    fn what_shows_beside_the_pointer_stays_on_screen() {
        let bounds = screen(1366.0, 768.0);
        assert_eq!(
            beside((100.0, 100.0), (80.0, 24.0), &bounds),
            (118.0, 118.0)
        );
        let (x, y) = beside((1360.0, 760.0), (80.0, 24.0), &bounds);
        assert!(x + 80.0 <= 1366.0 && y + 24.0 <= 768.0);
        assert_eq!(size_text(1920, 1080), "1920 × 1080");
    }

    #[test]
    fn the_thumbnail_sits_in_the_corner_and_leaves_by_the_edge() {
        for s in [
            screen(1366.0, 768.0),
            screen(1920.0, 1080.0),
            screen(2560.0, 1440.0),
        ] {
            let thumb = thumbnail(&s, (1920, 1080), 0.0);
            assert!((thumb.w - f64::from(THUMBNAIL)).abs() < 1e-9);
            assert!((s.right() - thumb.right() - f64::from(THUMBNAIL_MARGIN)).abs() < 1e-9);
            let out = thumbnail_out(&thumb, &s);
            assert!(thumbnail(&s, (1920, 1080), out).x >= s.right());
        }
        // A tall image keeps its height bounded.
        let tall = thumbnail(&screen(1920.0, 1080.0), (100, 2000), 0.0);
        assert!((tall.h - f64::from(THUMBNAIL)).abs() < 1e-9 && tall.w >= 24.0);
    }
}
