//! `toolkit-theme`: the colors of the desktop for GTK, Qt and KDE applications.
//!
//! - `generate [--force] <folder>` writes every generated file under `<folder>`, laid out as
//!   in a home folder (`.config/…`, `.local/share/…`), to look at them; it overwrites nothing
//!   without `--force`;
//! - `install` puts them in the user's folders, itself included, backing up what was there;
//! - `uninstall` takes them away and puts the backups back;
//! - `apply [dark|light]` switches the session to a theme, that of the appearance file by
//!   default;
//! - `follow` does it at start, then at every change of the appearance file: the SherlockOS
//!   session runs it.
//!
//! See `docs/third-party-theming.md`.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;

use sherlock_ui::color::Theme;
use toolkit_theme::{apply, install};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: toolkit-theme generate [--force] <folder> | install | uninstall | apply [dark|light] | follow";

fn main() -> ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let layout = install::Layout::from_env()?;
    match args.as_slice() {
        ["generate", folder] => {
            install::generate(&PathBuf::from(folder), false)?;
            Ok(())
        }
        ["generate", "--force", folder] => {
            install::generate(&PathBuf::from(folder), true)?;
            Ok(())
        }
        ["install"] => {
            let mut files = install::files(&layout);
            let program = std::fs::read(std::env::current_exe()?)?;
            files.push(install::File {
                path: layout.program(),
                content: program,
                executable: true,
            });
            report(&install::install(&layout, &files, &install::stamp())?)
        }
        ["uninstall"] => report(&install::uninstall(&layout, &install::stamp())?),
        ["apply"] => {
            apply::apply(&layout, appearance_theme()?);
            Ok(())
        }
        ["apply", "dark"] => {
            apply::apply(&layout, Theme::Dark);
            Ok(())
        }
        ["apply", "light"] => {
            apply::apply(&layout, Theme::Light);
            Ok(())
        }
        ["follow"] => {
            let path = appearance_config::path().ok_or("no path for the appearance file")?;
            let (send, themes) = mpsc::channel();
            appearance_config::follow_themes(path, move |theme| send.send(theme).is_ok())?;
            for theme in themes {
                apply::apply(&layout, theme);
            }
            Err("the appearance file is no longer followed".into())
        }
        _ => Err(USAGE.into()),
    }
}

fn appearance_theme() -> Result<Theme, &'static str> {
    let path = appearance_config::path().ok_or("no path for the appearance file")?;
    Ok(appearance_config::theme_now(&path))
}

/// One line per action, on the standard output: the person who ran the command reads it.
fn report(actions: &[install::Action]) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = std::io::stdout().lock();
    for action in actions {
        match action {
            install::Action::Wrote(p) => writeln!(out, "wrote     {}", p.display())?,
            install::Action::Unchanged(p) => writeln!(out, "unchanged {}", p.display())?,
            install::Action::BackedUp { path, backup } => {
                writeln!(out, "kept      {} as {}", path.display(), backup.display())?;
            }
            install::Action::Removed(p) => writeln!(out, "removed   {}", p.display())?,
            install::Action::Restored { path, backup } => {
                writeln!(
                    out,
                    "restored  {} from {}",
                    path.display(),
                    backup.display()
                )?;
            }
            install::Action::BackupMissing(b) => {
                writeln!(out, "missing   {}: nothing to put back", b.display())?;
            }
        }
    }
    Ok(())
}
