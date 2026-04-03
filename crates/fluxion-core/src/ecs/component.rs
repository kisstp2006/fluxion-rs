// ============================================================
// fluxion-core — Component trait
//
// All data components implement this trait.
// The bounds are intentionally minimal:
//   - 'static  — required by hecs for type-erased storage
//   - Send+Sync — allows components to be accessed from multiple threads
//                 (even if we're single-threaded now, it keeps the door open)
//
// The lifecycle hooks are optional (default implementations do nothing).
// Implement only the ones you need.
//
// C++ equivalent:
//   class IComponent { public: virtual void OnCreate() {} ... };
// C# equivalent:
//   interface IComponent { void OnCreate() {} ... }
//
// Lifecycle order:
//   add_component()   → on_create()  → on_enable()  (if enabled)
//   enabled = false   →               on_disable()
//   enabled = true    →               on_enable()
//   remove_component()→               on_disable() (if enabled) → on_destroy()
//   despawn()         →               on_disable() (if enabled) → on_destroy()  (for each component)
// ============================================================

/// Marker + lifecycle trait for all ECS components.
///
/// Implement this on any struct you want to attach to an entity.
/// All methods are optional — override only what you need.
///
/// # Example
/// ```rust
/// use fluxion_core::Component;
///
/// pub struct Health { pub current: f32, pub max: f32 }
///
/// impl Component for Health {
///     fn on_create(&mut self) {
///         self.current = self.max; // start at full health
///     }
/// }
/// ```
pub trait Component: 'static + Send + Sync {
    /// Called immediately after this component is added to an entity.
    /// Use this to initialize any derived state.
    fn on_create(&mut self) {}

    /// Called just before this component is removed from an entity,
    /// and also when the entity is despawned.
    /// Use this to clean up resources (e.g., release GPU handles).
    fn on_destroy(&mut self) {}

    /// Called when the owning entity (or just this component) is enabled.
    /// Also called right after `on_create()` if the entity is active.
    fn on_enable(&mut self) {}

    /// Called when the owning entity (or just this component) is disabled.
    fn on_disable(&mut self) {}
}
