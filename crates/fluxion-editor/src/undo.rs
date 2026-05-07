// ============================================================
// undo.rs — Undo/Redo stack for the editor
//
// Supports both field-level edits AND structural operations
// (entity create, delete, duplicate, reparent).
//
// Each UndoEntry holds the *inverse* action needed to reverse
// the operation.  On undo: pop from `undos`, compute the redo
// inverse, apply the undo action, push redo.
// ============================================================

use std::time::Instant;

use crate::rune_bindings::PendingEdit;
use fluxion_core::scene::SerializedComponent;
use fluxion_core::{ComponentRegistry, ECSWorld, EntityId};

/// Window in which consecutive field edits on the same `(entity, component, field)`
/// tuple are coalesced into a single undo entry. Tuned for slider/gizmo drags.
const COALESCE_WINDOW_MS: u128 = 250;

/// Full snapshot of an entity — enough to recreate it from scratch.
#[derive(Clone)]
pub struct EntitySnapshot {
    pub name:       String,
    pub uuid:       Option<String>,
    pub parent_bits: Option<u64>,
    pub tags:       Vec<String>,
    pub components: Vec<SerializedComponent>,
    pub prefab_source: Option<String>,
    pub prefab_overrides: std::collections::HashMap<String, serde_json::Value>,
}

/// Capture everything about an entity so it can be restored later.
pub fn snapshot_entity(
    world:    &ECSWorld,
    entity:   EntityId,
    registry: &ComponentRegistry,
) -> EntitySnapshot {
    let name = world.get_name(entity).to_string();
    let uuid = world.get_uuid(entity).map(str::to_string);
    let parent_bits = world.get_parent(entity).map(|p| p.to_bits());
    let tags: Vec<String> = world.tags_of(entity).map(str::to_string).collect();
    let prefab_source = world.get_prefab_source(entity).map(str::to_string);
    let prefab_overrides = world.get_prefab_overrides(entity)
        .cloned()
        .unwrap_or_default();

    let mut components = Vec::new();
    for &comp_type in world.component_names(entity) {
        if let Some(reflected) = registry.get_reflect(comp_type, world, entity) {
            components.push(SerializedComponent {
                component_type: comp_type.to_string(),
                data: reflected.to_serialized_data(),
            });
        }
    }

    EntitySnapshot { name, uuid, parent_bits, tags, components, prefab_source, prefab_overrides }
}

/// Respawn an entity from a snapshot. Returns the new EntityId.
fn respawn_from_snapshot(
    world:    &mut ECSWorld,
    snap:     &EntitySnapshot,
    registry: &ComponentRegistry,
) -> EntityId {
    let eid = world.spawn(Some(&snap.name));

    if let Some(ref uuid) = snap.uuid {
        world.set_uuid(eid, uuid.clone());
    }
    if let Some(ref src) = snap.prefab_source {
        world.set_prefab_source(eid, src.clone());
    }
    for (key, val) in &snap.prefab_overrides {
        world.set_prefab_override(eid, key.clone(), val.clone());
    }

    for tag in &snap.tags {
        world.add_tag(eid, tag);
    }

    for comp in &snap.components {
        if let Err(e) = registry.attach(&comp.component_type, &comp.data, world, eid) {
            log::warn!("undo respawn: attach '{}' failed: {e}", comp.component_type);
        }
    }

    if let Some(parent_bits) = snap.parent_bits {
        let parent = world.all_entities().find(|e| e.to_bits() == parent_bits);
        if let Some(p) = parent {
            world.set_parent(eid, Some(p), false);
        }
    }

    eid
}

/// A single undoable action.
#[derive(Clone)]
pub enum UndoAction {
    /// Inverse field-level edits (the old approach).
    FieldEdits(Vec<PendingEdit>),
    /// Undo a spawn: despawn the entity that was created.
    DespawnEntity { entity_bits: u64 },
    /// Undo a despawn: respawn from snapshot.
    RespawnEntity { snapshot: EntitySnapshot },
    /// Undo a reparent: restore old parent.
    Reparent { entity_bits: u64, old_parent_bits: Option<u64> },
    /// Multiple actions grouped (e.g. multi-entity delete).
    Batch(Vec<UndoAction>),
}

pub struct UndoEntry {
    pub label:  String,
    pub action: UndoAction,
}

pub struct UndoStack {
    undos: Vec<UndoEntry>,
    redos: Vec<UndoEntry>,
    capacity: usize,
    /// Wall-clock time of the most recent push, used by [`push_field_edit_coalesced`]
    /// to decide whether a new edit on the same field should merge into the
    /// previous entry (drag) or start a fresh entry (discrete click).
    last_push_at: Option<Instant>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            undos:    Vec::new(),
            redos:    Vec::new(),
            capacity: 200,
            last_push_at: None,
        }
    }

    /// Push a new undoable operation. Clears the redo stack.
    pub fn push_action(&mut self, label: impl Into<String>, action: UndoAction) {
        self.redos.clear();
        self.undos.push(UndoEntry { label: label.into(), action });
        if self.undos.len() > self.capacity {
            self.undos.remove(0);
        }
        self.last_push_at = Some(Instant::now());
    }

    /// Push a single field-edit inverse with temporal coalescing.
    ///
    /// If the previous undo entry was a single-field `FieldEdits` on the same
    /// `(entity, component, field)` tuple AND was pushed within
    /// [`COALESCE_WINDOW_MS`], the new push is dropped — the existing inverse
    /// already points to the original pre-drag value, which is what undo should
    /// restore.
    ///
    /// Use this for slider / gizmo drags that fire many edits per second.
    /// The 200-entry undo cap (line 125) used to exhaust in seconds; now a
    /// single drag occupies a single entry.
    pub fn push_field_edit_coalesced(
        &mut self,
        label:   impl Into<String>,
        inverse: PendingEdit,
    ) {
        let now = Instant::now();
        if let (Some(last_at), Some(last_entry)) = (self.last_push_at, self.undos.last()) {
            if now.duration_since(last_at).as_millis() < COALESCE_WINDOW_MS {
                if let UndoAction::FieldEdits(prev) = &last_entry.action {
                    if prev.len() == 1
                        && prev[0].entity    == inverse.entity
                        && prev[0].component == inverse.component
                        && prev[0].field     == inverse.field
                    {
                        // Coalesce: keep the original inverse, just slide the window.
                        self.last_push_at = Some(now);
                        return;
                    }
                }
            }
        }
        self.push_action(label, UndoAction::FieldEdits(vec![inverse]));
    }

    pub fn can_undo(&self) -> bool { !self.undos.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redos.is_empty() }

    #[allow(dead_code)]
    pub fn undo_label(&self) -> Option<&str> {
        self.undos.last().map(|e| e.label.as_str())
    }

    #[allow(dead_code)]
    pub fn redo_label(&self) -> Option<&str> {
        self.redos.last().map(|e| e.label.as_str())
    }

    pub fn undo(
        &mut self,
        world:    &mut ECSWorld,
        registry: &ComponentRegistry,
    ) -> bool {
        let Some(entry) = self.undos.pop() else { return false };
        let redo_action = apply_action(&entry.action, world, registry);
        self.redos.push(UndoEntry {
            label:  entry.label,
            action: redo_action,
        });
        true
    }

    pub fn redo(
        &mut self,
        world:    &mut ECSWorld,
        registry: &ComponentRegistry,
    ) -> bool {
        let Some(entry) = self.redos.pop() else { return false };
        let undo_action = apply_action(&entry.action, world, registry);
        self.undos.push(UndoEntry {
            label:  entry.label,
            action: undo_action,
        });
        true
    }
}

// ── Core apply logic ────────────────────────────────────────────────────────

/// Apply an action and return the inverse action for redo/undo.
fn apply_action(
    action:   &UndoAction,
    world:    &mut ECSWorld,
    registry: &ComponentRegistry,
) -> UndoAction {
    match action {
        UndoAction::FieldEdits(edits) => {
            let inverse = capture_field_inverses(edits, world, registry);
            apply_field_edits(edits, world, registry);
            UndoAction::FieldEdits(inverse)
        }
        UndoAction::DespawnEntity { entity_bits } => {
            let entity = world.all_entities().find(|e| e.to_bits() == *entity_bits);
            if let Some(eid) = entity {
                let snap = snapshot_entity(world, eid, registry);
                world.despawn(eid);
                UndoAction::RespawnEntity { snapshot: snap }
            } else {
                log::warn!("undo: entity {entity_bits} not found for despawn");
                UndoAction::Batch(Vec::new())
            }
        }
        UndoAction::RespawnEntity { snapshot } => {
            let eid = respawn_from_snapshot(world, snapshot, registry);
            fluxion_core::transform::system::TransformSystem::update(world);
            UndoAction::DespawnEntity { entity_bits: eid.to_bits() }
        }
        UndoAction::Reparent { entity_bits, old_parent_bits } => {
            let entity = world.all_entities().find(|e| e.to_bits() == *entity_bits);
            if let Some(eid) = entity {
                let current_parent_bits = world.get_parent(eid).map(|p| p.to_bits());
                let new_parent = old_parent_bits.and_then(|bits| {
                    world.all_entities().find(|e| e.to_bits() == bits)
                });
                world.set_parent(eid, new_parent, false);
                fluxion_core::transform::system::TransformSystem::update(world);
                UndoAction::Reparent { entity_bits: *entity_bits, old_parent_bits: current_parent_bits }
            } else {
                log::warn!("undo: entity {entity_bits} not found for reparent");
                UndoAction::Batch(Vec::new())
            }
        }
        UndoAction::Batch(actions) => {
            let mut inverses: Vec<UndoAction> = Vec::with_capacity(actions.len());
            for a in actions {
                inverses.push(apply_action(a, world, registry));
            }
            inverses.reverse();
            UndoAction::Batch(inverses)
        }
    }
}

// ── Field-edit helpers (unchanged from original) ────────────────────────────

fn capture_field_inverses(
    edits:    &[PendingEdit],
    world:    &ECSWorld,
    registry: &ComponentRegistry,
) -> Vec<PendingEdit> {
    let mut out = Vec::new();
    for edit in edits {
        if !edit.entity.is_valid() {
            continue;
        }
        if let Some(reflect) = registry.get_reflect(&edit.component, world, edit.entity) {
            if let Some(current_val) = reflect.get_field(&edit.field) {
                out.push(PendingEdit {
                    entity:    edit.entity,
                    component: edit.component.clone(),
                    field:     edit.field.clone(),
                    value:     current_val,
                });
            }
        }
    }
    out
}

fn apply_field_edits(
    edits:    &[PendingEdit],
    world:    &ECSWorld,
    registry: &ComponentRegistry,
) {
    for edit in edits {
        if !edit.entity.is_valid() {
            continue;
        }
        if let Err(e) = registry.set_reflect_field(
            &edit.component,
            world,
            edit.entity,
            &edit.field,
            edit.value.clone(),
        ) {
            log::warn!("UndoStack::apply_field_edits: {e}");
        }
    }
}
