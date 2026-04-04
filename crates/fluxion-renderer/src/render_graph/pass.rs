// ============================================================
// fluxion-renderer — RenderPass trait
//
// The core extensibility interface. Every render step (geometry fill,
// lighting, bloom, etc.) implements this trait.
//
// Custom passes (from JS scripts or Rust plugins) also implement this.
// The JS scripting layer wraps a JS object as a JsRenderPass that
// delegates to the JS prepare/execute functions.
//
// Why trait objects (Box<dyn RenderPass>) instead of generics?
//   - Passes are stored in a Vec and iterated in order — generics would
//     require HList or enum dispatch, both harder to extend from scripts.
//   - Each pass runs once per frame (not per-entity), so vtable overhead
//     is negligible (one pointer indirection per pass = < 1 μs total).
// ============================================================

use super::context::{RenderContext, RenderResources};

/// On native, passes are `Send + Sync` so the graph can live behind `Arc` if needed.
/// On `wasm32`, `wgpu` types are not `Send`/`Sync`, so bounds are relaxed.
#[cfg(not(target_arch = "wasm32"))]
pub trait RenderPassBounds: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync + ?Sized> RenderPassBounds for T {}

#[cfg(target_arch = "wasm32")]
pub trait RenderPassBounds {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> RenderPassBounds for T {}

/// A single render pass in the pipeline.
///
/// Implement this to add custom rendering to the engine.
///
/// # Minimal example
/// ```rust
/// struct MyOutlinePass;
///
/// impl RenderPass for MyOutlinePass {
///     fn name(&self) -> &str { "my_outline" }
///
///     fn execute(&mut self, ctx: &mut RenderContext) {
///         // Record wgpu commands using ctx.encoder
///     }
/// }
///
/// engine.renderer.render_graph.inject_pass(PassSlot::Overlay, "my_outline", Box::new(MyOutlinePass));
/// ```
pub trait RenderPass: RenderPassBounds {
    /// Unique name for this pass. Used for enable/disable and debug output.
    fn name(&self) -> &str;

    /// Called once after the device is created and shared render targets are ready.
    /// Create pipelines, bind group layouts, and other one-time GPU objects here.
    ///
    /// This is analogous to `Awake()` in Unity — runs once, before the first frame.
    fn prepare(&mut self, _device: &wgpu::Device, _resources: &RenderResources) {}

    /// Called each frame. Record GPU commands into `ctx.encoder`.
    ///
    /// This is the hot path — keep it lean. Do not create GPU objects here
    /// (create them in `prepare()` and reuse them).
    ///
    /// This is analogous to `OnRenderObject()` or `CommandBuffer.DrawMesh()` in Unity.
    fn execute(&mut self, ctx: &mut RenderContext);

    /// Called when the window is resized. Recreate any size-dependent textures
    /// (intermediate render targets, etc.) here.
    fn resize(&mut self, _device: &wgpu::Device, _width: u32, _height: u32) {}
}
