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

/// Entry in an open menu popup — either a clickable item or a separator.
#[derive(Clone)]
enum MenuEntry {
    Item(String),
    Separator,
}

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
    /// Map from menu-label → clicked item label.
    /// Set by menu_end, consumed by menu_item on the next frame.
    static MENU_CLICKED: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
    /// Accumulated items for the current menu (filled by menu_item/menu_separator).
    static MENU_ITEMS: RefCell<Vec<MenuEntry>> = RefCell::new(Vec::new());
    /// Label of the currently building menu (set by menu_begin).
    static MENU_LABEL: RefCell<String> = RefCell::new(String::new());
    /// Rect of the button rendered by menu_begin — used to anchor the popup.
    static MENU_BTN_RECT: Cell<egui::Rect> = Cell::new(egui::Rect::NOTHING);
    /// True if the popup was toggled open THIS frame (suppresses immediate close).
    static MENU_JUST_OPENED: Cell<bool> = Cell::new(false);
    // ── Floating input dialog ────────────────────────────────────────────────────────────
    /// True when the dialog window is currently shown.
    static DIALOG_OPEN:   Cell<bool>      = Cell::new(false);
    /// Title shown in the dialog window title bar.
    static DIALOG_TITLE:  RefCell<String> = RefCell::new(String::new());
    /// Live text-edit buffer (updated while user types).
    static DIALOG_INPUT:  RefCell<String> = RefCell::new(String::new());
    /// The text the user confirmed; populated on OK / Enter.
    static DIALOG_RESULT: RefCell<String> = RefCell::new(String::new());
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
        with_ui(|ui| {
            let raw = label.as_ref();
            let display = raw.split("##").next().unwrap_or(raw);
            ui.button(display).clicked()
        }).unwrap_or(false)
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
            // Support "Display##unique_id" convention to avoid egui ID clashes.
            let raw = label.as_ref();
            let (display, id_str) = if let Some(pos) = raw.find("##") {
                (&raw[..pos], &raw[pos+2..])
            } else {
                (raw, raw)
            };
            let id = ui.make_persistent_id(id_str);
            let is_open = ui.memory_mut(|m| {
                m.data.get_persisted::<bool>(id).unwrap_or(true)
            });
            let clicked = ui.horizontal(|ui| {
                let sym = if is_open { "▼" } else { "▶" };
                ui.label(display);
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

    m.function("indent_push", || {
        with_ui(|ui| { ui.add_space(0.0); });
    }).build()?;
    m.function("indent_pop", || {}).build()?;

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
            // Support "Display##unique_id" convention to avoid egui ID clashes.
            let raw = label.as_ref();
            let (display, id_str) = if let Some(pos) = raw.find("##") {
                (&raw[..pos], &raw[pos+2..])
            } else {
                (raw, raw)
            };
            let mut chosen = current.as_ref().to_string();
            let resp = egui::ComboBox::from_id_salt(id_str)
                .selected_text(current.as_ref())
                .show_ui(ui, |ui| {
                    ui.label(display);
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

    // ── Menu bar helpers ─────────────────────────────────────────────────────
    // Used by menubar.rn inside egui::menu::bar().
    //
    // Design: Rune calls menu_begin("File") which renders the top-bar button
    // and, if the popup is open, collects all items into MENU_ITEMS.
    // menu_item / menu_separator push entries into MENU_ITEMS.
    // menu_end() actually renders the popup from the accumulated list and
    // returns which item (if any) was clicked.
    //
    // Because egui popups require a closure to draw into, we use a two-phase
    // approach:
    //   Phase 1 (menu_begin → menu_end):  collect item labels into a Vec
    //   Phase 2 (inside menu_end):        open popup_below_widget + draw items
    //
    // This avoids storing a live &mut Ui across Rune call boundaries.

    m.function("menu_begin", |label: Ref<str>| -> bool {
        MENU_ITEMS.with(|c| c.borrow_mut().clear());
        MENU_JUST_OPENED.with(|c| c.set(false));
        // Do NOT clear MENU_CLICKED here — it must survive to this frame's
        // menu_item calls so Rune can act on last-frame's click.

        let label_str = label.as_ref().to_string();
        MENU_LABEL.with(|c| *c.borrow_mut() = label_str.clone());

        // Render the top-bar button and toggle popup on click.
        with_ui(|ui| {
            let popup_id = egui::Id::new(&label_str).with("__mnupop__");
            let btn = ui.button(&label_str);
            MENU_BTN_RECT.with(|c| c.set(btn.rect));
            if btn.clicked() {
                ui.memory_mut(|m| m.toggle_popup(popup_id));
                // If we just opened the popup, suppress the close-outside
                // check this frame so it doesn't immediately close again.
                if ui.memory(|m| m.is_popup_open(popup_id)) {
                    MENU_JUST_OPENED.with(|c| c.set(true));
                }
            }
            // Return whether the popup is currently open so the Rune script
            // can conditionally call menu_item only while open.
            ui.memory(|m| m.is_popup_open(popup_id))
        }).unwrap_or(false)
    }).build()?;

    m.function("menu_item", |label: Ref<str>| -> bool {
        // Just queue the label; rendering happens in menu_end.
        MENU_ITEMS.with(|c| c.borrow_mut().push(MenuEntry::Item(label.as_ref().to_string())));
        // Consume the click if it matches this item's label (scoped to current menu).
        let menu = MENU_LABEL.with(|c| c.borrow().clone());
        MENU_CLICKED.with(|map| {
            let mut m = map.borrow_mut();
            if m.get(&menu).map(|s| s.as_str()) == Some(label.as_ref()) {
                m.remove(&menu);
                true
            } else {
                false
            }
        })
    }).build()?;

    m.function("menu_separator", || {
        MENU_ITEMS.with(|c| c.borrow_mut().push(MenuEntry::Separator));
    }).build()?;

    m.function("menu_end", || {
        let label_str = MENU_LABEL.with(|c| c.borrow().clone());
        let items: Vec<MenuEntry> = MENU_ITEMS.with(|c| c.borrow().clone());

        with_ui(|ui| {
            let popup_id = egui::Id::new(&label_str).with("__mnupop__");
            if ui.memory(|m| m.is_popup_open(popup_id)) {
                // Find the button rect so we can anchor the popup below it.
                // egui stores the last allocated rect; we recreate the id:
                let btn_id = ui.id().with(&label_str);
                let btn_rect = ui.ctx().memory(|m| m.area_rect(btn_id));

                // Use a fixed pos area — place below the menu bar.
                let bar_bottom = ui.min_rect().bottom();
                // Anchor left-aligned under the button using the stored rect.
                let stored_btn = MENU_BTN_RECT.with(|c| c.get());
                let x = if stored_btn != egui::Rect::NOTHING { stored_btn.left() }
                        else { btn_rect.map(|r| r.left()).unwrap_or(0.0) };
                let pos = egui::pos2(x, bar_bottom);

                let mut clicked_item: Option<String> = None;
                egui::Area::new(popup_id)
                    .order(egui::Order::Foreground)
                    .fixed_pos(pos)
                    .show(ui.ctx(), |popup_ui| {
                        egui::Frame::popup(popup_ui.style()).show(popup_ui, |inner| {
                            inner.set_min_width(160.0);
                            for entry in &items {
                                match entry {
                                    MenuEntry::Item(lbl) => {
                                        if inner.button(lbl).clicked() {
                                            clicked_item = Some(lbl.clone());
                                        }
                                    }
                                    MenuEntry::Separator => { inner.separator(); }
                                }
                            }
                        });
                    });

                if let Some(ref item) = clicked_item {
                    MENU_CLICKED.with(|map| {
                        map.borrow_mut().insert(label_str.clone(), item.clone());
                    });
                    ui.memory_mut(|m| m.close_popup());
                }

                // Close when clicking outside — but not on the same frame
                // the popup was opened (that click IS the button click).
                let just_opened = MENU_JUST_OPENED.with(|c| c.get());
                if !just_opened {
                    let pointer_pos = ui.input(|i| i.pointer.interact_pos());
                    let popup_rect  = ui.ctx().memory(|m| m.area_rect(popup_id));
                    let btn_rect    = MENU_BTN_RECT.with(|c| c.get());
                    if ui.input(|i| i.pointer.any_click()) {
                        let outside = match (pointer_pos, popup_rect) {
                            (Some(p), Some(r)) => {
                                !r.contains(p) && !btn_rect.contains(p)
                            }
                            _ => true,
                        };
                        if outside {
                            ui.memory_mut(|m| m.close_popup());
                        }
                    }
                }
            }
        });

        MENU_ITEMS.with(|c| c.borrow_mut().clear());
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

    // ── Floating input dialog API ─────────────────────────────────────────────

    // input_dialog_open(title, initial_text) — open a centered input dialog.
    // Follow with input_dialog_update() every frame to render it.
    m.function("input_dialog_open", |title: String, initial: String| {
        DIALOG_TITLE.with(|t| *t.borrow_mut() = title);
        DIALOG_INPUT.with(|i| *i.borrow_mut() = initial);
        DIALOG_RESULT.with(|r| r.borrow_mut().clear());
        DIALOG_OPEN.with(|o| o.set(true));
    }).build()?;

    // input_dialog_update() — render the dialog if open (call every frame).
    // Returns: ""          when not open (idle)
    //          "pending"   while the user is typing
    //          "confirmed" after OK or Enter  → text available via input_dialog_result()
    //          "cancelled" after Cancel or Escape
    m.function("input_dialog_update", || -> String {
        if !DIALOG_OPEN.with(|o| o.get()) {
            return String::new();
        }
        with_ui(|ui| {
            use std::cell::RefCell as LC;
            let title  = DIALOG_TITLE.with(|t| t.borrow().clone());
            let text:   LC<String> = LC::new(DIALOG_INPUT.with(|i| i.borrow().clone()));
            let action: LC<String> = LC::new("pending".to_string());

            egui::Window::new(title.as_str())
                .collapsible(false)
                .resizable(false)
                .min_width(260.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    let mut v  = text.borrow().clone();
                    let resp   = ui.text_edit_singleline(&mut v);
                    *text.borrow_mut() = v.clone();

                    let enter  = resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    let ok_on  = !v.is_empty();

                    ui.horizontal(|ui| {
                        if ui.add_enabled(ok_on, egui::Button::new("OK")).clicked()
                            || (enter && ok_on)
                        {
                            DIALOG_RESULT.with(|r| *r.borrow_mut() = v.clone());
                            *action.borrow_mut() = "confirmed".to_string();
                            DIALOG_OPEN.with(|o| o.set(false));
                        }
                        if ui.button("Cancel").clicked() || escape {
                            *action.borrow_mut() = "cancelled".to_string();
                            DIALOG_OPEN.with(|o| o.set(false));
                        }
                    });
                });

            DIALOG_INPUT.with(|i| *i.borrow_mut() = text.into_inner());
            action.into_inner()
        }).unwrap_or_else(|| "pending".to_string())
    }).build()?;

    // input_dialog_result() — confirmed text; valid after update() → "confirmed".
    m.function("input_dialog_result", || -> String {
        DIALOG_RESULT.with(|r| r.borrow().clone())
    }).build()?;

    // ── Directory header with context menu ────────────────────────────────────

    // dir_header_with_menu(label, menu_items)
    // Renders a folder icon + toggle + label row with right-click context menu.
    // Supports "Display##unique_id" convention to avoid egui ID clashes.
    // Returns Vec<String>: ["open" | "closed",  menu_action_or_empty_string].
    m.function("dir_header_with_menu", |label: Ref<str>, items: Vec<String>| -> Vec<String> {
        with_ui(|ui| {
            use std::cell::RefCell as LC;
            let raw = label.as_ref();
            let (display, id_str) = if let Some(pos) = raw.find("##") {
                (&raw[..pos], &raw[pos+2..])
            } else {
                (raw, raw)
            };
            let id      = ui.make_persistent_id(id_str);
            let is_open = ui.memory_mut(|m| m.data.get_persisted::<bool>(id).unwrap_or(true));
            let tint    = ui.visuals().text_color();
            let action:  LC<String> = LC::new(String::new());
            let toggled: LC<bool>   = LC::new(false);

            let row = ui.horizontal(|ui| {
                let sym = if is_open { "\u{25BC}" } else { "\u{25B6}" };
                if ui.small_button(sym).clicked() {
                    *toggled.borrow_mut() = true;
                }
                if let Some(bytes) = crate::icons::icon_bytes("folder") {
                    let uri = crate::icons::icon_uri("folder");
                    ui.add(
                        egui::Image::from_bytes(uri, bytes)
                            .fit_to_exact_size(egui::vec2(14.0, 14.0))
                            .tint(tint),
                    );
                }
                ui.label(display);
            });

            if *toggled.borrow() {
                ui.memory_mut(|m| m.data.insert_persisted(id, !is_open));
            }

            row.response.context_menu(|ui| {
                for item in &items {
                    if ui.button(item.as_str()).clicked() {
                        *action.borrow_mut() = item.clone();
                        ui.close_menu();
                    }
                }
            });

            let open_str = if is_open { "open" } else { "closed" };
            vec![open_str.to_string(), action.into_inner()]
        }).unwrap_or_else(|| vec!["closed".to_string(), String::new()])
    }).build()?;

    // ── Icon + selectable row with right-click menu ───────────────────────────

    // icon_selectable
    m.function("icon_selectable", |icon: Ref<str>, label: Ref<str>, is_sel: bool, items: Vec<String>| -> String {
        with_ui(|ui| {
            use std::cell::RefCell as LC;
            let action: LC<String> = LC::new(String::new());
            ui.horizontal(|ui| {
                let sz   = 16.0f32;
                let tint = ui.visuals().text_color();
                if let Some(bytes) = crate::icons::icon_bytes(icon.as_ref()) {
                    let uri = crate::icons::icon_uri(icon.as_ref());
                    ui.add(
                        egui::Image::from_bytes(uri, bytes)
                            .fit_to_exact_size(egui::vec2(sz, sz))
                            .tint(tint),
                    );
                }
                let resp = ui.selectable_label(is_sel, label.as_ref());
                LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(resp.clone()));
                if resp.clicked() && action.borrow().is_empty() {
                    *action.borrow_mut() = "select".to_string();
                }
                resp.context_menu(|ui| {
                    for item in &items {
                        if ui.button(item.as_str()).clicked() {
                            *action.borrow_mut() = item.clone();
                            ui.close_menu();
                        }
                    }
                });
            });
            action.into_inner()
        }).unwrap_or_default()
    }).build()?;

    // ── SVG icon widgets ──────────────────────────────────────────────────────

    // icon(name, size) — render a Feather SVG icon inline (no interaction).
    // name: icon filename without path/extension, e.g. "image", "box", "music".
    // size: pixel size (square).
    m.function("icon", |name: Ref<str>, size: f64| {
        with_ui(|ui| {
            let sz   = size as f32;
            let tint = ui.visuals().text_color();
            if let Some(bytes) = crate::icons::icon_bytes(name.as_ref()) {
                let uri = crate::icons::icon_uri(name.as_ref());
                ui.add(
                    egui::Image::from_bytes(uri, bytes)
                        .fit_to_exact_size(egui::vec2(sz, sz))
                        .tint(tint),
                );
            }
        });
    }).build()?;

    // icon_button(name, tooltip, size) — clickable icon, returns true when clicked.
    m.function("icon_button", |name: Ref<str>, tooltip: Ref<str>, size: f64| -> bool {
        with_ui(|ui| {
            let sz   = size as f32;
            let tint = ui.visuals().text_color();
            if let Some(bytes) = crate::icons::icon_bytes(name.as_ref()) {
                let uri  = crate::icons::icon_uri(name.as_ref());
                let img  = egui::Image::from_bytes(uri, bytes)
                    .fit_to_exact_size(egui::vec2(sz, sz))
                    .tint(tint);
                let resp = ui.add(egui::ImageButton::new(img));
                let resp = if tooltip.as_ref().is_empty() { resp } else { resp.on_hover_text(tooltip.as_ref()) };
                return resp.clicked();
            }
            false
        }).unwrap_or(false)
    }).build()?;

    // asset_row_button(icon, label, meta, size) — composite row: icon + label + dim meta text.
    // Returns true when the row is clicked.
    m.function("asset_row_button", |icon: Ref<str>, label: Ref<str>, meta: Ref<str>, size: f64| -> bool {
        with_ui(|ui| {
            let sz    = size as f32;
            let tint  = ui.visuals().text_color();
            let weak  = ui.visuals().weak_text_color();
            let mut clicked = false;
            ui.horizontal(|ui| {
                // Icon column
                if let Some(bytes) = crate::icons::icon_bytes(icon.as_ref()) {
                    let uri = crate::icons::icon_uri(icon.as_ref());
                    ui.add(
                        egui::Image::from_bytes(uri, bytes)
                            .fit_to_exact_size(egui::vec2(sz, sz))
                            .tint(tint),
                    );
                }
                // Label + secondary text
                ui.vertical(|ui| {
                    if ui.button(label.as_ref()).clicked() {
                        clicked = true;
                    }
                    if !meta.as_ref().is_empty() {
                        ui.add(egui::Label::new(
                            egui::RichText::new(meta.as_ref()).small().color(weak),
                        ));
                    }
                });
            });
            clicked
        }).unwrap_or(false)
    }).build()?;

    Ok(m)
}
