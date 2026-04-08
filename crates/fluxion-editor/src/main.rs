// ============================================================
// fluxion-editor — standalone editor binary
//
// Hosts the full engine runtime (ECS + renderer + physics) and
// presents a hot-reloadable Rune-scripted UI with egui_dock
// docking support.
// ============================================================

mod dock;
mod host;
mod icons;
mod project_chooser;
mod rune_bindings;
mod theme;
mod toolbar;
mod ui_shell;
mod undo;
mod viewport_gizmo;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

static WANT_QUIT: AtomicBool = AtomicBool::new(false);

use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
    window::{Window, WindowId},
};

use fluxion_renderer::{FluxionRenderer, RendererConfig};
use fluxion_core::{ProjectConfig, EditorPrefs};
#[cfg(not(target_arch = "wasm32"))]
use fluxion_core::{load_editor_prefs, save_editor_prefs, save_project};
use fluxion_core::scene::{load_scene_into_world, world_to_scene_data, SceneSettings, save_scene_file, load_scene_file};

use crate::dock::{default_dock_state, show_dock, EditorTab};
use crate::host::EditorHost;
use crate::project_chooser::ProjectChooser;
use crate::rune_bindings::set_viewport_texture;
use crate::toolbar::{EditorMode, TransformTool};
use crate::ui_shell::UiShell;
use notify::{Watcher, RecursiveMode};

// ── Dual logger: stderr + editor Console panel ─────────────────────────────────────

/// Forwards every log record to the env_logger backend (stderr) AND to the
/// editor's in-memory console via `push_log`, so log::info!/warn!/error!
/// calls appear in the Console panel.
struct DualLogger {
    inner: env_logger::Logger,
}

impl log::Log for DualLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if !self.inner.enabled(record.metadata()) { return; }
        self.inner.log(record);
        let prefix = match record.level() {
            log::Level::Error => "[ERROR] ",
            log::Level::Warn  => "[WARN] ",
            _                 => "[INFO] ",
        };
        crate::rune_bindings::world_module::push_log(
            format!("{}{}", prefix, record.args())
        );
    }

    fn flush(&self) { self.inner.flush(); }
}

// ── Entry point ───────────────────────────────────────────────────────────────────────

fn main() {
    let inner = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("wgpu_core", log::LevelFilter::Warn)
        .filter_module("wgpu_hal",  log::LevelFilter::Warn)
        .filter_module("wgpu",      log::LevelFilter::Warn)
        .build();
    let max_level = inner.filter();
    log::set_boxed_logger(Box::new(DualLogger { inner }))
        .expect("Logger already set");
    log::set_max_level(max_level);

    log::info!("FluxionRS Editor — starting");

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = EditorApp::Uninitialized;
    event_loop.run_app(&mut app).expect("Event loop error");
}

// ── Application state ─────────────────────────────────────────────────────────

enum EditorApp {
    Uninitialized,
    /// Window is open but we are still showing the project chooser.
    Choosing {
        window:   Arc<Window>,
        renderer: FluxionRenderer,
        ui_shell: UiShell,
        chooser:  ProjectChooser,
    },
    /// Project loaded — main editor running.
    Running(Rc<RefCell<EditorInner>>),
}

/// All per-window state once a project is open.
struct EditorInner {
    window:     Arc<Window>,
    host:       EditorHost,
    renderer:   FluxionRenderer,
    ui_shell:   UiShell,
    dock_state: egui_dock::DockState<EditorTab>,

    // Editor metadata
    project:      ProjectConfig,
    project_root: PathBuf,
    scene_path:   Option<PathBuf>,
    scene_dirty:  bool,

    // Runtime state
    editor_mode:    EditorMode,
    transform_tool: TransformTool,
    modifiers:      ModifiersState,

    // Per-frame gizmo drag state (persisted between frames)
    gizmo_drag: viewport_gizmo::GizmoDragState,

    // Snap accumulators — track true unsnapped position/scale during a drag
    // so that sub-grid motion is not discarded each frame.
    snap_raw_pos:      glam::Vec3,
    snap_raw_scale:    glam::Vec3,
    snap_was_dragging: bool,

    // Applied once on the first frame; egui theme is static for the session.
    theme_applied:  bool,

    // Asset file watcher — triggers rescan when files change under assets/
    _file_watcher: Option<notify::RecommendedWatcher>,
    file_watcher_rx: Option<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>,
    /// Debounce: seconds remaining before the next rescan is allowed.
    file_watcher_cooldown: f32,

    // Editor preferences (user-level, persists across projects)
    editor_prefs: EditorPrefs,
    /// Autosave countdown timer in seconds.
    autosave_timer: f32,
}

// ── ApplicationHandler impl ───────────────────────────────────────────────────

impl ApplicationHandler for EditorApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !matches!(self, EditorApp::Uninitialized) {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("FluxionRS Editor")
            .with_inner_size(winit::dpi::LogicalSize::new(1600u32, 900u32));

        let window = Arc::new(
            event_loop.create_window(attrs).expect("Window creation failed"),
        );

        let (renderer, ui_shell) = pollster::block_on(async {
            let r = FluxionRenderer::new(window.clone(), RendererConfig::default())
                .await.expect("Renderer init failed");
            let fmt   = r.surface_format();
            let shell = UiShell::new(&window, &r.device, fmt);
            (r, shell)
        });

        *self = EditorApp::Choosing {
            window,
            renderer,
            ui_shell,
            chooser: ProjectChooser::new(),
        };
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event:      WindowEvent,
    ) {
        match self {
            // ── Project chooser ──────────────────────────────────────────────
            EditorApp::Choosing { window, renderer, ui_shell, chooser } => {
                let win = window.clone();
                let egui_resp = ui_shell.on_window_event(&win, &event);

                match event {
                    WindowEvent::CloseRequested => event_loop.exit(),
                    WindowEvent::Resized(size) => renderer.resize(size.width, size.height),
                    WindowEvent::RedrawRequested => {
                        // Draw the project chooser on the swap chain surface directly.
                        let w = renderer.width;
                        let h = renderer.height;
                        let result = renderer.render_ui_only(|device, queue, encoder, view| {
                            ui_shell.paint(&win, device, queue, encoder, view, w, h, |ctx| {
                                theme::apply_theme(ctx);
                                chooser.show(ctx);
                            })
                        });
                        if !result {
                            let s = win.inner_size();
                            renderer.resize(s.width, s.height);
                        }
                    }
                    _ => {}
                }

                let _ = egui_resp;

                // Check if project was chosen — transition to Running.
                if let Some(choice) = chooser.take_choice() {
                    self.transition_to_running(choice, event_loop);
                }
            }

            // ── Main editor ──────────────────────────────────────────────────
            EditorApp::Running(inner) => {
                let mut g = inner.borrow_mut();
                let window = g.window.clone();
                let egui_resp = g.ui_shell.on_window_event(&window, &event);
                if egui_resp.consumed { return; }

                match event {
                    WindowEvent::CloseRequested => event_loop.exit(),
                    WindowEvent::ModifiersChanged(mods) => {
                        g.modifiers = mods.state();
                    }
                    WindowEvent::KeyboardInput { event: kev, .. } => {
                        let pressed = kev.state == ElementState::Pressed;
                        if pressed {
                            let ctrl = g.modifiers.control_key();
                            match kev.physical_key {
                                PhysicalKey::Code(KeyCode::KeyS) if ctrl => g.save_scene(),
                                PhysicalKey::Code(KeyCode::KeyN) if ctrl => g.new_scene(),
                                PhysicalKey::Code(KeyCode::KeyZ) if ctrl => {
                                    let world    = &g.host.world    as *const _;
                                    let registry = &g.host.registry as *const _;
                                    // SAFETY: undo only reads world/registry; no aliased mutable refs exist.
                                    unsafe { g.host.undo.undo(&*world, &*registry); }
                                }
                                PhysicalKey::Code(KeyCode::KeyY) if ctrl => {
                                    let world    = &g.host.world    as *const _;
                                    let registry = &g.host.registry as *const _;
                                    unsafe { g.host.undo.redo(&*world, &*registry); }
                                }
                                PhysicalKey::Code(KeyCode::KeyD) if ctrl => {
                                    if g.host.duplicate_selected() {
                                        g.scene_dirty = true;
                                    }
                                }
                                PhysicalKey::Code(KeyCode::Comma) if ctrl => {
                                    crate::rune_bindings::settings_module::set_show_editor_prefs_flag(true);
                                }
                                PhysicalKey::Code(KeyCode::Delete) => g.delete_selected(),
                                _ => {}
                            }
                        }
                        if let PhysicalKey::Code(code) = kev.physical_key {
                            g.host.input.set_key_down(&format!("{code:?}"), kev.state == ElementState::Pressed);
                        }
                    }
                    WindowEvent::Resized(size) => g.renderer.resize(size.width, size.height),
                    WindowEvent::RedrawRequested => {
                        g.frame();
                        if WANT_QUIT.load(Ordering::Relaxed) {
                            event_loop.exit();
                        }
                    }
                    _ => {}
                }
            }

            EditorApp::Uninitialized => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id:  DeviceId,
        event:       DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            crate::rune_bindings::accumulate_raw_mouse_delta(dx, dy);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        match self {
            EditorApp::Choosing { window, .. } => window.request_redraw(),
            EditorApp::Running(inner) => inner.borrow().window.request_redraw(),
            EditorApp::Uninitialized => {}
        }
    }
}

impl EditorApp {
    fn transition_to_running(
        &mut self,
        choice: crate::project_chooser::ProjectChoice,
        _event_loop: &ActiveEventLoop,
    ) {
        // Destructure Choosing state, take ownership
        let (window, mut renderer, ui_shell) = match std::mem::replace(self, EditorApp::Uninitialized) {
            EditorApp::Choosing { window, renderer, ui_shell, .. } => (window, renderer, ui_shell),
            _ => return,
        };

        let scripts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
        let mut host = EditorHost::new(scripts_dir).expect("EditorHost init failed");
        // Rebuild camera manager for the default scene spawned inside EditorHost::new().
        host.camera_manager.rebuild(&host.world);

        // Enable gizmos in editor mode
        renderer.gizmos_enabled = true;

        // Resolve and load the default scene if it exists
        let scene_path = if !choice.config.default_scene.is_empty() {
            let sp = choice.root.join(&choice.config.default_scene);
            if sp.exists() {
                if let Ok(data) = load_scene_file(sp.to_str().unwrap_or("")) {
                    host.world.clear();
                    let _ = load_scene_into_world(&mut host.world, &data, true, &host.registry);
                    host.camera_manager.rebuild(&host.world);
                    log::info!("Loaded scene: {}", sp.display());
                }
                Some(sp)
            } else {
                None
            }
        } else {
            None
        };

        // Call on_editor_init.
        if let Err(e) = host.vm.on_editor_init() {
            log::warn!("on_editor_init: {e}");
        }

        // Seed editor camera state from the first Camera entity's Transform if present,
        // else use a sensible default position (5 units back, slightly above origin).
        {
            use fluxion_core::{Transform, Camera};
            let mut pos   = [0.0f64, 2.0f64, 5.0f64];
            let mut yaw   = 0.0f64;
            let mut pitch = -0.261799f64; // -15 degrees
            host.world.query_active::<(&Transform, &Camera), _>(|_, (t, c)| {
                if c.is_active {
                    pos = [t.position.x as f64, t.position.y as f64, t.position.z as f64];
                    let (p, y, _) = t.rotation.to_euler(glam::EulerRot::XYZ);
                    yaw   = y as f64;
                    pitch = p as f64;
                }
            });
            crate::rune_bindings::init_editor_cam(pos, yaw, pitch);
        }

        let scene_name = scene_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string());
        let _ = scene_name;

        // Set up asset file watcher
        let (watcher_opt, rx_opt) = {
            let assets_dir = choice.root.join("assets");
            let (tx, rx) = std::sync::mpsc::channel();
            let watcher_result = notify::recommended_watcher(move |res| {
                let _ = tx.send(res);
            });
            match watcher_result {
                Ok(mut w) => {
                    if assets_dir.exists() {
                        if let Err(e) = w.watch(&assets_dir, RecursiveMode::Recursive) {
                            log::warn!("Asset watcher: {e}");
                        } else {
                            log::info!("Asset watcher active on {:?}", assets_dir);
                        }
                    }
                    (Some(w), Some(rx))
                }
                Err(e) => {
                    log::warn!("Asset watcher init failed: {e}");
                    (None, None)
                }
            }
        };

        let mut inner = EditorInner {
            window,
            host,
            renderer,
            ui_shell,
            dock_state:     default_dock_state(),
            project:        choice.config,
            project_root:   choice.root,
            scene_path,
            scene_dirty:    false,
            editor_mode:    EditorMode::Editing,
            transform_tool: TransformTool::Translate,
            modifiers:      ModifiersState::default(),
            gizmo_drag:     viewport_gizmo::GizmoDragState::default(),
            snap_raw_pos:      glam::Vec3::ZERO,
            snap_raw_scale:    glam::Vec3::ONE,
            snap_was_dragging: false,
            theme_applied:  false,
            _file_watcher:  watcher_opt,
            file_watcher_rx: rx_opt,
            file_watcher_cooldown: 0.0,
            editor_prefs: EditorPrefs::default(),
            autosave_timer: 0.0,
        };

        // Push project root so Rune asset browser can enumerate files.
        crate::rune_bindings::set_project_root(&inner.project_root);
        // Also store in host so gameplay scripts can resolve asset paths.
        inner.host.project_root = inner.project_root.clone();

        // Scan the asset database now that we know the project root.
        inner.host.asset_db.scan(&inner.project_root);
        log::info!("AssetDatabase: {} assets indexed", inner.host.asset_db.count());
        crate::rune_bindings::load_asset_refs(&inner.project_root);

        // Load editor preferences and push settings context to Rune.
        let prefs = load_editor_prefs();
        // Apply prefs immediately
        crate::rune_bindings::world_module::set_editor_cam_speed(prefs.camera_speed as f64);
        crate::rune_bindings::world_module::set_asset_view_mode(&prefs.asset_view_mode);
        crate::rune_bindings::set_settings_context(
            inner.project.clone(),
            prefs.clone(),
            inner.project_root.clone(),
        );
        inner.editor_prefs = prefs;

        *self = EditorApp::Running(Rc::new(RefCell::new(inner)));
    }
}

// ── EditorInner ───────────────────────────────────────────────────────────────

impl EditorInner {
    fn frame(&mut self) {
        // Engine tick (skip physics while in Editing mode to avoid moving things)
        if self.editor_mode == EditorMode::Playing {
            self.host.tick();
        } else {
            // Still need transform propagation + hot reload even when paused.
            self.host.tick_editor_only();
        }

        // Push ECS context so Rune panels can read data this frame.
        // Guard clears thread-locals on drop (even on panic).
        let _world_ctx = self.host.push_world_context();

        // Push undo state so Rune can show Undo/Redo availability.
        crate::rune_bindings::set_undo_state(
            self.host.undo.can_undo(),
            self.host.undo.can_redo(),
        );

        // Push frame time for the debugger panel.
        crate::rune_bindings::set_frame_time(self.host.time.dt as f64 * 1000.0);
        crate::rune_bindings::set_time_elapsed(self.host.time.elapsed as f64);

        // Bake any dirty CsgShape components → upload scaled GPU mesh.
        self.renderer.upload_csg_meshes(&mut self.host.world);

        // Track which entity is the active editor camera (for hierarchy hiding).
        {
            use fluxion_core::{Transform, Camera};
            let mut cam_entity = None;
            self.host.world.query_active::<(&Transform, &Camera), _>(|id, (_, c)| {
                if c.is_active && cam_entity.is_none() {
                    cam_entity = Some(id);
                }
            });
            crate::rune_bindings::set_editor_cam_entity(cam_entity);
        }

        // Render 3-D scene to offscreen viewport texture(s).
        // Pane 0 is always the perspective view; panes 1-3 are ortho (Top/Front/Right).
        // Only render panes that are active in the current layout.
        use crate::rune_bindings::viewport_module::{get_layout, PaneKind};
        let layout = get_layout();

        // In editing/paused mode, pane 0 uses the virtual editor camera as an override so that
        // game Camera entities are never touched by the editor and keep their game-defined
        // position/rotation at all times. In play mode, no override → extract_camera picks the
        // game camera (preferring is_main=true if available).
        let editor_cam_override: Option<fluxion_renderer::CameraOverride> =
            if self.editor_mode != crate::toolbar::EditorMode::Playing
        {
            use fluxion_core::Camera;
            let pos   = crate::rune_bindings::get_editor_cam_pos();
            let yaw   = crate::rune_bindings::get_editor_cam_yaw()  as f32;
            let pitch = crate::rune_bindings::get_editor_cam_pitch() as f32;
            let eye   = glam::Vec3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32);
            let rot   = glam::Quat::from_euler(glam::EulerRot::YXZ, yaw, pitch, 0.0);
            let view  = glam::Mat4::look_at_rh(eye, eye + rot * glam::Vec3::NEG_Z, rot * glam::Vec3::Y);
            // Use the first active Camera entity's projection settings for a faithful preview.
            let w = self.renderer.width;
            let h = self.renderer.height;
            let mut proj = glam::Mat4::perspective_rh(
                60_f32.to_radians(),
                w as f32 / h.max(1) as f32,
                0.1, 1000.0,
            );
            self.host.world.query_active::<&Camera, _>(|_, cam| {
                if cam.is_active {
                    proj = cam.projection_matrix_ex(w, h);
                }
            });
            Some(fluxion_renderer::CameraOverride { view, proj, position: eye })
        } else {
            None
        };

        for &pane in layout.active_panes() {
            let cam_override = if pane > 0 {
                // Build a simple orthographic override for ortho panes.
                use fluxion_renderer::CameraOverride;
                let kind = PaneKind::from_idx(pane);
                let w = self.renderer.width.max(1) as f32;
                let h = self.renderer.height.max(1) as f32;
                let (pane_w, pane_h) = (w / 2.0, h / 2.0);
                let ortho_size = 8.0f32;
                let proj = glam::Mat4::orthographic_rh(
                    -ortho_size * pane_w / pane_h, ortho_size * pane_w / pane_h,
                    -ortho_size, ortho_size,
                    0.1, 1000.0,
                );
                let (view, pos) = match kind {
                    PaneKind::Top   => (
                        glam::Mat4::look_at_rh(
                            glam::vec3(0.0, 20.0, 0.0),
                            glam::Vec3::ZERO,
                            glam::Vec3::Z,
                        ),
                        glam::vec3(0.0, 20.0, 0.0),
                    ),
                    PaneKind::Front => (
                        glam::Mat4::look_at_rh(
                            glam::vec3(0.0, 0.0, 20.0),
                            glam::Vec3::ZERO,
                            glam::Vec3::Y,
                        ),
                        glam::vec3(0.0, 0.0, 20.0),
                    ),
                    _ => (
                        glam::Mat4::look_at_rh(
                            glam::vec3(20.0, 0.0, 0.0),
                            glam::Vec3::ZERO,
                            glam::Vec3::Y,
                        ),
                        glam::vec3(20.0, 0.0, 0.0),
                    ),
                };
                Some(CameraOverride { view, proj, position: pos })
            } else {
                // Pane 0: use editor virtual camera in editing/paused; None in play mode.
                editor_cam_override.clone()
            };
            let dbg = if pane == 0 {
                crate::rune_bindings::viewport_module::get_debug_view()
            } else {
                0
            };
            // prefer_main only matters when cam_override is None (i.e. play mode).
            let prefer_main = pane == 0 && cam_override.is_none();
            if let Err(e) = self.renderer.render_to_pane(&self.host.world, &self.host.time, pane, cam_override, prefer_main, dbg) {
                log::error!("render_to_pane({pane}): {e}");
            }
        }

        // Push camera snapshot so Rune scripts can use screen↔world math.
        {
            use crate::rune_bindings::{set_camera_snapshot, CameraSnapshot};
            let vp = self.renderer.last_proj_matrix * self.renderer.last_view_matrix;
            let inv_vp = vp.inverse();
            let cam_pos = {
                use fluxion_core::components::Camera;
                let mut pos = glam::Vec3::ZERO;
                self.host.world.query_active::<(&fluxion_core::Transform, &Camera), _>(|_, (t, c)| {
                    if c.is_active { pos = t.world_position; }
                });
                pos
            };
            set_camera_snapshot(CameraSnapshot {
                view_proj:     vp.to_cols_array_2d(),
                inv_view_proj: inv_vp.to_cols_array_2d(),
                position:      cam_pos.to_array(),
                viewport_w:    self.renderer.width,
                viewport_h:    self.renderer.height,
            });
        }

        // Register / update per-pane viewport textures with egui.
        let vp_w = self.renderer.width;
        let vp_h = self.renderer.height;
        for &pane in get_layout().active_panes() {
            if let Some(view) = self.renderer.viewport_view_for_pane(pane) {
                let tid = self.ui_shell.register_viewport_texture(
                    &self.renderer.device,
                    view,
                    pane,
                );
                if pane == 0 {
                    set_viewport_texture(tid, vp_w, vp_h);
                } else {
                    crate::rune_bindings::set_pane_texture(pane, tid);
                }
            }
        }
        if vp_w > 0 {
            self.host.vm.push_viewport(vp_w, vp_h);
        }

        let w      = self.renderer.width;
        let h      = self.renderer.height;
        let window = self.window.clone();

        // Pre-extract gizmo data (selected entity world pos + camera matrices)
        let gizmo_view = self.renderer.last_view_matrix;
        let gizmo_proj = self.renderer.last_proj_matrix;
        let gizmo_sel_pos: Option<glam::Vec3> = {
            use fluxion_core::transform::Transform;
            crate::rune_bindings::get_selected_id()
                .and_then(|id| self.host.world.get_component::<Transform>(id)
                    .map(|t| t.world_position))
        };
        let gizmo_csg_size: Option<[f32; 3]> = {
            use fluxion_core::components::CsgShape;
            crate::rune_bindings::get_selected_id()
                .and_then(|id| self.host.world.get_component::<CsgShape>(id)
                    .map(|c| c.size))
        };

        let ui_shell        = &mut self.ui_shell;
        let dock_state      = &mut self.dock_state;
        let vm              = &mut self.host.vm;
        let editor_mode     = &mut self.editor_mode;
        let transform_tool  = &mut self.transform_tool;
        let theme_applied   = &mut self.theme_applied;
        let gizmo_drag      = &mut self.gizmo_drag;
        let project         = &self.project;
        let scene_path      = &self.scene_path;
        let scene_dirty     = self.scene_dirty;
        let _project_root   = &self.project_root;

        let scene_name = scene_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string());
        let title = if scene_dirty {
            format!("{} — {}*  |  FluxionRS Editor", project.name, scene_name)
        } else {
            format!("{} — {}  |  FluxionRS Editor", project.name, scene_name)
        };
        window.set_title(&title);

        // Push current editor state to Rune thread-locals before UI calls.
        {
            let mode_str = match *editor_mode {
                crate::toolbar::EditorMode::Editing => "Editing",
                crate::toolbar::EditorMode::Playing => "Playing",
                crate::toolbar::EditorMode::Paused  => "Paused",
            };
            let tool_str = match *transform_tool {
                crate::toolbar::TransformTool::Translate => "Translate",
                crate::toolbar::TransformTool::Rotate    => "Rotate",
                crate::toolbar::TransformTool::Scale     => "Scale",
            };
            crate::rune_bindings::set_editor_shell_state(
                mode_str, tool_str, &project.name, &scene_name,
            );
        }

        // Deferred scene-save request from menu / toolbar.
        let mut do_save_scene  = false;
        let mut do_open_scene  = false;
        let mut do_new_scene   = false;
        let mut do_load_scene: Option<std::path::PathBuf> = None;

        let result = self.renderer.render_ui_only(|device, queue, encoder, view| {
            ui_shell.paint(&window, device, queue, encoder, view, w, h, |ui| {
                let ctx = ui.ctx().clone();
                // Cache ctx once per frame so settings window bindings can use it
                // without requiring a live CURRENT_UI pointer.
                crate::rune_bindings::set_egui_ctx(&ctx);
                // Install SVG/image loaders (idempotent after first call).
                crate::icons::install_loaders(&ctx);

                if !*theme_applied {
                    theme::apply_theme(&ctx);
                    *theme_applied = true;
                }

                // ── Menu bar (Rune-driven) ───────────────────────────────
                egui::Panel::top("editor_menu")
                    .frame(egui::Frame::NONE
                        .fill(crate::theme::MENU_BG)
                        .inner_margin(egui::Margin::symmetric(4, 2)))
                    .show_inside(ui, |ui| {
                        egui::MenuBar::new().ui(ui, |ui| {
                            let _guard = crate::rune_bindings::set_current_ui(ui);
                            if let Err(e) = vm.call_fn(&["menubar", "panel"], ()) {
                                log::error!("menubar::panel: {e:#}");
                            }
                        });
                    });

                // ── Toolbar (Rune-driven) ────────────────────────────────────
                egui::Panel::top("toolbar_panel")
                    .exact_size(32.0)
                    .frame(egui::Frame::NONE
                        .fill(crate::theme::TOOLBAR_BG)
                        .inner_margin(egui::Margin::symmetric(6, 4)))
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            let _guard = crate::rune_bindings::set_current_ui(ui);
                            if let Err(e) = vm.call_fn(&["toolbar", "panel"], ()) {
                                log::error!("toolbar::panel: {e:#}");
                            }
                        });
                    });

                // ── Dock area ───────────────────────────────────────────────
                show_dock(ui, dock_state, vm);

                // ── Settings modals (run every frame, ctx-only, no UI pointer needed) ─
                if let Err(e) = vm.call_fn(&["settings", "project_panel"], ()) {
                    log::error!("settings::project_panel: {e:#}");
                }
                if let Err(e) = vm.call_fn(&["settings", "editor_panel"], ()) {
                    log::error!("settings::editor_panel: {e:#}");
                }

                // ── Editor camera update ─────────────────────────────────────
                // Called after show_dock so VP_RESPONSE is already set by viewport::panel.
                // Runs only in Editing/Paused mode (editor_camera.rn checks mode internally).
                {
                    let dt = self.host.time.dt as f64;
                    if let Err(e) = vm.call_fn(&["editor_camera", "update"], (dt,)) {
                        log::warn!("editor_camera::update: {e:#}");
                    }
                    // Reset raw delta accumulator after camera script has read it.
                    crate::rune_bindings::drain_raw_mouse_delta();
                }

                // ── Viewport gizmo overlay ───────────────────────────────────
                // VP_RECT is set by viewport.rn after image_interactive.
                // We read it here and overlay the gizmo using an egui Area.
                let vp_rect = crate::rune_bindings::get_viewport_rect();
                if vp_rect.is_positive() {
                    if let Some(world_pos) = gizmo_sel_pos {
                        let box_mode_raw = crate::rune_bindings::get_box_gizmo_mode_raw();

                        // If box gizmo mode is active but the selected entity has no
                        // CsgShape, clear the stale mode so we fall through to the
                        // arrow gizmo. This prevents a ghost Area being left from the
                        // previous selection.
                        if box_mode_raw != 0 && gizmo_csg_size.is_none() {
                            crate::rune_bindings::world_module::set_box_gizmo_mode_raw(0);
                        }

                        // Always render the overlay Area so egui never leaves a ghost
                        // from a previous frame. Content is chosen inside the closure.
                        egui::Area::new(egui::Id::new("gizmo_overlay"))
                            .fixed_pos(vp_rect.min)
                            .order(egui::Order::Foreground)
                            .show(&ctx, |ui| {
                                ui.set_clip_rect(vp_rect);

                                // Box gizmo when CsgShape is present.
                                if box_mode_raw != 0 {
                                    if let Some(csg_size) = gizmo_csg_size {
                                        let box_mode = if box_mode_raw == 1 {
                                            viewport_gizmo::GizmoMode::BoxFaceHandles
                                        } else {
                                            viewport_gizmo::GizmoMode::BoxAxisArrows
                                        };
                                        viewport_gizmo::draw_box_and_interact(
                                            ui, vp_rect, world_pos, csg_size,
                                            gizmo_view, gizmo_proj, box_mode,
                                            gizmo_drag,
                                        );
                                        return;
                                    }
                                }

                                // Fallback: standard translate/rotate/scale gizmo.
                                let mode = match *transform_tool {
                                    TransformTool::Translate => viewport_gizmo::GizmoMode::Translate,
                                    TransformTool::Rotate    => viewport_gizmo::GizmoMode::Rotate,
                                    TransformTool::Scale     => viewport_gizmo::GizmoMode::Scale,
                                };
                                viewport_gizmo::draw_and_interact(
                                    ui, vp_rect, world_pos,
                                    gizmo_view, gizmo_proj, mode,
                                    gizmo_drag,
                                );
                            });
                    }
                }
            })
        });

        // Consume action signals queued by Rune scripts this frame.
        for signal in crate::rune_bindings::drain_action_signals() {
            match signal.as_str() {
                "new_scene"      => do_new_scene  = true,
                "open_scene"     => do_open_scene = true,
                "save_scene"     => do_save_scene = true,
                "exit"           => WANT_QUIT.store(true, Ordering::Relaxed),
                "rescan_assets"  => {
                    self.host.asset_db.scan(&self.project_root);
                    self.host.physmat_cache.clear();
                    log::info!("AssetDatabase rescan: {} assets", self.host.asset_db.count());
                    crate::rune_bindings::rescan_asset_refs(
                        &self.host.world,
                        &self.host.registry,
                        &self.host.asset_db,
                        &self.project_root,
                    );
                }
                s if s.starts_with("update_asset_paths:") => {
                    let rest = &s["update_asset_paths:".len()..];
                    if let Some(tab) = rest.find('\t') {
                        let old_path = &rest[..tab];
                        let new_path = &rest[tab + 1..];
                        crate::rune_bindings::apply_path_rename(
                            old_path, new_path,
                            &self.host.world,
                            &self.host.registry,
                        );
                    }
                }
                s if s.starts_with("clear_asset_refs:") => {
                    let guid = &s["clear_asset_refs:".len()..];
                    crate::rune_bindings::clear_asset_refs_by_guid(
                        guid,
                        &self.host.world,
                        &self.host.registry,
                    );
                }
                "do_undo" => {
                    let world    = &self.host.world    as *const _;
                    let registry = &self.host.registry as *const _;
                    unsafe { self.host.undo.undo(&*world, &*registry); }
                }
                "do_redo" => {
                    let world    = &self.host.world    as *const _;
                    let registry = &self.host.registry as *const _;
                    unsafe { self.host.undo.redo(&*world, &*registry); }
                }
                s if s.starts_with("load_scene:") => {
                    let rel = &s["load_scene:".len()..];
                    let path = if std::path::Path::new(rel).is_absolute() {
                        std::path::PathBuf::from(rel)
                    } else {
                        // Asset-panel paths are relative to assets/ — try that first.
                        let with_assets = self.project_root.join("assets").join(rel);
                        if with_assets.exists() {
                            with_assets
                        } else {
                            let direct = self.project_root.join(rel);
                            direct // fall back; load_scene_from_path will log the error
                        }
                    };
                    do_load_scene = Some(path);
                }
                _                => {}
            }
        }

        // Apply cursor grab/visibility requests from Rune scripts.
        if let Some(grab) = crate::rune_bindings::drain_cursor_grab() {
            use winit::window::CursorGrabMode;
            if grab {
                // Try Locked first (hides + confines), fall back to Confined.
                let _ = self.window.set_cursor_grab(CursorGrabMode::Locked)
                    .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined));
            } else {
                let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            }
        }
        if let Some(visible) = crate::rune_bindings::drain_cursor_visible() {
            self.window.set_cursor_visible(visible);
        }

        // (Editor camera is now rendered via CameraOverride — no longer writes to Camera entity)

        // Sync editor mode and transform tool from Rune state.
        let mode_str = crate::rune_bindings::get_editor_mode_str();
        let prev_mode = editor_mode.clone();
        *editor_mode = match mode_str.as_str() {
            "Playing" => crate::toolbar::EditorMode::Playing,
            "Paused"  => crate::toolbar::EditorMode::Paused,
            _         => crate::toolbar::EditorMode::Editing,
        };
        // Rebuild gameplay scripts when transitioning INTO play mode.
        if *editor_mode == crate::toolbar::EditorMode::Playing
            && prev_mode != crate::toolbar::EditorMode::Playing
        {
            use fluxion_core::Camera;
            let cam_count = {
                let mut n = 0usize;
                self.host.world.query_active::<&Camera, _>(|_, c| {
                    if c.is_active { n += 1; }
                });
                n
            };
            if cam_count == 0 {
                log::error!("[Editor] Cannot start Play mode: no active Camera in the scene.");
                *editor_mode = crate::toolbar::EditorMode::Editing;
                crate::rune_bindings::force_editor_mode("Editing");
            } else {
                self.host.rebuild_gameplay_scripts();
            }
        }
        let tool_str = crate::rune_bindings::get_transform_tool_str();
        *transform_tool = match tool_str.as_str() {
            "Rotate" => crate::toolbar::TransformTool::Rotate,
            "Scale"  => crate::toolbar::TransformTool::Scale,
            _        => crate::toolbar::TransformTool::Translate,
        };

        // Apply gizmo drag delta to selected entity transform / CsgShape.
        let drag_active_now = self.gizmo_drag.active_axis.is_some()
            || self.gizmo_drag.box_drag_face.is_some();
        if let Some((idx, delta, mode)) = self.gizmo_drag.pending_delta.take() {
            if let Some(sel_id) = crate::rune_bindings::get_selected_id() {
                use fluxion_core::transform::Transform;
                use crate::viewport_gizmo::GizmoMode;

                // ── Box resize modes (CsgShape) ──────────────────────────────────────
                match mode {
                    GizmoMode::BoxFaceHandles => {
                        use fluxion_core::components::CsgShape;
                        let axis_idx = idx / 2;
                        let sign = if idx % 2 == 0 { 1.0_f32 } else { -1.0_f32 };
                        if let Some(mut csg) = self.host.world.get_component_mut::<CsgShape>(sel_id) {
                            csg.size[axis_idx] = (csg.size[axis_idx] + delta).max(0.01);
                            csg.dirty = true;
                        }
                        if let Some(mut t) = self.host.world.get_component_mut::<Transform>(sel_id) {
                            match axis_idx {
                                0 => t.position.x += sign * delta * 0.5,
                                1 => t.position.y += sign * delta * 0.5,
                                _ => t.position.z += sign * delta * 0.5,
                            }
                            t.dirty = true;
                        }
                        self.scene_dirty = true;
                    }
                    GizmoMode::BoxAxisArrows => {
                        use fluxion_core::components::CsgShape;
                        if let Some(mut csg) = self.host.world.get_component_mut::<CsgShape>(sel_id) {
                            csg.size[idx] = (csg.size[idx] + delta * 2.0).max(0.01);
                            csg.dirty = true;
                        }
                        self.scene_dirty = true;
                    }
                    _ => {
                // ── Regular transform gizmo ───────────────────────────────────────
                let snap = crate::rune_bindings::get_snap_enabled();
                let axis = idx;
                if let Some(mut t) = self.host.world.get_component_mut::<Transform>(sel_id) {
                    if snap {
                        // On first frame of drag, capture the raw position/scale.
                        if !self.snap_was_dragging {
                            self.snap_raw_pos   = t.position;
                            self.snap_raw_scale = t.scale;
                        }
                        match (mode, axis) {
                            (viewport_gizmo::GizmoMode::Translate, 0) => {
                                self.snap_raw_pos.x += delta;
                                let s = crate::rune_bindings::get_snap_translate() as f32;
                                t.position.x = (self.snap_raw_pos.x / s).round() * s;
                            }
                            (viewport_gizmo::GizmoMode::Translate, 1) => {
                                self.snap_raw_pos.y += delta;
                                let s = crate::rune_bindings::get_snap_translate() as f32;
                                t.position.y = (self.snap_raw_pos.y / s).round() * s;
                            }
                            (viewport_gizmo::GizmoMode::Translate, 2) => {
                                self.snap_raw_pos.z += delta;
                                let s = crate::rune_bindings::get_snap_translate() as f32;
                                t.position.z = (self.snap_raw_pos.z / s).round() * s;
                            }
                            (viewport_gizmo::GizmoMode::Rotate, axis_idx) => {
                                // Accumulate rotation delta; only apply when it crosses a step.
                                let step = crate::rune_bindings::get_snap_rotate().to_radians() as f32;
                                let snapped = (delta / step).round() * step;
                                if snapped.abs() > 1e-6 {
                                    let rot_axis = match axis_idx {
                                        0 => glam::Vec3::X,
                                        1 => glam::Vec3::Y,
                                        _ => glam::Vec3::Z,
                                    };
                                    t.rotation = (glam::Quat::from_axis_angle(rot_axis, snapped) * t.rotation).normalize();
                                }
                            }
                            (viewport_gizmo::GizmoMode::Scale, 0) => {
                                self.snap_raw_scale.x += delta;
                                let s = crate::rune_bindings::get_snap_scale() as f32;
                                t.scale.x = ((self.snap_raw_scale.x / s).round() * s).max(0.001);
                            }
                            (viewport_gizmo::GizmoMode::Scale, 1) => {
                                self.snap_raw_scale.y += delta;
                                let s = crate::rune_bindings::get_snap_scale() as f32;
                                t.scale.y = ((self.snap_raw_scale.y / s).round() * s).max(0.001);
                            }
                            (viewport_gizmo::GizmoMode::Scale, 2) => {
                                self.snap_raw_scale.z += delta;
                                let s = crate::rune_bindings::get_snap_scale() as f32;
                                t.scale.z = ((self.snap_raw_scale.z / s).round() * s).max(0.001);
                            }
                            _ => {}
                        }
                    } else {
                        match (mode, axis) {
                            (viewport_gizmo::GizmoMode::Translate, 0) => t.position.x += delta,
                            (viewport_gizmo::GizmoMode::Translate, 1) => t.position.y += delta,
                            (viewport_gizmo::GizmoMode::Translate, 2) => t.position.z += delta,
                            (viewport_gizmo::GizmoMode::Rotate, axis_idx) => {
                                let rot_axis = match axis_idx {
                                    0 => glam::Vec3::X,
                                    1 => glam::Vec3::Y,
                                    _ => glam::Vec3::Z,
                                };
                                t.rotation = (glam::Quat::from_axis_angle(rot_axis, delta) * t.rotation).normalize();
                            }
                            (viewport_gizmo::GizmoMode::Scale, 0) => t.scale.x = (t.scale.x + delta).max(0.001),
                            (viewport_gizmo::GizmoMode::Scale, 1) => t.scale.y = (t.scale.y + delta).max(0.001),
                            (viewport_gizmo::GizmoMode::Scale, 2) => t.scale.z = (t.scale.z + delta).max(0.001),
                            _ => {}
                        }
                    }
                    t.dirty = true;
                }
                    } // end _ => (transform gizmo arm)
                } // end match mode
            }
        }
        self.snap_was_dragging = drag_active_now;

        // Push per-frame stats for the viewport overlay.
        {
            let draw_calls = self.renderer.last_draw_call_count;
            let entity_count = self.host.world.all_entities().count() as u32;
            crate::rune_bindings::set_frame_stats(draw_calls, entity_count);
        }

        // Poll file watcher — auto-rescan assets on changes (debounced).
        {
            let dt = self.host.time.dt;
            if self.file_watcher_cooldown > 0.0 {
                self.file_watcher_cooldown -= dt;
            } else if let Some(ref rx) = self.file_watcher_rx {
                let mut got_event = false;
                let mut rn_changed = false;
                while let Ok(ev) = rx.try_recv() {
                    got_event = true;
                    if let Ok(event) = ev {
                        for path in &event.paths {
                            if path.extension().and_then(|e| e.to_str()) == Some("rn") {
                                rn_changed = true;
                            }
                        }
                    }
                }
                if got_event {
                    self.host.asset_db.scan(&self.project_root);
                    self.host.physmat_cache.clear();
                    log::info!("Asset watcher: rescan triggered ({} assets)", self.host.asset_db.count());
                    crate::rune_bindings::rescan_asset_refs(
                        &self.host.world, &self.host.registry,
                        &self.host.asset_db, &self.project_root,
                    );
                    self.file_watcher_cooldown = 0.5; // 500 ms debounce

                    // Hot-reload gameplay scripts if any .rn file changed while playing.
                    if rn_changed {
                        let is_playing = crate::rune_bindings::get_editor_mode_str() == "Playing";
                        if is_playing {
                            self.host.rebuild_gameplay_scripts();
                            log::info!("Script hot-reload: rebuilt gameplay scripts after .rn change");
                        }
                        crate::rune_bindings::set_script_hotreload_pending(true);
                    }
                }
            }
        }

        // Drain material hot-reload signals queued by flush_pending_edits.
        {
            let reloads: Vec<String> = std::mem::take(&mut self.host.pending_material_reloads);
            for path in reloads {
                // Resolve the project-relative path to an absolute path for disk reads.
                // Try project_root/assets/<path> first, then project_root/<path>.
                let abs = {
                    let with_assets = self.project_root.join("assets").join(&path);
                    if with_assets.exists() {
                        with_assets.to_string_lossy().to_string()
                    } else {
                        let direct = self.project_root.join(&path);
                        if direct.exists() {
                            direct.to_string_lossy().to_string()
                        } else {
                            path.clone()
                        }
                    }
                };
                if let Err(e) = self.renderer.reload_material(&mut self.host.world, &path) {
                    // Retry with the absolute path if the relative path failed.
                    if abs != path {
                        if let Err(e2) = self.renderer.reload_material_abs(&mut self.host.world, &path, &abs) {
                            log::warn!("material hot-reload '{path}': {e2}");
                        }
                    } else {
                        log::warn!("material hot-reload '{path}': {e}");
                    }
                }
            }
            if std::mem::take(&mut self.host.needs_asset_rescan) {
                self.host.asset_db.scan(&self.project_root);
                crate::rune_bindings::set_asset_db_context(&self.host.asset_db);
            }
        }

        // Handle file menu actions (after the render closure, to avoid borrow issues).
        if do_save_scene {
            self.save_scene();
        }
        if do_open_scene {
            self.open_scene_dialog();
        }
        if do_new_scene {
            self.new_scene();
        }
        if let Some(path) = do_load_scene {
            self.load_scene_from_path(path);
        }

        // ── Settings saves (drain dirty flags written by Rune settings panel) ──
        {
            let (proj_save, prefs_save) = crate::rune_bindings::drain_settings_saves();

            if let Some((cfg, root)) = proj_save {
                // Persist to disk atomically.
                match save_project(&root, &cfg) {
                    Ok(()) => log::info!("Project settings saved."),
                    Err(e) => log::error!("Failed to save project settings: {e}"),
                }
                self.project = cfg;
            }

            if let Some(prefs) = prefs_save {
                // Apply live camera speed
                crate::rune_bindings::world_module::set_editor_cam_speed(prefs.camera_speed as f64);
                crate::rune_bindings::world_module::set_asset_view_mode(&prefs.asset_view_mode);
                match save_editor_prefs(&prefs) {
                    Ok(()) => log::info!("Editor preferences saved."),
                    Err(e) => log::error!("Failed to save editor prefs: {e}"),
                }
                self.editor_prefs = prefs;
            }
        }

        // ── Autosave ──────────────────────────────────────────────────────────
        {
            let interval = self.editor_prefs.autosave_interval_secs;
            if interval > 0 && self.scene_dirty {
                let dt = self.host.time.dt;
                self.autosave_timer += dt;
                if self.autosave_timer >= interval as f32 {
                    self.autosave_timer = 0.0;
                    self.save_scene();
                    log::info!("Autosave triggered.");
                }
            } else {
                self.autosave_timer = 0.0;
            }
        }

        // _world_ctx drops here, clearing world thread-locals.
        drop(_world_ctx);
        // Clear physics context pointer after every frame.
        self.host.clear_rune_context();

        if !result {
            let size = self.window.inner_size();
            self.renderer.resize(size.width, size.height);
        }
    }

    // ── Scene operations ──────────────────────────────────────────────────────

    pub fn save_scene(&mut self) {
        let path = if let Some(p) = &self.scene_path {
            p.clone()
        } else {
            // No path yet — prompt for one.
            if let Some(p) = rfd::FileDialog::new()
                .add_filter("Fluxion Scene", &["scene"])
                .set_directory(&self.project_root)
                .save_file()
            {
                self.scene_path = Some(p.clone());
                p
            } else {
                return;
            }
        };

        let settings = SceneSettings::default();
        let scene_name = path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "scene".to_string());
        let data = world_to_scene_data(&self.host.world, &self.host.registry, scene_name, settings, None);
        match save_scene_file(path.to_str().unwrap_or(""), &data) {
            Ok(()) => {
                self.scene_dirty = false;
                log::info!("Scene saved to {}", path.display());
            }
            Err(e) => log::error!("Save scene failed: {e}"),
        }
    }

    pub fn open_scene_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Fluxion Scene", &["scene"])
            .set_directory(&self.project_root)
            .pick_file()
        {
            self.load_scene_from_path(path);
        }
    }

    pub fn new_scene(&mut self) {
        self.host.world.clear();
        host::EditorHost::spawn_default_scene_pub(&mut self.host.world);
        self.host.camera_manager.rebuild(&self.host.world);
        self.scene_path  = None;
        self.scene_dirty = false;
        log::info!("New scene created");
    }

    fn load_scene_from_path(&mut self, path: std::path::PathBuf) {
        match load_scene_file(path.to_str().unwrap_or("")) {
            Ok(data) => {
                self.host.world.clear();
                if let Err(e) = load_scene_into_world(
                    &mut self.host.world, &data, true, &self.host.registry,
                ) {
                    log::error!("Scene load failed: {e}");
                    return;
                }
                // Re-seed editor camera from scene's Camera entity (or default).
                {
                    use fluxion_core::{Transform, Camera};
                    let mut pos   = [0.0f64, 2.0f64, 5.0f64];
                    let mut yaw   = 0.0f64;
                    let mut pitch = -0.261799f64;
                    self.host.world.query_active::<(&Transform, &Camera), _>(|_, (t, c)| {
                        if c.is_active {
                            pos = [t.position.x as f64, t.position.y as f64, t.position.z as f64];
                            let (p, y, _) = t.rotation.to_euler(glam::EulerRot::XYZ);
                            yaw   = y as f64;
                            pitch = p as f64;
                        }
                    });
                    crate::rune_bindings::init_editor_cam(pos, yaw, pitch);
                }
                self.host.camera_manager.rebuild(&self.host.world);
                self.scene_path  = Some(path);
                self.scene_dirty = false;
                log::info!("Scene loaded");
            }
            Err(e) => log::error!("Open scene failed: {e}"),
        }
    }

    pub fn delete_selected(&mut self) {
        if let Some(id) = self.host.selected_entity() {
            self.host.world.despawn(id);
            self.scene_dirty = true;
        }
    }
}
