# ADR 0024: The mascot in 3D, rendered with wgpu inside iced, model kept outside the repository

- Status: proposed
- Date: 2026-11-26
- Lot: D-30

## Context

In the OS, Sherlock is drawn from `sherlock.glb`, the model of the public repository
`dodolereporter/sherlock`: one mesh of 14 primitives, a skeleton of 30 bones, 30 clips, two
JPEG textures, no morph target. Its soft shadows are baked in the vertex colors, so it needs no
lighting. Its first use is the lock screen (D-25b): Sherlock animated in a round bubble, inside
an iced program. Two questions:

- **How to draw it.** iced 0.14 already renders the shell with wgpu on Vulkan, on the AMD chip
  (ADR 0018), and offers a `shader` widget whose primitives get the wgpu device. A 3D engine
  (Bevy, three-d, rend3) would bring its own window, device and dependency tree, for a model that
  needs a single unlit shader.
- **Where the model lives.** The rights on the mascot in a published OS are not settled
  (`docs/contexte.md`), and the file weighs 1.4 MiB. This repository is public.

## Decision

**Rendering.** A crate of our own, `desktop/sherlock-3d`:

- reads the file with the `gltf` crate (pure Rust, without its `import` feature) and decodes the
  textures with the `image` crate the workspace already has;
- animates on the CPU: channels sampled (LINEAR, STEP, CUBICSPLINE), cross-fades of 0.25 s
  between body actions, face layers laid over the action with a weight per bone (one eye can
  close alone), eyes aimed by code;
- draws with the wgpu that iced pulls (`=27.0.1`, Vulkan only): one WGSL shader, skinning with
  4 bones per vertex, color = vertex color × base color × texture, premultiplied alpha, in a
  texture of its own with MSAA ×4 and depth, then laid over iced's target, cut round, by a
  second small shader;
- shows in iced through `iced::widget::shader` (lot D-30b). In software rendering (tiny-skia),
  the widget shows a still image instead.

**Model.** `sherlock.glb` is not committed. `just mascot` downloads it from
`dodolereporter/sherlock` at a pinned revision (`5af31dc4`) and checks its SHA-256. The crate
looks for it, in order, in `SHERLOCKOS_MASCOT`, `$XDG_DATA_HOME/sherlockos/mascot/`,
`~/.local/share/sherlockos/mascot/`, then `sherlockos/mascot/` under every folder of
`$XDG_DATA_DIRS`, where the OS package will install it. Without the file, the program shows the
still image and the tests skip with a message, except in the CI, which downloads the model and
renders on Mesa's software Vulkan driver (lavapipe), with `SHERLOCK_3D_REQUIRE=1`.

Test captures go to `target/`. No new picture of Sherlock rendered in high definition enters the
repository.

## Consequences

- One new dependency, `gltf =1.4.1` (with `gltf-json` and `gltf-derive`); `glam`, `bytemuck`,
  `image` and `wgpu` were already in the workspace.
- A program that shows Sherlock pays, on the GPU, two render passes per frame at 30 frames per
  second at most, and nothing while the widget is hidden or paused.
- The lock screen and the login screen depend on a file outside the repository: the OS image
  must install it, or they fall back to the still image.
- The renderer only knows what this model uses: one skin, no morph target, no lighting, no
  normal map (the scarf's is ignored). A richer model would need a new look at this choice.
- The CI installs `mesa-vulkan-drivers` and downloads 1.4 MiB from GitHub on each run.

## What would make us reconsider

- The author decides the mascot may live in this repository, or must not be downloaded from
  GitHub at all (then: a package of its own).
- iced's `shader` widget changes in a way that forbids a pass of our own, or wgpu leaves iced.
- The mascot needs lighting, morph targets or effects a single unlit shader cannot give.
