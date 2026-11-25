//! Contenu d'un dossier : lecture, type de chaque élément, tri par colonne.

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sherlock_ui::icon::Icon;
use sherlock_ui::texts::{self as all_texts, LocalTime, files as texts};

use crate::clock;

use crate::git::GitStatus;

/// Nature d'un élément. Un lien vers un dossier compte comme un dossier ; un lien cassé reste
/// un lien.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    Link,
}

/// Un élément d'un dossier, ou de la corbeille.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Nom affiché. Dans la corbeille, le nom d'origine.
    pub name: String,
    /// Chemin réel. Dans la corbeille, celui du fichier rangé dans `Trash/files`.
    pub path: PathBuf,
    pub kind: Kind,
    /// Taille en octets d'un fichier.
    pub size: u64,
    /// Nombre d'éléments d'un dossier, s'il se lit.
    pub children: Option<usize>,
    /// Date de modification. Dans la corbeille, date de suppression.
    pub modified: Option<SystemTime>,
    pub git: Option<GitStatus>,
    /// Dans la corbeille : chemin d'origine.
    pub origin: Option<PathBuf>,
    /// Textes de la ligne, préparés à la lecture : la vue ne fait ni `stat` ni calcul.
    pub labels: Labels,
}

/// Textes d'une ligne de la liste.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Labels {
    pub date: String,
    pub size: String,
    pub kind: String,
    /// Nom en minuscules, pour la recherche.
    pub lower: String,
}

impl Entry {
    /// Lit un élément sur le disque.
    pub fn read(path: &Path, show_hidden: bool) -> io::Result<Self> {
        let link = fs::symlink_metadata(path)?;
        let (kind, meta) = match fs::metadata(path) {
            Ok(meta) if meta.is_dir() => (Kind::Folder, meta),
            Ok(meta) => (Kind::File, meta),
            Err(_) => (Kind::Link, link),
        };
        let children = match kind {
            Kind::Folder => fs::read_dir(path).ok().map(|items| {
                items
                    .filter_map(Result::ok)
                    .filter(|item| show_hidden || !is_hidden(&item.file_name().to_string_lossy()))
                    .count()
            }),
            _ => None,
        };
        let mut entry = Self {
            name: path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into(),
            ),
            path: path.to_path_buf(),
            kind,
            size: meta.len(),
            children,
            modified: meta.modified().ok(),
            git: None,
            origin: None,
            labels: Labels::default(),
        };
        entry.relabel(clock::now());
        Ok(entry)
    }

    /// Prépare les textes de la ligne. `now` sert aux dates relatives (« hier »).
    pub fn relabel(&mut self, now: Option<LocalTime>) {
        self.labels = Labels {
            date: self
                .modified
                .and_then(clock::local)
                .zip(now)
                .map(|(when, now)| all_texts::short_date(when, now))
                .unwrap_or_default(),
            size: match self.kind {
                Kind::Folder => self.children.map(all_texts::items).unwrap_or_default(),
                Kind::File => all_texts::size(self.size),
                Kind::Link => String::new(),
            },
            kind: self.kind_label(),
            lower: self.name.to_lowercase(),
        };
    }

    /// Type affiché dans la colonne Type : « Dossier », « Markdown », « Verrou Cargo »…
    pub fn kind_label(&self) -> String {
        match self.kind {
            Kind::Folder => return texts::KIND_FOLDER.into(),
            Kind::Link => return texts::KIND_LINK.into(),
            Kind::File => {}
        }
        if let Some((_, label)) = texts::KINDS_BY_NAME.iter().find(|(n, _)| *n == self.name) {
            return (*label).into();
        }
        match self.extension() {
            Some(ext) => texts::KINDS_BY_EXTENSION
                .iter()
                .find(|(e, _)| *e == ext)
                .map_or_else(|| texts::kind_from_extension(&ext), |(_, l)| (*l).into()),
            None => texts::KIND_FILE.into(),
        }
    }

    /// Extension en minuscules, sans le point. Un fichier caché sans point de plus n'en a pas.
    pub fn extension(&self) -> Option<String> {
        Path::new(&self.name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
    }

    /// Une image que l'aperçu sait montrer.
    pub fn is_image(&self) -> bool {
        self.kind == Kind::File
            && matches!(self.extension().as_deref(), Some("png" | "jpg" | "jpeg"))
    }

    /// Icône de la ligne.
    pub fn icon(&self) -> Icon {
        match self.kind {
            Kind::Folder => Icon::Folder,
            _ if self.is_image() => Icon::Image,
            _ if self.name == "Dockerfile" || self.name.starts_with("compose.") => Icon::Cube,
            _ => Icon::Document,
        }
    }
}

/// Un nom caché commence par un point.
pub fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Lit un dossier, sans les éléments cachés si `show_hidden` est faux. Les éléments qui
/// disparaissent pendant la lecture sont ignorés.
pub fn read_dir(dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        if !show_hidden && is_hidden(&item.file_name().to_string_lossy()) {
            continue;
        }
        if let Ok(entry) = Entry::read(&item.path(), show_hidden) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// Colonne de tri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Modified,
    Size,
    Kind,
    Git,
}

/// Tri courant : une colonne et un sens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub column: Column,
    pub ascending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            column: Column::Name,
            ascending: true,
        }
    }
}

impl Sort {
    /// Un clic sur une colonne : la même inverse le sens, une autre trie en croissant.
    pub fn toggle(self, column: Column) -> Self {
        Self {
            column,
            ascending: self.column != column || !self.ascending,
        }
    }
}

/// Trie les éléments. Les dossiers restent devant, quel que soit le sens ; à égalité, le nom
/// départage.
pub fn sort(entries: &mut [Entry], sort: Sort) {
    entries.sort_by(|a, b| {
        let folders = (b.kind == Kind::Folder).cmp(&(a.kind == Kind::Folder));
        let by_column = match sort.column {
            Column::Name => Ordering::Equal,
            Column::Modified => a.modified.cmp(&b.modified),
            Column::Size => size_key(a).cmp(&size_key(b)),
            Column::Kind => a.labels.kind.cmp(&b.labels.kind),
            // Sans état git en dernier : `None` passe après les états, dans le sens croissant.
            Column::Git => match (a.git, b.git) {
                (Some(x), Some(y)) => x.cmp(&y),
                (x, y) => y.is_some().cmp(&x.is_some()),
            },
        };
        let by_column = by_column.then_with(|| natural(&a.name, &b.name));
        folders.then(if sort.ascending {
            by_column
        } else {
            by_column.reverse()
        })
    });
}

fn size_key(entry: &Entry) -> u64 {
    match entry.kind {
        Kind::Folder => entry.children.map_or(0, |n| n as u64),
        _ => entry.size,
    }
}

/// Ordre naturel, sans tenir compte de la casse : « image2 » passe avant « image10 ».
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(d) = it.next_if(char::is_ascii_digit) {
                        digits.push(d);
                    }
                    let trimmed = digits.trim_start_matches('0').to_owned();
                    (trimmed.len(), trimmed)
                };
                let order = number(&mut a).cmp(&number(&mut b));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn entry(name: &str, kind: Kind, size: u64, age: u64, git: Option<GitStatus>) -> Entry {
        let mut entry = Entry {
            name: name.into(),
            path: PathBuf::from(name),
            kind,
            size,
            children: (kind == Kind::Folder).then_some(size as usize),
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age)),
            git,
            origin: None,
            labels: Labels::default(),
        };
        entry.relabel(None);
        entry
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn sample() -> Vec<Entry> {
        vec![
            entry("README.md", Kind::File, 2_000, 30, None),
            entry("target", Kind::Folder, 9, 1, Some(GitStatus::Ignored)),
            entry("image10.png", Kind::File, 500, 20, Some(GitStatus::New)),
            entry("Cargo.toml", Kind::File, 800, 10, Some(GitStatus::Modified)),
            entry("src", Kind::Folder, 6, 5, Some(GitStatus::Modified)),
            entry("image2.png", Kind::File, 40_000, 40, None),
        ]
    }

    #[test]
    fn le_nom_trie_dans_l_ordre_naturel_dossiers_devant() {
        let mut entries = sample();
        sort(&mut entries, Sort::default());
        assert_eq!(
            names(&entries),
            [
                "src",
                "target",
                "Cargo.toml",
                "image2.png",
                "image10.png",
                "README.md"
            ]
        );
        sort(&mut entries, Sort::default().toggle(Column::Name));
        assert_eq!(
            names(&entries),
            [
                "target",
                "src",
                "README.md",
                "image10.png",
                "image2.png",
                "Cargo.toml"
            ]
        );
    }

    #[test]
    fn la_taille_la_date_le_type_et_git_trient_aussi() {
        let mut entries = sample();
        let by = |column| Sort::default().toggle(column);
        sort(&mut entries, by(Column::Size));
        assert_eq!(
            names(&entries),
            [
                "src",
                "target",
                "image10.png",
                "Cargo.toml",
                "README.md",
                "image2.png"
            ]
        );
        sort(&mut entries, by(Column::Modified));
        assert_eq!(
            names(&entries),
            [
                "src",
                "target",
                "image2.png",
                "README.md",
                "image10.png",
                "Cargo.toml"
            ]
        );
        sort(&mut entries, by(Column::Kind));
        assert_eq!(
            names(&entries),
            [
                "src",
                "target",
                "image2.png",
                "image10.png",
                "README.md",
                "Cargo.toml"
            ]
        );
        sort(&mut entries, by(Column::Git));
        assert_eq!(
            names(&entries),
            [
                "src",
                "target",
                "Cargo.toml",
                "image10.png",
                "image2.png",
                "README.md"
            ]
        );
    }

    #[test]
    fn le_type_vient_du_nom_puis_de_l_extension() {
        let file = |name| entry(name, Kind::File, 0, 0, None);
        assert_eq!(file("Cargo.lock").kind_label(), "Verrou Cargo");
        assert_eq!(file("notes.MD").kind_label(), "Markdown");
        assert_eq!(file("archive.xyz").kind_label(), "Fichier XYZ");
        assert_eq!(file(".bashrc").kind_label(), "Fichier");
        assert_eq!(
            entry("src", Kind::Folder, 0, 0, None).kind_label(),
            "Dossier"
        );
        assert!(file("photo.JPG").is_image());
    }
}
