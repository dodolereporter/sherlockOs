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
| `animator` | The moods (`Mood`) and the `Animator` that plays them frame after frame (lot D-30b) |
| `widget` | `Sherlock3d`, the iced widget (lot D-30b) |

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

## Moods

`Animator` plays a `Mood` by those rules; `set_mood` changes it, `look_at` aims the eyes,
`advance(dt)` moves time on, `pose` and `joints` give the frame.

| Mood | Body | Face |
| --- | --- | --- |
| `Idle` | `repos` | `cligne` every 2.5 to 6 s |
| `Wave` | `salut` once, then `repos` | |
| `EyesClosed` | `repos` | `yeux_fermes`, last frame held |
| `Peek` | `repos`, head rolled by 0.14 rad | `yeux_fermes` on the right eye; the left one 40 % open |
| `Sorry` | `desole` once, then `repos` | |
| `Success` | `reussite` once, then `repos` | |
| `Sleep` | `dort_entree`, then `dort`, its exit clip when it ends | |
| `Think` | `reflechit_entree`, then `reflechit`, its exit clip when it ends | |

- A loop already playing goes on: `Idle`, `EyesClosed` and `Peek` share `repos`, so going from
  one to the other only moves the eyes.
- Every change of body action cross-fades from the pose shown, eased, over `FADE` (0.25 s).
- The eyes close along `yeux_fermes` and reopen by a fade from its last frame. The arcs of
  closed eyes (`eye_happy_*`) only rise when an eye is nearly shut, so the ajar eye of `Peek`
  shows no arc. Blinks only come while both eyes are open.
- `look_at(Some(point))` turns `eye_L` and `eye_R` toward a point of the model's space, by
  `GAZE_LIMIT` (0.22 rad) at most, smoothed; `None` looks ahead again.

## The widget

```rust
let mascot = Sherlock3d::new(model.clone()) // Option<Arc<Model>>
    .mood(Mood::Peek)
    .look_at(Some(field_center))            // relative to the widget, may lie outside
    .background(palette.system_discs[1].into())
    .width(side)
    .height(side);
```

- On the GPU, it is an `iced::widget::shader`: its `Program` keeps an `Animator` in the
  widget state and moves it on each `RedrawRequested`; its `Primitive` uploads the frame in
  `prepare` and draws the two passes in `render`, the composite limited by a scissor to the
  widget's area; its `Pipeline` keeps one renderer per widget and drops those not drawn in a
  frame (`trim`).
- It asks for the next frame `FRAME` (1/30 s) later: 30 frames per second at most. Out of the
  view it gets no event and asks for nothing; `.paused(true)` stops on the frame shown and asks
  for nothing either.
- `.cut(Cut::Circle { radius })` cuts the bubble (by default, radius 1: the disc touches the
  smaller side); `Cut::None` keeps the rectangle. `.camera(Camera::FULL_BODY)` shows the whole
  mascot.
- In software (tiny-skia), or without the model file, it shows the still picture of the design
  boards (`docs/design/images/sherlock-neutre.png`), cropped to its top square and rounded, and
  asks for no frame. It knows the renderer when drawing, as `sherlock_ui::surface` does.
- The gaze point is projected on a plane in front of the face (`Camera::unproject`).

`examples/viewer` goes through the moods, one every three seconds. It opens a window: never run
it in the CI.

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
| `tests/animator.rs` | `repos` at start with the accessories hidden; `Peek` closes `eyelid_R` only, `eyelid_L` ajar, head tilted; closed eyes hold, then reopen by a fade; a reaction fades in without a jump and `repos` comes back; `Sleep` enters, loops and leaves by its exit; the gaze turns within its limit |
| `tests/captures.rs` | Captures rest, wave (mid-gesture), eyes closed, peek and sorry in the bubble, dark and light, at 170 and 340 pixels, and checks that the eyes differ |

Without the model, or without a Vulkan driver, the tests say why and pass.
`SHERLOCK_3D_REQUIRE=1` turns that into a failure: the CI sets it, downloads the model with
`just mascot` and installs Mesa's software Vulkan driver (lavapipe). `tests/offscreen.rs` picks
the AMD chip as the shell does, or else lavapipe, never another driver; it runs without libtest's
harness since it changes the environment, like `tests/captures.rs`. Captures go to
`target/tmp/sherlock-3d/`, never into the repository.
