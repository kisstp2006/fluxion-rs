// ============================================================
// fluxion-core — AudioSource ECS component + AudioSystem
//
// Attach to any entity to give it a 3D sound emitter.
// AudioSystem::update() reads entity world positions and the
// active Camera's position as the listener, computing per-source
// gain (distance attenuation) and left/right panning.
//
// The actual audio playback handle is managed externally
// (fluxion-audio crate).  AudioSystem writes back the computed
// gain so the audio engine can apply it without knowing about ECS.
// ============================================================

use serde::{Deserialize, Serialize};
use fluxion_reflect_derive::Reflect;
use crate::ecs::component::Component;
use crate::ecs::world::ECSWorld;
use crate::components::camera::Camera;
use crate::transform::Transform;

/// Distance falloff model for 3D audio sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AudioRolloffMode {
    /// Gain = 1 / max(distance, 1).
    #[default]
    InverseDistance,
    /// Gain drops linearly from 1 at min_distance to 0 at max_distance.
    Linear,
    /// No distance attenuation — always at full volume.
    None,
}

/// 3D audio emitter component.  Place on any entity that should emit sound.
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "camelCase")]
pub struct AudioSource {
    /// Asset path of the audio clip to play (relative to project root).
    #[reflect(asset_type = "audio", header = "Audio Clip", tooltip = "The audio clip to play.")]
    pub clip_path: String,
    /// Volume multiplier before spatial attenuation. Range: [0, 1].
    #[reflect(range(min = 0.0, max = 1.0), slider, header = "Playback", tooltip = "Volume multiplier (0 = silent, 1 = full).")]
    pub volume: f32,
    /// Pitch multiplier. 1.0 = normal. Range: [0.1, 3.0].
    #[reflect(range(min = 0.1, max = 3.0), slider, tooltip = "Pitch shift (1.0 = normal speed).")]
    pub pitch: f32,
    /// Whether the clip loops.
    pub looping: bool,
    /// Whether the source starts playing on scene load.
    pub play_on_awake: bool,
    /// Distance falloff model.
    #[reflect(header = "3D Sound", variants("InverseDistance", "Linear", "None"))]
    pub rolloff_mode: AudioRolloffMode,
    /// Distance at which volume starts falling off.
    #[reflect(range(min = 0.0, max = 500.0), tooltip = "Distance at which attenuation begins.")]
    pub min_distance: f32,
    /// Distance at which the source becomes inaudible.
    #[reflect(range(min = 1.0, max = 2000.0), tooltip = "Distance at which the source is silent.")]
    pub max_distance: f32,
    /// Blend between fully 3D (1.0) and fully 2D (0.0).
    #[reflect(range(min = 0.0, max = 1.0), slider, tooltip = "0 = 2D (no spatialization), 1 = full 3D.")]
    pub spatial_blend: f32,
    /// Computed gain this frame (read by audio engine, not serialised).
    #[serde(skip)]
    #[reflect(skip)]
    pub computed_gain: f32,
    /// Runtime playback handle (not serialised).
    #[serde(skip)]
    #[reflect(skip)]
    pub play_handle: i64,
}

impl Default for AudioSource {
    fn default() -> Self {
        Self {
            clip_path:     String::new(),
            volume:        1.0,
            pitch:         1.0,
            looping:       false,
            play_on_awake: true,
            rolloff_mode:  AudioRolloffMode::InverseDistance,
            min_distance:  1.0,
            max_distance:  50.0,
            spatial_blend: 1.0,
            computed_gain: 1.0,
            play_handle:   0,
        }
    }
}

impl Component for AudioSource {}

// ── AudioSystem ───────────────────────────────────────────────────────────────

/// Per-frame 3D audio update.
/// Call from the editor host after `tick_editor_only` / `tick`.
pub struct AudioSystem;

impl AudioSystem {
    /// Update spatial audio: compute `computed_gain` for every `AudioSource`
    /// based on the active camera's world position as the listener.
    pub fn update(world: &ECSWorld) {
        // Locate listener position (first Camera entity).
        let mut listener_pos = glam::Vec3::ZERO;
        world.query_all::<(&Transform, &Camera), _>(|_eid, (t, _cam)| {
            listener_pos = t.world_position;
        });

        // Collect source positions then update gains.
        let mut updates: Vec<(crate::EntityId, f32)> = Vec::new();
        world.query_all::<(&Transform, &AudioSource), _>(|eid, (t, src)| {
            let dist = (t.world_position - listener_pos).length();
            let gain = compute_gain(src, dist);
            updates.push((eid, gain));
        });

        for (eid, gain) in updates {
            if let Some(mut src) = world.get_component_mut::<AudioSource>(eid) {
                src.computed_gain = gain;
            }
        }
    }
}

fn compute_gain(src: &AudioSource, distance: f32) -> f32 {
    let spatial = src.spatial_blend.clamp(0.0, 1.0);
    let spatial_gain = match src.rolloff_mode {
        AudioRolloffMode::None => 1.0,
        AudioRolloffMode::InverseDistance => {
            if distance >= src.max_distance { 0.0 }
            else {
                let d = distance.max(src.min_distance);
                src.min_distance / d
            }
        }
        AudioRolloffMode::Linear => {
            if distance >= src.max_distance { 0.0 }
            else if distance <= src.min_distance { 1.0 }
            else {
                1.0 - (distance - src.min_distance) / (src.max_distance - src.min_distance)
            }
        }
    };
    let final_spatial = spatial_gain.clamp(0.0, 1.0);
    // Blend between 2D (gain = 1) and 3D.
    let blended = 1.0 * (1.0 - spatial) + final_spatial * spatial;
    (blended * src.volume).clamp(0.0, 1.0)
}
