//! Aperçu de l'élément sélectionné : début d'un texte, image, ou simples informations.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use sherlock_ui::metrics;

use crate::listing::{Entry, Kind};

/// Octets lus au plus pour l'aperçu d'un texte : de quoi remplir le panneau.
const READ_LIMIT: u64 = 16 * 1024;
/// Largeur d'une tabulation dans l'aperçu.
const TAB: &str = "    ";

/// Ce que montre le panneau d'aperçu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    /// Premières lignes d'un fichier texte.
    Text(Vec<String>),
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
    let lines = text
        .lines()
        .take(metrics::PREVIEW_LINES)
        .map(|line| {
            line.replace('\t', TAB)
                .chars()
                .take(metrics::PREVIEW_COLUMNS)
                .collect()
        })
        .collect();
    Preview::Text(lines)
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
        assert_eq!(lines[0], "RUN    étape 0");
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
