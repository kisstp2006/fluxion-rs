// ============================================================
// fluxion-core — Debug draw buffer (CPU-side MVP)
//
// Editor-style line/box drawing can consume this from a future
// wgpu debug pass; no renderer dependency here.
// ============================================================

use glam::{Vec3, Vec4};

/// Single line segment in world space (start → end), RGBA color per endpoint optional later.
#[derive(Debug, Clone, Copy)]
pub struct DebugLine {
    pub start: Vec3,
    pub end:   Vec3,
    pub color: Vec4,
}

/// Accumulated debug geometry for one frame (clear after upload to GPU).
#[derive(Debug, Clone, Default)]
pub struct DebugDraw {
    pub lines: Vec<DebugLine>,
}

impl DebugDraw {
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn line(&mut self, start: Vec3, end: Vec3, color: Vec4) {
        self.lines.push(DebugLine { start, end, color });
    }
}
