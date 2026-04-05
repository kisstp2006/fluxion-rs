// ============================================================
// viewport_gizmo.rs — Rust-side 3D transform gizmo
//
// Supports five modes:
//   Translate      — colored axis arrows (cone tips)
//   Rotate         — colored axis arcs (circles in screen space)
//   Scale          — colored axis arrows with box caps
//   BoxFaceHandles — 6 colored squares, one per CSG box face
//   BoxAxisArrows  — 3 symmetric arrows from face edges
//
// Axis convention: X=red, Y=green, Z=blue (right-hand, Y-up)
// ============================================================

use egui::{Color32, Painter, Pos2, Rect, Stroke, Ui, Vec2, pos2};
use glam::{Mat4, Vec3, Vec4};

// ── Gizmo modes ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
    /// CSG box: drag individual faces (asymmetric, adjusts size + position).
    BoxFaceHandles,
    /// CSG box: 3 symmetric arrows extending from face edges outward.
    BoxAxisArrows,
}

// ── Per-axis state ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(usize)]
pub enum Axis { X = 0, Y = 1, Z = 2 }

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
    /// The two tangent directions in the plane perpendicular to this axis.
    fn tangents(self) -> (Vec3, Vec3) {
        match self {
            Axis::X => (Vec3::Y, Vec3::Z),
            Axis::Y => (Vec3::X, Vec3::Z),
            Axis::Z => (Vec3::X, Vec3::Y),
        }
    }
}

const AXES: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

// ── Drag state ────────────────────────────────────────────────────────────────

pub struct GizmoDragState {
    pub active_axis: Option<Axis>,
    /// Delta produced this frame — consumed by caller after egui frame.
    pub pending_delta: Option<(usize, f32, GizmoMode)>,
    /// Active face index for BoxFaceHandles (0=+X, 1=-X, 2=+Y, 3=-Y, 4=+Z, 5=-Z).
    pub box_drag_face: Option<u8>,
}

impl Default for GizmoDragState {
    fn default() -> Self { Self { active_axis: None, pending_delta: None, box_drag_face: None } }
}

// ── Main entry ────────────────────────────────────────────────────────────────

/// Draw the gizmo and process interaction for one entity.
pub fn draw_and_interact(
    ui:               &mut Ui,
    viewport_rect:    Rect,
    entity_world_pos: Vec3,
    view:             Mat4,
    proj:             Mat4,
    mode:             GizmoMode,
    drag:             &mut GizmoDragState,
) -> Option<(usize, f32)> {
    let vp_w = viewport_rect.width();
    let vp_h = viewport_rect.height();
    if vp_w < 1.0 || vp_h < 1.0 { return None; }

    let view_proj = proj * view;
    let center_screen = world_to_screen(entity_world_pos, view_proj, viewport_rect)?;

    let handle_len = (vp_w.min(vp_h) * 0.10).clamp(40.0, 90.0);

    // Build screen-space axis tip positions (used for translate & scale).
    let axis_tips: Vec<(Axis, Pos2)> = AXES.iter().filter_map(|&ax| {
        let tip_world  = entity_world_pos + ax.world_dir();
        let tip_screen = world_to_screen(tip_world, view_proj, viewport_rect)?;
        let dir = (tip_screen - center_screen).normalized();
        if dir.length() < 0.001 { return None; }
        Some((ax, center_screen + dir * handle_len))
    }).collect();

    // ── Mouse interaction ──────────────────────────────────────────────────────

    let cursor       = ui.input(|i| i.pointer.hover_pos());
    let dragging     = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let just_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

    if just_released {
        drag.active_axis = None;
    }

    // Hit detection radius varies by mode.
    let hit_radius = match mode {
        GizmoMode::Rotate => 8.0,
        _                 => 10.0,
    };

    // Pick active axis on drag start.
    if dragging && drag.active_axis.is_none() {
        if let Some(cp) = cursor {
            if viewport_rect.contains(cp) {
                match mode {
                    GizmoMode::Translate | GizmoMode::Scale => {
                        for &(ax, tip) in &axis_tips {
                            if dist_point_to_segment(cp, center_screen, tip) < hit_radius {
                                drag.active_axis = Some(ax);
                                break;
                            }
                        }
                    }
                    GizmoMode::Rotate => {
                        // Hit-test against each arc (ring at handle_len radius).
                        for &ax in &AXES {
                            let arc_pts = arc_points(ax, entity_world_pos, handle_len,
                                                     view_proj, viewport_rect, center_screen);
                            if hit_arc(&arc_pts, cp, hit_radius) {
                                drag.active_axis = Some(ax);
                                break;
                            }
                        }
                    }
                    // Box modes handled by draw_box_and_interact.
                    GizmoMode::BoxFaceHandles | GizmoMode::BoxAxisArrows => {}
                }
            }
        }
    }

    drag.pending_delta = None;
    let mut result: Option<(usize, f32)> = None;

    if dragging {
        if let Some(active) = drag.active_axis {
            let drag_delta = ui.input(|i| i.pointer.delta());
            let axis_idx   = AXES.iter().position(|&a| a == active).unwrap();

            let delta = match mode {
                GizmoMode::Translate | GizmoMode::Scale => {
                    // Project drag onto the screen-space axis direction.
                    if let Some(&(_, tip)) = axis_tips.iter().find(|(ax, _)| *ax == active) {
                        let screen_dir = (tip - center_screen).normalized();
                        let sd = drag_delta.dot(screen_dir);
                        sd / handle_len
                    } else { 0.0 }
                }
                GizmoMode::Rotate => {
                    // For rotation, interpret horizontal mouse drag as angle.
                    // Scale so 1 full viewport width ≈ 2π radians.
                    drag_delta.x / vp_w * std::f32::consts::TAU
                }
                // Box modes handled by draw_box_and_interact — no delta here.
                GizmoMode::BoxFaceHandles | GizmoMode::BoxAxisArrows => 0.0,
            };

            if delta.abs() > 1e-7 {
                result = Some((axis_idx, delta));
                drag.pending_delta = Some((axis_idx, delta, mode));
            }
        }
    }

    // ── Drawing ────────────────────────────────────────────────────────────────

    let painter = ui.painter_at(viewport_rect);

    match mode {
        GizmoMode::Translate => {
            for &(ax, tip) in &axis_tips {
                let color = handle_color(ax, drag.active_axis, cursor, center_screen, tip,
                                         viewport_rect, GizmoMode::Translate);
                let thickness = if drag.active_axis == Some(ax) { 3.0 } else { 2.0 };
                draw_translate_handle(&painter, center_screen, tip, color, thickness);
            }
        }
        GizmoMode::Scale => {
            for &(ax, tip) in &axis_tips {
                let color = handle_color(ax, drag.active_axis, cursor, center_screen, tip,
                                         viewport_rect, GizmoMode::Scale);
                let thickness = if drag.active_axis == Some(ax) { 3.0 } else { 2.0 };
                draw_scale_handle(&painter, center_screen, tip, color, thickness);
            }
        }
        GizmoMode::Rotate => {
            for &ax in &AXES {
                let arc_pts = arc_points(ax, entity_world_pos, handle_len,
                                         view_proj, viewport_rect, center_screen);
                let is_active  = drag.active_axis == Some(ax);
                let is_hovered = cursor.map(|c|
                    viewport_rect.contains(c) && hit_arc(&arc_pts, c, 8.0)
                ).unwrap_or(false);
                let base  = ax.color();
                let color = if is_active { brighten(base, 1.4) }
                            else if is_hovered { brighten(base, 1.2) }
                            else { base };
                let thickness = if is_active { 3.0 } else { 2.0 };
                draw_arc(&painter, &arc_pts, color, thickness);
            }
        }
        // Box modes are handled by draw_box_and_interact — nothing to draw here.
        GizmoMode::BoxFaceHandles | GizmoMode::BoxAxisArrows => {}
    }

    // Center dot
    painter.circle_filled(center_screen, 4.0, Color32::WHITE);

    result
}

// ── Handle visuals ────────────────────────────────────────────────────────────

/// Translate: line + arrowhead cone.
fn draw_translate_handle(painter: &Painter, from: Pos2, to: Pos2, color: Color32, thickness: f32) {
    painter.line_segment([from, to], Stroke::new(thickness, color));
    let dir  = (to - from).normalized();
    let perp = Vec2::new(-dir.y, dir.x);
    let head = 10.0;
    let a1   = to - dir * head + perp * (head * 0.4);
    let a2   = to - dir * head - perp * (head * 0.4);
    painter.line_segment([to, a1], Stroke::new(thickness, color));
    painter.line_segment([to, a2], Stroke::new(thickness, color));
}

/// Scale: line + small solid square cap.
fn draw_scale_handle(painter: &Painter, from: Pos2, to: Pos2, color: Color32, thickness: f32) {
    painter.line_segment([from, to], Stroke::new(thickness, color));
    let half = 5.0;
    let rect = egui::Rect::from_center_size(to, egui::vec2(half * 2.0, half * 2.0));
    painter.rect_filled(rect, 1.0, color);
}

/// Compute hit-test color for translate/scale axis handles.
fn handle_color(
    ax:           Axis,
    active:       Option<Axis>,
    cursor:       Option<Pos2>,
    center:       Pos2,
    tip:          Pos2,
    vp_rect:      Rect,
    _mode:        GizmoMode,
) -> Color32 {
    let base = ax.color();
    if active == Some(ax) { return brighten(base, 1.4); }
    let hovered = cursor.map(|c|
        vp_rect.contains(c) && dist_point_to_segment(c, center, tip) < 10.0
    ).unwrap_or(false);
    if hovered { brighten(base, 1.2) } else { base }
}

// ── Rotate arc helpers ────────────────────────────────────────────────────────

/// Generate screen-space points for a rotation ring around `ax`.
///
/// Projects the two tangent unit vectors into screen space to get the
/// ellipse semi-axes `u` and `v`, then sweeps `center + u*cos(a) + v*sin(a)`.
/// This draws all three axes correctly at any viewing angle, including
/// edge-on cases where individual circle-point projection collapses.
fn arc_points(
    ax:            Axis,
    world_pos:     Vec3,
    radius_px:     f32,
    view_proj:     Mat4,
    vp_rect:       Rect,
    center_screen: Pos2,
) -> Vec<Pos2> {
    let (t1, t2) = ax.tangents();

    // Project center + tangent unit vectors to get screen-space directions.
    // We scale by `radius_px` after: the ellipse semi-axes in pixels.
    let c_proj = world_to_screen(world_pos,          view_proj, vp_rect);
    let t1_proj = world_to_screen(world_pos + t1,    view_proj, vp_rect);
    let t2_proj = world_to_screen(world_pos + t2,    view_proj, vp_rect);

    let (c, p1, p2) = match (c_proj, t1_proj, t2_proj) {
        (Some(c), Some(p1), Some(p2)) => (c, p1, p2),
        _ => return Vec::new(),
    };

    // Screen-space delta per world unit along each tangent.
    let u: Vec2 = p1 - c;
    let v: Vec2 = p2 - c;

    // Normalize to radius_px so the ring has a consistent screen size.
    let u_len = u.length();
    let v_len = v.length();
    if u_len < 0.5 && v_len < 0.5 { return Vec::new(); } // fully edge-on, skip

    // Use the larger magnitude to set radius; the other axis scales proportionally.
    let scale = radius_px / u_len.max(v_len).max(0.01);
    let u = u * scale;
    let v = v * scale;

    const STEPS: usize = 64;
    (0..=STEPS).map(|i| {
        let angle = i as f32 / STEPS as f32 * std::f32::consts::TAU;
        center_screen + u * angle.cos() + v * angle.sin()
    }).collect()
}

/// Draw the arc as a polyline.
fn draw_arc(painter: &Painter, pts: &[Pos2], color: Color32, thickness: f32) {
    for w in pts.windows(2) {
        painter.line_segment([w[0], w[1]], Stroke::new(thickness, color));
    }
}

/// Returns true if `cursor` is within `threshold` pixels of any arc segment.
fn hit_arc(pts: &[Pos2], cursor: Pos2, threshold: f32) -> bool {
    pts.windows(2).any(|w| dist_point_to_segment(cursor, w[0], w[1]) < threshold)
}

// ── Math helpers ──────────────────────────────────────────────────────────────

fn world_to_screen(pos: Vec3, view_proj: Mat4, rect: Rect) -> Option<Pos2> {
    let clip = view_proj * Vec4::new(pos.x, pos.y, pos.z, 1.0);
    if clip.w.abs() < 1e-6 { return None; }
    let ndc_x = clip.x / clip.w;
    let ndc_y = clip.y / clip.w;
    let ndc_z = clip.z / clip.w;
    if ndc_z < -1.0 || ndc_z > 1.0 { return None; }
    let sx = rect.min.x + (ndc_x  * 0.5 + 0.5) * rect.width();
    let sy = rect.min.y + (-ndc_y * 0.5 + 0.5) * rect.height();
    Some(pos2(sx, sy))
}

fn dist_point_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab   = b - a;
    let ap   = p - a;
    let len2 = ab.length_sq();
    if len2 < 1e-6 { return (p - a).length(); }
    let t       = (ap.dot(ab) / len2).clamp(0.0, 1.0);
    let closest = a + ab * t;
    (p - closest).length()
}

fn brighten(c: Color32, factor: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * factor).min(255.0) as u8,
        (c.g() as f32 * factor).min(255.0) as u8,
        (c.b() as f32 * factor).min(255.0) as u8,
    )
}

// ── Box resize gizmo ──────────────────────────────────────────────────────────

/// Face descriptors: (world_offset_sign, axis, face_idx)
/// Faces: 0=+X, 1=-X, 2=+Y, 3=-Y, 4=+Z, 5=-Z
const FACES: [(f32, Axis, usize); 6] = [
    ( 1.0, Axis::X, 0),
    (-1.0, Axis::X, 1),
    ( 1.0, Axis::Y, 2),
    (-1.0, Axis::Y, 3),
    ( 1.0, Axis::Z, 4),
    (-1.0, Axis::Z, 5),
];

/// Draw the box resize gizmo and handle interaction for a `CsgShape` entity.
///
/// `csg_size` = `[width, height, depth]` (full extents, not half-extents).
///
/// # Return value
/// `Some((idx, delta))` on drag:
/// - **BoxFaceHandles**: `idx` = face index 0..5, `delta` = outward movement.
///   The caller is responsible for updating both `size[axis]` and `position[axis]`.
/// - **BoxAxisArrows**: `idx` = axis index 0..2, `delta` = symmetric resize delta.
///   The caller should add `delta * 2.0` to `size[axis]`.
pub fn draw_box_and_interact(
    ui:               &mut Ui,
    viewport_rect:    Rect,
    entity_world_pos: Vec3,
    csg_size:         [f32; 3],
    view:             Mat4,
    proj:             Mat4,
    mode:             GizmoMode,
    drag:             &mut GizmoDragState,
) -> Option<(usize, f32)> {
    let vp_w = viewport_rect.width();
    let vp_h = viewport_rect.height();
    if vp_w < 1.0 || vp_h < 1.0 { return None; }

    let view_proj  = proj * view;
    let handle_len = (vp_w.min(vp_h) * 0.10).clamp(40.0, 90.0);
    let painter    = ui.painter_at(viewport_rect);

    let cursor        = ui.input(|i| i.pointer.hover_pos());
    let dragging      = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let just_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

    if just_released {
        drag.active_axis  = None;
        drag.box_drag_face = None;
    }

    drag.pending_delta = None;

    let center_screen = world_to_screen(entity_world_pos, view_proj, viewport_rect);

    match mode {
        // ── 6 face-square handles ─────────────────────────────────────────────
        GizmoMode::BoxFaceHandles => {
            // Compute face center positions and their screen projections.
            let half = [csg_size[0] * 0.5, csg_size[1] * 0.5, csg_size[2] * 0.5];
            let face_world: Vec<Vec3> = FACES.iter().map(|&(sign, ax, _)| {
                entity_world_pos + ax.world_dir() * sign * half[ax as usize]
            }).collect();
            let face_screen: Vec<Option<Pos2>> = face_world.iter()
                .map(|&wp| world_to_screen(wp, view_proj, viewport_rect))
                .collect();

            // Pick active face on drag start.
            if dragging && drag.box_drag_face.is_none() {
                if let Some(cp) = cursor {
                    if viewport_rect.contains(cp) {
                        for (fi, fs) in face_screen.iter().enumerate() {
                            if let Some(sp) = *fs {
                                if (cp - sp).length() < 14.0 {
                                    drag.box_drag_face = Some(fi as u8);
                                    drag.active_axis   = Some(FACES[fi].1);
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            // Compute drag delta.
            let mut result = None;
            if dragging {
                if let (Some(fi), Some(cs)) = (drag.box_drag_face, center_screen) {
                    let fi = fi as usize;
                    let (sign, ax, face_idx) = FACES[fi];
                    let drag_delta = ui.input(|i| i.pointer.delta());
                    // Project onto screen-space outward direction.
                    let outward_world = entity_world_pos + ax.world_dir() * sign;
                    if let Some(outward_screen) = world_to_screen(outward_world, view_proj, viewport_rect) {
                        let screen_dir = (outward_screen - cs).normalized();
                        let delta = drag_delta.dot(screen_dir) / handle_len;
                        if delta.abs() > 1e-7 {
                            result = Some((face_idx, delta));
                            drag.pending_delta = Some((face_idx, delta, GizmoMode::BoxFaceHandles));
                        }
                    }
                    let _ = sign; let _ = ax;
                }
            }

            // Draw: thin guide lines from center + colored squares at faces.
            if let Some(cs) = center_screen {
                for (fi, fs) in face_screen.iter().enumerate() {
                    if let Some(sp) = *fs {
                        let (_, ax, _) = FACES[fi];
                        let base = ax.color();
                        let is_active  = drag.box_drag_face == Some(fi as u8);
                        let is_hovered = cursor.map(|c|
                            viewport_rect.contains(c) && (c - sp).length() < 14.0
                        ).unwrap_or(false);
                        let color = if is_active  { brighten(base, 1.5) }
                                    else if is_hovered { brighten(base, 1.2) }
                                    else { Color32::from_rgba_premultiplied(
                                               base.r(), base.g(), base.b(), 210) };
                        painter.line_segment([cs, sp], Stroke::new(1.0,
                            Color32::from_rgba_premultiplied(base.r(), base.g(), base.b(), 80)));
                        let r = egui::Rect::from_center_size(sp, egui::vec2(14.0, 14.0));
                        painter.rect_filled(r, 2.0, color);
                        painter.rect_stroke(r, 2.0, Stroke::new(1.5, Color32::WHITE));
                    }
                }
                painter.circle_filled(cs, 3.5, Color32::WHITE);
            }

            result
        }

        // ── 3 symmetric axis arrows from face edges ───────────────────────────
        GizmoMode::BoxAxisArrows => {
            let half = [csg_size[0] * 0.5, csg_size[1] * 0.5, csg_size[2] * 0.5];

            // Arrow: starts at face edge (+side), points outward.
            let axis_tips: Vec<(Axis, usize, Pos2, Pos2)> = AXES.iter().enumerate().filter_map(|(ai, &ax)| {
                let edge_world = entity_world_pos + ax.world_dir() * half[ai];
                let tip_world  = edge_world + ax.world_dir();
                let edge_screen = world_to_screen(edge_world, view_proj, viewport_rect)?;
                let tip_screen  = world_to_screen(tip_world,  view_proj, viewport_rect)?;
                let dir = (tip_screen - edge_screen).normalized();
                if dir.length() < 0.001 { return None; }
                Some((ax, ai, edge_screen, edge_screen + dir * handle_len))
            }).collect();

            // Pick active axis.
            if dragging && drag.active_axis.is_none() {
                if let Some(cp) = cursor {
                    if viewport_rect.contains(cp) {
                        for &(ax, _, from, to) in &axis_tips {
                            if dist_point_to_segment(cp, from, to) < 10.0 {
                                drag.active_axis = Some(ax);
                                break;
                            }
                        }
                    }
                }
            }

            // Compute drag delta.
            let mut result = None;
            if dragging {
                if let Some(active) = drag.active_axis {
                    let drag_delta = ui.input(|i| i.pointer.delta());
                    if let Some(&(_, ai, from, to)) = axis_tips.iter().find(|(ax, ..)| *ax == active) {
                        let screen_dir = (to - from).normalized();
                        let delta = drag_delta.dot(screen_dir) / handle_len;
                        if delta.abs() > 1e-7 {
                            result = Some((ai, delta));
                            drag.pending_delta = Some((ai, delta, GizmoMode::BoxAxisArrows));
                        }
                    }
                }
            }

            // Draw arrows.
            for &(ax, _, from, to) in &axis_tips {
                let base = ax.color();
                let is_active  = drag.active_axis == Some(ax);
                let is_hovered = cursor.map(|c|
                    viewport_rect.contains(c) && dist_point_to_segment(c, from, to) < 10.0
                ).unwrap_or(false);
                let color = if is_active  { brighten(base, 1.5) }
                            else if is_hovered { brighten(base, 1.2) }
                            else { base };
                let thickness = if is_active { 3.0 } else { 2.0 };
                draw_translate_handle(&painter, from, to, color, thickness);
            }
            if let Some(cs) = center_screen {
                painter.circle_filled(cs, 3.5, Color32::WHITE);
            }

            result
        }

        _ => None,
    }
}
