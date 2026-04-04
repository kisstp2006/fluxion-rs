//! egui + wgpu + winit integration (native sandbox only).

use std::sync::Arc;

use egui_wgpu::wgpu;
use egui_wgpu::ScreenDescriptor;
use winit::window::Window;

pub struct EguiShell {
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

impl EguiShell {
    pub fn new(window: &Window, device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let ctx = egui::Context::default();
        let max_tex = device.limits().max_texture_dimension_2d as usize;
        let state = egui_winit::State::new(
            ctx,
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            Some(max_tex),
        );
        let renderer = egui_wgpu::Renderer::new(device, surface_format, None, 1, false);
        Self { state, renderer }
    }

    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &winit::event::WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    pub fn paint(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        width: u32,
        height: u32,
        ui_fn: impl FnMut(&egui::Context),
    ) -> Vec<wgpu::CommandBuffer> {
        let raw_input = self.state.take_egui_input(window);
        let output = self.state.egui_ctx().run(raw_input, ui_fn);
        self.state.handle_platform_output(window, output.platform_output);

        for (id, image_delta) in &output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, image_delta);
        }

        let paint_jobs = self
            .state
            .egui_ctx()
            .tessellate(output.shapes, output.pixels_per_point);

        let screen = ScreenDescriptor {
            size_in_pixels: [width.max(1), height.max(1)],
            pixels_per_point: window.scale_factor() as f32,
        };

        let extras = self
            .renderer
            .update_buffers(device, queue, encoder, &paint_jobs, &screen);

        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui_overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.renderer
                .render(&mut rpass.forget_lifetime(), &paint_jobs, &screen);
        }

        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }

        extras
    }
}

pub fn paint_sandbox_panel(
    shell: &mut EguiShell,
    window: &Arc<Window>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    surface_view: &wgpu::TextureView,
    width: u32,
    height: u32,
    ui_debug_lines: &[String],
    ents: usize,
    dt: f32,
    smooth_fps: f32,
    elapsed: f32,
    frame: u64,
    scene_loaded: bool,
    gamepad: bool,
    ball_y: Option<f32>,
) -> Vec<wgpu::CommandBuffer> {
    let lines: Vec<String> = ui_debug_lines.to_vec();
    shell.paint(
        window.as_ref(),
        device,
        queue,
        encoder,
        surface_view,
        width,
        height,
        |ctx| {
            egui::Window::new("Fluxion Sandbox").default_pos([12.0, 12.0]).show(ctx, |ui| {
                ui.heading("Runtime");
                ui.label(format!(
                    "dt: {:.2} ms | FPS: {:.0} | frame: {}",
                    dt * 1000.0,
                    smooth_fps,
                    frame
                ));
                ui.label(format!("elapsed: {:.2}s | entities: {}", elapsed, ents));
                ui.label(format!(
                    "scene settings: {} | gamepad: {}",
                    if scene_loaded { "yes" } else { "demo" },
                    if gamepad { "yes" } else { "no" }
                ));
                if let Some(y) = ball_y {
                    ui.label(format!("physics test ball Y: {:.2}", y));
                }
                ui.separator();
                ui.label("Engine.ui lines (from scripts)");
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    if lines.is_empty() {
                        ui.label("(empty — use Engine.ui.pushLine(\"text\") in JS)");
                    } else {
                        for line in &lines {
                            ui.label(line);
                        }
                    }
                });
            });
        },
    )
}
