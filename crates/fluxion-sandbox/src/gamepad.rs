//! Poll the first connected gamepad via gilrs (native only).

#[cfg(not(target_arch = "wasm32"))]
use gilrs::{Axis, Button, Gilrs};

use fluxion_core::InputState;

#[cfg(not(target_arch = "wasm32"))]
pub fn poll_gamepad(input: &mut InputState, gil: &mut Option<Gilrs>) {
    let Some(g) = gil.as_mut() else {
        input.clear_gamepad();
        return;
    };

    while g.next_event().is_some() {}

    input.clear_gamepad();

    for (_id, gp) in g.gamepads() {
        if !gp.is_connected() {
            continue;
        }

        let lx = gp.value(Axis::LeftStickX);
        let ly = gp.value(Axis::LeftStickY);
        let rx = gp.value(Axis::RightStickX);
        let ry = gp.value(Axis::RightStickY);
        let lt = gp.value(Axis::LeftZ).max(0.0);
        let rt = gp.value(Axis::RightZ).max(0.0);

        let mut bits = 0u32;
        if gp.is_pressed(Button::South) {
            bits |= 1 << 0;
        }
        if gp.is_pressed(Button::East) {
            bits |= 1 << 1;
        }
        if gp.is_pressed(Button::North) {
            bits |= 1 << 2;
        }
        if gp.is_pressed(Button::West) {
            bits |= 1 << 3;
        }
        if gp.is_pressed(Button::LeftTrigger) {
            bits |= 1 << 4;
        }
        if gp.is_pressed(Button::RightTrigger) {
            bits |= 1 << 5;
        }
        if gp.is_pressed(Button::Select) {
            bits |= 1 << 6;
        }
        if gp.is_pressed(Button::Start) {
            bits |= 1 << 7;
        }
        if gp.is_pressed(Button::DPadUp) {
            bits |= 1 << 8;
        }
        if gp.is_pressed(Button::DPadDown) {
            bits |= 1 << 9;
        }
        if gp.is_pressed(Button::DPadLeft) {
            bits |= 1 << 10;
        }
        if gp.is_pressed(Button::DPadRight) {
            bits |= 1 << 11;
        }

        input.set_gamepad_snapshot((lx, ly), (rx, ry), lt, rt, bits);
        return;
    }
}

#[cfg(target_arch = "wasm32")]
pub fn poll_gamepad(_input: &mut InputState, _gil: &mut ()) {}
