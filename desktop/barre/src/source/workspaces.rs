//! Bureaux et application active.
//!
//! Le compositeur ne publie encore ni ses bureaux (`ext-workspace-v1`) ni ses fenêtres
//! (`ext-foreign-toplevel-list-v1`). En attendant, `Placeholder` est une source factice, fixée
//! au démarrage par deux variables d'environnement. Ce que le compositeur devra fournir est
//! décrit dans `docs/barre.md`.

use super::{Source, SourceError};

/// Noms des bureaux factices, séparés par des virgules. Un nom vide garde le seul numéro.
pub const NAMES_VAR: &str = "BARRE_BUREAUX";
/// Nom de l'application active factice.
pub const APP_VAR: &str = "BARRE_APPLICATION";
/// Nombre de bureaux factices sans `BARRE_BUREAUX`.
const DEFAULT_COUNT: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspaces {
    /// Nom de chaque bureau, dans l'ordre de leurs numéros.
    pub names: Vec<Option<String>>,
    /// Indice du bureau actif dans `names`.
    pub active: usize,
    /// Nom de l'application qui a le focus.
    pub app: Option<String>,
}

impl Workspaces {
    /// Les bureaux factices : ceux de `names` (variable `BARRE_BUREAUX`), le premier actif.
    pub fn placeholder(names: Option<&str>, app: Option<&str>) -> Self {
        let names = match names {
            Some(list) => list
                .split(',')
                .map(|n| Some(n.trim().to_owned()).filter(|n| !n.is_empty()))
                .collect(),
            None => vec![None; DEFAULT_COUNT],
        };
        let app = app
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned);
        Self {
            names,
            active: 0,
            app,
        }
    }
}

/// Source factice : lit les variables une fois, ne change jamais.
#[derive(Debug, Default)]
pub struct Placeholder;

impl Source for Placeholder {
    type State = Workspaces;

    fn read(&mut self) -> Result<Workspaces, SourceError> {
        let names = std::env::var(NAMES_VAR).ok();
        let app = std::env::var(APP_VAR).ok();
        Ok(Workspaces::placeholder(names.as_deref(), app.as_deref()))
    }

    fn wait(&mut self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sans_variable_trois_bureaux_numerotes() {
        let w = Workspaces::placeholder(None, None);
        assert_eq!(w.names, [None, None, None]);
        assert_eq!((w.active, w.app), (0, None));
    }

    #[test]
    fn les_noms_viennent_de_la_variable() {
        let w = Workspaces::placeholder(Some("Code, ,Jeu"), Some(" Éditeur "));
        assert_eq!(
            w.names,
            [Some(String::from("Code")), None, Some(String::from("Jeu"))]
        );
        assert_eq!(w.app.as_deref(), Some("Éditeur"));
    }
}
