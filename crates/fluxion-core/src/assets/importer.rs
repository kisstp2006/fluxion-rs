// ============================================================
// fluxion-core — Importer pipeline (B1 scaffolding)
//
// Per-format processing seam between source assets (assets/) and
// cooked runtime artifacts (.fluxion-cache/). Concrete cookers
// (TextureImporter for BC7, GltfImporter, RuneScriptImporter, …)
// land in later milestones — this module establishes the trait,
// registry, and a no-op PassthroughImporter so the seam is wired
// without changing observable behavior.
//
// Pattern matches EZ Engine's `ezAssetDocumentManager` and
// Urho3D's `ResourceCache` factories: a registry of per-format
// importers; a fallback that copies bytes verbatim.
// ============================================================

use std::collections::BTreeMap;
use std::path::Path;

use super::database::AssetType;

/// Output of a single import. The cache layer (B2) hashes
/// `cooked_bytes` to derive the on-disk filename so identical
/// inputs share a cooked artifact.
#[derive(Debug, Clone)]
pub struct ImportedAsset {
    /// Broad classification — written into the manifest so the
    /// runtime can pick the right loader without re-inspecting bytes.
    pub kind: AssetType,
    /// Cooked bytes ready to ship. For [`PassthroughImporter`] this
    /// is the source file verbatim.
    pub cooked_bytes: Vec<u8>,
    /// Logical asset paths this output depends on (e.g. textures
    /// referenced by a material, scripts referenced by a scene).
    /// Used by the dependency tracker (B4).
    pub deps: Vec<String>,
    /// Hash of the source file at import time. The cache layer
    /// uses this to skip reimport when the source hasn't changed.
    pub source_hash: u64,
}

/// Per-extension import settings sourced from `.fluxmeta` sidecars.
/// Stored as flat string-string for now (matching `AssetRecord::import_settings`).
pub type MetaImportSettings = BTreeMap<String, String>;

/// Trait implemented by per-format importers (texture, glTF, audio, …).
/// Sync only for now — async cooking is a v2 concern.
pub trait Importer: Send + Sync {
    /// Lowercase extensions this importer handles, e.g. `&["png", "jpg"]`.
    fn extensions(&self) -> &[&'static str];

    /// Read `src` and produce an [`ImportedAsset`]. Errors are returned
    /// as plain strings so the caller can surface them in the editor
    /// console without dragging in `anyhow` at the core boundary.
    fn import(&self, src: &Path, settings: &MetaImportSettings) -> Result<ImportedAsset, String>;
}

// ── Passthrough fallback ──────────────────────────────────────────────────────

/// Copies the source file verbatim. Registered for every currently-handled
/// extension so the new pipeline produces byte-identical output to the
/// pre-importer behavior — proves the seam without any user-visible change.
pub struct PassthroughImporter {
    /// Extensions this instance will report. Fixed at construction time.
    exts: &'static [&'static str],
}

impl PassthroughImporter {
    /// All extensions currently classified by [`AssetType::from_extension`].
    pub const ALL_EXTS: &'static [&'static str] = &[
        "scene",
        "glb", "gltf", "obj", "fbx",
        "png", "jpg", "jpeg", "webp", "gif", "bmp", "tga", "hdr", "exr", "ktx", "dds",
        "wav", "ogg", "mp3", "flac", "aac",
        "rn", "js", "lua", "py",
        "wgsl", "vert", "frag", "glsl", "hlsl",
        "fluxmat",
        "prefab", "fluxprefab",
        "json",
    ];

    pub fn new() -> Self {
        Self { exts: Self::ALL_EXTS }
    }
}

impl Default for PassthroughImporter {
    fn default() -> Self { Self::new() }
}

impl Importer for PassthroughImporter {
    fn extensions(&self) -> &[&'static str] { self.exts }

    fn import(&self, src: &Path, _settings: &MetaImportSettings) -> Result<ImportedAsset, String> {
        let bytes = std::fs::read(src)
            .map_err(|e| format!("PassthroughImporter: read '{}' failed: {e}", src.display()))?;
        let ext = src.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        let kind = AssetType::from_extension(&ext);
        let hash = fxhash64(&bytes);
        Ok(ImportedAsset { kind, cooked_bytes: bytes, deps: Vec::new(), source_hash: hash })
    }
}

// ── Registry ──────────────────────────────────────────────────────────────────

/// Holds the available importers and resolves an extension to one. The
/// registry is owned by [`AssetDatabase`]; callers ask for an importer by
/// extension and then call [`Importer::import`].
pub struct ImporterRegistry {
    importers: Vec<Box<dyn Importer>>,
    /// Last-resort handler used when no registered importer claims the
    /// extension. Always set to a [`PassthroughImporter`] in [`Default`].
    fallback:  Box<dyn Importer>,
}

impl ImporterRegistry {
    /// Build an empty registry with only the passthrough fallback.
    pub fn empty() -> Self {
        Self {
            importers: Vec::new(),
            fallback:  Box::new(PassthroughImporter::new()),
        }
    }

    /// Register a custom importer. The first registered importer that
    /// claims an extension wins (LIFO would be surprising for a config-style
    /// list — keep it explicit and stable).
    pub fn register(&mut self, importer: Box<dyn Importer>) {
        self.importers.push(importer);
    }

    /// Find the importer responsible for `ext` (case-insensitive). Returns
    /// the passthrough fallback when no specialized importer matches.
    pub fn resolve(&self, ext: &str) -> &dyn Importer {
        let ext_low = ext.to_ascii_lowercase();
        for imp in &self.importers {
            if imp.extensions().iter().any(|e| *e == ext_low) {
                return imp.as_ref();
            }
        }
        self.fallback.as_ref()
    }
}

impl Default for ImporterRegistry {
    /// Production default: passthrough only. Specialized importers
    /// (texture, glTF, …) opt in via [`Self::register`] in later milestones.
    fn default() -> Self { Self::empty() }
}

// ── Hashing ──────────────────────────────────────────────────────────────────

/// Stable, dependency-free 64-bit hash (FxHash variant). Used for source-hash
/// comparison and cooked-filename derivation; not cryptographic.
pub(crate) fn fxhash64(bytes: &[u8]) -> u64 {
    const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;
    let mut h: u64 = SEED;
    for chunk in bytes.chunks(8) {
        let mut buf = [0u8; 8];
        buf[..chunk.len()].copy_from_slice(chunk);
        let v = u64::from_le_bytes(buf);
        h = h.rotate_left(5) ^ v;
        h = h.wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    h
}
