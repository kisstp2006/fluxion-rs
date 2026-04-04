// ============================================================
// ui_module.rs — fluxion::ui Rune module
//
// Wraps a subset of egui widgets as native Rune functions.
// Access to the current `egui::Ui` is provided through a
// thread-local raw pointer that is set/cleared by dock.rs
// around each Rune panel call (always synchronous — no Send needed).
// ============================================================

use std::cell::Cell;
use std::ptr::NonNull;

use rune::Module;

thread_local! {
    static CURRENT_UI: Cell<Option<NonNull<egui::Ui>>> = Cell::new(None);
}

/// Set the active UI context before calling a Rune panel function.
///
/// # Safety
/// The caller guarantees that `ui` outlives the Rune call that follows.
pub fn set_current_ui(ui: &mut egui::Ui) {
    CURRENT_UI.with(|c| c.set(Some(NonNull::from(ui))));
}

/// Clear the active UI context after the Rune panel function returns.
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

    m.function("space", |pixels: f64| {
        with_ui(|ui| { ui.add_space(pixels as f32); });
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
            let tid = egui::TextureId::Managed(texture_id as u64);
            let size = if w > 0.0 && h > 0.0 {
                egui::Vec2::new(w as f32, h as f32)
            } else {
                ui.available_size()
            };
            ui.add(egui::Image::new(egui::load::SizedTexture::new(tid, size)));
        });
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

    // ── Size query ────────────────────────────────────────────────────────────
    m.function("available_width", || -> f64 {
        with_ui(|ui| ui.available_width() as f64).unwrap_or(0.0)
    }).build()?;

    m.function("available_height", || -> f64 {
        with_ui(|ui| ui.available_height() as f64).unwrap_or(0.0)
    }).build()?;

    Ok(m)
}
