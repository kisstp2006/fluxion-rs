// ============================================================
// fluxion-scripting — JsRenderPass
//
// Adapts a JavaScript object as a Rust RenderPass, allowing scripts
// to inject custom render passes into the render graph.
//
// JS usage:
//   Engine.renderGraph.inject("PostFx", {
//     name: "my_outline",
//
//     prepare(ctx) {
//       // ctx.loadShader("shaders/outline.wgsl")
//       // Called once when the pass is added to the graph
//     },
//
//     execute(ctx) {
//       // ctx.blitFullscreen(this.shader);
//       // Called every frame
//     }
//   });
//
// The Rust side wraps this object and calls its prepare/execute methods
// via rquickjs. The ctx parameter exposes a limited, safe subset of the
// renderer API — scripts cannot access raw wgpu objects.
// ============================================================

use fluxion_renderer::render_graph::{RenderPass, RenderContext};

/// A render pass backed by a JavaScript object.
///
/// The JS object must have:
///   - `name: string`        — unique pass identifier
///   - `execute(ctx): void`  — called every frame
/// Optional:
///   - `prepare(ctx): void`  — called once during graph setup
///   - `resize(w, h): void`  — called on viewport resize
pub struct JsRenderPass {
    /// The JS-side name of this pass (from the `name` property).
    pass_name: String,
    /// Cached QuickJS function references. We store them as serialized
    /// source to be re-evaluated — this avoids lifetime issues with
    /// rquickjs values across frames.
    ///
    /// In a production implementation these would be rquickjs::Persistent<Function>.
    /// For Phase 1 we call by name via the VM's global scope.
    #[allow(dead_code)]
    execute_fn_name: String,
}

impl JsRenderPass {
    /// Create a JsRenderPass from the name of a globally-registered JS pass object.
    ///
    /// The JS side calls `__fluxion_register_pass(passObject)` which stores the
    /// object in a global registry keyed by name.
    pub fn new(pass_name: &str) -> Self {
        let execute_fn_name = format!("__fluxion_pass_execute_{}", pass_name.replace('-', "_"));
        Self {
            pass_name:       pass_name.to_string(),
            execute_fn_name,
        }
    }
}

impl RenderPass for JsRenderPass {
    fn name(&self) -> &str { &self.pass_name }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn execute(&mut self, _ctx: &mut RenderContext) {
        // TODO: call the JS execute function via the VM.
        // The VM reference would be held via Arc<Mutex<JsVm>> in the full implementation.
        // For Phase 1, this is a stub that demonstrates the injection pattern.
        log::debug!("JsRenderPass::execute() — pass '{}'", self.pass_name);
    }
}

/// The JavaScript code that enables render graph injection from scripts.
///
/// Inject this into the VM alongside the FluxionBehaviour stdlib.
pub const RENDER_GRAPH_JS: &str = r#"
// ── Render graph injection API ────────────────────────────────────────────────
// Scripts call Engine.renderGraph.inject(slot, passObject) to add a custom pass.
// Available slots: "PostOpaque", "PostFx", "Overlay"

const __registeredPasses = {};

const RenderGraph = {
    /**
     * Inject a custom render pass into the pipeline.
     *
     * @param {string} slot - Where to inject: "PostOpaque", "PostFx", or "Overlay"
     * @param {object} pass - Object with { name, prepare(ctx), execute(ctx) }
     *
     * Example:
     *   RenderGraph.inject("Overlay", {
     *     name: "outline",
     *     execute(ctx) { ctx.blitFullscreen("shaders/outline.wgsl"); }
     *   });
     */
    inject(slot, pass) {
        if (!pass.name) { console.error("RenderGraph.inject: pass must have a .name property"); return; }
        if (!pass.execute) { console.error("RenderGraph.inject: pass must have an .execute() method"); return; }
        __registeredPasses[pass.name] = { slot, pass };
        // Notify Rust to create a JsRenderPass for this JS object.
        // In the full implementation, __fluxion_inject_pass is a Rust-bound function.
        if (typeof __fluxion_inject_pass === "function") {
            __fluxion_inject_pass(slot, pass.name);
        } else {
            console.warn("RenderGraph.inject: __fluxion_inject_pass not bound (render graph injection disabled)");
        }
    },

    /** Enable or disable a pass by name. */
    setEnabled(name, enabled) {
        if (typeof __fluxion_set_pass_enabled === "function") {
            __fluxion_set_pass_enabled(name, enabled);
        }
    }
};

// Expose on Engine object (created by bindings.rs)
if (typeof Engine !== "undefined") {
    Engine.renderGraph = RenderGraph;
}
"#;
