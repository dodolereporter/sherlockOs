//! A notification as the daemon keeps it, read from the arguments of `Notify`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use sherlock_ui::notifications as ui;
use zbus::zvariant::OwnedValue;

use crate::markup::{self, Span};

/// The key of the action a click on the bubble itself triggers.
pub const DEFAULT_ACTION: &str = "default";
/// Largest side of an `image-data` picture taken: more is refused, not scaled.
const MAX_PIXELS_SIDE: i32 = 1024;

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
}

/// The picture of a notification, from the `image-data`, `image-path` hints or `app_icon`.
#[derive(Debug, Clone, PartialEq)]
pub enum Icon {
    None,
    /// An icon name of the theme: not looked up yet (see `docs/notifications.md`).
    Name(String),
    File(PathBuf),
    /// Raw pixels, made RGBA.
    Pixels {
        width: u32,
        height: u32,
        rgba: Arc<Vec<u8>>,
    },
}

/// The arguments of `Notify`.
#[derive(Debug, Default)]
pub struct Request {
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
            .map(|[key, label]| (key.clone(), label.clone()))
            .collect();
        Self {
            id,
            app_name: request.app_name.trim().to_owned(),
            summary: request.summary.chars().take(ui::MAX_BODY_CHARS).collect(),
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

    /// The actions shown as buttons: all but the default one.
    pub fn buttons(&self) -> impl Iterator<Item = &(String, String)> {
        self.actions.iter().filter(|(key, _)| key != DEFAULT_ACTION)
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

/// An icon given as text: a `file://` URI, an absolute path or a theme name.
fn named(text: &str) -> Icon {
    let text = text.trim();
    match text.strip_prefix("file://").unwrap_or(text) {
        "" => Icon::None,
        path if path.starts_with('/') => Icon::File(PathBuf::from(path)),
        _ => Icon::Name(text.to_owned()),
    }
}

/// `(iiibiiay)`: width, height, row stride, alpha, bits per sample, channels, data. Only 8 bits
/// per sample, 3 or 4 channels, within `MAX_PIXELS_SIDE`; the rows are made RGBA.
fn pixels(value: &OwnedValue) -> Option<Icon> {
    type Raw = (i32, i32, i32, bool, i32, i32, Vec<u8>);
    let (width, height, stride, alpha, bits, channels, data) =
        Raw::try_from(value.try_clone().ok()?).ok()?;
    let size = 1..=MAX_PIXELS_SIDE;
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
        width: width.unsigned_abs(),
        height: height.unsigned_abs(),
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
        let path = Value::from("file:///tmp/a.png");
        let note = Notification::new(1, request(-1, vec![("image-path", path)]));
        assert_eq!(note.icon, Icon::File(PathBuf::from("/tmp/a.png")));
        assert_eq!(named("firefox"), Icon::Name(String::from("firefox")));
        assert_eq!(named(" "), Icon::None);
    }
}
