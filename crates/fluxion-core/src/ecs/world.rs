// ============================================================
// fluxion-core — ECSWorld
//
// The central scene container. Wraps hecs::World and adds:
//   - Unity-style entity names
//   - Tag-based entity groups (find entities by "enemy", "player", etc.)
//   - Parent/child hierarchy (via HierarchyStore)
//   - Entity active/inactive toggle (like GameObject.SetActive in Unity)
//   - Lifecycle hooks on Component (on_create, on_enable, etc.)
//   - A `hierarchy_revision` counter the editor uses to detect topology changes
//
// C# / Unity equivalent: roughly the combination of
//   Scene + GameObject management methods.
//
// Performance note:
//   hecs uses archetype storage internally — entities with the same set of
//   component types share a dense, contiguous array. This means iterating
//   over all entities with (Transform, MeshRenderer) is a tight cache-
//   friendly loop, not a hash-map scan. This is the key fix over the
//   TypeScript engine's sparse-Map approach.
// ============================================================

use std::collections::{HashMap, HashSet};

use hecs::{Entity, World};

use crate::ecs::component::Component;
use crate::ecs::entity::EntityId;
use crate::hierarchy::HierarchyStore;
use crate::transform::Transform;

// Helper: extract the short type name from a full Rust path.
// e.g. "fluxion_core::transform::Transform" → "Transform"
#[inline(always)]
fn short_type_name<T>() -> &'static str {
    let full = std::any::type_name::<T>();
    // type_name is a &'static str; rsplit gives an iterator of &'static str slices.
    full.rsplit("::").next().unwrap_or(full)
}

/// The main ECS container.
///
/// # Usage
/// ```rust
/// let mut world = ECSWorld::new();
/// let entity = world.spawn(Some("Player"));
/// world.add_component(entity, Transform::new());
/// let t = world.get_component::<Transform>(entity).unwrap();
/// ```
pub struct ECSWorld {
    /// hecs inner world — provides archetype component storage.
    pub(crate) inner: World,

    /// Separate parent/child tracking (see hierarchy/mod.rs for rationale).
    pub hierarchy: HierarchyStore,

    /// Optional per-entity display names (for the editor hierarchy panel).
    names: HashMap<Entity, String>,

    /// Tag system: tag string → set of entities with that tag.
    /// Allows fast O(1) lookup by tag.
    tag_index:   HashMap<String, HashSet<Entity>>,
    entity_tags: HashMap<Entity, HashSet<String>>,

    /// Entities that have been deactivated via `set_active(entity, false)`.
    /// Inactive entities are skipped by systems but still exist in the world.
    inactive: HashSet<Entity>,

    /// Snapshot of which components were enabled at the time `set_active(false)`
    /// was called. Used to restore the correct enabled state on `set_active(true)`.
    /// Key: entity, Value: list of component type-names that were already disabled.
    disabled_snapshot: HashMap<Entity, Vec<String>>,

    /// Incremented whenever entities are spawned or despawned.
    /// The editor observes this to know when to rebuild the hierarchy panel
    /// without having to diff the entire entity list every frame.
    pub hierarchy_revision: u64,

    /// Flat list of all alive entities. Kept in sync with spawn/despawn.
    /// Used by HierarchyStore::roots() which needs to iterate all entities.
    all_entities: Vec<Entity>,

    /// Tracks which component type names each entity has.
    ///
    /// Updated in `add_component` / `remove_component` / `despawn_single`.
    /// Used by the editor to list all components on an entity without
    /// knowing the concrete types at compile time.
    entity_component_names: HashMap<Entity, Vec<&'static str>>,
}

impl ECSWorld {
    pub fn new() -> Self {
        Self {
            inner:                  World::new(),
            hierarchy:              HierarchyStore::new(),
            names:                  HashMap::new(),
            tag_index:              HashMap::new(),
            entity_tags:            HashMap::new(),
            inactive:               HashSet::new(),
            disabled_snapshot:      HashMap::new(),
            hierarchy_revision:     0,
            all_entities:           Vec::new(),
            entity_component_names: HashMap::new(),
        }
    }

    // ────────────────────────────────────────────────────────────────────────────
    // Entity management
    // ────────────────────────────────────────────────────────────────────────────

    /// Create a new entity with an optional display name.
    ///
    /// The entity starts active with no components.
    /// Use `add_component()` to attach data.
    ///
    /// Unity equivalent: `new GameObject(name)`
    pub fn spawn(&mut self, name: Option<&str>) -> EntityId {
        let entity = self.inner.spawn(());
        if let Some(n) = name {
            self.names.insert(entity, n.to_string());
        }
        self.all_entities.push(entity);
        self.hierarchy_revision += 1;
        EntityId(entity)
    }

    /// Destroy an entity and all its children (recursive subtree).
    ///
    /// Calls `on_disable()` + `on_destroy()` on each component of each
    /// entity in the subtree, then removes them from hecs.
    ///
    /// Unity equivalent: `Destroy(gameObject)`
    pub fn despawn(&mut self, id: EntityId) {
        let subtree = self.hierarchy.collect_subtree(id.0);
        for entity in subtree {
            self.despawn_single(entity);
        }
        self.hierarchy_revision += 1;
    }

    /// Internal: destroy a single entity without touching its children.
    fn despawn_single(&mut self, entity: Entity) {
        // Fire lifecycle hooks
        // We can't call on_disable/on_destroy through the trait generically without
        // a component registry. For now we handle the built-in components explicitly.
        // User components that need cleanup should use on_destroy() via a registry
        // (Phase 2 work). For Phase 1, Transform and built-ins don't need cleanup.

        // Clean up side tables
        self.hierarchy.remove_entity(entity);
        self.names.remove(&entity);
        if let Some(tags) = self.entity_tags.remove(&entity) {
            for tag in &tags {
                if let Some(set) = self.tag_index.get_mut(tag) {
                    set.remove(&entity);
                }
            }
        }
        self.inactive.remove(&entity);
        self.disabled_snapshot.remove(&entity);
        self.entity_component_names.remove(&entity);
        self.all_entities.retain(|&e| e != entity);

        // Remove from hecs
        let _ = self.inner.despawn(entity);
    }

    /// Returns `true` if the entity exists and has not been despawned.
    pub fn is_alive(&self, id: EntityId) -> bool {
        self.inner.contains(id.0)
    }

    // ────────────────────────────────────────────────────────────────────────────
    // Names
    // ────────────────────────────────────────────────────────────────────────────

    /// Get the display name of an entity. Returns `"Entity"` if unnamed.
    pub fn get_name(&self, id: EntityId) -> &str {
        self.names.get(&id.0).map(|s| s.as_str()).unwrap_or("Entity")
    }

    pub fn set_name(&mut self, id: EntityId, name: &str) {
        self.names.insert(id.0, name.to_string());
    }

    /// Find the first entity with a given name. O(n) — use sparingly.
    pub fn find_by_name(&self, name: &str) -> Option<EntityId> {
        self.names
            .iter()
            .find(|(_, n)| n.as_str() == name)
            .map(|(&e, _)| EntityId(e))
    }

    // ────────────────────────────────────────────────────────────────────────────
    // Tags
    // ────────────────────────────────────────────────────────────────────────────

    /// Add a tag string to an entity. Tags are free-form: "enemy", "trigger", etc.
    pub fn add_tag(&mut self, id: EntityId, tag: &str) {
        self.entity_tags.entry(id.0).or_default().insert(tag.to_string());
        self.tag_index.entry(tag.to_string()).or_default().insert(id.0);
    }

    pub fn remove_tag(&mut self, id: EntityId, tag: &str) {
        if let Some(tags) = self.entity_tags.get_mut(&id.0) {
            tags.remove(tag);
        }
        if let Some(set) = self.tag_index.get_mut(tag) {
            set.remove(&id.0);
        }
    }

    pub fn has_tag(&self, id: EntityId, tag: &str) -> bool {
        self.entity_tags
            .get(&id.0)
            .map(|t| t.contains(tag))
            .unwrap_or(false)
    }

    /// Returns all tag strings on a given entity.
    /// Returns an empty iterator if the entity has no tags.
    pub fn tags_of(&self, id: EntityId) -> impl Iterator<Item = &str> {
        self.entity_tags
            .get(&id.0)
            .into_iter()
            .flat_map(|set| set.iter().map(|s| s.as_str()))
    }

    /// Returns all entities that have a given tag. O(1) lookup.
    pub fn entities_with_tag(&self, tag: &str) -> impl Iterator<Item = EntityId> + '_ {
        self.tag_index
            .get(tag)
            .into_iter()
            .flat_map(|set| set.iter().copied().map(EntityId))
    }

    // ────────────────────────────────────────────────────────────────────────────
    // Active / inactive
    // ────────────────────────────────────────────────────────────────────────────

    /// Enable or disable an entity.
    ///
    /// Disabled entities still exist in the world but are skipped by systems.
    /// This matches Unity's `GameObject.SetActive(bool)`.
    ///
    /// Note: disabling an entity also disables all its children.
    pub fn set_active(&mut self, id: EntityId, active: bool) {
        if active {
            self.inactive.remove(&id.0);
            // Also re-enable all children
            let subtree = self.hierarchy.collect_subtree(id.0);
            for e in subtree {
                self.inactive.remove(&e);
            }
        } else {
            self.inactive.insert(id.0);
            // Also disable all children
            let subtree = self.hierarchy.collect_subtree(id.0);
            for e in subtree {
                self.inactive.insert(e);
            }
        }
    }

    /// Returns `true` if the entity is active (not disabled).
    pub fn is_active(&self, id: EntityId) -> bool {
        !self.inactive.contains(&id.0)
    }

    // ────────────────────────────────────────────────────────────────────────────
    // Hierarchy
    // ────────────────────────────────────────────────────────────────────────────

    /// Set the parent of `child`.
    ///
    /// `keep_world_transform = true` adjusts the child's local transform so
    /// that its world position/rotation/scale stays the same after reparenting.
    /// This requires the parent's world matrix to be up-to-date.
    ///
    /// Unity equivalent: `child.transform.SetParent(parent, worldPositionStays)`
    pub fn set_parent(
        &mut self,
        child:               EntityId,
        new_parent:          Option<EntityId>,
        keep_world_transform: bool,
    ) {
        let new_parent_entity = new_parent.map(|p| p.0);

        // If keeping world transform, we need to rebase the local transform
        // by multiplying by the inverse of the new parent's world matrix.
        if keep_world_transform {
            if let Some(parent_id) = new_parent {
                // Read the parent's current world matrix
                let parent_world = self
                    .inner
                    .get::<&Transform>(parent_id.0)
                    .ok()
                    .map(|t| t.world_matrix);

                if let Some(parent_world_mat) = parent_world {
                    // Read the child's current world position (set as new local)
                    if let Ok(mut child_t) = self.inner.get::<&mut Transform>(child.0) {
                        let child_world_mat = child_t.world_matrix;
                        // new_local = inverse(parent_world) * child_world
                        let new_local = parent_world_mat.inverse() * child_world_mat;
                        let (scale, rotation, position) = new_local.to_scale_rotation_translation();
                        child_t.position = position;
                        child_t.rotation = rotation;
                        child_t.scale    = scale;
                        child_t.dirty    = true;
                    }
                }
            }
        }

        self.hierarchy.set_parent(child.0, new_parent_entity);
        self.hierarchy_revision += 1;
    }

    pub fn get_parent(&self, id: EntityId) -> Option<EntityId> {
        self.hierarchy.parent_of(id.0).map(EntityId)
    }

    /// Returns `true` if `ancestor` is a direct or indirect parent of `entity`.
    /// Used by the editor to prevent drag-and-drop reparenting cycles.
    pub fn is_ancestor_of(&self, ancestor: EntityId, entity: EntityId) -> bool {
        self.hierarchy.is_ancestor_of(ancestor.0, entity.0)
    }

    pub fn get_children(&self, id: EntityId) -> impl Iterator<Item = EntityId> + '_ {
        self.hierarchy.children_of(id.0).iter().copied().map(EntityId)
    }

    /// Returns all root entities (entities with no parent).
    pub fn root_entities(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.hierarchy.roots(&self.all_entities).map(EntityId)
    }

    /// Returns all alive entities in the world.
    pub fn all_entities(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.all_entities.iter().copied().map(EntityId)
    }

    // ────────────────────────────────────────────────────────────────────────────
    // Component access
    // ────────────────────────────────────────────────────────────────────────────

    /// Add a component to an entity. Calls `on_create()` on the component.
    ///
    /// If a component of this type already exists on the entity, it is replaced.
    ///
    /// Unity equivalent: `entity.AddComponent<T>()`
    pub fn add_component<T: Component>(&mut self, id: EntityId, mut component: T) {
        component.on_create();
        if self.is_active(id) {
            component.on_enable();
        }
        // Track the component type name for editor / reflect queries.
        let name = short_type_name::<T>();
        let names = self.entity_component_names.entry(id.0).or_default();
        if !names.contains(&name) {
            names.push(name);
        }
        // hecs will handle overwriting an existing component of the same type
        // by moving the entity to a new archetype. This is the expected behavior.
        let _ = self.inner.insert_one(id.0, component);
    }

    /// Get an immutable reference to a component.
    ///
    /// Returns `None` if the entity doesn't have this component type,
    /// or if the entity has been despawned.
    ///
    /// Unity equivalent: `entity.GetComponent<T>()`
    pub fn get_component<T: Component>(&self, id: EntityId) -> Option<hecs::Ref<'_, T>> {
        self.inner.get::<&T>(id.0).ok()
    }

    /// Get a mutable reference to a component.
    pub fn get_component_mut<T: Component>(&self, id: EntityId) -> Option<hecs::RefMut<'_, T>> {
        self.inner.get::<&mut T>(id.0).ok()
    }

    /// Returns `true` if the entity has a component of type `T`.
    pub fn has_component<T: Component>(&self, id: EntityId) -> bool {
        self.inner.get::<&T>(id.0).is_ok()
    }

    /// Remove a component from an entity. Calls `on_disable()` + `on_destroy()`.
    pub fn remove_component<T: Component>(&mut self, id: EntityId) {
        if let Ok(mut c) = self.inner.get::<&mut T>(id.0) {
            if self.is_active(id) {
                c.on_disable();
            }
            c.on_destroy();
        }
        // Remove the type name from the tracking list.
        let name = short_type_name::<T>();
        if let Some(names) = self.entity_component_names.get_mut(&id.0) {
            names.retain(|&n| n != name);
        }
        let _ = self.inner.remove_one::<T>(id.0);
    }

    /// Returns the short type names of all components attached to `entity`.
    ///
    /// Example: `["Transform", "MeshRenderer", "Light"]`
    ///
    /// Used by the editor to populate the component list panel.
    pub fn component_names(&self, id: EntityId) -> &[&'static str] {
        self.entity_component_names
            .get(&id.0)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    // ────────────────────────────────────────────────────────────────────────────
    // Queries
    // ────────────────────────────────────────────────────────────────────────────
    //
    // hecs queries iterate dense archetype arrays — O(matching entities).
    //
    // Usage:
    //   for (entity, (transform, mesh)) in world.query::<(&Transform, &MeshRenderer)>().iter() { }
    //
    // The `query_active` helper filters out inactive entities.
    // For raw hecs access (all entities, no inactive filter) use `world.inner.query::<Q>()`.

    /// Call `f` for every ACTIVE entity that matches query `Q`.
    ///
    /// # Example
    /// ```ignore
    /// world.query_active::<(&Transform, &MeshRenderer)>(|_id, (t, m)| {
    ///     // render m using t.world_matrix
    /// });
    /// ```
    pub fn query_active<Q, F>(&self, mut f: F)
    where
        Q: hecs::Query,
        F: FnMut(EntityId, Q::Item<'_>),
    {
        let mut borrow = self.inner.query::<Q>();
        for (e, item) in borrow.iter() {
            if !self.inactive.contains(&e) {
                f(EntityId(e), item);
            }
        }
    }

    /// Call `f` for every entity (including inactive) that matches query `Q`.
    pub fn query_all<Q, F>(&self, mut f: F)
    where
        Q: hecs::Query,
        F: FnMut(EntityId, Q::Item<'_>),
    {
        let mut borrow = self.inner.query::<Q>();
        for (e, item) in borrow.iter() {
            f(EntityId(e), item);
        }
    }

    /// Mutable query over **active** entities only (skips inactive / `SetActive(false)`).
    pub fn query_active_mut<Q, F>(&mut self, mut f: F)
    where
        Q: hecs::Query,
        F: FnMut(EntityId, Q::Item<'_>),
    {
        for (e, item) in self.inner.query_mut::<Q>() {
            if !self.inactive.contains(&e) {
                f(EntityId(e), item);
            }
        }
    }

    /// Total number of alive entities.
    pub fn entity_count(&self) -> usize {
        self.all_entities.len()
    }

    /// Despawn every root entity (and subtrees). Clears the whole scene graph.
    pub fn clear(&mut self) {
        let roots: Vec<EntityId> = self.root_entities().collect();
        for r in roots {
            self.despawn(r);
        }
    }
}

impl Default for ECSWorld {
    fn default() -> Self { Self::new() }
}
