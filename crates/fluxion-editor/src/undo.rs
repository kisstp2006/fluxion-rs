// ============================================================
// undo.rs — Undo/Redo stack for the editor
//
// Each UndoEntry holds the *inverse* PendingEdits needed to
// reverse the operation.  On undo: pop from `undos`, apply
// the inverse edits, push the re-do inverse onto `redos`.
// ============================================================

use crate::rune_bindings::PendingEdit;
use fluxion_core::{ComponentRegistry, ECSWorld};

pub struct UndoEntry {
    pub label:   String,
    /// Edits that will undo this operation (captured before applying the original).
    pub inverse: Vec<PendingEdit>,
}

pub struct UndoStack {
    undos: Vec<UndoEntry>,
    redos: Vec<UndoEntry>,
    /// Maximum number of undo steps kept.
    capacity: usize,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            undos:    Vec::new(),
            redos:    Vec::new(),
            capacity: 200,
        }
    }

    /// Push a new undoable operation. Clears the redo stack.
    pub fn push(&mut self, label: impl Into<String>, inverse: Vec<PendingEdit>) {
        if inverse.is_empty() {
            return;
        }
        self.redos.clear();
        self.undos.push(UndoEntry { label: label.into(), inverse });
        if self.undos.len() > self.capacity {
            self.undos.remove(0);
        }
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

    /// Pop the top undo entry and apply its inverse edits.
    /// Returns the re-do inverse (the current field values before applying).
    pub fn undo(
        &mut self,
        world:    &ECSWorld,
        registry: &ComponentRegistry,
    ) -> bool {
        let Some(entry) = self.undos.pop() else { return false };
        // Capture re-do inverses (current values before we apply undo).
        let redo_inverse = capture_inverses(&entry.inverse, world, registry);
        apply_edits(&entry.inverse, world, registry);
        self.redos.push(UndoEntry {
            label:   entry.label,
            inverse: redo_inverse,
        });
        true
    }

    /// Pop the top redo entry and apply its inverse edits.
    pub fn redo(
        &mut self,
        world:    &ECSWorld,
        registry: &ComponentRegistry,
    ) -> bool {
        let Some(entry) = self.redos.pop() else { return false };
        let undo_inverse = capture_inverses(&entry.inverse, world, registry);
        apply_edits(&entry.inverse, world, registry);
        self.undos.push(UndoEntry {
            label:   entry.label,
            inverse: undo_inverse,
        });
        true
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// For each edit in `edits`, read the *current* field value from the world
/// and build the inverse PendingEdit list.
fn capture_inverses(
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

/// Apply a list of PendingEdits directly (bypasses the queue).
fn apply_edits(
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
            log::warn!("UndoStack::apply_edits: {e}");
        }
    }
}
