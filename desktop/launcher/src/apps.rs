//! The installed applications, read by `mime-apps`: every desktop entry that is not hidden
//! from menus (`NoDisplay`), deleted (`Hidden`) or missing its program (`TryExec`). Their icons
//! come from `icon-theme`, looked up the first time an application is shown.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Mutex;

use icon_theme::Lookup;
use mime_apps::{Catalog, DesktopEntry, Dirs};
use sherlock_ui::launcher::{self as look, text};

use crate::search::Haystack;
use crate::source::{Action, Command, Hit, Query, Source, Tile};

/// The applications and what they are called, folded once.
#[derive(Debug, Default)]
pub struct Apps {
    catalog: Catalog,
    names: Vec<Haystack>,
    /// Where the icons are; none: every application shows its initial.
    icons: Option<Lookup>,
    /// Side the icons are drawn at, and the scale they are looked up for.
    icon_size: (u32, u32),
    /// Icons already looked up, by icon name.
    found: Mutex<HashMap<String, Option<PathBuf>>>,
}

impl Apps {
    /// Reads the applications under `dirs`, names in `locales` when they have one.
    /// `Terminal=true` entries run in `terminal`, and are left out without one.
    pub fn load(dirs: &Dirs, locales: &[String], terminal: Option<Vec<OsString>>) -> Self {
        Self::new(mime_apps::read_applications(dirs, locales), terminal)
    }

    pub fn new(entries: Vec<DesktopEntry>, terminal: Option<Vec<OsString>>) -> Self {
        let applications: Vec<DesktopEntry> = entries
            .into_iter()
            .filter(|app| !app.no_display && (!app.terminal || terminal.is_some()))
            .collect();
        let names = applications
            .iter()
            .map(|app| Haystack::new(&app.name, app.generic_name.as_deref(), &app.keywords))
            .collect();
        let catalog = Catalog {
            applications,
            terminal,
            ..Catalog::default()
        };
        Self {
            catalog,
            names,
            icons: None,
            icon_size: (look::APP_ICON as u32, look::ICON_SCALE),
            found: Mutex::default(),
        }
    }

    /// Shows the icons `lookup` finds.
    pub fn with_icons(mut self, lookup: Lookup) -> Self {
        self.icons = Some(lookup);
        self
    }

    /// Looks the icons up for `size` at `scale`: the applications view draws them larger.
    pub fn with_icon_size(mut self, size: u32, scale: u32) -> Self {
        self.icon_size = (size, scale);
        self
    }

    /// The icon file of `app`, looked up once.
    pub fn icon(&self, app: &DesktopEntry) -> Option<PathBuf> {
        let (lookup, name) = (self.icons.as_ref()?, app.icon.as_deref()?);
        let mut found = self.found.lock().ok()?;
        let (size, scale) = self.icon_size;
        found
            .entry(name.to_owned())
            .or_insert_with(|| lookup.find(name, size, scale))
            .clone()
    }

    /// Every application, with what it is called, in the order of the files.
    pub fn entries(&self) -> impl Iterator<Item = (&DesktopEntry, &Haystack)> {
        self.catalog.applications.iter().zip(&self.names)
    }

    /// What starts `app`, without a shell: its `Exec` (in a terminal for `Terminal=true`), in
    /// its `Path`.
    pub fn start(&self, app: &DesktopEntry) -> Option<Command> {
        let (program, args) = self.catalog.command(app, &[])?;
        Some(Command {
            program,
            args,
            dir: app.working_dir.clone(),
        })
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

impl Source for Apps {
    fn section(&self) -> &'static str {
        text::APPLICATIONS
    }

    /// Best score first; then by folded name, then by desktop id: the order never depends on
    /// the order of the files.
    fn search(&self, query: &Query, limit: usize) -> Vec<Hit> {
        let terms = &query.terms;
        let mut found: Vec<(u32, &Haystack, &DesktopEntry)> = self
            .names
            .iter()
            .zip(&self.catalog.applications)
            .filter_map(|(name, app)| Some((name.score(terms)?, name, app)))
            .collect();
        found.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.name().cmp(b.1.name()))
                .then_with(|| a.2.id.cmp(&b.2.id))
        });
        found
            .into_iter()
            .filter_map(|(_, _, app)| {
                Some(Hit {
                    title: app.name.clone(),
                    subtitle: app.generic_name.clone(),
                    tile: Tile::App(self.icon(app)),
                    app: Some(
                        app.id
                            .strip_suffix(".desktop")
                            .unwrap_or(&app.id)
                            .to_owned(),
                    ),
                    action: Action::Start(self.start(app)?),
                })
            })
            .take(limit)
            .collect()
    }
}
