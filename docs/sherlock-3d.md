# sherlock-3d

`desktop/sherlock-3d` draws Sherlock, the mascot, in 3D from `sherlock.glb`, the model of the
public repository `dodolereporter/sherlock` (lot D-30, ADR 0024). Its first user is the lock
screen (D-25b): Sherlock in a round bubble, framed on the top of his body.

## The model file

`sherlock.glb` is not in this repository (rights on the mascot, size). Get it with:

```sh
just mascot            # into ~/.local/share/sherlockos/mascot/sherlock.glb
just mascot /some/file # anywhere else
```

The recipe downloads it at the revision pinned in the `justfile` (`mascot_revision`) and checks
its SHA-256 (`mascot_sha256`); it does nothing when the file is already there and right. To
follow a new version of the model, change both values together.

The crate looks for the file, in order (`locate::candidates`):

1. the file named by `SHERLOCKOS_MASCOT`;
2. `$XDG_DATA_HOME/sherlockos/mascot/sherlock.glb`;
3. `~/.local/share/sherlockos/mascot/sherlock.glb`;
4. `sherlockos/mascot/sherlock.glb` under every folder of `$XDG_DATA_DIRS`
   (`/usr/local/share:/usr/share` when unset), where the OS installs it.

Never change the model here: it is produced by Blender scripts in its own repository.

## What is in it

| Part | Content |
| --- | --- |
| Skeleton | 30 bones; `eye_L` and `eye_R` are driven by code only, `eyelid_L` and `eyelid_R` are separate |
| Mesh | One mesh, 14 primitives: position, normal, UV, color, joints, weights |
| Materials | 14, unlit: the soft shadows are baked in the vertex colors |
| Textures | 2 JPEG (the scarf's knit and its normal map, which is ignored) |
| Clips | 30: loops, entries and exits, reactions, face layers |

## Modules

| Module | Role |
| --- | --- |
| `model` | Reads the `.glb`: nodes and their bind pose, skin, vertices, materials, textures with their mipmaps, clips |
| `locate` | Where the file is looked for |
| `clip` | Samples a channel (LINEAR, STEP, CUBICSPLINE), lays a clip over a pose with a weight per bone, cross-fades two poses, computes the skinning matrices |
| `render` | The wgpu renderer: scene pass and composite pass, camera, round cut, off-screen capture |
| `headless` | A wgpu device without a window (Vulkan backend only) |

## Playing the clips

The rules come from the model's repository (`docs/contexte.md`, Sherlock, the mascot):

- A body action is sampled over the bind pose: it sets the whole body and hides the
  accessories (scale 0). Without one, every accessory shows: `repos` starts at load.
- Two actions cross-fade over 0.25 s (`Pose::blend`).
- A face layer (`cligne`, `yeux_fermes`, `bouche_parle`) is laid over the action, on the bones
  it drives only (`Clip::apply` with a mask). Never fade to a layer alone: the body would fall
  back to the T-pose.
- `yeux_fermes` ends with the eyes closed: hold its last frame, then fade back.
- A mask closes one eye alone: `yeux_fermes` on `eyelid_R` at full weight, on `eyelid_L` at 0.6
  (the eye stays 40 % open).

## Rendering

Two passes per frame:

1. **Scene.** The skinned mesh in a texture of the size of the widget, `Rgba8UnormSrgb`,
   multisampled ×4 with a 32-bit depth buffer, cleared transparent, then resolved. Color =
   vertex color × base color × texture, written premultiplied; materials are double-sided.
   Opaque parts first, then any blended part (the model has none).
2. **Composite.** The resolved texture laid over the target in premultiplied alpha, like iced,
   cut to a disc (`Cut::Circle`, one pixel of antialiasing) or not, over an optional background.
   When the target has no sRGB format (iced with web colors), the shader encodes to sRGB itself.

`Camera::UPPER_BODY` frames the cap, the face, the scarf and the top of the body, legs out of
the frame; `Camera::FULL_BODY` the whole mascot.

The renderer uses the wgpu of iced (`=27.0.1`, Vulkan only, ADR 0018). A program picks the
driver first (`sherlock_ui::gpu::Choice`): only the AMD chip that drives the screen, never the
NVIDIA card.

## Tests

```sh
just mascot
cargo test -p sherlock-3d
```

| Test | What it checks |
| --- | --- |
| `tests/model.rs` | Counts of bones, clips, primitives, materials, textures; no clip drives the eyes; sampling on and between keys; `yeux_fermes` ends closed; the bone mask closes one eye; cross-fade; skinning |
| `tests/offscreen.rs` | Renders off screen and checks pixels: transparent corners, opaque face, antialiased edges, nothing outside the bubble, its background inside |

Without the model, or without a Vulkan driver, the tests say why and pass.
`SHERLOCK_3D_REQUIRE=1` turns that into a failure: the CI sets it, downloads the model with
`just mascot` and installs Mesa's software Vulkan driver (lavapipe). `tests/offscreen.rs` picks
the AMD chip as the shell does, or else lavapipe, never another driver; it runs without libtest's
harness since it changes the environment. Captures go to `target/tmp/sherlock-3d/`, never into
the repository.
