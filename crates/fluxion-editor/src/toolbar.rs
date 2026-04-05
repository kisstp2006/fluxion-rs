// ============================================================
// toolbar.rs — Play / Pause / Stop toolbar + transform mode
// ============================================================

use egui::{Color32, Context, RichText, TopBottomPanel, FontId};

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
            .inner_margin(egui::Margin::symmetric(6.0, 4.0)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // ── Transform tool selector ──────────────────────────────────
                let tool_btn = |ui: &mut egui::Ui, lbl: &str, t: TransformTool, current: TransformTool| {
                    let selected = t == current;
                    let text = RichText::new(lbl)
                        .font(FontId::proportional(12.0))
                        .color(if selected { Color32::from_rgb(220, 180, 60) } else { Color32::from_rgb(180, 180, 190) });
                    ui.selectable_label(selected, text)
                };

                if tool_btn(ui, "⇔ Move",   TransformTool::Translate, *tool).clicked() { *tool = TransformTool::Translate; }
                if tool_btn(ui, "↻ Rotate", TransformTool::Rotate,    *tool).clicked() { *tool = TransformTool::Rotate;    }
                if tool_btn(ui, "⊞ Scale",  TransformTool::Scale,     *tool).clicked() { *tool = TransformTool::Scale;     }

                ui.separator();

                // ── Play / Pause / Stop ──────────────────────────────────────
                let play_col  = if mode == EditorMode::Playing { Color32::from_rgb(100, 220, 100) } else { Color32::from_rgb(180, 180, 190) };
                let pause_col = if mode == EditorMode::Paused  { Color32::from_rgb(220, 200, 60)  } else { Color32::from_rgb(180, 180, 190) };
                let stop_col  = if mode == EditorMode::Editing { Color32::from_rgb(180, 180, 190) } else { Color32::from_rgb(230, 100, 80)  };

                if ui.add(egui::Button::new(
                    RichText::new("▶").font(FontId::proportional(14.0)).color(play_col)
                )).on_hover_text("Play").clicked() {
                    new_mode = if mode == EditorMode::Playing { EditorMode::Editing } else { EditorMode::Playing };
                }

                if ui.add(egui::Button::new(
                    RichText::new("⏸").font(FontId::proportional(14.0)).color(pause_col)
                )).on_hover_text("Pause").clicked() {
                    if mode == EditorMode::Playing {
                        new_mode = EditorMode::Paused;
                    } else if mode == EditorMode::Paused {
                        new_mode = EditorMode::Playing;
                    }
                }

                if ui.add(egui::Button::new(
                    RichText::new("⏹").font(FontId::proportional(14.0)).color(stop_col)
                )).on_hover_text("Stop").clicked() {
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
