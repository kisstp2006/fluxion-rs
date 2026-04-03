// ============================================================
// fluxion-renderer — RenderGraph
//
// An ordered list of RenderPass objects executed each frame.
// Passes are grouped into named slots so scripts and plugins can
// inject custom passes at well-defined points in the pipeline.
//
// Execution order:
//   Shadow      — shadow map generation
//   Geometry    — GBuffer fill (opaque geometry)
//   Lighting    — deferred PBR light accumulation
//   Skybox      — sky / background
//   PostOpaque  — [INJECT HERE] custom passes after lighting, before post-fx
//   Ssao        — screen-space ambient occlusion
//   Bloom       — bloom effect
//   PostFx      — [INJECT HERE] custom post-processing effects
//   Tonemap     — tonemapping, final blit to surface
//   Overlay     — [INJECT HERE] UI, debug lines, no post-processing
//
// Compared to a full render graph DAG (e.g. Vulkan render passes with
// explicit dependencies), this linear approach is simpler to understand
// and debug. For the current scale of the engine it is sufficient.
// ============================================================

pub mod pass;
pub mod context;

pub use pass::RenderPass;
pub use context::{RenderContext, RenderResources};

/// Named injection slots in the render pipeline.
/// Use these when calling `RenderGraph::inject_pass()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PassSlot {
    Shadow     = 0,
    Geometry   = 1,
    Lighting   = 2,
    Skybox     = 3,
    PostOpaque = 4,  // ← inject here for custom opaque post-geometry passes
    Ssao       = 5,
    Bloom      = 6,
    PostFx     = 7,  // ← inject here for custom post-fx (before tonemap)
    Tonemap    = 8,
    Overlay    = 9,  // ← inject here for UI, debug, no tonemapping
}

struct PassEntry {
    name:    String,
    slot:    PassSlot,
    enabled: bool,
    pass:    Box<dyn RenderPass>,
}

/// The render pipeline manager.
///
/// Calling `execute()` runs all enabled passes in slot order.
pub struct RenderGraph {
    passes: Vec<PassEntry>,
}

impl RenderGraph {
    pub fn new() -> Self { Self { passes: Vec::new() } }

    /// Add a built-in or custom pass at a specific slot.
    ///
    /// Passes within the same slot execute in the order they were added.
    pub fn add_pass(
        &mut self,
        name: &str,
        slot: PassSlot,
        pass: Box<dyn RenderPass>,
    ) {
        self.passes.push(PassEntry {
            name:    name.to_string(),
            slot,
            enabled: true,
            pass,
        });
        // Sort by slot so execution order stays correct after any insertions.
        self.passes.sort_by_key(|e| e.slot as u8);
    }

    /// Inject a custom pass into a named slot (same as add_pass but named for clarity).
    ///
    /// C++/C# developers: this is the extension point for custom rendering.
    /// JS scripts call: Engine.renderGraph.inject("PostFx", myCustomPass);
    pub fn inject_pass(
        &mut self,
        slot: PassSlot,
        name: &str,
        pass: Box<dyn RenderPass>,
    ) {
        self.add_pass(name, slot, pass);
    }

    /// Enable or disable a pass by name. Disabled passes are skipped but not removed.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        if let Some(e) = self.passes.iter_mut().find(|e| e.name == name) {
            e.enabled = enabled;
        } else {
            log::warn!("RenderGraph::set_enabled: pass '{}' not found", name);
        }
    }

    /// Remove a pass by name.
    pub fn remove_pass(&mut self, name: &str) {
        self.passes.retain(|e| e.name != name);
    }

    /// Returns all pass names in execution order.
    pub fn pass_names(&self) -> Vec<&str> {
        self.passes.iter().map(|e| e.name.as_str()).collect()
    }

    /// Prepare all passes (called once after device creation, before first frame).
    pub fn prepare(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        for entry in &mut self.passes {
            entry.pass.prepare(device, resources);
        }
    }

    /// Notify all passes of a viewport resize.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        for entry in &mut self.passes {
            entry.pass.resize(device, width, height);
        }
    }

    /// Execute all enabled passes in slot order.
    pub fn execute(&mut self, ctx: &mut RenderContext) {
        for entry in &mut self.passes {
            if !entry.enabled { continue; }
            entry.pass.execute(ctx);
        }
    }
}

impl Default for RenderGraph { fn default() -> Self { Self::new() } }
