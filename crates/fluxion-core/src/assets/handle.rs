// ============================================================
// fluxion-core — AssetId (B5 scaffolding)
//
// Stable identifier for an asset across renames, moves, and
// project layouts. Backed by the GUID written into the asset's
// `.fluxmeta` sidecar at scan time.
//
// `AssetId` is a thin newtype around `String` (the GUID) so it
// (a) carries clear semantic intent at API boundaries, and
// (b) can be migrated to a smaller representation (e.g. `Uuid`)
//     without touching every call site.
//
// Components currently still hold `Option<String>` paths
// (registry.rs:374-375 and friends). Migrating each component
// to `Option<AssetId>` is a separate follow-up — this module
// just gives the type and the lookup glue.
// ============================================================

use std::fmt;

/// Stable identifier for a project asset. Wraps the GUID stored in the
/// asset's `.fluxmeta` sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AssetId(pub String);

impl AssetId {
    /// Wrap a GUID string. The caller is responsible for ensuring the GUID
    /// is well-formed; the registry's GUID writer is the canonical source.
    pub fn from_guid(s: impl Into<String>) -> Self { Self(s.into()) }

    /// Empty/sentinel id used to represent "no asset" before [`Option`]
    /// support was widespread. Prefer `Option<AssetId>` for new fields.
    pub const fn empty() -> Self { Self(String::new()) }

    /// True when the inner GUID string is empty.
    pub fn is_empty(&self) -> bool { self.0.is_empty() }

    /// Borrow the inner GUID as a `&str`.
    pub fn as_str(&self) -> &str { &self.0 }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for AssetId {
    fn from(s: String) -> Self { Self(s) }
}

impl From<&str> for AssetId {
    fn from(s: &str) -> Self { Self(s.to_string()) }
}

/// Resolution helper: given either a GUID or a project-relative path string,
/// return the canonical GUID by consulting the database. Returns `None` for
/// strings that resolve to neither.
///
/// This is the **migration glue** for B5: scenes loaded from v3 store paths
/// like `"models/cube.glb"`, while v4+ stores the GUID directly. The reader
/// path normalises both to GUID via this helper before any component sees it.
pub fn resolve_to_guid(
    db:    &super::AssetDatabase,
    input: &str,
) -> Option<String> {
    if input.is_empty() { return None; }
    // Already a GUID? AssetDatabase keeps a `guid → path` index.
    if db.path_by_guid(input).is_some() {
        return Some(input.to_string());
    }
    // Otherwise treat as a project-relative path.
    db.guid_by_path(input).map(str::to_string)
}
