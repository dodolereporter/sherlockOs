//! Animation clips and the poses they produce.
//!
//! A pose holds the local transform of every node. A body action is sampled over the bind
//! pose; a face layer (`cligne`, `yeux_fermes`, `bouche_parle`) is then laid over it with a
//! weight per node, which is how one eye alone can close.

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::model::{Model, Transform};

/// Which part of a node's transform a channel drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Property {
    Translation,
    Rotation,
    Scale,
}

/// How a channel goes from one key to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
    /// Hermite curve: each key holds an in-tangent, a value and an out-tangent.
    CubicSpline,
}

/// The keys of one property of one node. Translations and scales leave `w` at 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub node: usize,
    pub property: Property,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: Vec<[f32; 4]>,
}

impl Channel {
    /// The value at `time`, held before the first key and after the last.
    pub fn sample(&self, time: f32) -> Vec4 {
        let keys = self.times.len();
        if keys == 0 {
            return Vec4::ZERO;
        }
        let value = |key: usize| {
            let at = match self.interpolation {
                Interpolation::CubicSpline => key * 3 + 1,
                _ => key,
            };
            Vec4::from(self.values.get(at).copied().unwrap_or_default())
        };
        let after = self.times.partition_point(|&t| t <= time);
        if after == 0 {
            return value(0);
        }
        if after == keys {
            return value(keys - 1);
        }
        let (i, j) = (after - 1, after);
        let span = self.times[j] - self.times[i];
        let t = if span > 0.0 {
            (time - self.times[i]) / span
        } else {
            0.0
        };
        match self.interpolation {
            Interpolation::Step => value(i),
            Interpolation::Linear => match self.property {
                Property::Rotation => {
                    let a = Quat::from_vec4(value(i));
                    let b = Quat::from_vec4(value(j));
                    Vec4::from(a.slerp(b, t))
                }
                _ => value(i).lerp(value(j), t),
            },
            Interpolation::CubicSpline => {
                let get = |at: usize| Vec4::from(self.values.get(at).copied().unwrap_or_default());
                let (p0, m0) = (get(i * 3 + 1), get(i * 3 + 2) * span);
                let (p1, m1) = (get(j * 3 + 1), get(j * 3) * span);
                let (t2, t3) = (t * t, t * t * t);
                let v = p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
                    + m0 * (t3 - 2.0 * t2 + t)
                    + p1 * (-2.0 * t3 + 3.0 * t2)
                    + m1 * (t3 - t2);
                match self.property {
                    Property::Rotation => v.normalize_or_zero(),
                    _ => v,
                }
            }
        }
    }
}

/// A named animation of the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Time of the last key, in seconds.
    pub duration: f32,
    pub channels: Vec<Channel>,
}

impl Clip {
    /// Lays the clip at `time` over `pose`. Each node moves toward the clip's value by `weight`
    /// times what `mask` gives it (0 leaves the node alone, 1 replaces its value).
    pub fn apply(&self, pose: &mut Pose, time: f32, weight: f32, mask: impl Fn(usize) -> f32) {
        for channel in &self.channels {
            let w = weight * mask(channel.node);
            if w <= 0.0 {
                continue;
            }
            let Some(transform) = pose.transforms.get_mut(channel.node) else {
                continue;
            };
            let value = channel.sample(time);
            match channel.property {
                Property::Translation => {
                    transform.translation = transform.translation.lerp(value.truncate(), w);
                }
                Property::Scale => transform.scale = transform.scale.lerp(value.truncate(), w),
                Property::Rotation => {
                    let target = Quat::from_vec4(value).normalize();
                    transform.rotation = transform.rotation.slerp(target, w).normalize();
                }
            }
        }
    }

    /// Whether the clip drives `node`.
    pub fn drives(&self, node: usize) -> bool {
        self.channels.iter().any(|c| c.node == node)
    }
}

/// The local transform of every node of the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    pub transforms: Vec<Transform>,
}

impl Pose {
    /// The bind pose: a T-pose, every accessory shown.
    pub fn rest(model: &Model) -> Self {
        Self {
            transforms: model.nodes.iter().map(|n| n.rest).collect(),
        }
    }

    /// The bind pose with `clip` at `time` over it: what a body action shows on its own.
    pub fn sampled(model: &Model, clip: &Clip, time: f32) -> Self {
        let mut pose = Self::rest(model);
        clip.apply(&mut pose, time, 1.0, |_| 1.0);
        pose
    }

    /// Cross-fade: `t` is 0 for `self`, 1 for `other`.
    pub fn blend(&self, other: &Self, t: f32) -> Self {
        Self {
            transforms: self
                .transforms
                .iter()
                .zip(&other.transforms)
                .map(|(a, b)| a.lerp(b, t))
                .collect(),
        }
    }

    /// The transform of every node in the space of the model.
    pub fn globals(&self, model: &Model) -> Vec<Mat4> {
        let mut globals = vec![Mat4::IDENTITY; self.transforms.len()];
        for &node in &model.order {
            let local = self.transforms[node].matrix();
            globals[node] = match model.nodes[node].parent {
                Some(parent) => globals[parent] * local,
                None => local,
            };
        }
        globals
    }

    /// The skinning matrices, in the order of the skin's joints.
    pub fn joint_matrices(&self, model: &Model) -> Vec<Mat4> {
        let globals = self.globals(model);
        model
            .joints
            .iter()
            .zip(&model.inverse_bind)
            .map(|(&joint, inverse)| globals[joint] * *inverse)
            .collect()
    }

    /// Where `node` sits in the space of the model.
    pub fn position(&self, model: &Model, node: usize) -> Vec3 {
        self.globals(model)
            .get(node)
            .map_or(Vec3::ZERO, |m| m.w_axis.truncate())
    }
}
