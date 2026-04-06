// ============================================================
// toolbar.rs — Play / Pause / Stop toolbar + transform mode
// ============================================================

use egui::{Color32, Context, RichText, TopBottomPanel, FontId};
use crate::icons;

// ── Editor mode ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Editing,
    Playing,
    Paused,
}

// ── Transform tool ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformTool {
    Translate,
    Rotate,
    Scale,
}

// ── Show ─────────────────────────────────────────────────────────────────────

/// Render the toolbar panel.  Returns the new `EditorMode` if it changed.
#[allow(dead_code)]
pub fn show_toolbar(
    ctx:        &Context,
    mode:       EditorMode,
    tool:       &mut TransformTool,
    proj_name:  &str,
    scene_name: &str,
) -> EditorMode {
    let mut new_mode = mode;

    TopBottomPanel::top("toolbar_panel")
        .exact_height(32.0)
        .frame(egui::Frame::none()
            .fill(crate::theme::TOOLBAR_BG)
            .inner_margin(egui::Margin::symmetric(6, 4)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // ── Transform tool selector ──────────────────────────────────
                let icon_tool_btn = |ui: &mut egui::Ui, icon: &str, tip: &str, t: TransformTool, current: TransformTool| {
                    let selected = t == current;
                    let tint = if selected { Color32::from_rgb(220, 180, 60) } else { Color32::from_rgb(160, 160, 175) };
                    let resp = ui.add(
                        egui::ImageButton::new(icons::img(icon, 16.0, tint)).frame(false)
                    ).on_hover_text(tip);
                    if selected {
                        let rect = resp.rect;
                        ui.painter().rect_stroke(rect.expand(2.0), 2.0, egui::Stroke::new(1.0, tint), egui::StrokeKind::Outside);
                    }
                    resp
                };

                if icon_tool_btn(ui, "move",       "Move (W)",   TransformTool::Translate, *tool).clicked() { *tool = TransformTool::Translate; }
                if icon_tool_btn(ui, "rotate-cw",  "Rotate (E)", TransformTool::Rotate,    *tool).clicked() { *tool = TransformTool::Rotate;    }
                if icon_tool_btn(ui, "maximize-2", "Scale (R)",  TransformTool::Scale,     *tool).clicked() { *tool = TransformTool::Scale;     }

                ui.separator();

                // ── Play / Pause / Stop ──────────────────────────────────────
                let play_col  = if mode == EditorMode::Playing { Color32::from_rgb(100, 220, 100) } else { Color32::from_rgb(180, 180, 190) };
                let pause_col = if mode == EditorMode::Paused  { Color32::from_rgb(220, 200, 60)  } else { Color32::from_rgb(180, 180, 190) };
                let stop_col  = if mode == EditorMode::Editing { Color32::from_rgb(180, 180, 190) } else { Color32::from_rgb(230, 100, 80)  };

                if ui.add(egui::ImageButton::new(icons::img("play",   18.0, play_col )).frame(false)).on_hover_text("Play") .clicked() {
                    new_mode = if mode == EditorMode::Playing { EditorMode::Editing } else { EditorMode::Playing };
                }
                if ui.add(egui::ImageButton::new(icons::img("pause",  18.0, pause_col)).frame(false)).on_hover_text("Pause").clicked() {
                    if mode == EditorMode::Playing      { new_mode = EditorMode::Paused;  }
                    else if mode == EditorMode::Paused  { new_mode = EditorMode::Playing; }
                }
                if ui.add(egui::ImageButton::new(icons::img("square", 18.0, stop_col )).frame(false)).on_hover_text("Stop") .clicked() {
                    new_mode = EditorMode::Editing;
                }

                ui.separator();

                // ── Project / scene name ─────────────────────────────────────
                ui.label(
                    RichText::new(format!("{proj_name}  ›  {scene_name}"))
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(140, 140, 155)),
                );

                // ── Mode indicator badge ──────────────────────────────────────
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (badge_text, badge_col) = match mode {
                        EditorMode::Editing => ("EDITOR",  Color32::from_rgb(100, 140, 220)),
                        EditorMode::Playing => ("PLAYING", Color32::from_rgb(100, 210, 100)),
                        EditorMode::Paused  => ("PAUSED",  Color32::from_rgb(220, 190, 60)),
                    };
                    ui.label(
                        RichText::new(badge_text)
                            .font(FontId::monospace(10.0))
                            .color(badge_col),
                    );
                });
            });
        });

    new_mode
}
