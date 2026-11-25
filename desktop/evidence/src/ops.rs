//! Opérations sur les fichiers : copier, déplacer, renommer, dupliquer, nouveau dossier.
//!
//! Rien n'écrase jamais un fichier existant. Une copie dont le nom est pris devient
//! « nom (copie) » ; un déplacement ou un renommage vers un nom pris est refusé.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sherlock_ui::texts::evidence as texts;

/// Ce qui peut empêcher une opération. Le message s'adresse à l'utilisateur.
#[derive(Debug)]
pub enum Error {
    /// Un élément de ce nom existe déjà à la destination.
    Exists(String),
    /// Nom vide, « . », « .. » ou contenant « / ».
    InvalidName,
    /// Copier ou déplacer un dossier dans lui-même.
    IntoItself,
    /// La corbeille de l'utilisateur n'est pas sur le même disque.
    OtherDisk,
    /// L'élément n'est plus dans la corbeille.
    NotInTrash,
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exists(name) => f.write_str(&texts::already_exists(name)),
            Self::InvalidName => f.write_str(texts::INVALID_NAME),
            Self::IntoItself => f.write_str(texts::INTO_ITSELF),
            Self::OtherDisk => f.write_str(texts::OTHER_DISK),
            Self::NotInTrash => f.write_str(texts::NOT_IN_TRASH),
            Self::Io(error) => match error.kind() {
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
                    f.write_str(texts::NOT_ALLOWED)
                }
                io::ErrorKind::NotFound => f.write_str(texts::NOT_FOUND),
                io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
                    f.write_str(texts::DISK_FULL)
                }
                _ => f.write_str(&texts::system_error(&error.to_string())),
            },
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Refuse les noms qu'un système de fichiers ne peut pas porter.
pub fn validate_name(name: &str) -> Result<()> {
    let invalid =
        name.trim().is_empty() || name == "." || name == ".." || name.contains(['/', '\0']);
    if invalid {
        Err(Error::InvalidName)
    } else {
        Ok(())
    }
}

fn exists(path: &Path) -> bool {
    // Un lien cassé occupe aussi son nom.
    fs::symlink_metadata(path).is_ok()
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or(Error::InvalidName)
}

/// Premier nom libre dans `dir` : `name` s'il l'est, sinon « name (copie) », « name (copie 2) »…
/// L'extension d'un fichier reste à la fin ; celle d'un dossier n'est pas séparée.
pub fn free_name(dir: &Path, name: &str, is_dir: bool) -> String {
    if !exists(&dir.join(name)) {
        return name.to_owned();
    }
    let path = Path::new(name);
    let (stem, extension) = match (is_dir, path.file_stem(), path.extension()) {
        (false, Some(stem), Some(ext)) => (
            stem.to_string_lossy().into_owned(),
            Some(ext.to_string_lossy().into_owned()),
        ),
        _ => (name.to_owned(), None),
    };
    (1..)
        .map(|n| texts::copy_name(&stem, n, extension.as_deref()))
        .find(|candidate| !exists(&dir.join(candidate)))
        .unwrap_or_else(|| name.to_owned())
}

/// Refuse de mettre un dossier dans lui-même ou dans l'un de ses sous-dossiers.
fn check_not_inside(source: &Path, dest: &Path) -> Result<()> {
    let source = fs::canonicalize(source)?;
    let dest = fs::canonicalize(dest)?;
    if dest.starts_with(&source) {
        Err(Error::IntoItself)
    } else {
        Ok(())
    }
}

/// Copie récursive. Les liens symboliques sont recopiés comme liens, pas suivis.
fn copy_recursive(from: &Path, to: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(from)?, to)
    } else if meta.is_dir() {
        fs::create_dir(to)?;
        for item in fs::read_dir(from)? {
            let item = item?;
            copy_recursive(&item.path(), &to.join(item.file_name()))?;
        }
        fs::set_permissions(to, meta.permissions())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}

/// Copie chaque source dans `dest`. Rend les chemins créés.
pub fn copy_into(sources: &[PathBuf], dest: &Path) -> Result<Vec<PathBuf>> {
    let mut created = Vec::with_capacity(sources.len());
    for source in sources {
        let is_dir = fs::symlink_metadata(source)?.is_dir();
        if is_dir {
            check_not_inside(source, dest)?;
        }
        let target = dest.join(free_name(dest, &file_name(source)?, is_dir));
        copy_recursive(source, &target)?;
        created.push(target);
    }
    Ok(created)
}

/// Copie chaque élément à côté de lui-même : « nom (copie) ».
pub fn duplicate(sources: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut created = Vec::with_capacity(sources.len());
    for source in sources {
        let parent = source.parent().ok_or(Error::InvalidName)?;
        created.extend(copy_into(std::slice::from_ref(source), parent)?);
    }
    Ok(created)
}

/// Déplace un élément vers `target`, même d'un disque à l'autre (copie puis effacement).
pub fn move_path(source: &Path, target: &Path) -> Result<()> {
    if exists(target) {
        return Err(Error::Exists(file_name(target)?));
    }
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            copy_recursive(source, target)?;
            if fs::symlink_metadata(source)?.is_dir() {
                fs::remove_dir_all(source)?;
            } else {
                fs::remove_file(source)?;
            }
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// Déplace chaque source dans `dest`. Une source déjà dans `dest` ne bouge pas. Rend les
/// nouveaux chemins.
pub fn move_into(sources: &[PathBuf], dest: &Path) -> Result<Vec<PathBuf>> {
    let mut moved = Vec::with_capacity(sources.len());
    for source in sources {
        let target = dest.join(file_name(source)?);
        if source.parent() == Some(dest) {
            moved.push(target);
            continue;
        }
        if fs::symlink_metadata(source)?.is_dir() {
            check_not_inside(source, dest)?;
        }
        move_path(source, &target)?;
        moved.push(target);
    }
    Ok(moved)
}

/// Renomme un élément dans son dossier. Rend son nouveau chemin.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf> {
    validate_name(new_name)?;
    let parent = path.parent().ok_or(Error::InvalidName)?;
    let target = parent.join(new_name);
    if target == path {
        return Ok(target);
    }
    if exists(&target) {
        return Err(Error::Exists(new_name.to_owned()));
    }
    fs::rename(path, &target)?;
    Ok(target)
}

/// Crée « Nouveau dossier », ou « Nouveau dossier 2 »… si le nom est pris.
pub fn new_folder(dir: &Path) -> Result<PathBuf> {
    for n in 1.. {
        let path = dir.join(texts::new_folder_name(n));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::InvalidName)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("dossier parent");
        }
        fs::write(path, content).expect("écriture");
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).expect("lecture")
    }

    #[test]
    fn copier_un_dossier_copie_tout_son_contenu() {
        let tmp = TempDir::new("copie");
        let (src, dest) = (tmp.path().join("projet"), tmp.path().join("dest"));
        write(&src.join("a.txt"), "a");
        write(&src.join("sous/b.txt"), "b");
        std::os::unix::fs::symlink("a.txt", src.join("lien")).expect("lien");
        fs::create_dir(&dest).expect("dossier");

        let created = copy_into(std::slice::from_ref(&src), &dest).expect("copie");
        assert_eq!(created, [dest.join("projet")]);
        assert_eq!(read(&dest.join("projet/sous/b.txt")), "b");
        assert_eq!(
            fs::read_link(dest.join("projet/lien")).expect("lien copié"),
            Path::new("a.txt")
        );
        // La source est intacte.
        assert_eq!(read(&src.join("a.txt")), "a");
    }

    #[test]
    fn une_copie_au_nom_pris_devient_copie_sans_rien_ecraser() {
        let tmp = TempDir::new("conflit");
        let file = tmp.path().join("notes.txt");
        write(&file, "original");
        let first = duplicate(std::slice::from_ref(&file)).expect("copie");
        let second = duplicate(std::slice::from_ref(&file)).expect("copie");
        assert_eq!(first, [tmp.path().join("notes (copie).txt")]);
        assert_eq!(second, [tmp.path().join("notes (copie 2).txt")]);
        assert_eq!(read(&file), "original");
        assert_eq!(read(&second[0]), "original");

        let dir = tmp.path().join("v1.2");
        fs::create_dir(&dir).expect("dossier");
        assert_eq!(
            duplicate(&[dir]).expect("copie"),
            [tmp.path().join("v1.2 (copie)")]
        );
    }

    #[test]
    fn un_dossier_ne_va_pas_dans_lui_meme() {
        let tmp = TempDir::new("soi");
        let dir = tmp.path().join("a");
        fs::create_dir_all(dir.join("b")).expect("dossiers");
        assert!(matches!(
            copy_into(std::slice::from_ref(&dir), &dir.join("b")),
            Err(Error::IntoItself)
        ));
        assert!(matches!(
            move_into(std::slice::from_ref(&dir), &dir),
            Err(Error::IntoItself)
        ));
    }

    #[test]
    fn deplacer_vide_la_source_et_refuse_un_nom_pris() {
        let tmp = TempDir::new("deplacement");
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        write(&a.join("x.txt"), "x");
        write(&a.join("y.txt"), "y");
        write(&b.join("y.txt"), "autre");

        let moved = move_into(&[a.join("x.txt")], &b).expect("déplacement");
        assert_eq!(moved, [b.join("x.txt")]);
        assert!(!a.join("x.txt").exists());
        assert_eq!(read(&b.join("x.txt")), "x");

        let refused = move_into(&[a.join("y.txt")], &b);
        assert!(matches!(refused, Err(Error::Exists(ref name)) if name == "y.txt"));
        assert_eq!(read(&a.join("y.txt")), "y");
        assert_eq!(read(&b.join("y.txt")), "autre");
        // Déplacer là où l'élément est déjà ne fait rien.
        assert_eq!(
            move_into(&[b.join("x.txt")], &b).expect("sur place"),
            [b.join("x.txt")]
        );
    }

    #[test]
    fn renommer_verifie_le_nom() {
        let tmp = TempDir::new("renommage");
        let file = tmp.path().join("avant.txt");
        write(&file, "contenu");
        write(&tmp.path().join("pris.txt"), "");

        let renamed = rename(&file, "après.txt").expect("renommage");
        assert_eq!(read(&renamed), "contenu");
        assert!(!file.exists());
        assert!(matches!(
            rename(&renamed, "pris.txt"),
            Err(Error::Exists(_))
        ));
        for bad in ["", "  ", ".", "..", "a/b"] {
            assert!(
                matches!(rename(&renamed, bad), Err(Error::InvalidName)),
                "{bad:?}"
            );
        }
        assert_eq!(rename(&renamed, "après.txt").expect("même nom"), renamed);
    }

    #[test]
    fn un_nouveau_dossier_prend_un_nom_libre() {
        let tmp = TempDir::new("nouveau");
        assert_eq!(
            new_folder(tmp.path()).expect("création"),
            tmp.path().join("Nouveau dossier")
        );
        assert_eq!(
            new_folder(tmp.path()).expect("création"),
            tmp.path().join("Nouveau dossier 2")
        );
    }

    #[test]
    fn les_erreurs_se_disent_en_francais() {
        let missing = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(Error::from(missing).to_string(), texts::NOT_FOUND);
        assert_eq!(
            Error::Exists("a.txt".into()).to_string(),
            "« a.txt » existe déjà ici."
        );
    }
}
