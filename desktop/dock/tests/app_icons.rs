//! The dock with our application icons (batch D-03), found in the hicolor theme of sherlock-ui
//! through `icon-theme`, and Sherlock's tile: headless captures at rest, magnified, small and
//! in the light theme.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
    use std::time::Duration;

    use dock::app::{Dock, Sources};
    use dock::entry::Catalog;
    use dock::icons::IconLookup;
    use dock::items::Glyph;
    use dock::windows::{Placeholder, Recorder};
    use dock_config::DockConfig;
    use iced::widget::container;
    use iced::{Element, Length};
    use iced_test::simulator::Simulator;
    use sherlock_ui::app_icon::AppIcon;
    use sherlock_ui::color::{self, Theme};
    use sherlock_ui::theme;

    /// A capture sets the active theme: one at a time.
    static THEME: Mutex<()> = Mutex::new(());

    fn in_theme(which: Theme) -> MutexGuard<'static, ()> {
        let guard = THEME.lock().unwrap_or_else(PoisonError::into_inner);
        color::set_active(which);
        guard
    }

    fn manifest() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// Desktop entries for our applications: Evidence's own, and stand-ins for those that do not
    /// exist yet, each naming its icon. The applications without a chosen name get a neutral one.
    /// Written once, before any test reads them.
    fn applications() -> PathBuf {
        static DATA: OnceLock<PathBuf> = OnceLock::new();
        DATA.get_or_init(write_applications).clone()
    }

    fn write_applications() -> PathBuf {
        let data = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("dock-app-icons");
        let applications = data.join("applications");
        std::fs::create_dir_all(&applications).expect("folder");
        let evidence = manifest().join("../evidence/evidence.desktop");
        std::fs::copy(evidence, applications.join("evidence.desktop")).expect("Evidence's entry");
        let others = [
            (AppIcon::Inquiry, "Inquiry"),
            (AppIcon::ControlRoom, "Control Room"),
            (AppIcon::Lab, "Application d'exemple 1"),
            (AppIcon::Games, "Application d'exemple 2"),
            (AppIcon::Installer, "Application d'exemple 3"),
        ];
        for (icon, name) in others {
            let id = icon.id();
            let entry =
                format!("[Desktop Entry]\nType=Application\nName={name}\nExec={id}\nIcon={id}\n");
            std::fs::write(applications.join(format!("{id}.desktop")), entry).expect("entry");
        }
        data
    }

    /// The entries above, the theme of sherlock-ui, and the fixtures for the hicolor index that
    /// the system provides (the theme ships none).
    fn sources(config: DockConfig) -> Sources {
        let data_dirs = vec![
            applications(),
            manifest().join("../sherlock-ui"),
            manifest().join("fixtures/data"),
        ];
        let pinned = AppIcon::ALL
            .iter()
            .filter(|i| **i != AppIcon::Sherlock)
            .map(|i| i.id().to_owned())
            .collect();
        Sources {
            catalog: Catalog::load(&data_dirs),
            lookup: IconLookup::new(&data_dirs, None, None),
            config,
            config_path: None,
            pinned,
            data_dirs,
            home: None,
            gtk_theme: None,
            control: Arc::new(Recorder::default()),
        }
    }

    fn dock_with(config: DockConfig) -> Dock {
        let mut dock = Dock::new(sources(config));
        let Placeholder(windows) = Placeholder::new("evidence", Some("evidence"));
        let _ = dock.update(dock::app::Message::Windows(windows));
        dock
    }

    fn view(dock: &Dock) -> Simulator<'_, dock::app::Message> {
        let element: Element<'_, dock::app::Message> = container(dock.view_dock())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::desktop)
            .into();
        let size = (dock.surface_size.width, dock.surface_size.height);
        Simulator::with_size(dock::settings(), size, element)
    }

    /// Writes `target/tmp/<name>-tiny-skia.png`, returns its size in bytes.
    fn capture(mut ui: Simulator<'_, dock::app::Message>, name: &str) -> u64 {
        let snapshot = ui.snapshot(&theme::theme()).expect("snapshot");
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        let written = path.with_file_name(format!("{name}-tiny-skia.png"));
        let _ = std::fs::remove_file(&written);
        snapshot.matches_image(&path).expect("screenshot written");
        std::fs::metadata(&written).map(|m| m.len()).unwrap_or(0)
    }

    fn file_name(path: &Path) -> String {
        let parts: Vec<_> = path.iter().rev().take(3).collect();
        parts
            .into_iter()
            .rev()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }

    #[test]
    fn our_icons_are_found_in_the_theme() {
        let sources = sources(DockConfig::default());
        for icon in AppIcon::ALL {
            // The scalable SVG at every size the dock draws, from 32 at rest to 96 magnified 2 times.
            for size in [32, 52, 88, 192] {
                let found = sources.lookup.find(icon.id(), size).expect("icon found");
                assert_eq!(
                    file_name(&found),
                    format!("scalable/apps/{}.svg", icon.id())
                );
                assert!(found.starts_with(manifest().join("../sherlock-ui/icons/hicolor")));
            }
        }
        let dock = Dock::new(sources);
        let names: Vec<&str> = dock.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names[..3], ["Evidence", "Inquiry", "Control Room"]);
        assert_eq!(names[3], "Sherlock", "Sherlock in the middle");
        // Every icon is a file: none fell back to its initial.
        assert!(
            dock.items.iter().all(|i| matches!(i.glyph, Glyph::File(_))),
            "an icon was not found"
        );
    }

    #[test]
    fn the_dock_renders_our_icons() {
        let _theme = in_theme(Theme::Dark);
        let dock = dock_with(DockConfig::default());
        assert!(capture(view(&dock), "dock-app-icons") > 2_000);
        // Magnified over Sherlock.
        let mut dock = dock_with(DockConfig::default());
        let point = dock.layout(Duration::ZERO).icons[3].center();
        let _ = dock.hover(Some(point), Duration::ZERO);
        dock.frame(Duration::from_secs(2));
        let mut ui = view(&dock);
        assert!(ui.find("Sherlock").is_ok(), "label of the hovered icon");
        assert!(capture(ui, "dock-app-icons-hover") > 2_000);
        // The smallest dock the settings allow.
        let mut config = DockConfig::default();
        config.icon_size = sherlock_ui::dock::ICON_SIZE_MIN;
        let dock = dock_with(config);
        assert!(capture(view(&dock), "dock-app-icons-small") > 1_000);
    }

    #[test]
    fn the_dock_renders_our_icons_in_the_light_theme() {
        let _theme = in_theme(Theme::Light);
        let dock = dock_with(DockConfig::default());
        assert!(capture(view(&dock), "dock-app-icons-light") > 2_000);
        color::set_active(Theme::Dark);
    }
}
