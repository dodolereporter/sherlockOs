//! A notification as the daemon keeps it, read from the arguments of `Notify`. Everything a
//! client sends is bounded here: texts, actions, icons. See `docs/notifications.md`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use sherlock_ui::notifications as ui;
use zbus::zvariant::OwnedValue;

use crate::markup::{self, Span};
use crate::picture;

/// The key of the action a click on the bubble itself triggers.
pub const DEFAULT_ACTION: &str = "default";
/// Actions kept at most, and the longest key: a longer key is dropped with its action, since a
/// cut key would no longer be the application's.
pub const MAX_ACTIONS: usize = 32;
pub const MAX_ACTION_KEY_BYTES: usize = 256;
/// Longest icon name of a theme, in bytes: a file name.
pub const MAX_ICON_NAME_BYTES: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

/// Why a notification closed: the codes of the `NotificationClosed` signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
    /// Closed by the daemon itself: too many waiting, or hidden by Do not disturb.
    Undefined = 4,
}

/// The picture of a notification, from the `image-data`, `image-path` hints or `app_icon`.
#[derive(Debug, Clone, PartialEq)]
pub enum Icon {
    None,
    /// An icon name of the theme: not looked up yet (see `docs/notifications.md`).
    Name(String),
    /// A file read by the daemon and checked (`picture`): its bytes, never its path.
    Png(Arc<Vec<u8>>),
    Svg(Arc<Vec<u8>>),
    /// Raw pixels, made RGBA.
    Pixels {
        width: u32,
        height: u32,
        rgba: Arc<Vec<u8>>,
    },
}

/// The arguments of `Notify`, and the unique bus name of who sent it.
#[derive(Debug, Default)]
pub struct Request {
    pub sender: String,
    pub app_name: String,
    pub replaces_id: u32,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<String>,
    pub hints: HashMap<String, OwnedValue>,
    pub expire_timeout: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub id: u32,
    /// The unique bus name of the application: only it may replace or close the notification.
    pub sender: String,
    pub app_name: String,
    pub summary: String,
    pub body: Vec<Span>,
    pub icon: Icon,
    /// Action keys and their labels, in the order the application gave them.
    pub actions: Vec<(String, String)>,
    pub urgency: Urgency,
    /// `None`: stays until it is closed.
    pub timeout: Option<Duration>,
    /// Stays after one of its actions was invoked (`resident` hint).
    pub resident: bool,
    pub received: SystemTime,
}

impl Notification {
    /// Reads a request, bounding all it holds. Files are read here, off the interface thread.
    pub fn new(id: u32, request: Request) -> Self {
        let hints = &request.hints;
        let urgency = match hint::<u8>(hints, "urgency") {
            Some(0) => Urgency::Low,
            Some(2) => Urgency::Critical,
            _ => Urgency::Normal,
        };
        let timeout = match request.expire_timeout {
            _ if urgency == Urgency::Critical => None,
            0 => None,
            t if t < 0 => Some(ui::DEFAULT_TIMEOUT),
            t => Some(Duration::from_millis(t.unsigned_abs().into())),
        };
        let actions = request
            .actions
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|[key, _]| !key.is_empty() && key.len() <= MAX_ACTION_KEY_BYTES)
            .take(MAX_ACTIONS)
            .map(|[key, label]| (key.clone(), markup::clean(label, ui::MAX_LABEL_CHARS)))
            .collect();
        Self {
            id,
            sender: request.sender,
            app_name: markup::clean(&request.app_name, ui::MAX_APP_NAME_CHARS),
            summary: markup::clean(&request.summary, ui::MAX_SUMMARY_CHARS),
            body: markup::parse(&request.body, ui::MAX_BODY_CHARS),
            icon: icon(&request.app_icon, hints),
            actions,
            urgency,
            timeout,
            resident: hint::<bool>(hints, "resident").unwrap_or(false),
            received: SystemTime::now(),
        }
    }

    /// Whether a click on the bubble invokes an action.
    pub fn has_default(&self) -> bool {
        self.actions.iter().any(|(key, _)| key == DEFAULT_ACTION)
    }

    /// The actions shown as buttons, but the default one: a row of `ui::MAX_BUTTONS`, then the
    /// « Options » list (`ui::MAX_OPTIONS`).
    pub fn buttons(&self) -> impl Iterator<Item = &(String, String)> {
        self.actions
            .iter()
            .filter(|(key, _)| key != DEFAULT_ACTION)
            .take(ui::MAX_BUTTONS - 1 + ui::MAX_OPTIONS)
    }
}

fn hint<T>(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    T::try_from(hints.get(key)?.try_clone().ok()?).ok()
}

/// The picture: raw pixels first, then a picture path, then the application icon, as the
/// specification orders them. The older names of the hints are accepted too.
fn icon(app_icon: &str, hints: &HashMap<String, OwnedValue>) -> Icon {
    ["image-data", "image_data", "icon_data"]
        .iter()
        .find_map(|key| pixels(hints.get(*key)?))
        .or_else(|| {
            ["image-path", "image_path"]
                .iter()
                .find_map(|key| hint::<String>(hints, key).filter(|p| !p.is_empty()))
                .map(|path| named(&path))
        })
        .unwrap_or_else(|| named(app_icon))
}

/// An icon given as text: a `file://` URI or an absolute path, read now; or a theme name.
fn named(text: &str) -> Icon {
    let text = text.trim();
    match text.strip_prefix("file://").unwrap_or(text) {
        "" => Icon::None,
        path if path.starts_with('/') => match picture::read(Path::new(path)) {
            Ok((picture::Kind::Png, bytes)) => Icon::Png(Arc::new(bytes)),
            Ok((picture::Kind::Svg, bytes)) => Icon::Svg(Arc::new(bytes)),
            Err(error) => {
                tracing::debug!(%error, "icon file refused");
                Icon::None
            }
        },
        name if name.len() <= MAX_ICON_NAME_BYTES => themed(name),
        _ => Icon::None,
    }
}

/// The icon `name` of the user's theme, read like a file given by path (D-23d): the same checks,
/// at twice the tile for screens at scale 2. Not found, or refused, the name stays and the
/// bubble shows the initial. Looked up in the D-Bus thread that reads the notification, never
/// in the interface's.
fn themed(name: &str) -> Icon {
    static LOOKUP: std::sync::OnceLock<icon_theme::Lookup> = std::sync::OnceLock::new();
    let lookup = LOOKUP.get_or_init(|| {
        let home = std::env::var_os("HOME").map_or_else(std::path::PathBuf::new, Into::into);
        let dirs = mime_apps::Dirs::from_env(&home);
        let data: Vec<std::path::PathBuf> = dirs.data().cloned().collect();
        let theme = icon_theme::user_theme(&dirs.config_home);
        icon_theme::Lookup::new(&data, Some(&home), theme.as_deref())
    });
    themed_in(lookup, name)
}

fn themed_in(lookup: &icon_theme::Lookup, name: &str) -> Icon {
    let found = lookup
        .find(name, ui::ICON_LOOKUP_SIZE, 1)
        .map(|path| picture::read(&path));
    match found {
        Some(Ok((picture::Kind::Png, bytes))) => Icon::Png(Arc::new(bytes)),
        Some(Ok((picture::Kind::Svg, bytes))) => Icon::Svg(Arc::new(bytes)),
        Some(Err(error)) => {
            tracing::debug!(%error, "theme icon refused");
            Icon::Name(name.to_owned())
        }
        None => Icon::Name(name.to_owned()),
    }
}

/// `(iiibiiay)`: width, height, row stride, alpha, bits per sample, channels, data. Only 8 bits
/// per sample, 3 or 4 channels, within `MAX_PIXELS_SIDE`; the rows are made RGBA.
fn pixels(value: &OwnedValue) -> Option<Icon> {
    type Raw = (i32, i32, i32, bool, i32, i32, Vec<u8>);
    let (width, height, stride, alpha, bits, channels, data) =
        Raw::try_from(value.try_clone().ok()?).ok()?;
    let size = 1..=ui::MAX_PIXELS_SIDE;
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    let fits = size.contains(&width) && size.contains(&height) && bits == 8;
    if !fits || channels != if alpha { 4 } else { 3 } {
        return None;
    }
    let (w, h, stride, channels) = (
        usize::try_from(width).ok()?,
        usize::try_from(height).ok()?,
        usize::try_from(stride).ok()?,
        usize::try_from(channels).ok()?,
    );
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let line = data.get(row * stride..row * stride + w * channels)?;
        for pixel in line.chunks_exact(channels) {
            rgba.extend_from_slice(&pixel[..3]);
            rgba.push(if alpha { pixel[3] } else { u8::MAX });
        }
    }
    Some(Icon::Pixels {
        width,
        height,
        rgba: Arc::new(rgba),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{StructureBuilder, Value};

    fn request(timeout: i32, hints: Vec<(&str, Value<'static>)>) -> Request {
        Request {
            app_name: String::from(" Test "),
            summary: String::from("Titre"),
            body: String::from("<b>Corps</b>"),
            actions: ["default", "Ouvrir", "logs", "Voir les logs", "odd"]
                .map(String::from)
                .to_vec(),
            hints: hints
                .into_iter()
                .map(|(k, v)| (k.to_owned(), OwnedValue::try_from(v).unwrap()))
                .collect(),
            expire_timeout: timeout,
            ..Request::default()
        }
    }

    #[test]
    fn timeouts_follow_the_application_then_the_default_and_never_for_critical() {
        let note = Notification::new(1, request(-1, vec![]));
        assert_eq!(note.timeout, Some(ui::DEFAULT_TIMEOUT));
        assert_eq!(note.urgency, Urgency::Normal);
        assert_eq!(note.app_name, "Test");
        let note = Notification::new(1, request(2500, vec![("urgency", Value::U8(0))]));
        assert_eq!(
            (note.timeout, note.urgency),
            (Some(Duration::from_millis(2500)), Urgency::Low)
        );
        assert_eq!(Notification::new(1, request(0, vec![])).timeout, None);
        let critical = Notification::new(1, request(2500, vec![("urgency", Value::U8(2))]));
        assert_eq!(
            (critical.timeout, critical.urgency),
            (None, Urgency::Critical)
        );
    }

    #[test]
    fn actions_come_in_pairs_and_the_default_is_no_button() {
        let note = Notification::new(1, request(-1, vec![("resident", Value::Bool(true))]));
        assert!(note.has_default() && note.resident);
        let buttons: Vec<_> = note.buttons().map(|(k, _)| k.as_str()).collect();
        assert_eq!(buttons, ["logs"]);
    }

    #[test]
    fn icons_from_pixels_paths_and_names() {
        let mut data: Vec<u8> = vec![10, 20, 30, 40, 50, 60, 0, 0];
        data.extend([0; 8]);
        let too_big = StructureBuilder::new()
            .add_field(257_i32)
            .add_field(1_i32)
            .add_field(771_i32)
            .add_field(false)
            .add_field(8_i32)
            .add_field(3_i32)
            .add_field(vec![0_u8; 771])
            .build()
            .unwrap();
        let note = Notification::new(1, request(-1, vec![("image-data", Value::from(too_big))]));
        assert_eq!(note.icon, Icon::None);
        let image = StructureBuilder::new()
            .add_field(2_i32)
            .add_field(2_i32)
            .add_field(8_i32)
            .add_field(false)
            .add_field(8_i32)
            .add_field(3_i32)
            .add_field(data)
            .build()
            .unwrap();
        let note = Notification::new(1, request(-1, vec![("image-data", Value::from(image))]));
        let Icon::Pixels {
            width,
            height,
            rgba,
        } = note.icon
        else {
            panic!("no pixels: {:?}", note.icon);
        };
        assert_eq!((width, height, rgba.len()), (2, 2, 16));
        assert_eq!(rgba[..8], [10, 20, 30, 255, 40, 50, 60, 255]);
        let path = Value::from("file:///dev/zero");
        let note = Notification::new(1, request(-1, vec![("image-path", path)]));
        assert_eq!(note.icon, Icon::None);
        let unknown = "sherlockos-test-no-such-icon";
        assert_eq!(named(unknown), Icon::Name(String::from(unknown)));
        assert_eq!(named(" "), Icon::None);
        // A name of the theme is read from the theme's file, with the same checks.
        let data = std::env::temp_dir().join(format!("notifications-theme-{}", std::process::id()));
        let apps = data.join("icons/hicolor/scalable/apps");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            data.join("icons/hicolor/index.theme"),
            "[Icon Theme]\nName=Hicolor\nDirectories=scalable/apps\n\n[scalable/apps]\n\
             Size=48\nType=Scalable\nMinSize=16\nMaxSize=512\n",
        )
        .unwrap();
        std::fs::write(
            apps.join("mail.svg"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        )
        .unwrap();
        std::fs::write(
            apps.join("far.svg"),
            "<svg><image href=\"/dev/zero\"/></svg>",
        )
        .unwrap();
        let lookup = icon_theme::Lookup::new(std::slice::from_ref(&data), None, None);
        assert!(matches!(themed_in(&lookup, "mail"), Icon::Svg(_)));
        assert_eq!(themed_in(&lookup, "far"), Icon::Name(String::from("far")));
        std::fs::remove_dir_all(&data).unwrap();
        assert_eq!(named(&"a".repeat(MAX_ICON_NAME_BYTES + 1)), Icon::None);
        let svg =
            std::env::temp_dir().join(format!("notifications-model-{}.svg", std::process::id()));
        std::fs::write(&svg, "<svg/>").unwrap();
        let read = named(&format!("file://{}", svg.display()));
        std::fs::remove_file(&svg).unwrap();
        assert_eq!(read, Icon::Svg(Arc::new(b"<svg/>".to_vec())));
    }

    #[test]
    fn a_huge_request_is_bounded() {
        let mut actions = Vec::new();
        for i in 0..10_000 {
            actions.push(format!("key{i}"));
            actions.push("l".repeat(1000));
        }
        actions.push("k".repeat(MAX_ACTION_KEY_BYTES + 1));
        actions.push(String::from("Clé trop longue"));
        let note = Notification::new(
            1,
            Request {
                app_name: "A".repeat(8 << 20),
                summary: "T".repeat(8 << 20),
                body: "<b>".repeat(4 << 20),
                app_icon: "n".repeat(1 << 20),
                actions,
                ..Request::default()
            },
        );
        assert_eq!(note.app_name.chars().count(), ui::MAX_APP_NAME_CHARS + 1);
        assert_eq!(note.summary.chars().count(), ui::MAX_SUMMARY_CHARS + 1);
        assert!(note.body.iter().map(|s| s.text.len()).sum::<usize>() < 4 * ui::MAX_BODY_CHARS);
        assert_eq!(note.actions.len(), MAX_ACTIONS);
        assert!(
            note.actions
                .iter()
                .all(|(_, l)| l.chars().count() <= ui::MAX_LABEL_CHARS + 1)
        );
        assert_eq!(
            note.buttons().count(),
            ui::MAX_BUTTONS - 1 + ui::MAX_OPTIONS
        );
        assert_eq!(note.icon, Icon::None);
    }
}
