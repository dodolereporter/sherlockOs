//! The global menu rendered without a screen (lot D-36): Kate's menus after its name, « Fichier »
//! open with a submenu, « Affichage » with its check marks and radio dots, in both themes, on a
//! 1920 × 1080 screen and on a 1366 × 768 one, where the menus that do not fit go into « » ».
//! The bar and each popup are rendered apart, then laid on the desktop's color where the
//! compositor would put them: `global-menu-<width>[-light].png` and
//! `global-menu-view-<width>[-light].png` in `target/tmp`.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Mutex, PoisonError};

    use bar::app::{Bar, Message};
    use bar::dbusmenu::{Item, Toggle};
    use bar::global_menu::{self, MORE_ID};
    use bar::source::clock::LocalTime;
    use bar::source::workspaces::{AppMenu, Desktop};
    use iced::window::Id;
    use iced_test::simulator::Simulator;
    use sherlock_ui::color::{self, Theme};
    use sherlock_ui::{metrics, theme};

    /// A capture sets the active theme: one at a time.
    static THEME: Mutex<()> = Mutex::new(());

    fn entry(id: i32, label: &str) -> Item {
        Item {
            id,
            label: label.to_owned(),
            ..Item::default()
        }
    }

    fn keys(item: Item, keys: &str) -> Item {
        Item {
            keys: Some(keys.to_owned()),
            ..item
        }
    }

    fn separator(id: i32) -> Item {
        Item {
            separator: true,
            ..entry(id, "")
        }
    }

    fn menu(id: i32, label: &str, children: Vec<Item>) -> Item {
        Item {
            submenu: true,
            children,
            ..entry(id, label)
        }
    }

    /// Kate 25.04's menus, with the entries of « Fichier » and « Affichage » it exports.
    fn kate() -> Item {
        let file = menu(
            1,
            "Fichier",
            vec![
                keys(entry(2, "Nouveau"), "Ctrl+N"),
                keys(entry(3, "Nouvelle fenêtre"), "Ctrl+Maj+N"),
                separator(4),
                keys(entry(5, "Ouvrir..."), "Ctrl+O"),
                Item {
                    enabled: false,
                    ..menu(6, "Récemment ouvert(s)", vec![entry(9, "Aucun élément")])
                },
                keys(entry(192, "Ouvrir un dossier..."), "Ctrl+T, O"),
                separator(11),
                keys(entry(12, "Tout enregistrer"), "Ctrl+L"),
                entry(14, "Tout recharger"),
                separator(21),
                menu(
                    22,
                    "Actions sur les fichiers",
                    vec![
                        Item {
                            enabled: false,
                            ..entry(23, "Renommer...")
                        },
                        entry(25, "Comparer"),
                        separator(26),
                        entry(27, "Copier un emplacement"),
                    ],
                ),
                separator(15),
                keys(entry(16, "Fermer"), "Ctrl+W"),
                keys(entry(20, "Fermer la fenêtre"), "Ctrl+Maj+W"),
                keys(entry(31, "Quitter"), "Ctrl+Q"),
            ],
        );
        let check = |id, label, on| Item {
            toggle: Toggle::Check(on),
            ..entry(id, label)
        };
        let radio = |id, label, on| Item {
            toggle: Toggle::Radio(on),
            ..entry(id, label)
        };
        let view = menu(
            38,
            "Affichage",
            vec![
                check(40, "Afficher la barre latérale", true),
                check(41, "Afficher la barre d'état", true),
                check(42, "Afficher les onglets", false),
                separator(43),
                radio(44, "Retour à la ligne dynamique", true),
                radio(45, "Pas de retour à la ligne", false),
                separator(46),
                keys(entry(47, "Plein écran"), "Ctrl+Maj+F"),
            ],
        );
        let names = [
            (32, "Édition"),
            (37, "Sélection"),
            (68, "Aller"),
            (193, "Projets"),
            (239, "Client LSP"),
            (85, "Sessions"),
            (86, "Outils"),
            (92, "Configuration"),
            (116, "Aide"),
        ];
        let mut tops = vec![file, view];
        tops.extend(
            names
                .iter()
                .map(|(id, name)| menu(*id, name, vec![entry(id + 1000, "Annuler")])),
        );
        menu(0, "", tops)
    }

    fn address() -> AppMenu {
        AppMenu {
            service: String::from(":1.42"),
            path: String::from("/MenuBar/2"),
        }
    }

    /// The bar of a `width` × `height` screen, Kate active, its menus read.
    fn bar_with_kate(width: f32, height: f32) -> Bar {
        let mut state = Bar::default();
        let surface = Id::unique();
        let _ = state.update(Message::Surface(surface));
        let _ = state.update(Message::Output(surface, None, Some((width, height))));
        let _ = state.update(Message::Resized(
            surface,
            width - 2.0 * metrics::TOP_BAR_MARGIN_SIDE,
        ));
        let _ = state.update(Message::Clock(LocalTime {
            year: 2026,
            weekday: 2,
            day: 24,
            month: 11,
            hour: 21,
            minute: 46,
        }));
        let mut desktop = Desktop::placeholder(None, Some("Sans titre — Kate"));
        desktop.menu = Some(address());
        desktop.app_id = Some(String::from("org.kde.kate"));
        let _ = state.update(Message::Desktop(desktop));
        let _ = state.update(Message::AppName(
            String::from("org.kde.kate"),
            Some(String::from("Kate")),
        ));
        let _ = state.update(Message::AppMenuRead(address(), 0, Ok(kate())));
        state
    }

    /// Opens the menu `id` of the bar, as once the application has filled it.
    fn open(state: &mut Bar, id: i32) {
        let _ = state.update(Message::OpenAppMenu(id));
        if id != MORE_ID {
            let item = kate().find(id).cloned().unwrap();
            let _ = state.update(Message::AppMenuRead(address(), id, Ok(item)));
        }
        assert_eq!(state.global.open_top(), Some(id));
    }

    /// Renders `view` at `size`, returns the image.
    fn render(view: iced::Element<'_, Message>, size: (u32, u32), name: &str) -> image::RgbaImage {
        let mut ui = Simulator::with_size(bar::settings(), (size.0 as f32, size.1 as f32), view);
        let snapshot = ui.snapshot(&theme::theme()).unwrap();
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        let renderer = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
        let written = path.with_file_name(format!("{name}-{renderer}.png"));
        let _ = std::fs::remove_file(&written);
        let _ = snapshot.matches_image(&path).unwrap();
        image::open(&written).unwrap().to_rgba8()
    }

    /// Lays the bar and every open popup on the desktop of a `width` × `height` screen, at the
    /// scale the captures are rendered at, cut under the lowest menu.
    fn screen(state: &Bar, (width, height): (u32, u32), name: &str) -> PathBuf {
        let bar_size = (
            width - 2 * metrics::TOP_BAR_MARGIN_SIDE as u32,
            metrics::TOP_BAR_HEIGHT as u32,
        );
        let bar = render(state.view_bar(), bar_size, &format!("{name}-bar"));
        let scale = i64::from(bar.width() / bar_size.0);
        let desktop = color::palette().desktop;
        let mut out = image::RgbaImage::from_pixel(
            width * scale as u32,
            height * scale as u32,
            image::Rgba([desktop.r, desktop.g, desktop.b, 255]),
        );
        let (bar_x, bar_y) = (
            metrics::TOP_BAR_MARGIN_SIDE as i64,
            metrics::TOP_BAR_MARGIN_TOP as i64,
        );
        image::imageops::overlay(&mut out, &bar, bar_x * scale, bar_y * scale);
        // Where the compositor puts each popup: the menu of the bar under its button, each
        // submenu at the right of its parent, level with the entry that opens it (software:
        // no shadow, the gap below the bar).
        let placement = state.placement();
        let mut parent: Option<(i64, i64)> = None;
        let mut bottom = bar_y + metrics::TOP_BAR_HEIGHT as i64;
        for (level, shown) in state.global.levels.iter().enumerate() {
            let popup = render(
                state.view(shown.popup),
                shown.size,
                &format!("{name}-level-{level}"),
            );
            let (x, y) = match parent {
                None => {
                    let (button, _) =
                        global_menu::GlobalMenu::button(&placement, shown.item).unwrap();
                    let top = metrics::TOP_BAR_HEIGHT as i64;
                    (bar_x + button as i64, bar_y + top)
                }
                Some((x, y)) => {
                    let up = &state.global.levels[level - 1];
                    let entries = state.global.entries(up.item, &placement);
                    let index = entries.iter().position(|e| e.id == shown.item).unwrap();
                    let top = global_menu::entry_top(&entries, index);
                    let below_bar = if level == 1 {
                        metrics::TOP_BAR_MENU_GAP
                    } else {
                        0.0
                    };
                    (x + up.size.0 as i64 - 1, y + (below_bar + top) as i64)
                }
            };
            image::imageops::overlay(&mut out, &popup, x * scale, y * scale);
            bottom = bottom.max(y + i64::from(shown.size.1));
            parent = Some((x, y));
        }
        let cut = ((bottom + 24) * scale).min(i64::from(out.height())) as u32;
        let out = image::imageops::crop_imm(&out, 0, 0, out.width(), cut).to_image();
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        out.save(&path).unwrap();
        path
    }

    #[test]
    fn kates_menus_render_in_both_themes_at_two_sizes() {
        let _one = THEME.lock().unwrap_or_else(PoisonError::into_inner);
        for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
            color::set_active(which);
            for (width, height) in [(1920u32, 1080u32), (1366, 768)] {
                let mut state = bar_with_kate(width as f32, height as f32);
                let placement = state.placement();
                if width == 1920 {
                    assert_eq!(placement.menus.len(), 11, "every menu fits at 1920");
                    assert_eq!(placement.more, None);
                } else {
                    assert!(placement.more.is_some(), "« » » at 1366");
                }
                // « Fichier », the pointer on « Actions sur les fichiers »: its submenu opens.
                open(&mut state, 1);
                let placement = state.placement();
                let entries = state.global.entries(1, &placement);
                let actions = entries.iter().position(|e| e.id == 22).unwrap();
                let _ = state.update(Message::HoverEntry(0, actions));
                assert_eq!(state.global.levels.len(), 2);
                let _ = state.update(Message::HoverEntry(1, 1));
                let path = screen(
                    &state,
                    (width, height),
                    &format!("global-menu-{width}{suffix}"),
                );
                assert!(std::fs::metadata(&path).unwrap().len() > 10_000);
                // « Affichage », reached by Right from « Fichier »: checks and radio dots.
                let _ = state.update(Message::MenuKey(bar::menu::Key::Left));
                assert_eq!(state.global.levels.len(), 1, "Left closes the submenu");
                let _ = state.update(Message::HoverEntry(0, 0));
                let _ = state.update(Message::MenuKey(bar::menu::Key::Right));
                let view = kate().find(38).cloned().unwrap();
                let _ = state.update(Message::AppMenuRead(address(), 38, Ok(view)));
                assert_eq!(state.global.open_top(), Some(38));
                assert_eq!(
                    state.global.levels[0].cursor,
                    Some(0),
                    "first entry from the keys"
                );
                let name = format!("global-menu-view-{width}{suffix}");
                screen(&state, (width, height), &name);
                // On the small screen, « » » holds the menus that do not fit.
                if width == 1366 {
                    open(&mut state, MORE_ID);
                    let more = state.global.entries(MORE_ID, &placement);
                    assert_eq!(more.last().map(|i| i.label.as_str()), Some("Aide"));
                    let _ = state.update(Message::HoverEntry(0, 0));
                    let name = format!("global-menu-more-{width}{suffix}");
                    screen(&state, (width, height), &name);
                }
            }
        }
        color::set_active(Theme::Dark);
    }

    #[test]
    fn the_menus_follow_the_active_application() {
        let mut state = bar_with_kate(1920.0, 1080.0);
        open(&mut state, 1);
        assert!(state.app_menu_open());
        // Another window becomes active: Kate's menus close and the bar waits for the new ones.
        let other = AppMenu {
            service: String::from(":1.43"),
            path: String::from("/MenuBar/1"),
        };
        let mut desktop = Desktop::placeholder(None, Some("Konsole"));
        desktop.menu = Some(other.clone());
        let _ = state.update(Message::Desktop(desktop));
        assert!(!state.app_menu_open());
        assert_eq!(state.global.address.as_ref(), Some(&other));
        assert!(state.global.root.is_none());
        // Kate's late answer is not taken for Konsole's.
        let _ = state.update(Message::AppMenuRead(address(), 0, Ok(kate())));
        assert!(state.global.root.is_none());
        // A window without a menu: the name alone.
        let _ = state.update(Message::Desktop(Desktop::placeholder(None, Some("xterm"))));
        assert_eq!(state.global.address, None);
        assert_eq!(state.app_name().as_deref(), Some("xterm"));
        assert!(state.global.tops().is_empty());
    }

    #[test]
    fn moving_from_menu_to_menu_with_the_pointer_and_the_keys() {
        let mut state = bar_with_kate(1920.0, 1080.0);
        // Hovering a menu while none is open does nothing.
        let _ = state.update(Message::HoverAppMenu(38));
        assert!(!state.app_menu_open());
        open(&mut state, 1);
        // While one is open, the pointer on another opens it, as on a Mac.
        let _ = state.update(Message::HoverAppMenu(38));
        let view = kate().find(38).cloned().unwrap();
        let _ = state.update(Message::AppMenuRead(address(), 38, Ok(view)));
        assert_eq!(state.global.open_top(), Some(38));
        // Down walks the entries, past the separators.
        for expected in [0, 1, 2, 4] {
            let _ = state.update(Message::MenuKey(bar::menu::Key::Next));
            assert_eq!(state.global.levels[0].cursor, Some(expected));
        }
        // A second click on the open menu closes it; Escape closes too.
        let _ = state.update(Message::OpenAppMenu(38));
        assert!(!state.app_menu_open());
        open(&mut state, 1);
        let _ = state.update(Message::MenuKey(bar::menu::Key::Close));
        assert!(!state.app_menu_open());
        // The compositor closes the popups on a click elsewhere.
        open(&mut state, 1);
        let popup = state.global.levels[0].popup;
        let _ = state.update(Message::Closed(popup));
        assert!(!state.app_menu_open());
    }
}
