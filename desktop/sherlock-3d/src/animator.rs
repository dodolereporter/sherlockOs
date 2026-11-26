//! What Sherlock does: a mood, played by the rules of the model's repository.
//!
//! - A body action plays at all times: `repos` from the start, so no accessory shows by
//!   mistake. Two actions cross-fade over 0.25 s.
//! - A reaction (`salut`, `desole`, `reussite`) plays once, then `repos` comes back. A state
//!   with an entry (`dort`, `reflechit`) plays its entry, then loops; leaving it plays its exit.
//! - The face layers play over the action, never alone: `yeux_fermes` is held on its last frame
//!   while the eyes stay closed, then fades back; `cligne` comes now and then while they are
//!   open. A weight per eyelid closes one eye alone (`Mood::Peek`).
//! - The eyes (`eye_L`, `eye_R`, which no clip drives) turn toward a point given by code.

use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};

use crate::clip::Pose;
use crate::model::Model;

/// Length of a cross-fade between two body actions, and of the eyes reopening.
pub const FADE: f32 = 0.25;
/// How closed the open eye of `Mood::Peek` is: it stays 40 % open.
pub const PEEK_CLOSURE: f32 = 0.6;
/// Head roll of `Mood::Peek`, in radians (about 8 degrees).
pub const PEEK_TILT: f32 = 0.14;
/// How far an eye turns toward what it looks at, in radians: further, it would slide off the
/// face.
pub const GAZE_LIMIT: f32 = 0.22;
/// Shortest and longest wait between two blinks, in seconds.
const BLINK_WAIT: (f32, f32) = (2.5, 6.0);
/// Rate of the exponential smoothing of the gaze and of the head roll, per second.
const SMOOTHING: f32 = 10.0;

/// What Sherlock shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mood {
    /// At rest, blinking now and then.
    #[default]
    Idle,
    /// Waves hello, then rests.
    Wave,
    /// Closes both eyes and keeps them closed.
    EyesClosed,
    /// One eye closed, the other ajar, the head slightly tilted: curious and a bit worried.
    Peek,
    /// Sorry, then rests.
    Sorry,
    /// Happy, then rests.
    Success,
    /// Falls asleep and sleeps.
    Sleep,
    /// Thinks.
    Think,
}

impl Mood {
    /// The body actions of the mood, in order: every one but the last plays once.
    fn plan(self) -> &'static [&'static str] {
        match self {
            Self::Idle | Self::EyesClosed | Self::Peek => &["repos"],
            Self::Wave => &["salut", "repos"],
            Self::Sorry => &["desole", "repos"],
            Self::Success => &["reussite", "repos"],
            Self::Sleep => &["dort_entree", "dort"],
            Self::Think => &["reflechit_entree", "reflechit"],
        }
    }

    /// The clip that brings the body back to the neutral pose when the mood ends.
    fn exit(self) -> Option<&'static str> {
        match self {
            Self::Sleep => Some("dort_sortie"),
            Self::Think => Some("reflechit_sortie"),
            _ => None,
        }
    }

    /// How closed each eye is, left then right.
    fn eyes(self) -> (f32, f32) {
        match self {
            Self::EyesClosed => (1.0, 1.0),
            Self::Peek => (PEEK_CLOSURE, 1.0),
            _ => (0.0, 0.0),
        }
    }
}

/// A body action under way.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Action {
    clip: usize,
    time: f32,
    looping: bool,
}

/// Bones the face uses, found once.
#[derive(Debug, Clone, Copy)]
struct Face {
    eyelids: [usize; 2],
    arcs: [usize; 2],
    eyes: [usize; 2],
    head: usize,
    closing: usize,
    blink: usize,
}

/// Plays a mood on the model, frame after frame.
#[derive(Debug, Clone)]
pub struct Animator {
    model: Arc<Model>,
    face: Option<Face>,
    mood: Mood,
    action: Option<Action>,
    /// Actions still to come; the last one loops.
    queue: Vec<usize>,
    /// The pose being faded from, and how long ago the fade started.
    fading: Option<(Pose, f32)>,
    /// Time in `yeux_fermes`, held at its end.
    closing_time: f32,
    /// Weight of `yeux_fermes` on each eye, left then right.
    closure: [f32; 2],
    /// Time in `cligne`, or `None` between blinks.
    blink_time: Option<f32>,
    until_blink: f32,
    seed: u32,
    tilt: f32,
    gaze_target: Option<Vec3>,
    /// Rotation of each eye toward its target, in the space of its rest pose.
    gaze: [Quat; 2],
}

impl Animator {
    /// Starts at rest: `repos` at once, as the model's notes ask.
    pub fn new(model: Arc<Model>) -> Self {
        let face = (|| {
            let node = |name| model.node(name);
            let clip = |name| model.clips.iter().position(|c| c.name == name);
            Some(Face {
                eyelids: [node("eyelid_L")?, node("eyelid_R")?],
                arcs: [node("eye_happy_L")?, node("eye_happy_R")?],
                eyes: [node("eye_L")?, node("eye_R")?],
                head: node("head")?,
                closing: clip("yeux_fermes")?,
                blink: clip("cligne")?,
            })
        })();
        let mut animator = Self {
            model,
            face,
            mood: Mood::Idle,
            action: None,
            queue: Vec::new(),
            fading: None,
            closing_time: 0.0,
            closure: [0.0; 2],
            blink_time: None,
            until_blink: BLINK_WAIT.0,
            seed: 0x5EED,
            tilt: 0.0,
            gaze_target: None,
            gaze: [Quat::IDENTITY; 2],
        };
        animator.queue = animator.clips(Mood::Idle.plan());
        animator.next_action();
        animator.fading = None;
        animator
    }

    pub fn model(&self) -> &Arc<Model> {
        &self.model
    }

    pub fn mood(&self) -> Mood {
        self.mood
    }

    /// Changes the mood. The same mood again changes nothing; a reaction asked again while it
    /// plays does not restart.
    pub fn set_mood(&mut self, mood: Mood) {
        if mood == self.mood {
            return;
        }
        let (left, right) = mood.eyes();
        if self.closure == [0.0; 2] && (left > 0.0 || right > 0.0) {
            // The eyes start closing: the layer plays from its first frame.
            self.closing_time = 0.0;
        }
        let mut plan = Vec::new();
        if let Some(exit) = self.mood.exit() {
            plan.extend(self.clips(&[exit]));
        }
        plan.extend(self.clips(mood.plan()));
        self.mood = mood;
        // A loop that is already playing goes on (Idle, EyesClosed and Peek share `repos`).
        if let Some(action) = self.action
            && action.looping
            && plan.first() == Some(&action.clip)
        {
            plan.remove(0);
            if plan.is_empty() {
                return;
            }
        }
        self.queue = plan;
        self.next_action();
    }

    /// Turns the eyes toward `target`, in the space of the model, or back ahead with `None`.
    pub fn look_at(&mut self, target: Option<Vec3>) {
        self.gaze_target = target;
    }

    /// Moves time on by `dt` seconds.
    pub fn advance(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.25);
        if let Some((_, elapsed)) = &mut self.fading {
            *elapsed += dt;
            if *elapsed >= FADE {
                self.fading = None;
            }
        }
        if let Some(action) = &mut self.action {
            action.time += dt;
            let duration = self.model.clips[action.clip].duration;
            if !action.looping && action.time >= duration && !self.queue.is_empty() {
                self.next_action();
            }
        }

        // Eyes: close at once along the clip, reopen by a fade from its last frame.
        let (left, right) = self.mood.eyes();
        self.closing_time += dt;
        let step = dt / FADE;
        for (weight, target) in self.closure.iter_mut().zip([left, right]) {
            *weight = if target >= *weight {
                target
            } else {
                (*weight - step).max(target)
            };
        }

        // Blinks, while both eyes are open.
        match &mut self.blink_time {
            Some(time) => {
                *time += dt;
                let length = self
                    .face
                    .map_or(0.0, |f| self.model.clips[f.blink].duration);
                if *time >= length {
                    self.blink_time = None;
                    self.until_blink = self.random_wait();
                }
            }
            None => {
                self.until_blink -= dt;
                if self.until_blink <= 0.0 {
                    if self.closure == [0.0; 2] {
                        self.blink_time = Some(0.0);
                    } else {
                        self.until_blink = self.random_wait();
                    }
                }
            }
        }

        let smoothing = 1.0 - (-SMOOTHING * dt).exp();
        let tilt = if self.mood == Mood::Peek {
            PEEK_TILT
        } else {
            0.0
        };
        self.tilt += (tilt - self.tilt) * smoothing;
        let wanted = self.gaze_wanted();
        for (gaze, wanted) in self.gaze.iter_mut().zip(wanted) {
            *gaze = gaze.slerp(wanted, smoothing).normalize();
        }
    }

    /// The pose of this frame.
    pub fn pose(&self) -> Pose {
        let mut pose = self.body();
        let Some(face) = self.face else {
            return pose;
        };
        let closing = &self.model.clips[face.closing];
        let time = self.closing_time.min(closing.duration);
        let [left, right] = self.closure;
        // The arcs of closed eyes only show when an eye is (nearly) shut.
        let arc = |w: f32| ((w - 0.8) / 0.2).clamp(0.0, 1.0);
        closing.apply(&mut pose, time, 1.0, |node| {
            if node == face.eyelids[0] {
                left
            } else if node == face.eyelids[1] {
                right
            } else if node == face.arcs[0] {
                arc(left)
            } else if node == face.arcs[1] {
                arc(right)
            } else {
                0.0
            }
        });
        if let Some(time) = self.blink_time {
            let open = 1.0 - left.max(right);
            self.model.clips[face.blink].apply(&mut pose, time, open, |_| 1.0);
        }
        let head = &mut pose.transforms[face.head];
        head.rotation = (head.rotation * Quat::from_rotation_z(self.tilt)).normalize();
        for (eye, gaze) in face.eyes.iter().zip(self.gaze) {
            let eye = &mut pose.transforms[*eye];
            eye.rotation = (eye.rotation * gaze).normalize();
        }
        pose
    }

    /// The skinning matrices of this frame.
    pub fn joints(&self) -> Vec<Mat4> {
        self.pose().joint_matrices(&self.model)
    }

    /// How closed each eye is, left then right: 0 open, 1 closed.
    pub fn closure(&self) -> [f32; 2] {
        self.closure
    }

    /// The name of the body action playing.
    pub fn action(&self) -> Option<&str> {
        self.action.map(|a| self.model.clips[a.clip].name.as_str())
    }

    /// The body action alone, cross-faded from the previous one.
    fn body(&self) -> Pose {
        let model = &self.model;
        let current = match self.action {
            Some(action) => {
                let clip = &model.clips[action.clip];
                let time = if action.looping && clip.duration > 0.0 {
                    action.time % clip.duration
                } else {
                    action.time.min(clip.duration)
                };
                Pose::sampled(model, clip, time)
            }
            None => Pose::rest(model),
        };
        match &self.fading {
            Some((from, elapsed)) => from.blend(&current, smooth(*elapsed / FADE)),
            None => current,
        }
    }

    /// Starts the next action of the queue, fading from the pose shown now.
    fn next_action(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let from = self.body();
        let clip = self.queue.remove(0);
        self.action = Some(Action {
            clip,
            time: 0.0,
            looping: self.queue.is_empty(),
        });
        self.fading = Some((from, 0.0));
    }

    fn clips(&self, names: &[&str]) -> Vec<usize> {
        names
            .iter()
            .filter_map(|name| self.model.clips.iter().position(|c| c.name == *name))
            .collect()
    }

    /// The rotation that turns each eye toward the target, limited to `GAZE_LIMIT`.
    fn gaze_wanted(&self) -> [Quat; 2] {
        let (Some(face), Some(target)) = (self.face, self.gaze_target) else {
            return [Quat::IDENTITY; 2];
        };
        let pose = self.body();
        let globals = pose.globals(&self.model);
        face.eyes.map(|eye| {
            let global = globals[eye];
            let (_, rotation, position) = global.to_scale_rotation_translation();
            // The bone points at the eye along its Y axis: the eye slides on the face as the
            // bone turns. Both directions in the space of the bone at rest.
            let toward = rotation.inverse() * (target - position);
            if toward.length_squared() < 1e-6 {
                return Quat::IDENTITY;
            }
            let full = Quat::from_rotation_arc(Vec3::Y, toward.normalize());
            let (axis, angle) = full.to_axis_angle();
            Quat::from_axis_angle(axis, angle.min(GAZE_LIMIT))
        })
    }

    /// A wait between two blinks, from a small deterministic generator.
    fn random_wait(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let unit = (self.seed >> 8) as f32 / (1u32 << 24) as f32;
        BLINK_WAIT.0 + unit * (BLINK_WAIT.1 - BLINK_WAIT.0)
    }
}

/// Ease in and out, for the cross-fades.
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
