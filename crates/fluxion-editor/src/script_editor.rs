// ============================================================
// script_editor.rs — In-editor .rn script editor
//
// Renders inside a dockable tab via the Rune panel system.
// State is held here (Rust side) and exposed to Rune via
// world_module bindings.
// ============================================================

use std::path::PathBuf;
use std::sync::Mutex;

use egui_code_editor::{CodeEditor, ColorTheme, Syntax};

// ── Global state (accessed from Rune bindings + main.rs) ─────────────────────

/// The single in-editor script editor state.
pub static EDITOR: Mutex<ScriptEditor> = Mutex::new(ScriptEditor::new());

pub struct ScriptEditor {
    pub open_path:  Option<PathBuf>,
    pub source:     String,
    pub dirty:      bool,
    pub diagnostics: Vec<Diagnostic>,
    /// Set by main.rs after a save triggers hot-reload compile.
    pub needs_compile: bool,
}

#[derive(Clone)]
pub struct Diagnostic {
    pub line:     usize,
    pub col:      usize,
    pub message:  String,
    pub is_error: bool,
}

impl ScriptEditor {
    pub const fn new() -> Self {
        Self {
            open_path:    None,
            source:       String::new(),
            dirty:        false,
            diagnostics:  Vec::new(),
            needs_compile: false,
        }
    }

    pub fn open(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(src) => {
                self.source       = src;
                self.open_path    = Some(path);
                self.dirty        = false;
                self.diagnostics  = Vec::new();
                self.needs_compile = true;
            }
            Err(e) => log::error!("[ScriptEditor] failed to read {}: {e}", path.display()),
        }
    }

    pub fn save(&mut self) -> bool {
        let Some(path) = self.open_path.as_ref() else { return false; };
        match std::fs::write(path, &self.source) {
            Ok(()) => {
                self.dirty = false;
                self.needs_compile = true;
                log::info!("[ScriptEditor] saved {}", path.display());
                true
            }
            Err(e) => {
                log::error!("[ScriptEditor] save failed: {e}");
                false
            }
        }
    }

    /// Run a quick rune compile check and populate diagnostics.
    pub fn compile_check(&mut self) {
        self.needs_compile = false;
        let src = self.source.clone();
        let path_str = self.open_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "<untitled>".into());

        let mut sources = rune::Sources::new();
        let _ = sources.insert(
            rune::Source::new(path_str.clone(), src.clone())
                .unwrap_or_else(|_| rune::Source::memory("").unwrap()),
        );

        let context = rune::Context::with_default_modules()
            .unwrap_or_default();
        let mut diagnostics = rune::Diagnostics::new();
        let _ = rune::prepare(&mut sources)
            .with_context(&context)
            .with_diagnostics(&mut diagnostics)
            .build();

        // Clear diagnostics if no errors — warnings from rune's default context
        // (e.g. unused variable hints) are not relevant in the built-in editor.
        if !diagnostics.has_error() {
            self.diagnostics = Vec::new();
            return;
        }

        // Emit diagnostics to a String buffer via rune's built-in emitter,
        // then parse the text output for line numbers (the span fields are private).
        use rune::termcolor::Buffer;
        let mut buf = Buffer::no_color();
        let _ = diagnostics.emit(&mut buf, &sources);
        let text = String::from_utf8_lossy(buf.as_slice()).to_string();

        let mut diags = Vec::new();
        let is_error = diagnostics.has_error();
        for line_text in text.lines() {
            // Lines look like: "  --> path:LINE:COL"  or the message line.
            if line_text.trim_start().starts_with("-->") {
                // Parse location
                let loc_part = line_text.trim_start().trim_start_matches("-->").trim();
                // loc_part is "path:line:col" or similar
                let mut parts = loc_part.rsplitn(3, ':');
                let col  = parts.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                let line = parts.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                diags.push(Diagnostic { line, col, message: String::new(), is_error });
            } else if let Some(d) = diags.last_mut() {
                if d.message.is_empty() {
                    // First non-location line after the location is the message
                    let msg = line_text.trim().to_string();
                    if !msg.is_empty() && !msg.starts_with('|') && !msg.starts_with('^') {
                        d.message = msg;
                    }
                }
            }
        }
        // Only add the generic fallback if we know there was an error but couldn't parse location.
        if diags.is_empty() && diagnostics.has_error() {
            diags.push(Diagnostic { line: 0, col: 0, message: "Compile error (see console)".into(), is_error: true });
        }

        self.diagnostics = diags;
    }
}

// ── Rune syntax definition ────────────────────────────────────────────────────

fn rune_syntax() -> Syntax {
    Syntax::new("Rune")
        .with_comment("//")
        .with_comment_multiline(["/*", "*/"])
        .with_keywords([
            "fn", "let", "const", "pub", "if", "else", "while", "loop",
            "for", "in", "return", "break", "continue", "mod", "use",
            "struct", "enum", "match", "impl", "async", "await", "yield",
            "select", "is", "not", "and", "or", "typeof", "self",
        ])
        .with_special([
            "true", "false",
        ])
        .with_types([
            "i64", "f64", "bool", "String", "Vec", "Option",
        ])
}

// ── egui rendering ────────────────────────────────────────────────────────────

/// Render the script editor panel. Called each frame from `dock.rs`.
pub fn render(ui: &mut egui::Ui, editor: &mut ScriptEditor, save_requested: bool) {
    if save_requested && editor.dirty {
        if editor.save() {
            editor.compile_check();
        }
    }
    if editor.needs_compile {
        editor.compile_check();
    }

    let filename = editor.open_path.as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "No file open".to_string());

    let dirty_dot = if editor.dirty { " ●" } else { "" };

    // ── Header bar ────────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("📄 {filename}{dirty_dot}"))
            .strong()
            .color(if editor.dirty {
                egui::Color32::from_rgb(255, 200, 80)
            } else {
                egui::Color32::from_rgb(200, 200, 200)
            }));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("💾 Save").clicked() {
                if editor.save() { editor.compile_check(); }
            }
            if editor.open_path.is_none() {
                ui.label(egui::RichText::new("Open a .rn file from the Assets panel")
                    .italics()
                    .color(egui::Color32::from_rgb(128, 128, 128)));
            }
        });
    });
    ui.separator();

    if editor.open_path.is_none() {
        ui.centered_and_justified(|ui| {
            ui.label(egui::RichText::new(
                "Double-click a .rn file in the Assets panel to open it here."
            ).color(egui::Color32::from_rgb(128, 128, 128)).italics());
        });
        return;
    }

    // ── Error/warning badge ───────────────────────────────────────────────────
    let error_count = editor.diagnostics.iter().filter(|d| d.is_error).count();
    let warn_count  = editor.diagnostics.iter().filter(|d| !d.is_error).count();
    if error_count > 0 || warn_count > 0 {
        ui.horizontal(|ui| {
            if error_count > 0 {
                ui.label(egui::RichText::new(format!("✖ {error_count} error(s)"))
                    .color(egui::Color32::from_rgb(240, 80, 80)).small());
            }
            if warn_count > 0 {
                ui.label(egui::RichText::new(format!("⚠ {warn_count} warning(s)"))
                    .color(egui::Color32::from_rgb(255, 190, 60)).small());
            }
        });
    }

    // ── Layout: diagnostics panel pinned to bottom, code fills the rest ───────
    let fontsize    = 13.0_f32;
    let diag_height = if editor.diagnostics.is_empty() { 0.0_f32 } else { 110.0 };

    // Diagnostics pinned to the bottom of the available area.
    if !editor.diagnostics.is_empty() {
        let total_h = ui.available_height();
        // Code area fills everything above the diagnostics panel.
        let code_h = (total_h - diag_height - 6.0).max(60.0);
        let row_height = fontsize * 1.4;
        let rows = (code_h / row_height).floor().max(4.0) as usize;

        // Render code editor in a fixed-height child UI.
        let code_response = egui::Frame::NONE.show(ui, |ui| {
            ui.set_height(code_h);
            CodeEditor::default()
                .id_source("script_editor_code")
                .with_syntax(rune_syntax())
                .with_theme(ColorTheme::GITHUB_DARK)
                .with_rows(rows)
                .with_fontsize(fontsize)
                .with_numlines(true)
                .vscroll(true)
                .desired_width(f32::INFINITY)
                .show(ui, &mut editor.source)
        });
        if code_response.inner.response.changed() {
            editor.dirty = true;
        }

        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("script_diag_scroll")
            .max_height(diag_height)
            .show(ui, |ui| {
                for d in &editor.diagnostics {
                    let (icon, color) = if d.is_error {
                        ("✖", egui::Color32::from_rgb(240, 80, 80))
                    } else {
                        ("⚠", egui::Color32::from_rgb(255, 190, 60))
                    };
                    ui.label(egui::RichText::new(
                        format!("{icon} [{:>3}:{:<3}] {}", d.line, d.col, d.message)
                    ).color(color).monospace().small());
                }
            });
    } else {
        // No diagnostics — code editor takes the full remaining height.
        let code_h = ui.available_height().max(60.0);
        let row_height = fontsize * 1.4;
        let rows = (code_h / row_height).floor().max(4.0) as usize;

        let response = CodeEditor::default()
            .id_source("script_editor_code")
            .with_syntax(rune_syntax())
            .with_theme(ColorTheme::GITHUB_DARK)
            .with_rows(rows)
            .with_fontsize(fontsize)
            .with_numlines(true)
            .vscroll(true)
            .desired_width(f32::INFINITY)
            .show(ui, &mut editor.source);
        if response.response.changed() {
            editor.dirty = true;
        }
    }
}
