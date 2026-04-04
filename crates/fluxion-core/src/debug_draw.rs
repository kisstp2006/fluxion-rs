// ============================================================
// fluxion-core — Debug draw buffer + global singleton
//
// Provides a static, thread-safe queue of line segments that any
// part of the engine (scripts, physics, renderer gizmos) can push
// to.  The renderer drains this queue once per frame and renders
// the lines as an overlay (no depth test, Unity Debug.DrawLine
// style).
//
// Public surface:
//   fluxion_core::draw_line(start, end, color)
//   fluxion_core::draw_sphere(center, radius, color)
//   … etc.
//   fluxion_core::drain_debug_lines() -> Vec<DebugLine>   ← called by renderer
// ============================================================

use std::sync::Mutex;
use glam::{Quat, Vec3, Vec4};
use lazy_static::lazy_static;

use crate::Color;

// ── Types ──────────────────────────────────────────────────────────────────

/// A single world-space line segment with a per-segment RGBA color.
#[derive(Debug, Clone, Copy)]
pub struct DebugLine {
    pub start: Vec3,
    pub end:   Vec3,
    pub color: Vec4,
}

/// Accumulated debug geometry for one frame.
/// Prefer the free functions (`draw_line`, `draw_sphere`, …) which push into
/// the global singleton instead of constructing this directly.
#[derive(Debug, Clone, Default)]
pub struct DebugDraw {
    pub lines: Vec<DebugLine>,
}

impl DebugDraw {
    pub fn clear(&mut self) { self.lines.clear(); }

    pub fn line(&mut self, start: Vec3, end: Vec3, color: Vec4) {
        self.lines.push(DebugLine { start, end, color });
    }
}

// ── Global singleton ───────────────────────────────────────────────────────

lazy_static! {
    static ref GLOBAL: Mutex<Vec<DebugLine>> = Mutex::new(Vec::new());
}

#[inline]
fn push(start: Vec3, end: Vec3, color: Vec4) {
    if let Ok(mut g) = GLOBAL.lock() {
        g.push(DebugLine { start, end, color });
    }
}

/// Drain all pending debug lines (called once per frame by the renderer).
pub fn drain_debug_lines() -> Vec<DebugLine> {
    GLOBAL.lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default()
}

// ── Free-function API ──────────────────────────────────────────────────────

/// Draw a single world-space line.
pub fn draw_line(start: Vec3, end: Vec3, color: Color) {
    push(start, end, color.vec4());
}

/// Draw a ray from `origin` in `dir` with the given length.
pub fn draw_ray(origin: Vec3, dir: Vec3, color: Color) {
    push(origin, origin + dir, color.vec4());
}

/// Draw a wireframe sphere approximation (3 great circles, XY / XZ / YZ).
pub fn draw_sphere(center: Vec3, radius: f32, color: Color) {
    let c = color.vec4();
    let segs = 32usize;
    let step = std::f32::consts::TAU / segs as f32;
    for i in 0..segs {
        let a0 = i as f32 * step;
        let a1 = (i + 1) as f32 * step;
        let (s0, c0) = a0.sin_cos();
        let (s1, c1) = a1.sin_cos();
        // XY
        push(
            center + Vec3::new(c0 * radius, s0 * radius, 0.0),
            center + Vec3::new(c1 * radius, s1 * radius, 0.0),
            c,
        );
        // XZ
        push(
            center + Vec3::new(c0 * radius, 0.0, s0 * radius),
            center + Vec3::new(c1 * radius, 0.0, s1 * radius),
            c,
        );
        // YZ
        push(
            center + Vec3::new(0.0, c0 * radius, s0 * radius),
            center + Vec3::new(0.0, c1 * radius, s1 * radius),
            c,
        );
    }
}

/// Draw a wireframe axis-aligned bounding box (12 edges).
pub fn draw_aabb(min: Vec3, max: Vec3, color: Color) {
    let c = color.vec4();
    let corners = [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(max.x, max.y, max.z),
        Vec3::new(min.x, max.y, max.z),
    ];
    // bottom face
    push(corners[0], corners[1], c); push(corners[1], corners[2], c);
    push(corners[2], corners[3], c); push(corners[3], corners[0], c);
    // top face
    push(corners[4], corners[5], c); push(corners[5], corners[6], c);
    push(corners[6], corners[7], c); push(corners[7], corners[4], c);
    // verticals
    push(corners[0], corners[4], c); push(corners[1], corners[5], c);
    push(corners[2], corners[6], c); push(corners[3], corners[7], c);
}

/// Draw a wireframe oriented box (centre + half-extents + rotation).
pub fn draw_box_rotated(center: Vec3, half: Vec3, rot: Quat, color: Color) {
    let c = color.vec4();
    let signs: [[f32; 3]; 8] = [
        [-1.,-1.,-1.],[1.,-1.,-1.],[1.,1.,-1.],[-1.,1.,-1.],
        [-1.,-1., 1.],[1.,-1., 1.],[1.,1., 1.],[-1.,1., 1.],
    ];
    let corners: Vec<Vec3> = signs.iter().map(|s| {
        center + rot * Vec3::new(s[0]*half.x, s[1]*half.y, s[2]*half.z)
    }).collect();
    push(corners[0], corners[1], c); push(corners[1], corners[2], c);
    push(corners[2], corners[3], c); push(corners[3], corners[0], c);
    push(corners[4], corners[5], c); push(corners[5], corners[6], c);
    push(corners[6], corners[7], c); push(corners[7], corners[4], c);
    push(corners[0], corners[4], c); push(corners[1], corners[5], c);
    push(corners[2], corners[6], c); push(corners[3], corners[7], c);
}

/// Draw a capsule (two sphere approximations + 4 connecting lines along local Y).
pub fn draw_capsule(center: Vec3, half_height: f32, radius: f32, rot: Quat, color: Color) {
    let up      = rot * Vec3::Y;
    let right   = rot * Vec3::X;
    let forward = rot * Vec3::Z;
    let top = center + up * half_height;
    let bot = center - up * half_height;
    draw_sphere(top, radius, color);
    draw_sphere(bot, radius, color);
    let c = color.vec4();
    for dir in [forward, right, -forward, -right] {
        push(top + dir * radius, bot + dir * radius, c);
    }
}

/// Draw a cross (3 short axis-aligned lines) at `pos`.
pub fn draw_cross(pos: Vec3, size: f32, color: Color) {
    let c = color.vec4();
    let h = size * 0.5;
    push(pos - Vec3::X * h, pos + Vec3::X * h, c);
    push(pos - Vec3::Y * h, pos + Vec3::Y * h, c);
    push(pos - Vec3::Z * h, pos + Vec3::Z * h, c);
}

/// Draw an XZ grid centred at the origin with axis-coloured centre lines.
pub fn draw_grid(size: f32, divisions: u32, color: Color) {
    let c      = color.vec4();
    let ax_red  = Color::Red.vec4();
    let ax_grn  = Color::Green.vec4();
    let ax_blu  = Color::Blue.vec4();
    let half    = size * 0.5;
    let step    = size / divisions as f32;

    for i in 0..=divisions {
        let pos = -half + i as f32 * step;
        let on_centre = pos.abs() < step * 0.01;
        if on_centre {
            // X axis (positive = red, negative = dim)
            push(Vec3::new(-half, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0), c);
            push(Vec3::new(0.0, 0.0, 0.0), Vec3::new(half, 0.0, 0.0), ax_red);
            // Z axis (positive = blue, negative = dim)
            push(Vec3::new(0.0, 0.0, -half), Vec3::new(0.0, 0.0, 0.0), c);
            push(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, half), ax_blu);
        } else {
            push(Vec3::new(-half, 0.0, pos), Vec3::new(half, 0.0, pos), c);
            push(Vec3::new(pos, 0.0, -half), Vec3::new(pos, 0.0, half), c);
        }
    }
    // Y axis stub (green)
    push(Vec3::ZERO, Vec3::new(0.0, 3.0, 0.0), ax_grn);
}

/// Draw a cone (for lights, particles).  `apex` is the tip, `dir` is the
/// direction from apex toward the base, `half_angle` is in radians.
pub fn draw_cone(apex: Vec3, dir: Vec3, half_angle: f32, length: f32, segs: u32, color: Color) {
    let c        = color.vec4();
    let radius   = length * half_angle.tan();
    let base_ctr = apex + dir * length;

    // Compute two perpendicular vectors to `dir`
    let (right, up) = perp_basis(dir);

    let step = std::f32::consts::TAU / segs as f32;
    let mut prev = base_ctr + right * radius;
    for i in 1..=segs {
        let a    = i as f32 * step;
        let next = base_ctr + (right * a.cos() + up * a.sin()) * radius;
        push(prev, next, c);       // rim segment
        if i % 2 == 0 {
            push(apex, next, c);   // line from tip to rim
        }
        prev = next;
    }
}

/// Draw a perspective frustum from camera parameters.
pub fn draw_frustum(
    pos:    Vec3,
    fwd:    Vec3,
    up:     Vec3,
    right:  Vec3,
    fov_y:  f32,  // degrees
    aspect: f32,
    near:   f32,
    far:    f32,
    color:  Color,
) {
    let c      = color.vec4();
    let tan_h  = (fov_y.to_radians() * 0.5).tan();
    let near_h = tan_h * near;
    let near_w = near_h * aspect;
    let far_h  = tan_h * far;
    let far_w  = far_h * aspect;

    let nc = pos + fwd * near;
    let fc = pos + fwd * far;

    let corners: [Vec3; 8] = [
        nc - right*near_w + up*near_h,  // near top-left
        nc + right*near_w + up*near_h,  // near top-right
        nc + right*near_w - up*near_h,  // near bot-right
        nc - right*near_w - up*near_h,  // near bot-left
        fc - right*far_w  + up*far_h,
        fc + right*far_w  + up*far_h,
        fc + right*far_w  - up*far_h,
        fc - right*far_w  - up*far_h,
    ];

    // near plane
    push(corners[0], corners[1], c); push(corners[1], corners[2], c);
    push(corners[2], corners[3], c); push(corners[3], corners[0], c);
    // far plane
    push(corners[4], corners[5], c); push(corners[5], corners[6], c);
    push(corners[6], corners[7], c); push(corners[7], corners[4], c);
    // connecting edges
    for i in 0..4 { push(corners[i], corners[i + 4], c); }
}

// ── Internal helpers ───────────────────────────────────────────────────────

fn perp_basis(d: Vec3) -> (Vec3, Vec3) {
    let d = d.normalize_or_zero();
    let tmp = if d.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let right = d.cross(tmp).normalize_or_zero();
    let up    = right.cross(d).normalize_or_zero();
    (right, up)
}
