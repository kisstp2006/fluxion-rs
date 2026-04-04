// ============================================================
// dock.rs — egui_dock layout and Rune panel dispatch
//
// Each dockable tab holds a name + the Rune function path to
// call each frame.  The RuneTabViewer sets the thread-local Ui
// pointer before calling the Rune function, then clears it.
// ============================================================

use egui_dock::{DockArea, DockState, NodeIndex, Style};

use crate::rune_bindings::{set_current_ui, UiContextGuard};

// ── Tab data ─────────────────────────────────────────────────────────────────

/// Data stored per dockable tab.
#[derive(Debug, Clone)]
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
        // Build the &[&str] path from "module::function".
        let parts: Vec<&str> = tab.rune_fn.split("::").collect();

        // Guard clears CURRENT_UI on drop — safe on both normal return and panic.
        let _ui_guard: UiContextGuard = set_current_ui(ui);

        let result = self.vm.call_fn(&parts, ());
        if let Err(e) = result {
            // Use {e:#} to show the full anyhow error chain (not just the wrapper message).
            let msg = format!("{e:#}");
            log::error!("Rune panel '{}': {msg}", tab.rune_fn);
            ui.colored_label(egui::Color32::RED, format!("⚠ {}: {msg}", tab.rune_fn));
        }
        // _ui_guard drops here (or on early return above), clearing CURRENT_UI.
    }

    fn closeable(&mut self, _tab: &mut EditorTab) -> bool {
        false // panels are not closeable in basic mode
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

    // Split bottom 25% for Console (anchor to hierarchy node so it spans the full width)
    surface.split_below(
        hier_node,
        0.75,
        vec![EditorTab::new("Console", "console::panel")],
    );

    let _ = centre2;

    state
}

// ── Show ──────────────────────────────────────────────────────────────────────

/// Render the entire dock area for this frame.
pub fn show_dock(
    ctx:        &egui::Context,
    dock_state: &mut DockState<EditorTab>,
    vm:         &mut fluxion_rune_scripting::RuneVm,
) {
    DockArea::new(dock_state)
        .style(Style::from_egui(ctx.style().as_ref()))
        .show(ctx, &mut RuneTabViewer { vm });
}
