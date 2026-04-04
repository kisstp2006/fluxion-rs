// ============================================================
// fluxion-scripting — Window / Screen / Application API
//
// Unity equivalents:
//   Screen.width / Screen.height / Screen.fullScreen
//   Application.targetFrameRate
//   Application.Quit()
//   Application.OpenURL(url)
//   Application.productName / companyName / version
//   Screen.SetResolution(w, h, fullscreen)
// ============================================================

use std::sync::Mutex;
use fluxion_core::ReflectValue;
use crate::binding_registry::{BindingEntry, ParamMeta, ScriptBindingRegistry, ScriptType};

// ── Shared mutable window state ────────────────────────────────────────────────
// Rust reads these and applies them to the winit window at end of frame.

#[derive(Debug, Clone)]
pub struct WindowRequest {
    pub kind: WindowRequestKind,
}

#[derive(Debug, Clone)]
pub enum WindowRequestKind {
    SetTitle(String),
    SetResolution { w: u32, h: u32, fullscreen: bool },
    SetFullscreen(bool),
    SetTargetFrameRate(i32),
    Quit,
    OpenURL(String),
    SetCursorVisible(bool),
    SetCursorLockMode(CursorLockMode),
    SetVSync(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorLockMode { None, Locked, Confined }

lazy_static::lazy_static! {
    pub static ref WINDOW_REQUESTS: Mutex<Vec<WindowRequest>> = Mutex::new(Vec::new());

    static ref WINDOW_STATE: Mutex<WindowState> = Mutex::new(WindowState::default());
}

#[derive(Debug, Clone)]
pub struct WindowState {
    pub width:             u32,
    pub height:            u32,
    pub fullscreen:        bool,
    pub title:             String,
    pub target_frame_rate: i32,
    pub vsync:             bool,
    pub cursor_visible:    bool,
    pub cursor_lock_mode:  CursorLockMode,
    pub product_name:      String,
    pub company_name:      String,
    pub version:           String,
}

impl Default for WindowState {
    fn default() -> Self {
        WindowState {
            width: 1280, height: 720, fullscreen: false,
            title: "FluxionRS".into(),
            target_frame_rate: -1,
            vsync: true,
            cursor_visible: true,
            cursor_lock_mode: CursorLockMode::None,
            product_name: "FluxionRS Game".into(),
            company_name: "".into(),
            version: "0.1.0".into(),
        }
    }
}

fn push_request(kind: WindowRequestKind) {
    if let Ok(mut q) = WINDOW_REQUESTS.lock() {
        q.push(WindowRequest { kind });
    }
}

/// Drain pending window requests — call once per frame from the main loop.
pub fn drain_requests() -> Vec<WindowRequest> {
    WINDOW_REQUESTS.lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

/// Update the cached window state (call from main loop after applying changes).
pub fn update_state(width: u32, height: u32, fullscreen: bool) {
    if let Ok(mut s) = WINDOW_STATE.lock() {
        s.width = width;
        s.height = height;
        s.fullscreen = fullscreen;
    }
}

pub fn set_title_state(title: &str) {
    if let Ok(mut s) = WINDOW_STATE.lock() {
        s.title = title.to_string();
    }
}

pub fn register(reg: &mut ScriptBindingRegistry) {
    // ── Screen ────────────────────────────────────────────────────────────────
    reg.register("Screen", BindingEntry::new(
        "getWidth",
        "Returns the current screen/window width in pixels.",
        vec![],
        Some(ScriptType::Int),
        |_| {
            let w = WINDOW_STATE.lock().map(|s| s.width).unwrap_or(1280);
            Ok(Some(ReflectValue::U32(w)))
        },
    ));

    reg.register("Screen", BindingEntry::new(
        "getHeight",
        "Returns the current screen/window height in pixels.",
        vec![],
        Some(ScriptType::Int),
        |_| {
            let h = WINDOW_STATE.lock().map(|s| s.height).unwrap_or(720);
            Ok(Some(ReflectValue::U32(h)))
        },
    ));

    reg.register("Screen", BindingEntry::new(
        "getFullScreen",
        "Returns whether the application is running in full-screen mode.",
        vec![],
        Some(ScriptType::Bool),
        |_| {
            let fs = WINDOW_STATE.lock().map(|s| s.fullscreen).unwrap_or(false);
            Ok(Some(ReflectValue::Bool(fs)))
        },
    ));

    reg.register("Screen", BindingEntry::new(
        "setFullScreen",
        "Enter or exit full-screen mode.",
        vec![ParamMeta::new("fullscreen", ScriptType::Bool)],
        None,
        |args| {
            let fs = match args.first() { Some(ReflectValue::Bool(b)) => *b, _ => false };
            push_request(WindowRequestKind::SetFullscreen(fs));
            Ok(None)
        },
    ));

    reg.register("Screen", BindingEntry::new(
        "SetResolution",
        "Sets the screen resolution. fullscreen is optional (default false).",
        vec![
            ParamMeta::new("width",      ScriptType::Int),
            ParamMeta::new("height",     ScriptType::Int),
            ParamMeta::new("fullscreen", ScriptType::Bool).optional(),
        ],
        None,
        |args| {
            let w  = match args.first()     { Some(ReflectValue::U32(n)) => *n, Some(ReflectValue::F32(f)) => *f as u32, _ => 1280 };
            let h  = match args.get(1)      { Some(ReflectValue::U32(n)) => *n, Some(ReflectValue::F32(f)) => *f as u32, _ => 720  };
            let fs = match args.get(2)      { Some(ReflectValue::Bool(b)) => *b, _ => false };
            push_request(WindowRequestKind::SetResolution { w, h, fullscreen: fs });
            Ok(None)
        },
    ));

    reg.register("Screen", BindingEntry::new(
        "getDpi",
        "Returns the approximate DPI of the screen.",
        vec![],
        Some(ScriptType::Float),
        |_| Ok(Some(ReflectValue::F32(96.0))),
    ));

    // ── Application ───────────────────────────────────────────────────────────
    reg.register("Application", BindingEntry::new(
        "Quit",
        "Quits the application.",
        vec![],
        None,
        |_| { push_request(WindowRequestKind::Quit); Ok(None) },
    ));

    reg.register("Application", BindingEntry::new(
        "OpenURL",
        "Opens the specified URL in the default browser.",
        vec![ParamMeta::new("url", ScriptType::String)],
        None,
        |args| {
            let url = match args.first() { Some(ReflectValue::Str(s)) => s.clone(), _ => return Ok(None) };
            push_request(WindowRequestKind::OpenURL(url));
            Ok(None)
        },
    ));

    reg.register("Application", BindingEntry::new(
        "getProductName",
        "Returns the product name set in project settings.",
        vec![],
        Some(ScriptType::String),
        |_| {
            let n = WINDOW_STATE.lock().map(|s| s.product_name.clone()).unwrap_or_default();
            Ok(Some(ReflectValue::Str(n)))
        },
    ));

    reg.register("Application", BindingEntry::new(
        "getCompanyName",
        "Returns the company name.",
        vec![],
        Some(ScriptType::String),
        |_| {
            let n = WINDOW_STATE.lock().map(|s| s.company_name.clone()).unwrap_or_default();
            Ok(Some(ReflectValue::Str(n)))
        },
    ));

    reg.register("Application", BindingEntry::new(
        "getVersion",
        "Returns the application version string.",
        vec![],
        Some(ScriptType::String),
        |_| {
            let v = WINDOW_STATE.lock().map(|s| s.version.clone()).unwrap_or_default();
            Ok(Some(ReflectValue::Str(v)))
        },
    ));

    reg.register("Application", BindingEntry::new(
        "getTargetFrameRate",
        "Returns the target frame rate (-1 = unlimited).",
        vec![],
        Some(ScriptType::Int),
        |_| {
            let r = WINDOW_STATE.lock().map(|s| s.target_frame_rate).unwrap_or(-1);
            Ok(Some(ReflectValue::F32(r as f32)))
        },
    ));

    reg.register("Application", BindingEntry::new(
        "setTargetFrameRate",
        "Sets the target frame rate. -1 = unlimited.",
        vec![ParamMeta::new("rate", ScriptType::Int)],
        None,
        |args| {
            let r = match args.first() { Some(ReflectValue::F32(f)) => *f as i32, Some(ReflectValue::U32(n)) => *n as i32, _ => -1 };
            if let Ok(mut s) = WINDOW_STATE.lock() { s.target_frame_rate = r; }
            push_request(WindowRequestKind::SetTargetFrameRate(r));
            Ok(None)
        },
    ));

    reg.register("Application", BindingEntry::new(
        "getDataPath",
        "Returns the path to the application data folder.",
        vec![],
        Some(ScriptType::String),
        |_| Ok(Some(ReflectValue::Str("assets".into()))),
    ));

    reg.register("Application", BindingEntry::new(
        "getPlatform",
        "Returns the runtime platform identifier string.",
        vec![],
        Some(ScriptType::String),
        |_| {
            let p = if cfg!(target_os = "windows") { "WindowsPlayer" }
                    else if cfg!(target_os = "macos") { "OSXPlayer" }
                    else if cfg!(target_os = "linux") { "LinuxPlayer" }
                    else { "Unknown" };
            Ok(Some(ReflectValue::Str(p.into())))
        },
    ));

    // ── Cursor ────────────────────────────────────────────────────────────────
    reg.register("Cursor", BindingEntry::new(
        "setVisible",
        "Shows or hides the cursor.",
        vec![ParamMeta::new("visible", ScriptType::Bool)],
        None,
        |args| {
            let v = match args.first() { Some(ReflectValue::Bool(b)) => *b, _ => true };
            push_request(WindowRequestKind::SetCursorVisible(v));
            Ok(None)
        },
    ));

    reg.register("Cursor", BindingEntry::new(
        "setLockMode",
        "Sets cursor lock mode: 0=None, 1=Locked, 2=Confined.",
        vec![ParamMeta::new("mode", ScriptType::Int)],
        None,
        |args| {
            let mode = match args.first() {
                Some(ReflectValue::F32(f)) => match *f as u32 { 1 => CursorLockMode::Locked, 2 => CursorLockMode::Confined, _ => CursorLockMode::None },
                _ => CursorLockMode::None,
            };
            push_request(WindowRequestKind::SetCursorLockMode(mode));
            Ok(None)
        },
    ));

    // ── Window title (direct convenience) ────────────────────────────────────
    reg.register("Application", BindingEntry::new(
        "setTitle",
        "Sets the window title.",
        vec![ParamMeta::new("title", ScriptType::String)],
        None,
        |args| {
            let t = match args.first() { Some(ReflectValue::Str(s)) => s.clone(), _ => return Ok(None) };
            push_request(WindowRequestKind::SetTitle(t.clone()));
            set_title_state(&t);
            Ok(None)
        },
    ));

    reg.register("Application", BindingEntry::new(
        "getTitle",
        "Returns the current window title.",
        vec![],
        Some(ScriptType::String),
        |_| {
            let t = WINDOW_STATE.lock().map(|s| s.title.clone()).unwrap_or_default();
            Ok(Some(ReflectValue::Str(t)))
        },
    ));
}

// ── JS extension ───────────────────────────────────────────────────────────────
pub const WINDOW_JS_EXTENSION: &str = r#"
// ── Screen: property getters/setters ─────────────────────────────────────────
Object.defineProperties(Screen, {
    width:      { get() { return Screen.getWidth(); },       configurable: true },
    height:     { get() { return Screen.getHeight(); },      configurable: true },
    fullScreen: {
        get() { return Screen.getFullScreen(); },
        set(v) { Screen.setFullScreen(v); },
        configurable: true,
    },
    dpi: { get() { return Screen.getDpi(); }, configurable: true },
});

// ── Application: property getters/setters ────────────────────────────────────
Object.defineProperties(Application, {
    productName:     { get() { return Application.getProductName(); }, configurable: true },
    companyName:     { get() { return Application.getCompanyName(); }, configurable: true },
    version:         { get() { return Application.getVersion();     }, configurable: true },
    dataPath:        { get() { return Application.getDataPath();     }, configurable: true },
    platform:        { get() { return Application.getPlatform();     }, configurable: true },
    targetFrameRate: {
        get() { return Application.getTargetFrameRate(); },
        set(v) { Application.setTargetFrameRate(v);     },
        configurable: true,
    },
});

// ── Cursor lock mode constants ────────────────────────────────────────────────
const CursorLockMode = Object.freeze({ None: 0, Locked: 1, Confined: 2 });

// ── Cursor property shims ─────────────────────────────────────────────────────
Object.defineProperties(Cursor, {
    visible: {
        get() { return true; },
        set(v) { Cursor.setVisible(v); },
        configurable: true,
    },
    lockState: {
        get() { return 0; },
        set(v) { Cursor.setLockMode(v); },
        configurable: true,
    },
});

// ── document.title shim (web-like convenience) ────────────────────────────────
if (typeof document === "undefined") {
    const _doc = {
        get title() { return Application.getTitle(); },
        set title(v){ Application.setTitle(v); },
    };
    // Make globally accessible as `document` fallback
    if (typeof globalThis !== "undefined") globalThis.document = _doc;
}
"#;
