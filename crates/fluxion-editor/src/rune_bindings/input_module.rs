// ============================================================
// input_module.rs — fluxion::input Rune bindings
//
// Exposes per-frame input state to Rune scripts.
// Unity-style API: key_down, get_axis, mouse_delta, mouse_button,
// scroll_delta, mouse_position, gamepad_*.
//
// Thread-local pointer pattern — same as world/physics/audio modules.
// Set before any Rune call via set_input_context, cleared after.
// ============================================================

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

use rune::{Module, ContextError, runtime::Ref};
use fluxion_core::{InputState, InputAction, InputBinding};

// ── Thread-local pointer ──────────────────────────────────────────────────────

thread_local! {
    static INPUT_PTR:   Cell<Option<NonNull<InputState>>> = Cell::new(None);
    /// Per-frame copy of the project's input action map.
    static ACTION_MAP:  RefCell<Vec<InputAction>>         = RefCell::new(Vec::new());
}

pub fn set_input_context(input: &mut InputState) {
    INPUT_PTR.with(|c| c.set(Some(NonNull::from(input))));
}

pub fn clear_input_context() {
    INPUT_PTR.with(|c| c.set(None));
}

/// Called each frame by the host to push the current action map.
pub fn set_action_map(actions: &[InputAction]) {
    ACTION_MAP.with(|a| *a.borrow_mut() = actions.to_vec());
}

fn with_input<T, F: FnOnce(&InputState) -> T>(f: F) -> T {
    INPUT_PTR.with(|c| {
        let ptr = c.get().expect("input context not set");
        // SAFETY: pointer is valid for the duration of the Rune call frame.
        f(unsafe { ptr.as_ref() })
    })
}

// ── Module builder ────────────────────────────────────────────────────────────

pub fn build_input_module() -> Result<Module, ContextError> {
    let mut m = Module::with_crate_item("fluxion", ["input"])?;

    // ── Keyboard ──────────────────────────────────────────────────────────────

    // Returns true while the key identified by `code` is held down.
    // Key codes follow JS KeyboardEvent.code: "KeyW", "KeyA", "Space", "ShiftLeft", etc.
    m.function("key_down", |code: String| -> bool {
        with_input(|i| i.is_key_down(&code))
    }).build()?;

    // Unity-style axis: returns -1..1 from two opposing keys.
    m.function("get_axis", |neg: String, pos: String| -> f64 {
        with_input(|i| i.get_axis(&neg, &pos) as f64)
    }).build()?;

    // Default horizontal axis: A = -1, D = +1.
    m.function("axis_horizontal", || -> f64 {
        with_input(|i| i.axis_horizontal() as f64)
    }).build()?;

    // Default vertical axis: S = -1, W = +1.
    m.function("axis_vertical", || -> f64 {
        with_input(|i| i.axis_vertical() as f64)
    }).build()?;

    // ── Mouse buttons ─────────────────────────────────────────────────────────

    m.function("mouse_left", || -> bool {
        with_input(|i| i.mouse_left())
    }).build()?;

    m.function("mouse_middle", || -> bool {
        with_input(|i| i.mouse_middle())
    }).build()?;

    m.function("mouse_right", || -> bool {
        with_input(|i| i.mouse_right())
    }).build()?;

    // Returns [left, middle, right] as i64 (0 or 1).
    m.function("mouse_buttons", || -> Vec<i64> {
        with_input(|i| vec![
            i.mouse_left()   as i64,
            i.mouse_middle() as i64,
            i.mouse_right()  as i64,
        ])
    }).build()?;

    // ── Mouse movement ────────────────────────────────────────────────────────

    // Returns [dx, dy] pixel delta since last frame.
    m.function("mouse_delta", || -> Vec<f64> {
        with_input(|i| {
            let (dx, dy) = i.mouse_delta();
            vec![dx as f64, dy as f64]
        })
    }).build()?;

    // Returns [x, y] current mouse position in window pixels.
    m.function("mouse_position", || -> Vec<f64> {
        with_input(|i| {
            let (x, y) = i.mouse_position();
            vec![x as f64, y as f64]
        })
    }).build()?;

    // Returns [dx, dy] scroll wheel delta since last frame.
    m.function("scroll_delta", || -> Vec<f64> {
        with_input(|i| {
            let (dx, dy) = i.scroll_delta();
            vec![dx as f64, dy as f64]
        })
    }).build()?;

    // ── Gamepad ───────────────────────────────────────────────────────────────

    m.function("gamepad_connected", || -> bool {
        with_input(|i| i.gamepad_connected)
    }).build()?;

    // Returns left stick as [x, y] in -1..1.
    m.function("gamepad_left_stick", || -> Vec<f64> {
        with_input(|i| vec![
            i.gamepad_left_stick.0 as f64,
            i.gamepad_left_stick.1 as f64,
        ])
    }).build()?;

    // Returns right stick as [x, y] in -1..1.
    m.function("gamepad_right_stick", || -> Vec<f64> {
        with_input(|i| vec![
            i.gamepad_right_stick.0 as f64,
            i.gamepad_right_stick.1 as f64,
        ])
    }).build()?;

    // Returns [left_trigger, right_trigger] in 0..1.
    m.function("gamepad_triggers", || -> Vec<f64> {
        with_input(|i| vec![
            i.gamepad_left_trigger  as f64,
            i.gamepad_right_trigger as f64,
        ])
    }).build()?;

    // Returns true if the gamepad button at `bit` is pressed.
    // Bit mask: 0=South, 1=East, 2=North, 3=West, 4=LB, 5=RB,
    //           6=Select, 7=Start, 8=DUp, 9=DDown, 10=DLeft, 11=DRight.
    m.function("gamepad_button", |bit: i64| -> bool {
        with_input(|i| (i.gamepad_buttons >> (bit as u32)) & 1 == 1)
    }).build()?;

    // ── Action Map ────────────────────────────────────────────────────────────

    m.function("action_pressed", |name: Ref<str>| -> bool {
        with_input(|i| {
            ACTION_MAP.with(|a| i.action_pressed(&a.borrow(), name.as_ref()))
        })
    }).build()?;

    m.function("action_value", |name: Ref<str>| -> f64 {
        with_input(|i| {
            ACTION_MAP.with(|a| i.action_value(&a.borrow(), name.as_ref()) as f64)
        })
    }).build()?;

    // Adds a key binding to the named action at runtime.
    // binding_str format: "Key:Space", "Key:MouseLeft", "GamepadButton:0",
    //   "GamepadAxis:LeftX:0.5"
    m.function("rebind_action", |name: Ref<str>, binding_str: Ref<str>| {
        let binding = parse_binding_str(binding_str.as_ref());
        if let Some(b) = binding {
            let name_str = name.as_ref().to_string();
            let b_clone = b.clone();
            // Persist into project config so the change survives the next set_action_map call.
            super::settings_module::modify_project_config(|cfg| {
                if let Some(action) = cfg.settings.input.actions.iter_mut().find(|a| a.name == name_str) {
                    action.bindings.push(b_clone);
                }
            });
            // Also update the live thread-local for the current frame.
            ACTION_MAP.with(|a| {
                let mut map = a.borrow_mut();
                if let Some(action) = map.iter_mut().find(|ac| ac.name == name.as_ref()) {
                    action.bindings.push(b);
                }
            });
        }
    }).build()?;

    // Returns [[name, analog_str, binding1, binding2, ...], ...] for all actions.
    m.function("action_list", || -> Vec<Vec<String>> {
        ACTION_MAP.with(|a| {
            a.borrow().iter().map(|ac| {
                let mut row = vec![ac.name.clone(), if ac.analog { "1".into() } else { "0".into() }];
                for b in &ac.bindings { row.push(b.label()); }
                row
            }).collect()
        })
    }).build()?;

    Ok(m)
}

fn parse_binding_str(s: &str) -> Option<InputBinding> {
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    match parts.as_slice() {
        ["Key", code]              => Some(InputBinding::Key { code: code.to_string() }),
        ["GamepadButton", idx]     => idx.parse::<u32>().ok().map(|i| InputBinding::GamepadButton { index: i }),
        ["GamepadAxis", axis, thr] => thr.parse::<f32>().ok().map(|t| InputBinding::GamepadAxis {
            axis: axis.to_string(), threshold: t,
        }),
        _                          => None,
    }
}
