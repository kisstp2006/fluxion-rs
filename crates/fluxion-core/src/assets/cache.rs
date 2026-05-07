// ============================================================
// fluxion-core — Cooked-asset cache (B2 scaffolding)
//
// Source assets live in `assets/`. Cooked, runtime-ready binaries
// live in `.fluxion-cache/` next to the project root, named by
// `<guid>-<contenthash>.fxa`. A small JSON manifest at
// `.fluxion-cache/manifest.json` maps GUID → filename + source
// mtime/hash so a rescan can skip up-to-date entries (B3) and
// the runtime can resolve an asset by GUID without reading the
// source file (D5).
//
// This module is the scaffolding only — nothing writes cooked
// bytes yet (that lands when concrete importers ship in B6/B7).
// The directory and an empty manifest are created on first scan
// so the layout is in place for follow-up milestones.
// ============================================================

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Logical name of the cache directory placed alongside `assets/`.
pub const CACHE_DIR_NAME: &str = ".fluxion-cache";

/// Manifest filename inside the cache directory.
pub const MANIFEST_NAME:  &str = "manifest.json";

/// Per-asset cache entry. The cooked artifact lives at
/// `{project_root}/.fluxion-cache/{cooked_filename}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CacheEntry {
    /// Cooked filename: `<guid>-<contenthash>.fxa`.
    pub cooked_filename: String,
    /// Source file mtime in milliseconds since UNIX_EPOCH at last cook.
    /// Compared against the live mtime to short-circuit reimport (B3).
    pub source_mtime_ms: u64,
    /// Source file content hash (matches [`super::importer::ImportedAsset::source_hash`]).
    /// Authoritative when mtime is unreliable (e.g. networked filesystems).
    pub source_hash: u64,
}

/// Lazy-loaded JSON manifest keyed by asset GUID. Sorted by GUID so the
/// on-disk file diffs cleanly across runs.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CacheManifest {
    /// Manifest format version. Bump with the format; the loader treats
    /// mismatched versions as "fresh start" (entries discarded silently).
    #[serde(default = "default_version")]
    pub version: u32,
    /// GUID → entry. `BTreeMap` for stable on-disk ordering.
    #[serde(default)]
    pub entries: BTreeMap<String, CacheEntry>,
}

const CURRENT_VERSION: u32 = 1;

fn default_version() -> u32 { CURRENT_VERSION }

impl CacheManifest {
    pub fn new() -> Self { Self::default() }

    /// Compute the cache directory path for a given project root.
    pub fn cache_dir(project_root: &Path) -> PathBuf {
        project_root.join(CACHE_DIR_NAME)
    }

    /// Manifest filepath for a given project root.
    pub fn manifest_path(project_root: &Path) -> PathBuf {
        Self::cache_dir(project_root).join(MANIFEST_NAME)
    }

    /// Load the manifest from `<project_root>/.fluxion-cache/manifest.json`.
    /// Returns an empty manifest if the file is missing, parse-fails, or
    /// has a different version. Never panics.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(project_root: &Path) -> Self {
        let path = Self::manifest_path(project_root);
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return Self::new(),
        };
        match serde_json::from_str::<CacheManifest>(&raw) {
            Ok(m) if m.version == CURRENT_VERSION => m,
            Ok(_) => {
                log::info!("[AssetCache] Manifest version mismatch — starting fresh.");
                Self::new()
            }
            Err(e) => {
                log::warn!("[AssetCache] Manifest parse failed ({e}) — starting fresh.");
                Self::new()
            }
        }
    }

    /// Atomically write the manifest to disk. Creates `.fluxion-cache/` if
    /// needed. Best-effort — caller logs but does not abort on I/O errors.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self, project_root: &Path) -> Result<(), String> {
        let dir  = Self::cache_dir(project_root);
        let path = Self::manifest_path(project_root);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("create_dir_all '{}': {e}", dir.display()))?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serialize manifest: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)
            .map_err(|e| format!("write '{}': {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| format!("rename: {e}"))?;
        Ok(())
    }

    /// Ensure the cache directory exists. Idempotent.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn ensure_dir(project_root: &Path) -> Result<PathBuf, String> {
        let dir = Self::cache_dir(project_root);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("create_dir_all '{}': {e}", dir.display()))?;
        Ok(dir)
    }

    /// Look up a cache entry by asset GUID.
    pub fn get(&self, guid: &str) -> Option<&CacheEntry> {
        self.entries.get(guid)
    }

    /// Insert or replace a cache entry.
    pub fn insert(&mut self, guid: String, entry: CacheEntry) {
        self.entries.insert(guid, entry);
    }

    /// Remove orphaned entries — useful after a rescan when some sources
    /// have been deleted.
    #[allow(dead_code)]
    pub fn retain_guids(&mut self, live: &std::collections::HashSet<String>) {
        self.entries.retain(|k, _| live.contains(k));
    }
}

/// Build the standard cooked filename for `(guid, content_hash)`.
/// Used by importers when writing a new cooked artifact.
pub fn cooked_filename(guid: &str, content_hash: u64) -> String {
    format!("{guid}-{content_hash:016x}.fxa")
}
