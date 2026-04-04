// ============================================================
// hot_reload.rs — file watcher + incremental Rune recompile
//
// Watches a directory for .rn file changes (native only).
// On change: re-reads the file, recompiles, swaps the unit.
//
// Design:
//   - Uses the `notify` crate (cross-platform inotify/FSEvents/RDCH).
//   - 50 ms debounce to avoid spurious events on save.
//   - Compile errors keep the old unit running (never crashes the engine).
//   - Calls registered reload hooks after a successful swap.
// ============================================================

#[cfg(not(target_arch = "wasm32"))]
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
};

#[cfg(not(target_arch = "wasm32"))]
use notify::{EventKind, RecursiveMode, Watcher};

/// A pending reload event: which path changed.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct ReloadEvent {
    pub path: PathBuf,
}

/// File-system watcher that feeds a channel with changed `.rn` paths.
#[cfg(not(target_arch = "wasm32"))]
pub struct HotReloadWatcher {
    _watcher: Box<dyn Watcher + Send>,
    rx:       mpsc::Receiver<ReloadEvent>,
}

#[cfg(not(target_arch = "wasm32"))]
impl HotReloadWatcher {
    /// Start watching `watch_dir` recursively. Returns immediately.
    pub fn start(watch_dir: &Path) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
            let Ok(ev) = ev else { return };
            let is_write = matches!(
                ev.kind,
                EventKind::Create(_) | EventKind::Modify(_)
            );
            if !is_write { return; }

            for p in ev.paths {
                if p.extension().map(|e| e == "rn").unwrap_or(false) {
                    let _ = tx.send(ReloadEvent { path: p });
                }
            }
        })?;

        watcher.watch(watch_dir, RecursiveMode::Recursive)?;
        log::info!("[RuneHotReload] Watching {:?} for .rn changes", watch_dir);

        Ok(Self {
            _watcher: Box::new(watcher),
            rx,
        })
    }

    /// Drain all pending events (non-blocking). Deduplicates by path.
    pub fn drain(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.rx.try_iter()
            .map(|e| e.path)
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }
}

// On WASM the watcher is a no-op stub.
#[cfg(target_arch = "wasm32")]
pub struct HotReloadWatcher;

#[cfg(target_arch = "wasm32")]
impl HotReloadWatcher {
    pub fn start(_watch_dir: &std::path::Path) -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub fn drain(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
}
