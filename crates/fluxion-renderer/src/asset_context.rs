// ============================================================
// fluxion-renderer — Asset GUID resolution context (B5 / Week 6b)
//
// Renderer code stores asset references on components by GUID,
// not by project-relative path. To open the actual file the
// renderer needs to translate GUID → path; this is done via the
// project's `AssetDatabase`, which the host pushes into a
// thread-local pointer once per frame (mirroring the
// world / physics / audio context pattern in fluxion-editor).
//
// Use [`with_resolver`] inside renderer code to look up a path:
//
// ```ignore
// if let Some(path) = with_resolver(|r| r.path_for(guid)) { … }
// ```
// ============================================================

use std::cell::Cell;

use fluxion_core::AssetDatabase;

thread_local! {
    /// Raw pointer to the active `AssetDatabase`. Set by [`set_db`] from the
    /// host immediately before any renderer call that may resolve GUIDs;
    /// cleared by the returned guard on drop.
    static DB_PTR: Cell<Option<*const AssetDatabase>> = Cell::new(None);
}

/// RAII guard returned by [`set_db`]. Drops the thread-local pointer when it
/// goes out of scope so renderer code outside the guarded region cannot
/// accidentally read a stale [`AssetDatabase`].
pub struct AssetDbGuard;

impl Drop for AssetDbGuard {
    fn drop(&mut self) {
        DB_PTR.with(|p| p.set(None));
    }
}

/// Push an [`AssetDatabase`] reference into the thread-local resolver slot.
/// The returned guard clears the slot on drop. Calling [`set_db`] while a
/// previous guard is still alive overwrites the pointer and is allowed —
/// the new pointer is restored to `None` when ANY guard drops.
///
/// # Safety
/// The caller must keep `db` alive for at least as long as the returned
/// guard. Passing a stack-borrow that outlives the guard is the typical
/// pattern (host's tick function holds `&mut self` across the renderer call).
pub fn set_db<'a>(db: &'a AssetDatabase) -> AssetDbGuard {
    DB_PTR.with(|p| p.set(Some(db as *const AssetDatabase)));
    AssetDbGuard
}

/// Resolver handed to the closure passed to [`with_resolver`]. Wraps the
/// thread-local raw pointer with a safe `&AssetDatabase` reborrow.
pub struct Resolver<'a> {
    db: &'a AssetDatabase,
}

impl<'a> Resolver<'a> {
    /// Translate an asset GUID to its project-relative path. Returns `None`
    /// if the GUID is unknown to the database.
    pub fn path_for(&self, guid: &str) -> Option<&'a str> {
        self.db.path_by_guid(guid)
    }

    /// Translate a project-relative path back to its asset GUID. Used when
    /// matching incoming "asset by path" actions (e.g. hot-reload of a
    /// `.fluxmat` whose disk path is known) against components that store
    /// GUIDs.
    pub fn guid_for(&self, path: &str) -> Option<&'a str> {
        self.db.guid_by_path(path)
    }
}

/// Run `f` against the currently-pushed `AssetDatabase`, if any. Returns
/// `None` when no database has been set — callers should treat this the
/// same as a missing GUID (skip the asset, log a warning).
pub fn with_resolver<R>(f: impl FnOnce(&Resolver<'_>) -> R) -> Option<R> {
    DB_PTR.with(|p| {
        let raw = p.get()?;
        // SAFETY: `set_db` requires the caller to keep `db` alive for the
        // guard's lifetime. The closure runs synchronously inside that
        // window, so the reborrow is sound.
        let db: &AssetDatabase = unsafe { &*raw };
        Some(f(&Resolver { db }))
    })
}

/// Convenience: resolve an asset reference string to an owned path string.
///
/// The string is treated as a GUID if it has the canonical UUID shape
/// (36 chars, 4 hyphens at positions 8/13/18/23) — in that case we look up
/// the path via the thread-local [`AssetDatabase`] and return `None` when
/// no database is set or the GUID is unknown.
///
/// Strings that don't match the GUID shape are treated as already being a
/// project-relative path and returned as-is. This keeps the sandbox /
/// runtime crates working without an `AssetDatabase` while the editor
/// still benefits from GUID indirection in `.scene` files.
pub fn resolve_path(reference: &str) -> Option<String> {
    if reference.is_empty() {
        return None;
    }
    if looks_like_guid(reference) {
        with_resolver(|r| r.path_for(reference).map(str::to_string)).flatten()
    } else {
        Some(reference.to_string())
    }
}

/// Heuristic: a 36-character string with hyphens at the canonical UUID
/// positions is treated as a GUID. Avoids dragging in the `uuid` crate
/// just for shape detection.
fn looks_like_guid(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() == 36
        && b[8]  == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
}

/// Convenience: reverse-resolve a project-relative path to its asset GUID.
/// Strings already in GUID shape are returned as-is, so callers can pass
/// either form interchangeably.
pub fn resolve_guid(reference: &str) -> Option<String> {
    if reference.is_empty() {
        return None;
    }
    if looks_like_guid(reference) {
        Some(reference.to_string())
    } else {
        with_resolver(|r| r.guid_for(reference).map(str::to_string)).flatten()
    }
}
