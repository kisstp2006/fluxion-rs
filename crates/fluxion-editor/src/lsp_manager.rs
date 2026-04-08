// ── LSP process manager ───────────────────────────────────────────────────────
//
// Owns the `fluxion-rune-lsp` child process.
// The editor spawns it on project open and kills it on exit.
// VS Code connects to the same binary path independently via stdio transport;
// the editor does NOT proxy LSP messages.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Global flag readable from Rune via `fluxion::world::lsp_running()`.
pub static LSP_RUNNING: AtomicBool = AtomicBool::new(false);

pub struct LspManager {
    child: Option<std::process::Child>,
}

impl LspManager {
    pub fn new() -> Self {
        Self { child: None }
    }

    /// Resolve the LSP binary.
    ///
    /// Search order:
    ///   1. Workspace `target/debug/fluxion-rune-lsp[.exe]`  (cargo build)
    ///   2. Workspace `target/release/fluxion-rune-lsp[.exe]`
    ///   3. `rune-languageserver[.exe]` in `$CARGO_HOME/bin` (cargo install fallback)
    pub fn resolve_binary() -> Option<PathBuf> {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // crates/fluxion-editor → workspace root (two levels up)
        if let Some(root) = manifest.parent().and_then(|p| p.parent()) {
            for profile in &["debug", "release"] {
                let p = root.join("target").join(profile).join(Self::bin_name());
                if p.exists() {
                    return Some(p);
                }
            }
        }

        // Fallback: $CARGO_HOME/bin (cargo install rune-languageserver)
        let fallback_name = if cfg!(target_os = "windows") {
            "rune-languageserver.exe"
        } else {
            "rune-languageserver"
        };
        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs_next::home_dir().map(|h| h.join(".cargo")));
        if let Some(home) = cargo_home {
            let p = home.join("bin").join(fallback_name);
            if p.exists() {
                return Some(p);
            }
        }

        None
    }

    #[cfg(target_os = "windows")]
    fn bin_name() -> &'static str { "fluxion-rune-lsp.exe" }
    #[cfg(not(target_os = "windows"))]
    fn bin_name() -> &'static str { "fluxion-rune-lsp" }

    /// Spawn the LSP server process (stdio transport).
    pub fn start(&mut self, binary: &Path) {
        if self.is_running() {
            log::info!("[LSP] already running, skipping start");
            return;
        }
        match std::process::Command::new(binary)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => {
                log::info!("[LSP] started: {}", binary.display());
                self.child = Some(child);
                LSP_RUNNING.store(true, Ordering::Relaxed);
            }
            Err(e) => {
                log::error!("[LSP] failed to start {}: {e}", binary.display());
            }
        }
    }

    /// Kill the LSP server process and wait for it to exit.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            log::info!("[LSP] stopped");
        }
        LSP_RUNNING.store(false, Ordering::Relaxed);
    }

    /// Stop then start again.
    pub fn restart(&mut self, binary: &Path) {
        self.stop();
        self.start(binary);
    }

    /// Check if the child is still alive (polls exit status without blocking).
    pub fn is_running(&mut self) -> bool {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    // Process exited unexpectedly.
                    self.child = None;
                    LSP_RUNNING.store(false, Ordering::Relaxed);
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            }
        } else {
            false
        }
    }
}

impl Drop for LspManager {
    fn drop(&mut self) {
        self.stop();
    }
}
