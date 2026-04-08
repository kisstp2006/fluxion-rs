// ============================================================
// project_chooser.rs — Startup project chooser screen
//
// Shown fullscreen before the main editor.  Lets the user:
//   • Open a recent project (click in the list)
//   • Browse for an existing project folder
//   • Create a new project (name + directory)
// Returns a `ProjectChoice` to the caller when the user confirms.
// ============================================================

use std::path::PathBuf;

use egui::{Align2, Color32, Context, FontId, RichText, Vec2, Window};

use fluxion_core::{
    ProjectConfig, RecentProject,
    load_project, load_recent_projects, push_recent_project,
    TemplateRegistry, TemplateCategory, TemplateOptions, TemplateInstaller,
};

// ── Result type ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ProjectChoice {
    pub config: ProjectConfig,
    pub root:   PathBuf,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct ProjectChooser {
    recent:        Vec<RecentProject>,
    new_name:      String,
    new_dir:       String,
    error_msg:     Option<String>,
    tab:           ChooserTab,
    ready:         Option<ProjectChoice>,
    // Template system
    template_registry: TemplateRegistry,
    selected_template: Option<String>,
    template_search: String,
    template_filter_category: Option<TemplateCategory>,
}

#[derive(PartialEq, Clone, Copy)]
enum ChooserTab { Recent, New }

impl ProjectChooser {
    pub fn new() -> Self {
        Self {
            recent:    load_recent_projects(),
            new_name:  String::new(),
            new_dir:   String::new(),
            error_msg: None,
            tab:       ChooserTab::Recent,
            ready:     None,
            // Template system
            template_registry: TemplateRegistry::new(),
            selected_template: Some("empty_3d".to_string()), // Default template
            template_search: String::new(),
            template_filter_category: None,
        }
    }

    /// Returns `Some(ProjectChoice)` once the user has confirmed a selection.
    /// After this returns `Some`, discard the `ProjectChooser`.
    pub fn take_choice(&mut self) -> Option<ProjectChoice> {
        self.ready.take()
    }

    // ── Rendering ────────────────────────────────────────────────────────────

    /// Call every frame inside an `egui::Context::run` closure.
    pub fn show(&mut self, ctx: &Context) {
        // Dark translucent overlay behind the window.
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("chooser_backdrop"))
            .order(egui::Order::Background)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen, 0.0, Color32::from_rgba_unmultiplied(18, 18, 22, 255));
            });

        Window::new("Fluxion Project Manager")
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .frame(egui::Frame::window(&ctx.global_style()).fill(Color32::from_rgb(30, 30, 38)))
            .fixed_size([900.0, 600.0])
            .show(ctx, |ui| {
                ui.style_mut().spacing.item_spacing = Vec2::new(8.0, 6.0);
                self.show_inner(ui);
            });
    }

    fn show_inner(&mut self, ui: &mut egui::Ui) {
        // Title row
        ui.add_space(4.0);
        ui.label(
            RichText::new("FluxionRS  —  Project Manager")
                .font(FontId::proportional(20.0))
                .color(Color32::from_rgb(220, 180, 80)),
        );
        ui.add_space(6.0);
        ui.separator();

        // Tab bar
        ui.horizontal(|ui| {
            if ui.selectable_label(self.tab == ChooserTab::Recent, "Recent Projects").clicked() {
                self.tab = ChooserTab::Recent;
            }
            if ui.selectable_label(self.tab == ChooserTab::New, "New Project").clicked() {
                self.tab = ChooserTab::New;
            }
        });
        ui.separator();

        match self.tab {
            ChooserTab::Recent => self.show_recent(ui),
            ChooserTab::New    => self.show_new(ui),
        }

        if let Some(ref msg) = self.error_msg.clone() {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("⚠  {msg}")).color(Color32::from_rgb(240, 100, 80)));
        }
    }

    fn show_recent(&mut self, ui: &mut egui::Ui) {
        let recent = self.recent.clone();
        if recent.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("No recent projects.\nUse 'New Project' to get started.")
                        .color(Color32::GRAY)
                        .font(FontId::proportional(16.0)),
                );
            });
        } else {
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for entry in &recent {
                    let clicked = self.recent_row(ui, entry);
                    if clicked {
                        self.open_from_recent(entry.path.clone());
                        return;
                    }
                }
            });
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("📂  Open Project…").clicked() {
                self.open_browse();
            }
        });
    }

    fn recent_row(&self, ui: &mut egui::Ui, entry: &RecentProject) -> bool {
        let frame = egui::Frame::NONE
            .fill(Color32::from_rgb(40, 40, 50))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .corner_radius(4.0);

        let mut clicked = false;
        frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&entry.name).font(FontId::proportional(14.0)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(&entry.last_opened)
                            .font(FontId::proportional(11.0))
                            .color(Color32::GRAY),
                    );
                });
            });
            ui.label(RichText::new(&entry.path).font(FontId::proportional(11.0)).color(Color32::from_rgb(130, 130, 150)));
            if ui.button("Open").clicked() {
                clicked = true;
            }
        });
        clicked
    }

    fn show_new(&mut self, ui: &mut egui::Ui) {
        // Split view: template gallery on left, project details on right
        ui.horizontal(|ui| {
            // Left side - Template Gallery (60% width)
            ui.allocate_ui_with_layout([540.0, 400.0].into(), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                self.show_template_gallery(ui);
            });
            
            ui.separator();
            
            // Right side - Project Details (40% width)
            ui.allocate_ui_with_layout([340.0, 400.0].into(), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                self.show_project_details(ui);
            });
        });
    }
    
    fn show_template_gallery(&mut self, ui: &mut egui::Ui) {
        ui.heading("Choose Template");
        ui.add_space(4.0);
        
        // Search and filter
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.add(egui::TextEdit::singleline(&mut self.template_search)
                .hint_text("Search templates...")
                .desired_width(200.0));
            
            ui.separator();
            
            // Category filter
            egui::ComboBox::from_label("Category")
                .selected_text(match self.template_filter_category {
                    Some(TemplateCategory::Empty) => "Empty",
                    Some(TemplateCategory::ThreeD) => "3D",
                    Some(TemplateCategory::TwoD) => "2D", 
                    Some(TemplateCategory::VR) => "VR",
                    Some(TemplateCategory::Mobile) => "Mobile",
                    Some(TemplateCategory::Educational) => "Educational",
                    None => "All",
                })
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.template_filter_category.is_none(), "All").clicked() {
                        self.template_filter_category = None;
                    }
                    if ui.selectable_label(self.template_filter_category == Some(TemplateCategory::Empty), "Empty").clicked() {
                        self.template_filter_category = Some(TemplateCategory::Empty);
                    }
                    if ui.selectable_label(self.template_filter_category == Some(TemplateCategory::ThreeD), "3D").clicked() {
                        self.template_filter_category = Some(TemplateCategory::ThreeD);
                    }
                    if ui.selectable_label(self.template_filter_category == Some(TemplateCategory::TwoD), "2D").clicked() {
                        self.template_filter_category = Some(TemplateCategory::TwoD);
                    }
                    if ui.selectable_label(self.template_filter_category == Some(TemplateCategory::VR), "VR").clicked() {
                        self.template_filter_category = Some(TemplateCategory::VR);
                    }
                    if ui.selectable_label(self.template_filter_category == Some(TemplateCategory::Mobile), "Mobile").clicked() {
                        self.template_filter_category = Some(TemplateCategory::Mobile);
                    }
                    if ui.selectable_label(self.template_filter_category == Some(TemplateCategory::Educational), "Educational").clicked() {
                        self.template_filter_category = Some(TemplateCategory::Educational);
                    }
                });
        });
        
        ui.add_space(8.0);
        
        // Template grid
        let templates = if self.template_search.is_empty() && self.template_filter_category.is_none() {
            self.template_registry.get_all()
        } else if !self.template_search.is_empty() {
            self.template_registry.search(&self.template_search)
        } else {
            self.template_registry.get_by_category(self.template_filter_category.unwrap())
        };
        
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            let mut clicked_template = None;
            
            for (i, template_meta) in templates.iter().enumerate() {
                let template_id = if let Some(_id) = self.template_registry.get_all().iter()
                    .find(|m| std::ptr::eq(*m, template_meta))
                    .and_then(|m| self.template_registry.get_all().iter().position(|x| std::ptr::eq(x, m))) {
                    // Get template ID by matching metadata - this is a workaround
                    // In a real implementation, we'd have a better way to map metadata to ID
                    match i {
                        0 => "empty_3d",
                        1 => "empty_2d", 
                        2 => "basic_3d",
                        _ => "empty_3d",
                    }
                } else {
                    "empty_3d"
                };
                
                let is_selected = self.selected_template.as_ref() == Some(&template_id.to_string());
                
                let frame = egui::Frame::NONE
                    .fill(if is_selected { 
                        Color32::from_rgb(60, 80, 120) 
                    } else { 
                        Color32::from_rgb(40, 40, 50) 
                    })
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .corner_radius(6.0);
                
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Template icon/thumbnail placeholder
                        ui.label(
                            RichText::new("📁")
                                .font(FontId::proportional(24.0))
                                .color(Color32::from_rgb(180, 180, 180))
                        );
                        
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(&template_meta.name)
                                    .font(FontId::proportional(14.0))
                                    .strong()
                                    .color(Color32::WHITE)
                            );
                            ui.label(
                                RichText::new(&template_meta.description)
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(150, 150, 150))
                            );
                            
                            // Tags
                            if !template_meta.tags.is_empty() {
                                ui.horizontal_wrapped(|ui| {
                                    for tag in &template_meta.tags {
                                        ui.label(
                                            RichText::new(format!("#{}", tag))
                                                .font(FontId::proportional(10.0))
                                                .color(Color32::from_rgb(100, 150, 200))
                                        );
                                    }
                                });
                            }
                        });
                    });
                    
                    if ui.button("Select").clicked() {
                        clicked_template = Some(template_id.to_string());
                    }
                });
                
                ui.add_space(4.0);
            }
            
            if let Some(template_id) = clicked_template {
                self.selected_template = Some(template_id);
            }
        });
    }
    
    fn show_project_details(&mut self, ui: &mut egui::Ui) {
        ui.heading("Project Details");
        ui.add_space(8.0);
        
        // Show selected template details
        if let Some(template_id) = &self.selected_template {
            if let Some(template_meta) = self.template_registry.get_metadata(template_id) {
                ui.group(|ui| {
                    ui.heading(&template_meta.name);
                    ui.add_space(4.0);
                    ui.label(&template_meta.long_description);
                    
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Difficulty:");
                        ui.label(
                            RichText::new(format!("{:?}", template_meta.difficulty))
                                .color(Color32::from_rgb(150, 200, 150))
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Size:");
                        ui.label(
                            RichText::new(format!("{:?}", template_meta.size))
                                .color(Color32::from_rgb(150, 150, 200))
                        );
                    });
                });
            }
        }
        
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        
        // Project creation form
        ui.heading("Project Settings");
        ui.add_space(4.0);
        
        egui::Grid::new("new_proj_grid")
            .num_columns(2)
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                ui.label("Project Name:");
                ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("MyGame").desired_width(200.0));
                ui.end_row();

                ui.label("Directory:");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_dir).hint_text("/path/to/projects").desired_width(140.0));
                    if ui.button("Browse…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.new_dir = path.to_string_lossy().to_string();
                        }
                    }
                });
                ui.end_row();
            });

        ui.add_space(12.0);

        let can_create = !self.new_name.trim().is_empty() 
            && !self.new_dir.trim().is_empty() 
            && self.selected_template.is_some();
            
        ui.add_enabled_ui(can_create, |ui| {
            if ui.button(RichText::new("  ✔  Create Project  ").font(FontId::proportional(14.0))).clicked() {
                self.create_new_from_template();
            }
        });
    }

    // ── Actions ──────────────────────────────────────────────────────────────

    fn open_from_recent(&mut self, path: String) {
        let root = PathBuf::from(&path);
        match load_project(&root) {
            Ok(config) => {
                self.record_and_emit(config, root);
            }
            Err(e) => {
                self.error_msg = Some(format!("Could not open project: {e}"));
            }
        }
    }

    fn open_browse(&mut self) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            let root = folder.clone();
            match load_project(&root) {
                Ok(config) => {
                    self.record_and_emit(config, root);
                }
                Err(e) => {
                    self.error_msg = Some(format!("Could not open project: {e}"));
                }
            }
        }
    }

    fn create_new_from_template(&mut self) {
        let name = self.new_name.trim().to_string();
        let base = PathBuf::from(self.new_dir.trim());
        let root = base.join(&name);
        
        if let Some(template_id) = &self.selected_template {
            let options = TemplateOptions {
                name: name.clone(),
                directory: root.to_string_lossy().to_string(),
                custom_options: std::collections::HashMap::new(),
            };
            
            match TemplateInstaller::new(template_id.clone(), options) {
                Ok(installer) => {
                    match installer.install() {
                        Ok(()) => {
                            // Load the created project
                            match load_project(&root) {
                                Ok(config) => {
                                    self.record_and_emit(config, root);
                                }
                                Err(e) => {
                                    self.error_msg = Some(format!("Failed to load created project: {e}"));
                                }
                            }
                        }
                        Err(e) => {
                            self.error_msg = Some(format!("Template installation failed: {e}"));
                        }
                    }
                }
                Err(e) => {
                    self.error_msg = Some(format!("Failed to create installer: {e}"));
                }
            }
        } else {
            self.error_msg = Some("No template selected".to_string());
        }
    }

    fn record_and_emit(&mut self, config: ProjectConfig, root: PathBuf) {
        self.error_msg = None;
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        push_recent_project(RecentProject {
            name:        config.name.clone(),
            path:        root.to_string_lossy().to_string(),
            last_opened: now,
        });
        self.ready = Some(ProjectChoice { config, root });
    }
}
