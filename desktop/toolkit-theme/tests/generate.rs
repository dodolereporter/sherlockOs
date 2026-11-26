//! The generated files are well formed and hold exactly the colors of the palettes.

use std::collections::BTreeMap;

use sherlock_ui::color::{Color, Palette, Theme};
use toolkit_theme::gtk::{self, Gtk};
use toolkit_theme::{css, qt, selection};

/// `unwrap` for these tests, whose helpers clippy reads as shipped code: a failure is a failed
/// test.
trait Must<T> {
    fn must(self) -> T;
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self) -> T {
        match self {
            Ok(value) => value,
            Err(error) => panic!("{error:?}"),
        }
    }
}

impl<T> Must<T> for Option<T> {
    fn must(self) -> T {
        match self {
            Some(value) => value,
            None => panic!("nothing where something was expected"),
        }
    }
}

/// Parses `#RRGGBB` and `rgba(r, g, b, a)`.
fn parse_color(text: &str) -> Color {
    if let Some(hex) = text.strip_prefix('#') {
        assert_eq!(hex.len(), 6, "{text}");
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).must();
        return Color::rgb(byte(0), byte(2), byte(4));
    }
    let inner = text
        .strip_prefix("rgba(")
        .and_then(|t| t.strip_suffix(')'))
        .must();
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    assert_eq!(parts.len(), 4, "{text}");
    Color::rgba(
        parts[0].parse().must(),
        parts[1].parse().must(),
        parts[2].parse().must(),
        parts[3].parse().must(),
    )
}

/// A small CSS reader, enough for what GTK accepts here: comments, `@import url("…");`,
/// `@define-color name value;`, and rules `selectors { property: value; … }`. Returns the
/// named colors; panics on anything else.
fn parse_css(text: &str) -> BTreeMap<String, Color> {
    let mut rest = String::new();
    let mut chars = text;
    while let Some(start) = chars.find("/*") {
        rest.push_str(&chars[..start]);
        let end = chars[start..].find("*/").must();
        chars = &chars[start + end + 2..];
    }
    rest.push_str(chars);
    let mut colors = BTreeMap::new();
    let mut text = rest.trim();
    while !text.is_empty() {
        if let Some(statement) = text.strip_prefix('@') {
            let end = statement.find(';').must();
            let (body, after) = (&statement[..end], &statement[end + 1..]);
            if let Some(url) = body.strip_prefix("import ") {
                assert!(url.starts_with("url(\"") && url.ends_with("\")"), "{url}");
            } else if let Some(define) = body.strip_prefix("define-color ") {
                let (name, value) = define.split_once(' ').must();
                assert!(
                    name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                    "{name}"
                );
                assert!(
                    colors
                        .insert(name.to_owned(), parse_color(value.trim()))
                        .is_none(),
                    "{name} twice"
                );
            } else {
                panic!("unknown statement @{body}");
            }
            text = after.trim_start();
        } else {
            let open = text.find('{').must();
            let close = text.find('}').must();
            assert!(open < close, "braces out of order");
            let selectors = text[..open].trim();
            assert!(
                !selectors.is_empty() && !selectors.contains(['{', '}', ';']),
                "{selectors}"
            );
            let block = &text[open + 1..close];
            assert!(!block.contains('{'), "nested block in {selectors}");
            for declaration in block.split(';').map(str::trim).filter(|d| !d.is_empty()) {
                let (property, value) = declaration.split_once(':').must();
                let property = property.trim();
                assert!(
                    property
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'-'),
                    "{property}"
                );
                assert!(!value.trim().is_empty(), "{declaration}");
                if let Some(name) = value.trim().strip_prefix('@') {
                    assert!(
                        colors.contains_key(name),
                        "{name} used before it is defined"
                    );
                }
            }
            text = text[close + 1..].trim_start();
        }
    }
    colors
}

/// Same channels, and the same opacity to the three decimals CSS is written with.
fn assert_same(found: &BTreeMap<String, Color>, wanted: &BTreeMap<String, Color>) {
    assert_eq!(
        found.keys().collect::<Vec<_>>(),
        wanted.keys().collect::<Vec<_>>()
    );
    for (name, w) in wanted {
        let f = found[name];
        assert_eq!((f.r, f.g, f.b), (w.r, w.g, w.b), "{name}");
        assert!((f.a - w.a).abs() < 0.0005, "{name}: {} for {}", f.a, w.a);
    }
}

/// The `:root` variables of the libadwaita file.
fn variables(text: &str) -> BTreeMap<String, Color> {
    let block = &text[text.find(":root {").must() + 7..text.rfind('}').must()];
    block
        .lines()
        .filter_map(|l| l.trim().strip_prefix("--"))
        .map(|l| {
            let (name, value) = l.split_once(':').must();
            (
                name.to_owned(),
                parse_color(value.trim().trim_end_matches(';')),
            )
        })
        .collect()
}

#[test]
fn the_stylesheets_parse_and_define_every_named_color() {
    for theme in Theme::ALL {
        let named: BTreeMap<String, Color> = gtk::named_colors(theme)
            .into_iter()
            .map(|(n, c)| (n.to_owned(), c))
            .collect();
        for version in [Gtk::V3, Gtk::V4] {
            let sheet = gtk::stylesheet(gtk::builtin_base(version, theme), theme);
            assert_same(&parse_css(&sheet), &named);
        }
        let adw = gtk::libadwaita(theme);
        let defines = parse_css(&adw[..adw.find(":root").must()]);
        assert_same(&defines, &named);
        let vars: BTreeMap<String, Color> = variables(&adw)
            .into_iter()
            .map(|(n, c)| (n.replace('-', "_"), c))
            .collect();
        assert_same(&vars, &named);
    }
}

#[test]
fn the_named_colors_are_those_of_the_palettes() {
    for (theme, p) in [
        (Theme::Dark, &Palette::DARK),
        (Theme::Light, &Palette::LIGHT),
    ] {
        let named: BTreeMap<_, _> = gtk::named_colors(theme).into_iter().collect();
        assert_eq!(named["window_bg_color"], p.window);
        assert_eq!(named["window_fg_color"], p.text_primary);
        assert_eq!(named["headerbar_bg_color"], p.title_bar);
        assert_eq!(named["sidebar_bg_color"], p.sidebar);
        assert_eq!(named["card_bg_color"], p.card);
        assert_eq!(named["accent_bg_color"], p.accent);
        assert_eq!(named["accent_color"], p.accent_text);
        assert_eq!(named["accent_fg_color"], p.text_on_accent);
        assert_eq!(named["destructive_bg_color"], p.salmon);
        assert_eq!(named["warning_color"], p.amber_text);
        assert_eq!(
            named["theme_selected_bg_color"],
            p.selected_row.over(p.window)
        );
        assert_eq!(named["insensitive_fg_color"], p.text_tertiary);
        // Written exactly: an opaque color as #RRGGBB, a veil with its opacity.
        assert_eq!(
            css(p.window),
            format!("#{:02X}{:02X}{:02X}", p.window.r, p.window.g, p.window.b)
        );
        let shadow = parse_color(&css(p.shadow));
        assert_eq!(
            (shadow.r, shadow.g, shadow.b),
            (p.shadow.r, p.shadow.g, p.shadow.b)
        );
        assert!((shadow.a - p.shadow.a).abs() < 0.0005);
    }
}

#[test]
fn the_qtct_palettes_have_every_role() {
    for theme in Theme::ALL {
        let p = theme.palette();
        for count in [qt::QT5_ROLES, qt::QT6_ROLES] {
            let text = qt::qtct_palette(theme, count);
            let mut lines = text.lines();
            assert_eq!(lines.next(), Some("[ColorScheme]"));
            for key in ["active_colors=", "disabled_colors=", "inactive_colors="] {
                let line = lines.next().must();
                let colors: Vec<&str> = line.strip_prefix(key).must().split(", ").collect();
                assert_eq!(colors.len(), count, "{key}");
                for c in &colors {
                    assert!(c.len() == 9 && c.starts_with("#ff"), "{c}");
                    assert!(c[1..].bytes().all(|b| b.is_ascii_hexdigit()), "{c}");
                }
                let role = |i: usize| parse_color(&format!("#{}", &colors[i][3..]).to_uppercase());
                // Window, Base, Button, AlternateBase.
                assert_eq!(role(10), p.sidebar);
                assert_eq!(role(9), p.window);
                assert_eq!(role(1), p.card);
                assert_eq!(role(16), p.card);
                let text_color = if key == "disabled_colors=" {
                    p.text_tertiary
                } else {
                    p.text_primary
                };
                assert_eq!(role(0), text_color);
                assert_eq!(role(6), text_color);
            }
            assert_eq!(lines.next(), None);
        }
    }
}

#[test]
fn the_kde_schemes_are_well_formed() {
    for theme in Theme::ALL {
        let p = theme.palette();
        let text = qt::kde_colors(theme);
        let mut groups: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut group = String::new();
        for line in text.lines().filter(|l| !l.is_empty()) {
            if line.starts_with('[') {
                assert!(line.ends_with(']'), "{line}");
                group = line.to_owned();
                assert!(
                    groups.insert(group.clone(), BTreeMap::new()).is_none(),
                    "{line} twice"
                );
            } else {
                let (key, value) = line.split_once('=').must();
                groups
                    .get_mut(&group)
                    .must()
                    .insert(key.to_owned(), value.to_owned());
            }
        }
        let rgb = |group: &str, key: &str| {
            let parts: Vec<u8> = groups[group][key]
                .split(',')
                .map(|v| v.parse().must())
                .collect();
            assert_eq!(parts.len(), 3, "{group} {key}");
            Color::rgb(parts[0], parts[1], parts[2])
        };
        for name in [
            "Button",
            "Complementary",
            "Header",
            "Selection",
            "Tooltip",
            "View",
            "Window",
        ] {
            let g = format!("[Colors:{name}]");
            assert_eq!(groups[&g].len(), 12, "{g}");
            for key in groups[&g].keys().cloned().collect::<Vec<_>>() {
                rgb(&g, &key);
            }
        }
        assert!(groups.contains_key("[Colors:Header][Inactive]"));
        assert_eq!(rgb("[Colors:View]", "BackgroundNormal"), p.window);
        assert_eq!(rgb("[Colors:Window]", "BackgroundNormal"), p.sidebar);
        assert_eq!(rgb("[Colors:Button]", "BackgroundNormal"), p.card);
        assert_eq!(rgb("[Colors:Header]", "BackgroundNormal"), p.title_bar);
        assert_eq!(rgb("[Colors:Selection]", "BackgroundNormal"), selection(p));
        assert_eq!(rgb("[Colors:View]", "ForegroundNormal"), p.text_primary);
        assert_eq!(rgb("[Colors:View]", "ForegroundNegative"), p.salmon_text);
        assert_eq!(rgb("[WM]", "activeBackground"), p.title_bar);
        let scheme = qt::kde_file_name(theme).trim_end_matches(".colors");
        assert_eq!(groups["[General]"]["ColorScheme"], scheme);
    }
}

#[test]
fn switching_the_qt_palette_keeps_the_other_settings() {
    let dark = std::path::Path::new("/t/qt6ct/dark.conf");
    let light = std::path::Path::new("/t/qt6ct/light.conf");
    let settings = format!("{}icon_theme=breeze\n", qt::qtct_settings(dark));
    let switched = qt::with_palette(&settings, light).must();
    assert_eq!(switched, settings.replace("dark.conf", "light.conf"));
    assert_eq!(qt::with_palette(&switched, light), None);
    // Settings without the key are written again in full.
    assert_eq!(
        qt::with_palette("[Fonts]\n", light),
        Some(qt::qtct_settings(light))
    );
}

#[test]
fn the_shown_names_are_those_of_sherlock_ui() {
    use sherlock_ui::strings::toolkit_theme as names;
    for (theme, shown) in [(Theme::Dark, names::DARK), (Theme::Light, names::LIGHT)] {
        let index = gtk::index(theme);
        assert!(index.contains(&format!("\nName={shown}\n")), "{index}");
        assert!(index.contains(&format!("\nComment={}\n", names::COMMENT)));
        assert!(index.contains(&format!(
            "\nGtkTheme={}\n",
            toolkit_theme::gtk_theme_name(theme)
        )));
        let kde = qt::kde_colors(theme);
        assert!(kde.contains(&format!("\nName={shown}\n")), "{kde}");
        for line in index.lines().chain(kde.lines()) {
            if line.starts_with("Name=") || line.starts_with("Comment=") {
                assert!(!line.contains("SherlockOS-"), "the technical name: {line}");
            }
        }
    }
}
