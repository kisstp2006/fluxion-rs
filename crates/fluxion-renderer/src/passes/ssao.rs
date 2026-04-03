// ============================================================
// fluxion-renderer — SsaoPass (stub)
//
// SSAO requires a precomputed hemisphere sample kernel and a noise
// texture. Full implementation left as an extension point.
// The pass is registered but currently outputs white (no occlusion),
// which is correct "disabled" behavior (multiplies lighting by 1).
// ============================================================

use crate::render_graph::{RenderPass, RenderContext};

pub struct SsaoPass {
    pub enabled: bool,
}

impl SsaoPass {
    pub fn new() -> Self { Self { enabled: false } }
}

impl RenderPass for SsaoPass {
    fn name(&self) -> &str { "ssao" }
    fn execute(&mut self, _ctx: &mut RenderContext) {
        // Stub: SSAO disabled by default. Enable by setting enabled = true
        // and implementing the full pass using ssao.wgsl.
    }
}

impl Default for SsaoPass { fn default() -> Self { Self::new() } }
