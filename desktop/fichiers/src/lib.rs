//! Fichiers, l'explorateur de fichiers du desktop : navigation, liste avec état git, copier,
//! couper, coller, renommer, corbeille et aperçu. Voir `docs/fichiers.md`.
//!
//! Les modules `listing`, `ops`, `trash`, `git` et `preview` ne dépendent pas d'iced : ils se
//! testent sur des dossiers temporaires. `app` et `view` forment l'application iced.

pub mod app;
pub mod clock;
pub mod git;
pub mod listing;
pub mod ops;
pub mod places;
pub mod preview;
pub mod trash;
mod view;

#[cfg(test)]
mod scenarios;
#[cfg(test)]
mod testing;
