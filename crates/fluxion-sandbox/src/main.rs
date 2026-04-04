// ============================================================
// fluxion-sandbox
//
// Test harness for the FluxionRS engine. Opens a window, creates
// a demo scene (or loads a `.scene` JSON path from argv on native),
// and runs the engine loop.
//
// JS scripting demo (native): `assets/scripts/spinner.js`
//
// Controls:
//   Esc — exit
//
// Native: optional first CLI argument — path to a FluxionJS-compatible `.scene` file.
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
    components::{Camera, Light, MeshRenderer},
    components::light::LightType,
    components::mesh_renderer::PrimitiveType,
    scene::{load_scene_file, load_scene_into_world},
    transform::Transform,
    transform::system::TransformSystem,
};
use fluxion_renderer::{FluxionRenderer, MaterialAsset};
use fluxion_scripting::{JsVm, bindings};

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
}

impl SandboxInner {
    fn tick(&mut self) {
        let (fixed_steps, dt) = self.time.tick();

        for _ in 0..fixed_steps {
            if let Err(e) = self.scripts.fixed_update(self.time.fixed_dt) {
                log::warn!("Script fixed_update error: {e}");
            }
            TransformSystem::update(&mut self.world);
        }

        if let Err(e) = bindings::update_time_global(
            &self.scripts, dt, self.time.elapsed, self.time.fixed_dt, self.time.frame_count,
        ) {
            log::warn!("Time global update failed: {e}");
        }

        if let Err(e) = bindings::update_input_global(&self.scripts, &self.input) {
            log::warn!("Input global update failed: {e}");
        }

        if let Err(e) = self.scripts.update(dt) {
            log::warn!("Script update error: {e}");
        }

        TransformSystem::update(&mut self.world);

        self.input.begin_frame();

        let Some(ref mut renderer) = self.renderer else {
            return;
        };

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
    let (pending_demo, asset_root) = bootstrap_world(&mut world);

    let scripts = JsVm::new().expect("JS VM init failed");
    bindings::setup_bindings(&scripts).expect("JS binding setup failed");

    #[cfg(not(target_arch = "wasm32"))]
    {
        let script_path = "assets/scripts/spinner.js";
        if std::path::Path::new(script_path).exists() {
            if let Err(e) = scripts.load_script(script_path) {
                log::warn!("Failed to load spinner script: {e}");
            }
        } else {
            log::info!("No spinner.js found — running without JS demo script");
        }
    }

    let inner = Rc::new(RefCell::new(SandboxInner {
        window: window.clone(),
        world,
        time: Time::new(),
        scripts,
        input: InputState::new(),
        renderer: None,
        pending_demo,
        asset_root,
    }));

    #[cfg(not(target_arch = "wasm32"))]
    {
        let r = pollster::block_on(FluxionRenderer::new(window.clone())).expect("Renderer init failed");
        inner.borrow_mut().renderer = Some(r);
        finish_renderer_setup(&mut inner.borrow_mut());
    }

    #[cfg(target_arch = "wasm32")]
    {
        let win = window.clone();
        let weak = Rc::downgrade(&inner);
        wasm_bindgen_futures::spawn_local(async move {
            match FluxionRenderer::new(win).await {
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

    if let Some(ref demo) = g.pending_demo {
        setup_materials(renderer, &mut g.world, demo);
        g.pending_demo = None;
    } else if let Err(e) = renderer.hydrate_scene_materials(&mut g.world, g.asset_root.as_deref()) {
        log::warn!("hydrate_scene_materials: {e}");
    }
}

fn bootstrap_world(world: &mut ECSWorld) -> (Option<SceneEntities>, Option<PathBuf>) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = std::env::args().nth(1) {
        let p = PathBuf::from(&path);
        let root = p.parent().map(PathBuf::from);
        let path_str = p.to_string_lossy();
        match load_scene_file(path_str.as_ref()).and_then(|data| {
            load_scene_into_world(world, &data, true).map_err(|e| e)
        }) {
            Ok(_) => {
                log::info!("Loaded scene from {}", path_str);
                return (None, root);
            }
            Err(e) => {
                log::warn!("Scene load failed ({e}) — using demo scene");
                world.clear();
            }
        }
    }

    (Some(setup_scene(world)), None)
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
                }
                if let PhysicalKey::Code(code) = key_ev.physical_key {
                    let name = format!("{:?}", code);
                    state.input.set_key_down(&name, key_ev.state == ElementState::Pressed);
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                state.input.set_mouse_position(position.x as f32, position.y as f32);
            }

            WindowEvent::MouseInput { state: btn_state, button, .. } => {
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
