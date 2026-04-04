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

    Ok(m)
}
