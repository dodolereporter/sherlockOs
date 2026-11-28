//! The moods, played on the model. Skipped, with a message, without `sherlock.glb`.

mod support;

use std::sync::Arc;

use sherlock_3d::animator::{FADE, GAZE_DOWN, GAZE_LIMIT, GAZE_UP, PEEK_OPENING, PEEK_TILT};
use sherlock_3d::{Animator, Camera, Model, Mood, Pose};

fn animator() -> Option<Animator> {
    support::model().map(|model| Animator::new(Arc::new(model)))
}

fn node(model: &Model, name: &str) -> usize {
    let Some(node) = model.node(name) else {
        panic!("no bone {name}");
    };
    node
}

/// The height of an eyelid: 1 open, 0.07 closed.
fn eyelid(pose: &Pose, model: &Model, name: &str) -> f32 {
    pose.transforms[node(model, name)].scale.min_element()
}

/// Plays `seconds` at 30 frames per second.
fn play(animator: &mut Animator, seconds: f32) {
    for _ in 0..(seconds * 30.0).round() as usize {
        animator.advance(1.0 / 30.0);
    }
}

#[test]
fn rest_plays_from_the_start_and_hides_the_accessories() {
    let Some(animator) = animator() else { return };
    assert_eq!(animator.action(), Some("repos"));
    let model = animator.model().clone();
    let pose = animator.pose();
    for prop in ["prop_loupe", "prop_ampoule", "prop_z", "prop_coche"] {
        let scale = pose.transforms[node(&model, prop)].scale;
        assert!(scale.max_element() < 1e-3, "{prop} shows: {scale}");
    }
}

#[test]
fn peek_closes_the_right_eye_only_and_tilts_the_head() {
    let Some(mut animator) = animator() else {
        return;
    };
    let model = animator.model().clone();
    let head = node(&model, "head");
    let before = animator.pose().transforms[head].rotation;
    animator.set_mood(Mood::Peek);
    play(&mut animator, 1.0);
    let pose = animator.pose();
    assert_eq!(
        eyelid(&pose, &model, "eyelid_R"),
        0.0,
        "the right eye is hidden"
    );
    // The left eye is half open: flattened to the height asked, as wide as an open eye.
    let left = pose.transforms[node(&model, "eyelid_L")].scale;
    assert!(
        (left.z - PEEK_OPENING).abs() < 0.02,
        "the left eye is ajar: {left}"
    );
    assert!(
        left.x > 0.99 && left.y > 0.99,
        "the left eye keeps its width: {left}"
    );
    let [open, _] = animator.openness();
    assert!((open - PEEK_OPENING).abs() < 0.02);
    // The rest of the body still plays `repos`; the head leans to the side.
    assert_eq!(animator.action(), Some("repos"));
    let roll = pose.transforms[head].rotation.angle_between(before);
    assert!(roll > PEEK_TILT * 0.5, "the head is tilted: {roll}");
}

#[test]
fn closed_eyes_hold_then_reopen_by_a_fade() {
    let Some(mut animator) = animator() else {
        return;
    };
    let model = animator.model().clone();
    animator.set_mood(Mood::EyesClosed);
    play(&mut animator, 5.0);
    let pose = animator.pose();
    assert!(eyelid(&pose, &model, "eyelid_L") < 0.2);
    assert!(eyelid(&pose, &model, "eyelid_R") < 0.2);
    assert_eq!(
        animator.action(),
        Some("repos"),
        "no fade to the layer alone"
    );

    animator.set_mood(Mood::Idle);
    play(&mut animator, FADE / 2.0);
    let half = eyelid(&animator.pose(), &model, "eyelid_L");
    assert!(half > 0.2 && half < 0.9, "half way: {half}");
    play(&mut animator, FADE);
    assert!(animator.openness().iter().all(|h| (h - 1.0).abs() < 1e-4));
    assert!(eyelid(&animator.pose(), &model, "eyelid_L") > 0.9);
}

#[test]
fn a_reaction_fades_in_plays_once_and_rest_comes_back() {
    let Some(mut animator) = animator() else {
        return;
    };
    play(&mut animator, 0.5);
    let before = animator.pose();
    animator.set_mood(Mood::Wave);
    assert_eq!(animator.action(), Some("salut"));
    // No jump: the first frame of the fade is the pose shown before.
    for (a, b) in animator.pose().transforms.iter().zip(&before.transforms) {
        assert!(a.translation.abs_diff_eq(b.translation, 1e-4));
    }
    let salut = animator.model().clip("salut").map(|c| c.duration).unwrap();
    play(&mut animator, salut + FADE + 0.1);
    assert_eq!(animator.action(), Some("repos"));
    // Asked again, the same mood changes nothing.
    animator.set_mood(Mood::Wave);
    assert_eq!(animator.action(), Some("repos"));
}

#[test]
fn sleep_enters_loops_and_leaves_through_its_exit() {
    let Some(mut animator) = animator() else {
        return;
    };
    animator.set_mood(Mood::Sleep);
    assert_eq!(animator.action(), Some("dort_entree"));
    play(&mut animator, 2.0);
    assert_eq!(animator.action(), Some("dort"));
    animator.set_mood(Mood::Idle);
    assert_eq!(animator.action(), Some("dort_sortie"));
    play(&mut animator, 2.0);
    assert_eq!(animator.action(), Some("repos"));
}

#[test]
fn the_eyes_turn_toward_a_point_within_a_limit() {
    let Some(mut animator) = animator() else {
        return;
    };
    let model = animator.model().clone();
    let eye = node(&model, "eye_L");
    let ahead = animator.pose().transforms[eye].rotation;
    // Far down and to the side: the limit holds, between the one down and the one aside.
    animator.look_at(Some(glam::Vec3::new(-8.0, -6.0, 4.0)));
    play(&mut animator, 1.0);
    let turned = animator.pose().transforms[eye].rotation;
    let angle = turned.angle_between(ahead);
    assert!(
        angle > GAZE_DOWN * 0.9 && angle < GAZE_LIMIT * 1.01,
        "{angle}"
    );
    animator.look_at(None);
    play(&mut animator, 1.0);
    assert!(
        animator.pose().transforms[eye]
            .rotation
            .angle_between(ahead)
            < 0.01
    );
}

/// Whether the arc of a closed eye is out of the skin (its translation goes from 0.71 to 0.85).
fn arc_out(pose: &Pose, model: &Model, name: &str) -> bool {
    pose.transforms[node(model, name)].translation.y > 0.84
}

/// On every frame where an arc of a closed eye shows, the eye it replaces is hidden, gaze or
/// not: a dot of pupil or of reflection never stays beside the arc.
fn only_arcs_when_closed(mood: Mood, eyes: &[(&str, &str)], gaze: Option<glam::Vec3>) {
    let Some(mut animator) = animator() else {
        return;
    };
    let model = animator.model().clone();
    play(&mut animator, 0.5);
    animator.look_at(gaze);
    animator.set_mood(mood);
    let mut closed_frames = 0;
    for frame in 0..90 {
        let pose = animator.pose();
        for (lid, arc) in eyes {
            if arc_out(&pose, &model, arc) {
                closed_frames += 1;
                let scale = pose.transforms[node(&model, lid)].scale;
                assert_eq!(
                    scale,
                    glam::Vec3::ZERO,
                    "frame {frame}: {lid} shows beside its arc"
                );
            }
        }
        animator.advance(1.0 / 30.0);
    }
    assert!(
        closed_frames > 80 * eyes.len(),
        "the eyes close: {closed_frames}"
    );
}

#[test]
fn closed_eyes_show_only_their_arcs_on_every_frame() {
    let both = [("eyelid_L", "eye_happy_L"), ("eyelid_R", "eye_happy_R")];
    only_arcs_when_closed(Mood::EyesClosed, &both, None);
    only_arcs_when_closed(
        Mood::EyesClosed,
        &both,
        Some(glam::Vec3::new(-3.0, -4.0, 4.0)),
    );
    only_arcs_when_closed(
        Mood::Peek,
        &both[1..],
        Some(glam::Vec3::new(1.0, -4.0, 4.0)),
    );
}

#[test]
fn the_ajar_eye_shows_no_arc() {
    let Some(mut animator) = animator() else {
        return;
    };
    let model = animator.model().clone();
    animator.set_mood(Mood::Peek);
    for _ in 0..60 {
        animator.advance(1.0 / 30.0);
        assert!(!arc_out(&animator.pose(), &model, "eye_happy_L"));
    }
}

#[test]
fn from_closed_to_peek_the_left_eye_opens_half_way() {
    let Some(mut animator) = animator() else {
        return;
    };
    let model = animator.model().clone();
    animator.set_mood(Mood::EyesClosed);
    play(&mut animator, 1.0);
    animator.set_mood(Mood::Peek);
    play(&mut animator, 1.0);
    let pose = animator.pose();
    let left = pose.transforms[node(&model, "eyelid_L")].scale;
    assert!((left.z - PEEK_OPENING).abs() < 0.02, "{left}");
    assert_eq!(eyelid(&pose, &model, "eyelid_R"), 0.0);
}

#[test]
fn a_shut_eye_does_not_follow_the_gaze() {
    let Some(mut animator) = animator() else {
        return;
    };
    let model = animator.model().clone();
    let (right, left) = (node(&model, "eye_R"), node(&model, "eye_L"));
    animator.set_mood(Mood::Peek);
    play(&mut animator, 1.0);
    let ahead = animator.pose();
    // The password field, far below.
    animator.look_at(Some(glam::Vec3::new(0.0, -6.0, 4.0)));
    play(&mut animator, 1.0);
    let down = animator.pose();
    let turned = |eye: usize| {
        down.transforms[eye]
            .rotation
            .angle_between(ahead.transforms[eye].rotation)
    };
    assert!(
        turned(right) < 1e-3,
        "the closed eye stays: {}",
        turned(right)
    );
    assert!(
        turned(left) > GAZE_DOWN * 0.9,
        "the ajar eye looks down: {}",
        turned(left)
    );
}

/// How far the center of an eye may move from its rest place on the head, in the units of the
/// model: a third of the radius of the eye (about 0.21).
const ORBIT: f32 = 0.07;

/// The vertices that a bone named in `names` carries the most, by index.
fn carried(model: &Model, names: &[&str]) -> Vec<usize> {
    let joints: Vec<u16> = names
        .iter()
        .map(|name| {
            let bone = node(model, name);
            let Some(joint) = model.joints.iter().position(|&j| j == bone) else {
                panic!("{name} carries no vertex");
            };
            joint as u16
        })
        .collect();
    (0..model.vertices.len())
        .filter(|&i| {
            let v = &model.vertices[i];
            let main = (0..4)
                .max_by(|&a, &b| v.weights[a].total_cmp(&v.weights[b]))
                .unwrap_or(0);
            joints.contains(&v.joints[main])
        })
        .collect()
}

/// Where `vertex` lies in this frame, in the space of the model.
fn skinned(model: &Model, matrices: &[glam::Mat4], vertex: usize) -> glam::Vec3 {
    let v = &model.vertices[vertex];
    let p = glam::Vec3::from(v.position);
    (0..4)
        .map(|k| matrices[usize::from(v.joints[k])].transform_point3(p) * v.weights[k])
        .sum()
}

/// In every mood, wherever the eyes look and however the head turns or leans, each eye stays
/// in its socket on the head, no eye nor arc of a closed eye covers the mouth, the two open
/// eyes turn by the same rotation (no squint), and by a few degrees at most.
#[test]
fn the_eyes_stay_in_their_sockets_and_look_the_same_way() {
    let Some(probe) = animator() else { return };
    let model = probe.model().clone();
    let head = node(&model, "head");
    let lids = [node(&model, "eyelid_L"), node(&model, "eyelid_R")];
    let eyes = [node(&model, "eye_L"), node(&model, "eye_R")];
    let rest = Pose::rest(&model).globals(&model);
    let socket = lids.map(|lid| {
        rest[head]
            .inverse()
            .transform_point3(rest[lid].w_axis.truncate())
    });
    let faces = carried(
        &model,
        &["eyelid_L", "eyelid_R", "eye_happy_L", "eye_happy_R"],
    );
    let mouths = carried(&model, &["mouth"]);
    let field = Camera::UPPER_BODY.unproject(glam::Vec2::new(0.0, -2.6), 1.0, 4.0);
    let targets = [
        None,
        Some(field),
        Some(glam::Vec3::new(0.0, -6.0, 4.0)),
        Some(glam::Vec3::new(-8.0, -6.0, 4.0)),
        Some(glam::Vec3::new(8.0, 3.0, 4.0)),
        // Straight ahead, far.
        Some(glam::Vec3::new(0.0, 1.95, 100.0)),
        // Close in front of the nose.
        Some(glam::Vec3::new(0.0, 1.9, 1.3)),
    ];
    let most = GAZE_LIMIT.max(GAZE_UP).max(GAZE_DOWN);
    for mood in [
        Mood::Idle,
        Mood::Wave,
        Mood::EyesClosed,
        Mood::Peek,
        Mood::Sorry,
        Mood::Success,
        Mood::Sleep,
        Mood::Think,
    ] {
        for target in targets {
            let mut animator = Animator::new(model.clone());
            animator.look_at(target);
            animator.set_mood(mood);
            for frame in 0..90 {
                animator.advance(1.0 / 30.0);
                let what = format!("{mood:?}, looking at {target:?}, frame {frame}");
                let pose = animator.pose();
                let globals = pose.globals(&model);
                let into_head = globals[head].inverse();
                for (side, lid) in lids.into_iter().enumerate() {
                    let at = into_head.transform_point3(globals[lid].w_axis.truncate());
                    let off = at.distance(socket[side]);
                    assert!(off < ORBIT, "{what}: eye {side} {off} out of its socket");
                }
                // Seen from the front of the head, no eye covers the mouth.
                let matrices = pose.joint_matrices(&model);
                let in_head = |v| into_head.transform_point3(skinned(&model, &matrices, v));
                let (low, high) = mouths
                    .iter()
                    .map(|&v| in_head(v))
                    .fold((glam::Vec3::MAX, glam::Vec3::MIN), |(low, high), p| {
                        (low.min(p), high.max(p))
                    });
                for &v in &faces {
                    let p = in_head(v);
                    let inside = p.x > low.x && p.x < high.x && p.y > low.y && p.y < high.y;
                    assert!(!inside, "{what}: an eye covers the mouth at {p}");
                }
                // The gaze each eye adds to its rest, in the space of the head.
                let gaze = eyes.map(|eye| {
                    pose.transforms[eye].rotation * model.nodes[eye].rest.rotation.inverse()
                });
                for turn in gaze {
                    let angle = turn.angle_between(glam::Quat::IDENTITY);
                    assert!(angle <= most + 1e-3, "{what}: the eyes turn by {angle}");
                }
                let [left, right] = animator.openness();
                if left > 0.99 && right > 0.99 {
                    let apart = gaze[0].angle_between(gaze[1]);
                    assert!(apart < 1e-3, "{what}: the eyes squint by {apart}");
                }
                // At rest, the head only sways: a point far ahead leaves the eyes where they rest.
                if mood == Mood::Idle && target == targets[5] {
                    let ahead = gaze[0].angle_between(glam::Quat::IDENTITY);
                    assert!(
                        ahead < 0.04,
                        "{what}: looking ahead turns the eyes by {ahead}"
                    );
                }
            }
        }
    }
}
