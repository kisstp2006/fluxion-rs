// ============================================================
// ui_module.rs — fluxion::ui Rune module
//
// Wraps a subset of egui widgets as native Rune functions.
// Access to the current `egui::Ui` is provided through a
// thread-local raw pointer that is set/cleared by dock.rs
// around each Rune panel call (always synchronous — no Send needed).
// ============================================================

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

use rune::Module;

thread_local! {
    static CURRENT_UI: Cell<Option<NonNull<egui::Ui>>> = Cell::new(None);
    /// Stored response from the last `image_interactive` call.
    /// Used by gizmo queries (drag delta, hover, painter).
    static VP_RESPONSE: RefCell<Option<egui::Response>> = RefCell::new(None);
    /// Stored rect of the viewport image for coordinate conversion.
    static VP_RECT: Cell<egui::Rect> = Cell::new(egui::Rect::NOTHING);
}

/// RAII guard returned by `set_current_ui`.
/// Clears the `CURRENT_UI` thread-local on drop, even if a panic unwinds
/// through the panel call.
pub struct UiContextGuard;

impl Drop for UiContextGuard {
    fn drop(&mut self) {
        CURRENT_UI.with(|c| c.set(None));
    }
}

/// Set the active UI context before calling a Rune panel function.
/// Returns a `UiContextGuard` that clears the pointer on drop.
///
/// # Safety
/// The caller guarantees that `ui` outlives the returned guard.
pub fn set_current_ui(ui: &mut egui::Ui) -> UiContextGuard {
    CURRENT_UI.with(|c| c.set(Some(NonNull::from(ui))));
    UiContextGuard
}

/// Clear the active UI context immediately.
/// Prefer holding the `UiContextGuard` from `set_current_ui` instead.
pub fn clear_current_ui() {
    CURRENT_UI.with(|c| c.set(None));
}

fn with_ui<R>(f: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    CURRENT_UI.with(|c| {
        // SAFETY: pointer is valid for the duration of the panel call.
        c.get().map(|mut ptr| unsafe { f(ptr.as_mut()) })
    })
}

pub fn build_ui_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["ui"])?;

    // ── Basic text ────────────────────────────────────────────────────────────

    m.function("label", |text: String| {
        with_ui(|ui| { ui.label(text); });
    }).build()?;

    m.function("heading", |text: String| {
        with_ui(|ui| { ui.heading(text); });
    }).build()?;

    m.function("small", |text: String| {
        with_ui(|ui| { ui.small(text); });
    }).build()?;

    m.function("separator", || {
        with_ui(|ui| { ui.separator(); });
    }).build()?;

    m.function("space", |pixels: i64| {
        with_ui(|ui| { ui.add_space(pixels.max(0) as f32); });
    }).build()?;

    // ── Interactive widgets ───────────────────────────────────────────────────

    m.function("button", |label: String| -> bool {
        with_ui(|ui| ui.button(&label).clicked()).unwrap_or(false)
    }).build()?;

    m.function("checkbox", |label: String, value: bool| -> bool {
        with_ui(|ui| {
            let mut v = value;
            ui.checkbox(&mut v, &label);
            v
        }).unwrap_or(value)
    }).build()?;

    m.function("drag_float", |label: String, value: f64, speed: f64, min: f64, max: f64| -> f64 {
        with_ui(|ui| {
            let mut v = value as f32;
            ui.add(
                egui::DragValue::new(&mut v)
                    .speed(speed as f32)
                    .range(min as f32..=max as f32)
                    .prefix(format!("{label}: ")),
            );
            v as f64
        }).unwrap_or(value)
    }).build()?;

    m.function("drag_int", |label: String, value: i64| -> i64 {
        with_ui(|ui| {
            let mut v = value as i32;
            ui.add(egui::DragValue::new(&mut v).prefix(format!("{label}: ")));
            v as i64
        }).unwrap_or(value)
    }).build()?;

    m.function("slider_float", |label: String, value: f64, min: f64, max: f64| -> f64 {
        with_ui(|ui| {
            let mut v = value as f32;
            ui.horizontal(|ui| {
                ui.label(&label);
                ui.add(egui::Slider::new(&mut v, min as f32..=max as f32));
            });
            v as f64
        }).unwrap_or(value)
    }).build()?;

    m.function("input_text", |label: String, value: String| -> String {
        with_ui(|ui| {
            let mut v = value.clone();
            ui.horizontal(|ui| {
                ui.label(&label);
                ui.text_edit_singleline(&mut v);
            });
            v
        }).unwrap_or(value)
    }).build()?;

    // ── Color pickers ─────────────────────────────────────────────────────────

    m.function("color3", |label: String, r: f64, g: f64, b: f64| -> Vec<f64> {
        with_ui(|ui| {
            let mut c = [r as f32, g as f32, b as f32];
            ui.horizontal(|ui| {
                ui.label(&label);
                ui.color_edit_button_rgb(&mut c);
            });
            vec![c[0] as f64, c[1] as f64, c[2] as f64]
        }).unwrap_or_else(|| vec![r, g, b])
    }).build()?;

    m.function("color4", |label: String, r: f64, g: f64, b: f64, a: f64| -> Vec<f64> {
        with_ui(|ui| {
            let mut c = [r as f32, g as f32, b as f32, a as f32];
            ui.horizontal(|ui| {
                ui.label(&label);
                ui.color_edit_button_rgba_unmultiplied(&mut c);
            });
            vec![c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64]
        }).unwrap_or_else(|| vec![r, g, b, a])
    }).build()?;

    // ── Layout helpers ────────────────────────────────────────────────────────

    /// Begin a collapsing section. Returns true if the section is open.
    /// Pair with collapsing_end() for visual symmetry in Rune code.
    m.function("collapsing_begin", |label: String| -> bool {
        with_ui(|ui| {
            let id = ui.make_persistent_id(&label);
            let is_open = ui.memory_mut(|m| {
                m.data.get_persisted::<bool>(id).unwrap_or(true)
            });
            let clicked = ui.horizontal(|ui| {
                let sym = if is_open { "▼" } else { "▶" };
                ui.small_button(sym).clicked()
            }).inner;
            if clicked {
                let toggled = !is_open;
                ui.memory_mut(|m| m.data.insert_persisted(id, toggled));
            }
            is_open
        }).unwrap_or(false)
    }).build()?;

    m.function("collapsing_end", || {
        // Semantic end-marker — no-op in Rust.
    }).build()?;

    m.function("horizontal_begin", || {
        // Horizontal layout is handled per-widget; this is a no-op placeholder.
    }).build()?;

    m.function("horizontal_end", || {}).build()?;

    // ── Scroll area helpers (no-op stubs — scroll is automatic in egui) ────────
    m.function("scroll_begin", || {}).build()?;
    m.function("scroll_end",   || {}).build()?;

    // ── Viewport image ────────────────────────────────────────────────────────
    // texture_id is the raw u64 from egui::TextureId::Managed (cast to i64 for Rune).
    // Pass width=0, height=0 to auto-fill available space.
    m.function("image", |texture_id: i64, w: f64, h: f64| {
        with_ui(|ui| {
            if texture_id < 0 { return; }
            let raw = texture_id as u64;
            let tid = if (raw >> 62) & 1 == 1 {
                egui::TextureId::User(raw & !(1u64 << 62))
            } else {
                egui::TextureId::Managed(raw)
            };
            let size = if w > 0.0 && h > 0.0 {
                egui::Vec2::new(w as f32, h as f32)
            } else {
                ui.available_size()
            };
            ui.add(egui::Image::new(egui::load::SizedTexture::new(tid, size)));
        });
    }).build()?;

    // ── Entity row (selectable + right-click context menu) ────────────────────
    // Returns: "" (no action), "select" (left-clicked),
    //          "Rename", "Duplicate", or "Delete" (menu item chosen).
    m.function("entity_row", |label: String, _selected: bool| -> String {
        with_ui(|ui| {
            if ui.button(label.as_str()).clicked() {
                "select".to_string()
            } else {
                String::new()
            }
        }).unwrap_or_default()
    }).build()?;

    // ── Colored label ─────────────────────────────────────────────────────────
    m.function("colored_label", |text: String, r: f64, g: f64, b: f64| {
        with_ui(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(
                    (r * 255.0) as u8,
                    (g * 255.0) as u8,
                    (b * 255.0) as u8,
                ),
                &text,
            );
        });
    }).build()?;

    // ── Selectable with right-click context menu ───────────────────────────────
    // Returns "" (no action), "select" (left-clicked), or the menu item string.
    m.function("selectable_with_menu", |label: String, selected: bool, items: Vec<String>| -> String {
        with_ui(|ui| {
            let mut action = String::new();
            let response = ui.selectable_label(selected, &label);
            response.context_menu(|ui| {
                for item in &items {
                    if ui.button(item).clicked() {
                        action = item.clone();
                        ui.close_menu();
                    }
                }
            });
            if response.clicked() && action.is_empty() {
                "select".to_string()
            } else {
                action
            }
        }).unwrap_or_default()
    }).build()?;

    // ── Combo box ─────────────────────────────────────────────────────────────
    // Returns the selected item string, or "" if nothing was chosen this frame.
    m.function("combo_box", |label: String, items: Vec<String>| -> String {
        with_ui(|ui| {
            let mut chosen = String::new();
            egui::ComboBox::from_label(&label)
                .selected_text(&label)
                .show_ui(ui, |ui| {
                    for item in &items {
                        if ui.selectable_label(false, item).clicked() {
                            chosen = item.clone();
                        }
                    }
                });
            chosen
        }).unwrap_or_default()
    }).build()?;

    // ── Size query ────────────────────────────────────────────────────────────
    m.function("available_width", || -> f64 {
        with_ui(|ui| ui.available_width() as f64).unwrap_or(0.0)
    }).build()?;

    m.function("available_height", || -> f64 {
        with_ui(|ui| ui.available_height() as f64).unwrap_or(0.0)
    }).build()?;

    // ── Interactive viewport image (stores response for gizmo queries) ─────────
    // Returns [rect_x, rect_y, rect_w, rect_h] of the rendered image.
    m.function("image_interactive", |texture_id: i64, w: f64, h: f64| -> Vec<f64> {
        with_ui(|ui| {
            if texture_id < 0 {
                return vec![0.0f64; 4];
            }
            let raw = texture_id as u64;
            let tid = if (raw >> 62) & 1 == 1 {
                egui::TextureId::User(raw & !(1u64 << 62))
            } else {
                egui::TextureId::Managed(raw)
            };
            let size = if w > 0.0 && h > 0.0 {
                egui::Vec2::new(w as f32, h as f32)
            } else {
                ui.available_size()
            };
            let resp = ui.add(
                egui::Image::new(egui::load::SizedTexture::new(tid, size))
                    .sense(egui::Sense::drag()),
            );
            let rect = resp.rect;
            VP_RECT.with(|c| c.set(rect));
            VP_RESPONSE.with(|r| *r.borrow_mut() = Some(resp));
            vec![rect.min.x as f64, rect.min.y as f64, rect.width() as f64, rect.height() as f64]
        }).unwrap_or_else(|| vec![0.0f64; 4])
    }).build()?;

    // Returns [drag_dx, drag_dy] for the last image_interactive widget.
    m.function("viewport_drag_delta", || -> Vec<f64> {
        let delta = VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| resp.drag_delta())
        }).unwrap_or(egui::Vec2::ZERO);
        vec![delta.x as f64, delta.y as f64]
    }).build()?;

    // Returns true if the mouse is hovering the viewport image.
    m.function("viewport_hovered", || -> bool {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| resp.hovered()).unwrap_or(false)
        })
    }).build()?;

    // Returns true if the mouse is pressed/dragging on the viewport image.
    m.function("viewport_dragging", || -> bool {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| resp.dragged()).unwrap_or(false)
        })
    }).build()?;

    // Returns [mouse_x, mouse_y] relative to viewport image top-left (pixels).
    m.function("viewport_mouse_pos", || -> Vec<f64> {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().and_then(|resp| resp.hover_pos()).map(|p| {
                let rect = VP_RECT.with(|c| c.get());
                vec![(p.x - rect.min.x) as f64, (p.y - rect.min.y) as f64]
            })
        }).unwrap_or_else(|| vec![-1.0, -1.0])
    }).build()?;

    // Draw a line overlay on top of the viewport.
    // pts = [x1, y1, x2, y2] (viewport-relative pixels)
    // style = [r, g, b, a, thickness]
    m.function("painter_line", |pts: Vec<f64>, style: Vec<f64>| {
        if pts.len() < 4 || style.len() < 5 { return; }
        VP_RESPONSE.with(|resp_ref| {
            let borrow = resp_ref.borrow();
            let Some(resp) = borrow.as_ref() else { return; };
            let rect = VP_RECT.with(|c| c.get());
            let painter = resp.ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("gizmo_layer"),
            ));
            let p1 = egui::pos2(rect.min.x + pts[0] as f32, rect.min.y + pts[1] as f32);
            let p2 = egui::pos2(rect.min.x + pts[2] as f32, rect.min.y + pts[3] as f32);
            let color = egui::Color32::from_rgba_unmultiplied(
                (style[0] * 255.0) as u8,
                (style[1] * 255.0) as u8,
                (style[2] * 255.0) as u8,
                (style[3] * 255.0) as u8,
            );
            painter.line_segment([p1, p2], egui::Stroke::new(style[4] as f32, color));
        });
    }).build()?;

    // Draw an arrow from (sx,sy) to (ex,ey) with arrowhead.
    // pts = [sx, sy, ex, ey]; style = [r, g, b, thickness]
    m.function("painter_arrow", |pts: Vec<f64>, style: Vec<f64>| {
        if pts.len() < 4 || style.len() < 4 { return; }
        VP_RESPONSE.with(|resp_ref| {
            let borrow = resp_ref.borrow();
            let Some(resp) = borrow.as_ref() else { return; };
            let rect = VP_RECT.with(|c| c.get());
            let painter = resp.ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("gizmo_arrow_layer"),
            ));
            let origin = egui::pos2(rect.min.x + pts[0] as f32, rect.min.y + pts[1] as f32);
            let tip    = egui::pos2(rect.min.x + pts[2] as f32, rect.min.y + pts[3] as f32);
            let color  = egui::Color32::from_rgb(
                (style[0] * 255.0) as u8,
                (style[1] * 255.0) as u8,
                (style[2] * 255.0) as u8,
            );
            let thickness = style[3] as f32;
            painter.line_segment([origin, tip], egui::Stroke::new(thickness, color));
            let dx = tip.x - origin.x;
            let dy = tip.y - origin.y;
            let len = (dx * dx + dy * dy).sqrt().max(0.001);
            let nx = dx / len;
            let ny = dy / len;
            let head = 10.0f32;
            let a1 = egui::pos2(tip.x - head * (nx + ny * 0.5), tip.y - head * (ny - nx * 0.5));
            let a2 = egui::pos2(tip.x - head * (nx - ny * 0.5), tip.y - head * (ny + nx * 0.5));
            painter.line_segment([tip, a1], egui::Stroke::new(thickness, color));
            painter.line_segment([tip, a2], egui::Stroke::new(thickness, color));
        });
    }).build()?;

    Ok(m)
}
