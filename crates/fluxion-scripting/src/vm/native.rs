// ============================================================
// fluxion-scripting — JsVm (QuickJS, native targets)
// ============================================================

use std::path::Path;

use anyhow::Context as AnyhowContext;
use rquickjs::{Context, Runtime};

/// The QuickJS VM. One instance per engine.
pub struct JsVm {
    rt: Runtime,
    pub ctx: Context,
}

impl JsVm {
    /// Create a new VM and inject all engine globals.
    pub fn new() -> anyhow::Result<Self> {
        let rt = Runtime::new().context("Failed to create QuickJS runtime")?;
        let ctx = Context::full(&rt).context("Failed to create QuickJS context")?;

        rt.set_memory_limit(64 * 1024 * 1024);
        rt.set_max_stack_size(1024 * 1024);

        Ok(Self { rt, ctx })
    }

    /// Evaluate raw JavaScript source.
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
    pub fn load_typescript(&self, ts_path: &str) -> anyhow::Result<()> {
        let out_dir = std::env::temp_dir();
        let out_path = out_dir
            .join(
                Path::new(ts_path)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref(),
            )
            .with_extension("js");

        let status = std::process::Command::new("tsc")
            .args([
                "--target",
                "ES2020",
                "--module",
                "commonjs",
                "--outDir",
                out_dir.to_str().unwrap_or("/tmp"),
                ts_path,
            ])
            .status()
            .with_context(|| "Failed to run tsc — is TypeScript installed?")?;

        if !status.success() {
            anyhow::bail!("tsc failed for '{ts_path}' — check TypeScript errors");
        }

        self.load_script(out_path.to_str().unwrap_or(ts_path))
    }

    /// Evaluate an expression and decode the result as a UTF-8 string.
    pub fn eval_string_result(&self, source: &str, name: &str) -> anyhow::Result<String> {
        self.ctx.with(|ctx| {
            let v: rquickjs::Value = ctx
                .eval(source.as_bytes())
                .map_err(|e| anyhow::anyhow!("JS eval error in '{}': {}", name, e))?;
            let s: rquickjs::String = v
                .get()
                .map_err(|e| anyhow::anyhow!("JS result not a string in '{}': {}", name, e))?;
            let std = s
                .to_string()
                .map_err(|e| anyhow::anyhow!("JS string convert in '{}': {}", name, e))?;
            Ok(std)
        })
    }

    pub fn update(&self, dt: f32) -> anyhow::Result<()> {
        self.ctx.with(|ctx| {
            let globals = ctx.globals();
            if let Ok(tick_fn) = globals.get::<_, rquickjs::Function>("__fluxion_tick") {
                tick_fn
                    .call::<_, ()>((dt,))
                    .map_err(|e| anyhow::anyhow!("Script update error: {e}"))?;
            }
            Ok(())
        })
    }

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

    pub fn get_global<T: for<'js> rquickjs::FromJs<'js>>(&self, name: &str) -> anyhow::Result<T> {
        self.ctx.with(|ctx| {
            ctx.globals()
                .get::<_, T>(name)
                .map_err(|e| anyhow::anyhow!("Failed to get global '{name}': {e}"))
        })
    }
}
