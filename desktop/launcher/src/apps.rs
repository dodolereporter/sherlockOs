//! The installed applications, read by `mime-apps`: every desktop entry that is not hidden
//! from menus (`NoDisplay`), deleted (`Hidden`) or missing its program (`TryExec`).

use std::ffi::OsString;

use mime_apps::{Catalog, DesktopEntry, Dirs};
use sherlock_ui::launcher::text;

use crate::search::Haystack;
use crate::source::{Action, Command, Hit, Source};

/// The applications and what they are called, folded once.
#[derive(Debug, Clone, Default)]
pub struct Apps {
    catalog: Catalog,
    names: Vec<Haystack>,
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
        Self { catalog, names }
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
    fn search(&self, terms: &[String], limit: usize) -> Vec<Hit> {
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
                let (program, args) = self.catalog.command(app, &[])?;
                Some(Hit {
                    title: app.name.clone(),
                    subtitle: app.generic_name.clone(),
                    action: Action::Start(Command {
                        program,
                        args,
                        dir: app.working_dir.clone(),
                    }),
                })
            })
            .take(limit)
            .collect()
    }
}
