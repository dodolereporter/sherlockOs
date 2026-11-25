//! Git status of a folder's items, read through the machine's `git` command
//! (`git status --porcelain=v2`), without linking libgit2.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Git status of an item, from most to least important: a folder that holds a modified file
/// and a new file is "modified".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitStatus {
    Modified,
    New,
    Ignored,
}

/// What git says about a folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repo {
    /// Current branch, absent on a detached head.
    pub branch: Option<String>,
    /// Status of each item of the folder, by name.
    pub statuses: HashMap<String, GitStatus>,
    /// Status of the whole folder, when git reports it as a block (new or ignored folder).
    pub whole: Option<GitStatus>,
}

impl Repo {
    /// Status of an item of the folder.
    pub fn status(&self, name: &str) -> Option<GitStatus> {
        self.statuses.get(name).copied().or(self.whole)
    }
}

/// Runs git in `dir`. `None` if the folder is not in a repository or git is missing.
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

/// Parses the output of `git status --porcelain=v2 -z --branch`. Paths there are relative to
/// the repository root; `prefix` is the listed folder's (`git rev-parse --show-prefix`), empty
/// at the root, ending with "/" otherwise.
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
            // Modified: `1 XY sub mH mI mW hH hI path`. A file added to the index (X = A) is
            // still new to the user.
            Some(b'1') => {
                let added = record.get(2..3) == Some("A");
                let status = if added {
                    GitStatus::New
                } else {
                    GitStatus::Modified
                };
                (status, record.splitn(9, ' ').nth(8))
            }
            // Renamed or copied: the original path follows, in its own field.
            Some(b'2') => {
                records.next();
                (GitStatus::Modified, record.splitn(10, ' ').nth(9))
            }
            // Merge conflict.
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
    fn each_item_of_the_folder_gets_its_status() {
        let repo = parse(OUTPUT.as_bytes(), "desktop/");
        assert_eq!(repo.branch.as_deref(), Some("main"));
        // A folder that holds a modified and a new item is modified.
        assert_eq!(repo.status("src"), Some(GitStatus::Modified));
        assert_eq!(repo.status("Dockerfile"), Some(GitStatus::New));
        // The renamed name keeps its spaces; the old name does not appear.
        assert_eq!(repo.status("nouveau nom.md"), Some(GitStatus::Modified));
        assert_eq!(repo.status("ancien.md"), None);
        assert_eq!(repo.status("essais"), Some(GitStatus::New));
        assert_eq!(repo.status("target"), Some(GitStatus::Ignored));
        // Outside the listed folder.
        assert_eq!(repo.status("README.md"), None);
        assert_eq!(repo.statuses.len(), 5);
    }

    #[test]
    fn at_the_root_the_prefix_is_empty() {
        let repo = parse(OUTPUT.as_bytes(), "");
        assert_eq!(repo.status("desktop"), Some(GitStatus::Modified));
        assert_eq!(repo.status("README.md"), Some(GitStatus::Modified));
    }

    #[test]
    fn a_folder_ignored_as_a_block_marks_all_its_content() {
        let repo = parse(
            b"# branch.head (detached)\0! desktop/target/\0",
            "desktop/target/",
        );
        assert_eq!(repo.branch, None);
        assert_eq!(repo.status("debug"), Some(GitStatus::Ignored));
    }

    #[test]
    fn git_reads_a_real_repository() {
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
                .expect("git is installed")
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q", "-b", "essai"]);
        std::fs::write(dir.path().join(".gitignore"), "cible/\n").expect("write");
        std::fs::write(dir.path().join("suivi.txt"), "un").expect("write");
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "initial"]);
        std::fs::write(dir.path().join("suivi.txt"), "deux").expect("write");
        std::fs::write(dir.path().join("neuf.txt"), "").expect("write");
        std::fs::create_dir(dir.path().join("cible")).expect("folder");
        std::fs::write(dir.path().join("cible/sortie"), "").expect("write");

        let repo = status(dir.path()).expect("repository recognised");
        assert_eq!(repo.branch.as_deref(), Some("essai"));
        assert_eq!(repo.status("suivi.txt"), Some(GitStatus::Modified));
        assert_eq!(repo.status("neuf.txt"), Some(GitStatus::New));
        assert_eq!(repo.status("cible"), Some(GitStatus::Ignored));
        assert_eq!(repo.status(".gitignore"), None);
        assert!(status(&std::env::temp_dir().join("not-a-repository-evidence")).is_none());
    }
}
