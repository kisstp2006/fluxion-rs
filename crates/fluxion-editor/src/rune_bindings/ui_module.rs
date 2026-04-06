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

/// Unified drag-and-drop payload shared across all editor panels.
/// Stored in egui's per-frame DragAndDrop context (egui 0.29).
#[derive(Clone, Debug)]
pub(crate) enum DndPayload {
    Asset  { path: String, asset_type: String },
    #[allow(dead_code)]
    Entity { id: i64 },
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
    /// Map from menu-label → clicked item label (set this frame, read next frame).
    static MENU_CLICKED: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
    /// Accumulated items for the current menu (filled by menu_item/menu_separator).
    static MENU_ITEMS: RefCell<Vec<MenuEntry>> = RefCell::new(Vec::new());
    /// Label of the currently building menu (set by menu_begin).
    static MENU_LABEL: RefCell<String> = RefCell::new(String::new());
    /// Items from the PREVIOUS frame per menu label — rendered inside ui.menu_button.
    static MENU_ITEMS_PREV: RefCell<std::collections::HashMap<String, Vec<MenuEntry>>> = RefCell::new(std::collections::HashMap::new());
    /// Keyboard-highlighted row index in the autocomplete dropdown (-1 = none).
    static AUTOCOMPLETE_IDX: Cell<i64> = Cell::new(-1);
    /// True if the pointer was inside the autocomplete popup area last frame.
    /// Used to keep the popup alive through the click (mouse-up) frame.
    static POPUP_HOVERED: Cell<bool> = Cell::new(false);
    // ── Settings modals ──────────────────────────────────────────────────────────────────
    /// ID of the currently open modal window, or None.
    static MODAL_OPEN:  RefCell<Option<String>> = RefCell::new(None);
    /// Set to true by modal_close() to signal the modal should close this frame.
    static MODAL_CLOSE: Cell<bool>              = Cell::new(false);
    /// Cached egui Context for the current frame — set once per frame from main.rs.
    /// Lets settings window bindings work without needing a live CURRENT_UI pointer.
    static CURRENT_CTX: RefCell<Option<egui::Context>> = RefCell::new(None);

    // ── Texture preview cache ───────────────────────────────────────────────────────────
    /// Maps asset-path → loaded egui TextureHandle so we don't re-upload every frame.
    static TEXTURE_CACHE: RefCell<std::collections::HashMap<String, egui::TextureHandle>>
        = RefCell::new(std::collections::HashMap::new());

    // ── ltreeview context menu result ───────────────────────────────────────────────────
    /// Written by context_menu closures inside ltreeview_hierarchy; read after show().
    /// Format: (node_id, action_label). -1 means no action this frame.
    static LTREE_CTX_ACTION: RefCell<(i64, String)> = RefCell::new((-1, String::new()));
    /// Reverse map for ltreeview_assets: path_hash(i64) → path string.
    static LTREE_ASSET_PATHS: RefCell<std::collections::HashMap<i64, String>>
        = RefCell::new(std::collections::HashMap::new());

    // ── Horizontal layout state ──────────────────────────────────────────────────────────
    /// Saved parent UI pointer while inside a horizontal_begin/end block.
    static HORIZ_PARENT: Cell<Option<NonNull<egui::Ui>>> = Cell::new(None);
    /// Owned child UI for horizontal layout.
    static HORIZ_CHILD:  RefCell<Option<Box<egui::Ui>>> = RefCell::new(None);

    // ── Two-column layout state ─────────────────────────────────────────────────────────
    /// Saved parent UI pointer while inside a columns layout.
    static COLS_PARENT: Cell<Option<NonNull<egui::Ui>>> = Cell::new(None);
    /// Owned left child UI (heap-allocated so pointer is stable).
    static LEFT_CHILD:  RefCell<Option<Box<egui::Ui>>> = RefCell::new(None);
    /// Owned right child UI.
    static RIGHT_CHILD: RefCell<Option<Box<egui::Ui>>> = RefCell::new(None);

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

#[allow(dead_code)]
pub fn clear_current_ui() {
    CURRENT_UI.with(|c| c.set(None));
}

fn longest_common_prefix(strs: &[String]) -> String {
    if strs.is_empty() { return String::new(); }
    let first = strs[0].as_str();
    let mut len = first.len();
    for s in &strs[1..] {
        len = first.chars().zip(s.chars()).take_while(|(a,b)| a == b).count().min(len);
    }
    first[..len].to_string()
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

/// Cache the egui Context for this frame.  Must be called once per frame from
/// main.rs before any Rune script that may invoke settings window bindings.
pub fn set_egui_ctx(ctx: &egui::Context) {
    CURRENT_CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
}

// ══════════════════════════════════════════════════════════════════════════════
// V3 Settings UI — free rendering helpers (used by project_settings_window /
// editor_prefs_window Rune bindings)
// ══════════════════════════════════════════════════════════════════════════════

#[inline] fn sc_label()    -> egui::Color32 { egui::Color32::from_rgb(157,157,157) }
#[inline] fn sc_accent()   -> egui::Color32 { egui::Color32::from_rgb(77,158,255) }
#[inline] fn sc_accent_d() -> egui::Color32 { egui::Color32::from_rgba_unmultiplied(77,158,255,38) }
#[inline] fn sc_yellow()   -> egui::Color32 { egui::Color32::from_rgb(204,167,0) }
#[inline] fn sc_red()      -> egui::Color32 { egui::Color32::from_rgb(241,76,76) }
#[inline] fn sc_sidebar()  -> egui::Color32 { egui::Color32::from_rgb(37,37,38) }
#[inline] fn sc_text()     -> egui::Color32 { egui::Color32::from_rgb(204,204,204) }

const S_ROW_H:   f32 = 22.0;
const S_LABEL_W: f32 = 140.0;
const S_RESET_W: f32 = 22.0;

// ── Sidebar ────────────────────────────────────────────────────────────────────

fn v3_sidebar(ui: &mut egui::Ui, cats: &[&str], counts: &[usize], active: &str) -> String {
    let mut result = active.to_string();
    for (i, cat) in cats.iter().enumerate() {
        let is_active = *cat == active;
        let cnt = counts.get(i).copied().unwrap_or(0);
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 26.0), egui::Sense::click()
        );
        let bg = if is_active { sc_accent_d() }
                 else if resp.hovered() { egui::Color32::from_rgb(45,45,45) }
                 else { egui::Color32::TRANSPARENT };
        ui.painter().rect_filled(rect, 0.0, bg);
        if is_active {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
                0.0, sc_accent(),
            );
        }
        let tc = if is_active { sc_text() } else { sc_label() };
        ui.painter().text(
            egui::pos2(rect.min.x + 10.0, rect.center().y),
            egui::Align2::LEFT_CENTER, *cat,
            egui::FontId::proportional(12.0), tc,
        );
        if cnt > 0 {
            ui.painter().text(
                egui::pos2(rect.max.x - 6.0, rect.center().y),
                egui::Align2::RIGHT_CENTER, cnt.to_string(),
                egui::FontId::proportional(10.0), sc_yellow(),
            );
        }
        if resp.clicked() { result = cat.to_string(); }
    }
    result
}

// ── Section header ─────────────────────────────────────────────────────────────

fn v3_section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).color(sc_text()).size(11.5).strong());
    ui.separator();
}

// ── Property rows ──────────────────────────────────────────────────────────────

fn v3_f32(ui: &mut egui::Ui, label: &str, desc: &str,
          val: f32, def: f32, speed: f64, min: f64, max: f64, dec: usize) -> Option<f32>
{
    let is_mod = (val - def).abs() > 1e-5;
    let mut res: Option<f32> = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(S_LABEL_W, S_ROW_H), egui::Layout::left_to_right(egui::Align::Center),
            |ui| { ui.add(egui::Label::new(egui::RichText::new(label).color(sc_label()).size(11.0)).truncate()).on_hover_text(desc); }
        );
        let rw = if is_mod { S_RESET_W } else { 0.0 };
        let mut v = val;
        if ui.add_sized(
            [(ui.available_width() - rw).max(20.0), S_ROW_H - 2.0],
            egui::DragValue::new(&mut v).speed(speed).range(min..=max).max_decimals(dec)
        ).changed() { res = Some(v); }
        if is_mod && ui.add(crate::icons::img("rotate-ccw", S_RESET_W - 4.0, sc_yellow())
            .sense(egui::Sense::click())).on_hover_text("Reset to default").clicked() { res = Some(def); }
    });
    res
}

fn v3_bool(ui: &mut egui::Ui, label: &str, desc: &str, val: bool, def: bool) -> Option<bool> {
    let is_mod = val != def;
    let mut res: Option<bool> = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(S_LABEL_W, S_ROW_H), egui::Layout::left_to_right(egui::Align::Center),
            |ui| { ui.add(egui::Label::new(egui::RichText::new(label).color(sc_label()).size(11.0)).truncate()).on_hover_text(desc); }
        );
        let rw = if is_mod { S_RESET_W } else { 0.0 };
        let avail = (ui.available_width() - rw).max(20.0);
        let mut v = val;
        ui.allocate_ui_with_layout(
            egui::vec2(avail, S_ROW_H), egui::Layout::left_to_right(egui::Align::Center),
            |ui| { if ui.checkbox(&mut v, "").changed() { res = Some(v); } }
        );
        if is_mod && ui.add(crate::icons::img("rotate-ccw", S_RESET_W - 4.0, sc_yellow())
            .sense(egui::Sense::click())).on_hover_text("Reset to default").clicked() { res = Some(def); }
    });
    res
}

fn v3_slider(ui: &mut egui::Ui, label: &str, desc: &str,
             val: f32, def: f32, min: f64, max: f64, dec: usize) -> Option<f32>
{
    let is_mod = (val - def).abs() > 1e-5;
    let mut res: Option<f32> = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(S_LABEL_W, S_ROW_H), egui::Layout::left_to_right(egui::Align::Center),
            |ui| { ui.add(egui::Label::new(egui::RichText::new(label).color(sc_label()).size(11.0)).truncate()).on_hover_text(desc); }
        );
        let rw = if is_mod { S_RESET_W } else { 0.0 };
        let mut v = val;
        if ui.add_sized(
            [(ui.available_width() - rw).max(20.0), S_ROW_H - 2.0],
            egui::Slider::new(&mut v, min as f32..=max as f32).max_decimals(dec)
        ).changed() { res = Some(v); }
        if is_mod && ui.add(crate::icons::img("rotate-ccw", S_RESET_W - 4.0, sc_yellow())
            .sense(egui::Sense::click())).on_hover_text("Reset to default").clicked() { res = Some(def); }
    });
    res
}

fn v3_u32(ui: &mut egui::Ui, label: &str, desc: &str,
          val: u32, def: u32, min: u32, max: u32) -> Option<u32>
{
    let is_mod = val != def;
    let mut res: Option<u32> = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(S_LABEL_W, S_ROW_H), egui::Layout::left_to_right(egui::Align::Center),
            |ui| { ui.add(egui::Label::new(egui::RichText::new(label).color(sc_label()).size(11.0)).truncate()).on_hover_text(desc); }
        );
        let rw = if is_mod { S_RESET_W } else { 0.0 };
        let mut v = val;
        if ui.add_sized(
            [(ui.available_width() - rw).max(20.0), S_ROW_H - 2.0],
            egui::DragValue::new(&mut v).range(min..=max)
        ).changed() { res = Some(v); }
        if is_mod && ui.add(crate::icons::img("rotate-ccw", S_RESET_W - 4.0, sc_yellow())
            .sense(egui::Sense::click())).on_hover_text("Reset to default").clicked() { res = Some(def); }
    });
    res
}

fn v3_string(ui: &mut egui::Ui, label: &str, desc: &str, val: &str, def: &str) -> Option<String> {
    let is_mod = val != def;
    let mut res: Option<String> = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(S_LABEL_W, S_ROW_H), egui::Layout::left_to_right(egui::Align::Center),
            |ui| { ui.add(egui::Label::new(egui::RichText::new(label).color(sc_label()).size(11.0)).truncate()).on_hover_text(desc); }
        );
        let rw = if is_mod { S_RESET_W } else { 0.0 };
        let mut s = val.to_string();
        if ui.add_sized(
            [(ui.available_width() - rw).max(20.0), S_ROW_H - 2.0],
            egui::TextEdit::singleline(&mut s)
        ).changed() { res = Some(s); }
        if is_mod && ui.add(crate::icons::img("rotate-ccw", S_RESET_W - 4.0, sc_yellow())
            .sense(egui::Sense::click())).on_hover_text("Reset to default").clicked() { res = Some(def.to_string()); }
    });
    res
}

fn v3_select(ui: &mut egui::Ui, label: &str, desc: &str,
             val: &str, def: &str, opts: &[&str]) -> Option<String>
{
    let is_mod = val != def;
    let mut res: Option<String> = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(S_LABEL_W, S_ROW_H), egui::Layout::left_to_right(egui::Align::Center),
            |ui| { ui.add(egui::Label::new(egui::RichText::new(label).color(sc_label()).size(11.0)).truncate()).on_hover_text(desc); }
        );
        let rw = if is_mod { S_RESET_W } else { 0.0 };
        let avail = (ui.available_width() - rw).max(20.0);
        egui::ComboBox::from_id_salt(label)
            .selected_text(val)
            .width(avail)
            .show_ui(ui, |ui| {
                for opt in opts {
                    if ui.selectable_label(val == *opt, *opt).clicked() {
                        res = Some(opt.to_string());
                    }
                }
            });
        if is_mod && ui.add(crate::icons::img("rotate-ccw", S_RESET_W - 4.0, sc_yellow())
            .sense(egui::Sense::click())).on_hover_text("Reset to default").clicked() { res = Some(def.to_string()); }
    });
    res
}

fn v3_vec3(ui: &mut egui::Ui, label: &str, desc: &str,
           val: [f32;3], def: [f32;3], speed: f64) -> Option<[f32;3]>
{
    let is_mod = val != def;
    let mut res: Option<[f32;3]> = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(S_LABEL_W, S_ROW_H), egui::Layout::left_to_right(egui::Align::Center),
            |ui| { ui.add(egui::Label::new(egui::RichText::new(label).color(sc_label()).size(11.0)).truncate()).on_hover_text(desc); }
        );
        let rw = if is_mod { S_RESET_W } else { 0.0 };
        let w3 = ((ui.available_width() - rw) / 3.0).max(20.0);
        let mut v = val;
        let mut changed = false;
        for c in &mut v {
            if ui.add_sized([w3, S_ROW_H - 2.0], egui::DragValue::new(c).speed(speed).max_decimals(3)).changed() {
                changed = true;
            }
        }
        if changed { res = Some(v); }
        if is_mod && ui.add(crate::icons::img("rotate-ccw", S_RESET_W - 4.0, sc_yellow())
            .sense(egui::Sense::click())).on_hover_text("Reset to default").clicked() { res = Some(def); }
    });
    res
}

// ── Project Settings tab content ───────────────────────────────────────────────

fn render_project_content_v3(ui: &mut egui::Ui, tab: &str) {
    use crate::rune_bindings::settings_module as sm;
    match tab {
        "Physics" => {
            v3_section(ui, "Physics");
            let g  = sm::with_project_config(|c| c.settings.physics.gravity).unwrap_or([0.0,-9.81,0.0]);
            let gd = sm::with_project_defaults(|c| c.settings.physics.gravity).unwrap_or([0.0,-9.81,0.0]);
            if let Some(v) = v3_vec3(ui, "Gravity", "World gravity vector", g, gd, 0.01) {
                sm::modify_project_config(|c| c.settings.physics.gravity = v);
            }
            let ts  = sm::with_project_config(|c| c.settings.physics.fixed_timestep).unwrap_or(1.0/60.0);
            let tsd = sm::with_project_defaults(|c| c.settings.physics.fixed_timestep).unwrap_or(1.0/60.0);
            if let Some(v) = v3_f32(ui, "Fixed Timestep", "Physics fixed update interval (s)", ts, tsd, 0.001, 0.001, 1.0, 4) {
                sm::modify_project_config(|c| c.settings.physics.fixed_timestep = v.clamp(0.001, 1.0));
            }
        }
        "Rendering" => {
            v3_section(ui, "Rendering");
            let sh  = sm::with_project_config(|c| c.settings.render.shadows).unwrap_or(true);
            let shd = sm::with_project_defaults(|c| c.settings.render.shadows).unwrap_or(true);
            if let Some(v) = v3_bool(ui, "Shadows", "Enable shadow rendering", sh, shd) {
                sm::modify_project_config(|c| c.settings.render.shadows = v);
            }
            let sms  = sm::with_project_config(|c| c.settings.render.shadow_map_size).unwrap_or(2048);
            let smsd = sm::with_project_defaults(|c| c.settings.render.shadow_map_size).unwrap_or(2048);
            let sms_str  = sms.to_string();
            let smsd_str = smsd.to_string();
            if let Some(v) = v3_select(ui, "Shadow Map Size", "Shadow map resolution (px)", &sms_str, &smsd_str, &["256","512","1024","2048","4096","8192"]) {
                if let Ok(n) = v.parse::<u32>() { sm::modify_project_config(|c| c.settings.render.shadow_map_size = n); }
            }
            let tm  = sm::with_project_config(|c| c.settings.render.tone_mapping.clone()).unwrap_or_else(|| "ACES".to_string());
            let tmd = sm::with_project_defaults(|c| c.settings.render.tone_mapping.clone()).unwrap_or_else(|| "ACES".to_string());
            if let Some(v) = v3_select(ui, "Tone Mapping", "HDR tone mapping operator", &tm, &tmd, &["ACES","Filmic","Linear","Reinhard"]) {
                sm::modify_project_config(|c| c.settings.render.tone_mapping = v);
            }
            let exp  = sm::with_project_config(|c| c.settings.render.exposure).unwrap_or(1.2);
            let expd = sm::with_project_defaults(|c| c.settings.render.exposure).unwrap_or(1.2);
            if let Some(v) = v3_f32(ui, "Exposure", "Camera exposure multiplier", exp, expd, 0.01, 0.0, 10.0, 2) {
                sm::modify_project_config(|c| c.settings.render.exposure = v.clamp(0.0, 10.0));
            }
            v3_section(ui, "Grid & Snap");
            let sg  = sm::with_project_config(|c| c.settings.editor.show_grid).unwrap_or(true);
            let sgd = sm::with_project_defaults(|c| c.settings.editor.show_grid).unwrap_or(true);
            if let Some(v) = v3_bool(ui, "Show Grid", "Display world grid in viewport", sg, sgd) {
                sm::modify_project_config(|c| c.settings.editor.show_grid = v);
            }
            let st   = sm::with_project_config(|c| c.settings.editor.snap_translation).unwrap_or(1.0);
            let stdf = sm::with_project_defaults(|c| c.settings.editor.snap_translation).unwrap_or(1.0);
            if let Some(v) = v3_f32(ui, "Snap Translate", "Translation snap step (m)", st, stdf, 0.01, 0.001, 100.0, 3) {
                sm::modify_project_config(|c| c.settings.editor.snap_translation = v.clamp(0.001,100.0));
                crate::rune_bindings::world_module::set_snap_translate_value(v as f64);
            }
            let sr   = sm::with_project_config(|c| c.settings.editor.snap_rotation).unwrap_or(15.0);
            let srdf = sm::with_project_defaults(|c| c.settings.editor.snap_rotation).unwrap_or(15.0);
            if let Some(v) = v3_f32(ui, "Snap Rotate °", "Rotation snap step (degrees)", sr, srdf, 0.1, 0.1, 180.0, 1) {
                sm::modify_project_config(|c| c.settings.editor.snap_rotation = v.clamp(0.1,180.0));
                crate::rune_bindings::world_module::set_snap_rotate_value(v as f64);
            }
            let ss   = sm::with_project_config(|c| c.settings.editor.snap_scale).unwrap_or(0.25);
            let ssdf = sm::with_project_defaults(|c| c.settings.editor.snap_scale).unwrap_or(0.25);
            if let Some(v) = v3_f32(ui, "Snap Scale", "Scale snap step", ss, ssdf, 0.01, 0.001, 10.0, 3) {
                sm::modify_project_config(|c| c.settings.editor.snap_scale = v.clamp(0.001,10.0));
                crate::rune_bindings::world_module::set_snap_scale_value(v as f64);
            }
        }
        "Audio" => {
            v3_section(ui, "Audio");
            let mv  = sm::with_project_config(|c| c.settings.audio.master_volume).unwrap_or(1.0);
            let mvd = sm::with_project_defaults(|c| c.settings.audio.master_volume).unwrap_or(1.0);
            if let Some(v) = v3_slider(ui, "Master Volume", "Overall audio volume", mv, mvd, 0.0, 1.0, 2) {
                sm::modify_project_config(|c| c.settings.audio.master_volume = v.clamp(0.0,1.0));
            }
            let mu  = sm::with_project_config(|c| c.settings.audio.music_volume).unwrap_or(1.0);
            let mud = sm::with_project_defaults(|c| c.settings.audio.music_volume).unwrap_or(1.0);
            if let Some(v) = v3_slider(ui, "Music Volume", "Background music volume", mu, mud, 0.0, 1.0, 2) {
                sm::modify_project_config(|c| c.settings.audio.music_volume = v.clamp(0.0,1.0));
            }
            let sx  = sm::with_project_config(|c| c.settings.audio.sfx_volume).unwrap_or(1.0);
            let sxd = sm::with_project_defaults(|c| c.settings.audio.sfx_volume).unwrap_or(1.0);
            if let Some(v) = v3_slider(ui, "SFX Volume", "Sound effect volume", sx, sxd, 0.0, 1.0, 2) {
                sm::modify_project_config(|c| c.settings.audio.sfx_volume = v.clamp(0.0,1.0));
            }
        }
        "Input" => {
            v3_section(ui, "Input");
            let ms  = sm::with_project_config(|c| c.settings.input.mouse_sensitivity).unwrap_or(1.0);
            let msd = sm::with_project_defaults(|c| c.settings.input.mouse_sensitivity).unwrap_or(1.0);
            if let Some(v) = v3_slider(ui, "Mouse Sensitivity", "Mouse look sensitivity", ms, msd, 0.05, 10.0, 2) {
                sm::modify_project_config(|c| c.settings.input.mouse_sensitivity = v.clamp(0.05,10.0));
            }
            let dz  = sm::with_project_config(|c| c.settings.input.gamepad_deadzone).unwrap_or(0.15);
            let dzd = sm::with_project_defaults(|c| c.settings.input.gamepad_deadzone).unwrap_or(0.15);
            if let Some(v) = v3_slider(ui, "Gamepad Deadzone", "Analog stick dead zone", dz, dzd, 0.0, 0.9, 2) {
                sm::modify_project_config(|c| c.settings.input.gamepad_deadzone = v.clamp(0.0,0.9));
            }

            v3_section(ui, "Input Actions");
            let actions = sm::with_project_config(|c| c.settings.input.actions.clone()).unwrap_or_default();
            let mut remove_idx: Option<usize> = None;
            for (idx, action) in actions.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(&action.name).color(sc_text()).size(11.0).strong());
                    let kind_str = if action.analog { "analog" } else { "digital" };
                    ui.label(egui::RichText::new(kind_str).color(sc_label()).size(10.0).italics());
                    if !action.bindings.is_empty() {
                        let binding_labels: Vec<String> = action.bindings.iter().map(|b| b.label()).collect();
                        ui.label(egui::RichText::new(binding_labels.join(", ")).color(sc_label()).size(10.0));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("−").color(sc_red()).size(11.0)).small().frame(false)).clicked() {
                            remove_idx = Some(idx);
                        }
                    });
                });
            }
            if let Some(i) = remove_idx {
                sm::modify_project_config(|c| { c.settings.input.actions.remove(i); });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(egui::RichText::new("+ Digital").size(10.0)).small()).clicked() {
                    sm::modify_project_config(|c| {
                        let n = c.settings.input.actions.len() + 1;
                        c.settings.input.actions.push(fluxion_core::InputAction::new_digital(
                            format!("Action{}", n), "Space"));
                    });
                }
                if ui.add(egui::Button::new(egui::RichText::new("+ Analog").size(10.0)).small()).clicked() {
                    sm::modify_project_config(|c| {
                        let n = c.settings.input.actions.len() + 1;
                        c.settings.input.actions.push(fluxion_core::InputAction::new_analog(
                            format!("Axis{}", n)));
                    });
                }
            });
        }
        "Tags & Layers" => {
            v3_section(ui, "Tags");
            let tags = sm::with_project_config(|c| c.settings.tags.list.clone()).unwrap_or_default();
            let mut to_remove: Option<String> = None;
            for tag in &tags {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tag).color(sc_label()).size(11.0));
                    if ui.add(egui::Button::new(egui::RichText::new("−").color(sc_red()).size(11.0)).small().frame(false)).clicked() {
                        to_remove = Some(tag.clone());
                    }
                });
            }
            if let Some(t) = to_remove {
                sm::modify_project_config(|c| c.settings.tags.list.retain(|x| x != &t));
            }
            ui.add_space(4.0);

            v3_section(ui, "Collision Layers");
            ui.label(egui::RichText::new("Layers 0–15 (bit index = layer number):").color(sc_label()).size(10.0));
            let layer_names = sm::with_project_config(|c| c.settings.collision_layers.names.clone())
                .unwrap_or_default();
            for i in 0..16usize {
                let name = layer_names.get(i).cloned().unwrap_or_else(|| format!("Layer {}", i));
                let mut buf = name.clone();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{:>2}", i)).color(sc_label()).monospace().size(10.0));
                    ui.add_space(4.0);
                    let resp = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(160.0).font(egui::TextStyle::Small));
                    if resp.lost_focus() && buf != name {
                        let idx = i;
                        let new_name = buf.clone();
                        sm::modify_project_config(move |c| {
                            if c.settings.collision_layers.names.len() <= idx {
                                c.settings.collision_layers.names.resize(32, String::new());
                            }
                            c.settings.collision_layers.names[idx] = new_name.clone();
                        });
                    }
                });
            }
            ui.add_space(4.0);
        }
        "Build" => {
            v3_section(ui, "Build");
            let pl  = sm::with_project_config(|c| c.settings.build.target_platform.clone()).unwrap_or_else(|| "Windows".to_string());
            let pld = sm::with_project_defaults(|c| c.settings.build.target_platform.clone()).unwrap_or_else(|| "Windows".to_string());
            if let Some(v) = v3_select(ui, "Target Platform", "Build target OS", &pl, &pld, &["Windows","Linux","macOS","Web (WASM)"]) {
                sm::modify_project_config(|c| c.settings.build.target_platform = v);
            }
            let wt  = sm::with_project_config(|c| c.settings.build.window_title.clone()).unwrap_or_default();
            let wtd = sm::with_project_defaults(|c| c.settings.build.window_title.clone()).unwrap_or_default();
            if let Some(v) = v3_string(ui, "Window Title", "App window title", &wt, &wtd) {
                sm::modify_project_config(|c| c.settings.build.window_title = v);
            }
            let ww  = sm::with_project_config(|c| c.settings.build.window_width).unwrap_or(1920);
            let wwd = sm::with_project_defaults(|c| c.settings.build.window_width).unwrap_or(1920);
            if let Some(v) = v3_u32(ui, "Window Width", "Default window width (px)", ww, wwd, 320, 7680) {
                sm::modify_project_config(|c| c.settings.build.window_width = v);
            }
            let wh  = sm::with_project_config(|c| c.settings.build.window_height).unwrap_or(1080);
            let whd = sm::with_project_defaults(|c| c.settings.build.window_height).unwrap_or(1080);
            if let Some(v) = v3_u32(ui, "Window Height", "Default window height (px)", wh, whd, 200, 4320) {
                sm::modify_project_config(|c| c.settings.build.window_height = v);
            }
            let vs  = sm::with_project_config(|c| c.settings.build.vsync).unwrap_or(true);
            let vsd = sm::with_project_defaults(|c| c.settings.build.vsync).unwrap_or(true);
            if let Some(v) = v3_bool(ui, "VSync", "Vertical synchronization", vs, vsd) {
                sm::modify_project_config(|c| c.settings.build.vsync = v);
            }
            let fs  = sm::with_project_config(|c| c.settings.build.fullscreen).unwrap_or(false);
            let fsd = sm::with_project_defaults(|c| c.settings.build.fullscreen).unwrap_or(false);
            if let Some(v) = v3_bool(ui, "Fullscreen", "Start in fullscreen mode", fs, fsd) {
                sm::modify_project_config(|c| c.settings.build.fullscreen = v);
            }
            let errs = sm::validate_project();
            if !errs.is_empty() {
                ui.add_space(4.0);
                for e in &errs {
                    ui.horizontal(|ui| {
                        ui.add(crate::icons::img("alert-triangle", 12.0, sc_yellow()));
                        ui.label(egui::RichText::new(e.as_str()).color(sc_yellow()).size(11.0));
                    });
                }
            }
        }
        _ => {}
    }
}

// ── Editor Prefs tab content ───────────────────────────────────────────────────

fn render_prefs_content_v3(ui: &mut egui::Ui, tab: &str) {
    use crate::rune_bindings::settings_module as sm;
    match tab {
        "General" => {
            v3_section(ui, "Appearance");
            let th  = sm::with_prefs(|p| p.theme.clone()).unwrap_or_else(|| "dark".to_string());
            let thd = sm::with_prefs_defaults(|p| p.theme.clone()).unwrap_or_else(|| "dark".to_string());
            if let Some(v) = v3_select(ui, "Theme", "Editor color theme", &th, &thd, &["dark","light"]) {
                sm::modify_prefs(|p| p.theme = v);
            }
            let fs  = sm::with_prefs(|p| p.font_size).unwrap_or(13.0);
            let fsd = sm::with_prefs_defaults(|p| p.font_size).unwrap_or(13.0);
            if let Some(v) = v3_slider(ui, "Font Size", "UI font size (pt)", fs, fsd, 9.0, 24.0, 1) {
                sm::modify_prefs(|p| p.font_size = v.clamp(9.0,24.0));
            }
            v3_section(ui, "Autosave");
            let ai  = sm::with_prefs(|p| p.autosave_interval_secs).unwrap_or(120);
            let aid = sm::with_prefs_defaults(|p| p.autosave_interval_secs).unwrap_or(120);
            if let Some(v) = v3_u32(ui, "Autosave Interval", "Autosave interval in seconds (0 = off)", ai, aid, 0, 3600) {
                sm::modify_prefs(|p| p.autosave_interval_secs = v);
            }
            let rl  = sm::with_prefs(|p| p.restore_layout).unwrap_or(true);
            let rld = sm::with_prefs_defaults(|p| p.restore_layout).unwrap_or(true);
            if let Some(v) = v3_bool(ui, "Restore Layout", "Restore panel layout on startup", rl, rld) {
                sm::modify_prefs(|p| p.restore_layout = v);
            }
        }
        "Camera" => {
            v3_section(ui, "Fly Camera");
            let cs  = sm::with_prefs(|p| p.camera_speed).unwrap_or(5.0);
            let csd = sm::with_prefs_defaults(|p| p.camera_speed).unwrap_or(5.0);
            if let Some(v) = v3_f32(ui, "Camera Speed", "Editor fly camera speed (m/s)", cs, csd, 0.1, 0.1, 500.0, 1) {
                sm::modify_prefs(|p| p.camera_speed = v.clamp(0.1,500.0));
                crate::rune_bindings::world_module::set_editor_cam_speed(v as f64);
            }
            let se  = sm::with_prefs(|p| p.camera_sensitivity).unwrap_or(1.0);
            let sed = sm::with_prefs_defaults(|p| p.camera_sensitivity).unwrap_or(1.0);
            if let Some(v) = v3_f32(ui, "Mouse Sensitivity", "Editor camera mouse look sensitivity", se, sed, 0.01, 0.05, 10.0, 2) {
                sm::modify_prefs(|p| p.camera_sensitivity = v.clamp(0.05,10.0));
            }
        }
        "Console" => {
            v3_section(ui, "Console");
            let lm  = sm::with_prefs(|p| p.log_max_entries).unwrap_or(10_000);
            let lmd = sm::with_prefs_defaults(|p| p.log_max_entries).unwrap_or(10_000);
            if let Some(v) = v3_u32(ui, "Max Log Entries", "Maximum console log lines (100–100 000)", lm, lmd, 100, 100_000) {
                sm::modify_prefs(|p| p.log_max_entries = v);
            }
        }
        _ => {}
    }
}

/// Build a depth-first ordered list of directory paths with their nesting depth.
/// Used by asset_folder_tree_v2 to render a nested folder sidebar.
fn dir_depth_first(dirs: &[String], parent: &str, depth: usize, out: &mut Vec<(String, usize)>) {
    let mut children: Vec<&String> = dirs.iter()
        .filter(|d| d.rfind('/').map(|i| &d[..i]).unwrap_or("") == parent)
        .collect();
    children.sort();
    for child in children {
        out.push((child.clone(), depth));
        dir_depth_first(dirs, child.as_str(), depth + 1, out);
    }
}

// ══════════════════════════════════════════════════════════════════════════════
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
            let resp = ui.checkbox(&mut v, label.as_ref());
            LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(resp));
            v
        }).unwrap_or(value)
    }).build()?;

    m.function("drag_float", |label: Ref<str>, value: f64, speed: f64, min: f64, max: f64| -> f64 {
        with_ui(|ui| {
            let mut v = value as f32;
            let resp = ui.add(
                egui::DragValue::new(&mut v)
                    .speed(speed as f32)
                    .range(min as f32..=max as f32)
                    .prefix(format!("{}: ", label.as_ref())),
            );
            LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(resp));
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

    // slider(label, value, min, max, step) → f64
    // Unity [Range(min,max)] style — shows a visible slider bar.
    // `step` controls drag sensitivity (0.0 = auto).
    m.function("slider", |label: Ref<str>, value: f64, min: f64, max: f64, step: f64| -> f64 {
        with_ui(|ui| {
            let mut v = value as f32;
            let lbl_w = 100.0_f32;
            let rw     = if (v - min as f32).abs() > 1e-5 { 20.0_f32 } else { 0.0_f32 };
            let hresp = ui.horizontal(|ui| {
                ui.set_min_height(18.0);
                let lbl_color = egui::Color32::from_rgb(180, 180, 190);
                let display_lbl = label.as_ref().split("##").next().unwrap_or(label.as_ref());
                ui.add_sized([lbl_w, 16.0],
                    egui::Label::new(egui::RichText::new(display_lbl).color(lbl_color).size(11.5)));
                let avail = (ui.available_width() - rw).max(40.0);
                let mut sl = egui::Slider::new(&mut v, min as f32..=max as f32)
                    .show_value(true)
                    .clamping(egui::SliderClamping::Always);
                if step > 0.0 { sl = sl.step_by(step); }
                if ui.add_sized([avail, 16.0], sl).changed() {}
                if rw > 0.0 {
                    if ui.add(crate::icons::img("rotate-ccw", 12.0,
                        egui::Color32::from_rgb(200, 160, 60)).sense(egui::Sense::click()))
                        .on_hover_text("Reset").clicked() { v = min as f32; }
                }
            }).response;
            LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(hresp));
            v as f64
        }).unwrap_or(value)
    }).build()?;

    // header_label(text) — Unity [Header("...")] style: bold section separator.
    m.function("header_label", |text: Ref<str>| {
        with_ui(|ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(text.as_ref())
                            .strong()
                            .color(egui::Color32::from_rgb(160, 160, 175))
                            .size(10.5)
                    )
                );
                ui.separator();
            });
        });
    }).build()?;

    m.function("input_text", |label: Ref<str>, value: Ref<str>| -> String {
        with_ui(|ui| {
            let mut v = value.as_ref().to_string();
            let hresp = ui.horizontal(|ui| {
                ui.label(label.as_ref());
                ui.text_edit_singleline(&mut v);
            }).response;
            LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(hresp));
            v
        }).unwrap_or_else(|| value.as_ref().to_string())
    }).build()?;

    // input_text_autocomplete(label, value, suggestions) → [submitted, value, completed]
    // submitted  = "1" if Enter was pressed with no item selected in the dropdown.
    // completed  = the chosen suggestion, or "" if nothing was chosen yet.
    // Tab fills the longest common prefix; ↑/↓ navigate the dropdown.
    m.function("input_text_autocomplete", |label: Ref<str>, value: Ref<str>, suggestions: Vec<String>| -> Vec<String> {
        with_ui(|ui| {
            let mut v       = value.as_ref().to_string();
            let mut submitted  = false;
            let mut completed  = String::new();

            // Use a stable Id so we can query focus state from the previous frame.
            let edit_id = egui::Id::new("__cvar_cmd_input__");
            let had_focus = ui.ctx().memory(|m| m.has_focus(edit_id));

            // ── Tab: pre-consume before text_edit renders ─────────────────────
            // egui processes Tab for focus-cycling during widget rendering, so we
            // must consume it beforehand using last frame's focus state.
            let tab_pressed = if had_focus && !suggestions.is_empty() {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
            } else {
                false
            };

            let resp = ui.horizontal(|ui| {
                ui.label(label.as_ref());
                let r = ui.add(
                    egui::TextEdit::singleline(&mut v)
                        .id(edit_id)
                        .desired_width(ui.available_width())
                );
                r
            }).inner;

            let has_focus = resp.has_focus();

            // ── Tab: apply completion ─────────────────────────────────────────
            if tab_pressed {
                let lcp = longest_common_prefix(&suggestions);
                if !lcp.is_empty() {
                    if suggestions.len() == 1 {
                        completed = suggestions[0].clone();
                    } else {
                        completed = lcp;
                    }
                }
                AUTOCOMPLETE_IDX.with(|c| c.set(-1));
            }

            // ── Arrow key navigation ──────────────────────────────────────────
            if has_focus && !suggestions.is_empty() {
                let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
                let up   = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
                AUTOCOMPLETE_IDX.with(|c| {
                    let mut idx = c.get();
                    if down { idx = (idx + 1).min(suggestions.len() as i64 - 1); }
                    if up   { idx = (idx - 1).max(-1); }
                    c.set(idx);
                });
            }

            // Reset index when suggestions list changes length (new typing).
            let idx = AUTOCOMPLETE_IDX.with(|c| c.get());
            if idx >= suggestions.len() as i64 {
                AUTOCOMPLETE_IDX.with(|c| c.set(-1));
            }
            let idx = AUTOCOMPLETE_IDX.with(|c| c.get());

            // ── Escape: dismiss dropdown ──────────────────────────────────────
            if has_focus {
                let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if esc {
                    AUTOCOMPLETE_IDX.with(|c| c.set(-1));
                }
            }

            // ── Enter: submit OR confirm dropdown selection ───────────────────
            if has_focus {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter {
                    if idx >= 0 {
                        // Confirm dropdown selection.
                        completed = suggestions[idx as usize].clone();
                        AUTOCOMPLETE_IDX.with(|c| c.set(-1));
                    } else {
                        submitted = true;
                    }
                }
            }

            // ── Dropdown popup ────────────────────────────────────────────────
            // POPUP_HOVERED tracks mouse presence inside the popup area.
            // This keeps the popup alive through mouse-down → mouse-up so
            // row.clicked() can fire even after the text field lost focus.
            // AUTOCOMPLETE_IDX is now ONLY mutated by arrow keys, never by
            // hover, so it cannot stale-intercept Enter submissions.
            let popup_was_hovered = POPUP_HOVERED.with(|c| c.get());
            if !suggestions.is_empty() && (has_focus || popup_was_hovered) {
                let popup_id  = egui::Id::new("__cvar_autocomplete_popup__");
                let field_pos = egui::pos2(resp.rect.left(), resp.rect.top());
                let mut any_row_hovered = false;

                egui::Area::new(popup_id)
                    .order(egui::Order::Foreground)
                    .pivot(egui::Align2::LEFT_BOTTOM)
                    .fixed_pos(field_pos)
                    .show(ui.ctx(), |popup_ui| {
                        egui::Frame::popup(popup_ui.style()).show(popup_ui, |inner| {
                            inner.set_min_width(resp.rect.width());
                            egui::ScrollArea::vertical()
                                .max_height(130.0)
                                .show(inner, |scroll| {
                                    for (i, suggestion) in suggestions.iter().enumerate() {
                                        let kbd_selected = i as i64 == idx;
                                        let row = scroll.selectable_label(
                                            kbd_selected,
                                            egui::RichText::new(suggestion).monospace().size(11.0),
                                        );
                                        if row.hovered() {
                                            // Hover gives visual feedback via egui's built-in
                                            // hover style on selectable_label; we do NOT update
                                            // AUTOCOMPLETE_IDX here to avoid stale state.
                                            any_row_hovered = true;
                                        }
                                        if row.clicked() {
                                            completed = suggestion.clone();
                                            AUTOCOMPLETE_IDX.with(|c| c.set(-1));
                                        }
                                    }
                                });
                        });
                    });

                POPUP_HOVERED.with(|c| c.set(any_row_hovered));
            } else {
                POPUP_HOVERED.with(|c| c.set(false));
                if suggestions.is_empty() {
                    AUTOCOMPLETE_IDX.with(|c| c.set(-1));
                }
            }

            // ── Restore focus after completion ────────────────────────────────
            if !completed.is_empty() {
                ui.ctx().memory_mut(|m| m.request_focus(edit_id));
            }

            vec![
                if submitted { "1".to_string() } else { "0".to_string() },
                v,
                completed,
            ]
        }).unwrap_or_else(|| vec!["0".to_string(), value.as_ref().to_string(), String::new()])
    }).build()?;

    m.function("input_text_enter", |label: Ref<str>, value: Ref<str>| -> Vec<String> {
        with_ui(|ui| {
            let mut v = value.as_ref().to_string();
            let mut submitted = false;
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                let resp = ui.text_edit_singleline(&mut v);
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submitted = true;
                }
            });
            vec![if submitted { "1".to_string() } else { "0".to_string() }, v]
        }).unwrap_or_else(|| vec!["0".to_string(), value.as_ref().to_string()])
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
                let chev = if is_open { "chevron-down" } else { "chevron-right" };
                let resp = ui.add(crate::icons::img(chev, 12.0, ui.visuals().text_color()).sense(egui::Sense::click()));
                ui.label(display);
                resp.clicked()
            }).inner;
            if clicked {
                let toggled = !is_open;
                ui.memory_mut(|m| m.data.insert_persisted(id, toggled));
            }
            is_open
        }).unwrap_or(false)
    }).build()?;

    m.function("collapsing_end", || {}).build()?;

    // icon_collapsing_begin(icon, label) → bool
    // Same as collapsing_begin but prepends a 14px SVG icon on the left.
    // icon: Lucide icon name without path/extension (e.g. "box", "camera").
    // label: supports "Display##unique_id" convention.
    m.function("icon_collapsing_begin", |icon: Ref<str>, label: Ref<str>| -> bool {
        with_ui(|ui| {
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
                let sz   = 14.0f32;
                let tint = ui.visuals().text_color();
                if let Some(bytes) = crate::icons::icon_bytes(icon.as_ref()) {
                    let uri = crate::icons::icon_uri(icon.as_ref());
                    ui.add(
                        egui::Image::from_bytes(uri, bytes)
                            .fit_to_exact_size(egui::vec2(sz, sz))
                            .tint(tint),
                    );
                }
                let chev = if is_open { "chevron-down" } else { "chevron-right" };
                let resp = ui.add(crate::icons::img(chev, 12.0, ui.visuals().text_color()).sense(egui::Sense::click()));
                ui.label(display);
                resp.clicked()
            }).inner;
            if clicked {
                let toggled = !is_open;
                ui.memory_mut(|m| m.data.insert_persisted(id, toggled));
            }
            is_open
        }).unwrap_or(false)
    }).build()?;

    // horizontal_begin / horizontal_end — lay out widgets left-to-right (wrapping)
    // by creating a child UI with left-to-right wrapping layout.
    m.function("horizontal_begin", || {
        let parent_ptr = CURRENT_UI.with(|c| c.get());
        HORIZ_PARENT.with(|p| p.set(parent_ptr));
        let Some(mut ptr) = parent_ptr else { return };
        let child = unsafe {
            let ui      = ptr.as_mut();
            let avail_w = ui.available_width();
            // Use a compact height so items are not vertically centered in the
            // full remaining panel height (which would create huge empty gaps).
            let avail_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y * 2.0;
            let cursor  = ui.cursor().min;
            let rect    = egui::Rect::from_min_size(cursor, egui::vec2(avail_w, avail_h));
            Box::new(ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true)),
            ))
        };
        HORIZ_CHILD.with(|c| *c.borrow_mut() = Some(child));
        let child_ptr = HORIZ_CHILD.with(|c| {
            c.borrow().as_ref().map(|b| NonNull::from(b.as_ref()))
        });
        CURRENT_UI.with(|c| c.set(child_ptr));
    }).build()?;

    m.function("horizontal_end", || {
        let child_rect = HORIZ_CHILD.with(|c| c.borrow().as_ref().map(|u| u.min_rect()));
        let parent_ptr = HORIZ_PARENT.with(|p| p.get());
        CURRENT_UI.with(|c| c.set(parent_ptr));
        if let Some(mut ptr) = parent_ptr {
            if let Some(cr) = child_rect {
                unsafe { ptr.as_mut() }.allocate_rect(cr, egui::Sense::hover());
            }
        }
        HORIZ_CHILD .with(|c| *c.borrow_mut() = None);
        HORIZ_PARENT.with(|p| p.set(None));
    }).build()?;

    m.function("scroll_begin", || {}).build()?;
    m.function("scroll_end",   || {}).build()?;

    // ── Two-column layout ──────────────────────────────────────────────────────
    // columns_begin(left_width) — start a 2-column layout. Left column gets
    // `left_width` px; right gets the remainder. CURRENT_UI is switched to the
    // left child. Call columns_next() then columns_end() to finish.
    m.function("columns_begin", |left_w: f64| {
        let parent_ptr = CURRENT_UI.with(|c| c.get());
        COLS_PARENT.with(|p| p.set(parent_ptr));
        let Some(mut ptr) = parent_ptr else { return };
        let (left, right) = unsafe {
            let ui      = ptr.as_mut();
            let spacing = ui.spacing().item_spacing.x;
            let lw      = left_w as f32;
            let rw      = (ui.available_width() - lw - spacing).max(10.0);
            let avail_h = ui.available_height();
            let cursor  = ui.cursor().min;
            let lr = egui::Rect::from_min_size(cursor, egui::vec2(lw, avail_h));
            let rr = egui::Rect::from_min_size(
                cursor + egui::vec2(lw + spacing, 0.0),
                egui::vec2(rw, avail_h));
            let l = Box::new(ui.new_child(egui::UiBuilder::new().max_rect(lr).layout(egui::Layout::top_down(egui::Align::LEFT))));
            let r = Box::new(ui.new_child(egui::UiBuilder::new().max_rect(rr).layout(egui::Layout::top_down(egui::Align::LEFT))));
            (l, r)
        };
        LEFT_CHILD.with(|l| *l.borrow_mut()  = Some(left));
        RIGHT_CHILD.with(|r| *r.borrow_mut() = Some(right));
        // Point CURRENT_UI at the left child (Box keeps heap addr stable).
        let left_ptr = LEFT_CHILD.with(|l| {
            l.borrow().as_ref().map(|b| NonNull::from(b.as_ref()))
        });
        CURRENT_UI.with(|c| c.set(left_ptr));
    }).build()?;

    // columns_next() — switch CURRENT_UI to the right column.
    m.function("columns_next", || {
        let right_ptr = RIGHT_CHILD.with(|r| {
            r.borrow().as_ref().map(|b| NonNull::from(b.as_ref()))
        });
        CURRENT_UI.with(|c| c.set(right_ptr));
    }).build()?;

    // columns_end() — restore parent UI and advance its cursor past both children.
    m.function("columns_end", || {
        let left_rect  = LEFT_CHILD .with(|l| l.borrow().as_ref().map(|u| u.min_rect()));
        let right_rect = RIGHT_CHILD.with(|r| r.borrow().as_ref().map(|u| u.min_rect()));
        let parent_ptr = COLS_PARENT.with(|p| p.get());
        CURRENT_UI.with(|c| c.set(parent_ptr));
        if let Some(mut ptr) = parent_ptr {
            if let (Some(lr), Some(rr)) = (left_rect, right_rect) {
                let union_rect = lr.union(rr);
                unsafe { ptr.as_mut() }.allocate_rect(union_rect, egui::Sense::hover());
            }
        }
        LEFT_CHILD .with(|l| *l.borrow_mut() = None);
        RIGHT_CHILD.with(|r| *r.borrow_mut() = None);
        COLS_PARENT.with(|p| p.set(None));
    }).build()?;

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
                        ui.close();
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
            let raw = label.as_ref();
            let (display, id_str) = if let Some(pos) = raw.find("##") {
                (&raw[..pos], &raw[pos+2..])
            } else {
                (raw, raw)
            };
            let mut chosen = String::new();
            egui::ComboBox::from_id_salt(id_str)
                .selected_text(display)
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

    // enum_combo_pipe(label, options_pipe, current) → String
    // Like enum_combo but takes a "|"-separated options string instead of Vec<String>.
    // Also accepts the raw "enum:a|b|c" format returned by asset_get_import.
    m.function("enum_combo_pipe", |label: Ref<str>, options_pipe: Ref<str>, current: Ref<str>| -> String {
        with_ui(|ui| {
            let raw_lbl = label.as_ref();
            let (display, id_str) = if let Some(pos) = raw_lbl.find("##") {
                (&raw_lbl[..pos], &raw_lbl[pos+2..])
            } else {
                (raw_lbl, raw_lbl)
            };
            let raw  = options_pipe.as_ref();
            let pipe = raw.strip_prefix("enum:").unwrap_or(raw);
            let opts: Vec<&str> = pipe.split('|').collect();
            let mut selected = current.as_ref().to_string();
            egui::ComboBox::from_id_salt(id_str)
                .selected_text(if selected.is_empty() { display } else { &selected })
                .show_ui(ui, |ui| {
                    ui.label(display);
                    for opt in &opts {
                        if ui.selectable_label(*opt == selected.as_str(), *opt).clicked() {
                            selected = opt.to_string();
                        }
                    }
                });
            selected
        }).unwrap_or_else(|| current.as_ref().to_string())
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
                        ui.close();
                    }
                    if ui.button("Paste value").clicked() {
                        action = "paste".to_string();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Reset to default").clicked() {
                        action = "reset".to_string();
                        ui.close();
                    }
                });
            }
        });
        action
    }).build()?;

    // prop_tooltip(text) — show tooltip on hover over the last widget.
    // Call immediately after a property widget (before prop_context_menu).
    // Works because all property widgets store their response in LAST_WIDGET_RESP.
    m.function("prop_tooltip", |text: String| {
        if text.is_empty() { return; }
        LAST_WIDGET_RESP.with(|resp_ref| {
            if let Some(resp) = resp_ref.borrow().as_ref() {
                resp.clone().on_hover_text(&text);
            }
        });
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

    // asset_path_picker(label, path, type_filter) → String
    // type_filter: "material" | "mesh" | "audio" | "scene" | "texture" | ""
    // Renders a compact asset row: [type-icon] [filename-monospace] [clear-btn]
    m.function("asset_path_picker", |label: Ref<str>, path: Ref<str>, type_filter: Ref<str>| -> String {
        with_ui(|ui| {
            let mut v = path.as_ref().to_string();
            let type_flt = type_filter.as_ref();

            // icon + tint per asset type
            let (icon_name, tint) = match type_flt {
                "material" => ("layers",   egui::Color32::from_rgb(100, 160, 220)),
                "mesh"     => ("box",      egui::Color32::from_rgb(180, 130,  80)),
                "audio"    => ("volume-2", egui::Color32::from_rgb( 80, 190, 140)),
                "scene"    => ("film",     egui::Color32::from_rgb(200, 160,  60)),
                "texture"  => ("image",    egui::Color32::from_rgb(160, 100, 210)),
                _          => ("file",     egui::Color32::from_rgb(160, 160, 175)),
            };

            let label_color = egui::Color32::from_rgb(180, 180, 190);
            let lbl_w = 100.0_f32;

            let hresp = ui.horizontal(|ui| {
                ui.set_min_height(20.0);
                ui.add_sized([lbl_w, 16.0],
                    egui::Label::new(egui::RichText::new(label.as_ref()).color(label_color).size(11.5)));

                // dark background box for the asset path
                let avail = ui.available_width() - 18.0;
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(avail, 20.0), egui::Sense::click());
                ui.painter().rect_filled(
                    rect, 3.0, egui::Color32::from_rgb(40, 40, 48));

                // type icon inside the box
                let icon_rect = egui::Rect::from_min_size(
                    rect.min + egui::vec2(2.0, 2.0),
                    egui::vec2(16.0, 16.0));
                ui.put(icon_rect, crate::icons::img(icon_name, 14.0, tint));

                // filename text
                let fname: String = if v.is_empty() {
                    format!("None ({})", if type_flt.is_empty() { "Asset" } else { type_flt })
                } else {
                    v.split(['/', '\\']).last().unwrap_or(&v).to_string()
                };
                let fname_color = if v.is_empty() {
                    egui::Color32::from_rgb(120, 120, 130)
                } else {
                    egui::Color32::from_rgb(200, 200, 210)
                };
                let text_rect = egui::Rect::from_min_size(
                    rect.min + egui::vec2(20.0, 2.0),
                    egui::vec2(avail - 24.0, 16.0));
                ui.painter().text(
                    text_rect.min,
                    egui::Align2::LEFT_TOP,
                    &fname,
                    egui::FontId::monospace(10.5),
                    fname_color,
                );

                // edit popup on click — for now use a text edit fallback
                if ui.interact(rect, egui::Id::new((label.as_ref(), "asset_click")),
                    egui::Sense::click()).double_clicked() {
                    let mut tmp = v.clone();
                    ui.text_edit_singleline(&mut tmp);
                    v = tmp;
                }

                // ── DnD drop zone: accept Asset payload ──────────────────────
                let drop_id   = egui::Id::new((label.as_ref(), "asset_drop"));
                let drop_resp = ui.interact(rect, drop_id, egui::Sense::hover());
                if drop_resp.dnd_hover_payload::<DndPayload>().is_some() {
                    ui.painter().rect_stroke(
                        rect, 3.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(80, 200, 80)), egui::StrokeKind::Outside);
                }
                if let Some(payload) = drop_resp.dnd_release_payload::<DndPayload>() {
                    if let DndPayload::Asset { path: dropped, asset_type: dropped_t } = payload.as_ref() {
                        let type_ok = type_flt.is_empty()
                            || dropped_t == type_flt
                            || (type_flt == "mesh" && dropped_t == "model");
                        if type_ok {
                            v = dropped.clone();
                        }
                    }
                }

                // clear button (×)
                if !v.is_empty() {
                    if ui.add(
                        crate::icons::img("x", 12.0, egui::Color32::from_rgb(180, 80, 80))
                            .sense(egui::Sense::click())
                    ).on_hover_text("Clear").clicked() {
                        v.clear();
                    }
                }
            }).response;
            LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(hresp));
            v
        }).unwrap_or_else(|| path.as_ref().to_string())
    }).build()?;

    // entity_picker(label, entity_id) → i64
    // Shows entity name + "Pick" button. Returns the (possibly changed) entity ID.
    m.function("entity_picker", |label: Ref<str>, entity_id: i64| -> i64 {
        let lbl = label.as_ref().to_string();
        with_ui(|ui| {
            let lbl_w = 100.0_f32;
            let label_color = egui::Color32::from_rgb(180, 180, 190);
            let mut result = entity_id;
            let hresp = ui.horizontal(|ui| {
                ui.set_min_height(20.0);
                ui.add_sized([lbl_w, 16.0],
                    egui::Label::new(egui::RichText::new(lbl.as_str()).color(label_color).size(11.5)));

                let entity_name = crate::rune_bindings::world_module::entity_name_for_id(entity_id);

                let avail = ui.available_width() - 40.0;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(avail, 20.0), egui::Sense::click());
                ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb(40, 40, 48));
                let name_color = if entity_id < 0 {
                    egui::Color32::from_rgb(120, 120, 130)
                } else {
                    egui::Color32::from_rgb(200, 200, 210)
                };
                ui.painter().text(rect.min + egui::vec2(4.0, 3.0),
                    egui::Align2::LEFT_TOP, &entity_name,
                    egui::FontId::proportional(11.0), name_color);

                // ── DnD drop zone: accept Entity payload ─────────────────────
                let ep_drop_id   = egui::Id::new((lbl.as_str(), "entity_drop"));
                let ep_drop_resp = ui.interact(rect, ep_drop_id, egui::Sense::hover());
                if ep_drop_resp.dnd_hover_payload::<DndPayload>().is_some() {
                    ui.painter().rect_stroke(
                        rect, 3.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(80, 200, 80)), egui::StrokeKind::Outside);
                }
                if let Some(payload) = ep_drop_resp.dnd_release_payload::<DndPayload>() {
                    if let DndPayload::Entity { id: dropped_id } = payload.as_ref() {
                        result = *dropped_id;
                    }
                }

                // clear / pick  — just clear for now
                if entity_id >= 0 {
                    if ui.small_button("×").on_hover_text("Clear").clicked() {
                        result = -1;
                    }
                }
            }).response;
            LAST_WIDGET_RESP.with(|r| *r.borrow_mut() = Some(hresp));
            result
        }).unwrap_or(entity_id)
    }).build()?;

    // vec3_uniform_scale(label, x, y, z) → Vec<f64>
    // XYZ drag fields with a proportional-lock toggle button.
    // Returns [x, y, z, lock_state] where lock_state is 0.0 or 1.0.
    m.function("vec3_uniform_scale", |label: Ref<str>, x: f64, y: f64, z: f64| -> Vec<f64> {
        with_ui(|ui| {
            let lbl = label.as_ref().to_string();
            let lbl_id = egui::Id::new((lbl.clone(), "scale_lock"));
            let locked: bool = ui.data_mut(|d| *d.get_temp_mut_or(lbl_id, false));

            let lbl_w   = 100.0_f32;
            let col_w   = 18.0_f32;
            let lbl_color = egui::Color32::from_rgb(180, 180, 190);

            let mut vx = x as f32;
            let mut vy = y as f32;
            let mut vz = z as f32;
            let old_x = vx; let old_y = vy; let old_z = vz;

            let lock_tint = if locked {
                egui::Color32::from_rgb(220, 180, 60)
            } else {
                egui::Color32::from_rgb(120, 120, 140)
            };

            ui.horizontal(|ui| {
                ui.set_min_height(20.0);
                ui.add_sized([lbl_w, 16.0],
                    egui::Label::new(egui::RichText::new(lbl.as_str()).color(lbl_color).size(11.5)));

                let field_w = ((ui.available_width() - col_w - 4.0) / 3.0).max(30.0);

                // X (red badge)
                let xc = egui::Color32::from_rgb(210, 80, 80);
                ui.add_sized([8.0, 16.0],
                    egui::Label::new(egui::RichText::new("X").color(xc).strong().size(10.0)));
                ui.add_sized([field_w, 16.0], egui::DragValue::new(&mut vx).speed(0.01).max_decimals(3));

                // Y (green badge)
                let yc = egui::Color32::from_rgb(80, 200, 80);
                ui.add_sized([8.0, 16.0],
                    egui::Label::new(egui::RichText::new("Y").color(yc).strong().size(10.0)));
                ui.add_sized([field_w, 16.0], egui::DragValue::new(&mut vy).speed(0.01).max_decimals(3));

                // Z (blue badge)
                let zc = egui::Color32::from_rgb(80, 120, 220);
                ui.add_sized([8.0, 16.0],
                    egui::Label::new(egui::RichText::new("Z").color(zc).strong().size(10.0)));
                ui.add_sized([field_w, 16.0], egui::DragValue::new(&mut vz).speed(0.01).max_decimals(3));

                // lock button
                let lock_icon = if locked { "lock" } else { "unlock" };
                if ui.add(
                    crate::icons::img(lock_icon, 13.0, lock_tint).sense(egui::Sense::click())
                ).on_hover_text(if locked { "Unlock proportional scale" } else { "Lock proportional scale" })
                 .clicked() {
                    let new_lock = !locked;
                    ui.data_mut(|d| d.insert_temp(lbl_id, new_lock));
                }
            });

            // apply uniform scaling if locked
            if locked {
                if (vx - old_x).abs() > 1e-6 && old_x.abs() > 1e-6 {
                    let ratio = vx / old_x;
                    vy = old_y * ratio;
                    vz = old_z * ratio;
                } else if (vy - old_y).abs() > 1e-6 && old_y.abs() > 1e-6 {
                    let ratio = vy / old_y;
                    vx = old_x * ratio;
                    vz = old_z * ratio;
                } else if (vz - old_z).abs() > 1e-6 && old_z.abs() > 1e-6 {
                    let ratio = vz / old_z;
                    vx = old_x * ratio;
                    vy = old_y * ratio;
                }
            }

            vec![vx as f64, vy as f64, vz as f64, if locked { 1.0 } else { 0.0 }]
        }).unwrap_or_else(|| vec![x, y, z, 0.0])
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

    // icon_tool_button(icon_name, active) → bool
    // Renders an SVG icon as a toolbar toggle button.
    // active=true tints the icon yellow; false = muted grey.
    m.function("icon_tool_button", |icon: Ref<str>, active: bool| -> bool {
        with_ui(|ui| {
            let tint = if active {
                egui::Color32::from_rgb(220, 180, 60)
            } else {
                egui::Color32::from_rgb(160, 160, 175)
            };
            let resp = ui.add(
                egui::Button::image(crate::icons::img(icon.as_ref(), 18.0, tint)).frame(false)
            );
            if active {
                ui.painter().rect_stroke(resp.rect.expand(1.0), 2.0, egui::Stroke::new(1.0, tint), egui::StrokeKind::Outside);
            }
            resp.clicked()
        }).unwrap_or(false)
    }).build()?;

    // icon_button(icon_name, size, r, g, b) → bool
    // Renders a frameless SVG icon button with explicit tint color.
    m.function("icon_button", |icon: Ref<str>, size: f64, r: f64, g: f64, b: f64| -> bool {
        with_ui(|ui| {
            let tint = egui::Color32::from_rgb(
                (r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8
            );
            ui.add(
                egui::Button::image(crate::icons::img(icon.as_ref(), size as f32, tint)).frame(false)
            ).clicked()
        }).unwrap_or(false)
    }).build()?;

    // icon_label(icon_name, size, r, g, b) — renders an inline SVG icon (non-interactive).
    m.function("icon_label", |icon: Ref<str>, size: f64, r: f64, g: f64, b: f64| {
        with_ui(|ui| {
            let tint = egui::Color32::from_rgb(
                (r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8
            );
            ui.add(crate::icons::img(icon.as_ref(), size as f32, tint));
        });
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
        let label_str = label.as_ref().to_string();
        MENU_LABEL.with(|c| *c.borrow_mut() = label_str.clone());
        // Clear items being collected this frame.
        MENU_ITEMS.with(|c| c.borrow_mut().clear());

        // Fetch items that were collected LAST frame for this menu.
        let prev_items: Vec<MenuEntry> = MENU_ITEMS_PREV.with(|c| {
            c.borrow().get(&label_str).cloned().unwrap_or_default()
        });

        let mut clicked_item = String::new();
        // Use egui's native menu_button which handles popup state, positioning,
        // and close-on-outside-click correctly.
        let is_open = with_ui(|ui| {
            let resp = ui.menu_button(label_str.as_str(), |popup_ui| {
                for entry in &prev_items {
                    match entry {
                        MenuEntry::Item(lbl) => {
                            if popup_ui.button(lbl).clicked() {
                                clicked_item = lbl.clone();
                                popup_ui.close();
                            }
                        }
                        MenuEntry::Separator => { popup_ui.separator(); }
                    }
                }
            });
            resp.inner.is_some()
        }).unwrap_or(false);

        if !clicked_item.is_empty() {
            MENU_CLICKED.with(|c| c.borrow_mut().insert(label_str.clone(), clicked_item));
        }

        // On the very first frame a menu is encountered, return true so Rune
        // queues the items (they will render on the next open).
        is_open
    }).build()?;

    m.function("menu_item", |label: Ref<str>| {
        // Just queue the label; rendering and click detection happen in menu_end.
        MENU_ITEMS.with(|c| c.borrow_mut().push(MenuEntry::Item(label.as_ref().to_string())));
    }).build()?;

    m.function("menu_separator", || {
        MENU_ITEMS.with(|c| c.borrow_mut().push(MenuEntry::Separator));
    }).build()?;

    // menu_end() → String
    // Promotes this frame's collected items to MENU_ITEMS_PREV so they appear
    // the next time the menu is open, then returns any clicked item label.
    // Must always be called after every menu_begin/menu_item block.
    m.function("menu_end", || -> String {
        let label_str = MENU_LABEL.with(|c| c.borrow().clone());

        // Promote current items → prev (used by menu_begin next frame).
        let items: Vec<MenuEntry> = MENU_ITEMS.with(|c| c.borrow().clone());
        MENU_ITEMS_PREV.with(|c| {
            if !items.is_empty() {
                c.borrow_mut().insert(label_str.clone(), items);
            }
        });
        MENU_ITEMS.with(|c| c.borrow_mut().clear());

        // Return (and clear) any item that was clicked this frame.
        MENU_CLICKED.with(|c| {
            c.borrow_mut().remove(&label_str).unwrap_or_default()
        })
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

    // viewport_overlay_text(text) — draw small stats text at the top of the current panel.
    m.function("viewport_overlay_text", |text: String| {
        with_ui(|ui| {
            let color = egui::Color32::from_rgba_unmultiplied(220, 220, 220, 200);
            ui.label(
                egui::RichText::new(&text)
                    .font(egui::FontId::monospace(11.0))
                    .color(color),
            );
        });
    }).build()?;

    // ── Clipboard ────────────────────────────────────────────────────────────

    // clipboard_set(text) — write text to the OS clipboard.
    m.function("clipboard_set", |text: String| {
        with_ui(|ui| { ui.ctx().copy_text(text); });
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
                        ui.close();
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
                            ui.close();
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

    // ── Settings modal windows ───────────────────────────────────────────────

    // modal_begin(id, title, width, height) → bool
    // Opens a centered egui::Window with a dim overlay.
    // Returns true while the modal is open and should render its body.
    // Must always be followed by modal_end().
    m.function("modal_begin",
        |id: Ref<str>, title: Ref<str>, width: f64, height: f64| -> bool
    {
        let id_s    = id.as_ref().to_string();
        let title_s = title.as_ref().to_string();
        // If close was signalled last frame, clear modal
        if MODAL_CLOSE.with(|c| c.get()) {
            MODAL_CLOSE.with(|c| c.set(false));
            MODAL_OPEN.with(|m| *m.borrow_mut() = None);
            return false;
        }
        // If no modal open yet with this id, open it
        let currently_open = MODAL_OPEN.with(|m| m.borrow().as_deref() == Some(&id_s));
        if !currently_open {
            // If another modal is open, don't override
            let any_open = MODAL_OPEN.with(|m| m.borrow().is_some());
            if any_open { return false; }
            MODAL_OPEN.with(|m| *m.borrow_mut() = Some(id_s.clone()));
        }
        with_ui(|ui| {
            // Dim overlay behind the modal
            let screen = ui.ctx().content_rect();
            ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("modal_overlay"),
            )).rect_filled(screen, 0.0, egui::Color32::from_black_alpha(140));

            let mut open = true;
            egui::Window::new(title_s.as_str())
                .id(egui::Id::new(id_s.as_str()))
                .collapsible(false)
                .resizable(true)
                .min_size([width as f32 * 0.5, height as f32 * 0.5])
                .default_size([width as f32, height as f32])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ui.ctx(), |_inner_ui| {
                    // Body is rendered by Rune script between modal_begin/end
                });
            if !open {
                MODAL_OPEN.with(|m| *m.borrow_mut() = None);
            }
            open
        }).unwrap_or(false)
    }).build()?;

    // modal_end() — must be called unconditionally after modal_begin.
    m.function("modal_end", || {
        // No-op: body was already rendered inside the Window::show closure via
        // the side-by-side layout approach below. State cleanup is handled by modal_begin.
    }).build()?;

    // modal_close() — call from Rune to close the current modal next frame.
    m.function("modal_close", || {
        MODAL_CLOSE.with(|c| c.set(true));
    }).build()?;

    // modal_open(id) — open a modal by setting the active modal ID.
    // Call this from menubar / button handler to trigger a modal next frame.
    m.function("modal_open", |id: Ref<str>| {
        let close_pending = MODAL_CLOSE.with(|c| c.get());
        if !close_pending {
            MODAL_OPEN.with(|m| *m.borrow_mut() = Some(id.as_ref().to_string()));
        }
    }).build()?;

    // modal_is_open(id) → bool — returns true when this modal id is active.
    m.function("modal_is_open", |id: Ref<str>| -> bool {
        MODAL_OPEN.with(|m| m.borrow().as_deref() == Some(id.as_ref()))
    }).build()?;

    // project_settings_window(width, height) → bool
    // Renders the full V3-style Project Settings modal (dim overlay + sidebar + rows).
    // Returns true while open, false when ✕ is clicked.
    m.function("project_settings_window", |width: f64, height: f64| -> bool {
        use super::settings_module as sm;
        let ctx = match CURRENT_CTX.with(|c| c.borrow().clone()) {
            Some(c) => c,
            None    => return false,
        };
        let mut keep_open = true;
        {
            let screen = ctx.content_rect();
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("proj_settings_v3_overlay"),
            )).rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));

            let w = width as f32;
            let h = height as f32;
            egui::Area::new(egui::Id::new("proj_settings_v3"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    egui::Frame::window(ui.style())
                        .fill(egui::Color32::from_rgb(30, 30, 30))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(60,60,60)))
                        .show(ui, |ui| {
                            ui.set_min_size(egui::vec2(w, h));
                            ui.set_max_width(w);

                            // Header bar
                            ui.horizontal(|ui| {
                                ui.add(crate::icons::img("settings", 14.0, sc_text()));
                                ui.label(egui::RichText::new(" Project Settings").size(13.0).color(sc_text()).strong());
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if crate::icons::btn(ui, "x", 14.0, sc_text(), "Close") {
                                        sm::close_project_settings_ui();
                                        keep_open = false;
                                    }
                                    if crate::icons::btn(ui, "rotate-ccw", 13.0, sc_red(), "Reset All to defaults") {
                                        sm::reset_project_to_defaults();
                                    }
                                });
                            });
                            ui.separator();

                            // Search bar
                            let mut search = sm::settings_search_query();
                            let search_resp = ui.horizontal(|ui| {
                                ui.add(crate::icons::img("search", 13.0, egui::Color32::from_gray(140)));
                                ui.add(egui::TextEdit::singleline(&mut search)
                                    .hint_text("Search settings…")
                                    .desired_width(ui.available_width()))
                            }).inner;
                            if search_resp.changed() {
                                sm::set_settings_search_query(search.clone());
                            }
                            ui.separator();

                            // Sidebar + content
                            let cats = ["Physics","Rendering","Audio","Input","Tags & Layers","Build"];
                            let counts: Vec<usize> = cats.iter().map(|c| sm::project_category_modified_count(c)).collect();

                            egui::Panel::left("proj_settings_sidebar_v3")
                                .exact_size(160.0)
                                .frame(egui::Frame::default()
                                    .fill(sc_sidebar())
                                    .inner_margin(egui::Margin::same(4))
                                )
                                .show_inside(ui, |ui| {
                                    let active = sm::project_tab();
                                    let new_tab = v3_sidebar(ui, &cats, &counts, &active);
                                    if new_tab != active { sm::set_project_tab_ui(new_tab); }
                                });

                            egui::ScrollArea::vertical()
                                .id_salt("proj_settings_content_v3")
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    ui.add_space(4.0);
                                    let search = sm::settings_search_query();
                                    if search.is_empty() {
                                        render_project_content_v3(ui, &sm::project_tab());
                                    } else {
                                        for cat in &cats { render_project_content_v3(ui, cat); }
                                    }
                                    ui.add_space(8.0);
                                });
                        });
                });
            keep_open
        }
    }).build()?;

    // editor_prefs_window(width, height) → bool
    // Renders the full V3-style Editor Preferences modal.
    // Returns true while open, false when ✕ is clicked.
    m.function("editor_prefs_window", |width: f64, height: f64| -> bool {
        use super::settings_module as sm;
        let ctx = match CURRENT_CTX.with(|c| c.borrow().clone()) {
            Some(c) => c,
            None    => return false,
        };
        let mut keep_open = true;
        {
            let screen = ctx.content_rect();
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("editor_prefs_v3_overlay"),
            )).rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));

            let w = width as f32;
            let h = height as f32;
            egui::Area::new(egui::Id::new("editor_prefs_v3"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    egui::Frame::window(ui.style())
                        .fill(egui::Color32::from_rgb(30, 30, 30))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(60,60,60)))
                        .show(ui, |ui| {
                            ui.set_min_size(egui::vec2(w, h));
                            ui.set_max_width(w);

                            // Header bar
                            ui.horizontal(|ui| {
                                ui.add(crate::icons::img("settings", 14.0, sc_text()));
                                ui.label(egui::RichText::new(" Editor Preferences").size(13.0).color(sc_text()).strong());
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if crate::icons::btn(ui, "x", 14.0, sc_text(), "Close") {
                                        sm::close_editor_prefs_ui();
                                        keep_open = false;
                                    }
                                    if crate::icons::btn(ui, "rotate-ccw", 13.0, sc_red(), "Reset All to defaults") {
                                        sm::reset_prefs_to_defaults();
                                    }
                                });
                            });
                            ui.separator();

                            // Search bar
                            let mut search = sm::settings_search_query();
                            let search_resp = ui.horizontal(|ui| {
                                ui.add(crate::icons::img("search", 13.0, egui::Color32::from_gray(140)));
                                ui.add(egui::TextEdit::singleline(&mut search)
                                    .hint_text("Search preferences…")
                                    .desired_width(ui.available_width()))
                            }).inner;
                            if search_resp.changed() {
                                sm::set_settings_search_query(search.clone());
                            }
                            ui.separator();

                            let cats = ["General","Camera","Console"];
                            let counts: Vec<usize> = cats.iter().map(|c| sm::prefs_category_modified_count(c)).collect();

                            egui::Panel::left("editor_prefs_sidebar_v3")
                                .exact_size(140.0)
                                .frame(egui::Frame::default()
                                    .fill(sc_sidebar())
                                    .inner_margin(egui::Margin::same(4))
                                )
                                .show_inside(ui, |ui| {
                                    let active = sm::prefs_tab();
                                    let new_tab = v3_sidebar(ui, &cats, &counts, &active);
                                    if new_tab != active { sm::set_prefs_tab_ui(new_tab); }
                                });

                            egui::ScrollArea::vertical()
                                .id_salt("editor_prefs_content_v3")
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    ui.add_space(4.0);
                                    let search = sm::settings_search_query();
                                    if search.is_empty() {
                                        render_prefs_content_v3(ui, &sm::prefs_tab());
                                    } else {
                                        for cat in &cats { render_prefs_content_v3(ui, cat); }
                                    }
                                    ui.add_space(8.0);
                                });
                        });
                });
            keep_open
        }
    }).build()?;

    // ── Console panel widgets ────────────────────────────────────────────────

    // Renders the console toolbar row (Clear + level toggles + Collapse button).
    // Returns a string indicating which button was activated:
    //   "clear" | "info" | "warn" | "error" | "collapse" | "" (nothing clicked)
    // counts: [info_cnt, warn_cnt, error_cnt]
    m.function("console_toolbar",
        |show_info: bool, show_warn: bool, show_error: bool,
         collapse: bool, counts: Vec<i64>|
        -> String
    {
        let info_cnt  = counts.get(0).copied().unwrap_or(0);
        let warn_cnt  = counts.get(1).copied().unwrap_or(0);
        let error_cnt = counts.get(2).copied().unwrap_or(0);
        with_ui(|ui| {
            let mut action = String::new();
            ui.horizontal(|ui| {
                if ui.button("🗑 Clear").on_hover_text("Clear all log messages").clicked() {
                    action = "clear".to_string();
                }
                ui.separator();
                let info_col  = egui::Color32::from_rgb(120, 190, 255);
                let warn_col  = egui::Color32::from_rgb(255, 215, 60);
                let error_col = egui::Color32::from_rgb(255, 100, 100);
                let r = ui.selectable_label(show_info,
                    egui::RichText::new(format!("ℹ  {}", info_cnt)).color(info_col).size(11.5));
                if r.clicked() { action = "info".to_string(); }
                let r = ui.selectable_label(show_warn,
                    egui::RichText::new(format!("⚠  {}", warn_cnt)).color(warn_col).size(11.5));
                if r.clicked() { action = "warn".to_string(); }
                let r = ui.selectable_label(show_error,
                    egui::RichText::new(format!("✕  {}", error_cnt)).color(error_col).size(11.5));
                if r.clicked() { action = "error".to_string(); }
                ui.separator();
                let r = ui.selectable_label(collapse,
                    egui::RichText::new("Collapse").size(11.5));
                if r.clicked() { action = "collapse".to_string(); }
            });
            action
        }).unwrap_or_default()
    }).build()?;

    // A single-line search bar without a visible label.
    // Returns the current text value every frame (modified or not).
    m.function("search_bar", |placeholder: Ref<str>, current: Ref<str>| -> String {
        with_ui(|ui| {
            let mut v = current.as_ref().to_string();
            let changed = ui.horizontal(|ui| {
                ui.add(crate::icons::img("search", 13.0, ui.visuals().weak_text_color()));
                let hint = egui::RichText::new(placeholder.as_ref())
                    .color(ui.visuals().weak_text_color());
                ui.add(
                    egui::TextEdit::singleline(&mut v)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                ).changed()
            }).inner;
            let _ = changed;
            v
        }).unwrap_or_else(|| current.as_ref().to_string())
    }).build()?;

    // Renders the scrollable log entry list inside a real egui ScrollArea.
    // entries: Vec of [level_str, message, count_str, time_str, global_idx_str]
    // Returns the global_idx of the clicked row, or -1 if none clicked.
    m.function("console_log_list",
        |entries: Vec<Vec<String>>, selected_idx: i64, auto_scroll: bool, height: f64|
        -> i64
    {
        with_ui(|ui| {
            let mut clicked: i64 = -1;
            let row_h = 18.0f32;
            egui::ScrollArea::vertical()
                .id_salt("console_log_list")
                .max_height(height as f32)
                .auto_shrink([false, false])
                .stick_to_bottom(auto_scroll)
                .show_rows(ui, row_h, entries.len(), |ui, row_range| {
                    for i in row_range {
                        let row = &entries[i];
                        let level   = row.get(0).map(|s| s.as_str()).unwrap_or("info");
                        let message = row.get(1).map(|s| s.as_str()).unwrap_or("");
                        let count   = row.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        let time    = row.get(3).map(|s| s.as_str()).unwrap_or("");
                        let g_idx   = row.get(4).and_then(|s| s.parse::<i64>().ok()).unwrap_or(i as i64);
                        let is_sel  = g_idx == selected_idx;

                        let (icon, icon_col) = match level {
                            "error" => ("✕", egui::Color32::from_rgb(255, 100, 100)),
                            "warn"  => ("⚠", egui::Color32::from_rgb(255, 215, 60)),
                            _       => ("ℹ", egui::Color32::from_rgb(120, 190, 255)),
                        };
                        let text_col = match level {
                            "error" => egui::Color32::from_rgb(255, 130, 130),
                            "warn"  => egui::Color32::from_rgb(255, 230, 130),
                            _       => ui.visuals().text_color(),
                        };

                        // Alternating row background
                        let bg = if i % 2 == 0 {
                            egui::Color32::from_rgba_premultiplied(255, 255, 255, 6)
                        } else {
                            egui::Color32::TRANSPARENT
                        };

                        let resp = ui.horizontal(|ui| {
                            let avail = ui.available_width();
                            let time_w = 68.0f32;
                            let cnt_w  = if count > 1 { 30.0f32 } else { 0.0 };

                            // Background highlight for selected or alternating
                            if is_sel {
                                let r = ui.max_rect();
                                ui.painter().rect_filled(r, 0.0,
                                    egui::Color32::from_rgba_premultiplied(100, 160, 255, 40));
                            } else if bg != egui::Color32::TRANSPARENT {
                                let r = ui.max_rect();
                                ui.painter().rect_filled(r, 0.0, bg);
                            }

                            // Icon
                            ui.colored_label(icon_col,
                                egui::RichText::new(icon).size(11.0).monospace());

                            // Message text — truncated to fit
                            let msg_w = (avail - time_w - cnt_w - 20.0).max(50.0);
                            let truncated = if message.len() > 160 {
                                format!("{}…", &message[..157])
                            } else {
                                message.to_string()
                            };
                            ui.add_sized(
                                [msg_w, row_h],
                                egui::Label::new(
                                    egui::RichText::new(&truncated).color(text_col).size(11.0)
                                ).truncate(),
                            );

                            // Count badge
                            if count > 1 {
                                ui.add_sized(
                                    [cnt_w, row_h],
                                    egui::Label::new(
                                        egui::RichText::new(format!("×{}", count))
                                            .color(egui::Color32::from_rgb(180, 180, 180))
                                            .size(10.0)
                                    ),
                                );
                            }

                            // Timestamp (right-aligned)
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add(egui::Label::new(
                                    egui::RichText::new(time)
                                        .color(egui::Color32::from_rgb(130, 130, 150))
                                        .size(10.0)
                                        .monospace()
                                ));
                            });
                        });

                        // Click-to-select
                        if resp.response.interact(egui::Sense::click()).clicked() {
                            clicked = g_idx;
                        }
                    }
                });
            clicked
        }).unwrap_or(-1)
    }).build()?;

    // ── Texture preview widget (Phase 5) ──────────────────────────────────────
    //
    // texture_preview(path, display_size) — loads the image at `path` (relative
    // to project root, under assets/), caches the egui TextureHandle, and renders
    // a centred image at `display_size` × `display_size` pixels.
    // Silently no-ops for unknown / unloadable files.
    m.function("texture_preview", |path: Ref<str>, display_size: f64| {
        let path_s = path.as_ref().to_string();
        // Resolve absolute path via world_module::get_project_root.
        let abs_path = {
            let root = crate::rune_bindings::world_module::get_project_root();
            root.join("assets").join(&path_s)
        };

        with_ui(|ui| {
            // Get or load the TextureHandle.
            let has_cached = TEXTURE_CACHE.with(|c| c.borrow().contains_key(&path_s));
            if !has_cached {
                if let Ok(img) = image::open(&abs_path) {
                    let rgba   = img.to_rgba8();
                    let size   = [rgba.width() as usize, rgba.height() as usize];
                    let pixels = rgba.into_raw();
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                    let handle = ui.ctx().load_texture(
                        path_s.clone(),
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );
                    TEXTURE_CACHE.with(|c| { c.borrow_mut().insert(path_s.clone(), handle); });
                }
            }

            TEXTURE_CACHE.with(|c| {
                if let Some(handle) = c.borrow().get(&path_s) {
                    let sz = display_size as f32;
                    // Centred image in available width.
                    let avail_w = ui.available_width();
                    if avail_w > sz {
                        ui.add_space((avail_w - sz) * 0.5);
                    }
                    ui.add(
                        egui::Image::new(handle)
                            .fit_to_exact_size(egui::vec2(sz, sz))
                            .corner_radius(4.0),
                    );
                }
            });
        });
    }).build()?;

    // asset_preview_ready(path) → bool — true once the texture has been loaded.
    // Call this from Rune to avoid showing a flash of missing content.
    m.function("asset_preview_ready", |path: Ref<str>| -> bool {
        TEXTURE_CACHE.with(|c| c.borrow().contains_key(path.as_ref()))
    }).build()?;

    // ── Asset browser compound widgets (Phase 2) ──────────────────────────────

    // asset_folder_tree(dirs, active_dir) → String
    // Renders a left-panel folder tree. Each entry shows a folder icon + name.
    // The active_dir row is highlighted. Returns the dir name that was clicked,
    // or "" if nothing was clicked. Special "(root)" entry is always first.
    m.function("asset_folder_tree", |dirs: Vec<String>, active_dir: Ref<str>| -> String {
        with_ui(|ui| {
            use std::cell::RefCell as LC;
            let active = active_dir.as_ref().to_string();
            // Seed with active so that "no click" returns active_dir unchanged
            // (prevents the caller's `if clicked != active_dir` from falsely
            // resetting to root every frame).
            let clicked: LC<String> = LC::new(active.clone());
            let tint_normal = egui::Color32::from_rgb(160, 160, 175);
            let tint_active = egui::Color32::from_rgb(100, 180, 255);
            let bg_active   = egui::Color32::from_rgba_unmultiplied(100, 180, 255, 30);

            egui::ScrollArea::vertical()
                .id_salt("asset_folder_tree")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(130.0);

                    // "(root)" entry
                    {
                        let is_active = active.is_empty() || active == "(root)";
                        let row_resp  = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let tint = if is_active { tint_active } else { tint_normal };
                            if let Some(bytes) = crate::icons::icon_bytes("folder") {
                                let uri = crate::icons::icon_uri("folder");
                                ui.add(egui::Image::from_bytes(uri, bytes)
                                    .fit_to_exact_size(egui::vec2(14.0, 14.0)).tint(tint));
                            }
                            let label_color = if is_active {
                                egui::Color32::from_rgb(200, 230, 255)
                            } else {
                                egui::Color32::from_rgb(200, 200, 210)
                            };
                            ui.add(egui::Label::new(
                                egui::RichText::new("(root)").color(label_color).size(12.0)
                            ).sense(egui::Sense::click()))
                        });
                        if is_active {
                            ui.painter().rect_filled(
                                row_resp.response.rect, 2.0, bg_active);
                        }
                        if row_resp.response.interact(egui::Sense::click()).clicked()
                            || row_resp.inner.clicked()
                        {
                            *clicked.borrow_mut() = String::new();
                        }
                    }

                    // Subdirectory entries
                    for dir in &dirs {
                        let is_active = dir == &active;
                        let row_resp = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let tint = if is_active { tint_active } else { tint_normal };
                            if let Some(bytes) = crate::icons::icon_bytes("folder") {
                                let uri = crate::icons::icon_uri("folder");
                                ui.add(egui::Image::from_bytes(uri, bytes)
                                    .fit_to_exact_size(egui::vec2(14.0, 14.0)).tint(tint));
                            }
                            let label_color = if is_active {
                                egui::Color32::from_rgb(200, 230, 255)
                            } else {
                                egui::Color32::from_rgb(200, 200, 210)
                            };
                            // Show only last segment for nested dirs
                            let display = dir.rsplit('/').next().unwrap_or(dir.as_str());
                            ui.add(egui::Label::new(
                                egui::RichText::new(display).color(label_color).size(12.0)
                            ).sense(egui::Sense::click()))
                        });
                        if is_active {
                            ui.painter().rect_filled(
                                row_resp.response.rect, 2.0, bg_active);
                        }
                        if row_resp.response.interact(egui::Sense::click()).clicked()
                            || row_resp.inner.clicked()
                        {
                            *clicked.borrow_mut() = dir.clone();
                        }
                    }
                });
            clicked.into_inner()
        }).unwrap_or_default()
    }).build()?;

    // asset_grid(paths, selected_path, zoom) → Vec<String>
    // Renders a tile grid (wrapping) of asset tiles.
    // Each tile: type-colored icon square + truncated filename.
    // Returns [action, path] where action = "select" | "open" | "ctx:<item>" | "".
    m.function("asset_grid", |paths: Vec<String>, selected_path: Ref<str>, zoom: f64| -> Vec<String> {
        with_ui(|ui| {
            use std::cell::RefCell as LC;
            let sel     = selected_path.as_ref().to_string();
            let result: LC<Vec<String>> = LC::new(vec!["".to_string(), String::new()]);
            let tile_sz = (64.0 * zoom as f32).clamp(32.0, 128.0);
            let font_sz = (tile_sz * 0.165).clamp(9.0, 14.0);

            // Type → (icon_name, r, g, b)
            let type_color = |t: &str| -> (&'static str, u8, u8, u8) {
                match t {
                    "texture"  => ("image",     160,  90, 210),
                    "model"    => ("box",        180, 120,  60),
                    "audio"    => ("music",       60, 180, 120),
                    "script"   => ("code",        80, 150, 220),
                    "shader"   => ("layers",      60, 190, 190),
                    "scene"    => ("film",        200, 160,  50),
                    "material" => ("droplet",     100, 160, 220),
                    "prefab"   => ("package",     210, 130,  60),
                    "json"     => ("file-text",   160, 160, 160),
                    _          => ("file",        140, 140, 155),
                }
            };

            // Context menu items per type
            let ctx_items = |t: &str| -> Vec<&'static str> {
                match t {
                    "scene"    => vec!["Open Scene", "Rename", "Duplicate", "Copy Path", "Delete"],
                    "script"   => vec!["Attach to Selected", "Rename", "Duplicate", "Copy Path", "Delete"],
                    "material" => vec!["Edit Material", "Assign to Selected", "Rename", "Duplicate", "Copy Path", "Delete"],
                    _          => vec!["Rename", "Duplicate", "Copy Path", "Delete"],
                }
            };

            egui::ScrollArea::vertical()
                .id_salt("asset_grid_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let spacing = 8.0_f32;
                    ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
                    ui.with_layout(
                        egui::Layout::left_to_right(egui::Align::TOP).with_main_wrap(true),
                        |ui| {
                            for path in &paths {
                                // determine type info
                                let filename = path.rsplit('/').next().unwrap_or(path.as_str());
                                let ext = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                                let t = match ext.as_str() {
                                    "png"|"jpg"|"jpeg"|"webp"|"bmp"|"tga"|"hdr"|"exr"|"ktx"|"dds" => "texture",
                                    "glb"|"gltf"|"obj"|"fbx" => "model",
                                    "wav"|"ogg"|"mp3"|"flac"|"aac" => "audio",
                                    "rn"|"js"|"lua"|"py" => "script",
                                    "wgsl"|"vert"|"frag"|"glsl"|"hlsl" => "shader",
                                    "scene" => "scene",
                                    "fluxmat" => "material",
                                    "prefab"|"fluxprefab" => "prefab",
                                    "json" => "json",
                                    _ => "unknown",
                                };
                                let (icon_name, ir, ig, ib) = type_color(t);
                                let icon_tint = egui::Color32::from_rgb(ir, ig, ib);
                                let icon_bg   = egui::Color32::from_rgba_unmultiplied(ir, ig, ib, 28);
                                let is_sel    = path == &sel;

                                // ── Wrap tile in dnd_drag_source ─────────────
                                let tile_total = egui::vec2(tile_sz, tile_sz + font_sz + 4.0);
                                let drag_tile_id = egui::Id::new(("tile_dnd", path.as_str()));
                                let dnd_inner = ui.dnd_drag_source(
                                    drag_tile_id,
                                    DndPayload::Asset { path: path.clone(), asset_type: t.to_string() },
                                    |ui| {
                                        let (tile_rect, tile_resp) = ui.allocate_exact_size(
                                            tile_total, egui::Sense::click());
                                        let painter = ui.painter_at(tile_rect);

                                        // Background
                                        let bg = if is_sel {
                                            egui::Color32::from_rgb(50, 80, 120)
                                        } else if tile_resp.hovered() {
                                            egui::Color32::from_rgb(55, 55, 65)
                                        } else {
                                            egui::Color32::from_rgb(42, 42, 50)
                                        };
                                        painter.rect_filled(
                                            egui::Rect::from_min_size(tile_rect.min, egui::vec2(tile_sz, tile_sz)),
                                            4.0, bg);

                                        // Icon background square (tinted)
                                        let icon_margin = tile_sz * 0.15;
                                        let icon_area = egui::Rect::from_min_size(
                                            tile_rect.min + egui::vec2(icon_margin, icon_margin),
                                            egui::vec2(tile_sz - icon_margin * 2.0, tile_sz - icon_margin * 2.0));
                                        painter.rect_filled(icon_area, 3.0, icon_bg);

                                        // Selection / hover border
                                        if is_sel {
                                            painter.rect_stroke(
                                                egui::Rect::from_min_size(tile_rect.min, egui::vec2(tile_sz, tile_sz)),
                                                4.0,
                                                egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 150, 255)), egui::StrokeKind::Outside);
                                        }

                                        // SVG icon centered in icon_area
                                        if let Some(bytes) = crate::icons::icon_bytes(icon_name) {
                                            let uri = crate::icons::icon_uri(icon_name);
                                            let inner_size = icon_area.size() * 0.55;
                                            let inner_rect = egui::Rect::from_center_size(
                                                icon_area.center(), inner_size);
                                            let img = egui::Image::from_bytes(uri, bytes)
                                                .fit_to_exact_size(inner_size)
                                                .tint(icon_tint);
                                            img.paint_at(ui, inner_rect);
                                        }

                                        // Filename text below tile
                                        let label_rect = egui::Rect::from_min_size(
                                            tile_rect.min + egui::vec2(0.0, tile_sz + 2.0),
                                            egui::vec2(tile_sz, font_sz + 2.0));
                                        let stem = filename.rfind('.').map(|i| &filename[..i]).unwrap_or(filename);
                                        let display_name = if stem.len() > 12 {
                                            format!("{}…", &stem[..11])
                                        } else {
                                            stem.to_string()
                                        };
                                        painter.text(
                                            label_rect.center_top(),
                                            egui::Align2::CENTER_TOP,
                                            &display_name,
                                            egui::FontId::proportional(font_sz),
                                            if is_sel {
                                                egui::Color32::from_rgb(180, 210, 255)
                                            } else {
                                                egui::Color32::from_rgb(190, 190, 200)
                                            });

                                        tile_resp
                                    });
                                let tile_resp = dnd_inner.inner;

                                // Click / double-click
                                if tile_resp.clicked() {
                                    *result.borrow_mut() = vec!["select".to_string(), path.clone()];
                                }
                                if tile_resp.double_clicked() {
                                    *result.borrow_mut() = vec!["open".to_string(), path.clone()];
                                }

                                // Context menu
                                tile_resp.context_menu(|ui| {
                                    for item in ctx_items(t) {
                                        if ui.button(item).clicked() {
                                            *result.borrow_mut() = vec![
                                                format!("ctx:{item}"), path.clone()];
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        });
                });
            result.into_inner()
        }).unwrap_or_else(|| vec!["".to_string(), String::new()])
    }).build()?;

    // Readonly multiline text area for the console detail panel.
    m.function("text_readonly", |text: Ref<str>, height: f64| {
        with_ui(|ui| {
            egui::ScrollArea::vertical()
                .id_salt("console_detail")
                .max_height(height as f32)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut text.as_ref().to_string())
                            .desired_width(f32::INFINITY)
                            .desired_rows(3)
                            .interactive(false)
                            .font(egui::TextStyle::Monospace),
                    );
                });
        });
    }).build()?;

    // ── Phase A3: read-only DnD state queries ─────────────────────────────────

    // dnd_is_dragging() → bool
    m.function("dnd_is_dragging", || -> bool {
        CURRENT_CTX.with(|c| {
            c.borrow().as_ref()
                .map(|ctx| egui::DragAndDrop::payload::<DndPayload>(ctx).is_some())
                .unwrap_or(false)
        })
    }).build()?;

    // dnd_drag_type() → "asset" | "entity" | ""
    m.function("dnd_drag_type", || -> String {
        CURRENT_CTX.with(|c| {
            c.borrow().as_ref()
                .and_then(|ctx| egui::DragAndDrop::payload::<DndPayload>(ctx))
                .map(|p| match p.as_ref() {
                    DndPayload::Asset  { .. } => "asset".to_string(),
                    DndPayload::Entity { .. } => "entity".to_string(),
                })
                .unwrap_or_default()
        })
    }).build()?;

    // dnd_drag_asset_path() → String (empty if not dragging an asset)
    m.function("dnd_drag_asset_path", || -> String {
        CURRENT_CTX.with(|c| {
            c.borrow().as_ref()
                .and_then(|ctx| egui::DragAndDrop::payload::<DndPayload>(ctx))
                .and_then(|p| if let DndPayload::Asset { path, .. } = p.as_ref() { Some(path.clone()) } else { None })
                .unwrap_or_default()
        })
    }).build()?;

    // dnd_drag_asset_type() → String (empty if not dragging an asset)
    m.function("dnd_drag_asset_type", || -> String {
        CURRENT_CTX.with(|c| {
            c.borrow().as_ref()
                .and_then(|ctx| egui::DragAndDrop::payload::<DndPayload>(ctx))
                .and_then(|p| if let DndPayload::Asset { asset_type, .. } = p.as_ref() { Some(asset_type.clone()) } else { None })
                .unwrap_or_default()
        })
    }).build()?;

    // dnd_drag_entity_id() → i64 (-1 if not dragging an entity)
    m.function("dnd_drag_entity_id", || -> i64 {
        CURRENT_CTX.with(|c| {
            c.borrow().as_ref()
                .and_then(|ctx| egui::DragAndDrop::payload::<DndPayload>(ctx))
                .and_then(|p| if let DndPayload::Entity { id } = p.as_ref() { Some(*id) } else { None })
                .unwrap_or(-1)
        })
    }).build()?;

    // ── Phase B2: compound widgets ─────────────────────────────────────────────

    // breadcrumb_bar(path, can_back, can_fwd) → String
    // Renders [←][→][↑] nav buttons + clickable breadcrumb path.
    // Returns "back" | "forward" | "up" | "dir:<path>" | "".
    m.function("breadcrumb_bar", |path: Ref<str>, can_back: bool, can_fwd: bool| -> String {
        with_ui(|ui| {
            let path_str = path.as_ref().to_string();
            let action: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;

                let nav_tint = |active: bool| if active {
                    egui::Color32::from_rgb(180, 180, 200)
                } else {
                    egui::Color32::from_rgb(70, 70, 82)
                };

                if ui.add(crate::icons::img("arrow-left",  14.0, nav_tint(can_back))
                    .sense(egui::Sense::click())).on_hover_text("Back").clicked() && can_back
                {
                    *action.borrow_mut() = "back".to_string();
                }
                if ui.add(crate::icons::img("arrow-right", 14.0, nav_tint(can_fwd))
                    .sense(egui::Sense::click())).on_hover_text("Forward").clicked() && can_fwd
                {
                    *action.borrow_mut() = "forward".to_string();
                }
                let up_ok = !path_str.is_empty();
                if ui.add(crate::icons::img("arrow-up", 14.0, nav_tint(up_ok))
                    .sense(egui::Sense::click())).on_hover_text("Go up one level").clicked() && up_ok
                {
                    *action.borrow_mut() = "up".to_string();
                }

                ui.add_space(6.0);

                // Root "assets" link
                let is_root   = path_str.is_empty();
                let root_col  = if is_root { egui::Color32::from_rgb(100, 180, 255) }
                                else       { egui::Color32::from_rgb(150, 150, 170) };
                if ui.add(egui::Label::new(
                    egui::RichText::new("assets").color(root_col).size(12.0))
                    .sense(egui::Sense::click())).clicked()
                    && action.borrow().is_empty()
                {
                    *action.borrow_mut() = "dir:".to_string();
                }

                // Segment links
                let mut accumulated = String::new();
                for seg in path_str.split('/').filter(|s| !s.is_empty()) {
                    ui.label(egui::RichText::new("›").color(egui::Color32::from_rgb(90, 90, 105)).size(12.0));
                    if !accumulated.is_empty() { accumulated.push('/'); }
                    accumulated.push_str(seg);
                    let seg_path = accumulated.clone();
                    let is_last  = seg_path == path_str;
                    let seg_col  = if is_last { egui::Color32::from_rgb(100, 180, 255) }
                                   else       { egui::Color32::from_rgb(150, 150, 170) };
                    if ui.add(egui::Label::new(
                        egui::RichText::new(seg).color(seg_col).size(12.0))
                        .sense(egui::Sense::click())).clicked()
                        && action.borrow().is_empty()
                    {
                        *action.borrow_mut() = format!("dir:{seg_path}");
                    }
                }
            });

            action.into_inner()
        }).unwrap_or_default()
    }).build()?;

    // asset_folder_tree_v2(dirs, active_dir) → String
    // Nested folder sidebar using CollapsingState for hierarchy, DnD drop zones per folder.
    // Returns the newly clicked dir path, or active_dir if nothing was clicked.
    m.function("asset_folder_tree_v2", |dirs: Vec<String>, active_dir: Ref<str>| -> String {
        with_ui(|ui| {
            use std::cell::RefCell as LC;
            let active = active_dir.as_ref().to_string();
            let clicked: LC<String> = LC::new(active.clone());

            let tint_n = egui::Color32::from_rgb(160, 160, 175);
            let tint_a = egui::Color32::from_rgb(100, 180, 255);
            let bg_a   = egui::Color32::from_rgba_unmultiplied(100, 180, 255, 30);

            egui::ScrollArea::vertical()
                .id_salt("asset_folder_tree_v2")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(130.0);

                    // ── (root) entry ─────────────────────────────────────────
                    {
                        let is_active = active.is_empty();
                        let row = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let t = if is_active { tint_a } else { tint_n };
                            if let Some(b) = crate::icons::icon_bytes("folder") {
                                ui.add(egui::Image::from_bytes(crate::icons::icon_uri("folder"), b)
                                    .fit_to_exact_size(egui::vec2(14.0, 14.0)).tint(t));
                            }
                            let lc = if is_active { egui::Color32::from_rgb(200, 230, 255) }
                                     else         { egui::Color32::from_rgb(200, 200, 210) };
                            ui.add(egui::Label::new(
                                egui::RichText::new("(root)").color(lc).size(12.0))
                                .sense(egui::Sense::click()))
                        });
                        if is_active {
                            ui.painter().rect_filled(row.response.rect, 2.0, bg_a);
                        }
                        // DnD drop zone on root
                        let drop_r = ui.interact(row.response.rect,
                            egui::Id::new("folder_drop_root"), egui::Sense::hover());
                        if drop_r.dnd_hover_payload::<DndPayload>().is_some() {
                            ui.painter().rect_stroke(row.response.rect, 2.0,
                                egui::Stroke::new(1.5, egui::Color32::from_rgb(60, 200, 100)), egui::StrokeKind::Outside);
                        }
                        if let Some(p) = drop_r.dnd_release_payload::<DndPayload>() {
                            if let DndPayload::Asset { path, .. } = p.as_ref() {
                                *clicked.borrow_mut() = format!("drop_move::{path}");
                            }
                        }
                        if row.response.interact(egui::Sense::click()).clicked() || row.inner.clicked() {
                            *clicked.borrow_mut() = String::new();
                        }
                    }

                    // ── Build depth-first ordered list ───────────────────────
                    let mut ordered: Vec<(String, usize)> = Vec::new();
                    dir_depth_first(&dirs, "", 0, &mut ordered);

                    // ── Render each folder ────────────────────────────────────
                    for (dir, depth) in &ordered {
                        let has_children = dirs.iter().any(|d| {
                            d.rfind('/').map(|i| &d[..i]).unwrap_or("") == dir.as_str()
                        });
                        let is_active = dir == &active;
                        let indent = *depth as f32 * 14.0;
                        let display = dir.rsplit('/').next().unwrap_or(dir.as_str());

                        let tint = if is_active { tint_a } else { tint_n };

                        let row = if has_children {
                            let state_id = ui.make_persistent_id(("dir_tree", dir.as_str()));
                            let _state = egui::collapsing_header::CollapsingState
                                ::load_with_default_open(ui.ctx(), state_id, false);
                            // Render as plain indent row; CollapsingState manages open/closed
                            // via a small ► toggle implicitly in the allocate space.
                            // For simplicity we use a plain selectable row + indent:
                            ui.scope(|ui| {
                                ui.add_space(indent);
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let lc = if is_active { egui::Color32::from_rgb(200, 230, 255) }
                                         else         { egui::Color32::from_rgb(200, 200, 210) };
                                let icon = if has_children { "folder" } else { "folder" };
                                let h_resp = ui.horizontal(|ui| {
                                    if let Some(b) = crate::icons::icon_bytes(icon) {
                                        ui.add(egui::Image::from_bytes(crate::icons::icon_uri(icon), b)
                                            .fit_to_exact_size(egui::vec2(14.0, 14.0)).tint(tint));
                                    }
                                    ui.add(egui::Label::new(
                                        egui::RichText::new(display).color(lc).size(12.0))
                                        .sense(egui::Sense::click()))
                                });
                                h_resp
                            }).inner
                        } else {
                            ui.scope(|ui| {
                                ui.add_space(indent);
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let lc = if is_active { egui::Color32::from_rgb(200, 230, 255) }
                                         else         { egui::Color32::from_rgb(200, 200, 210) };
                                ui.horizontal(|ui| {
                                    if let Some(b) = crate::icons::icon_bytes("folder") {
                                        ui.add(egui::Image::from_bytes(crate::icons::icon_uri("folder"), b)
                                            .fit_to_exact_size(egui::vec2(14.0, 14.0)).tint(tint));
                                    }
                                    ui.add(egui::Label::new(
                                        egui::RichText::new(display).color(lc).size(12.0))
                                        .sense(egui::Sense::click()))
                                })
                            }).inner
                        };

                        if is_active {
                            ui.painter().rect_filled(row.response.rect, 2.0, bg_a);
                        }

                        // DnD drop zone
                        let drop_id = egui::Id::new(("folder_drop", dir.as_str()));
                        let drop_r  = ui.interact(row.response.rect, drop_id, egui::Sense::hover());
                        if drop_r.dnd_hover_payload::<DndPayload>().is_some() {
                            ui.painter().rect_stroke(row.response.rect, 2.0,
                                egui::Stroke::new(1.5, egui::Color32::from_rgb(60, 200, 100)), egui::StrokeKind::Outside);
                        }
                        if let Some(p) = drop_r.dnd_release_payload::<DndPayload>() {
                            if let DndPayload::Asset { path, .. } = p.as_ref() {
                                *clicked.borrow_mut() = format!("drop_move:{dir}:{path}");
                            }
                        }

                        if row.response.interact(egui::Sense::click()).clicked() || row.inner.clicked() {
                            *clicked.borrow_mut() = dir.clone();
                        }
                    }
                });

            clicked.into_inner()
        }).unwrap_or_default()
    }).build()?;

    // asset_grid_v2(paths, selected_path, zoom) → Vec<String>
    // Enhanced tile grid: DnD drag sources + richer context menu + keyboard shortcuts.
    // Returns [action, path] same as asset_grid.
    m.function("asset_grid_v2", |paths: Vec<String>, selected_path: Ref<str>, zoom: f64| -> Vec<String> {
        with_ui(|ui| {
            use std::cell::RefCell as LC;
            let sel      = selected_path.as_ref().to_string();
            let result: LC<Vec<String>> = LC::new(vec!["".to_string(), String::new()]);
            let tile_sz  = (64.0 * zoom as f32).clamp(32.0, 128.0);
            let font_sz  = (tile_sz * 0.165).clamp(9.0, 14.0);

            let type_color = |t: &str| -> (&'static str, u8, u8, u8) {
                match t {
                    "texture"  => ("image",      160,  90, 210),
                    "model"    => ("box",         180, 120,  60),
                    "audio"    => ("music",        60, 180, 120),
                    "script"   => ("code",         80, 150, 220),
                    "shader"   => ("layers",       60, 190, 190),
                    "scene"    => ("film",         200, 160,  50),
                    "material" => ("droplet",      100, 160, 220),
                    "prefab"   => ("package",      210, 130,  60),
                    "json"     => ("file-text",    160, 160, 160),
                    _          => ("file",         140, 140, 155),
                }
            };

            let ctx_items = |t: &str| -> Vec<&'static str> {
                match t {
                    "scene"    => vec!["Open Scene", "Rename", "Duplicate", "Copy Path", "Show in Explorer", "Delete"],
                    "script"   => vec!["Attach to Selected", "Rename", "Duplicate", "Copy Path", "Show in Explorer", "Delete"],
                    "material" => vec!["Edit Material", "Assign to Selected", "Rename", "Duplicate", "Copy Path", "Show in Explorer", "Delete"],
                    "prefab"   => vec!["Instantiate", "Rename", "Duplicate", "Copy Path", "Show in Explorer", "Delete"],
                    _          => vec!["Rename", "Duplicate", "Copy Path", "Show in Explorer", "Delete"],
                }
            };

            // ── Keyboard shortcuts when a tile is selected ────────────────────
            with_ui(|ui| {
                let has_sel = !sel.is_empty();
                if has_sel {
                    let del = ui.input(|i| i.key_pressed(egui::Key::Delete));
                    let f2  = ui.input(|i| i.key_pressed(egui::Key::F2));
                    let ctrl_d = ui.input(|i| i.key_pressed(egui::Key::D) && i.modifiers.ctrl);
                    let ctrl_c = ui.input(|i| i.key_pressed(egui::Key::C) && i.modifiers.ctrl);
                    if del  { *result.borrow_mut() = vec!["ctx:Delete".to_string(),    sel.clone()]; }
                    if f2   { *result.borrow_mut() = vec!["ctx:Rename".to_string(),    sel.clone()]; }
                    if ctrl_d { *result.borrow_mut() = vec!["ctx:Duplicate".to_string(), sel.clone()]; }
                    if ctrl_c { *result.borrow_mut() = vec!["ctx:Copy Path".to_string(), sel.clone()]; }
                }
            });

            egui::ScrollArea::vertical()
                .id_salt("asset_grid_v2_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let spacing = 8.0_f32;
                    ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
                    ui.with_layout(
                        egui::Layout::left_to_right(egui::Align::TOP).with_main_wrap(true),
                        |ui| {
                            for path in &paths {
                                if !result.borrow()[0].is_empty() { break; }

                                let filename = path.rsplit('/').next().unwrap_or(path.as_str());
                                let ext = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                                let t = match ext.as_str() {
                                    "png"|"jpg"|"jpeg"|"webp"|"bmp"|"tga"|"hdr"|"exr" => "texture",
                                    "glb"|"gltf"|"obj"|"fbx" => "model",
                                    "wav"|"ogg"|"mp3"|"flac" => "audio",
                                    "rn"|"js"|"lua" => "script",
                                    "wgsl"|"glsl"|"hlsl" => "shader",
                                    "scene" => "scene",
                                    "fluxmat" => "material",
                                    "prefab"|"fluxprefab" => "prefab",
                                    _ => "unknown",
                                };
                                let (icon_name, ir, ig, ib) = type_color(t);
                                let icon_tint = egui::Color32::from_rgb(ir, ig, ib);
                                let icon_bg   = egui::Color32::from_rgba_unmultiplied(ir, ig, ib, 28);
                                let is_sel    = path == &sel;

                                let tile_total = egui::vec2(tile_sz, tile_sz + font_sz + 4.0);
                                let drag_id    = egui::Id::new(("v2_tile_dnd", path.as_str()));
                                let dnd = ui.dnd_drag_source(
                                    drag_id,
                                    DndPayload::Asset { path: path.clone(), asset_type: t.to_string() },
                                    |ui| {
                                        let (tile_rect, tile_resp) = ui.allocate_exact_size(
                                            tile_total, egui::Sense::click());
                                        let painter = ui.painter_at(tile_rect);

                                        let bg = if is_sel { egui::Color32::from_rgb(50, 80, 120) }
                                                 else if tile_resp.hovered() { egui::Color32::from_rgb(55, 55, 65) }
                                                 else { egui::Color32::from_rgb(42, 42, 50) };
                                        painter.rect_filled(
                                            egui::Rect::from_min_size(tile_rect.min, egui::vec2(tile_sz, tile_sz)),
                                            4.0, bg);

                                        let icon_margin = tile_sz * 0.15;
                                        let icon_area = egui::Rect::from_min_size(
                                            tile_rect.min + egui::vec2(icon_margin, icon_margin),
                                            egui::vec2(tile_sz - icon_margin*2.0, tile_sz - icon_margin*2.0));
                                        painter.rect_filled(icon_area, 3.0, icon_bg);

                                        if is_sel {
                                            painter.rect_stroke(
                                                egui::Rect::from_min_size(tile_rect.min, egui::vec2(tile_sz, tile_sz)),
                                                4.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 150, 255)), egui::StrokeKind::Outside);
                                        }

                                        if let Some(bytes) = crate::icons::icon_bytes(icon_name) {
                                            let inner_sz = icon_area.size() * 0.55;
                                            egui::Image::from_bytes(crate::icons::icon_uri(icon_name), bytes)
                                                .fit_to_exact_size(inner_sz).tint(icon_tint)
                                                .paint_at(ui, egui::Rect::from_center_size(icon_area.center(), inner_sz));
                                        }

                                        let stem = filename.rfind('.').map(|i| &filename[..i]).unwrap_or(filename);
                                        let dname = if stem.len() > 12 { format!("{}…", &stem[..11]) } else { stem.to_string() };
                                        painter.text(
                                            tile_rect.min + egui::vec2(tile_sz / 2.0, tile_sz + 2.0),
                                            egui::Align2::CENTER_TOP, &dname,
                                            egui::FontId::proportional(font_sz),
                                            if is_sel { egui::Color32::from_rgb(180,210,255) }
                                            else { egui::Color32::from_rgb(190,190,200) });

                                        tile_resp
                                    });
                                let tile_resp = dnd.inner;

                                if tile_resp.clicked() {
                                    *result.borrow_mut() = vec!["select".to_string(), path.clone()];
                                }
                                if tile_resp.double_clicked() {
                                    *result.borrow_mut() = vec!["open".to_string(), path.clone()];
                                }
                                tile_resp.context_menu(|ui| {
                                    for item in ctx_items(t) {
                                        if ui.button(item).clicked() {
                                            *result.borrow_mut() = vec![format!("ctx:{item}"), path.clone()];
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        });
                });

            result.into_inner()
        }).unwrap_or_else(|| vec!["".to_string(), String::new()])
    }).build()?;

    // new_asset_menu_button(dir_prefix) → String
    // Renders a "+" button. On click shows popup with: New Folder / New File / New Scene / New Script / New Material.
    // Returns the chosen item name, or "".
    m.function("new_asset_menu_button", |dir_prefix: Ref<str>| -> String {
        with_ui(|ui| {
            let prefix = dir_prefix.as_ref().to_string();
            let result: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
            let _popup_id = ui.make_persistent_id(("new_asset_popup", prefix.as_str()));

            let btn = ui.add(egui::Button::new(
                egui::RichText::new("+  New").size(12.0).color(egui::Color32::from_rgb(100, 200, 100)))
                .frame(true));
            egui::Popup::from_toggle_button_response(&btn)
                .show(|ui: &mut egui::Ui| {
                    ui.set_min_width(130.0);
                    for item in &["New Folder", "New Script", "New Scene", "New Material", "New File"] {
                        if ui.selectable_label(false, *item).clicked() {
                            *result.borrow_mut() = item.to_string();
                            ui.close();
                        }
                    }
                });

            result.into_inner()
        }).unwrap_or_default()
    }).build()?;

    // ── ltreeview_hierarchy ───────────────────────────────────────────────────
    // ltreeview_hierarchy(nodes: Vec<Vec<String>>) → Vec<String>
    // Renders the entity hierarchy tree using egui_ltreeview.
    // Input: flat list; each row = [id_str, parent_id_str, name, icon, is_selected_str]
    // Returns action strings:
    //   "select:<id>"            — selection changed
    //   "reparent:<target>:<src>"— DnD drop (cycle-guarded)
    //   "unparent:<id>"          — dropped outside tree (make root)
    //   "ctx:<id>:<action>"      — context menu item clicked
    m.function("ltreeview_hierarchy", |nodes: Vec<Vec<String>>| -> Vec<String> {
        // Reset context action for this frame
        LTREE_CTX_ACTION.with(|c| { let mut b = c.borrow_mut(); b.0 = -1; b.1.clear(); });

        struct NodeInfo {
            id: i64,
            parent: i64,
            name: String,
            icon: String,
            is_selected: bool,
        }

        let parsed: Vec<NodeInfo> = nodes.iter()
            .filter_map(|row| {
                if row.len() < 5 { return None; }
                Some(NodeInfo {
                    id:          row[0].parse().ok()?,
                    parent:      row[1].parse().ok()?,
                    name:        row[2].clone(),
                    icon:        row[3].clone(),
                    is_selected: row[4] == "true",
                })
            })
            .collect();

        // Build parent → children index map
        let mut children: std::collections::HashMap<i64, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, n) in parsed.iter().enumerate() {
            children.entry(n.parent).or_default().push(i);
        }

        with_ui(|ui| {
            enum Visit { Enter(usize), CloseDir }

            let (_, actions) =
                egui_ltreeview::TreeView::new(egui::Id::new("ltreeview_hierarchy"))
                    .show(ui, |builder| {
                        let mut stack: Vec<Visit> = Vec::new();
                        if let Some(roots) = children.get(&-1_i64) {
                            for &idx in roots.iter().rev() {
                                stack.push(Visit::Enter(idx));
                            }
                        }
                        while let Some(visit) = stack.pop() {
                            match visit {
                                Visit::CloseDir => { builder.close_dir(); }
                                Visit::Enter(idx) => {
                                    let node   = &parsed[idx];
                                    let has_ch = children.contains_key(&node.id);
                                    let nid    = node.id;
                                    let icon   = node.icon.clone();
                                    let sel    = node.is_selected;
                                    let name   = node.name.clone();

                                    let nb = if has_ch {
                                        egui_ltreeview::NodeBuilder::dir(nid)
                                            .drop_allowed(true)
                                    } else {
                                        egui_ltreeview::NodeBuilder::leaf(nid)
                                            .drop_allowed(true)
                                    }
                                    .icon(move |ui| {
                                        if !icon.is_empty() {
                                            let tint = if sel {
                                                egui::Color32::from_rgb(200, 220, 255)
                                            } else {
                                                egui::Color32::from_rgb(160, 165, 185)
                                            };
                                            ui.add(crate::icons::img(&icon, 13.0, tint));
                                        }
                                    })
                                    .label(name)
                                    .context_menu(move |ui| {
                                        ui.set_min_width(130.0);
                                        for &item in &[
                                            "Rename", "Duplicate",
                                            "Create Prefab", "Delete", "Unparent",
                                        ] {
                                            if ui.button(item).clicked() {
                                                LTREE_CTX_ACTION.with(|c| {
                                                    let mut b = c.borrow_mut();
                                                    b.0 = nid;
                                                    b.1 = item.to_string();
                                                });
                                                ui.close();
                                            }
                                        }
                                    });

                                    builder.node(nb);

                                    if has_ch {
                                        stack.push(Visit::CloseDir);
                                        if let Some(ch) = children.get(&nid) {
                                            for &ci in ch.iter().rev() {
                                                stack.push(Visit::Enter(ci));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    });

            let mut results = Vec::new();

            // Context menu action (may be set from this or previous frame)
            LTREE_CTX_ACTION.with(|c| {
                let b = c.borrow();
                if b.0 >= 0 && !b.1.is_empty() {
                    results.push(format!("ctx:{}:{}", b.0, b.1));
                }
            });

            for action in actions {
                match action {
                    egui_ltreeview::Action::SetSelected(ids) => {
                        if let Some(&id) = ids.first() {
                            results.push(format!("select:{id}"));
                        }
                    }
                    egui_ltreeview::Action::Move(dnd) => {
                        let target = dnd.target;
                        let source = dnd.source.first().copied().unwrap_or(-1);
                        if source >= 0 && target >= 0 && source != target {
                            let is_cycle = crate::rune_bindings::world_module
                                ::check_is_ancestor(source, target);
                            if !is_cycle {
                                results.push(format!("reparent:{target}:{source}"));
                            }
                        }
                    }
                    egui_ltreeview::Action::MoveExternal(dnd) => {
                        if let Some(&src) = dnd.source.first() {
                            if src >= 0 {
                                results.push(format!("unparent:{src}"));
                            }
                        }
                    }
                    _ => {}
                }
            }

            results
        }).unwrap_or_default()
    }).build()?;

    // ── ltreeview_assets ─────────────────────────────────────────────────────
    // ltreeview_assets(dirs: Vec<String>, active_dir: String) → String
    // Renders a folder tree for the asset browser using egui_ltreeview.
    // Returns: "select:<path>" | "drop_move:<dest>:<src>" | ""
    m.function("ltreeview_assets", |dirs: Vec<String>, active_dir: Ref<str>| -> String {
        let _active = active_dir.as_ref().to_string();

        fn path_hash(s: &str) -> i64 {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut h);
            h.finish() as i64
        }

        // Build parent → children map keyed by parent path string
        let mut path_children: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for dir in &dirs {
            let parent = dir.rfind('/').map(|i| dir[..i].to_string())
                            .unwrap_or_default();
            path_children.entry(parent).or_default().push(dir.clone());
        }

        // Rebuild reverse hash map
        LTREE_ASSET_PATHS.with(|m| {
            let mut map = m.borrow_mut();
            map.clear();
            map.insert(0_i64, String::new());
            for dir in &dirs {
                map.insert(path_hash(dir), dir.clone());
            }
        });

        with_ui(|ui| {
            enum Visit { Enter(String), CloseDir }

            let (_, actions) =
                egui_ltreeview::TreeView::new(egui::Id::new("ltreeview_assets"))
                    .show(ui, |builder| {
                        let mut stack: Vec<Visit> = Vec::new();
                        if let Some(roots) = path_children.get("") {
                            let mut sorted = roots.clone();
                            sorted.sort();
                            for dir in sorted.iter().rev() {
                                stack.push(Visit::Enter(dir.clone()));
                            }
                        }
                        while let Some(visit) = stack.pop() {
                            match visit {
                                Visit::CloseDir => { builder.close_dir(); }
                                Visit::Enter(path) => {
                                    let has_ch = path_children.contains_key(&path);
                                    let nid    = path_hash(&path);
                                    let stem   = path.rfind('/')
                                        .map(|i| path[i+1..].to_string())
                                        .unwrap_or_else(|| path.clone());

                                    let nb = if has_ch {
                                        egui_ltreeview::NodeBuilder::dir(nid)
                                            .drop_allowed(true)
                                    } else {
                                        egui_ltreeview::NodeBuilder::leaf(nid)
                                            .drop_allowed(true)
                                    }
                                    .label(stem);

                                    builder.node(nb);

                                    if has_ch {
                                        stack.push(Visit::CloseDir);
                                        if let Some(ch) = path_children.get(&path) {
                                            let mut sorted = ch.clone();
                                            sorted.sort();
                                            for child in sorted.iter().rev() {
                                                stack.push(Visit::Enter(child.clone()));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    });

            let mut result = String::new();
            for action in actions {
                match action {
                    egui_ltreeview::Action::SetSelected(ids) => {
                        if let Some(&id) = ids.first() {
                            let path = LTREE_ASSET_PATHS.with(|m| {
                                m.borrow().get(&id).cloned().unwrap_or_default()
                            });
                            result = format!("select:{path}");
                        }
                    }
                    egui_ltreeview::Action::Move(dnd) => {
                        let dest_path = LTREE_ASSET_PATHS.with(|m| {
                            m.borrow().get(&dnd.target).cloned().unwrap_or_default()
                        });
                        let src_path = LTREE_ASSET_PATHS.with(|m| {
                            m.borrow().get(dnd.source.first().unwrap_or(&0))
                                .cloned().unwrap_or_default()
                        });
                        if !src_path.is_empty() {
                            result = format!("drop_move:{dest_path}:{src_path}");
                        }
                    }
                    _ => {}
                }
            }
            result
        }).unwrap_or_default()
    }).build()?;

    Ok(m)
}
