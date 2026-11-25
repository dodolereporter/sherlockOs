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

/// Settings forced on every git command Evidence runs, over whatever the repository's own
/// `.git/config` (or a file it includes) says. A folder can come from a USB key or an
/// archive: its repository configuration is not trusted, and must not be able to run a
/// program when the folder is merely listed.
const LOCKED_CONFIG: [&str; 6] = [
    // The fsmonitor hook is a command git runs on `status`.
    "core.fsmonitor=false",
    // The untracked cache can be rebuilt with the fsmonitor; keep it out of the way.
    "core.untrackedCache=false",
    // `status` runs no hook today; this keeps it so if that changes.
    "core.hooksPath=/dev/null",
    // Nothing should ever ask for a password or open a pager.
    "core.askPass=",
    "core.pager=cat",
    // Diff drivers and their text conversions are commands too.
    "diff.external=",
];

/// Environment of every git command: no system or user configuration (only the repository's,
/// neutralised above), no prompt, no lock taken, and none of the caller's `GIT_*` overrides.
fn git_command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(dir)
        .arg("--no-optional-locks");
    for setting in LOCKED_CONFIG {
        command.arg("-c").arg(setting);
    }
    command
}

fn run(mut command: Command) -> Option<Vec<u8>> {
    let output = command.stdin(std::process::Stdio::null()).output().ok()?;
    output.status.success().then_some(output.stdout)
}

/// The filter drivers the repository configuration declares (`filter.<name>.clean`,
/// `.smudge`, `.process`), included files counted. Reading the configuration runs nothing.
fn filter_drivers(dir: &Path) -> Vec<String> {
    let mut command = git_command(dir);
    command.args([
        "config",
        "--includes",
        "--name-only",
        "--get-regexp",
        r"^filter\.",
    ]);
    let output = run(command).unwrap_or_default();
    let mut names: Vec<String> = String::from_utf8_lossy(&output)
        .lines()
        .filter_map(|key| {
            let rest = key.strip_prefix("filter.")?;
            let (name, _) = rest.rsplit_once('.')?;
            Some(name.to_owned())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Runs git in `dir`, with a locked-down configuration (see `LOCKED_CONFIG`): every filter
/// driver the repository declares is emptied, which git treats as no filter at all, and
/// submodules (each with its own configuration) are not entered. `None` if the folder is
/// not in a repository or git is missing.
pub fn status(dir: &Path) -> Option<Repo> {
    let mut command = git_command(dir);
    command.args(["rev-parse", "--show-prefix"]);
    let prefix = run(command)?;
    let prefix = String::from_utf8_lossy(&prefix).trim_end().to_owned();
    let mut command = git_command(dir);
    for name in filter_drivers(dir) {
        for (key, value) in [
            ("clean", ""),
            ("smudge", ""),
            ("process", ""),
            ("required", "false"),
        ] {
            command
                .arg("-c")
                .arg(format!("filter.{name}.{key}={value}"));
        }
    }
    command.args([
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--ignored",
        "--untracked-files=normal",
        "--ignore-submodules=all",
        "--",
        ".",
    ]);
    Some(parse(&run(command)?, &prefix))
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

    #[test]
    fn a_trapped_repository_runs_nothing() {
        let dir = crate::testing::TempDir::new("trapped");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).expect("folder");
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                ])
                .args(args)
                .output()
                .expect("git is installed")
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("tracked.txt"), "one").expect("write");
        std::fs::write(repo.join(".gitattributes"), "* filter=trap\n").expect("write");
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "initial"]);
        // Every hook a status could reach writes a witness file.
        let witness = |name: &str| dir.path().join(name);
        let touch = |name: &str| format!("touch '{}'", witness(name).display());
        let included = dir.path().join("included.config");
        std::fs::write(
            &included,
            format!("[core]\n\tfsmonitor = \"{}\"\n", touch("included")),
        )
        .expect("write");
        let config = format!(
            "[core]\n\tfsmonitor = \"{}\"\n\thooksPath = hooks\n\
             [filter \"trap\"]\n\tclean = \"{} ; cat\"\n\tprocess = \"{}\"\n\trequired = true\n\
             [include]\n\tpath = {}\n",
            touch("fsmonitor"),
            touch("clean"),
            touch("process"),
            included.display(),
        );
        let git_config = repo.join(".git/config");
        let mut full = std::fs::read_to_string(&git_config).expect("config");
        full.push_str(&config);
        std::fs::write(&git_config, full).expect("write");
        // A modified file makes status hash it again, through the clean filter.
        std::fs::write(repo.join("tracked.txt"), "two!").expect("write");

        let found = status(&repo).expect("repository read");
        assert_eq!(found.status("tracked.txt"), Some(GitStatus::Modified));
        for name in ["fsmonitor", "included", "clean", "process"] {
            assert!(!witness(name).exists(), "git ran the {name} command");
        }
    }
}
