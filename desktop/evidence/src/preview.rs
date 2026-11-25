//! Aperçu de l'élément sélectionné : début d'un texte, image, ou simples informations.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use sherlock_ui::component::CodeRole;
use sherlock_ui::metrics;

use crate::listing::{Entry, Kind};

/// Octets lus au plus pour l'aperçu d'un texte : de quoi remplir le panneau.
const READ_LIMIT: u64 = 16 * 1024;
/// Largeur d'une tabulation dans l'aperçu.
const TAB: &str = "    ";

/// Ce que montre le panneau d'aperçu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    /// Premières lignes d'un fichier texte, découpées en morceaux colorés.
    Text(Vec<Vec<(String, CodeRole)>>),
    /// Image à afficher.
    Image(PathBuf),
    /// Rien à montrer de plus que les informations du fichier.
    Info,
}

/// Prépare l'aperçu d'un élément. Un fichier illisible, binaire ou vide n'a que ses
/// informations.
pub fn load(entry: &Entry) -> Preview {
    if entry.kind != Kind::File {
        return Preview::Info;
    }
    if entry.is_image() {
        return Preview::Image(entry.path.clone());
    }
    let mut bytes = Vec::new();
    let read = File::open(&entry.path).and_then(|f| f.take(READ_LIMIT).read_to_end(&mut bytes));
    if read.is_err() || bytes.is_empty() || bytes.contains(&0) {
        return Preview::Info;
    }
    // La lecture a pu couper un caractère en deux : seule la fin peut être invalide.
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) if bytes.len() - error.valid_up_to() < 4 => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or_default()
        }
        Err(_) => return Preview::Info,
    };
    let language = Language::of(entry);
    let lines = text
        .lines()
        .take(metrics::PREVIEW_LINES)
        .map(|line| {
            let line: String = line
                .replace('\t', TAB)
                .chars()
                .take(metrics::PREVIEW_COLUMNS)
                .collect();
            language.highlight(&line)
        })
        .collect();
    Preview::Text(lines)
}

/// Coloration légère de l'aperçu : mots-clés et commentaires de quelques langages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Docker,
    Rust,
    Toml,
    Shell,
    Python,
    Plain,
}

const DOCKER: &[&str] = &[
    "FROM",
    "AS",
    "RUN",
    "COPY",
    "ADD",
    "WORKDIR",
    "ENV",
    "ARG",
    "CMD",
    "ENTRYPOINT",
    "EXPOSE",
    "USER",
    "LABEL",
    "VOLUME",
    "HEALTHCHECK",
];
const RUST: &[&str] = &[
    "as", "async", "await", "const", "crate", "else", "enum", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "return", "self", "Self", "static",
    "struct", "trait", "type", "use", "where", "while",
];
const SHELL: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "case", "esac", "while",
    "function", "export", "local", "return",
];
const PYTHON: &[&str] = &[
    "def", "class", "import", "from", "as", "if", "elif", "else", "for", "in", "while", "return",
    "with", "try", "except", "None", "True", "False", "lambda", "yield",
];

impl Language {
    fn of(entry: &Entry) -> Self {
        if entry.name == "Dockerfile" || entry.name.starts_with("Dockerfile.") {
            return Self::Docker;
        }
        match entry.extension().as_deref() {
            Some("rs") => Self::Rust,
            Some("toml") => Self::Toml,
            Some("sh" | "bash" | "zsh") => Self::Shell,
            Some("py") => Self::Python,
            _ if entry.name == "justfile" || entry.name == "Makefile" => Self::Shell,
            _ => Self::Plain,
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Docker => DOCKER,
            Self::Rust => RUST,
            Self::Shell => SHELL,
            Self::Python => PYTHON,
            Self::Toml | Self::Plain => &[],
        }
    }

    fn comment(self) -> Option<&'static str> {
        match self {
            Self::Rust => Some("//"),
            Self::Plain => None,
            _ => Some("#"),
        }
    }

    /// Découpe une ligne en morceaux colorés.
    fn highlight(self, line: &str) -> Vec<(String, CodeRole)> {
        let trimmed = line.trim_start();
        if self.comment().is_some_and(|c| trimmed.starts_with(c)) {
            return vec![(line.to_owned(), CodeRole::Comment)];
        }
        if self == Self::Toml && trimmed.starts_with('[') {
            return vec![(line.to_owned(), CodeRole::Keyword)];
        }
        let keywords = self.keywords();
        let mut pieces: Vec<(String, CodeRole)> = Vec::new();
        let mut push = |piece: &str, role: CodeRole| match pieces.last_mut() {
            Some((last, r)) if *r == role => last.push_str(piece),
            _ => pieces.push((piece.to_owned(), role)),
        };
        let mut word_start = None;
        for (i, c) in line
            .char_indices()
            .chain(std::iter::once((line.len(), ' ')))
        {
            let in_word = c.is_alphanumeric() || c == '_';
            match (in_word, word_start) {
                (true, None) => word_start = Some(i),
                (false, Some(start)) => {
                    let word = &line[start..i];
                    let role = if keywords.contains(&word) {
                        CodeRole::Keyword
                    } else {
                        CodeRole::Plain
                    };
                    push(word, role);
                    word_start = None;
                    if i < line.len() {
                        push(&line[i..i + c.len_utf8()], CodeRole::Plain);
                    }
                }
                (false, None) if i < line.len() => {
                    push(&line[i..i + c.len_utf8()], CodeRole::Plain)
                }
                _ => {}
            }
        }
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn entry(path: PathBuf) -> Entry {
        Entry::read(&path, false).expect("élément")
    }

    #[test]
    fn un_texte_montre_ses_premieres_lignes() {
        let tmp = TempDir::new("apercu");
        let path = tmp.path().join("Dockerfile");
        let content: String = (0..100).map(|i| format!("RUN\tétape {i}\n")).collect();
        std::fs::write(&path, content).expect("écriture");
        let Preview::Text(lines) = load(&entry(path)) else {
            panic!("aperçu texte attendu");
        };
        assert_eq!(lines.len(), metrics::PREVIEW_LINES);
        assert_eq!(
            lines[0],
            [
                ("RUN".to_owned(), CodeRole::Keyword),
                ("    étape 0".to_owned(), CodeRole::Plain)
            ]
        );
    }

    #[test]
    fn la_coloration_trouve_mots_cles_et_commentaires() {
        let rust = Language::Rust.highlight("pub fn main() // fin");
        assert_eq!(rust[0], ("pub".to_owned(), CodeRole::Keyword));
        assert_eq!(rust[2], ("fn".to_owned(), CodeRole::Keyword));
        let comment = Language::Shell.highlight("  # commentaire");
        assert_eq!(comment, [("  # commentaire".to_owned(), CodeRole::Comment)]);
        let text: String = Language::Docker
            .highlight("COPY . .")
            .into_iter()
            .map(|(piece, _)| piece)
            .collect();
        assert_eq!(text, "COPY . .");
    }

    #[test]
    fn un_binaire_une_image_et_un_dossier_ont_leur_apercu() {
        let tmp = TempDir::new("apercu-autres");
        let binary = tmp.path().join("programme");
        std::fs::write(&binary, [0x7f, b'E', b'L', b'F', 0, 1]).expect("écriture");
        assert_eq!(load(&entry(binary)), Preview::Info);
        let image = tmp.path().join("photo.png");
        std::fs::write(&image, []).expect("écriture");
        assert_eq!(load(&entry(image.clone())), Preview::Image(image));
        assert_eq!(load(&entry(tmp.path().to_path_buf())), Preview::Info);
        // Un caractère coupé par la limite de lecture ne gâche pas l'aperçu.
        let cut = tmp.path().join("long.txt");
        let mut text = "a".repeat(READ_LIMIT as usize - 1).into_bytes();
        text.extend("é".as_bytes());
        std::fs::write(&cut, text).expect("écriture");
        assert!(matches!(load(&entry(cut)), Preview::Text(_)));
    }
}
