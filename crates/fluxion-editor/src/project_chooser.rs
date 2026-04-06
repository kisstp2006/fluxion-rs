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
    create_project, load_project, load_recent_projects, push_recent_project,
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
        let screen = ctx.screen_rect();
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
            .frame(egui::Frame::window(&ctx.style()).fill(Color32::from_rgb(30, 30, 38)))
            .fixed_size([680.0, 440.0])
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
        let frame = egui::Frame::none()
            .fill(Color32::from_rgb(40, 40, 50))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .rounding(4.0);

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
        egui::Grid::new("new_proj_grid")
            .num_columns(2)
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                ui.label("Project Name:");
                ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("MyGame").desired_width(300.0));
                ui.end_row();

                ui.label("Directory:");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_dir).hint_text("/path/to/projects").desired_width(240.0));
                    if ui.button("Browse…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.new_dir = path.to_string_lossy().to_string();
                        }
                    }
                });
                ui.end_row();
            });

        ui.add_space(12.0);

        let can_create = !self.new_name.trim().is_empty() && !self.new_dir.trim().is_empty();
        ui.add_enabled_ui(can_create, |ui| {
            if ui.button(RichText::new("  ✔  Create Project  ").font(FontId::proportional(14.0))).clicked() {
                self.create_new();
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

    fn create_new(&mut self) {
        let name = self.new_name.trim().to_string();
        let base = PathBuf::from(self.new_dir.trim());
        let root = base.join(&name);
        match create_project(&root, &name) {
            Ok(config) => {
                self.record_and_emit(config, root);
            }
            Err(e) => {
                self.error_msg = Some(format!("Failed to create project: {e}"));
            }
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
