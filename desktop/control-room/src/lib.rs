//! Control Room, the settings application: a page per domain, each setting written to the file
//! its program already reads and follows live. See `docs/control-room.md`.
//!
//! `settings`, `search` and `thumbnails` do not depend on iced, so that they are tested on
//! temporary folders; `app`, `view` and `pages` make up the iced application.

pub mod app;
mod pages;
pub mod search;
pub mod settings;
pub mod thumbnails;
mod view;

#[cfg(test)]
mod scenarios;
