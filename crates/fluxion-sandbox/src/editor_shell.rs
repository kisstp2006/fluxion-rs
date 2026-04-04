// ============================================================
// editor_shell.rs — full reflection-driven editor UI
//
// Layout:
//   ┌──────────────┬─────────────────┬─────────────────┐
//   │  Hierarchy   │  (viewport)     │   Inspector     │
//   │  entity list │                 │   component     │
//   │              │                 │   fields        │
//   ├──────────────┴─────────────────┴─────────────────┤
//   │  Console / debug lines                           │
//   └──────────────────────────────────────────────────┘
//
// All inspector content is driven by `Reflect` metadata — no
// component type names are hard-coded here.
// ============================================================

use std::sync::Arc;

use egui_wgpu::wgpu;
use egui_wgpu::ScreenDescriptor;
use winit::window::Window;

use fluxion_core::{
    ECSWorld, EntityId, ComponentRegistry,
    reflect::{ReflectFieldType, ReflectValue},
};

// ── egui / wgpu plumbing ──────────────────────────────────────────────────────

pub struct EditorShell {
    state:    egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

impl EditorShell {
    pub fn new(window: &Window, device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let ctx      = egui::Context::default();
        let max_tex  = device.limits().max_texture_dimension_2d as usize;
        let state    = egui_winit::State::new(
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
        event:  &winit::event::WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    pub fn paint(
        &mut self,
        window:       &Window,
        device:       &wgpu::Device,
        queue:        &wgpu::Queue,
        encoder:      &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        width:        u32,
        height:       u32,
        ui_fn:        impl FnMut(&egui::Context),
    ) -> Vec<wgpu::CommandBuffer> {
        let raw_input = self.state.take_egui_input(window);
        let output    = self.state.egui_ctx().run(raw_input, ui_fn);
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

        let extras = self.renderer.update_buffers(device, queue, encoder, &paint_jobs, &screen);

        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui_editor"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           surface_view,
                    resolve_target: None,
                    ops:            wgpu::Operations {
                        load:  wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes:         None,
                occlusion_query_set:      None,
            });
            self.renderer.render(&mut rpass.forget_lifetime(), &paint_jobs, &screen);
        }

        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }

        extras
    }
}

// ── Editor state ──────────────────────────────────────────────────────────────

/// Persistent editor state shared across frames.
pub struct EditorState {
    pub selected_entity:    Option<EntityId>,
    /// Pending field edit: (component_type, field_name, new_value)
    pending_edits:          Vec<(String, String, ReflectValue)>,
    /// Collapsed state for component headers in inspector.
    collapsed_components:   std::collections::HashSet<String>,
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            selected_entity:         None,
            pending_edits:           Vec::new(),
            collapsed_components:    std::collections::HashSet::new(),
            }
    }

    /// Apply all pending field edits to the world. Call AFTER the egui frame.
    pub fn flush_edits(&mut self, registry: &ComponentRegistry, world: &ECSWorld) {
        let entity = match self.selected_entity {
            Some(e) => e,
            None => { self.pending_edits.clear(); return; }
        };
        for (comp_type, field, value) in self.pending_edits.drain(..) {
            if let Err(e) = registry.set_reflect_field(&comp_type, world, entity, &field, value) {
                log::warn!("Inspector edit failed: {e}");
            }
        }
    }
}

// ── Main paint function ───────────────────────────────────────────────────────

pub fn paint_editor(
    shell:         &mut EditorShell,
    editor:        &mut EditorState,
    window:        &Arc<Window>,
    device:        &wgpu::Device,
    queue:         &wgpu::Queue,
    encoder:       &mut wgpu::CommandEncoder,
    surface_view:  &wgpu::TextureView,
    width:         u32,
    height:        u32,
    world:         &ECSWorld,
    registry:      &ComponentRegistry,
    ui_debug_lines: &[String],
    dt:            f32,
    smooth_fps:    f32,
    elapsed:       f32,
    frame:         u64,
) -> Vec<wgpu::CommandBuffer> {
    let result = shell.paint(
        window.as_ref(),
        device,
        queue,
        encoder,
        surface_view,
        width,
        height,
        |ctx| {
            draw_editor_ui(ctx, editor, world, registry, ui_debug_lines, dt, smooth_fps, elapsed, frame);
        },
    );

    // Apply field edits after the frame so we don't borrow world mutably inside egui closure.
    editor.flush_edits(registry, world);

    result
}

// ── Editor layout ─────────────────────────────────────────────────────────────

fn draw_editor_ui(
    ctx:            &egui::Context,
    editor:         &mut EditorState,
    world:          &ECSWorld,
    registry:       &ComponentRegistry,
    ui_debug_lines: &[String],
    dt:             f32,
    smooth_fps:     f32,
    elapsed:        f32,
    frame:          u64,
) {
    // ── Top menu bar ──────────────────────────────────────────────────────────
    egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("⚡ FluxionRS");
            ui.separator();
            ui.label(format!(
                "{:.0} fps  |  {:.2} ms  |  frame {}  |  {:.1}s  |  {} entities",
                smooth_fps, dt * 1000.0, frame, elapsed,
                world.entity_count()
            ));
        });
    });

    // ── Bottom console ────────────────────────────────────────────────────────
    egui::TopBottomPanel::bottom("console_panel")
        .resizable(true)
        .default_height(100.0)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new("Console").strong());
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                if ui_debug_lines.is_empty() {
                    ui.label(egui::RichText::new("(no output — use Debug.Log() in scripts)")
                        .color(egui::Color32::GRAY));
                } else {
                    for line in ui_debug_lines {
                        ui.label(line);
                    }
                }
            });
        });

    // ── Left hierarchy panel ──────────────────────────────────────────────────
    egui::SidePanel::left("hierarchy_panel")
        .resizable(true)
        .default_width(200.0)
        .show(ctx, |ui| {
            draw_hierarchy(ui, editor, world);
        });

    // ── Right inspector panel ─────────────────────────────────────────────────
    egui::SidePanel::right("inspector_panel")
        .resizable(true)
        .default_width(260.0)
        .show(ctx, |ui| {
            draw_inspector(ui, editor, world, registry);
        });

    // ── Central area (just shows that the viewport lives here) ────────────────
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
            ui.label(
                egui::RichText::new("[ Viewport ]")
                    .color(egui::Color32::from_gray(60))
                    .size(18.0),
            );
        });
    });
}

// ── Hierarchy panel ───────────────────────────────────────────────────────────

fn draw_hierarchy(ui: &mut egui::Ui, editor: &mut EditorState, world: &ECSWorld) {
    ui.label(egui::RichText::new("Hierarchy").strong());
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        let entities: Vec<EntityId> = world.all_entities().collect();
        for entity in entities {
            let name   = world.get_name(entity);
            let is_sel = editor.selected_entity == Some(entity);

            let resp = ui.selectable_label(
                is_sel,
                format!("  {}", name),
            );

            if resp.clicked() {
                if is_sel {
                    editor.selected_entity = None;
                } else {
                    editor.selected_entity = Some(entity);
                }
            }

            if resp.double_clicked() {
                editor.selected_entity = Some(entity);
            }
        }
    });
}

// ── Inspector panel ───────────────────────────────────────────────────────────

fn draw_inspector(
    ui:       &mut egui::Ui,
    editor:   &mut EditorState,
    world:    &ECSWorld,
    registry: &ComponentRegistry,
) {
    let Some(entity) = editor.selected_entity else {
        ui.label(egui::RichText::new("Inspector").strong());
        ui.separator();
        ui.label(egui::RichText::new("Select an entity in the Hierarchy.").color(egui::Color32::GRAY));
        return;
    };

    let entity_name = world.get_name(entity).to_string();
    ui.label(egui::RichText::new("Inspector").strong());
    ui.separator();
    ui.label(egui::RichText::new(format!("🗂  {}", entity_name)).size(14.0).strong());
    ui.add_space(4.0);

    let type_names = registry.reflected_type_names();

    for type_name in type_names {
        let Some(comp) = registry.get_reflect(type_name, world, entity) else {
            continue; // entity doesn't have this component
        };

        let fields = comp.fields();
        if fields.is_empty() { continue; }

        // Collapsing header per component
        let header_id = egui::Id::new(format!("comp_{}", type_name));
        let collapsed = editor.collapsed_components.contains(type_name);

        ui.horizontal(|ui| {
            let arrow = if collapsed { "▸" } else { "▾" };
            if ui.button(arrow).clicked() {
                if collapsed {
                    editor.collapsed_components.remove(type_name);
                } else {
                    editor.collapsed_components.insert(type_name.to_string());
                }
            }
            ui.label(egui::RichText::new(type_name).strong());
        });

        if collapsed {
            ui.add_space(2.0);
            continue;
        }

        egui::Frame::none()
            .inner_margin(egui::Margin { left: 8.0, ..Default::default() })
            .show(ui, |ui| {
                egui::Grid::new(header_id)
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for field in fields {
                            let current_val = comp.get_field(field.name);

                            // Label column
                            ui.label(field.display_name);

                            // Widget column
                            let maybe_edit = draw_field_widget(ui, field, current_val.as_ref());

                            ui.end_row();

                            if let Some(new_val) = maybe_edit {
                                if !field.read_only {
                                    editor.pending_edits.push((
                                        type_name.to_string(),
                                        field.name.to_string(),
                                        new_val,
                                    ));
                                }
                            }
                        }
                    });
            });

        ui.add_space(4.0);
        ui.separator();
    }
}

// ── Field widgets ─────────────────────────────────────────────────────────────

/// Draw the appropriate widget for a field. Returns `Some(new_value)` if edited.
fn draw_field_widget(
    ui:      &mut egui::Ui,
    field:   &fluxion_core::reflect::FieldDescriptor,
    current: Option<&ReflectValue>,
) -> Option<ReflectValue> {
    use ReflectFieldType as FT;
    use ReflectValue    as RV;

    if field.read_only {
        // Read-only: just display the value
        let text = current.map(|v| reflect_value_display(v)).unwrap_or_default();
        ui.label(egui::RichText::new(text).color(egui::Color32::GRAY));
        return None;
    }

    match field.field_type {
        FT::F32 => {
            let mut v = match current { Some(RV::F32(f)) => *f, _ => 0.0 };
            let range = field.range;
            let widget = if range.min.is_some() && range.max.is_some() {
                let min = range.min.unwrap();
                let max = range.max.unwrap();
                egui::Slider::new(&mut v, min..=max)
            } else {
                egui::Slider::new(&mut v, -1_000_000.0_f32..=1_000_000.0_f32)
            };
            if ui.add(widget).changed() {
                return Some(RV::F32(v));
            }
        }

        FT::Bool => {
            let mut v = match current { Some(RV::Bool(b)) => *b, _ => false };
            if ui.checkbox(&mut v, "").changed() {
                return Some(RV::Bool(v));
            }
        }

        FT::U32 => {
            let mut v = match current { Some(RV::U32(n)) => *n as f32, _ => 0.0 };
            let range = field.range;
            let min = range.min.unwrap_or(0.0);
            let max = range.max.unwrap_or(u32::MAX as f32);
            if ui.add(egui::Slider::new(&mut v, min..=max).integer()).changed() {
                return Some(RV::U32(v as u32));
            }
        }

        FT::U8 => {
            let mut v = match current { Some(RV::U8(n)) => *n as f32, _ => 0.0 };
            if ui.add(egui::Slider::new(&mut v, 0.0_f32..=255.0).integer()).changed() {
                return Some(RV::U8(v as u8));
            }
        }

        FT::USize => {
            let mut v = match current { Some(RV::USize(n)) => *n as f32, _ => 0.0 };
            let max = field.range.max.unwrap_or(100_000.0);
            if ui.add(egui::Slider::new(&mut v, 0.0_f32..=max).integer()).changed() {
                return Some(RV::USize(v as usize));
            }
        }

        FT::Vec3 => {
            let [mut x, mut y, mut z] = match current { Some(RV::Vec3(a)) => *a, _ => [0.0; 3] };
            ui.horizontal(|ui| {
                let mut changed = false;
                ui.label("X"); changed |= ui.add(egui::DragValue::new(&mut x).speed(0.01)).changed();
                ui.label("Y"); changed |= ui.add(egui::DragValue::new(&mut y).speed(0.01)).changed();
                ui.label("Z"); changed |= ui.add(egui::DragValue::new(&mut z).speed(0.01)).changed();
                if changed {
                    // Return a value by putting it in the outer scope via a cell
                }
            });
            // Re-check after the horizontal block
            let [ox, oy, oz] = match current { Some(RV::Vec3(a)) => *a, _ => [0.0; 3] };
            if (x - ox).abs() > f32::EPSILON || (y - oy).abs() > f32::EPSILON || (z - oz).abs() > f32::EPSILON {
                return Some(RV::Vec3([x, y, z]));
            }
        }

        FT::Quat => {
            // Display as Euler angles (degrees) for usability
            let quat = match current {
                Some(RV::Quat(q)) => glam::Quat::from_array(*q),
                _ => glam::Quat::IDENTITY,
            };
            let (ax, ay, az) = quat.to_euler(glam::EulerRot::XYZ);
            let mut ex = ax.to_degrees();
            let mut ey = ay.to_degrees();
            let mut ez = az.to_degrees();
            let orig = [ex, ey, ez];
            ui.horizontal(|ui| {
                ui.label("X"); ui.add(egui::DragValue::new(&mut ex).speed(0.5).suffix("°"));
                ui.label("Y"); ui.add(egui::DragValue::new(&mut ey).speed(0.5).suffix("°"));
                ui.label("Z"); ui.add(egui::DragValue::new(&mut ez).speed(0.5).suffix("°"));
            });
            if [ex, ey, ez] != orig {
                let new_q = glam::Quat::from_euler(
                    glam::EulerRot::XYZ,
                    ex.to_radians(), ey.to_radians(), ez.to_radians(),
                ).normalize();
                return Some(RV::Quat(new_q.to_array()));
            }
        }

        FT::Color3 => {
            let [r, g, b] = match current { Some(RV::Color3(c)) => *c, _ => [1.0; 3] };
            let mut col = egui::Color32::from_rgb(
                (r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8,
            );
            if egui::color_picker::color_edit_button_srgba(
                ui, &mut col, egui::color_picker::Alpha::Opaque,
            ).changed() {
                let [nr, ng, nb, _] = col.to_normalized_gamma_f32();
                return Some(RV::Color3([nr, ng, nb]));
            }
        }

        FT::Color4 => {
            let [r, g, b, a] = match current { Some(RV::Color4(c)) => *c, _ => [1.0; 4] };
            let mut col = egui::Color32::from_rgba_unmultiplied(
                (r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, (a * 255.0) as u8,
            );
            if egui::color_picker::color_edit_button_srgba(
                ui, &mut col, egui::color_picker::Alpha::BlendOrAdditive,
            ).changed() {
                let [nr, ng, nb, na] = col.to_normalized_gamma_f32();
                return Some(RV::Color4([nr, ng, nb, na]));
            }
        }

        FT::Str => {
            let mut s = match current { Some(RV::Str(st)) => st.clone(), _ => String::new() };
            if ui.text_edit_singleline(&mut s).changed() {
                return Some(RV::Str(s));
            }
        }

        FT::OptionStr => {
            let mut s = match current {
                Some(RV::OptionStr(Some(st))) => st.clone(),
                _ => String::new(),
            };
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut s);
                if ui.small_button("✕").clicked() { s.clear(); }
            });
            let orig = match current { Some(RV::OptionStr(o)) => o.clone(), _ => None };
            let new_opt = if s.is_empty() { None } else { Some(s.clone()) };
            if new_opt != orig {
                return Some(RV::OptionStr(new_opt));
            }
        }

        FT::Enum => {
            let current_str = match current { Some(RV::Enum(e)) => e.as_str(), _ => "" };
            let mut s = current_str.to_string();
            if ui.text_edit_singleline(&mut s).changed() && !s.is_empty() {
                return Some(RV::Enum(s));
            }
        }
    }

    None
}

fn reflect_value_display(v: &ReflectValue) -> String {
    match v {
        ReflectValue::F32(f)          => format!("{:.3}", f),
        ReflectValue::Vec3([x,y,z])   => format!("({:.2}, {:.2}, {:.2})", x, y, z),
        ReflectValue::Quat(q)         => {
            let qu = glam::Quat::from_array(*q);
            let (x,y,z) = qu.to_euler(glam::EulerRot::XYZ);
            format!("({:.1}°, {:.1}°, {:.1}°)", x.to_degrees(), y.to_degrees(), z.to_degrees())
        }
        ReflectValue::Color3([r,g,b]) => format!("rgb({:.2},{:.2},{:.2})", r, g, b),
        ReflectValue::Color4([r,g,b,a]) => format!("rgba({:.2},{:.2},{:.2},{:.2})", r, g, b, a),
        ReflectValue::Bool(b)         => b.to_string(),
        ReflectValue::U32(n)          => n.to_string(),
        ReflectValue::U8(n)           => n.to_string(),
        ReflectValue::USize(n)        => n.to_string(),
        ReflectValue::Str(s)          => s.clone(),
        ReflectValue::OptionStr(o)    => o.clone().unwrap_or_default(),
        ReflectValue::Enum(e)         => e.clone(),
    }
}
