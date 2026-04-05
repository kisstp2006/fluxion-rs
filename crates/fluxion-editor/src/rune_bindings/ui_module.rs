// ============================================================
// ui_module.rs — fluxion::ui Rune module
//
// Wraps a subset of egui widgets as native Rune functions.
// All String parameters use Ref<str> so Rune does NOT snapshot
// the caller's variable — prevents M-000000 AccessError.
// ============================================================

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

use rune::{Module, runtime::Ref};

thread_local! {
    static CURRENT_UI: Cell<Option<NonNull<egui::Ui>>> = Cell::new(None);
    /// Stored response from the last `image_interactive` call.
    static VP_RESPONSE: RefCell<Option<egui::Response>> = RefCell::new(None);
    /// Stored rect of the viewport image for coordinate conversion.
    pub static VP_RECT: Cell<egui::Rect> = Cell::new(egui::Rect::NOTHING);
    /// Response from the last widget call — used by `prop_context_menu`.
    static LAST_WIDGET_RESP: RefCell<Option<egui::Response>> = RefCell::new(None);
    /// Pending cursor grab/visible requests from Rune scripts.
    static CURSOR_GRAB:    Cell<Option<bool>> = Cell::new(None);
    static CURSOR_VISIBLE: Cell<Option<bool>> = Cell::new(None);
    /// Raw mouse delta accumulated from DeviceEvent::MouseMotion each frame.
    /// This works even when the cursor is locked (unlike egui pointer delta).
    static RAW_MOUSE_DELTA: Cell<(f64, f64)> = Cell::new((0.0, 0.0));
}

/// Returns the last viewport image rect (set by `image_interactive`).
pub fn get_viewport_rect() -> egui::Rect {
    VP_RECT.with(|c| c.get())
}

pub struct UiContextGuard;

impl Drop for UiContextGuard {
    fn drop(&mut self) {
        CURRENT_UI.with(|c| c.set(None));
    }
}

pub fn set_current_ui(ui: &mut egui::Ui) -> UiContextGuard {
    CURRENT_UI.with(|c| c.set(Some(NonNull::from(ui))));
    UiContextGuard
}

pub fn clear_current_ui() {
    CURRENT_UI.with(|c| c.set(None));
}

fn with_ui<R>(f: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    CURRENT_UI.with(|c| {
        c.get().map(|mut ptr| unsafe { f(ptr.as_mut()) })
    })
}

/// Accumulate raw mouse motion (call from DeviceEvent::MouseMotion each event).
pub fn accumulate_raw_mouse_delta(dx: f64, dy: f64) {
    RAW_MOUSE_DELTA.with(|c| {
        let (ox, oy) = c.get();
        c.set((ox + dx, oy + dy));
    });
}

/// Drain (read and reset) the raw mouse delta for this frame.
pub fn drain_raw_mouse_delta() -> (f64, f64) {
    RAW_MOUSE_DELTA.with(|c| c.replace((0.0, 0.0)))
}

/// Drain the pending cursor grab request set by Rune scripts this frame.
/// Returns `Some(true)` = grab+hide, `Some(false)` = release+show, `None` = no change.
pub fn drain_cursor_grab() -> Option<bool> {
    CURSOR_GRAB.with(|c| c.take())
}

/// Drain the pending cursor visibility request.
pub fn drain_cursor_visible() -> Option<bool> {
    CURSOR_VISIBLE.with(|c| c.take())
}

pub fn build_ui_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["ui"])?;

    // ── Basic text ────────────────────────────────────────────────────────────

    m.function("label", |text: Ref<str>| {
        with_ui(|ui| { ui.label(text.as_ref()); });
    }).build()?;

    m.function("heading", |text: Ref<str>| {
        with_ui(|ui| { ui.heading(text.as_ref()); });
    }).build()?;

    m.function("small", |text: Ref<str>| {
        with_ui(|ui| { ui.small(text.as_ref()); });
    }).build()?;

    m.function("separator", || {
        with_ui(|ui| { ui.separator(); });
    }).build()?;

    m.function("space", |pixels: i64| {
        with_ui(|ui| { ui.add_space(pixels.max(0) as f32); });
    }).build()?;

    // ── Interactive widgets ───────────────────────────────────────────────────

    m.function("button", |label: Ref<str>| -> bool {
        with_ui(|ui| ui.button(label.as_ref()).clicked()).unwrap_or(false)
    }).build()?;

    m.function("checkbox", |label: Ref<str>, value: bool| -> bool {
        with_ui(|ui| {
            let mut v = value;
            ui.checkbox(&mut v, label.as_ref());
            v
        }).unwrap_or(value)
    }).build()?;

    m.function("drag_float", |label: Ref<str>, value: f64, speed: f64, min: f64, max: f64| -> f64 {
        with_ui(|ui| {
            let mut v = value as f32;
            ui.add(
                egui::DragValue::new(&mut v)
                    .speed(speed as f32)
                    .range(min as f32..=max as f32)
                    .prefix(format!("{}: ", label.as_ref())),
            );
            v as f64
        }).unwrap_or(value)
    }).build()?;

    m.function("drag_int", |label: Ref<str>, value: i64| -> i64 {
        with_ui(|ui| {
            let mut v = value as i32;
            ui.add(egui::DragValue::new(&mut v).prefix(format!("{}: ", label.as_ref())));
            v as i64
        }).unwrap_or(value)
    }).build()?;

    m.function("slider_float", |label: Ref<str>, value: f64, min: f64, max: f64| -> f64 {
        with_ui(|ui| {
            let mut v = value as f32;
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                ui.add(egui::Slider::new(&mut v, min as f32..=max as f32));
            });
            v as f64
        }).unwrap_or(value)
    }).build()?;

    m.function("input_text", |label: Ref<str>, value: Ref<str>| -> String {
        with_ui(|ui| {
            let mut v = value.as_ref().to_string();
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                ui.text_edit_singleline(&mut v);
            });
            v
        }).unwrap_or_else(|| value.as_ref().to_string())
    }).build()?;

    // ── Color pickers ─────────────────────────────────────────────────────────

    m.function("color3", |label: Ref<str>, r: f64, g: f64, b: f64| -> Vec<f64> {
        with_ui(|ui| {
            let mut c = [r as f32, g as f32, b as f32];
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                ui.color_edit_button_rgb(&mut c);
            });
            vec![c[0] as f64, c[1] as f64, c[2] as f64]
        }).unwrap_or_else(|| vec![r, g, b])
    }).build()?;

    m.function("color4", |label: Ref<str>, r: f64, g: f64, b: f64, a: f64| -> Vec<f64> {
        with_ui(|ui| {
            let mut c = [r as f32, g as f32, b as f32, a as f32];
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                ui.color_edit_button_rgba_unmultiplied(&mut c);
            });
            vec![c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64]
        }).unwrap_or_else(|| vec![r, g, b, a])
    }).build()?;

    // ── Layout helpers ────────────────────────────────────────────────────────

    m.function("collapsing_begin", |label: Ref<str>| -> bool {
        with_ui(|ui| {
            let id = ui.make_persistent_id(label.as_ref());
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

    m.function("collapsing_end", || {}).build()?;
    m.function("horizontal_begin", || {}).build()?;
    m.function("horizontal_end", || {}).build()?;
    m.function("scroll_begin", || {}).build()?;
    m.function("scroll_end",   || {}).build()?;

    // ── Viewport image ────────────────────────────────────────────────────────

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

    m.function("entity_row", |label: Ref<str>, _selected: bool| -> String {
        with_ui(|ui| {
            if ui.button(label.as_ref()).clicked() {
                "select".to_string()
            } else {
                String::new()
            }
        }).unwrap_or_default()
    }).build()?;

    m.function("colored_label", |text: Ref<str>, r: f64, g: f64, b: f64| {
        with_ui(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(
                    (r * 255.0) as u8,
                    (g * 255.0) as u8,
                    (b * 255.0) as u8,
                ),
                text.as_ref(),
            );
        });
    }).build()?;

    m.function("selectable_with_menu", |label: Ref<str>, selected: bool, items: Vec<String>| -> String {
        with_ui(|ui| {
            let mut action = String::new();
            let response = ui.selectable_label(selected, label.as_ref());
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

    m.function("combo_box", |label: Ref<str>, items: Vec<String>| -> String {
        with_ui(|ui| {
            let mut chosen = String::new();
            egui::ComboBox::from_label(label.as_ref())
                .selected_text(label.as_ref())
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

    // ── Interactive viewport image ────────────────────────────────────────────

    m.function("image_interactive", |texture_id: i64, w: f64, h: f64| -> Vec<f64> {
        with_ui(|ui| {
            if texture_id < 0 { return vec![0.0f64; 4]; }
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

    m.function("viewport_drag_delta", || -> Vec<f64> {
        let delta = VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| resp.drag_delta())
        }).unwrap_or(egui::Vec2::ZERO);
        vec![delta.x as f64, delta.y as f64]
    }).build()?;

    m.function("viewport_hovered", || -> bool {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| resp.hovered()).unwrap_or(false)
        })
    }).build()?;

    m.function("viewport_dragging", || -> bool {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| resp.dragged()).unwrap_or(false)
        })
    }).build()?;

    m.function("viewport_mouse_pos", || -> Vec<f64> {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().and_then(|resp| resp.hover_pos()).map(|p| {
                let rect = VP_RECT.with(|c| c.get());
                vec![(p.x - rect.min.x) as f64, (p.y - rect.min.y) as f64]
            })
        }).unwrap_or_else(|| vec![-1.0, -1.0])
    }).build()?;

    m.function("viewport_scroll_delta", || -> f64 {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| {
                resp.ctx.input(|i| i.smooth_scroll_delta.y) as f64
            }).unwrap_or(0.0)
        })
    }).build()?;

    m.function("viewport_right_dragging", || -> bool {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| {
                resp.ctx.input(|i| i.pointer.button_down(egui::PointerButton::Secondary))
            }).unwrap_or(false)
        })
    }).build()?;

    m.function("viewport_middle_dragging", || -> bool {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| {
                resp.ctx.input(|i| i.pointer.button_down(egui::PointerButton::Middle))
            }).unwrap_or(false)
        })
    }).build()?;

    m.function("viewport_alt_held", || -> bool {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| {
                resp.ctx.input(|i| i.modifiers.alt)
            }).unwrap_or(false)
        })
    }).build()?;

    m.function("viewport_shift_held", || -> bool {
        VP_RESPONSE.with(|r| {
            r.borrow().as_ref().map(|resp| {
                resp.ctx.input(|i| i.modifiers.shift)
            }).unwrap_or(false)
        })
    }).build()?;

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

    // ── Inline vector widgets (Unity-style horizontal X/Y/Z) ──────────────────

    m.function("vec3_inline", |label: Ref<str>, x: f64, y: f64, z: f64, speed: f64| -> Vec<f64> {
        with_ui(|ui| {
            let mut v = [x as f32, y as f32, z as f32];
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80),   "X");
                let resp = ui.add(egui::DragValue::new(&mut v[0]).speed(speed as f32));
                LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(resp));
                ui.colored_label(egui::Color32::from_rgb(80, 200, 80),   "Y");
                ui.add(egui::DragValue::new(&mut v[1]).speed(speed as f32));
                ui.colored_label(egui::Color32::from_rgb(80, 120, 220),  "Z");
                ui.add(egui::DragValue::new(&mut v[2]).speed(speed as f32));
            });
            vec![v[0] as f64, v[1] as f64, v[2] as f64]
        }).unwrap_or_else(|| vec![x, y, z])
    }).build()?;

    m.function("vec2_inline", |label: Ref<str>, x: f64, y: f64, speed: f64| -> Vec<f64> {
        with_ui(|ui| {
            let mut v = [x as f32, y as f32];
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80),  "X");
                let resp = ui.add(egui::DragValue::new(&mut v[0]).speed(speed as f32));
                LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(resp));
                ui.colored_label(egui::Color32::from_rgb(80, 200, 80),  "Y");
                ui.add(egui::DragValue::new(&mut v[1]).speed(speed as f32));
            });
            vec![v[0] as f64, v[1] as f64]
        }).unwrap_or_else(|| vec![x, y])
    }).build()?;

    m.function("vec4_inline", |label: Ref<str>, vals: Vec<f64>, speed: f64| -> Vec<f64> {
        let (x, y, z, w) = if vals.len() >= 4 {
            (vals[0], vals[1], vals[2], vals[3])
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };
        with_ui(|ui| {
            let mut v = [x as f32, y as f32, z as f32, w as f32];
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80),   "X");
                let resp = ui.add(egui::DragValue::new(&mut v[0]).speed(speed as f32));
                LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(resp));
                ui.colored_label(egui::Color32::from_rgb(80, 200, 80),   "Y");
                ui.add(egui::DragValue::new(&mut v[1]).speed(speed as f32));
                ui.colored_label(egui::Color32::from_rgb(80, 120, 220),  "Z");
                ui.add(egui::DragValue::new(&mut v[2]).speed(speed as f32));
                ui.colored_label(egui::Color32::from_rgb(160, 160, 160), "W");
                ui.add(egui::DragValue::new(&mut v[3]).speed(speed as f32));
            });
            vec![v[0] as f64, v[1] as f64, v[2] as f64, v[3] as f64]
        }).unwrap_or_else(|| vec![x, y, z, w])
    }).build()?;

    m.function("quat_euler_inline", |label: Ref<str>, pitch: f64, yaw: f64, roll: f64| -> Vec<f64> {
        with_ui(|ui| {
            let mut v = [pitch as f32, yaw as f32, roll as f32];
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80),   "P");
                let resp = ui.add(egui::DragValue::new(&mut v[0]).speed(0.5f32).range(-360.0f32..=360.0f32));
                LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(resp));
                ui.colored_label(egui::Color32::from_rgb(80, 200, 80),   "Y");
                ui.add(egui::DragValue::new(&mut v[1]).speed(0.5f32).range(-360.0f32..=360.0f32));
                ui.colored_label(egui::Color32::from_rgb(80, 120, 220),  "R");
                ui.add(egui::DragValue::new(&mut v[2]).speed(0.5f32).range(-360.0f32..=360.0f32));
            });
            vec![v[0] as f64, v[1] as f64, v[2] as f64]
        }).unwrap_or_else(|| vec![pitch, yaw, roll])
    }).build()?;

    // ── Two-column layout helpers ─────────────────────────────────────────────

    m.function("prop_row_begin", |label: Ref<str>| {
        with_ui(|ui| {
            let available = ui.available_width();
            let label_width = (available * 0.40).max(80.0).min(160.0);
            ui.horizontal(|ui| {
                ui.set_min_width(available);
                ui.add_sized(
                    egui::Vec2::new(label_width, ui.spacing().interact_size.y),
                    egui::Label::new(
                        egui::RichText::new(label.as_ref())
                            .color(egui::Color32::from_rgb(180, 180, 190))
                    ),
                );
            });
        });
    }).build()?;

    // ── Context menu system ───────────────────────────────────────────────────

    m.function("prop_context_menu", |_field_id: Ref<str>| -> String {
        let mut action = String::new();
        LAST_WIDGET_RESP.with(|resp_ref| {
            if let Some(resp) = resp_ref.borrow().as_ref() {
                resp.context_menu(|ui| {
                    if ui.button("Copy value").clicked() {
                        action = "copy".to_string();
                        ui.close_menu();
                    }
                    if ui.button("Paste value").clicked() {
                        action = "paste".to_string();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Reset to default").clicked() {
                        action = "reset".to_string();
                        ui.close_menu();
                    }
                });
            }
        });
        action
    }).build()?;

    m.function("copy_to_clipboard", |text: Ref<str>| {
        with_ui(|ui| {
            ui.ctx().copy_text(text.as_ref().to_string());
        });
    }).build()?;

    m.function("paste_from_clipboard", || -> String {
        with_ui(|ui| {
            ui.ctx().input(|i| i.events.iter().find_map(|e| {
                if let egui::Event::Paste(s) = e { Some(s.clone()) } else { None }
            })).unwrap_or_default()
        }).unwrap_or_default()
    }).build()?;

    // ── Enum combo-box ────────────────────────────────────────────────────────

    m.function("enum_combo", |label: Ref<str>, current: Ref<str>, options: Vec<String>| -> String {
        with_ui(|ui| {
            let mut chosen = current.as_ref().to_string();
            let resp = egui::ComboBox::from_label(label.as_ref())
                .selected_text(current.as_ref())
                .show_ui(ui, |ui| {
                    for opt in &options {
                        ui.selectable_value(&mut chosen, opt.clone(), opt.as_str());
                    }
                });
            LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(resp.response));
            chosen
        }).unwrap_or_else(|| current.as_ref().to_string())
    }).build()?;

    // ── Asset path picker ─────────────────────────────────────────────────────

    m.function("asset_path_picker", |label: Ref<str>, path: Ref<str>| -> String {
        with_ui(|ui| {
            let mut v = path.as_ref().to_string();
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                let resp = ui.text_edit_singleline(&mut v);
                LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(resp));
                ui.small("…");
            });
            v
        }).unwrap_or_else(|| path.as_ref().to_string())
    }).build()?;

    // ── Menu bar bindings (for menubar.rn) ────────────────────────────────────

    m.function("menu_bar_begin", || {
        // The actual TopBottomPanel is opened by the Rust main loop context.
        // This is a no-op placeholder — the panel is driven from main.rs via a
        // Rune call whose ui pointer is a menu-bar ui.
    }).build()?;

    m.function("menu_bar_end", || {}).build()?;

    m.function("menu_begin", |label: Ref<str>| -> bool {
        // Returns true so Rune code can call menu_item inside an `if` block.
        // The actual open/close is handled by egui's menu_button.
        // We use a thread-local bool to track open state per-frame.
        with_ui(|ui| {
            let mut opened = false;
            ui.menu_button(label.as_ref(), |_ui| {
                opened = true;
            });
            opened
        }).unwrap_or(false)
    }).build()?;

    m.function("menu_end", || {}).build()?;

    m.function("menu_item", |label: Ref<str>| -> bool {
        with_ui(|ui| ui.button(label.as_ref()).clicked()).unwrap_or(false)
    }).build()?;

    m.function("menu_separator", || {
        with_ui(|ui| { ui.separator(); });
    }).build()?;

    // ── Toolbar bindings (for toolbar.rn) ─────────────────────────────────────

    m.function("toolbar_begin", || {}).build()?;
    m.function("toolbar_end",   || {}).build()?;

    m.function("tool_button", |label: Ref<str>, active: bool| -> bool {
        with_ui(|ui| {
            let color = if active {
                egui::Color32::from_rgb(220, 180, 60)
            } else {
                egui::Color32::from_rgb(180, 180, 190)
            };
            let text = egui::RichText::new(label.as_ref())
                .font(egui::FontId::proportional(12.0))
                .color(color);
            ui.selectable_label(active, text).clicked()
        }).unwrap_or(false)
    }).build()?;

    m.function("toolbar_separator", || {
        with_ui(|ui| { ui.separator(); });
    }).build()?;

    // ── Raw mouse delta (works when cursor is locked) ───────────────────────

    // Returns [dx, dy] raw mouse motion this frame — valid even when cursor is
    // locked (DeviceEvent::MouseMotion). Use this for RMB fly-look.
    m.function("viewport_raw_mouse_delta", || -> Vec<f64> {
        let (dx, dy) = RAW_MOUSE_DELTA.with(|c| c.get());
        vec![dx, dy]
    }).build()?;

    // ── Cursor control (Unity-style) ─────────────────────────────────────────

    // Lock/grab the cursor to the window and hide it.
    // Equivalent to Unity's Cursor.lockState = CursorLockMode.Locked.
    m.function("cursor_lock", || {
        CURSOR_GRAB.with(|c| c.set(Some(true)));
        CURSOR_VISIBLE.with(|c| c.set(Some(false)));
    }).build()?;

    // Release the cursor and show it.
    // Equivalent to Unity's Cursor.lockState = CursorLockMode.None.
    m.function("cursor_unlock", || {
        CURSOR_GRAB.with(|c| c.set(Some(false)));
        CURSOR_VISIBLE.with(|c| c.set(Some(true)));
    }).build()?;

    // Show the cursor without releasing grab.
    m.function("cursor_visible", |v: bool| {
        CURSOR_VISIBLE.with(|c| c.set(Some(v)));
    }).build()?;

    // Explicitly set grab state.
    m.function("cursor_grab", |v: bool| {
        CURSOR_GRAB.with(|c| c.set(Some(v)));
    }).build()?;

    m.function("badge_right", |text: Ref<str>, r: f64, g: f64, b: f64| {
        with_ui(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(text.as_ref())
                        .font(egui::FontId::monospace(10.0))
                        .color(egui::Color32::from_rgb(
                            (r * 255.0) as u8,
                            (g * 255.0) as u8,
                            (b * 255.0) as u8,
                        )),
                );
            });
        });
    }).build()?;

    Ok(m)
}
