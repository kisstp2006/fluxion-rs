// ============================================================
// dock.rs — egui_dock layout and Rune panel dispatch
//
// Each dockable tab holds a name + the Rune function path to
// call each frame.  The RuneTabViewer sets the thread-local Ui
// pointer before calling the Rune function, then clears it.
// ============================================================

use egui_dock::{DockArea, DockState, NodeIndex, Style};

use crate::rune_bindings::{set_current_ui, UiContextGuard};
use crate::script_editor;

// ── Tab data ─────────────────────────────────────────────────────────────────

/// Data stored per dockable tab.
///
/// Serialize/Deserialize are required by [`save_dock_layout`] and
/// [`restore_dock_layout`] so the user's panel arrangement survives across
/// editor restarts (A5).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EditorTab {
    /// Display title shown on the tab header.
    pub title: String,
    /// Module path of the Rune function to call: e.g. `"hierarchy::panel"`.
    pub rune_fn: String,
}

impl EditorTab {
    pub fn new(title: impl Into<String>, rune_fn: impl Into<String>) -> Self {
        Self { title: title.into(), rune_fn: rune_fn.into() }
    }
}

// ── Tab viewer ────────────────────────────────────────────────────────────────

pub struct RuneTabViewer<'a> {
    pub vm: &'a mut fluxion_rune_scripting::RuneVm,
}

impl<'a> egui_dock::TabViewer for RuneTabViewer<'a> {
    type Tab = EditorTab;

    fn title(&mut self, tab: &mut EditorTab) -> egui::WidgetText {
        tab.title.as_str().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut EditorTab) {
        // Script Editor tab is rendered directly in Rust (no Rune call needed).
        if tab.rune_fn == "script_editor_panel::panel" {
            if let Ok(mut ed) = script_editor::EDITOR.lock() {
                let save_req = ui.input(|i| {
                    i.modifiers.ctrl && i.key_pressed(egui::Key::S)
                });
                script_editor::render(ui, &mut ed, save_req);
            }
            return;
        }

        // Build the &[&str] path from "module::function".
        let parts: Vec<&str> = tab.rune_fn.split("::").collect();

        // For the viewport tab: hard-clip to max_rect so any overflow is invisible
        // rather than causing a scroll region.  Other panels are left unchanged.
        if tab.rune_fn == "viewport::panel" {
            ui.set_clip_rect(ui.max_rect());
        }

        // Guard clears CURRENT_UI on drop — safe on both normal return and panic.
        let _ui_guard: UiContextGuard = set_current_ui(ui);

        // A8/C1 — panic guard. A bug in any panel script (hierarchy.rn,
        // inspector.rn, etc.) would otherwise unwind through egui_dock's
        // viewer and kill the editor process. Catch the unwind, surface the
        // panic message in-place, and let the user keep working in other tabs.
        let vm_ptr = self.vm as *mut fluxion_rune_scripting::RuneVm;
        let parts_ref = &parts;
        let call_outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: vm_ptr is derived from &mut self.vm and not aliased in
            // this scope; the closure runs to completion before vm_ptr leaves.
            unsafe { (*vm_ptr).call_fn(parts_ref, ()) }
        }));
        match call_outcome {
            Ok(Ok(_)) => { /* normal */ }
            Ok(Err(e)) => {
                let msg = format!("{e:#}");
                log::error!("Rune panel '{}': {msg}", tab.rune_fn);
                ui.colored_label(egui::Color32::RED, format!("⚠ {}: {msg}", tab.rune_fn));
            }
            Err(panic_payload) => {
                let msg = panic_payload.downcast_ref::<String>().map(|s| s.as_str())
                    .or_else(|| panic_payload.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                log::error!("Rune panel '{}' PANICKED: {msg}", tab.rune_fn);
                ui.colored_label(
                    egui::Color32::from_rgb(255, 100, 100),
                    format!("☠ {} panicked: {msg}", tab.rune_fn),
                );
                ui.label(egui::RichText::new(
                    "Other panels keep running. Fix the script and reload (panel scripts hot-reload on save).",
                ).size(11.0).color(egui::Color32::from_gray(180)));
            }
        }
        // _ui_guard drops here (or on early return above), clearing CURRENT_UI.
    }

    fn closeable(&mut self, _tab: &mut EditorTab) -> bool {
        false // panels are not closeable in basic mode
    }

    fn scroll_bars(&self, tab: &EditorTab) -> [bool; 2] {
        if tab.rune_fn == "viewport::panel" {
            [false, false]
        } else {
            [false, true]
        }
    }
}

// ── Default dock layout ───────────────────────────────────────────────────────

/// Build the initial dock layout.
///
/// ```text
/// ┌────────────┬──────────────────────────┬─────────────┐
/// │ Hierarchy  │       Viewport           │  Inspector  │
/// ├────────────┴──────────────────────────┴─────────────┤
/// │  Console                                            │
/// └─────────────────────────────────────────────────────┘
/// ```
pub fn default_dock_state() -> DockState<EditorTab> {
    // Centre column: Viewport
    let mut state = DockState::new(vec![
        EditorTab::new("Viewport", "viewport::panel"),
    ]);

    let surface = state.main_surface_mut();

    // Split left 20% for Hierarchy
    let [hier_node, centre] = surface.split_left(
        NodeIndex::root(),
        0.20,
        vec![EditorTab::new("Hierarchy", "hierarchy::panel")],
    );

    // Split right 22% (of remaining) for Inspector
    let [centre2, _insp] = surface.split_right(
        centre,
        0.78,
        vec![EditorTab::new("Inspector", "inspector::panel")],
    );

    // Split bottom 25% for Console + Assets + Debugger + Script Editor (tabbed together)
    surface.split_below(
        hier_node,
        0.75,
        vec![
            EditorTab::new("Console",       "console::panel"),
            EditorTab::new("Assets",        "assets::panel"),
            EditorTab::new("Debugger",      "debugger::panel"),
            EditorTab::new("Script Editor", "script_editor_panel::panel"),
        ],
    );

    let _ = centre2;

    state
}

// ── Layout persistence (A5) ───────────────────────────────────────────────────

/// Bump this whenever [`EditorTab`] gains/loses fields or [`default_dock_state`]
/// changes meaningfully. Layouts saved with a different version are discarded
/// silently in favour of [`default_dock_state`].
pub const DOCK_LAYOUT_VERSION: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedLayout {
    version: u32,
    state:   DockState<EditorTab>,
}

/// Serialize the current dock layout to a JSON string for storage in
/// [`fluxion_core::EditorPrefs::dock_layout`]. Returns `None` on serialization
/// failure (logged) — caller should leave the prefs field unchanged.
pub fn save_dock_layout(state: &DockState<EditorTab>) -> Option<String> {
    let wrapped = PersistedLayout { version: DOCK_LAYOUT_VERSION, state: state.clone() };
    match serde_json::to_string(&wrapped) {
        Ok(s) => Some(s),
        Err(e) => { log::warn!("Dock layout serialization failed: {e}"); None }
    }
}

/// Restore a saved dock layout JSON. Returns `None` (and logs at info level)
/// if the version doesn't match, so the caller falls back to
/// [`default_dock_state`] and the user gets a fresh layout.
pub fn restore_dock_layout(json: &str) -> Option<DockState<EditorTab>> {
    match serde_json::from_str::<PersistedLayout>(json) {
        Ok(p) if p.version == DOCK_LAYOUT_VERSION => Some(p.state),
        Ok(p) => {
            log::info!("Dock layout version mismatch (saved={}, current={}); using default.", p.version, DOCK_LAYOUT_VERSION);
            None
        }
        Err(e) => {
            log::warn!("Dock layout parse failed: {e} — using default");
            None
        }
    }
}

// ── Show ──────────────────────────────────────────────────────────────────────

/// Render the entire dock area for this frame.
pub fn show_dock(
    ui:         &mut egui::Ui,
    dock_state: &mut DockState<EditorTab>,
    vm:         &mut fluxion_rune_scripting::RuneVm,
) {
    DockArea::new(dock_state)
        .style(Style::from_egui(ui.ctx().global_style().as_ref()))
        .show_inside(ui, &mut RuneTabViewer { vm });
}
