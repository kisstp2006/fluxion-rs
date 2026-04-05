// ============================================================
// fluxion-core — EventBus
//
// A type-erased pub/sub event system, equivalent to the TypeScript
// EventSystem. Key design decisions:
//
//   - String event keys (same as TS: "engine:update", "entity:created")
//     → familiar, decoupled, no boilerplate for new events
//   - Priority-ordered dispatch (higher priority = called first)
//     → matches TS EventSystem behavior for systems that depend on ordering
//   - RAII unsubscription via EventHandle
//     → matches TS "return () => this.off(event, callback)" pattern
//   - Type-erased storage (Box<dyn FnMut>) with a thin type-checking wrapper
//     → avoids generics on EventBus itself (simpler for C++/C# developers)
//
// IMPORTANT: EventBus is NOT thread-safe. It is designed for use on a
// single thread (the main game loop). For cross-thread communication use
// channels (e.g. tokio::sync::mpsc).
// ============================================================

use std::any::Any;
use std::collections::HashMap;

// ── Internal types ─────────────────────────────────────────────────────────────

type ListenerId = u64;

struct ListenerEntry {
    id:       ListenerId,
    priority: i32,
    once:     bool,
    /// Type-erased callback. The actual function pointer is stored as
    /// `Box<dyn FnMut(&dyn Any)>`. The type-checking happens at emit time:
    /// if the emitted type doesn't match what the subscriber expects, the
    /// call is silently skipped (no panic).
    callback: Box<dyn FnMut(&dyn Any)>,
}

// ── Public types ───────────────────────────────────────────────────────────────

/// The main event dispatcher.
///
/// # Example
/// ```rust
/// let mut bus = EventBus::new();
///
/// // Subscribe
/// let _handle = bus.on(EngineEvent::UPDATE, |dt: &f32| {
///     println!("frame dt = {dt:.4}s");
/// }, 0);
///
/// // Emit
/// bus.emit(EngineEvent::UPDATE, 0.016_f32);
///
/// // _handle drops here → automatically unsubscribed
/// ```
pub struct EventBus {
    /// event name → sorted list of listeners (sorted by priority descending)
    listeners:   HashMap<&'static str, Vec<ListenerEntry>>,
    next_id:     ListenerId,
    /// Pending removes accumulated during an emit() call.
    /// We can't mutate listeners while iterating, so we queue removals.
    pending_remove: Vec<(&'static str, ListenerId)>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            listeners:      HashMap::new(),
            next_id:        1,
            pending_remove: Vec::new(),
        }
    }

    // ── Subscribe ─────────────────────────────────────────────────────────────

    /// Subscribe to an event. The callback runs every time the event is emitted.
    ///
    /// Returns an `EventHandle` — dropping it automatically unsubscribes.
    /// Store the handle somewhere for as long as you want to receive events.
    ///
    /// `priority`: higher values run first. Use 0 for normal priority.
    pub fn on<T: Any + 'static>(
        &mut self,
        event:    &'static str,
        callback: impl FnMut(&T) + 'static,
        priority: i32,
    ) -> EventHandle {
        self.add_listener(event, callback, priority, false)
    }

    /// Subscribe for a single emission, then automatically unsubscribe.
    pub fn once<T: Any + 'static>(
        &mut self,
        event:    &'static str,
        callback: impl FnMut(&T) + 'static,
        priority: i32,
    ) -> EventHandle {
        self.add_listener(event, callback, priority, true)
    }

    fn add_listener<T: Any + 'static>(
        &mut self,
        event:    &'static str,
        mut callback: impl FnMut(&T) + 'static,
        priority: i32,
        once:     bool,
    ) -> EventHandle {
        let id = self.next_id;
        self.next_id += 1;

        // Wrap the typed callback in a type-erased closure.
        // The Any downcast happens here — if the wrong type is emitted,
        // the downcast returns None and we skip the call silently.
        let erased: Box<dyn FnMut(&dyn Any)> = Box::new(move |data: &dyn Any| {
            if let Some(typed) = data.downcast_ref::<T>() {
                callback(typed);
            }
        });

        let entry = ListenerEntry { id, priority, once, callback: erased };

        let vec = self.listeners.entry(event).or_default();
        vec.push(entry);
        // Keep sorted: highest priority first (descending order)
        vec.sort_by(|a, b| b.priority.cmp(&a.priority));

        EventHandle {
            event,
            id,
            // We use a raw pointer to EventBus here to avoid lifetime parameters
            // on EventHandle. This is safe because:
            //   - EventBus outlives all EventHandles (game loop lifetime)
            //   - Drop impl calls unsubscribe() which only removes a Vec entry
            bus: self as *mut EventBus,
        }
    }

    // ── Emit ──────────────────────────────────────────────────────────────────

    /// Fire an event. All active listeners for `event` are called in priority order.
    ///
    /// Type safety: listeners registered for a different type `T` than what is emitted
    /// will silently not receive the call (Any downcast fails).
    pub fn emit<T: Any + 'static>(&mut self, event: &'static str, data: T) {
        // Drain pending removes from previous emit calls
        self.flush_pending_removes();

        let listeners = match self.listeners.get_mut(event) {
            Some(l) => l,
            None    => return,
        };

        let data_any: &dyn Any = &data;
        let mut once_ids: Vec<ListenerId> = Vec::new();

        for entry in listeners.iter_mut() {
            (entry.callback)(data_any);
            if entry.once {
                once_ids.push(entry.id);
            }
        }

        // Queue once-listeners for removal after the loop
        for id in once_ids {
            self.pending_remove.push((event, id));
        }
        self.flush_pending_removes();
    }

    fn flush_pending_removes(&mut self) {
        for (event, id) in self.pending_remove.drain(..).collect::<Vec<_>>() {
            self.unsubscribe(event, id);
        }
    }

    /// Remove all listeners for a specific event.
    pub fn clear(&mut self, event: &'static str) {
        self.listeners.remove(event);
    }

    /// Remove all listeners from all events.
    pub fn clear_all(&mut self) {
        self.listeners.clear();
    }

    /// Internal unsubscribe by ID.
    fn unsubscribe(&mut self, event: &'static str, id: ListenerId) {
        if let Some(vec) = self.listeners.get_mut(event) {
            vec.retain(|e| e.id != id);
        }
    }
}

impl Default for EventBus {
    fn default() -> Self { Self::new() }
}

// ── EventHandle ───────────────────────────────────────────────────────────────

/// RAII subscription handle. Drop this to unsubscribe.
///
/// Store it in a field to keep the subscription alive for the struct's lifetime.
/// Let it drop (or call `drop(handle)`) to unsubscribe early.
///
/// Equivalent to the return value of `events.on(...)` in the TypeScript engine,
/// which returned a `() => void` unsubscribe function.
pub struct EventHandle {
    event: &'static str,
    id:    ListenerId,
    bus:   *mut EventBus,
}

impl Drop for EventHandle {
    fn drop(&mut self) {
        // SAFETY: EventBus outlives all EventHandles in the game loop.
        // This is enforced by usage convention, not the type system.
        if !self.bus.is_null() {
            unsafe { (*self.bus).unsubscribe(self.event, self.id); }
        }
    }
}

// EventHandle is NOT Send/Sync because it holds a raw pointer.
// This is intentional — it ties the handle to the owning thread.

// ── Engine event name constants ───────────────────────────────────────────────

/// Well-known engine event names. Use these constants instead of raw strings
/// to avoid typos.
///
/// Custom events are fine too — just use your own string constants.
#[allow(non_snake_case)]
pub mod EngineEvent {
    /// Fired once after all subsystems are initialized. No data.
    pub const INIT:          &str = "engine:init";
    /// Fired once when the main loop starts. No data.
    pub const START:         &str = "engine:start";
    /// Fired every frame during variable update. Data: `f32` (delta time in seconds).
    pub const UPDATE:        &str = "engine:update";
    /// Fired at fixed intervals for physics. Data: `f32` (fixed_dt in seconds).
    pub const FIXED_UPDATE:  &str = "engine:fixedUpdate";
    /// Fired after UPDATE, before render. Data: `f32` (delta time).
    pub const LATE_UPDATE:   &str = "engine:lateUpdate";
    /// Fired when the viewport is resized. Data: `(u32, u32)` (width, height).
    pub const RESIZE:        &str = "engine:resize";
    /// Fired when the engine is shutting down. No data.
    pub const DESTROY:       &str = "engine:destroy";

    /// Fired after a scene finishes loading. Data: `String` (scene name).
    pub const SCENE_LOADED:   &str = "scene:loaded";
    /// Fired before a scene is unloaded. Data: `String` (scene name).
    pub const SCENE_UNLOADED: &str = "scene:unloaded";

    /// Fired when an entity is spawned. Data: `crate::EntityId`.
    pub const ENTITY_CREATED:   &str = "entity:created";
    /// Fired when an entity is despawned. Data: `crate::EntityId`.
    pub const ENTITY_DESTROYED: &str = "entity:destroyed";

    /// Fired when an asset file changes on disk (hot-reload). Data: `String` (path).
    pub const ASSET_CHANGED:    &str = "asset:changed";

    /// Fired by physics when two non-trigger colliders first touch.
    /// Data: `(crate::EntityId, crate::EntityId)`.
    pub const COLLISION_ENTER:  &str = "physics:collision-enter";
    /// Fired while two colliders remain in contact. Data: same as ENTER.
    pub const COLLISION_STAY:   &str = "physics:collision-stay";
    /// Fired when two colliders separate. Data: same as ENTER.
    pub const COLLISION_EXIT:   &str = "physics:collision-exit";
}
