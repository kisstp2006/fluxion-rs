// ============================================================
// icons.rs — Feather SVG icon loader
//
// Lazy-loads icons from `assets/icons/<name>.svg` on first use,
// patches `currentColor` → `white` so they render correctly on
// dark backgrounds, then caches the bytes for the lifetime of
// the process.
//
// Rendering is done via egui's image system (egui_extras SVG
// loader installed once in ui_shell.rs).
// ============================================================

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

// ── Byte cache ────────────────────────────────────────────────────────────────

static CACHE: OnceLock<Mutex<HashMap<String, Arc<[u8]>>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, Arc<[u8]>>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Install egui image loaders (SVG, PNG …).
/// Must be called once right after `egui::Context` is created.
pub fn install_loaders(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
}

/// Return the (white-patched) SVG bytes for `name`, loading from disk if needed.
///
/// The icon file is expected at `assets/icons/<name>.svg` relative to the
/// current working directory (the workspace root when running `cargo run`).
///
/// Returns `None` only if the file cannot be found in any search location.
pub fn icon_bytes(name: &str) -> Option<Arc<[u8]>> {
    // Fast path: already cached.
    {
        let Ok(c) = cache().lock() else { return None; };
        if let Some(b) = c.get(name) {
            return Some(b.clone());
        }
    }

    let raw = read_svg_file(name)?;

    // Patch currentColor → white so icons are visible on dark backgrounds.
    // (Feather icons all use currentColor; resvg defaults it to black.)
    let patched: Arc<[u8]> = std::str::from_utf8(&raw)
        .unwrap_or("")
        .replace("currentColor", "white")
        .into_bytes()
        .into();

    if let Ok(mut c) = cache().lock() {
        c.insert(name.to_string(), patched.clone());
    }
    Some(patched)
}

/// The stable `bytes://` URI used to reference icon `name` inside egui's image cache.
/// Always the same string for a given name so egui doesn't reload on every frame.
pub fn icon_uri(name: &str) -> String {
    format!("bytes://feather/{name}.svg")
}

// ── File loading ──────────────────────────────────────────────────────────────

fn read_svg_file(name: &str) -> Option<Vec<u8>> {
    // 1. Relative to CWD (workspace root when using `cargo run`).
    let cwd_path = format!("assets/icons/{name}.svg");
    if let Ok(b) = std::fs::read(&cwd_path) {
        return Some(b);
    }

    // 2. Relative to the running executable (deployed / release builds).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let exe_path = parent.join("assets/icons").join(format!("{name}.svg"));
            if let Ok(b) = std::fs::read(&exe_path) {
                return Some(b);
            }
        }
    }

    log::warn!("[icons] SVG not found: {name}  (looked in assets/icons/{name}.svg)");
    None
}
