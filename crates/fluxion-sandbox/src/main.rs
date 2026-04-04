// ============================================================
// fluxion-sandbox
//
// Test harness for the FluxionRS engine. Opens a window, creates
// a demo scene (or loads a `.scene` JSON path from argv on native),
// and runs the engine loop.
//
// JS scripting demo (native): `crates/fluxion-sandbox/assets/scripts/spinner.js`
//
// Controls:
//   Esc — exit
//
// Native: optional first CLI argument — path to a FluxionJS-compatible `.scene` file.
// If omitted and `assets/demo_gltf.scene` exists (next to this crate), that loads instead
// of the primitive demo — so `cargo run -p fluxion-sandbox` shows your glTF without extra args.
//
// WASM: renderer initializes asynchronously via `wasm_bindgen_futures::spawn_local`.
// ============================================================

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use glam::{Quat, Vec3};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use fluxion_core::{
    ECSWorld, EntityId, InputState, Time,
    components::{Camera, Light, MeshRenderer, ParticleEmitter},
    components::light::LightType,
    components::mesh_renderer::PrimitiveType,
    step_particle_emitters,
    scene::SceneSettings,
    transform::Transform,
    transform::system::TransformSystem,
};
#[cfg(not(target_arch = "wasm32"))]
use fluxion_core::scene::{load_scene_file, load_scene_into_world};
use fluxion_core::ComponentRegistry;
use fluxion_renderer::{FluxionRenderer, MaterialAsset, RendererConfig};
#[cfg(not(target_arch = "wasm32"))]
use fluxion_renderer::load_renderer_config;
use fluxion_scripting::{
    JsVm, bindings,
    apply_transforms_from_scripts_to_world, sync_transforms_from_world_to_scripts,
};

#[cfg(not(target_arch = "wasm32"))]
mod gamepad;
#[cfg(not(target_arch = "wasm32"))]
mod editor_shell;

// ── Entry point ────────────────────────────────────────────────────────────────

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("wgpu_core", log::LevelFilter::Warn)
        .filter_module("wgpu_hal",  log::LevelFilter::Warn)
        .filter_module("wgpu",      log::LevelFilter::Warn)
        .init();

    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        console_log::init_with_level(log::Level::Info).expect("console_log init failed");
    }

    log::info!("FluxionRS Sandbox — starting");

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = SandboxApp::Uninitialized;

    #[cfg(not(target_arch = "wasm32"))]
    event_loop.run_app(&mut app).expect("Event loop error");

    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(app);
    }
}

// ── Application state ──────────────────────────────────────────────────────────

enum SandboxApp {
    Uninitialized,
    Running(Rc<RefCell<SandboxInner>>),
}

struct SandboxInner {
    window: Arc<Window>,
    world: ECSWorld,
    time: Time,
    scripts: JsVm,
    input: InputState,
    renderer: Option<FluxionRenderer>,
    /// Demo entities for default `setup_materials`; `None` after applied or when a `.scene` was loaded.
    pending_demo: Option<SceneEntities>,
    /// Parent directory of loaded `.scene` (for `.fluxmat` resolution).
    asset_root: Option<PathBuf>,
    /// Scene JSON `settings` applied to the renderer after init (`None` = built-in demo).
    loaded_scene_settings: Option<SceneSettings>,
    /// Per-frame component registry (builtins registered at startup).
    #[cfg(not(target_arch = "wasm32"))]
    registry: ComponentRegistry,
    #[cfg(not(target_arch = "wasm32"))]
    gilrs: Option<gilrs::Gilrs>,
    #[cfg(not(target_arch = "wasm32"))]
    editor: Option<editor_shell::EditorShell>,
    #[cfg(not(target_arch = "wasm32"))]
    editor_state: editor_shell::EditorState,
    #[cfg(not(target_arch = "wasm32"))]
    ui_debug_lines: Vec<String>,
    #[cfg(not(target_arch = "wasm32"))]
    physics_ecs: Option<fluxion_physics::PhysicsEcsWorld>,
    #[cfg(not(target_arch = "wasm32"))]
    _audio_keepalive: Option<fluxion_audio::AudioEngine>,
}

#[cfg(target_arch = "wasm32")]
fn performance_now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

impl SandboxInner {
    fn tick(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        let (fixed_steps, dt) = self.time.tick();
        #[cfg(target_arch = "wasm32")]
        let (fixed_steps, dt) = self.time.tick_wasm(performance_now_ms());

        #[cfg(not(target_arch = "wasm32"))]
        gamepad::poll_gamepad(&mut self.input, &mut self.gilrs);

        for _ in 0..fixed_steps {
            if let Err(e) = self.scripts.fixed_update(self.time.fixed_dt) {
                log::warn!("Script fixed_update error: {e}");
            }
            // Propagate script-driven transforms before physics reads them.
            TransformSystem::update(&mut self.world);

            #[cfg(not(target_arch = "wasm32"))]
            if let Some(ref mut phys) = self.physics_ecs {
                phys.sync_from_ecs(&self.world);        // register new RigidBody entities
                phys.step(self.time.fixed_dt);           // advance simulation
                phys.sync_to_ecs(&self.world);           // write positions → Transform (Dynamic only)
                TransformSystem::update(&mut self.world); // propagate dirty flags to children
            }
        }

        if let Err(e) = bindings::update_time_global(
            &self.scripts, dt, self.time.elapsed, self.time.fixed_dt, self.time.frame_count,
        ) {
            log::warn!("Time global update failed: {e}");
        }

        if let Err(e) = bindings::update_input_global(&self.scripts, &self.input) {
            log::warn!("Input global update failed: {e}");
        }

        if let Err(e) = sync_transforms_from_world_to_scripts(&self.scripts, &self.world) {
            log::warn!("script transform sync (pre): {e}");
        }

        if let Err(e) = self.scripts.update(dt) {
            log::warn!("Script update error: {e}");
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.ui_debug_lines = bindings::drain_ui_debug_lines(&self.scripts);
        }

        if let Err(e) = apply_transforms_from_scripts_to_world(&self.scripts, &mut self.world) {
            log::warn!("script transform sync (post): {e}");
        }

        TransformSystem::update(&mut self.world);

        step_particle_emitters(&mut self.world, dt);


        self.input.begin_frame();

        let Some(ref mut renderer) = self.renderer else {
            return;
        };

        #[cfg(not(target_arch = "wasm32"))]
        {
            let win = self.window.clone();
            let lines = self.ui_debug_lines.clone();
            let dt = self.time.dt;
            let smooth_fps = self.time.smooth_fps;
            let elapsed = self.time.elapsed;
            let frame = self.time.frame_count;
            let w = renderer.width;
            let h = renderer.height;
            let registry = self.registry.clone();
            // Capture a raw pointer to world so we can share it between render_with
            // (which takes &self.world) and the paint closure without a double-borrow.
            let world_ptr: *const ECSWorld = &self.world;
            let res = {
                let editor_state = &mut self.editor_state;
                if let Some(ref mut editor) = self.editor {
                    // SAFETY: world_ptr points to self.world which lives for the duration of
                    // this block. render_with + the closure both take &ECSWorld (shared).
                    let world_ref: &ECSWorld = unsafe { &*world_ptr };
                    renderer.render_with(world_ref, &self.time, |device, queue, enc, view| {
                        editor_shell::paint_editor(
                            editor,
                            editor_state,
                            &win,
                            device,
                            queue,
                            enc,
                            view,
                            w,
                            h,
                            world_ref,
                            &registry,
                            &lines,
                            dt,
                            smooth_fps,
                            elapsed,
                            frame,
                        )
                    })
                } else {
                    let world_ref: &ECSWorld = unsafe { &*world_ptr };
                    renderer.render(world_ref, &self.time)
                }
            };
            match res {
                Ok(()) => {}
                Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                    let size = self.window.inner_size();
                    renderer.resize(size.width, size.height);
                }
                Err(e) => log::error!("Render error: {e}"),
            }
        }

        #[cfg(target_arch = "wasm32")]
        match renderer.render(&self.world, &self.time) {
            Ok(()) => {}
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                let size = self.window.inner_size();
                renderer.resize(size.width, size.height);
            }
            Err(e) => log::error!("Render error: {e}"),
        }
    }
}

fn create_inner(window: Arc<Window>) -> Rc<RefCell<SandboxInner>> {
    let mut world = ECSWorld::new();
    let (pending_demo, asset_root, loaded_scene_settings) = bootstrap_world(&mut world);

    let scripts = JsVm::new().expect("JS VM init failed");
    bindings::setup_bindings(&scripts).expect("JS binding setup failed");

    #[cfg(not(target_arch = "wasm32"))]
    {
        let script_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets/scripts/spinner.js");
        if script_path.is_file() {
            let script_path = script_path.to_string_lossy();
            if let Err(e) = scripts.load_script(script_path.as_ref()) {
                log::warn!("Failed to load spinner script: {e}");
            }
        } else {
            log::info!("No spinner.js at {} — running without JS demo script", script_path.display());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    let grav = loaded_scene_settings
        .as_ref()
        .map(|s| Vec3::from_array(s.physics_gravity))
        .unwrap_or(Vec3::new(0.0, -9.81, 0.0));
    #[cfg(not(target_arch = "wasm32"))]
    let physics_ecs = Some(fluxion_physics::PhysicsEcsWorld::new(grav));
    #[cfg(not(target_arch = "wasm32"))]
    let _audio_keepalive = fluxion_audio::AudioEngine::try_new();

    let inner = Rc::new(RefCell::new(SandboxInner {
        window: window.clone(),
        world,
        time: Time::new(),
        scripts,
        input: InputState::new(),
        renderer: None,
        pending_demo,
        asset_root,
        loaded_scene_settings,
        #[cfg(not(target_arch = "wasm32"))]
        registry: { let mut r = ComponentRegistry::new(); r.register_builtins(); r },
        #[cfg(not(target_arch = "wasm32"))]
        gilrs: gilrs::Gilrs::new().ok(),
        #[cfg(not(target_arch = "wasm32"))]
        editor: None,
        #[cfg(not(target_arch = "wasm32"))]
        editor_state: editor_shell::EditorState::new(),
        #[cfg(not(target_arch = "wasm32"))]
        ui_debug_lines: Vec::new(),
        #[cfg(not(target_arch = "wasm32"))]
        physics_ecs,
        #[cfg(not(target_arch = "wasm32"))]
        _audio_keepalive,
    }));

    #[cfg(not(target_arch = "wasm32"))]
    {
        let renderer_config = load_renderer_config("renderer.config.json")
            .unwrap_or_else(|e| { log::warn!("renderer.config.json: {e}"); RendererConfig::default() });
        let r = pollster::block_on(FluxionRenderer::new(window.clone(), renderer_config)).expect("Renderer init failed");
        inner.borrow_mut().renderer = Some(r);
        finish_renderer_setup(&mut inner.borrow_mut());

        // Write TypeScript declarations to <assets>/types/ for script IDE support.
        {
            let g = inner.borrow();
            let types_dir = std::path::Path::new("assets/types");
            fluxion_scripting::write_dts_files(types_dir, &g.registry);
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        let win = window.clone();
        let weak = Rc::downgrade(&inner);
        wasm_bindgen_futures::spawn_local(async move {
            match FluxionRenderer::new(win, RendererConfig::default()).await {
                Ok(r) => {
                    if let Some(cell) = weak.upgrade() {
                        cell.borrow_mut().renderer = Some(r);
                        finish_renderer_setup(&mut cell.borrow_mut());
                    }
                }
                Err(e) => log::error!("Renderer init failed: {e}"),
            }
        });
    }

    inner
}

fn finish_renderer_setup(g: &mut SandboxInner) {
    let Some(ref mut renderer) = g.renderer else { return };

    if let Some(ref s) = g.loaded_scene_settings {
        renderer.apply_scene_settings(s.clone());
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        g.editor = Some(editor_shell::EditorShell::new(
            g.window.as_ref(),
            &renderer.device,
            renderer.surface_format(),
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let root = g
            .asset_root
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let disk = std::sync::Arc::new(fluxion_core::assets::DiskAssetSource::new(root));
        renderer.set_asset_source(Some(disk.clone()));
        if let Err(e) = renderer.hydrate_mesh_paths_from_source(&mut g.world, disk.as_ref(), None) {
            log::warn!("hydrate_mesh_paths: {e}");
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        let mem = std::sync::Arc::new(fluxion_core::assets::MemoryAssetSource::default());
        renderer.set_asset_source(Some(mem.clone()));
        if let Err(e) = renderer.hydrate_mesh_paths_from_source(&mut g.world, mem.as_ref(), None) {
            log::warn!("hydrate_mesh_paths: {e}");
        }
    }

    if let Some(ref demo) = g.pending_demo {
        setup_materials(renderer, &mut g.world, demo);
        g.pending_demo = None;
    } else if let Err(e) = renderer.hydrate_scene_materials(&mut g.world, g.asset_root.as_deref()) {
        log::warn!("hydrate_scene_materials: {e}");
    }
}

fn bootstrap_world(world: &mut ECSWorld) -> (Option<SceneEntities>, Option<PathBuf>, Option<SceneSettings>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let candidate = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .or_else(|| {
                let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/demo_gltf.scene");
                p.is_file().then_some(p)
            });

        if let Some(p) = candidate {
            let root = p.parent().map(PathBuf::from);
            let path_str = p.to_string_lossy();
            let mut registry = ComponentRegistry::new();
            registry.register_builtins();
            match load_scene_file(path_str.as_ref()) {
                Ok(data) => {
                    let settings = data.settings.clone();
                    match load_scene_into_world(world, &data, true, &registry) {
                        Ok(_) => {
                            log::info!("Loaded scene from {}", path_str);
                            return (None, root, Some(settings));
                        }
                        Err(e) => {
                            log::warn!("Scene instantiate failed ({e}) — using built-in primitive demo");
                            world.clear();
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Scene load failed ({e}) — using built-in primitive demo");
                }
            }
        }
    }

    (Some(setup_scene(world)), None, None)
}

// ── Scene setup ────────────────────────────────────────────────────────────────

struct SceneEntities {
    cube:   EntityId,
    sphere: EntityId,
    ground: EntityId,
}

fn setup_scene(world: &mut ECSWorld) -> SceneEntities {
    let camera_entity = world.spawn(Some("MainCamera"));
    {
        let mut t  = Transform::new();
        t.position = Vec3::new(0.0, 1.5, 5.0);
        t.rotation = Quat::from_rotation_x(-15_f32.to_radians());
        t.dirty    = true;
        world.add_component(camera_entity, t);
        world.add_component(camera_entity, Camera::new());
    }

    let sun = world.spawn(Some("SunLight"));
    {
        let mut t  = Transform::new();
        t.rotation = Quat::from_euler(glam::EulerRot::XYZ, -45_f32.to_radians(), 30_f32.to_radians(), 0.0);
        t.dirty    = true;
        world.add_component(sun, t);
        world.add_component(sun, Light {
            light_type: LightType::Directional,
            color:      [1.0, 0.97, 0.88],
            intensity:  3.0,
            cast_shadow: true,
            ..Light::default()
        });
    }

    let cube = world.spawn(Some("Cube"));
    {
        let mut t  = Transform::new();
        t.position = Vec3::new(0.0, 0.5, 0.0);
        t.scale    = Vec3::splat(1.0);
        t.dirty    = true;
        world.add_component(cube, t);
        world.add_component(cube, MeshRenderer::from_primitive(PrimitiveType::Cube));
    }

    let sphere = world.spawn(Some("Sphere"));
    {
        let mut t  = Transform::new();
        t.position = Vec3::new(2.5, 0.5, 0.0);
        t.dirty    = true;
        world.add_component(sphere, t);
        world.add_component(sphere, MeshRenderer::from_primitive(PrimitiveType::Sphere));
    }

    let ground = world.spawn(Some("Ground"));
    {
        let mut t  = Transform::new();
        t.position = Vec3::new(0.0, 0.0, 0.0);
        t.scale    = Vec3::new(20.0, 1.0, 20.0);
        t.dirty    = true;
        world.add_component(ground, t);
        world.add_component(ground, MeshRenderer::from_primitive(PrimitiveType::Plane));
    }

    let sparks = world.spawn(Some("ParticleEmitter"));
    {
        let mut t = Transform::new();
        t.position = Vec3::new(0.0, 0.2, 0.0);
        t.dirty = true;
        world.add_component(sparks, t);
        let mut pe = ParticleEmitter::default();
        pe.spawn_per_second = 48.0;
        pe.max_particles = 512;
        pe.start_speed = 1.8;
        pe.lifetime = 1.4;
        pe.color = [0.9, 0.85, 0.3, 0.85];
        pe.size = 0.06;
        world.add_component(sparks, pe);
    }

    TransformSystem::update(world);

    log::info!("Scene created: {} entities", world.entity_count());
    SceneEntities { cube, sphere, ground }
}

fn setup_materials(
    renderer: &mut FluxionRenderer,
    world:    &mut ECSWorld,
    scene:    &SceneEntities,
) {
    let cube_mat = renderer.add_material(&MaterialAsset {
        name:      "Cube_Mat".to_string(),
        color:     [0.8, 0.3, 0.1, 1.0],
        roughness: 0.4,
        metalness: 0.0,
        ..MaterialAsset::default()
    }).expect("cube material");

    let sphere_mat = renderer.add_material(&MaterialAsset {
        name:      "Sphere_Mat".to_string(),
        color:     [0.15, 0.15, 0.2, 1.0],
        roughness: 0.1,
        metalness: 0.9,
        ..MaterialAsset::default()
    }).expect("sphere material");

    let ground_mat = renderer.add_material(&MaterialAsset {
        name:      "Ground_Mat".to_string(),
        color:     [0.5, 0.5, 0.48, 1.0],
        roughness: 0.85,
        metalness: 0.0,
        ..MaterialAsset::default()
    }).expect("ground material");

    renderer.set_entity_material(world, scene.cube,   cube_mat);
    renderer.set_entity_material(world, scene.sphere, sphere_mat);
    renderer.set_entity_material(world, scene.ground, ground_mat);
}

// ── winit ApplicationHandler ──────────────────────────────────────────────────

impl ApplicationHandler for SandboxApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if matches!(self, SandboxApp::Running(_)) { return; }

        let window = Arc::new(
            event_loop
                .create_window(
                    winit::window::WindowAttributes::default()
                        .with_title("FluxionRS Sandbox")
                        .with_inner_size(winit::dpi::PhysicalSize::new(1280u32, 720)),
                )
                .expect("Window creation failed"),
        );

        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowExtWebSys;
            web_sys::window()
                .and_then(|w| w.document())
                .and_then(|doc| {
                    let canvas = window.canvas()?;
                    doc.body()?.append_child(&canvas).ok()?;
                    Some(())
                })
                .expect("Failed to attach canvas to document body");
        }

        *self = SandboxApp::Running(create_inner(window));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let SandboxApp::Running(rc) = self else { return };
        let mut state = rc.borrow_mut();

        #[cfg(not(target_arch = "wasm32"))]
        let egui_consumed = {
            let win = state.window.clone();
            state
                .editor
                .as_mut()
                .map(|e| e.on_window_event(win.as_ref(), &event).consumed)
                .unwrap_or(false)
        };
        #[cfg(target_arch = "wasm32")]
        let egui_consumed = false;

        match event {
            WindowEvent::CloseRequested => {
                log::info!("Window close requested — exiting");
                event_loop.exit();
            }

            WindowEvent::KeyboardInput { event: key_ev, .. } => {
                if key_ev.state == ElementState::Pressed {
                    if let PhysicalKey::Code(KeyCode::Escape) = key_ev.physical_key {
                        event_loop.exit();
                        return;
                    }

                    // Ctrl+S — save the current world to sandbox_save.scene
                    #[cfg(not(target_arch = "wasm32"))]
                    if let PhysicalKey::Code(KeyCode::KeyS) = key_ev.physical_key {
                        let mut reg = ComponentRegistry::new();
                        reg.register_builtins();
                        let scene = fluxion_core::world_to_scene_data(
                            &state.world,
                            &reg,
                            "sandbox_save".to_string(),
                            fluxion_core::scene::SceneSettings::default(),
                            None,
                        );
                        match fluxion_core::scene::save_scene_file("sandbox_save.scene", &scene) {
                            Ok(()) => log::info!("Scene saved to sandbox_save.scene"),
                            Err(e) => log::error!("Scene save failed: {e}"),
                        }
                    }
                }
                if egui_consumed {
                    return;
                }
                if let PhysicalKey::Code(code) = key_ev.physical_key {
                    let name = format!("{:?}", code);
                    state.input.set_key_down(&name, key_ev.state == ElementState::Pressed);
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                if egui_consumed {
                    return;
                }
                state.input.set_mouse_position(position.x as f32, position.y as f32);
            }

            WindowEvent::MouseInput { state: btn_state, button, .. } => {
                if egui_consumed {
                    return;
                }
                let pressed = btn_state == ElementState::Pressed;
                let (mut l, mut m, mut r) = (
                    state.input.mouse_left(),
                    state.input.mouse_middle(),
                    state.input.mouse_right(),
                );
                match button {
                    MouseButton::Left   => l = pressed,
                    MouseButton::Middle => m = pressed,
                    MouseButton::Right  => r = pressed,
                    _ => {}
                }
                state.input.set_mouse_button(l, m, r);
            }

            WindowEvent::MouseWheel { delta, .. } => {
                if egui_consumed {
                    return;
                }
                match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        state.input.add_scroll(x * 32.0, y * 32.0);
                    }
                    MouseScrollDelta::PixelDelta(p) => {
                        state.input.add_scroll(p.x as f32, p.y as f32);
                    }
                }
            }

            WindowEvent::Resized(new_size) => {
                if let Some(r) = state.renderer.as_mut() {
                    r.resize(new_size.width, new_size.height);
                }
                log::info!("Resized to {}×{}", new_size.width, new_size.height);
            }

            WindowEvent::RedrawRequested => {
                drop(state);
                if let SandboxApp::Running(rc) = self {
                    rc.borrow_mut().tick();
                }
                if let SandboxApp::Running(rc) = self {
                    rc.borrow().window.request_redraw();
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let SandboxApp::Running(rc) = self {
            rc.borrow().window.request_redraw();
        }
    }
}
