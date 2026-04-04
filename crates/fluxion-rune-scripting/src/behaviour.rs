// ============================================================
// behaviour.rs — RuneBehaviour
//
// A `RuneBehaviour` is a single Rune script attached to an entity.
// It owns a `RuneVm` compiled from one .rn file and calls the
// standard lifecycle hooks each frame.
// ============================================================

use std::path::Path;
use anyhow::Context as _;

use crate::vm::RuneVm;

/// A Rune script component attached to an entity.
///
/// # Lifecycle
/// ```rune
/// pub fn start() { }
/// pub fn update(dt) { }
/// pub fn fixed_update(dt) { }
/// pub fn on_destroy() { }
///
/// // Editor-only hooks
/// pub fn on_editor_init() { }
/// pub fn on_hot_reload() { }
/// pub fn on_entity_selected() { }
/// ```
pub struct RuneBehaviour {
    pub vm:         RuneVm,
    pub script_path: std::path::PathBuf,
    started:        bool,
}

impl RuneBehaviour {
    /// Load a behaviour from a single `.rn` file.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let mut vm = RuneVm::new(&[path])
            .with_context(|| format!("Failed to compile {:?}", path))?;

        // Enable hot reload for the parent directory of this script.
        // poll_hot_reload() will call on_hot_reload_hook() after each successful reload.
        if let Some(dir) = path.parent() {
            let _ = vm.enable_hot_reload(dir);
        }

        Ok(Self {
            vm,
            script_path: path.to_path_buf(),
            started: false,
        })
    }

    /// Call per-frame. Calls `start()` on the first tick, then `update(dt)`.
    pub fn tick(&mut self, dt: f32) {
        // Process any pending hot-reload events first.
        self.vm.poll_hot_reload();

        if !self.started {
            self.started = true;
            if let Err(e) = self.vm.start() {
                log::error!("[RuneBehaviour] start() error in {:?}: {e}", self.script_path.file_name().unwrap_or_default());
            }
        }

        if let Err(e) = self.vm.update(dt as f64) {
            log::error!("[RuneBehaviour] update() error in {:?}: {e}", self.script_path.file_name().unwrap_or_default());
        }
    }

    /// Call during the physics sub-step.
    pub fn fixed_tick(&mut self, dt: f32) {
        if let Err(e) = self.vm.fixed_update(dt as f64) {
            log::error!("[RuneBehaviour] fixed_update() error: {e}");
        }
    }

    /// Call when the entity is destroyed.
    pub fn destroy(&self) {
        if let Err(e) = self.vm.on_destroy() {
            log::error!("[RuneBehaviour] on_destroy() error: {e}");
        }
    }

    /// Returns the last compile error (for the in-editor overlay).
    pub fn error(&self) -> Option<&str> {
        self.vm.last_error.as_deref()
    }
}
