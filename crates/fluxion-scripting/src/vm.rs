// ============================================================
// fluxion-scripting — JsVm
//
// Wraps the QuickJS Runtime + Context. One JsVm per engine instance.
//
// Script loading:
//   1. Pre-transpile .ts → .js using tsc (offline, or via a bundler)
//      OR load .js directly.
//   2. Call vm.load_script(path) or vm.eval(source, name).
//   3. Scripts access the engine via injected globals (see bindings.rs).
//
// Thread model: QuickJS is single-threaded. All JS calls happen on the
// main game loop thread. This matches the TypeScript engine's model.
//
// Hot-reload:
//   Call vm.reload_script(path) to re-evaluate a script file.
//   Script state (class instances) is reset on reload. This is simpler
//   than the TS engine's "preserve state between reloads" approach and
//   avoids stale reference bugs.
// ============================================================

use std::path::Path;
use rquickjs::{Context, Runtime};
use anyhow::Context as AnyhowContext;

/// The QuickJS VM. One instance per engine.
pub struct JsVm {
    rt:  Runtime,
    pub ctx: Context,
}

impl JsVm {
    /// Create a new VM and inject all engine globals.
    pub fn new() -> anyhow::Result<Self> {
        let rt  = Runtime::new().context("Failed to create QuickJS runtime")?;
        let ctx = Context::full(&rt).context("Failed to create QuickJS context")?;

        // Set memory limit (64 MB default — adjust as needed)
        rt.set_memory_limit(64 * 1024 * 1024);
        // Set max stack depth to prevent infinite recursion from crashing the engine
        rt.set_max_stack_size(1024 * 1024); // 1 MB stack

        let vm = Self { rt, ctx };
        Ok(vm)
    }

    /// Evaluate raw JavaScript source. Returns the result as a JSON string,
    /// or an error message if evaluation failed.
    ///
    /// `name` is used in stack traces — pass the filename.
    pub fn eval(&self, source: &str, name: &str) -> anyhow::Result<()> {
        self.ctx.with(|ctx| {
            ctx.eval::<rquickjs::Value, _>(source.as_bytes())
                .map_err(|e| anyhow::anyhow!("JS eval error in '{}': {}", name, e))?;
            Ok(())
        })
    }

    /// Load and evaluate a JavaScript file.
    pub fn load_script(&self, path: &str) -> anyhow::Result<()> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read script: {path}"))?;
        self.eval(&source, path)
    }

    /// Load a TypeScript file by first transpiling it with `tsc`.
    ///
    /// Requires `tsc` to be installed on the PATH.
    /// For production builds, pre-compile scripts with the project's build system.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_typescript(&self, ts_path: &str) -> anyhow::Result<()> {
        // Transpile: tsc --target ES2020 --module ES2020 --outDir /tmp <file>
        let out_dir  = std::env::temp_dir();
        let out_path = out_dir.join(
            Path::new(ts_path)
                .file_stem().unwrap_or_default()
                .to_string_lossy()
                .as_ref()
        ).with_extension("js");

        let status = std::process::Command::new("tsc")
            .args(["--target", "ES2020", "--module", "commonjs",
                   "--outDir", out_dir.to_str().unwrap_or("/tmp"),
                   ts_path])
            .status()
            .with_context(|| "Failed to run tsc — is TypeScript installed?")?;

        if !status.success() {
            anyhow::bail!("tsc failed for '{ts_path}' — check TypeScript errors");
        }

        self.load_script(out_path.to_str().unwrap_or(ts_path))
    }

    /// Execute a one-frame tick: call all registered script behaviours.
    /// `dt` is the frame delta time in seconds.
    pub fn update(&self, dt: f32) -> anyhow::Result<()> {
        // Call the global __fluxion_tick(dt) function if it exists.
        // Scripts register themselves into this via the FluxionBehaviour base class.
        self.ctx.with(|ctx| {
            let globals = ctx.globals();
            if let Ok(tick_fn) = globals.get::<_, rquickjs::Function>("__fluxion_tick") {
                tick_fn.call::<_, ()>((dt,))
                    .map_err(|e| anyhow::anyhow!("Script update error: {e}"))?;
            }
            Ok(())
        })
    }

    /// Execute fixed-rate script callbacks.
    pub fn fixed_update(&self, fixed_dt: f32) -> anyhow::Result<()> {
        self.ctx.with(|ctx| {
            let globals = ctx.globals();
            if let Ok(f) = globals.get::<_, rquickjs::Function>("__fluxion_fixed_tick") {
                f.call::<_, ()>((fixed_dt,))
                    .map_err(|e| anyhow::anyhow!("Script fixed_update error: {e}"))?;
            }
            Ok(())
        })
    }

    /// Inject a Rust value into the JS global scope.
    ///
    /// The value must implement `rquickjs::IntoJs`.
    pub fn set_global<T: for<'js> rquickjs::IntoJs<'js>>(
        &self,
        name: &str,
        value: T,
    ) -> anyhow::Result<()> {
        self.ctx.with(|ctx| {
            ctx.globals()
                .set(name, value)
                .map_err(|e| anyhow::anyhow!("Failed to set global '{name}': {e}"))
        })
    }

    /// Evaluate a snippet and retrieve the result as a specific Rust type.
    pub fn get_global<T: for<'js> rquickjs::FromJs<'js>>(&self, name: &str) -> anyhow::Result<T> {
        self.ctx.with(|ctx| {
            ctx.globals()
                .get::<_, T>(name)
                .map_err(|e| anyhow::anyhow!("Failed to get global '{name}': {e}"))
        })
    }
}
