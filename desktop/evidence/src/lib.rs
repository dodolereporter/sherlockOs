//! Evidence, the desktop file manager: navigation, listing with git status, copy, cut, paste,
//! rename, trash and preview. See `docs/evidence.md`.
//!
//! The `listing`, `ops`, `trash`, `git` and `preview` modules do not depend on iced, so they
//! are tested on temporary folders. `app` and `view` make up the iced application.

pub mod app;
pub mod clock;
pub mod devices;
pub mod git;
pub mod icons;
pub mod listing;
pub mod menu;
pub mod ops;
pub mod places;
pub mod preview;
pub mod probe;
pub mod selection;
pub mod startup;
pub mod trash;
pub mod udisks;
mod view;

#[cfg(test)]
mod scenarios;
#[cfg(test)]
mod testing;
