// ============================================================
// vm.rs — RuneVm: compile, run, hot-reload Rune scripts
// ============================================================

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, RwLock},
    collections::HashSet,
};

use anyhow::Context as _;
use rune::{
    termcolor::{ColorChoice, StandardStream},
    Diagnostics, Source, Sources, Unit,
};

use crate::hot_reload::HotReloadWatcher;

// ── Global snapshots (read by auto_binding modules) ───────────────────────────

/// Atomic snapshot of time values, updated each frame by the host.
pub static TIME_SNAPSHOT: TimeSnapshot = TimeSnapshot::new();
/// Atomic snapshot of input state. Accessed via `input_snapshot()`.
static INPUT_SNAPSHOT_CELL: OnceLock<InputSnapshot> = OnceLock::new();
/// Atomic snapshot of the editor viewport pixel size.
pub static VIEWPORT_SNAPSHOT: ViewportSnapshot = ViewportSnapshot::new();

/// Get the global input snapshot (lazily initialized).
pub fn input_snapshot() -> &'static InputSnapshot {
    INPUT_SNAPSHOT_CELL.get_or_init(InputSnapshot::default)
}

pub struct ViewportSnapshot {
    width:  std::sync::atomic::AtomicU32,
    height: std::sync::atomic::AtomicU32,
}

impl ViewportSnapshot {
    pub const fn new() -> Self {
        Self {
            width:  std::sync::atomic::AtomicU32::new(1280),
            height: std::sync::atomic::AtomicU32::new(720),
        }
    }
    pub fn update(&self, w: u32, h: u32) {
        use std::sync::atomic::Ordering::Relaxed;
        self.width.store(w, Relaxed);
        self.height.store(h, Relaxed);
    }
    pub fn load_width(&self)  -> u32 { self.width.load(std::sync::atomic::Ordering::Relaxed) }
    pub fn load_height(&self) -> u32 { self.height.load(std::sync::atomic::Ordering::Relaxed) }
}

pub struct TimeSnapshot {
    dt:      std::sync::atomic::AtomicU32,
    elapsed: std::sync::atomic::AtomicU32,
    frame:   std::sync::atomic::AtomicU64,
}

impl TimeSnapshot {
    const fn new() -> Self {
        Self {
            dt:      std::sync::atomic::AtomicU32::new(0),
            elapsed: std::sync::atomic::AtomicU32::new(0),
            frame:   std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn update(&self, dt: f32, elapsed: f32, frame: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        self.dt.store(dt.to_bits(), Relaxed);
        self.elapsed.store(elapsed.to_bits(), Relaxed);
        self.frame.store(frame, Relaxed);
    }

    pub fn load_dt(&self) -> f64 {
        f32::from_bits(self.dt.load(std::sync::atomic::Ordering::Relaxed)) as f64
    }

    pub fn load_elapsed(&self) -> f64 {
        f32::from_bits(self.elapsed.load(std::sync::atomic::Ordering::Relaxed)) as f64
    }

    pub fn load_frame(&self) -> u64 {
        self.frame.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Thread-safe input state snapshot.
pub struct InputSnapshot {
    held: Mutex<HashSet<String>>,
    down: Mutex<HashSet<String>>,
    up:   Mutex<HashSet<String>>,
}

impl Default for InputSnapshot {
    fn default() -> Self {
        Self {
            held: Mutex::new(HashSet::new()),
            down: Mutex::new(HashSet::new()),
            up:   Mutex::new(HashSet::new()),
        }
    }
}

impl InputSnapshot {

    pub fn update(&self, held: Vec<String>, down: Vec<String>, up: Vec<String>) {
        *self.held.lock().unwrap() = held.into_iter().collect();
        *self.down.lock().unwrap() = down.into_iter().collect();
        *self.up.lock().unwrap()   = up.into_iter().collect();
    }

    pub fn is_key_held(&self, key: &str) -> bool {
        self.held.lock().unwrap().contains(key)
    }

    pub fn is_key_down(&self, key: &str) -> bool {
        self.down.lock().unwrap().contains(key)
    }

    pub fn is_key_up(&self, key: &str) -> bool {
        self.up.lock().unwrap().contains(key)
    }
}

// ── RuneVm ────────────────────────────────────────────────────────────────────

/// Compiled Rune unit, shared across all VMs for a script set.
type SharedUnit = Arc<RwLock<Arc<Unit>>>;

/// The Rune scripting VM.
///
/// # Hot reload
///
/// Call `enable_hot_reload(watch_dir)` to start watching for `.rn` file
/// changes. Each frame, call `poll_hot_reload()` to process any pending
/// recompiles. On compile error the old unit keeps running.
pub struct RuneVm {
    /// Rune runtime (context + type info). Rebuilt only when modules change.
    runtime: Arc<rune::runtime::RuntimeContext>,
    /// Compiled bytecode, swappable on hot reload.
    unit:    SharedUnit,
    /// Paths of loaded source files (for incremental recompile).
    source_paths: Vec<PathBuf>,
    /// Compile context — includes all installed modules (engine + editor extras).
    /// Stored so `poll_hot_reload` recompiles with the same module set.
    compile_ctx: Option<Arc<rune::Context>>,
    /// Watches .rn files for changes (native only; None = hot reload disabled).
    watcher: Option<HotReloadWatcher>,
    /// Hooks called after every successful hot reload.
    reload_hooks: Vec<Box<dyn Fn() + Send + Sync>>,
    /// Last compile errors (shown in-editor).
    pub last_error: Option<String>,
}

impl RuneVm {
    /// Create a new VM, compiling the given source files.
    pub fn new(source_paths: &[&Path]) -> anyhow::Result<Self> {
        let mut ctx = rune::Context::with_default_modules()
            .context("Failed to create default Rune context")?;

        // Install engine modules.
        for m in crate::auto_binding::all_modules()? {
            ctx.install(m).context("Failed to install engine module")?;
        }

        let runtime = Arc::new(ctx.runtime().context("Failed to build Rune runtime")?);

        let paths: Vec<PathBuf> = source_paths.iter().map(|p| p.to_path_buf()).collect();
        let unit = Self::compile_paths(&paths)?;

        Ok(Self {
            runtime,
            unit:         Arc::new(RwLock::new(Arc::new(unit))),
            source_paths: paths,
            compile_ctx:  None,
            watcher:      None,
            reload_hooks: Vec::new(),
            last_error:   None,
        })
    }

    /// Create a new VM with additional caller-supplied Rune modules installed
    /// on top of the default engine modules.  Used by `fluxion-editor` to
    /// register `fluxion::ui` and `fluxion::world` without touching this crate.
    ///
    /// `extra_modules_fn` is called TWICE — once for the runtime context and
    /// once for the compile context — so each context gets freshly constructed
    /// module instances.  This is required because `rune::Module` does not
    /// implement `Clone`, so re-using the same instance would install it twice
    /// into the same context and cause a duplicate-function-hash panic.
    pub fn new_with_extra_modules(
        source_paths:    &[&Path],
        extra_modules_fn: impl Fn() -> anyhow::Result<Vec<rune::Module>>,
    ) -> anyhow::Result<Self> {
        let mut ctx = rune::Context::with_default_modules()
            .context("Failed to create default Rune context")?;

        for m in crate::auto_binding::all_modules()? {
            ctx.install(m).context("Failed to install engine module")?;
        }
        for m in extra_modules_fn()? {
            ctx.install(m).context("Failed to install extra module")?;
        }

        let runtime = Arc::new(ctx.runtime().context("Failed to build Rune runtime")?);

        let paths: Vec<PathBuf> = source_paths.iter().map(|p| p.to_path_buf()).collect();
        // Build a compile context identical to the runtime context.
        let mut compile_ctx = rune::Context::with_default_modules()
            .context("Failed to create compile context")?;
        for m in crate::auto_binding::all_modules()? {
            compile_ctx.install(m).context("Failed to install engine module (compile)")?;
        }
        for m in extra_modules_fn()? {
            compile_ctx.install(m).context("Failed to install extra module (compile)")?;
        }
        let unit = Self::compile_paths_with_ctx(&paths, &compile_ctx)?;

        Ok(Self {
            runtime,
            unit:         Arc::new(RwLock::new(Arc::new(unit))),
            source_paths: paths,
            compile_ctx:  Some(Arc::new(compile_ctx)),
            watcher:      None,
            reload_hooks: Vec::new(),
            last_error:   None,
        })
    }

    /// Create an empty VM (no scripts loaded).
    pub fn empty() -> anyhow::Result<Self> {
        Self::new(&[])
    }

    /// Load an additional script file and recompile.
    pub fn load_file(&mut self, path: &Path) -> anyhow::Result<()> {
        if !self.source_paths.contains(&path.to_path_buf()) {
            self.source_paths.push(path.to_path_buf());
        }
        let unit = Self::compile_paths(&self.source_paths)?;
        *self.unit.write().unwrap() = Arc::new(unit);
        self.last_error = None;
        Ok(())
    }

    /// Start watching `watch_dir` for `.rn` changes. Non-blocking.
    pub fn enable_hot_reload(&mut self, watch_dir: &Path) -> anyhow::Result<()> {
        self.watcher = Some(HotReloadWatcher::start(watch_dir)?);
        Ok(())
    }

    /// Register a hook to call after every successful hot reload.
    pub fn on_reload(&mut self, hook: impl Fn() + Send + Sync + 'static) {
        self.reload_hooks.push(Box::new(hook));
    }

    /// Check for pending hot-reload events and process them.
    /// Call once per frame from the engine loop.
    pub fn poll_hot_reload(&mut self) {
        let changed: Vec<PathBuf> = match &self.watcher {
            Some(w) => w.drain(),
            None    => return,
        };
        if changed.is_empty() { return; }

        for path in &changed {
            log::info!("[RuneHotReload] Reloading {:?}", path.file_name().unwrap_or_default());

            // Update source_paths with the changed file (add if new, otherwise keep existing).
            if !self.source_paths.contains(path) {
                self.source_paths.push(path.clone());
            }

            let compile_result = match &self.compile_ctx {
                Some(ctx) => Self::compile_paths_with_ctx(&self.source_paths, ctx),
                None      => Self::compile_paths(&self.source_paths),
            };
            match compile_result {
                Ok(unit) => {
                    *self.unit.write().unwrap() = Arc::new(unit);
                }
                Err(e) => {
                    let msg = format!("{:#}", e);
                    log::error!("[RuneHotReload] Compile error:\n{msg}");
                    self.last_error = Some(msg);
                    return;
                }
            }
        }

        self.last_error = None;
        // Call the in-script on_hot_reload() hook.
        let _ = self.on_hot_reload_hook();
        // Call registered Rust-side reload hooks.
        for hook in &self.reload_hooks {
            hook();
        }
    }

    /// Compile a set of .rn file paths into a `rune::Unit`.
    fn compile_paths(paths: &[PathBuf]) -> anyhow::Result<Unit> {
        let ctx = rune::Context::with_default_modules().context("Default context")?;
        Self::compile_paths_with_ctx(paths, &ctx)
    }

    /// Compile using a caller-supplied context (e.g. one that has extra modules).
    pub fn compile_paths_with_ctx(
        paths: &[PathBuf],
        ctx:   &rune::Context,
    ) -> anyhow::Result<Unit> {
        let mut sources = Sources::new();
        for path in paths {
            let stem = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("script")
                .to_string();
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read {:?}", path))?;
            let source = Source::new(stem, content)
                .with_context(|| format!("Failed to create source for {:?}", path))?;
            let _ = sources.insert(source);
        }

        let mut diagnostics = Diagnostics::new();
        let result = rune::prepare(&mut sources)
            .with_context(ctx)
            .with_diagnostics(&mut diagnostics)
            .build();

        if !diagnostics.is_empty() {
            let mut out = StandardStream::stderr(ColorChoice::Always);
            diagnostics.emit(&mut out, &sources)?;
        }

        result.context("Rune compilation failed")
    }

    // ── Script execution ──────────────────────────────────────────────────────

    /// Call a Rune function by name. Returns `Ok(None)` if the function
    /// does not exist in the current unit (not an error).
    pub fn call_fn(
        &self,
        fn_path: &[&str],
        args:    impl rune::runtime::Args,
    ) -> anyhow::Result<Option<rune::Value>> {
        let unit   = self.unit.read().unwrap().clone();
        let mut vm = rune::Vm::new(self.runtime.clone(), unit);

        let result = vm.execute(fn_path, args);
        match result {
            Ok(mut exec) => {
                match exec.complete().into_result() {
                    Ok(val) => Ok(Some(val)),
                    Err(e)  => {
                        // Log the full Rune error with source location if available.
                        log::error!("Rune VM error: {:?}", e);
                        Err(anyhow::anyhow!("Rune function panicked: {}", e))
                    }
                }
            }
            Err(e) => {
                let msg = format!("{e:?}");
                if msg.contains("MissingFunction") || msg.contains("missing function")
                    || msg.contains("MissingEntry")
                {
                    return Ok(None);
                }
                Err(anyhow::anyhow!("Rune call {:?}: {e}", fn_path))
            }
        }
    }

    // ── Lifecycle interface ───────────────────────────────────────────────────

    /// Run `start()` if defined.
    pub fn start(&self) -> anyhow::Result<()> {
        self.call_fn(&["start"], ())?;
        Ok(())
    }

    /// Run `update(dt)` if defined.
    pub fn update(&self, dt: f64) -> anyhow::Result<()> {
        self.call_fn(&["update"], (dt,))?;
        Ok(())
    }

    /// Run `fixed_update(dt)` if defined.
    pub fn fixed_update(&self, dt: f64) -> anyhow::Result<()> {
        self.call_fn(&["fixed_update"], (dt,))?;
        Ok(())
    }

    /// Run `on_destroy()` if defined.
    pub fn on_destroy(&self) -> anyhow::Result<()> {
        self.call_fn(&["on_destroy"], ())?;
        Ok(())
    }

    // ── Editor lifecycle ──────────────────────────────────────────────────────

    /// Run `on_editor_init()` — called once when the editor starts.
    pub fn on_editor_init(&self) -> anyhow::Result<()> {
        self.call_fn(&["on_editor_init"], ())?;
        Ok(())
    }

    /// Run `on_hot_reload()` — called after the script is reloaded.
    pub fn on_hot_reload_hook(&self) -> anyhow::Result<()> {
        self.call_fn(&["on_hot_reload"], ())?;
        Ok(())
    }

    /// Run `on_collision_enter(entity_a_id, entity_b_id)` — Unity-style collision callback.
    /// Called for every collision start event this frame.
    pub fn on_collision_enter(&self, entity_a: i64, entity_b: i64) -> anyhow::Result<()> {
        self.call_fn(&["on_collision_enter"], (entity_a, entity_b))?;
        Ok(())
    }

    /// Run `on_collision_exit(entity_a_id, entity_b_id)` — Unity-style collision callback.
    /// Called for every collision stop event this frame.
    pub fn on_collision_exit(&self, entity_a: i64, entity_b: i64) -> anyhow::Result<()> {
        self.call_fn(&["on_collision_exit"], (entity_a, entity_b))?;
        Ok(())
    }

    // ── Host data push ────────────────────────────────────────────────────────

    /// Push current frame timing into the global snapshot read by `fluxion::time`.
    pub fn push_time(&self, dt: f32, elapsed: f32, frame: u64) {
        TIME_SNAPSHOT.update(dt, elapsed, frame);
    }

    /// Push current viewport pixel size into the global snapshot read by `fluxion::viewport`.
    pub fn push_viewport(&self, width: u32, height: u32) {
        VIEWPORT_SNAPSHOT.update(width, height);
    }

    /// Push currently held keys into the global snapshot read by `fluxion::input`.
    /// `held` = keys currently pressed; `down` / `up` = just-pressed / just-released this frame.
    pub fn push_input(
        &self,
        held: impl IntoIterator<Item = impl Into<String>>,
        down: impl IntoIterator<Item = impl Into<String>>,
        up:   impl IntoIterator<Item = impl Into<String>>,
    ) {
        input_snapshot().update(
            held.into_iter().map(Into::into).collect(),
            down.into_iter().map(Into::into).collect(),
            up  .into_iter().map(Into::into).collect(),
        );
    }

    /// Whether the VM has any compile errors (from the last hot reload attempt).
    pub fn has_error(&self) -> bool {
        self.last_error.is_some()
    }
}
