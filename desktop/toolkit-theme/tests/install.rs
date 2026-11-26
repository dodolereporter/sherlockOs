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
    assert_eq!(
        apply::gsettings_keys(Theme::Light)[0],
        ("gtk-theme", "NOM_OS-light")
    );
    assert_eq!(
        apply::gsettings_keys(Theme::Dark)[1],
        ("color-scheme", "prefer-dark")
    );
    let _ = fs::remove_dir_all(&layout.home);
}
