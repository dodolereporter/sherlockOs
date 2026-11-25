//! Corbeille de l'utilisateur, selon la spécification freedesktop « Trash » 1.0 : chaque
//! élément jeté va dans `$XDG_DATA_HOME/Trash/files`, accompagné d'une fiche
//! `Trash/info/<nom>.trashinfo` qui garde son chemin d'origine et sa date de suppression.
//! Plasma, GNOME et `gio trash` lisent la même corbeille.
//!
//! Seule la corbeille du dossier personnel est gérée : un fichier sur un autre disque est
//! refusé (`Error::OtherDisk`), sans copie silencieuse.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::clock;
use crate::ops::{Error, Result};

const INFO_EXTENSION: &str = ".trashinfo";

/// Un élément de la corbeille.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Nom de l'élément dans `Trash/files`, unique dans la corbeille.
    pub id: String,
    /// Chemin rangé dans `Trash/files`.
    pub path: PathBuf,
    /// Chemin d'origine, où le restaurer.
    pub original: PathBuf,
    /// Date de suppression : celle de la fiche.
    pub deleted: Option<SystemTime>,
}

/// La corbeille, posée dans un dossier `Trash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trash {
    files: PathBuf,
    info: PathBuf,
}

impl Trash {
    /// Corbeille dans `root` (le dossier `Trash` lui-même). Les tests en posent une dans un
    /// dossier temporaire.
    pub fn at(root: &Path) -> Self {
        Self {
            files: root.join("files"),
            info: root.join("info"),
        }
    }

    /// Corbeille de l'utilisateur : `$XDG_DATA_HOME/Trash`, ou `~/.local/share/Trash`.
    pub fn user() -> Option<Self> {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::home_dir().map(|home| home.join(".local/share")))?;
        Some(Self::at(&data.join("Trash")))
    }

    /// Dossier `files`, où vivent les éléments jetés.
    pub fn files_dir(&self) -> &Path {
        &self.files
    }

    fn ensure(&self) -> io::Result<()> {
        // La spécification demande des dossiers que seul l'utilisateur peut lire.
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(&self.files)?;
        builder.create(&self.info)
    }

    fn info_path(&self, id: &str) -> PathBuf {
        self.info.join(format!("{id}{INFO_EXTENSION}"))
    }

    /// Met un élément à la corbeille. Rend son identifiant.
    pub fn put(&self, path: &Path) -> Result<String> {
        self.ensure()?;
        let path = std::path::absolute(path)?;
        let meta = fs::symlink_metadata(&path)?;
        if meta.dev() != fs::metadata(&self.files)?.dev() {
            return Err(Error::OtherDisk);
        }
        let name = path
            .file_name()
            .ok_or(Error::InvalidName)?
            .to_string_lossy()
            .into_owned();
        let deleted = clock::now().map(clock::iso).unwrap_or_default();
        let record = format!(
            "[Trash Info]\nPath={}\nDeletionDate={deleted}\n",
            encode(&path)
        );
        // La fiche est créée d'abord, en exclusivité : c'est elle qui réserve le nom.
        for n in 1usize.. {
            let id = match n {
                1 => name.clone(),
                n => format!("{name}.{n}"),
            };
            let info = self.info_path(&id);
            let mut file = match fs::File::options().write(true).create_new(true).open(&info) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            };
            let target = self.files.join(&id);
            if fs::symlink_metadata(&target).is_ok() {
                // Un élément sans fiche occupe ce nom : on garde la fiche réservée pour ne
                // pas le reprendre, comme le fait gio.
                continue;
            }
            let moved = file
                .write_all(record.as_bytes())
                .and_then(|()| fs::rename(&path, &target));
            if let Err(error) = moved {
                let _ = fs::remove_file(&info);
                return Err(match error.kind() {
                    io::ErrorKind::CrossesDevices => Error::OtherDisk,
                    _ => error.into(),
                });
            }
            return Ok(id);
        }
        Err(Error::InvalidName)
    }

    /// Les éléments de la corbeille. Une fiche sans élément, ou l'inverse, est ignorée.
    pub fn list(&self) -> io::Result<Vec<Item>> {
        let entries = match fs::read_dir(&self.info) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut items = Vec::new();
        for entry in entries.filter_map(io::Result::ok) {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = file_name.strip_suffix(INFO_EXTENSION) else {
                continue;
            };
            let path = self.files.join(id);
            if fs::symlink_metadata(&path).is_err() {
                continue;
            }
            let Some(original) = fs::read_to_string(entry.path())
                .ok()
                .and_then(|s| parse(&s))
            else {
                continue;
            };
            items.push(Item {
                id: id.to_owned(),
                path,
                original,
                deleted: entry.metadata().and_then(|m| m.modified()).ok(),
            });
        }
        Ok(items)
    }

    /// Remet un élément à sa place d'origine, en recréant ses dossiers parents s'il le faut.
    /// Refuse si un autre élément y a pris sa place.
    pub fn restore(&self, id: &str) -> Result<PathBuf> {
        let info = self.info_path(id);
        let original = fs::read_to_string(&info)
            .ok()
            .and_then(|s| parse(&s))
            .ok_or(Error::NotInTrash)?;
        let path = self.files.join(id);
        if fs::symlink_metadata(&path).is_err() {
            return Err(Error::NotInTrash);
        }
        if let Some(parent) = original.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::ops::move_path(&path, &original)?;
        fs::remove_file(&info)?;
        Ok(original)
    }

    /// Vide la corbeille pour de bon. Rend le nombre d'éléments effacés.
    pub fn empty(&self) -> Result<usize> {
        let mut count = 0;
        if let Ok(entries) = fs::read_dir(&self.files) {
            for entry in entries {
                let path = entry?.path();
                if fs::symlink_metadata(&path)?.is_dir() {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_file(&path)?;
                }
                count += 1;
            }
        }
        if let Ok(entries) = fs::read_dir(&self.info) {
            for entry in entries {
                fs::remove_file(entry?.path())?;
            }
        }
        Ok(count)
    }
}

/// Encode un chemin comme la spécification le demande (RFC 2396) : tout octet hors des
/// caractères sûrs devient `%XX`.
pub fn encode(path: &Path) -> String {
    let mut out = String::new();
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()/".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Décode un chemin encodé par `encode`.
pub fn decode(text: &str) -> Option<PathBuf> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(OsString::from_vec(out)))
}

/// Lit le chemin d'origine d'une fiche `.trashinfo`.
fn parse(record: &str) -> Option<PathBuf> {
    let mut in_group = false;
    for line in record.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Trash Info]";
        } else if in_group && let Some(path) = line.strip_prefix("Path=") {
            return decode(path).filter(|p| p.is_absolute());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn setup(name: &str) -> (TempDir, Trash) {
        let tmp = TempDir::new(name);
        let trash = Trash::at(&tmp.path().join("donnees/Trash"));
        (tmp, trash)
    }

    #[test]
    fn jeter_puis_restaurer_rend_le_fichier_a_sa_place() {
        let (tmp, trash) = setup("corbeille");
        let file = tmp.path().join("dossier/été 2026.txt");
        fs::create_dir_all(file.parent().expect("parent")).expect("dossier");
        fs::write(&file, "souvenirs").expect("écriture");

        let id = trash.put(&file).expect("mis à la corbeille");
        assert!(!file.exists());
        let info = fs::read_to_string(trash.info_path(&id)).expect("fiche");
        assert!(info.starts_with("[Trash Info]\nPath="));
        assert!(
            info.contains("/dossier/%C3%A9t%C3%A9%202026.txt\n"),
            "{info}"
        );
        assert!(info.contains("\nDeletionDate=2"));

        let items = trash.list().expect("liste");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].original, file);
        assert!(items[0].deleted.is_some());

        // Le dossier d'origine a disparu entre-temps : il est recréé.
        fs::remove_dir(file.parent().expect("parent")).expect("dossier vide");
        assert_eq!(trash.restore(&id).expect("restauré"), file);
        assert_eq!(fs::read_to_string(&file).expect("lecture"), "souvenirs");
        assert!(trash.list().expect("liste").is_empty());
        assert!(matches!(trash.restore(&id), Err(Error::NotInTrash)));
    }

    #[test]
    fn deux_elements_du_meme_nom_cohabitent() {
        let (tmp, trash) = setup("homonymes");
        let file = tmp.path().join("a.txt");
        fs::write(&file, "un").expect("écriture");
        let first = trash.put(&file).expect("premier");
        fs::write(&file, "deux").expect("écriture");
        let second = trash.put(&file).expect("second");
        assert_ne!(first, second);
        assert_eq!(trash.list().expect("liste").len(), 2);

        // La place est reprise : la restauration refuse d'écraser.
        fs::write(&file, "trois").expect("écriture");
        assert!(matches!(trash.restore(&first), Err(Error::Exists(_))));
        assert_eq!(fs::read_to_string(&file).expect("lecture"), "trois");
    }

    #[test]
    fn vider_efface_tout_et_seulement_la_corbeille() {
        let (tmp, trash) = setup("vider");
        let dir = tmp.path().join("projet");
        fs::create_dir_all(dir.join("src")).expect("dossier");
        fs::write(dir.join("src/main.rs"), "").expect("écriture");
        let keep = tmp.path().join("garde.txt");
        fs::write(&keep, "").expect("écriture");
        trash.put(&dir).expect("dossier jeté");

        assert_eq!(trash.empty().expect("vidée"), 1);
        assert!(trash.list().expect("liste").is_empty());
        assert!(keep.exists());
        assert_eq!(fs::read_dir(trash.files_dir()).expect("files").count(), 0);
    }

    #[test]
    fn l_encodage_fait_l_aller_retour() {
        let path = Path::new("/home/dodo/Téléchargements/a b%c#.txt");
        let encoded = encode(path);
        assert!(!encoded.contains(' ') && !encoded.contains('#'));
        assert_eq!(decode(&encoded).as_deref(), Some(path));
        assert_eq!(decode("/a%2"), None);
        assert_eq!(parse("[Trash Info]\nPath=relatif\n"), None);
    }
}
