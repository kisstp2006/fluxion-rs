// ============================================================
// theme.rs — Unity-like dark egui theme
// ============================================================

use egui::{Color32, Context, FontId, Rounding, Stroke, Visuals};

/// Apply the Unity-dark theme to the egui context.
pub fn apply_theme(ctx: &Context) {
    let mut visuals = Visuals::dark();

    // Window / panel backgrounds
    visuals.window_fill          = Color32::from_rgb(32, 32, 36);
    visuals.panel_fill           = Color32::from_rgb(24, 24, 28);
    visuals.faint_bg_color       = Color32::from_rgb(28, 28, 32);

    // Widgets
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(40, 40, 46);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(180, 180, 190));
    visuals.widgets.noninteractive.rounding  = Rounding::same(3.0);

    visuals.widgets.inactive.bg_fill   = Color32::from_rgb(50, 50, 58);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(200, 200, 210));
    visuals.widgets.inactive.rounding  = Rounding::same(3.0);

    visuals.widgets.hovered.bg_fill   = Color32::from_rgb(65, 65, 75);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(220, 200, 100));
    visuals.widgets.hovered.rounding  = Rounding::same(3.0);

    visuals.widgets.active.bg_fill   = Color32::from_rgb(80, 75, 30);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5, Color32::from_rgb(240, 200, 60));
    visuals.widgets.active.rounding  = Rounding::same(3.0);

    visuals.widgets.open.bg_fill   = Color32::from_rgb(55, 55, 65);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, Color32::from_rgb(200, 200, 210));

    // Selection / highlight accent (Unity orange-yellow)
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(220, 180, 60, 80);
    visuals.selection.stroke  = Stroke::new(1.0, Color32::from_rgb(220, 180, 60));

    visuals.hyperlink_color   = Color32::from_rgb(100, 160, 255);
    visuals.override_text_color = None;

    // Separators, borders, resize handles
    visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(55, 55, 65));
    visuals.extreme_bg_color = Color32::from_rgb(14, 14, 18);
    visuals.code_bg_color    = Color32::from_rgb(20, 20, 26);
    visuals.warn_fg_color    = Color32::from_rgb(255, 200, 60);
    visuals.error_fg_color   = Color32::from_rgb(240, 80,  70);

    ctx.set_visuals(visuals);

    // Font sizes
    let mut style = (*ctx.style()).clone();
    style.text_styles.insert(
        egui::TextStyle::Body,
        FontId::proportional(13.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        FontId::proportional(13.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        FontId::proportional(11.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::proportional(15.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        FontId::monospace(12.0),
    );
    ctx.set_style(style);
}

/// The editor accent color (Unity orange-yellow).
#[allow(dead_code)]
pub const ACCENT: Color32 = Color32::from_rgb(220, 180, 60);
/// Panel background color.
#[allow(dead_code)]
pub const PANEL_BG: Color32 = Color32::from_rgb(24, 24, 28);
/// Toolbar background color.
pub const TOOLBAR_BG: Color32 = Color32::from_rgb(36, 36, 42);
/// Menubar background color.
pub const MENU_BG: Color32 = Color32::from_rgb(28, 28, 34);
