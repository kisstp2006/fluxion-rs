// ============================================================
// viewport_gizmo.rs — Rust-side 3D transform gizmo
//
// Replaces the old Rune-scripted gizmo_module. Draws colored
// axis arrows directly with egui Painter using the actual
// camera view/projection matrices from the renderer.
//
// Design
// ──────
// - Called once per frame after the viewport image is shown
// - Projects entity world position into screen space
// - Draws X/Y/Z axis handles as arrows
// - Hit-tests mouse against arrows, accumulates drag delta
// - Returns (axis, delta_world_units) to the caller
//
// Axis convention: X=red, Y=green, Z=blue (right-hand, Y-up)
// ============================================================

use egui::{Color32, Painter, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2};
use glam::{Mat4, Vec3, Vec4};

// ── Gizmo modes ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

// ── Per-axis state ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Axis { X, Y, Z }

impl Axis {
    fn color(self) -> Color32 {
        match self {
            Axis::X => Color32::from_rgb(220, 60,  60),
            Axis::Y => Color32::from_rgb(60,  200, 60),
            Axis::Z => Color32::from_rgb(60,  100, 220),
        }
    }
    fn world_dir(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }
}

const AXES: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

// ── Drag state (frame-local, passed in/out) ───────────────────────────────────

pub struct GizmoDragState {
    pub active_axis: Option<Axis>,
    /// Delta produced this frame — consumed by caller after egui frame.
    pub pending_delta: Option<(usize, f32, GizmoMode)>,
}

impl Default for GizmoDragState {
    fn default() -> Self { Self { active_axis: None, pending_delta: None } }
}

// ── Main entry ────────────────────────────────────────────────────────────────

/// Draw the gizmo and process interaction for one entity.
///
/// - `viewport_rect`: the egui Rect of the scene image in screen space
/// - `entity_world_pos`: world-space position of the selected entity
/// - `view`, `proj`: camera matrices from renderer
/// - `mode`: translate / rotate / scale
/// - `drag`: mutable drag state (persisted across frames)
///
/// Returns `Some((axis_index, delta_world))` when a drag is active,
/// where `axis_index` is 0=X, 1=Y, 2=Z and `delta_world` is the
/// movement amount in world units this frame.
pub fn draw_and_interact(
    ui: &mut Ui,
    viewport_rect: Rect,
    entity_world_pos: Vec3,
    view: Mat4,
    proj: Mat4,
    _mode: GizmoMode,
    drag: &mut GizmoDragState,
) -> Option<(usize, f32)> {
    let vp_w = viewport_rect.width();
    let vp_h = viewport_rect.height();
    if vp_w < 1.0 || vp_h < 1.0 { return None; }

    let view_proj = proj * view;

    // Project entity center to screen space (NDC → pixels)
    let center_screen = world_to_screen(entity_world_pos, view_proj, viewport_rect)?;

    // Gizmo arrow length in pixels (scale with viewport)
    let arrow_len = (vp_w.min(vp_h) * 0.10).clamp(40.0, 90.0);

    // Project axis tip points
    let axis_tips: Vec<(Axis, Pos2)> = AXES.iter().filter_map(|&ax| {
        // Move 1 unit in axis direction to find screen delta
        let tip_world = entity_world_pos + ax.world_dir();
        let tip_screen = world_to_screen(tip_world, view_proj, viewport_rect)?;
        // Scale to arrow_len pixels
        let dir = (tip_screen - center_screen).normalized();
        if dir.length() < 0.001 { return None; }
        Some((ax, center_screen + dir * arrow_len))
    }).collect();

    // ── Mouse interaction ──────────────────────────────────────────────────────

    let cursor = ui.input(|i| i.pointer.hover_pos());
    let dragging = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let just_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

    if just_released {
        drag.active_axis = None;
    }

    // Hit-test: if no active axis, pick closest axis within threshold
    if dragging && drag.active_axis.is_none() {
        if let Some(cursor_pos) = cursor {
            if viewport_rect.contains(cursor_pos) {
                for &(ax, tip) in &axis_tips {
                    if dist_point_to_segment(cursor_pos, center_screen, tip) < 10.0 {
                        drag.active_axis = Some(ax);
                        break;
                    }
                }
            }
        }
    }

    // Reset pending delta each frame
    drag.pending_delta = None;

    // Compute delta for active axis
    let mut result: Option<(usize, f32)> = None;
    if dragging {
        if let Some(active) = drag.active_axis {
            let drag_delta = ui.input(|i| i.pointer.delta());
            if let Some(&(_, tip)) = axis_tips.iter().find(|(ax, _)| *ax == active) {
                let screen_dir = (tip - center_screen).normalized();
                let screen_delta = drag_delta.dot(screen_dir);
                let world_delta = screen_delta / arrow_len;
                let axis_idx = AXES.iter().position(|&a| a == active).unwrap();
                result = Some((axis_idx, world_delta));
                drag.pending_delta = Some((axis_idx, world_delta, _mode));
            }
        }
    }

    // ── Drawing ────────────────────────────────────────────────────────────────

    let painter = ui.painter_at(viewport_rect);

    for &(ax, tip) in &axis_tips {
        let is_active   = drag.active_axis == Some(ax);
        let is_hovered  = cursor.map(|c|
            viewport_rect.contains(c) &&
            dist_point_to_segment(c, center_screen, tip) < 10.0
        ).unwrap_or(false);

        let base_color  = ax.color();
        let color = if is_active {
            brighten(base_color, 1.4)
        } else if is_hovered {
            brighten(base_color, 1.2)
        } else {
            base_color
        };

        let thickness = if is_active { 3.0 } else { 2.0 };
        draw_arrow(&painter, center_screen, tip, color, thickness);
    }

    // Draw center dot
    painter.circle_filled(center_screen, 4.0, Color32::WHITE);

    result
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn world_to_screen(pos: Vec3, view_proj: Mat4, rect: Rect) -> Option<Pos2> {
    let clip = view_proj * Vec4::new(pos.x, pos.y, pos.z, 1.0);
    if clip.w.abs() < 1e-6 { return None; }
    let ndc_x = clip.x / clip.w;
    let ndc_y = clip.y / clip.w;
    let ndc_z = clip.z / clip.w;
    // Behind camera
    if ndc_z < -1.0 || ndc_z > 1.0 { return None; }
    let sx = rect.min.x + (ndc_x  * 0.5 + 0.5) * rect.width();
    let sy = rect.min.y + (-ndc_y * 0.5 + 0.5) * rect.height(); // Y-flip
    Some(pos2(sx, sy))
}

fn dist_point_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let len2 = ab.length_sq();
    if len2 < 1e-6 { return (p - a).length(); }
    let t = ap.dot(ab) / len2;
    let t = t.clamp(0.0, 1.0);
    let closest = a + ab * t;
    (p - closest).length()
}

fn draw_arrow(painter: &Painter, from: Pos2, to: Pos2, color: Color32, thickness: f32) {
    painter.line_segment([from, to], Stroke::new(thickness, color));
    let dir = (to - from).normalized();
    let perp = Vec2::new(-dir.y, dir.x);
    let head = 10.0;
    let a1 = to - dir * head + perp * (head * 0.4);
    let a2 = to - dir * head - perp * (head * 0.4);
    painter.line_segment([to, a1], Stroke::new(thickness, color));
    painter.line_segment([to, a2], Stroke::new(thickness, color));
}

fn brighten(c: Color32, factor: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * factor).min(255.0) as u8,
        (c.g() as f32 * factor).min(255.0) as u8,
        (c.b() as f32 * factor).min(255.0) as u8,
    )
}
