// ============================================================
// fluxion-editor — standalone editor binary
//
// Hosts the full engine runtime (ECS + renderer + physics) and
// presents a hot-reloadable Rune-scripted UI with egui_dock
// docking support.
// ============================================================

mod dock;
mod host;
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

use wgpu::SurfaceError;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
    window::{Window, WindowId},
};

use fluxion_renderer::{FluxionRenderer, RendererConfig};
use fluxion_core::ProjectConfig;
use fluxion_core::scene::{load_scene_into_world, world_to_scene_data, SceneSettings, save_scene_file, load_scene_file};

use crate::dock::{default_dock_state, show_dock, EditorTab};
use crate::host::EditorHost;
use crate::project_chooser::ProjectChooser;
use crate::rune_bindings::set_viewport_texture;
use crate::toolbar::{EditorMode, TransformTool};
use crate::ui_shell::UiShell;

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("wgpu_core", log::LevelFilter::Warn)
        .filter_module("wgpu_hal",  log::LevelFilter::Warn)
        .filter_module("wgpu",      log::LevelFilter::Warn)
        .init();

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

    // Applied once on the first frame; egui theme is static for the session.
    theme_applied:  bool,
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
                        if let Err(SurfaceError::Lost | SurfaceError::Outdated) = result {
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
                                PhysicalKey::Code(KeyCode::Delete) => g.delete_selected(),
                                _ => {}
                            }
                        }
                        if let PhysicalKey::Code(code) = kev.physical_key {
                            g.host.input.set_key_down(&format!("{code:?}"), kev.state == ElementState::Pressed);
                        }
                    }
                    WindowEvent::Resized(size) => g.renderer.resize(size.width, size.height),
                    WindowEvent::RedrawRequested => g.frame(),
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

        // Enable gizmos in editor mode
        renderer.gizmos_enabled = true;

        // Resolve and load the default scene if it exists
        let scene_path = if !choice.config.default_scene.is_empty() {
            let sp = choice.root.join(&choice.config.default_scene);
            if sp.exists() {
                if let Ok(data) = load_scene_file(sp.to_str().unwrap_or("")) {
                    host.world.clear();
                    let _ = load_scene_into_world(&mut host.world, &data, true, &host.registry);
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

        // Seed editor camera state from the active Camera entity's Transform.
        {
            use fluxion_core::{Transform, Camera};
            let mut pos   = [0.0f64; 3];
            let mut yaw   = 0.0f64;
            let mut pitch = 0.0f64;
            host.world.query_active::<(&Transform, &Camera), _>(|_, (t, c)| {
                if c.is_active {
                    pos = [t.position.x as f64, t.position.y as f64, t.position.z as f64];
                    // Extract yaw/pitch from the transform rotation (assumed Euler XYZ).
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

        let inner = EditorInner {
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
            modifiers:      ModifiersState::empty(),
            gizmo_drag:     viewport_gizmo::GizmoDragState::default(),
            theme_applied:  false,
        };

        // Push project root so Rune asset browser can enumerate files.
        crate::rune_bindings::set_project_root(&inner.project_root);

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

        // Render 3-D scene to offscreen viewport texture.
        if let Err(e) = self.renderer.render_to_viewport(&self.host.world, &self.host.time) {
            log::error!("render_to_viewport: {e}");
        }

        // Push camera snapshot so Rune scripts can use screen↔world math.
        {
            use crate::rune_bindings::{set_camera_snapshot, CameraSnapshot};
            let vp = self.renderer.last_view_matrix * self.renderer.last_proj_matrix;
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

        // Register / update the viewport texture with egui.
        if let Some(view) = self.renderer.viewport_view() {
            let vp_w = self.renderer.width;
            let vp_h = self.renderer.height;
            // Register the texture with the egui-wgpu renderer.
            let tid = self.ui_shell.register_viewport_texture(
                &self.renderer.device,
                view,
                vp_w,
                vp_h,
            );
            set_viewport_texture(tid, vp_w, vp_h);
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
        let project_root    = &self.project_root;

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
        let mut do_save_scene = false;
        let mut do_open_scene = false;
        let mut do_new_scene  = false;

        let result = self.renderer.render_ui_only(|device, queue, encoder, view| {
            ui_shell.paint(&window, device, queue, encoder, view, w, h, |ctx| {
                if !*theme_applied {
                    theme::apply_theme(ctx);
                    *theme_applied = true;
                }

                // ── Menu bar (Rune-driven) ───────────────────────────────
                egui::TopBottomPanel::top("editor_menu")
                    .frame(egui::Frame::none()
                        .fill(crate::theme::MENU_BG)
                        .inner_margin(egui::Margin::symmetric(4.0, 2.0)))
                    .show(ctx, |ui| {
                        egui::menu::bar(ui, |ui| {
                            let _guard = crate::rune_bindings::set_current_ui(ui);
                            if let Err(e) = vm.call_fn(&["menubar", "panel"], ()) {
                                log::error!("menubar::panel: {e:#}");
                            }
                        });
                    });

                // ── Toolbar (Rune-driven) ────────────────────────────────────
                egui::TopBottomPanel::top("toolbar_panel")
                    .exact_height(32.0)
                    .frame(egui::Frame::none()
                        .fill(crate::theme::TOOLBAR_BG)
                        .inner_margin(egui::Margin::symmetric(6.0, 4.0)))
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            let _guard = crate::rune_bindings::set_current_ui(ui);
                            if let Err(e) = vm.call_fn(&["toolbar", "panel"], ()) {
                                log::error!("toolbar::panel: {e:#}");
                            }
                        });
                    });

                // ── Dock area ───────────────────────────────────────────────
                show_dock(ctx, dock_state, vm);

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
                        let mode = match *transform_tool {
                            TransformTool::Translate => viewport_gizmo::GizmoMode::Translate,
                            TransformTool::Rotate    => viewport_gizmo::GizmoMode::Rotate,
                            TransformTool::Scale     => viewport_gizmo::GizmoMode::Scale,
                        };
                        egui::Area::new(egui::Id::new("gizmo_overlay"))
                            .fixed_pos(vp_rect.min)
                            .order(egui::Order::Foreground)
                            .show(ctx, |ui| {
                                ui.set_clip_rect(vp_rect);
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
                "new_scene"  => do_new_scene  = true,
                "open_scene" => do_open_scene = true,
                "save_scene" => do_save_scene = true,
                "exit"       => std::process::exit(0),
                _            => {}
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

        // Apply editor camera position/orientation to the Camera entity Transform.
        // Only done in Editing/Paused modes — Playing mode lets the game control the camera.
        if *editor_mode != crate::toolbar::EditorMode::Playing
            && crate::rune_bindings::take_editor_cam_dirty()
        {
            use fluxion_core::{Transform, Camera};
            let pos   = crate::rune_bindings::get_editor_cam_pos();
            let yaw   = crate::rune_bindings::get_editor_cam_yaw()  as f32;
            let pitch = crate::rune_bindings::get_editor_cam_pitch() as f32;
            let rotation = glam::Quat::from_euler(glam::EulerRot::YXZ, yaw, pitch, 0.0);
            let mut applied = false;
            self.host.world.query_active::<(&mut Transform, &Camera), _>(|_, (t, c)| {
                if c.is_active && !applied {
                    t.position   = glam::Vec3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32);
                    t.rotation   = rotation;
                    t.dirty      = true;
                    applied      = true;
                }
            });
        }

        // Sync editor mode and transform tool from Rune state.
        let mode_str = crate::rune_bindings::get_editor_mode_str();
        *editor_mode = match mode_str.as_str() {
            "Playing" => crate::toolbar::EditorMode::Playing,
            "Paused"  => crate::toolbar::EditorMode::Paused,
            _         => crate::toolbar::EditorMode::Editing,
        };
        let tool_str = crate::rune_bindings::get_transform_tool_str();
        *transform_tool = match tool_str.as_str() {
            "Rotate" => crate::toolbar::TransformTool::Rotate,
            "Scale"  => crate::toolbar::TransformTool::Scale,
            _        => crate::toolbar::TransformTool::Translate,
        };

        // Apply gizmo drag delta to selected entity transform.
        if let Some((axis, delta, mode)) = self.gizmo_drag.pending_delta.take() {
            if let Some(sel_id) = crate::rune_bindings::get_selected_id() {
                use fluxion_core::transform::Transform;
                if let Some(mut t) = self.host.world.get_component_mut::<Transform>(sel_id) {
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
                            let rot = glam::Quat::from_axis_angle(rot_axis, delta);
                            t.rotation = (rot * t.rotation).normalize();
                        }
                        (viewport_gizmo::GizmoMode::Scale, 0) => t.scale.x = (t.scale.x + delta).max(0.001),
                        (viewport_gizmo::GizmoMode::Scale, 1) => t.scale.y = (t.scale.y + delta).max(0.001),
                        (viewport_gizmo::GizmoMode::Scale, 2) => t.scale.z = (t.scale.z + delta).max(0.001),
                        _ => {}
                    }
                    t.dirty = true;
                }
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

        // _world_ctx drops here, clearing world thread-locals.
        drop(_world_ctx);
        // Clear physics context pointer after every frame.
        self.host.clear_rune_context();

        match result {
            Ok(()) => {}
            Err(SurfaceError::Lost | SurfaceError::Outdated) => {
                let size = self.window.inner_size();
                self.renderer.resize(size.width, size.height);
            }
            Err(e) => log::error!("Render error: {e}"),
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
