// ============================================================
// gizmo_module.rs — fluxion::gizmo Rune module
//
// Provides interactive transform gizmos for the viewport.
// Gizmo drawing uses egui Painter lines stored in thread-locals.
// Drag state is read from the viewport image response stored by
// the `image_interactive` call in ui_module.rs.
// ============================================================

use std::cell::{Cell, RefCell};
use rune::Module;

// ── Thread-local state ────────────────────────────────────────────────────────

thread_local! {
    /// Which gizmo axis (0=X,1=Y,2=Z) is currently being dragged, -1=none.
    static ACTIVE_AXIS:  Cell<i64>       = Cell::new(-1);
    /// Accumulated drag delta on the active axis (world-space units).
    static DRAG_ACCUM:   Cell<f64>       = Cell::new(0.0);
    /// Transform tool mode: "translate" | "rotate" | "scale"
    pub static TOOL_MODE: RefCell<String> = RefCell::new("translate".to_string());
    /// Gizmo screen-space lines queued this frame: [(x1,y1,x2,y2,r,g,b,a,thickness)]
    pub static GIZMO_LINES: RefCell<Vec<(f32,f32,f32,f32,f32,f32,f32,f32,f32)>> = RefCell::new(Vec::new());
    /// Gizmo hit test results: [(axis, center_x, center_y, end_x, end_y)]
    static GIZMO_AXES: RefCell<Vec<(i64, f32, f32, f32, f32)>> = RefCell::new(Vec::new());
}

// ── Public helpers called by host ────────────────────────────────────────────

/// Called by ui_module after the viewport image widget to process mouse drag.
/// `mouse_x/y` are screen-relative coordinates. `drag_dx/dy` are egui drag deltas.
/// Returns (axis, delta) if a drag is active.
pub fn process_gizmo_interaction(
    mouse_x: f32,
    mouse_y: f32,
    drag_dx: f32,
    drag_dy: f32,
    is_pressed: bool,
) -> (i64, f64) {
    if !is_pressed {
        // Mouse released — clear active axis
        ACTIVE_AXIS.with(|c| c.set(-1));
        DRAG_ACCUM.with(|c| c.set(0.0));
        return (-1, 0.0);
    }

    let active = ACTIVE_AXIS.with(|c| c.get());

    if active < 0 {
        // Try to pick an axis: check if mouse is near any gizmo axis end
        let picked = GIZMO_AXES.with(|axes| {
            let axes = axes.borrow();
            for &(axis, cx, cy, ex, ey) in axes.iter() {
                // Check distance from mouse to the axis line segment
                let dx = ex - cx;
                let dy = ey - cy;
                let len2 = dx * dx + dy * dy;
                if len2 < 0.001 { continue; }
                let t = ((mouse_x - cx) * dx + (mouse_y - cy) * dy) / len2;
                let t = t.clamp(0.0, 1.0);
                let px = cx + t * dx;
                let py = cy + t * dy;
                let dist = ((mouse_x - px).powi(2) + (mouse_y - py).powi(2)).sqrt();
                if dist < 12.0 {
                    return axis;
                }
            }
            -1
        });
        if picked >= 0 {
            ACTIVE_AXIS.with(|c| c.set(picked));
        }
        return (picked, 0.0);
    }

    // Compute delta along the active axis direction
    let delta = GIZMO_AXES.with(|axes| {
        let axes = axes.borrow();
        for &(axis, cx, cy, ex, ey) in axes.iter() {
            if axis == active {
                let dx = ex - cx;
                let dy = ey - cy;
                let len = (dx * dx + dy * dy).sqrt();
                if len < 0.001 { return 0.0f32; }
                // Project drag delta onto axis screen direction
                return (drag_dx * dx / len + drag_dy * dy / len) / 60.0;
            }
        }
        0.0f32
    });

    DRAG_ACCUM.with(|c| c.set(c.get() + delta as f64));
    (active, delta as f64)
}

/// Clear gizmo draw queues at start of each frame.
pub fn clear_gizmo_frame() {
    GIZMO_LINES.with(|l| l.borrow_mut().clear());
    GIZMO_AXES .with(|a| a.borrow_mut().clear());
}

// ── Rune module builder ───────────────────────────────────────────────────────

pub fn build_gizmo_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["gizmo"])?;

    // Set the active transform tool mode.
    m.function("set_tool", |mode: String| {
        TOOL_MODE.with(|t| *t.borrow_mut() = mode);
    }).build()?;

    m.function("get_tool", || -> String {
        TOOL_MODE.with(|t| t.borrow().clone())
    }).build()?;

    // Queue a translate gizmo for the given entity (projected screen position).
    // cx, cy = screen-space center of entity (pixels relative to viewport top-left).
    // size   = arrow length in pixels.
    m.function("draw_axes", |cx: f64, cy: f64, size: f64| {
        let (cx, cy, size) = (cx as f32, cy as f32, size as f32);
        // X axis — red, points right
        let ex = cx + size;
        let ey = cy;
        GIZMO_LINES.with(|l| l.borrow_mut().push((cx, cy, ex, ey, 1.0, 0.2, 0.2, 1.0, 2.5)));
        GIZMO_AXES .with(|a| a.borrow_mut().push((0, cx, cy, ex, ey)));
        // Y axis — green, points up (screen y inverted)
        let ex2 = cx;
        let ey2 = cy - size;
        GIZMO_LINES.with(|l| l.borrow_mut().push((cx, cy, ex2, ey2, 0.2, 1.0, 0.2, 1.0, 2.5)));
        GIZMO_AXES .with(|a| a.borrow_mut().push((1, cx, cy, ex2, ey2)));
        // Z axis — blue, points toward camera (diagonal on screen)
        let ex3 = cx - size * 0.6;
        let ey3 = cy - size * 0.6;
        GIZMO_LINES.with(|l| l.borrow_mut().push((cx, cy, ex3, ey3, 0.2, 0.4, 1.0, 1.0, 2.5)));
        GIZMO_AXES .with(|a| a.borrow_mut().push((2, cx, cy, ex3, ey3)));
    }).build()?;

    // Returns [axis, delta] for the last frame's drag interaction.
    // axis: 0=X, 1=Y, 2=Z, -1=none. delta: movement amount.
    m.function("drag_result", || -> Vec<f64> {
        let axis  = ACTIVE_AXIS.with(|c| c.get());
        let delta = DRAG_ACCUM .with(|c| c.get());
        // Consume the accumulation
        DRAG_ACCUM.with(|c| c.set(0.0));
        vec![axis as f64, delta]
    }).build()?;

    Ok(m)
}
