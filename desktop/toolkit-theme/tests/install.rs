//! Installing in a temporary home backs up what was there, and removing puts it back.

use std::fs;
use std::path::{Path, PathBuf};

use sherlock_ui::color::Theme;
use toolkit_theme::apply;
use toolkit_theme::install::{self, Action, File, Layout, Qt};

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

/// A fresh temporary home, its folders as `Layout` sees them.
fn home(test: &str) -> Layout {
    let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("home-{test}"));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).must();
    Layout {
        config: home.join(".config"),
        data: home.join(".local/share"),
        home,
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().must()).must();
    fs::write(path, text).must();
}

/// Every file under `dir`, with its content, sorted.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).must() {
            let path = entry.must().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push((path.clone(), fs::read(&path).must()));
            }
        }
    }
    files.sort();
    files
}

/// The user's own files before any installation: qt6ct settings, a GTK theme folder and a KDE
/// scheme of their own.
fn users_files(layout: &Layout) {
    write(
        &layout.qtct_settings(Qt::V6),
        "[Appearance]\nstyle=Breeze\n",
    );
    write(
        &layout.data.join("color-schemes/Mine.colors"),
        "[General]\nName=Mine\n",
    );
    write(
        &layout.config.join("gtk-3.0/gtk.css"),
        "@import 'colors.css';\n",
    );
}

#[test]
fn install_backs_up_and_uninstall_restores() {
    let layout = home("roundtrip");
    users_files(&layout);
    let before = tree(&layout.home);

    let files = install::files(&layout);
    let actions = install::install(&layout, &files, "20261126-100000").must();
    let qt6 = layout.qtct_settings(Qt::V6);
    let backup = qt6.with_file_name("qt6ct.conf.bak-20261126-100000");
    assert!(actions.contains(&Action::BackedUp {
        path: qt6.clone(),
        backup: backup.clone()
    }));
    assert_eq!(
        fs::read_to_string(&backup).must(),
        "[Appearance]\nstyle=Breeze\n"
    );
    for file in &files {
        assert_eq!(
            fs::read(&file.path).must(),
            file.content,
            "{}",
            file.path.display()
        );
    }
    // Plasma's GTK files are not among ours.
    assert!(
        files
            .iter()
            .all(|f| !f.path.starts_with(layout.config.join("gtk-3.0")))
    );
    assert!(
        files
            .iter()
            .all(|f| !f.path.starts_with(layout.config.join("gtk-4.0")))
    );
    assert_eq!(
        fs::read_to_string(layout.config.join("gtk-3.0/gtk.css")).must(),
        "@import 'colors.css';\n"
    );

    // The session switches the palette: the file stays ours.
    apply::qt(&layout, Theme::Light).must();
    assert!(fs::read_to_string(&qt6).must().contains("light.conf"));
    // Installing again keeps it, and keeps the palette of the moment.
    let again = install::install(&layout, &files, "20261126-100500").must();
    assert!(
        again.iter().all(|a| matches!(a, Action::Unchanged(_))),
        "{again:?}"
    );
    assert!(fs::read_to_string(&qt6).must().contains("light.conf"));

    let actions = install::uninstall(&layout, "20261126-110000").must();
    assert!(actions.contains(&Action::Restored {
        path: qt6.clone(),
        backup
    }));
    assert_eq!(tree(&layout.home), before);
    assert!(!layout.data.join("nom-os").exists());
    assert!(!layout.data.join("themes").join("NOM_OS-dark").exists());
    let _ = fs::remove_dir_all(&layout.home);
}

#[test]
fn installing_again_keeps_the_first_backup() {
    let layout = home("again");
    users_files(&layout);
    let before = tree(&layout.home);
    let files = install::files(&layout);
    install::install(&layout, &files, "20261126-100000").must();
    let again = install::install(&layout, &files, "20261126-100500").must();
    assert!(
        again.iter().all(|a| matches!(a, Action::Unchanged(_))),
        "{again:?}"
    );
    install::uninstall(&layout, "20261126-110000").must();
    assert_eq!(tree(&layout.home), before);
    let _ = fs::remove_dir_all(&layout.home);
}

#[test]
fn a_file_changed_since_is_never_lost() {
    let layout = home("changed");
    users_files(&layout);
    let files = install::files(&layout);
    install::install(&layout, &files, "20261126-100000").must();
    let qt6 = layout.qtct_settings(Qt::V6);
    let changed = "[Appearance]\nstyle=Fusion\n# changed by hand\n";
    fs::write(&qt6, changed).must();

    // Installing again over it keeps the change aside.
    let actions = install::install(&layout, &files, "20261126-100500").must();
    let aside = qt6.with_file_name("qt6ct.conf.bak-20261126-100500");
    assert!(actions.contains(&Action::BackedUp {
        path: qt6.clone(),
        backup: aside.clone()
    }));
    assert_eq!(fs::read_to_string(&aside).must(), changed);

    // Changed again, then removed: kept, and the original comes back.
    fs::write(&qt6, changed).must();
    let actions = install::uninstall(&layout, "20261126-110000").must();
    let kept = qt6.with_file_name("qt6ct.conf.bak-20261126-110000");
    assert!(actions.contains(&Action::BackedUp {
        path: qt6.clone(),
        backup: kept.clone()
    }));
    assert_eq!(fs::read_to_string(&kept).must(), changed);
    assert_eq!(
        fs::read_to_string(&qt6).must(),
        "[Appearance]\nstyle=Breeze\n"
    );
    let _ = fs::remove_dir_all(&layout.home);
}

#[test]
fn a_taken_backup_name_gets_a_number() {
    let layout = home("taken");
    let path = layout.config.join("qt5ct/qt5ct.conf");
    write(&path, "first\n");
    write(
        &path.with_file_name("qt5ct.conf.bak-20261126-100000"),
        "older\n",
    );
    let file = File {
        path: path.clone(),
        content: b"ours\n".to_vec(),
        executable: true,
    };
    install::install(&layout, &[file], "20261126-100000").must();
    let numbered = path.with_file_name("qt5ct.conf.bak-20261126-100000-2");
    assert_eq!(fs::read_to_string(&numbered).must(), "first\n");
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(
        fs::metadata(&path).must().permissions().mode() & 0o777,
        0o755
    );
    install::uninstall(&layout, "20261126-110000").must();
    assert_eq!(fs::read_to_string(&path).must(), "first\n");
    let older = path.with_file_name("qt5ct.conf.bak-20261126-100000");
    assert_eq!(fs::read_to_string(older).must(), "older\n");
    let _ = fs::remove_dir_all(&layout.home);
}

#[test]
fn the_session_alone_touches_the_gtk_settings() {
    let layout = home("session");
    let ours = layout.dconf_profile();
    assert!(apply::in_session(&layout, Some(ours.as_os_str())));
    assert!(!apply::in_session(&layout, None));
    assert!(!apply::in_session(&layout, Some("user".as_ref())));
    // A qt6ct settings file that install did not write is left alone.
    let qt6 = layout.qtct_settings(Qt::V6);
    write(&qt6, "[Appearance]\ncolor_scheme_path=/elsewhere.conf\n");
    assert!(apply::qt(&layout, Theme::Light).must().is_empty());
    let pairs = |theme, size| {
        apply::gsettings_keys(theme, size)
            .map(|(key, value)| format!("{key}={value}"))
            .join(" ")
    };
    assert_eq!(
        pairs(Theme::Light, 24),
        "gtk-theme=NOM_OS-light color-scheme=default cursor-theme=nom-os-light cursor-size=24"
    );
    assert_eq!(
        pairs(Theme::Dark, 48),
        "gtk-theme=NOM_OS-dark color-scheme=prefer-dark cursor-theme=nom-os cursor-size=48"
    );
    let _ = fs::remove_dir_all(&layout.home);
}

#[test]
fn the_cursor_size_comes_from_xcursor_size_or_is_24() {
    assert_eq!(apply::cursor_size(None), 24);
    assert_eq!(apply::cursor_size(Some("32".as_ref())), 32);
    assert_eq!(apply::cursor_size(Some(" 48 ".as_ref())), 48);
    for wrong in ["", "0", "-8", "big", "24px"] {
        assert_eq!(apply::cursor_size(Some(wrong.as_ref())), 24, "{wrong:?}");
    }
}

#[test]
fn a_link_that_leads_nowhere_comes_back() {
    let layout = home("dangling");
    let qt6 = layout.qtct_settings(Qt::V6);
    let target = Path::new("/nonexistent/qt6ct.conf");
    fs::create_dir_all(qt6.parent().must()).must();
    std::os::unix::fs::symlink(target, &qt6).must();
    // An empty folder of the user's, there before: it stays.
    fs::create_dir_all(layout.data.join("themes")).must();

    let files = install::files(&layout);
    let actions = install::install(&layout, &files, "20261126-100000").must();
    let backup = qt6.with_file_name("qt6ct.conf.bak-20261126-100000");
    assert!(actions.contains(&Action::BackedUp {
        path: qt6.clone(),
        backup: backup.clone()
    }));
    assert_eq!(fs::read_link(&backup).must(), target);

    install::uninstall(&layout, "20261126-110000").must();
    assert_eq!(fs::read_link(&qt6).must(), target);
    assert!(backup.symlink_metadata().is_err());
    // The folders installing created are gone, the others stay.
    assert!(!layout.config.join("qt5ct").exists());
    assert!(!layout.data.join("nom-os").exists());
    assert!(layout.data.join("themes").is_dir());
    assert!(qt6.parent().must().is_dir());
    let _ = fs::remove_dir_all(&layout.home);
}

#[test]
fn a_removal_stopped_halfway_goes_on_when_run_again() {
    use std::os::unix::fs::PermissionsExt as _;
    let layout = home("halfway");
    write(&layout.qtct_settings(Qt::V5), "mine5\n");
    write(&layout.qtct_settings(Qt::V6), "mine6\n");
    let before = tree(&layout.home);
    install::install(&layout, &install::files(&layout), "20261126-100000").must();

    // qt5ct comes last: its folder read only stops the removal there, qt6ct already restored.
    let qt5 = layout.config.join("qt5ct");
    fs::set_permissions(&qt5, fs::Permissions::from_mode(0o555)).must();
    let probe = qt5.join("probe");
    // Run as root, the folder stays writable and nothing stops.
    let blocked = fs::write(&probe, "").is_err();
    let _ = fs::remove_file(&probe);
    let first = install::uninstall(&layout, "20261126-110000");
    fs::set_permissions(&qt5, fs::Permissions::from_mode(0o755)).must();
    if blocked {
        assert!(first.is_err(), "{first:?}");
        assert_eq!(
            fs::read_to_string(layout.qtct_settings(Qt::V6)).must(),
            "mine6\n"
        );
    }

    install::uninstall(&layout, "20261126-110500").must();
    assert_eq!(tree(&layout.home), before);
    assert!(!layout.home.join(".local").exists());
    let _ = fs::remove_dir_all(&layout.home);
}

#[test]
fn a_backup_is_listed_before_the_file_is_written() {
    let layout = home("listed");
    let qt6 = layout.qtct_settings(Qt::V6);
    write(&qt6, "mine6\n");
    // A folder where the temporary file goes: writing ours fails after the backup.
    let blocker = qt6.with_file_name("qt6ct.conf.toolkit-theme-new");
    fs::create_dir_all(&blocker).must();
    let file = File {
        path: qt6.clone(),
        content: b"ours\n".to_vec(),
        executable: false,
    };
    assert!(install::install(&layout, &[file], "20261126-100000").is_err());
    assert!(!qt6.exists());

    fs::remove_dir(&blocker).must();
    install::uninstall(&layout, "20261126-110000").must();
    assert_eq!(fs::read_to_string(&qt6).must(), "mine6\n");
    let _ = fs::remove_dir_all(&layout.home);
}

#[test]
fn generate_overwrites_nothing_without_force() {
    let layout = home("generate");
    let root = layout.home.join("out");
    let written = install::generate(&root, false).must();
    let first = written.first().must();
    fs::write(first, "mine\n").must();

    let refused = install::generate(&root, false);
    assert!(
        matches!(&refused, Err(install::Error::Exists(p)) if p == first),
        "{refused:?}"
    );
    assert_eq!(fs::read_to_string(first).must(), "mine\n");

    install::generate(&root, true).must();
    assert_ne!(fs::read_to_string(first).must(), "mine\n");
    let _ = fs::remove_dir_all(&layout.home);
}
