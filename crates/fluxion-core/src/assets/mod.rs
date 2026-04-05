pub mod database;
pub use database::{AssetDatabase, AssetRecord, AssetType, new_guid};

// ============================================================
// fluxion-core — Asset pipeline (FluxionJS parity)
//
// Logical paths in .scene / .fluxmat / glTF match the TypeScript engine:
// forward slashes, relative to the scene or project root.
//
// Backends:
//   - DiskAssetSource (native): root directory + safe join
//   - MemoryAssetSource: preloaded bytes (WASM, tests, packs)
//   - FnAssetSource: custom resolver
//
// Classified file kinds mirror typical FluxionJS / editor exports:
//   .scene, .fluxmat, .glb/.gltf, images, WGSL shaders, generic JSON.
// ============================================================

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Errors from [`AssetSource::read`].
#[derive(Debug, Clone)]
pub enum AssetError {
    NotFound(String),
    Io(String),
    InvalidPath(String),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::NotFound(p) => write!(f, "asset not found: {p}"),
            AssetError::Io(e) => write!(f, "asset I/O: {e}"),
            AssetError::InvalidPath(p) => write!(f, "invalid asset path: {p}"),
        }
    }
}

impl std::error::Error for AssetError {}

/// Asset kinds used across FluxionJS-style projects (extension-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FluxionAssetKind {
    Scene,
    FluxMat,
    GltfBinary,
    GltfJson,
    Image,
    Shader,
    Json,
    Unknown,
}

/// Infer kind from a logical path (lowercase extension).
pub fn classify_path(path: &str) -> FluxionAssetKind {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".scene") {
        return FluxionAssetKind::Scene;
    }
    if lower.ends_with(".fluxmat") {
        return FluxionAssetKind::FluxMat;
    }
    if lower.ends_with(".glb") {
        return FluxionAssetKind::GltfBinary;
    }
    if lower.ends_with(".gltf") {
        return FluxionAssetKind::GltfJson;
    }
    if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
        || lower.ends_with(".bmp")
        || lower.ends_with(".tga")
    {
        return FluxionAssetKind::Image;
    }
    if lower.ends_with(".wgsl")
        || lower.ends_with(".vert")
        || lower.ends_with(".frag")
    {
        return FluxionAssetKind::Shader;
    }
    if lower.ends_with(".json") {
        return FluxionAssetKind::Json;
    }
    FluxionAssetKind::Unknown
}

/// Join optional base prefix and relative path (FluxionJS-style `/` paths).
pub fn join_logical(base: Option<&str>, rel: &str) -> String {
    let rel = rel.trim_start_matches(['/', '\\']);
    match base {
        None | Some("") => rel.to_string(),
        Some(b) => {
            let b = b.trim_end_matches(['/', '\\']);
            format!("{b}/{rel}")
        }
    }
}

/// Reject `..` segments when resolving under a root (zip-slip safety).
pub fn resolve_under_root(root: &Path, rel: &str) -> Result<PathBuf, AssetError> {
    let rel = rel.replace('\\', "/");
    if rel.split('/').any(|s| s == "..") {
        return Err(AssetError::InvalidPath(rel));
    }
    Ok(root.join(&rel))
}

/// Read a UTF-8 text asset (`.scene`, `.fluxmat`, `.wgsl`, …).
pub fn read_text(source: &dyn AssetSource, path: &str) -> Result<String, AssetError> {
    let bytes = source.read(path)?;
    String::from_utf8(bytes).map_err(|e| AssetError::Io(format!("{path}: {e}")))
}

/// Abstract byte source for assets (native disk, WASM memory, fetch wrapper, …).
pub trait AssetSource: Send + Sync {
    fn read(&self, path: &str) -> Result<Vec<u8>, AssetError>;

    fn exists(&self, path: &str) -> bool {
        self.read(path).is_ok()
    }

    /// Native disk root for multi-file `.gltf` import (`gltf::import` needs a real path).
    fn native_project_root(&self) -> Option<&Path> {
        None
    }
}

/// Filesystem backend: `read("models/a.glb")` → `root/models/a.glb`.
#[cfg(not(target_arch = "wasm32"))]
pub struct DiskAssetSource {
    pub root: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl DiskAssetSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl AssetSource for DiskAssetSource {
    fn read(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        let p = resolve_under_root(&self.root, path)?;
        std::fs::read(&p).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AssetError::NotFound(p.display().to_string())
            } else {
                AssetError::Io(format!("{}: {e}", p.display()))
            }
        })
    }

    fn exists(&self, path: &str) -> bool {
        resolve_under_root(&self.root, path)
            .map(|p| p.is_file())
            .unwrap_or(false)
    }

    fn native_project_root(&self) -> Option<&Path> {
        Some(self.root.as_path())
    }
}

/// In-memory pack (e.g. WASM after fetch or `include_dir!` preprocessing).
#[derive(Clone, Default)]
pub struct MemoryAssetSource {
    /// Keys: logical paths with forward slashes, no leading slash.
    pub files: HashMap<String, Vec<u8>>,
}

impl MemoryAssetSource {
    pub fn new(files: HashMap<String, Vec<u8>>) -> Self {
        Self { files }
    }

    fn key(path: &str) -> String {
        path.replace('\\', "/").trim_start_matches('/').to_string()
    }

    pub fn insert(&mut self, path: impl Into<String>, bytes: Vec<u8>) {
        self.files.insert(Self::key(&path.into()), bytes);
    }
}

impl AssetSource for MemoryAssetSource {
    fn read(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        let k = Self::key(path);
        self.files
            .get(&k)
            .cloned()
            .ok_or_else(|| AssetError::NotFound(k))
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(&Self::key(path))
    }
}

/// Closure-backed source (`WASM` fetch cache, tests).
pub struct FnAssetSource {
    f: std::sync::Arc<dyn Fn(&str) -> Result<Vec<u8>, AssetError> + Send + Sync>,
}

impl FnAssetSource {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&str) -> Result<Vec<u8>, AssetError> + Send + Sync + 'static,
    {
        Self { f: std::sync::Arc::new(f) }
    }
}

impl AssetSource for FnAssetSource {
    fn read(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        (self.f)(path)
    }
}
