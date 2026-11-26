//! Sherlock, the mascot, in 3D (lot D-30, ADR 0024).
//!
//! - `model`: reads `sherlock.glb` (skeleton, mesh, materials, textures, clips);
//! - `locate`: where the file is found, since it is not in this repository;
//! - `clip`: samples the clips into poses, cross-fades them, lays face layers over a body
//!   action with a weight per bone;
//! - `render`: draws a pose with wgpu, multisampled, cut round for the bubble;
//! - `headless`: a wgpu device without a window, for captures and tests.
//!
//! The rules for playing the clips come from the model's repository: see `docs/sherlock-3d.md`.

pub mod clip;
pub mod headless;
pub mod locate;
pub mod model;
pub mod render;

pub use clip::{Clip, Pose};
pub use model::Model;
pub use render::{Camera, Cut, Frame, Renderer};
