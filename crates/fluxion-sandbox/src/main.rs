// ============================================================
// fluxion-sandbox
//
// Test harness for the FluxionRS engine. Opens a window, creates
// a demo scene, and runs the engine loop.
//
// Demo scene:
//   - One spinning PBR cube (lit by the directional light)
//   - One static sphere
//   - One directional sun light
//   - One perspective camera
//
// JS scripting demo:
//   - A simple Spinner script is loaded from assets/scripts/spinner.js
//   - It rotates the cube entity each frame using FluxionBehaviour.update()
//
// Controls:
//   Esc — exit
//   F   — toggle fullscreen (not implemented yet)
//
// ── Platform notes ─────────────────────────────────────────────────────────────
// Native: winit creates an OS window, wgpu uses Vulkan/Metal/DX12.
// WASM:   winit targets a <canvas id="fluxion-canvas">, wgpu uses WebGPU/WebGL2.
//         Build with: cargo build -p fluxion-sandbox --target wasm32-unknown-unknown
// ============================================================

use std::sync::Arc;

use glam::{Quat, Vec3};
use winit::{
    application::ApplicationHandler,
    event::{WindowEvent, KeyEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use fluxion_core::{
    ECSWorld, Time,
    components::{Camera, Light, MeshRenderer},
    components::light::LightType,
    components::mesh_renderer::PrimitiveType,
    transform::Transform,
    transform::system::TransformSystem,
};
use fluxion_renderer::FluxionRenderer;
use fluxion_scripting::{JsVm, bindings};

// ── Entry point ────────────────────────────────────────────────────────────────

fn main() {
    // Set up logging
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        console_log::init_with_level(log::Level::Info).expect("console_log init failed");
    }

    log::info!("FluxionRS Sandbox — starting");

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll); // render as fast as possible

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
    Running(RunningState),
}

struct RunningState {
    window:   Arc<Window>,
    world:    ECSWorld,
    time:     Time,
    renderer: FluxionRenderer,
    scripts:  JsVm,
}

impl RunningState {
    fn new(window: Arc<Window>) -> Self {
        // ── Engine init ───────────────────────────────────────────────────────
        let mut world  = ECSWorld::new();
        let     time   = Time::new();

        // ── Create demo scene ─────────────────────────────────────────────────
        setup_scene(&mut world);

        // ── Renderer init (async, blocked by pollster on native) ──────────────
        #[cfg(not(target_arch = "wasm32"))]
        let renderer = pollster::block_on(FluxionRenderer::new(window.clone()))
            .expect("Renderer init failed");

        // ── JS scripting VM ───────────────────────────────────────────────────
        let scripts = JsVm::new().expect("JS VM init failed");
        bindings::setup_bindings(&scripts).expect("JS binding setup failed");

        // Load the demo spinner script
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

        RunningState { window, world, time, renderer, scripts }
    }

    fn tick(&mut self) {
        let (fixed_steps, dt) = self.time.tick();

        // ── Fixed update (physics-rate) ───────────────────────────────────────
        for _ in 0..fixed_steps {
            if let Err(e) = self.scripts.fixed_update(self.time.fixed_dt) {
                log::warn!("Script fixed_update error: {e}");
            }
            TransformSystem::update(&mut self.world);
        }

        // ── Variable update ───────────────────────────────────────────────────
        // Update JS Time global with current values
        if let Err(e) = bindings::update_time_global(
            &self.scripts, dt, self.time.elapsed, self.time.fixed_dt, self.time.frame_count,
        ) {
            log::warn!("Time global update failed: {e}");
        }

        if let Err(e) = self.scripts.update(dt) {
            log::warn!("Script update error: {e}");
        }

        // Run transform propagation before rendering
        TransformSystem::update(&mut self.world);

        // ── Render ────────────────────────────────────────────────────────────
        match self.renderer.render(&self.world, &self.time) {
            Ok(())                                    => {}
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                let size = self.window.inner_size();
                self.renderer.resize(size.width, size.height);
            }
            Err(e) => log::error!("Render error: {e}"),
        }
    }
}

// ── Scene setup ────────────────────────────────────────────────────────────────

fn setup_scene(world: &mut ECSWorld) {
    // ── Camera ────────────────────────────────────────────────────────────────
    let camera_entity = world.spawn(Some("MainCamera"));
    {
        let mut t  = Transform::new();
        t.position = Vec3::new(0.0, 1.5, 5.0);
        // Look toward the origin
        t.rotation = Quat::from_rotation_x(-15_f32.to_radians());
        t.dirty    = true;
        world.add_component(camera_entity, t);
        world.add_component(camera_entity, Camera::new());
    }

    // ── Sun light ─────────────────────────────────────────────────────────────
    let sun = world.spawn(Some("SunLight"));
    {
        let mut t  = Transform::new();
        // Rotate so forward direction (-Z) points down-and-forward
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

    // ── Cube (the main test object) ───────────────────────────────────────────
    let cube = world.spawn(Some("Cube"));
    {
        let mut t  = Transform::new();
        t.position = Vec3::new(0.0, 0.5, 0.0);
        t.scale    = Vec3::splat(1.0);
        t.dirty    = true;
        world.add_component(cube, t);
        world.add_component(cube, MeshRenderer::from_primitive(PrimitiveType::Cube));
        // The JS spinner script will look this up by name and rotate it
    }

    // ── Sphere ────────────────────────────────────────────────────────────────
    let sphere = world.spawn(Some("Sphere"));
    {
        let mut t  = Transform::new();
        t.position = Vec3::new(2.5, 0.5, 0.0);
        t.dirty    = true;
        world.add_component(sphere, t);
        world.add_component(sphere, MeshRenderer::from_primitive(PrimitiveType::Sphere));
    }

    // ── Ground plane ──────────────────────────────────────────────────────────
    let ground = world.spawn(Some("Ground"));
    {
        let mut t  = Transform::new();
        t.position = Vec3::new(0.0, 0.0, 0.0);
        t.scale    = Vec3::new(20.0, 1.0, 20.0);
        t.dirty    = true;
        world.add_component(ground, t);
        world.add_component(ground, MeshRenderer::from_primitive(PrimitiveType::Plane));
    }

    // Run transform propagation once so world matrices are valid from frame 0
    TransformSystem::update(world);

    log::info!("Scene created: {} entities", world.entity_count());
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

        // On WASM, point winit at the canvas element
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

        *self = SandboxApp::Running(RunningState::new(window));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let state = match self { SandboxApp::Running(s) => s, _ => return };

        match event {
            WindowEvent::CloseRequested => {
                log::info!("Window close requested — exiting");
                event_loop.exit();
            }

            WindowEvent::KeyboardInput {
                event: KeyEvent { physical_key: PhysicalKey::Code(KeyCode::Escape), state: winit::event::ElementState::Pressed, .. },
                ..
            } => {
                event_loop.exit();
            }

            WindowEvent::Resized(new_size) => {
                state.renderer.resize(new_size.width, new_size.height);
                log::info!("Resized to {}×{}", new_size.width, new_size.height);
            }

            WindowEvent::RedrawRequested => {
                state.tick();
                state.window.request_redraw(); // keep rendering
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Request a redraw on every iteration so we render continuously
        if let SandboxApp::Running(state) = self {
            state.window.request_redraw();
        }
    }
}
