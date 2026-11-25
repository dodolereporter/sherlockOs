//! Bibliothèque visuelle du desktop.
//!
//! Seul endroit où s'écrivent les couleurs, les tailles et les textes d'interface. Les jetons
//! (`color`, `metrics`, `motion`, `strings`, `typography`, `icon`, `logo`,
//! `window`) ne dépendent de rien. Le thème
//! et les composants iced sont derrière la fonction `iced`, active par défaut. La galerie vit
//! dans `examples/galerie`. Voir `docs/sherlock-ui.md`.

pub mod color;
pub mod icon;
pub mod logo;
pub mod metrics;
pub mod motion;
pub mod strings;
pub mod typography;
pub mod window;

#[cfg(feature = "iced")]
pub mod component;
#[cfg(feature = "iced")]
pub mod focus;
#[cfg(feature = "iced")]
pub mod theme;
