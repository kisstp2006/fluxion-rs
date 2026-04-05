// ============================================================
// fluxion-core — AssetDatabase
//
// GUID-tracked, sidecar-based asset catalogue.
//
// * Scans the project's `assets/` directory recursively.
// * Writes / reads `.fluxmeta` JSON sidecars for stable GUIDs.
// * Classifies files by extension (texture / model / audio …).
// * Provides a Unity-like query API used by the editor panel.
//
// All query methods work on WASM too (in-memory only).
// Only `scan()` and sidecar I/O are gated on `not(wasm32)`.
// ============================================================

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// ── Asset type ────────────────────────────────────────────────────────────────

/// Broad classification of an asset file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AssetType {
    Scene,
    Model,
    Texture,
    Audio,
    Script,
    Shader,
    Material,
    Prefab,
    Json,
    Unknown,
}

impl AssetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AssetType::Scene    => "scene",
            AssetType::Model    => "model",
            AssetType::Texture  => "texture",
            AssetType::Audio    => "audio",
            AssetType::Script   => "script",
            AssetType::Shader   => "shader",
            AssetType::Material => "material",
            AssetType::Prefab   => "prefab",
            AssetType::Json     => "json",
            AssetType::Unknown  => "unknown",
        }
    }

    /// Infer type from a lowercase file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext {
            "scene"                                                          => AssetType::Scene,
            "glb" | "gltf" | "obj" | "fbx"                                  => AssetType::Model,
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tga"
            | "hdr" | "exr" | "ktx" | "dds"                                 => AssetType::Texture,
            "wav" | "ogg" | "mp3" | "flac" | "aac"                          => AssetType::Audio,
            "rn" | "js" | "lua" | "py"                                       => AssetType::Script,
            "wgsl" | "vert" | "frag" | "glsl" | "hlsl"                      => AssetType::Shader,
            "fluxmat"                                                        => AssetType::Material,
            "prefab" | "fluxprefab"                                          => AssetType::Prefab,
            "json"                                                           => AssetType::Json,
            _                                                                => AssetType::Unknown,
        }
    }
}

// ── AssetRecord ───────────────────────────────────────────────────────────────

/// Metadata record for a single project asset.
#[derive(Debug, Clone)]
pub struct AssetRecord {
    /// Stable UUID v4 — persisted in a `.fluxmeta` sidecar file.
    /// Remains the same across renames (once the meta is present).
    pub guid: String,
    /// Project-relative path with forward slashes, no leading slash.
    /// Example: `"textures/skybox/noon.hdr"`
    pub path: String,
    /// Filename without extension. Example: `"noon"`
    pub name: String,
    /// Lowercase extension. Example: `"hdr"`
    pub extension: String,
    /// Broad type classification.
    pub asset_type: AssetType,
    /// File size in bytes.
    pub file_size: u64,
    /// ISO-8601 timestamp of first indexing.
    pub imported_at: String,
    /// User-assigned tags (stored in `.fluxmeta`).
    pub tags: Vec<String>,
}

impl AssetRecord {
    /// Short type string ("texture", "model", …) — convenient for Rune scripts.
    pub fn type_str(&self) -> &'static str {
        self.asset_type.as_str()
    }

    /// Human-readable size string (e.g. "1.2 MB").
    pub fn size_display(&self) -> String {
        let b = self.file_size;
        if b < 1024 {
            format!("{b} B")
        } else if b < 1_048_576 {
            format!("{:.1} KB", b as f64 / 1024.0)
        } else if b < 1_073_741_824 {
            format!("{:.1} MB", b as f64 / 1_048_576.0)
        } else {
            format!("{:.2} GB", b as f64 / 1_073_741_824.0)
        }
    }
}

// ── AssetDatabase ─────────────────────────────────────────────────────────────

/// In-memory catalogue of all project assets.
///
/// Call [`scan`] once after opening a project; call it again whenever the user
/// explicitly requests a rescan or after importing new files.
pub struct AssetDatabase {
    records:    Vec<AssetRecord>,
    path_index: HashMap<String, usize>,   // normalised path → index
    guid_index: HashMap<String, String>,  // guid → normalised path
    /// Root directory used for the last scan (i.e. the project root).
    pub root: PathBuf,
    /// Top-level subdirectories of assets/ — includes empty ones.
    dirs: BTreeSet<String>,
}

impl Default for AssetDatabase {
    fn default() -> Self {
        Self {
            records:    Vec::new(),
            path_index: HashMap::new(),
            guid_index: HashMap::new(),
            root:       PathBuf::new(),
            dirs:       BTreeSet::new(),
        }
    }
}

impl AssetDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Scan (native only) ────────────────────────────────────────────────────

    /// Scan `{root}/assets/` recursively (falls back to `root` itself if
    /// `assets/` doesn't exist).  Reads or creates `.fluxmeta` sidecars for
    /// every file so GUIDs are stable across restarts and renames.
    ///
    /// This replaces the previous contents of the database entirely.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn scan(&mut self, root: &Path) {
        self.records.clear();
        self.path_index.clear();
        self.guid_index.clear();
        self.root = root.to_path_buf();

        let scan_root = {
            let r = root.join("assets");
            if r.is_dir() { r } else { root.to_path_buf() }
        };

        Self::walk_dir(&scan_root, &scan_root, &mut self.records);
        self.records.sort_by(|a, b| a.path.cmp(&b.path));

        for (i, rec) in self.records.iter().enumerate() {
            self.path_index.insert(rec.path.clone(), i);
            self.guid_index.insert(rec.guid.clone(), rec.path.clone());
        }

        // Build the directory set: union of filesystem dirs + file-path-derived dirs.
        self.dirs.clear();
        // 1. Dirs inferred from file paths (always present even without a filesystem hit).
        for rec in &self.records {
            if let Some(slash) = rec.path.find('/') {
                self.dirs.insert(rec.path[..slash].to_string());
            }
        }
        // 2. Actual top-level subdirectories of scan_root (catches empty folders).
        if let Ok(entries) = std::fs::read_dir(&scan_root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_dir() { continue; }
                let name = p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                if name.starts_with('.') {
                    continue;
                }
                if matches!(name.as_str(), "target" | "node_modules" | ".git") {
                    continue;
                }
                self.dirs.insert(name);
            }
        }

        log::debug!(
            "[AssetDatabase] scan complete — {} assets under {:?}",
            self.records.len(),
            scan_root
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn walk_dir(scan_root: &Path, dir: &Path, records: &mut Vec<AssetRecord>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path      = entry.path();
            let file_name = entry.file_name();
            let name_str  = file_name.to_string_lossy();

            // Skip hidden directories / files and .fluxmeta sidecars.
            if name_str.starts_with('.') || name_str.ends_with(".fluxmeta") {
                continue;
            }
            // Skip common non-asset directories.
            if path.is_dir() {
                let dir_name = name_str.as_ref();
                if matches!(dir_name, "target" | "node_modules" | ".git") {
                    continue;
                }
                Self::walk_dir(scan_root, &path, records);
            } else if path.is_file() {
                if let Some(rec) = Self::make_record(scan_root, &path) {
                    records.push(rec);
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn make_record(scan_root: &Path, abs_path: &Path) -> Option<AssetRecord> {
        let rel     = abs_path.strip_prefix(scan_root).ok()?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");

        let ext = abs_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();

        let name = abs_path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let asset_type = AssetType::from_extension(&ext);
        let file_size  = std::fs::metadata(abs_path).map(|m| m.len()).unwrap_or(0);

        let meta_path = PathBuf::from(format!("{}.fluxmeta", abs_path.display()));
        let (guid, imported_at, tags) = Self::load_or_create_meta(&meta_path);

        Some(AssetRecord {
            guid,
            path: rel_str,
            name,
            extension: ext,
            asset_type,
            file_size,
            imported_at,
            tags,
        })
    }

    /// Read an existing `.fluxmeta`; write a fresh one if absent or corrupt.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_or_create_meta(meta_path: &Path) -> (String, String, Vec<String>) {
        if meta_path.is_file() {
            if let Ok(raw) = std::fs::read_to_string(meta_path) {
                if let Some(parsed) = parse_meta_json(&raw) {
                    return parsed;
                }
            }
        }
        let guid        = new_guid();
        let imported_at = iso_now();
        let tags        = Vec::<String>::new();
        let json        = write_meta_json(&guid, &imported_at, &tags);
        if let Err(e) = std::fs::write(meta_path, &json) {
            log::warn!("[AssetDatabase] could not write .fluxmeta {:?}: {e}", meta_path);
        }
        (guid, imported_at, tags)
    }

    // ── Query API (all platforms) ─────────────────────────────────────────────

    /// Look up a record by its project-relative path.
    pub fn get_by_path(&self, path: &str) -> Option<&AssetRecord> {
        let idx = *self.path_index.get(&norm_path(path))?;
        self.records.get(idx)
    }

    /// Look up a record by GUID.
    pub fn get_by_guid(&self, guid: &str) -> Option<&AssetRecord> {
        let path = self.guid_index.get(guid)?;
        self.get_by_path(path)
    }

    /// All records whose path starts with `subdir/` (non-recursive — direct
    /// children only).  Pass `""` to get root-level files.
    pub fn list_dir<'a>(&'a self, subdir: &str) -> Vec<&'a AssetRecord> {
        let prefix = if subdir.is_empty() {
            String::new()
        } else {
            format!("{}/", subdir.trim_end_matches('/'))
        };
        self.records.iter().filter(|r| {
            if prefix.is_empty() {
                !r.path.contains('/')
            } else {
                r.path.starts_with(&prefix) && !r.path[prefix.len()..].contains('/')
            }
        }).collect()
    }

    /// Unique immediate subdirectories of assets/ (sorted).
    /// Includes empty directories created on disk even if they contain no files.
    pub fn list_dirs(&self) -> Vec<String> {
        self.dirs.iter().cloned().collect()
    }

    /// Search by name or type.
    ///
    /// Supported query syntax:
    /// * `type:texture` — filter by asset type string
    /// * `name:sky`     — filter by filename (case-insensitive)
    /// * `sky`          — shorthand name search
    pub fn find(&self, query: &str) -> Vec<&AssetRecord> {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return self.records.iter().collect();
        }
        if let Some(rest) = q.strip_prefix("type:") {
            let t = rest.trim();
            return self.records.iter()
                .filter(|r| r.asset_type.as_str().contains(t))
                .collect();
        }
        let name_q = q.strip_prefix("name:").unwrap_or(&q);
        self.records.iter()
            .filter(|r| {
                r.name.to_ascii_lowercase().contains(name_q)
                    || r.path.to_ascii_lowercase().contains(name_q)
            })
            .collect()
    }

    /// Slice of all records (sorted by path).
    pub fn all(&self) -> &[AssetRecord] {
        &self.records
    }

    /// Total number of indexed assets.
    pub fn count(&self) -> usize {
        self.records.len()
    }
}

// ── GUID generation (UUID v4 style, no external deps) ────────────────────────

static GUID_CTR: AtomicU64 = AtomicU64::new(1);

pub fn new_guid() -> String {
    let t   = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let c   = GUID_CTR.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id() as u64;
    // Mix bits so simultaneous calls produce distinct GUIDs.
    let hi  = t ^ (pid.wrapping_shl(17)) ^ (c.wrapping_shl(3));
    let lo  = c.wrapping_mul(0x517c_c1b7_2722_0a95).wrapping_add(t);
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (hi >> 32) as u32,
        ((hi >> 16) & 0xffff) as u16,
        (hi & 0xfff) as u16,
        (((lo >> 48) & 0x3fff) | 0x8000) as u16,
        lo & 0x0000_ffff_ffff_ffff,
    )
}

// ── ISO-8601 timestamp (no chrono dep) ───────────────────────────────────────

fn iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s    = secs % 60;
    let m    = (secs / 60) % 60;
    let h    = (secs / 3600) % 24;
    let days = secs / 86400;
    let (y, mo, d) = days_to_ymd(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Converts days-since-epoch to (year, month, day), correct for 1970–2200.
fn days_to_ymd(mut days: u64) -> (u32, u32, u32) {
    let mut y = 1970u32;
    loop {
        let leap      = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let year_days = if leap { 366u64 } else { 365u64 };
        if days < year_days { break; }
        days -= year_days;
        y += 1;
    }
    let leap   = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let months = if leap {
        [31u64, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut mo = 1u32;
    for &mdays in &months {
        if days < mdays { break; }
        days -= mdays;
        mo += 1;
    }
    (y, mo, days as u32 + 1)
}

// ── .fluxmeta JSON (minimal, no serde dep in core) ───────────────────────────

/// Parse `{ "guid": "…", "imported_at": "…", "tags": […] }`.
fn parse_meta_json(raw: &str) -> Option<(String, String, Vec<String>)> {
    let guid        = json_str(raw, "guid")?;
    let imported_at = json_str(raw, "imported_at").unwrap_or_else(iso_now);
    let tags        = json_str_array(raw, "tags").unwrap_or_default();
    Some((guid, imported_at, tags))
}

fn write_meta_json(guid: &str, imported_at: &str, tags: &[String]) -> String {
    let tags_json = tags
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{{\n  \"guid\": \"{guid}\",\n  \"imported_at\": \"{imported_at}\",\n  \"tags\": [{tags_json}]\n}}\n"
    )
}

/// Extract the first string value for `key` from a flat JSON object.
fn json_str(raw: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start  = raw.find(&needle)?;
    let after  = raw[start + needle.len()..].trim_start_matches(|c: char| c == ':' || c.is_whitespace());
    if !after.starts_with('"') { return None; }
    let inner  = &after[1..];
    let end    = inner.find('"')?;
    Some(inner[..end].to_string())
}

/// Extract a JSON string array for `key` from a flat JSON object.
fn json_str_array(raw: &str, key: &str) -> Option<Vec<String>> {
    let needle  = format!("\"{key}\"");
    let start   = raw.find(&needle)?;
    let after   = raw[start + needle.len()..].trim_start_matches(|c: char| c == ':' || c.is_whitespace());
    if !after.starts_with('[') { return None; }
    let end     = after.find(']')?;
    let content = &after[1..end];
    let items   = content.split(',').filter_map(|s| {
        let s = s.trim();
        if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
            Some(s[1..s.len()-1].to_string())
        } else {
            None
        }
    }).collect();
    Some(items)
}

/// Normalise a path to forward-slash, no leading slash.
fn norm_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}
