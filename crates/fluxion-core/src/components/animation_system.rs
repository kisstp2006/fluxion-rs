// ============================================================
// fluxion-core — AnimationSystem
//
// Advances animator time and recomputes per-joint bone matrices
// every frame.  Called once per frame from the editor host and
// sandbox main loop before extract_frame_data.
//
// Output: Animator::joint_matrices (Vec<Mat4>, length MAX_JOINTS)
//   Each entry is the final bone matrix: inverse_bind_pose × global_joint.
//   The renderer uploads these as a storage/uniform buffer per entity.
// ============================================================

use glam::Mat4;

use crate::ecs::world::ECSWorld;
use crate::components::animator::{Animator, MAX_JOINTS};

pub struct AnimationSystem;

impl AnimationSystem {
    /// Advance all animators by `dt` seconds and recompute joint matrices.
    pub fn update(world: &ECSWorld, dt: f32) {
        let mut animators: Vec<crate::EntityId> = Vec::new();
        world.query_all::<&Animator, _>(|id, _| animators.push(id));

        for entity in animators {
            let Some(mut anim) = world.get_component_mut::<Animator>(entity) else { continue };

            // Advance time.
            if anim.playing {
                anim.time += dt * anim.speed;
            }

            let skeleton = match anim.skeleton.clone() {
                Some(s) => s,
                None => continue,
            };

            let clips = &skeleton.clips;
            if clips.is_empty() { continue; }
            let clip_idx = anim.clip_index.min(clips.len().saturating_sub(1));
            let clip = &clips[clip_idx];

            // Wrap or clamp time.
            if anim.looping {
                if clip.duration > 1e-5 {
                    anim.time = anim.time % clip.duration;
                }
            } else {
                anim.time = anim.time.min(clip.duration);
            }

            let t = anim.time;
            let joints = &skeleton.joints;
            let n = joints.len().min(MAX_JOINTS);

            // Step 1: compute each joint's local transform.
            let mut global: Vec<Mat4> = vec![Mat4::IDENTITY; n];
            for ji in 0..n {
                let (trans, rot, scale) = clip.sample_joint(ji, t);
                let local = Mat4::from_scale_rotation_translation(scale, rot, trans);
                let parent_global = match joints[ji].parent {
                    Some(pi) if pi < ji => global[pi],
                    _                   => Mat4::IDENTITY,
                };
                global[ji] = parent_global * local;
            }

            // Step 2: multiply by inverse bind-pose.
            anim.joint_matrices.resize(MAX_JOINTS, Mat4::IDENTITY);
            for ji in 0..n {
                let ibp = Mat4::from_cols_array_2d(&joints[ji].inverse_bind_pose);
                anim.joint_matrices[ji] = global[ji] * ibp;
            }
            // Identity for unused slots.
            for ji in n..MAX_JOINTS {
                anim.joint_matrices[ji] = Mat4::IDENTITY;
            }
        }
    }
}
