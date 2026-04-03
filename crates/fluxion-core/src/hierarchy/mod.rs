// ============================================================
// fluxion-core — Hierarchy store
//
// Parent/child relationships are stored HERE, not inside components.
// Keeping them separate from hecs component storage avoids "archetype
// thrashing" — every time a component is added/removed, hecs moves the
// entity to a new archetype. If we stored parent/child as components,
// parenting would trigger that churn on every set_parent() call.
//
// This mirrors the design of the TypeScript ECSManager which maintained
// `parentMap` and `childrenMap` as separate Maps alongside the component
// storage.
//
// All operations are O(1) using HashMaps, except get_children() which
// returns a slice and is O(1) to access (but the slice length is the
// number of children).
// ============================================================

use hecs::Entity;
use std::collections::HashMap;

/// Stores the parent/child hierarchy for all entities.
///
/// - Every entity with a parent has an entry in `parents`.
/// - Every entity with children has an entry in `children`.
/// - Root entities (no parent) have no entry in `parents`.
pub struct HierarchyStore {
    /// child → parent
    parents: HashMap<Entity, Entity>,
    /// parent → ordered list of children
    children: HashMap<Entity, Vec<Entity>>,
}

impl HierarchyStore {
    pub fn new() -> Self {
        Self {
            parents:  HashMap::new(),
            children: HashMap::new(),
        }
    }

    // ── Queries ────────────────────────────────────────────────────────────────

    /// Returns the parent of `entity`, or `None` if it is a root entity.
    #[inline]
    pub fn parent_of(&self, entity: Entity) -> Option<Entity> {
        self.parents.get(&entity).copied()
    }

    /// Returns the children of `entity` as a slice.
    /// Returns an empty slice if the entity has no children.
    #[inline]
    pub fn children_of(&self, entity: Entity) -> &[Entity] {
        self.children
            .get(&entity)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Returns `true` if `entity` has no parent.
    #[inline]
    pub fn is_root(&self, entity: Entity) -> bool {
        !self.parents.contains_key(&entity)
    }

    /// Iterates over all root entities (entities with no parent).
    /// Used by TransformSystem to seed the BFS traversal.
    pub fn roots<'a>(&'a self, all_entities: &'a [Entity]) -> impl Iterator<Item = Entity> + 'a {
        all_entities
            .iter()
            .copied()
            .filter(|e| self.is_root(*e))
    }

    // ── Mutations ─────────────────────────────────────────────────────────────

    /// Set `child`'s parent to `new_parent`.
    /// Pass `None` to make `child` a root entity.
    ///
    /// # Panics
    /// Panics in debug builds if setting the parent would create a cycle
    /// (e.g., making an entity its own ancestor). In release builds this
    /// is a no-op to avoid UB.
    pub fn set_parent(&mut self, child: Entity, new_parent: Option<Entity>) {
        // Validate: prevent cycles
        if let Some(parent) = new_parent {
            debug_assert!(
                !self.is_ancestor_of(parent, child),
                "set_parent() would create a hierarchy cycle: {:?} is already a descendant of {:?}",
                parent,
                child
            );
            if self.is_ancestor_of(parent, child) {
                log::warn!("set_parent(): cycle detected, ignoring");
                return;
            }
        }

        // Remove from old parent's children list
        if let Some(old_parent) = self.parents.remove(&child) {
            if let Some(siblings) = self.children.get_mut(&old_parent) {
                siblings.retain(|&e| e != child);
                if siblings.is_empty() {
                    self.children.remove(&old_parent);
                }
            }
        }

        // Register new parent
        if let Some(parent) = new_parent {
            self.parents.insert(child, parent);
            self.children.entry(parent).or_default().push(child);
        }
        // If new_parent is None, the entity is already removed from parents above.
    }

    /// Remove all hierarchy data for `entity` (called on despawn).
    /// Also removes the entity from its parent's child list.
    pub fn remove_entity(&mut self, entity: Entity) {
        // Detach from parent
        self.set_parent(entity, None);
        // Any children become roots (their parent entry is removed)
        if let Some(kids) = self.children.remove(&entity) {
            for child in kids {
                self.parents.remove(&child);
            }
        }
    }

    /// Returns `true` if `ancestor` is a (direct or indirect) ancestor of `entity`.
    /// Used for cycle detection in `set_parent()`.
    pub fn is_ancestor_of(&self, ancestor: Entity, entity: Entity) -> bool {
        let mut current = entity;
        // Walk up the parent chain. Max depth is bounded by hierarchy depth.
        // In practice game hierarchies are shallow (< 20 levels).
        for _ in 0..1024 {
            match self.parents.get(&current).copied() {
                None => return false,
                Some(p) if p == ancestor => return true,
                Some(p) => current = p,
            }
        }
        // If we get here, something is very wrong (existing cycle in the store)
        log::error!("is_ancestor_of(): walked 1024 levels — possible existing cycle!");
        false
    }

    /// Collect all descendants of `entity` (inclusive) in BFS order.
    /// Used by `ECSWorld::despawn()` to destroy an entire subtree.
    pub fn collect_subtree(&self, root: Entity) -> Vec<Entity> {
        let mut result = Vec::new();
        let mut queue = vec![root];
        let mut head  = 0;
        while head < queue.len() {
            let e = queue[head];
            head += 1;
            result.push(e);
            for &child in self.children_of(e) {
                queue.push(child);
            }
        }
        result
    }
}

impl Default for HierarchyStore {
    fn default() -> Self { Self::new() }
}
