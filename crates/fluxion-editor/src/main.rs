// ============================================================
// fluxion-editor — standalone editor binary
//
// Hosts the full engine runtime (ECS + renderer + physics) and
// presents a hot-reloadable Rune-scripted UI with egui_dock
// docking support.
// ============================================================

mod dock;
mod host;
mod rune_bindings;
mod ui_shell;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use wgpu::SurfaceError;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use fluxion_renderer::{FluxionRenderer, RendererConfig};

use crate::dock::{default_dock_state, show_dock, EditorTab};
use crate::host::EditorHost;
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
    Running(Rc<RefCell<EditorInner>>),
}

/// All per-window state.  Fields are separate so we can split borrows when
/// calling renderer.render_with (needs &mut renderer, &world, &time) while
/// also borrowing vm and dock_state for the egui closure.
struct EditorInner {
    window:     Arc<Window>,
    host:       EditorHost,
    renderer:   FluxionRenderer,
    ui_shell:   UiShell,
    dock_state: egui_dock::DockState<EditorTab>,
}

// ── ApplicationHandler impl ───────────────────────────────────────────────────

impl ApplicationHandler for EditorApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if matches!(self, EditorApp::Running(_)) {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("FluxionRS Editor")
            .with_inner_size(winit::dpi::LogicalSize::new(1600u32, 900u32));

        let window = Arc::new(
            event_loop.create_window(attrs).expect("Window creation failed"),
        );

        let scripts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
        let host = EditorHost::new(scripts_dir).expect("EditorHost init failed");

        let inner = pollster::block_on(EditorInner::new(window, host))
            .expect("EditorInner init failed");

        *self = EditorApp::Running(Rc::new(RefCell::new(inner)));
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event:      WindowEvent,
    ) {
        let EditorApp::Running(inner) = self else { return };
        let mut g = inner.borrow_mut();

        // Forward to egui first (clone Arc<Window> to avoid split-borrow through RefMut).
        let window = g.window.clone();
        let egui_resp = g.ui_shell.on_window_event(&window, &event);
        if egui_resp.consumed { return; }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    && event.state == ElementState::Pressed
                {
                    event_loop.exit();
                }
                let pressed = event.state == ElementState::Pressed;
                if let PhysicalKey::Code(code) = event.physical_key {
                    g.host.input.set_key_down(&format!("{code:?}"), pressed);
                }
            }
            WindowEvent::Resized(size) => {
                g.renderer.resize(size.width, size.height);
            }
            WindowEvent::RedrawRequested => {
                g.frame();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let EditorApp::Running(inner) = self {
            inner.borrow().window.request_redraw();
        }
    }
}

// ── EditorInner ───────────────────────────────────────────────────────────────

impl EditorInner {
    async fn new(window: Arc<Window>, host: EditorHost) -> anyhow::Result<Self> {
        let renderer = FluxionRenderer::new(window.clone(), RendererConfig::default()).await?;
        let fmt      = renderer.surface_format();
        let shell    = UiShell::new(&window, &renderer.device, fmt);

        // Call on_editor_init now that everything is ready.
        if let Err(e) = host.vm.on_editor_init() {
            log::warn!("on_editor_init: {e}");
        }

        Ok(Self {
            window,
            host,
            renderer,
            ui_shell:   shell,
            dock_state: default_dock_state(),
        })
    }

    fn frame(&mut self) {
        // Engine tick — physics, transforms, hot reload, flush pending edits.
        self.host.tick();
        // Set thread-locals so Rune panels can read ECS data this frame.
        self.host.push_world_context();

        let w      = self.renderer.width;
        let h      = self.renderer.height;
        let window = self.window.clone();

        // Split borrows: renderer needs &mut self.renderer + &self.host.world/time.
        // ui_shell, dock_state, and vm come from separate fields — Rust allows this.
        let ui_shell   = &mut self.ui_shell;
        let dock_state = &mut self.dock_state;
        let vm         = &mut self.host.vm;

        let result = self.renderer.render_with(
            &self.host.world,
            &self.host.time,
            |device, queue, encoder, view| {
                ui_shell.paint(
                    &window, device, queue, encoder, view, w, h,
                    |ctx| {
                        egui::TopBottomPanel::top("editor_menu").show(ctx, |ui| {
                            egui::menu::bar(ui, |ui| {
                                ui.menu_button("File", |ui| {
                                    if ui.button("Exit").clicked() {
                                        std::process::exit(0);
                                    }
                                });
                                ui.menu_button("Edit", |_ui| {});
                                ui.menu_button("View", |_ui| {});
                            });
                        });
                        show_dock(ctx, dock_state, vm);
                    },
                )
            },
        );

        // Clear world context after the full frame (panels are done rendering).
        self.host.pop_world_context();

        match result {
            Ok(()) => {}
            Err(SurfaceError::Lost | SurfaceError::Outdated) => {
                let size = self.window.inner_size();
                self.renderer.resize(size.width, size.height);
            }
            Err(e) => log::error!("Render error: {e}"),
        }
    }
}
