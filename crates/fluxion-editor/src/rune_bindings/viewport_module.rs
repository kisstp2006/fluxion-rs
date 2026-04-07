// ============================================================
// viewport_module.rs — fluxion::viewport Rune module
//
// Exposes multi-pane viewport layout state, per-pane TextureIds,
// fullscreen flag, and the component gizmo overlay registry.
// ============================================================

use std::cell::{Cell, RefCell};

use rune::{Module, Ref};

// ── Pane kind ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Perspective,
    Top,
    Front,
    Right,
}

impl PaneKind {
    pub fn label(self) -> &'static str {
        match self {
            PaneKind::Perspective => "Perspective",
            PaneKind::Top         => "Top",
            PaneKind::Front       => "Front",
            PaneKind::Right       => "Right",
        }
    }
    pub fn from_idx(i: usize) -> Self {
        match i {
            1 => PaneKind::Top,
            2 => PaneKind::Front,
            3 => PaneKind::Right,
            _ => PaneKind::Perspective,
        }
    }
}

// ── Viewport layout ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewportLayout {
    #[default]
    One,
    TwoH,
    TwoV,
    ThreeLeft,
    ThreeRight,
    ThreeTop,
    ThreeBottom,
    Four,
}

impl ViewportLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            ViewportLayout::One          => "One",
            ViewportLayout::TwoH         => "TwoH",
            ViewportLayout::TwoV         => "TwoV",
            ViewportLayout::ThreeLeft    => "ThreeLeft",
            ViewportLayout::ThreeRight   => "ThreeRight",
            ViewportLayout::ThreeTop     => "ThreeTop",
            ViewportLayout::ThreeBottom  => "ThreeBottom",
            ViewportLayout::Four         => "Four",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "TwoH"        => ViewportLayout::TwoH,
            "TwoV"        => ViewportLayout::TwoV,
            "ThreeLeft"   => ViewportLayout::ThreeLeft,
            "ThreeRight"  => ViewportLayout::ThreeRight,
            "ThreeTop"    => ViewportLayout::ThreeTop,
            "ThreeBottom" => ViewportLayout::ThreeBottom,
            "Four"        => ViewportLayout::Four,
            _             => ViewportLayout::One,
        }
    }
    /// Which pane indices are active for this layout.
    pub fn active_panes(self) -> &'static [usize] {
        match self {
            ViewportLayout::One          => &[0],
            ViewportLayout::TwoH         => &[0, 2],
            ViewportLayout::TwoV         => &[0, 1],
            ViewportLayout::ThreeLeft    => &[0, 1, 2],
            ViewportLayout::ThreeRight   => &[2, 0, 1],
            ViewportLayout::ThreeTop     => &[0, 1, 2],
            ViewportLayout::ThreeBottom  => &[0, 1, 2],
            ViewportLayout::Four         => &[0, 1, 2, 3],
        }
    }
}

// ── Gizmo registry entry ──────────────────────────────────────────────────────

#[derive(Clone)]
pub struct GizmoEntry {
    pub category: String,
    pub label:    String,
    pub icon:     String,
    pub enabled:  bool,
}

// ── Thread-locals ─────────────────────────────────────────────────────────────

thread_local! {
    static VP_TEXTURES:    RefCell<[Option<egui::TextureId>; 4]> = RefCell::new([None; 4]);
    static VP_LAYOUT:      Cell<ViewportLayout>                  = Cell::new(ViewportLayout::One);
    static VP_FULLSCREEN:  Cell<bool>                            = Cell::new(false);
    static GIZMO_REGISTRY: RefCell<Vec<GizmoEntry>>              = RefCell::new(Vec::new());
    /// 0=Lit 1=Albedo 2=Normal 3=Roughness 4=Metalness 5=AO 6=Emissive 7=Unlit
    static VP_DEBUG_VIEW:  Cell<u32>                             = Cell::new(0);
}

// ── Host-facing setters ───────────────────────────────────────────────────────

/// Set viewport pane 0 texture. Called by main.rs for backward compat.
pub fn set_viewport_texture(id: egui::TextureId, _width: u32, _height: u32) {
    VP_TEXTURES.with(|c| c.borrow_mut()[0] = Some(id));
}

/// Set a specific pane's texture ID.
pub fn set_pane_texture(pane: usize, id: egui::TextureId) {
    if pane < 4 {
        VP_TEXTURES.with(|c| c.borrow_mut()[pane] = Some(id));
    }
}

/// Returns the current layout.
pub fn get_layout() -> ViewportLayout {
    VP_LAYOUT.with(|c| c.get())
}

/// Returns true if viewport fullscreen mode is active.
pub fn get_fullscreen() -> bool {
    VP_FULLSCREEN.with(|c| c.get())
}

/// Returns the current debug view mode (0 = Lit / normal rendering).
pub fn get_debug_view() -> u32 {
    VP_DEBUG_VIEW.with(|c| c.get())
}

// ── Module builder ────────────────────────────────────────────────────────────

fn tex_id_to_i64(id: egui::TextureId) -> i64 {
    match id {
        egui::TextureId::Managed(v) => v as i64,
        egui::TextureId::User(v)    => (v | (1u64 << 62)) as i64,
    }
}

pub fn build_viewport_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["viewport"])?;

    // texture_id(pane) — pane 0..3; 0 = perspective (backward compat when called with no arg is not supported in Rune, use pane=0)
    m.function("texture_id", |pane: i64| -> i64 {
        let idx = pane.max(0).min(3) as usize;
        VP_TEXTURES.with(|c| {
            c.borrow()[idx].map(tex_id_to_i64).unwrap_or(-1)
        })
    }).build()?;

    // pane_label(pane)
    m.function("pane_label", |pane: i64| -> String {
        PaneKind::from_idx(pane.max(0).min(3) as usize).label().to_string()
    }).build()?;

    // Layout get/set
    m.function("get_layout", || -> String {
        VP_LAYOUT.with(|c| c.get().as_str().to_string())
    }).build()?;

    m.function("set_layout", |s: Ref<str>| {
        VP_LAYOUT.with(|c| c.set(ViewportLayout::from_str(s.as_ref())));
    }).build()?;

    // Fullscreen
    m.function("get_fullscreen", || -> bool {
        VP_FULLSCREEN.with(|c| c.get())
    }).build()?;

    m.function("set_fullscreen", |v: bool| {
        VP_FULLSCREEN.with(|c| c.set(v));
    }).build()?;

    // Active pane count for current layout
    m.function("pane_count", || -> i64 {
        VP_LAYOUT.with(|c| c.get().active_panes().len() as i64)
    }).build()?;

    // Pane index at layout slot k (e.g. slot 0 may be pane 2 in ThreeRight)
    m.function("pane_at_slot", |slot: i64| -> i64 {
        let panes = VP_LAYOUT.with(|c| c.get().active_panes());
        let k = slot.max(0) as usize;
        panes.get(k).copied().unwrap_or(0) as i64
    }).build()?;

    // Debug view mode
    m.function("get_debug_view", || -> i64 {
        VP_DEBUG_VIEW.with(|c| c.get() as i64)
    }).build()?;

    m.function("set_debug_view", |v: i64| {
        VP_DEBUG_VIEW.with(|c| c.set(v.max(0).min(7) as u32));
    }).build()?;

    // Gizmo registry
    m.function("register_gizmo", |category: Ref<str>, label: Ref<str>, icon: Ref<str>| {
        let lbl = label.as_ref().to_string();
        GIZMO_REGISTRY.with(|reg| {
            let mut r = reg.borrow_mut();
            if !r.iter().any(|e| e.label == lbl) {
                r.push(GizmoEntry {
                    category: category.as_ref().to_string(),
                    label:    lbl,
                    icon:     icon.as_ref().to_string(),
                    enabled:  true,
                });
            }
        });
    }).build()?;

    m.function("gizmo_enabled", |label: Ref<str>| -> bool {
        let lbl = label.as_ref();
        GIZMO_REGISTRY.with(|reg| {
            reg.borrow().iter()
                .find(|e| e.label == lbl)
                .map(|e| e.enabled)
                .unwrap_or(true)
        })
    }).build()?;

    m.function("set_gizmo_enabled", |label: Ref<str>, enabled: bool| {
        let lbl = label.as_ref().to_string();
        GIZMO_REGISTRY.with(|reg| {
            let mut r = reg.borrow_mut();
            if let Some(e) = r.iter_mut().find(|e| e.label == lbl) {
                e.enabled = enabled;
            }
        });
    }).build()?;

    // Returns [[category, label, icon, enabled_str], ...] for the gizmo overlay UI
    m.function("get_gizmo_registry", || -> Vec<Vec<String>> {
        GIZMO_REGISTRY.with(|reg| {
            reg.borrow().iter().map(|e| vec![
                e.category.clone(),
                e.label.clone(),
                e.icon.clone(),
                if e.enabled { "true".to_string() } else { "false".to_string() },
            ]).collect()
        })
    }).build()?;

    Ok(m)
}
