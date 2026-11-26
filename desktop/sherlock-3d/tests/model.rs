//! Reading the model and sampling its clips. Skipped, with a message, without `sherlock.glb`.

mod support;

use sherlock_3d::clip::{Interpolation, Property};
use sherlock_3d::{Model, Pose};

fn model() -> Option<Model> {
    support::model()
}

/// An eyelid is closed when its height falls to the 0.07 of the model's notes.
fn closed(pose: &Pose, model: &Model, name: &str) -> bool {
    let Some(node) = model.node(name) else {
        panic!("no bone {name}");
    };
    pose.transforms[node].scale.min_element() < 0.2
}

#[test]
fn the_model_has_its_skeleton_mesh_and_clips() {
    let Some(model) = model() else { return };
    assert_eq!(model.joints.len(), 30, "bones");
    assert_eq!(model.inverse_bind.len(), 30);
    assert_eq!(model.clips.len(), 30, "clips");
    assert_eq!(model.primitives.len(), 14, "primitives");
    assert_eq!(model.materials.len(), 14, "materials");
    assert_eq!(model.textures.len(), 2, "textures");
    assert!(model.indices.len() % 3 == 0 && !model.indices.is_empty());
    for bone in [
        "head", "body", "eye_L", "eye_R", "eyelid_L", "eyelid_R", "arm_L", "prop_z",
    ] {
        assert!(model.node(bone).is_some(), "{bone}");
    }
    for clip in [
        "repos",
        "salut",
        "yeux_fermes",
        "cligne",
        "desole",
        "reussite",
        "dort",
        "dort_entree",
        "reflechit",
        "bouche_parle",
    ] {
        assert!(model.clip(clip).is_some(), "{clip}");
    }
    // Every vertex points at a bone of the skin, with weights that add up to one.
    for vertex in &model.vertices {
        assert!(vertex.joints.iter().all(|&j| usize::from(j) < 30));
        let sum: f32 = vertex.weights.iter().sum();
        assert!((sum - 1.0).abs() < 0.01, "{sum}");
    }
}

#[test]
fn no_clip_drives_the_eyes() {
    let Some(model) = model() else { return };
    for eye in ["eye_L", "eye_R"] {
        let node = model.node(eye).unwrap();
        for clip in &model.clips {
            assert!(!clip.drives(node), "{} drives {eye}", clip.name);
        }
    }
}

#[test]
fn sampling_hits_the_keys_and_interpolates_between_them() {
    let Some(model) = model() else { return };
    let repos = model.clip("repos").unwrap();
    assert!((repos.duration - 3.0).abs() < 0.01, "{}", repos.duration);
    let mut linear = 0;
    let mut step = 0;
    for channel in &repos.channels {
        for (key, &time) in channel.times.iter().enumerate() {
            let value = glam::Vec4::from(channel.values[key]);
            assert!(channel.sample(time).abs_diff_eq(value, 1e-5));
        }
        // Before the first key and after the last, the value is held.
        let last = glam::Vec4::from(*channel.values.last().unwrap());
        assert!(channel.sample(100.0).abs_diff_eq(last, 1e-6));
        let (a, b) = (channel.values[0], channel.values[1]);
        let middle = (channel.times[0] + channel.times[1]) / 2.0;
        let sampled = channel.sample(middle);
        match channel.interpolation {
            Interpolation::Step => {
                step += 1;
                assert_eq!(sampled, glam::Vec4::from(a));
            }
            Interpolation::Linear if channel.property != Property::Rotation => {
                linear += 1;
                let expected = (glam::Vec4::from(a) + glam::Vec4::from(b)) / 2.0;
                assert!(sampled.abs_diff_eq(expected, 1e-5));
            }
            Interpolation::Linear => {
                linear += 1;
                // Half way along the arc: as far from both ends.
                let q = glam::Quat::from_vec4(sampled);
                let (qa, qb) = (glam::Quat::from_array(a), glam::Quat::from_array(b));
                assert!((q.angle_between(qa) - q.angle_between(qb)).abs() < 1e-3);
                assert!((q.length() - 1.0).abs() < 1e-4);
            }
            Interpolation::CubicSpline => {}
        }
    }
    assert!(linear > 0 && step > 0, "repos has both kinds of keys");
}

#[test]
fn closing_the_eyes_ends_closed() {
    let Some(model) = model() else { return };
    let repos = model.clip("repos").unwrap();
    let closing = model.clip("yeux_fermes").unwrap();
    let mut pose = Pose::sampled(&model, repos, 0.5);
    assert!(!closed(&pose, &model, "eyelid_L") && !closed(&pose, &model, "eyelid_R"));
    closing.apply(&mut pose, closing.duration, 1.0, |_| 1.0);
    assert!(closed(&pose, &model, "eyelid_L") && closed(&pose, &model, "eyelid_R"));
    // The layer moves the face only: the head keeps the action's pose.
    let head = model.node("head").unwrap();
    assert_eq!(
        pose.transforms[head],
        Pose::sampled(&model, repos, 0.5).transforms[head]
    );
}

#[test]
fn a_bone_mask_closes_one_eye_and_half_closes_the_other() {
    let Some(model) = model() else { return };
    let repos = model.clip("repos").unwrap();
    let closing = model.clip("yeux_fermes").unwrap();
    let (left, right) = (
        model.node("eyelid_L").unwrap(),
        model.node("eyelid_R").unwrap(),
    );
    let open = Pose::sampled(&model, repos, 0.0);
    let mut pose = open.clone();
    closing.apply(&mut pose, closing.duration, 1.0, |node| match node {
        n if n == right => 1.0,
        n if n == left => 0.6,
        _ => 0.0,
    });
    assert!(closed(&pose, &model, "eyelid_R"));
    assert!(!closed(&pose, &model, "eyelid_L"));
    let height = |p: &Pose| p.transforms[left].scale.min_element();
    assert!(height(&pose) < height(&open) - 0.4, "the left eye is ajar");
}

#[test]
fn a_cross_fade_goes_from_one_pose_to_the_other() {
    let Some(model) = model() else { return };
    let from = Pose::sampled(&model, model.clip("repos").unwrap(), 0.0);
    let to = Pose::sampled(&model, model.clip("salut").unwrap(), 0.75);
    assert_eq!(from.blend(&to, 0.0), from);
    let end = from.blend(&to, 1.0);
    for (a, b) in end.transforms.iter().zip(&to.transforms) {
        assert!(a.translation.abs_diff_eq(b.translation, 1e-5));
        assert!(
            a.rotation.abs_diff_eq(b.rotation, 1e-5) || a.rotation.abs_diff_eq(-b.rotation, 1e-5)
        );
    }
    // Half way, every translation sits in the middle.
    let middle = from.blend(&to, 0.5);
    for ((m, a), b) in middle
        .transforms
        .iter()
        .zip(&from.transforms)
        .zip(&to.transforms)
    {
        assert!(
            m.translation
                .abs_diff_eq((a.translation + b.translation) / 2.0, 1e-5)
        );
    }
    // An arm waves.
    let moved = ["arm_L", "arm_R", "forearm_L", "forearm_R"]
        .iter()
        .any(|bone| {
            let node = model.node(bone).unwrap();
            from.transforms[node] != to.transforms[node]
        });
    assert!(moved);
}

#[test]
fn the_skin_follows_the_pose() {
    let Some(model) = model() else { return };
    // In the bind pose every skinning matrix is the identity.
    for matrix in Pose::rest(&model).joint_matrices(&model) {
        assert!(matrix.abs_diff_eq(glam::Mat4::IDENTITY, 1e-3), "{matrix}");
    }
    let pose = Pose::sampled(&model, model.clip("repos").unwrap(), 0.0);
    let head = pose.position(&model, model.node("head").unwrap());
    assert!(head.y > 1.5 && head.y < 2.5, "{head}");
}
