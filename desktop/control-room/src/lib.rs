//! Control Room, the settings application: a page per domain, each setting written to the file
//! its program already reads and follows live. See `docs/control-room.md`.
//!
//! `settings`, `search`, `thumbnails`, `sound`, `apps` and `lock` do not depend on iced, so that they are tested on
//! temporary folders; `app`, `view` and `pages` make up the iced application.

pub mod app;
pub mod apps;
pub mod lock;
mod pages;
pub mod search;
pub mod settings;
pub mod sound;
pub mod thumbnails;
mod view;

#[cfg(test)]
mod scenarios;
