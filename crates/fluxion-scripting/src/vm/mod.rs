//! QuickJS-backed [`JsVm`] on native targets; no-op stub on `wasm32` (no `rquickjs-sys`).

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::JsVm;

#[cfg(target_arch = "wasm32")]
mod stub_wasm;
#[cfg(target_arch = "wasm32")]
pub use stub_wasm::JsVm;
