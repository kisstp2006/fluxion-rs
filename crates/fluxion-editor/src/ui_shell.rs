// ============================================================
// ui_shell.rs — egui + wgpu integration layer
//
// Manages the egui context, winit event handling, and wgpu
// rendering for the editor window.  One instance per window.
// ============================================================

use egui_wgpu::wgpu;
use egui_wgpu::ScreenDescriptor;
use winit::window::Window;

pub struct UiShell {
    state:              egui_winit::State,
    renderer:           egui_wgpu::Renderer,
    viewport_texture:   Option<egui::TextureId>,
}

impl UiShell {
    pub fn new(
        window:         &Window,
        device:         &wgpu::Device,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let ctx     = egui::Context::default();
        // Install image loaders (SVG, PNG, JPEG …) for icon rendering.
        crate::icons::install_loaders(&ctx);
        let max_tex = device.limits().max_texture_dimension_2d as usize;
        let state   = egui_winit::State::new(
            ctx,
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            Some(max_tex),
        );
        let renderer = egui_wgpu::Renderer::new(device, surface_format, egui_wgpu::RendererOptions::default());
        Self { state, renderer, viewport_texture: None }
    }

    #[allow(dead_code)]
    pub fn context(&self) -> &egui::Context {
        self.state.egui_ctx()
    }

    /// Register (first call) or update (subsequent calls) the offscreen viewport texture.
    /// Reuses the same `egui::TextureId` every frame to avoid leaking texture slots.
    pub fn register_viewport_texture(
        &mut self,
        device: &wgpu::Device,
        view:   &wgpu::TextureView,
        _width:  u32,
        _height: u32,
    ) -> egui::TextureId {
        match self.viewport_texture {
            None => {
                let id = self.renderer.register_native_texture(
                    device,
                    view,
                    wgpu::FilterMode::Linear,
                );
                self.viewport_texture = Some(id);
                id
            }
            Some(id) => {
                self.renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    view,
                    wgpu::FilterMode::Linear,
                    id,
                );
                id
            }
        }
    }

}

impl Drop for UiShell {
    fn drop(&mut self) {
        if let Some(id) = self.viewport_texture.take() {
            self.renderer.free_texture(&id);
        }
    }
}

impl UiShell {
    pub fn on_window_event(
        &mut self,
        window: &Window,
        event:  &winit::event::WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    /// Run `ui_fn` inside an egui frame, record render commands into `encoder`
    /// (or a secondary encoder), and return any extra command buffers produced
    /// by egui-wgpu callbacks.  The return value should be passed directly to
    /// `render_with`'s `after` closure return.
    pub fn paint(
        &mut self,
        window:       &Window,
        device:       &wgpu::Device,
        queue:        &wgpu::Queue,
        encoder:      &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        width:        u32,
        height:       u32,
        ui_fn:        impl FnMut(&mut egui::Ui),
    ) -> Vec<wgpu::CommandBuffer> {
        let raw_input = self.state.take_egui_input(window);
        let output    = self.state.egui_ctx().run_ui(raw_input, ui_fn);
        self.state.handle_platform_output(window, output.platform_output);

        for (id, delta) in &output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }

        let paint_jobs = self.state.egui_ctx()
            .tessellate(output.shapes, output.pixels_per_point);

        let screen = ScreenDescriptor {
            size_in_pixels:  [width.max(1), height.max(1)],
            pixels_per_point: window.scale_factor() as f32,
        };

        let extras = self.renderer.update_buffers(
            device, queue, encoder, &paint_jobs, &screen,
        );

        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui_editor"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           surface_view,
                    resolve_target: None,
                    depth_slice:    None,
                    ops:            wgpu::Operations {
                        load:  wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes:         None,
                occlusion_query_set:      None,
                multiview_mask:           None,
            });
            self.renderer.render(&mut rpass.forget_lifetime(), &paint_jobs, &screen);
        }

        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }

        extras
    }
}
