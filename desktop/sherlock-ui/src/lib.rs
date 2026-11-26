//! Visual library of the desktop.
//!
//! The only place where colors, sizes and interface texts are written. The tokens (`color`,
//! `cursor`, `metrics`, `motion`, `strings`, `texts`, `typography`, `icon`, `logo`, `window`,
//! `decorations`) depend on nothing, and neither does `gpu`, the choice of the renderer. The
//! theme and the iced components sit behind the `iced` feature, on by default. The gallery
//! lives in `examples/gallery`. See `docs/sherlock-ui.md`.

pub mod color;
pub mod cursor;
pub mod decorations;
pub mod dock;
pub mod gpu;
pub mod icon;
pub mod launcher;
pub mod logo;
pub mod metrics;
pub mod motion;
pub mod notifications;
pub mod strings;
pub mod texts;
pub mod typography;
pub mod wallpapers;
pub mod window;

#[cfg(feature = "iced")]
pub mod animated;
#[cfg(feature = "iced")]
pub mod component;
#[cfg(feature = "iced")]
pub mod focus;
#[cfg(feature = "iced")]
pub mod surface;
#[cfg(feature = "iced")]
pub mod theme;
