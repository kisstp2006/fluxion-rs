// ============================================================
// fluxion-core — Animator component + AnimationClip data types
//
// These are pure data structures; the actual sampling and bone-
// matrix computation lives in AnimationSystem (same crate).
//
// Compatible with glTF animation tracks (TRS per joint, linear
// or step interpolation).  The clip stores per-channel keyframes;
// the system samples them every frame and writes the resulting
// bone matrices into the Animator for the renderer to consume.
// ============================================================

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

// ── AnimationClip ─────────────────────────────────────────────────────────────

/// One keyframe value for a translation or scale channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyframeVec3 {
    pub time:  f32,
    pub value: [f32; 3],
}

/// One keyframe value for a rotation channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyframeQuat {
    pub time:  f32,
    pub value: [f32; 4],  // xyzw
}

/// Per-joint animation channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JointChannel {
    /// Index of the joint this channel drives.
    pub joint_index:   usize,
    pub translations:  Vec<KeyframeVec3>,
    pub rotations:     Vec<KeyframeQuat>,
    pub scales:        Vec<KeyframeVec3>,
}

/// One named animation clip (e.g. "Walk", "Run", "Idle").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationClip {
    pub name:     String,
    pub duration: f32,
    pub channels: Vec<JointChannel>,
}

impl AnimationClip {
    /// Sample a single joint's local TRS at time `t` (seconds, wraps by duration).
    pub fn sample_joint(&self, joint: usize, t: f32) -> (Vec3, Quat, Vec3) {
        let t = t % self.duration.max(1e-5);
        let ch = match self.channels.iter().find(|c| c.joint_index == joint) {
            Some(c) => c,
            None    => return (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE),
        };

        let translation = sample_vec3(&ch.translations, t);
        let rotation    = sample_quat(&ch.rotations,    t);
        let scale       = sample_vec3_with_default(&ch.scales, t, Vec3::ONE);

        (translation, rotation, scale)
    }
}

fn sample_vec3(keys: &[KeyframeVec3], t: f32) -> Vec3 {
    if keys.is_empty() { return Vec3::ZERO; }
    if keys.len() == 1 { return Vec3::from(keys[0].value); }
    let idx = keys.partition_point(|k| k.time <= t);
    if idx == 0 { return Vec3::from(keys[0].value); }
    if idx >= keys.len() { return Vec3::from(keys[keys.len() - 1].value); }
    let a = &keys[idx - 1];
    let b = &keys[idx];
    let span = (b.time - a.time).max(1e-6);
    let f = (t - a.time) / span;
    Vec3::from(a.value).lerp(Vec3::from(b.value), f)
}

fn sample_vec3_with_default(keys: &[KeyframeVec3], t: f32, default: Vec3) -> Vec3 {
    if keys.is_empty() { return default; }
    sample_vec3(keys, t)
}

fn sample_quat(keys: &[KeyframeQuat], t: f32) -> Quat {
    if keys.is_empty() { return Quat::IDENTITY; }
    if keys.len() == 1 {
        let v = keys[0].value;
        return Quat::from_xyzw(v[0], v[1], v[2], v[3]).normalize();
    }
    let idx = keys.partition_point(|k| k.time <= t);
    if idx == 0 {
        let v = keys[0].value;
        return Quat::from_xyzw(v[0], v[1], v[2], v[3]).normalize();
    }
    if idx >= keys.len() {
        let v = keys[keys.len() - 1].value;
        return Quat::from_xyzw(v[0], v[1], v[2], v[3]).normalize();
    }
    let a_v = keys[idx - 1].value;
    let b_v = keys[idx].value;
    let span = (keys[idx].time - keys[idx - 1].time).max(1e-6);
    let f = (t - keys[idx - 1].time) / span;
    let a = Quat::from_xyzw(a_v[0], a_v[1], a_v[2], a_v[3]).normalize();
    let b = Quat::from_xyzw(b_v[0], b_v[1], b_v[2], b_v[3]).normalize();
    a.slerp(b, f)
}

// ── Skeleton ──────────────────────────────────────────────────────────────────

/// Skeleton data: joint hierarchy + inverse bind-pose matrices.
/// Shared (Arc) across multiple Animator instances if the same skin is used.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skeleton {
    pub joints:          Vec<JointDef>,
    pub clips:           Vec<AnimationClip>,
}

/// One joint in the skeleton hierarchy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JointDef {
    pub name:              String,
    pub parent:            Option<usize>,
    /// Inverse bind-pose matrix (object space → joint space).
    pub inverse_bind_pose: [[f32; 4]; 4],
}

// ── Animator component ────────────────────────────────────────────────────────

pub const MAX_JOINTS: usize = 128;

/// Per-entity animator.  Attach to any entity that has a skinned mesh.
#[derive(Debug, Clone)]
pub struct Animator {
    /// The skeleton (joints + clips). `None` until the renderer loads the glTF skin.
    pub skeleton:     Option<std::sync::Arc<Skeleton>>,

    /// Index of the currently playing clip in `skeleton.clips`.
    pub clip_index:   usize,

    /// Playback time in seconds.
    pub time:         f32,

    /// Playback speed multiplier (1.0 = normal).
    pub speed:        f32,

    /// Loop the clip.
    pub looping:      bool,

    /// Whether the clip is currently playing.
    pub playing:      bool,

    /// Joint matrices (bone space → object space) computed each frame.
    /// Uploaded to the GPU as a uniform buffer by the skinned geometry pass.
    pub joint_matrices: Vec<Mat4>,
}

impl Animator {
    pub fn new() -> Self {
        Self {
            skeleton:      None,
            clip_index:    0,
            time:          0.0,
            speed:         1.0,
            looping:       true,
            playing:       true,
            joint_matrices: vec![Mat4::IDENTITY; MAX_JOINTS],
        }
    }

    pub fn play(&mut self, clip_index: usize) {
        self.clip_index = clip_index;
        self.time       = 0.0;
        self.playing    = true;
    }

    pub fn stop(&mut self) {
        self.playing = false;
    }
}

impl Default for Animator { fn default() -> Self { Self::new() } }

impl crate::ecs::component::Component for Animator {}
