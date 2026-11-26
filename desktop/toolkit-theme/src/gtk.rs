//! GTK: the named colors of libadwaita (which adw-gtk3 reads too) and of the GTK 3 themes, and
//! the stylesheet of the `SherlockOS-dark` and `SherlockOS-light` themes.

use std::fmt::Write as _;

use sherlock_ui::color::{Color, Theme};

use crate::{css, flat, popover, selection};

/// Every named color the generated files define, with its value. The libadwaita names come
/// first (`window_bg_color`…), then the older names of the GTK 3 themes (`theme_bg_color`…).
pub fn named_colors(theme: Theme) -> Vec<(&'static str, Color)> {
    let p = theme.palette();
    let border = flat(p.border, p.window);
    vec![
        ("accent_color", p.accent_text),
        ("accent_bg_color", p.accent),
        ("accent_fg_color", p.text_on_accent),
        ("destructive_color", p.salmon_text),
        ("destructive_bg_color", p.salmon),
        ("destructive_fg_color", p.text_on_accent),
        ("success_color", p.accent_text),
        ("success_bg_color", p.accent),
        ("success_fg_color", p.text_on_accent),
        ("warning_color", p.amber_text),
        ("warning_bg_color", p.amber),
        ("warning_fg_color", p.text_on_accent),
        ("error_color", p.salmon_text),
        ("error_bg_color", p.salmon),
        ("error_fg_color", p.text_on_accent),
        ("window_bg_color", p.window),
        ("window_fg_color", p.text_primary),
        ("view_bg_color", p.window),
        ("view_fg_color", p.text_primary),
        ("headerbar_bg_color", p.title_bar),
        ("headerbar_fg_color", p.text_primary),
        ("headerbar_border_color", p.text_primary),
        ("headerbar_backdrop_color", p.window),
        ("headerbar_shade_color", p.divider),
        ("sidebar_bg_color", p.sidebar),
        ("sidebar_fg_color", p.text_primary),
        ("sidebar_backdrop_color", p.sidebar),
        ("sidebar_shade_color", p.divider),
        ("secondary_sidebar_bg_color", p.sidebar),
        ("secondary_sidebar_fg_color", p.text_primary),
        ("card_bg_color", p.card),
        ("card_fg_color", p.text_primary),
        ("card_shade_color", p.card_border),
        ("dialog_bg_color", p.card),
        ("dialog_fg_color", p.text_primary),
        ("popover_bg_color", popover(p)),
        ("popover_fg_color", p.text_primary),
        ("popover_shade_color", p.menu_shadow),
        ("thumbnail_bg_color", p.card),
        ("thumbnail_fg_color", p.text_primary),
        ("shade_color", p.shadow),
        ("scrollbar_outline_color", p.border),
        ("theme_bg_color", p.window),
        ("theme_fg_color", p.text_primary),
        ("theme_base_color", p.window),
        ("theme_text_color", p.text_primary),
        ("theme_selected_bg_color", selection(p)),
        ("theme_selected_fg_color", p.text_primary),
        ("theme_unfocused_bg_color", p.window),
        ("theme_unfocused_fg_color", p.text_primary),
        ("theme_unfocused_base_color", p.window),
        ("theme_unfocused_text_color", p.text_primary),
        ("theme_unfocused_selected_bg_color", selection(p)),
        ("theme_unfocused_selected_fg_color", p.text_primary),
        ("insensitive_bg_color", p.window),
        ("insensitive_fg_color", p.text_tertiary),
        ("insensitive_base_color", p.window),
        ("borders", border),
        ("unfocused_borders", border),
        ("link_color", p.blue_light),
        // Not a GTK name: the hovered buttons of the rules below.
        ("button_hover_bg_color", flat(p.chip_hover, p.card)),
    ]
}

/// The named colors as libadwaita reads them: `@define-color`, then the CSS variables that
/// libadwaita 1.6 and later use. Both are needed: older versions know only the first.
pub fn libadwaita(theme: Theme) -> String {
    let colors = named_colors(theme);
    let mut out = String::new();
    for (name, color) in &colors {
        let _ = writeln!(out, "@define-color {name} {};", css(*color));
    }
    out.push_str(":root {\n");
    for (name, color) in &colors {
        let variable = name.replace('_', "-");
        let _ = writeln!(out, "  --{variable}: {};", css(*color));
    }
    out.push_str("}\n");
    out
}

/// Version of GTK a stylesheet is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gtk {
    V3,
    V4,
}

/// The theme a GTK 3 stylesheet starts from: adw-gtk3 when it is installed (`file://` URL of
/// its `gtk.css`), the theme built into GTK otherwise.
pub fn builtin_base(version: Gtk, theme: Theme) -> &'static str {
    match (version, theme) {
        (Gtk::V3, Theme::Dark) => "resource:///org/gtk/libgtk/theme/Adwaita/gtk-contained-dark.css",
        (Gtk::V3, Theme::Light) => "resource:///org/gtk/libgtk/theme/Adwaita/gtk-contained.css",
        (Gtk::V4, Theme::Dark) => "resource:///org/gtk/libgtk/theme/Default/Default-dark.css",
        (Gtk::V4, Theme::Light) => "resource:///org/gtk/libgtk/theme/Default/Default-light.css",
    }
}

/// The surfaces that the built-in themes paint with literal colors, painted again with the
/// named colors. adw-gtk3 reads the named colors itself: there these rules repeat it.
const RULES: &str = "
window.background, .background, dialog .dialog-vbox {
  background-color: @window_bg_color;
  color: @window_fg_color;
}
headerbar, .titlebar, actionbar, searchbar, .toolbar {
  background: @headerbar_bg_color;
  color: @headerbar_fg_color;
  border-color: @borders;
  box-shadow: none;
}
.view, textview text, treeview.view, iconview, list, listview, columnview, .content-view {
  background-color: @view_bg_color;
  color: @view_fg_color;
}
.sidebar, .navigation-sidebar, placessidebar list, stacksidebar list {
  background-color: @sidebar_bg_color;
  color: @sidebar_fg_color;
}
entry, spinbutton, button, combobox button, menubutton > button, dropdown > button {
  background: @card_bg_color;
  color: @card_fg_color;
  border-color: @borders;
  box-shadow: none;
  text-shadow: none;
}
button:hover {
  background: @button_hover_bg_color;
}
button.suggested-action, button.default {
  background: @accent_bg_color;
  color: @accent_fg_color;
  border-color: @accent_bg_color;
}
button.destructive-action {
  background: @destructive_bg_color;
  color: @destructive_fg_color;
  border-color: @destructive_bg_color;
}
button.flat, headerbar button, .titlebar button {
  background: transparent;
  border-color: transparent;
}
selection, row:selected, treeview.view:selected, iconview:selected, .view:selected {
  background-color: @theme_selected_bg_color;
  color: @theme_selected_fg_color;
}
switch:checked, progressbar progress, scale highlight, levelbar block.filled,
check:checked, radio:checked {
  background: @accent_bg_color;
  color: @accent_fg_color;
  border-color: @accent_bg_color;
}
popover, popover > contents, popover > arrow, menu, .menu, .context-menu, tooltip.background {
  background: @popover_bg_color;
  color: @popover_fg_color;
  border-color: @borders;
}
link, link:link, *:link {
  color: @link_color;
}
";

/// The stylesheet of a variant of the theme: the base theme, then the named colors, then the
/// surfaces painted again.
pub fn stylesheet(base: &str, theme: Theme) -> String {
    let mut out = format!(
        "/* SherlockOS {} theme, generated by toolkit-theme from the palettes of sherlock-ui. */\n",
        crate::variant(theme)
    );
    let _ = writeln!(out, "@import url(\"{base}\");\n");
    for (name, color) in named_colors(theme) {
        let _ = writeln!(out, "@define-color {name} {};", css(color));
    }
    out.push_str(RULES);
    out
}

/// `index.theme` of a theme folder.
pub fn index(theme: Theme) -> String {
    let folder = crate::gtk_theme_name(theme);
    let shown = crate::shown_name(theme);
    let comment = sherlock_ui::strings::toolkit_theme::COMMENT;
    format!(
        "[Desktop Entry]\nType=X-GNOME-Metatheme\nName={shown}\nComment={comment}\n\n\
         [X-GNOME-Metatheme]\nGtkTheme={folder}\n"
    )
}
