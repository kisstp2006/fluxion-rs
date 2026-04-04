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

use std::cell::Cell;
use std::ptr::NonNull;

use rune::{Module, ContextError};
use fluxion_core::InputState;

// ── Thread-local pointer ──────────────────────────────────────────────────────

thread_local! {
    static INPUT_PTR: Cell<Option<NonNull<InputState>>> = Cell::new(None);
}

pub fn set_input_context(input: &mut InputState) {
    INPUT_PTR.with(|c| c.set(Some(NonNull::from(input))));
}

pub fn clear_input_context() {
    INPUT_PTR.with(|c| c.set(None));
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

    Ok(m)
}
