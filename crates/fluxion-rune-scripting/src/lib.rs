// ============================================================
// fluxion-rune-scripting
//
// Full Rune scripting VM for the FluxionRS engine with first-class
// hot-reload support.  The editor itself is written in Rune, so
// every .rn file save is reflected live without restarting.
//
// # Public API
//   RuneVm        — compile, run, hot-reload a set of Rune scripts.
//   RuneBehaviour — single-script component (start/update/fixed/destroy).
//   TIME_SNAPSHOT — update time values visible to Rune `fluxion::time`.
//   INPUT_SNAPSHOT — update input state visible to Rune `fluxion::input`.
//
// # Example script (my_script.rn)
//   pub fn start() { fluxion::debug::log("Hello from Rune!") }
//   pub fn update(dt) { }
//   pub fn on_hot_reload() { fluxion::debug::log("reloaded!") }
// ============================================================

// All modules require the native `rune` dependency.
#[cfg(not(target_arch = "wasm32"))]
pub mod auto_binding;
#[cfg(not(target_arch = "wasm32"))]
pub mod behaviour;
pub mod hot_reload;
#[cfg(not(target_arch = "wasm32"))]
pub mod vm;

#[cfg(not(target_arch = "wasm32"))]
pub use behaviour::RuneBehaviour;
#[cfg(not(target_arch = "wasm32"))]
pub use vm::{RuneVm, TIME_SNAPSHOT, input_snapshot};
