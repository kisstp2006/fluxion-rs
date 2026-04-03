// ============================================================
// fluxion-scripting
//
// JavaScript/TypeScript scripting for FluxionRS using the QuickJS
// embedded engine (via the `rquickjs` crate).
//
// QuickJS is a small, fast, embeddable JS engine. It supports:
//   - Full ES2020 (ES modules, async/await, classes, etc.)
//   - TypeScript via a pre-transpilation step (tsc → JS before loading)
//   - Compiles to both native and WASM
//
// Architecture mirrors the TypeScript engine's ScriptSystem.ts:
//   - JsVm:       owns the QuickJS Runtime + Context
//   - JsBehaviour:lifecycle bindings (start/update/lateUpdate/onDestroy)
//   - JsRenderPass: wraps a JS object as a Rust RenderPass for render graph injection
//   - Bindings:   Rust functions/objects exposed to JS (world, Time, Input, Engine)
//
// Script lifecycle (matches FluxionBehaviour in the TS engine):
//   class MyScript extends FluxionBehaviour {
//     start()         {}  // called once on first frame
//     update(dt)      {}  // called every variable-rate frame
//     lateUpdate(dt)  {}  // after all updates
//     onDestroy()     {}  // when entity is despawned
//   }
//
// Render graph injection from JS:
//   Engine.renderGraph.inject("PostFx", {
//     name: "my_outline",
//     prepare(renderer) { ... },
//     execute(ctx)      { ... },
//   });
// ============================================================

pub mod vm;
pub mod behaviour;
pub mod bindings;
pub mod render_pass;

pub use vm::JsVm;
pub use render_pass::JsRenderPass;
