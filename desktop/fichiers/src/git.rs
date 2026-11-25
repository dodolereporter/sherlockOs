//! État git des éléments d'un dossier, lu par la commande `git` du poste
//! (`git status --porcelain=v2`), sans lier libgit2.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// État git d'un élément, du plus important au moins important : un dossier qui contient un
/// fichier modifié et un fichier nouveau est « modifié ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitStatus {
    Modified,
    New,
    Ignored,
}

/// Ce que git dit d'un dossier.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repo {
    /// Branche courante, absente en tête détachée.
    pub branch: Option<String>,
    /// État de chaque élément du dossier, par nom.
    pub statuses: HashMap<String, GitStatus>,
    /// État du dossier entier, quand git le donne d'un bloc (dossier nouveau ou ignoré).
    pub whole: Option<GitStatus>,
}

impl Repo {
    /// État d'un élément du dossier.
    pub fn status(&self, name: &str) -> Option<GitStatus> {
        self.statuses.get(name).copied().or(self.whole)
    }
}

/// Lance git dans `dir`. `None` si le dossier n'est pas dans un dépôt ou si git manque.
pub fn status(dir: &Path) -> Option<Repo> {
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .arg("--no-optional-locks")
            .args(args)
            .output()
            .ok()?;
        output.status.success().then_some(output.stdout)
    };
    let prefix = git(&["rev-parse", "--show-prefix"])?;
    let prefix = String::from_utf8_lossy(&prefix).trim_end().to_owned();
    let output = git(&[
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--ignored",
        "--untracked-files=normal",
        "--",
        ".",
    ])?;
    Some(parse(&output, &prefix))
}

/// Analyse la sortie de `git status --porcelain=v2 -z --branch`. Les chemins y sont relatifs à
/// la racine du dépôt ; `prefix` est celui du dossier listé (`git rev-parse --show-prefix`),
/// vide à la racine, terminé par « / » sinon.
pub fn parse(output: &[u8], prefix: &str) -> Repo {
    let mut repo = Repo::default();
    let text = String::from_utf8_lossy(output);
    let mut records = text.split('\0');
    while let Some(record) = records.next() {
        let (status, path) = match record.as_bytes().first() {
            Some(b'#') => {
                if let Some(head) = record.strip_prefix("# branch.head ") {
                    repo.branch = (head != "(detached)").then(|| head.to_owned());
                }
                continue;
            }
            // Modifié : `1 XY sub mH mI mW hH hI chemin`. Un fichier ajouté à l'index (X = A)
            // est encore nouveau pour l'utilisateur.
            Some(b'1') => {
                let added = record.get(2..3) == Some("A");
                let status = if added {
                    GitStatus::New
                } else {
                    GitStatus::Modified
                };
                (status, record.splitn(9, ' ').nth(8))
            }
            // Renommé ou copié : le chemin d'origine suit, dans son propre champ.
            Some(b'2') => {
                records.next();
                (GitStatus::Modified, record.splitn(10, ' ').nth(9))
            }
            // En conflit de fusion.
            Some(b'u') => (GitStatus::Modified, record.splitn(11, ' ').nth(10)),
            Some(b'?') => (GitStatus::New, record.get(2..)),
            Some(b'!') => (GitStatus::Ignored, record.get(2..)),
            _ => continue,
        };
        let Some(rest) = path.and_then(|p| p.strip_prefix(prefix)) else {
            continue;
        };
        let merge = |old: Option<GitStatus>| Some(old.map_or(status, |old| old.min(status)));
        match rest.split('/').find(|part| !part.is_empty()) {
            Some(name) => {
                let merged = merge(repo.statuses.get(name).copied());
                if let Some(merged) = merged {
                    repo.statuses.insert(name.to_owned(), merged);
                }
            }
            None => repo.whole = merge(repo.whole),
        }
    }
    repo
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = concat!(
        "# branch.oid 7671be4\0",
        "# branch.head main\0",
        "1 .M N... 100644 100644 100644 aaa bbb desktop/src/main.rs\0",
        "1 A. N... 000000 100644 100644 000 ccc desktop/Dockerfile\0",
        "2 R. N... 100644 100644 100644 ddd eee R100 desktop/nouveau nom.md\0desktop/ancien.md\0",
        "? desktop/src/brouillon.rs\0",
        "? desktop/essais/\0",
        "! desktop/target/\0",
        "1 .M N... 100644 100644 100644 fff ggg README.md\0",
    );

    #[test]
    fn chaque_element_du_dossier_prend_son_etat() {
        let repo = parse(OUTPUT.as_bytes(), "desktop/");
        assert_eq!(repo.branch.as_deref(), Some("main"));
        // Un dossier qui contient un modifié et un nouveau est modifié.
        assert_eq!(repo.status("src"), Some(GitStatus::Modified));
        assert_eq!(repo.status("Dockerfile"), Some(GitStatus::New));
        // Le nom renommé garde ses espaces ; l'ancien nom n'apparaît pas.
        assert_eq!(repo.status("nouveau nom.md"), Some(GitStatus::Modified));
        assert_eq!(repo.status("ancien.md"), None);
        assert_eq!(repo.status("essais"), Some(GitStatus::New));
        assert_eq!(repo.status("target"), Some(GitStatus::Ignored));
        // Hors du dossier listé.
        assert_eq!(repo.status("README.md"), None);
        assert_eq!(repo.statuses.len(), 5);
    }

    #[test]
    fn a_la_racine_le_prefixe_est_vide() {
        let repo = parse(OUTPUT.as_bytes(), "");
        assert_eq!(repo.status("desktop"), Some(GitStatus::Modified));
        assert_eq!(repo.status("README.md"), Some(GitStatus::Modified));
    }

    #[test]
    fn un_dossier_ignore_d_un_bloc_marque_tout_son_contenu() {
        let repo = parse(
            b"# branch.head (detached)\0! desktop/target/\0",
            "desktop/target/",
        );
        assert_eq!(repo.branch, None);
        assert_eq!(repo.status("debug"), Some(GitStatus::Ignored));
    }

    #[test]
    fn git_lit_un_vrai_depot() {
        let dir = crate::testing::TempDir::new("git");
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args([
                    "-c",
                    "user.name=Essai",
                    "-c",
                    "user.email=essai@example.invalid",
                ])
                .args(args)
                .output()
                .expect("git est installé")
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q", "-b", "essai"]);
        std::fs::write(dir.path().join(".gitignore"), "cible/\n").expect("écriture");
        std::fs::write(dir.path().join("suivi.txt"), "un").expect("écriture");
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "départ"]);
        std::fs::write(dir.path().join("suivi.txt"), "deux").expect("écriture");
        std::fs::write(dir.path().join("neuf.txt"), "").expect("écriture");
        std::fs::create_dir(dir.path().join("cible")).expect("dossier");
        std::fs::write(dir.path().join("cible/sortie"), "").expect("écriture");

        let repo = status(dir.path()).expect("dépôt reconnu");
        assert_eq!(repo.branch.as_deref(), Some("essai"));
        assert_eq!(repo.status("suivi.txt"), Some(GitStatus::Modified));
        assert_eq!(repo.status("neuf.txt"), Some(GitStatus::New));
        assert_eq!(repo.status("cible"), Some(GitStatus::Ignored));
        assert_eq!(repo.status(".gitignore"), None);
        assert!(status(&std::env::temp_dir().join("pas-un-depot-fichiers")).is_none());
    }
}
