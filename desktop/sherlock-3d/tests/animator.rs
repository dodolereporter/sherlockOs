//! The moods, played on the model. Skipped, with a message, without `sherlock.glb`.

mod support;

use std::sync::Arc;

use sherlock_3d::animator::{FADE, GAZE_LIMIT, PEEK_TILT};
use sherlock_3d::{Animator, Model, Mood, Pose};

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
    assert!(
        eyelid(&pose, &model, "eyelid_R") < 0.2,
        "the right eye is closed"
    );
    let left = eyelid(&pose, &model, "eyelid_L");
    assert!(left > 0.3 && left < 0.6, "the left eye is ajar: {left}");
    assert_eq!(animator.closure(), [0.6, 1.0]);
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
    assert_eq!(animator.closure(), [0.0, 0.0]);
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
    // Far down and to the side: the limit holds.
    animator.look_at(Some(glam::Vec3::new(-8.0, -6.0, 4.0)));
    play(&mut animator, 1.0);
    let turned = animator.pose().transforms[eye].rotation;
    let angle = turned.angle_between(ahead);
    assert!(
        angle > GAZE_LIMIT * 0.9 && angle < GAZE_LIMIT * 1.01,
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
