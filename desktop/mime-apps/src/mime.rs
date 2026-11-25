//! File types from the shared MIME-info database: the `globs2` files (patterns on names) and
//! the `subclasses` files (a Rust source file is also plain text). Content sniffing (`magic`)
//! is not done: a name without a known pattern is `application/octet-stream`.

use std::collections::HashMap;
use std::path::Path;

use crate::Dirs;

/// Type of a folder.
pub const DIRECTORY: &str = "inode/directory";
/// Type of anything else that is not recognised.
pub const UNKNOWN: &str = "application/octet-stream";
const TEXT: &str = "text/plain";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Glob {
    weight: u32,
    mime: String,
    pattern: String,
    case_sensitive: bool,
}

/// The MIME database, read once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MimeDb {
    globs: Vec<Glob>,
    parents: HashMap<String, Vec<String>>,
}

impl MimeDb {
    /// Reads `mime/globs2` and `mime/subclasses` from every data directory.
    pub fn load(dirs: &Dirs) -> Self {
        let mut db = Self::default();
        for dir in dirs.data() {
            if let Ok(text) = std::fs::read_to_string(dir.join("mime/globs2")) {
                db.add_globs(&text);
            }
            if let Ok(text) = std::fs::read_to_string(dir.join("mime/subclasses")) {
                db.add_subclasses(&text);
            }
        }
        // Heaviest first, then longest pattern, as the specification recommends.
        db.globs.sort_by(|a, b| {
            b.weight
                .cmp(&a.weight)
                .then(b.pattern.len().cmp(&a.pattern.len()))
        });
        db
    }

    /// Lines `weight:type:pattern[:flags]`.
    pub fn add_globs(&mut self, text: &str) {
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut fields = line.splitn(4, ':');
            let (Some(weight), Some(mime), Some(pattern)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let Ok(weight) = weight.parse() else {
                continue;
            };
            let case_sensitive = fields
                .next()
                .is_some_and(|flags| flags.split(',').any(|f| f == "cs"));
            self.globs.push(Glob {
                weight,
                mime: mime.to_owned(),
                pattern: pattern.to_owned(),
                case_sensitive,
            });
        }
    }

    /// Lines `type parent`.
    pub fn add_subclasses(&mut self, text: &str) {
        for line in text.lines() {
            if let Some((child, parent)) = line.split_once(' ') {
                self.parents
                    .entry(child.to_owned())
                    .or_default()
                    .push(parent.trim().to_owned());
            }
        }
    }

    /// The type of a file from its name; `inode/directory` for a folder.
    pub fn type_of(&self, path: &Path, is_dir: bool) -> String {
        if is_dir {
            return DIRECTORY.to_owned();
        }
        let Some(name) = path.file_name().map(|n| n.to_string_lossy()) else {
            return UNKNOWN.to_owned();
        };
        let lower = name.to_lowercase();
        self.globs
            .iter()
            .find(|glob| {
                let candidate = if glob.case_sensitive {
                    name.as_ref()
                } else {
                    lower.as_str()
                };
                let pattern = if glob.case_sensitive {
                    glob.pattern.clone()
                } else {
                    glob.pattern.to_lowercase()
                };
                matches(&pattern, candidate)
            })
            .map_or_else(|| UNKNOWN.to_owned(), |glob| glob.mime.clone())
    }

    /// The type and every type it derives from, closest first: `text/rust`, `text/plain`.
    /// Every `text/*` type derives from `text/plain`.
    pub fn ancestors(&self, mime: &str) -> Vec<String> {
        let mut out = vec![mime.to_owned()];
        let mut i = 0;
        while i < out.len() {
            if let Some(parents) = self.parents.get(&out[i]) {
                for parent in parents {
                    if !out.contains(parent) {
                        out.push(parent.clone());
                    }
                }
            }
            i += 1;
        }
        if mime.starts_with("text/") && !out.iter().any(|m| m == TEXT) {
            out.push(TEXT.to_owned());
        }
        out
    }
}

/// Glob matching with `*`, `?` and `[...]`, enough for the shared MIME database patterns.
fn matches(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    fn go(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|i| go(&p[1..], &n[i..])),
            Some('?') => !n.is_empty() && go(&p[1..], &n[1..]),
            Some('[') => {
                let Some(end) = p.iter().position(|c| *c == ']') else {
                    return n.first() == Some(&'[') && go(&p[1..], &n[1..]);
                };
                let set = &p[1..end];
                !n.is_empty() && set.contains(&n[0]) && go(&p[end + 1..], &n[1..])
            }
            Some(c) => n.first() == Some(c) && go(&p[1..], &n[1..]),
        }
    }
    go(&p, &n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> MimeDb {
        let mut db = MimeDb::default();
        db.add_globs(
            "50:text/rust:*.rs\n50:text/x-makefile:Makefile:cs\n50:image/jpeg:*.jpg\n\
             50:application/x-compressed-tar:*.tar.gz\n50:application/gzip:*.gz\n",
        );
        db.add_subclasses("text/rust text/plain\napplication/x-compressed-tar application/gzip\n");
        db.globs.sort_by(|a, b| {
            b.weight
                .cmp(&a.weight)
                .then(b.pattern.len().cmp(&a.pattern.len()))
        });
        db
    }

    #[test]
    fn names_give_types() {
        let db = db();
        assert_eq!(db.type_of(Path::new("/a/main.rs"), false), "text/rust");
        assert_eq!(db.type_of(Path::new("PHOTO.JPG"), false), "image/jpeg");
        assert_eq!(db.type_of(Path::new("Makefile"), false), "text/x-makefile");
        assert_eq!(db.type_of(Path::new("makefile"), false), UNKNOWN);
        // The longest pattern wins.
        assert_eq!(
            db.type_of(Path::new("a.tar.gz"), false),
            "application/x-compressed-tar"
        );
        assert_eq!(db.type_of(Path::new("src"), true), DIRECTORY);
        assert!(matches("*.[ch]", "x.h"));
    }

    #[test]
    fn types_have_their_parents() {
        let db = db();
        assert_eq!(db.ancestors("text/rust"), ["text/rust", "text/plain"]);
        assert_eq!(
            db.ancestors("text/x-makefile"),
            ["text/x-makefile", "text/plain"]
        );
        assert_eq!(
            db.ancestors("application/x-compressed-tar"),
            ["application/x-compressed-tar", "application/gzip"]
        );
    }
}
