// build.rs — FluxionRS Editor
//
// Copies the two runtime asset directories into the Cargo output directory
// (target/debug/ or target/release/) so the editor binary can find them
// regardless of the working directory it is launched from.
//
//   target/{profile}/assets/   ← from  <workspace_root>/assets/
//   target/{profile}/scripts/  ← from  <editor_crate>/scripts/
//
// Incremental: individual files are only overwritten when their content
// differs, which keeps the watcher-triggered rebuilds fast.

use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    // OUT_DIR = target/{profile}/build/<crate>-<hash>/out
    // Three parent() calls reach target/{profile}/.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target_dir = out_dir
        .parent().unwrap() // …/<crate>-<hash>
        .parent().unwrap() // …/build
        .parent().unwrap() // target/{profile}
        .to_path_buf();

    // ── Copy workspace-level assets/ ─────────────────────────────────────────
    // The editor crate lives at  <workspace>/crates/fluxion-editor/
    // so the workspace root is two levels up.
    let assets_src = manifest_dir.join("..").join("..").join("assets");
    let assets_dst = target_dir.join("assets");
    copy_dir_incremental(&assets_src, &assets_dst);

    // ── Copy editor Rune panel scripts/ ──────────────────────────────────────
    let scripts_src = manifest_dir.join("scripts");
    let scripts_dst = target_dir.join("scripts");
    copy_dir_incremental(&scripts_src, &scripts_dst);

    // ── Rebuild triggers ─────────────────────────────────────────────────────
    // Tell Cargo to re-run this script when either source directory changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=scripts");

    // For the workspace assets we need an absolute path because Cargo
    // interprets relative rerun-if-changed paths relative to the manifest dir.
    let assets_src_canon = assets_src
        .canonicalize()
        .unwrap_or_else(|_| assets_src.clone());
    println!("cargo:rerun-if-changed={}", assets_src_canon.display());
}

/// Recursively copy `src` into `dst`, skipping files whose content has not
/// changed.  New directories are created as needed.  Files that exist in
/// `dst` but no longer exist in `src` are left in place (safe for caching).
fn copy_dir_incremental(src: &Path, dst: &Path) {
    let src = match src.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // Source directory does not exist — nothing to copy.
            return;
        }
    };

    if let Err(e) = std::fs::create_dir_all(dst) {
        eprintln!("build.rs: failed to create {:?}: {e}", dst);
        return;
    }

    let entries = match std::fs::read_dir(&src) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("build.rs: cannot read {:?}: {e}", src);
            return;
        }
    };

    for entry in entries.flatten() {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        match entry.file_type() {
            Ok(ft) if ft.is_dir() => {
                copy_dir_incremental(&src_path, &dst_path);
            }
            Ok(_) => {
                copy_file_if_changed(&src_path, &dst_path);
            }
            Err(e) => {
                eprintln!("build.rs: file_type error for {:?}: {e}", src_path);
            }
        }
    }
}

/// Copy `src` → `dst` only when the destination is missing or its content
/// differs from the source.  This avoids touching the modification time of
/// unchanged files, keeping incremental builds fast.
fn copy_file_if_changed(src: &Path, dst: &Path) {
    // Read source bytes first; bail if we cannot.
    let src_bytes = match std::fs::read(src) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("build.rs: cannot read {:?}: {e}", src);
            return;
        }
    };

    // If the destination already exists and has the same content, skip it.
    if let Ok(dst_bytes) = std::fs::read(dst) {
        if dst_bytes == src_bytes {
            return;
        }
    }

    if let Err(e) = std::fs::write(dst, &src_bytes) {
        eprintln!("build.rs: cannot write {:?}: {e}", dst);
    }
}
