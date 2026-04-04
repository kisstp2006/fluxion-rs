// ============================================================
// fluxion-core — Input state (platform-agnostic)
//
// Frame snapshot model: winit / browser backends write into this struct;
// game code and script bindings read the same snapshot for the tick.
//
// Key strings follow the same convention as the JS `KeyboardEvent.code`
// values used in FluxionJS (e.g. "KeyW", "Space", "ArrowLeft").
// ============================================================

use std::collections::HashSet;

/// Per-frame input snapshot. Reset deltas via [`InputState::begin_frame`] at tick start.
#[derive(Debug, Clone)]
pub struct InputState {
    keys_down: HashSet<String>,
    mouse_position: (f32, f32),
    mouse_delta: (f32, f32),
    last_mouse_position: Option<(f32, f32)>,
    scroll_delta: (f32, f32),
    mouse_left: bool,
    mouse_middle: bool,
    mouse_right: bool,

    /// First connected gamepad (gilrs / winit backends fill this each frame).
    pub gamepad_connected: bool,
    pub gamepad_left_stick:  (f32, f32),
    pub gamepad_right_stick: (f32, f32),
    pub gamepad_left_trigger:  f32,
    pub gamepad_right_trigger: f32,
    /// Bitmask: bit0 South, 1 East, 2 North, 3 West, 4 LB, 5 RB, 6 Select, 7 Start,
    /// 8 DUp, 9 DDown, 10 DLeft, 11 DRight.
    pub gamepad_buttons: u32,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            keys_down: HashSet::new(),
            mouse_position: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            last_mouse_position: None,
            scroll_delta: (0.0, 0.0),
            mouse_left: false,
            mouse_middle: false,
            mouse_right: false,
            gamepad_connected: false,
            gamepad_left_stick:  (0.0, 0.0),
            gamepad_right_stick: (0.0, 0.0),
            gamepad_left_trigger:  0.0,
            gamepad_right_trigger: 0.0,
            gamepad_buttons: 0,
        }
    }
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call once at the beginning of each frame (before processing queued events or after, depending on your loop;
    /// sandbox uses: begin_frame → winit drains events → scripts read).
    pub fn begin_frame(&mut self) {
        self.mouse_delta = (0.0, 0.0);
        self.scroll_delta = (0.0, 0.0);
    }

    /// Reset gamepad snapshot (call at frame start; backend overwrites if a pad is active).
    pub fn clear_gamepad(&mut self) {
        self.gamepad_connected = false;
        self.gamepad_left_stick = (0.0, 0.0);
        self.gamepad_right_stick = (0.0, 0.0);
        self.gamepad_left_trigger = 0.0;
        self.gamepad_right_trigger = 0.0;
        self.gamepad_buttons = 0;
    }

    pub fn set_gamepad_snapshot(
        &mut self,
        left_stick: (f32, f32),
        right_stick: (f32, f32),
        left_trigger: f32,
        right_trigger: f32,
        buttons: u32,
    ) {
        self.gamepad_connected = true;
        self.gamepad_left_stick = left_stick;
        self.gamepad_right_stick = right_stick;
        self.gamepad_left_trigger = left_trigger.clamp(0.0, 1.0);
        self.gamepad_right_trigger = right_trigger.clamp(0.0, 1.0);
        self.gamepad_buttons = buttons;
    }

    pub fn is_key_down(&self, code: &str) -> bool {
        self.keys_down.contains(code)
    }

    pub fn set_key_down(&mut self, code: &str, down: bool) {
        if down {
            self.keys_down.insert(code.to_string());
        } else {
            self.keys_down.remove(code);
        }
    }

    pub fn set_mouse_position(&mut self, x: f32, y: f32) {
        if let Some((lx, ly)) = self.last_mouse_position {
            self.mouse_delta.0 += x - lx;
            self.mouse_delta.1 += y - ly;
        }
        self.last_mouse_position = Some((x, y));
        self.mouse_position = (x, y);
    }

    pub fn add_mouse_delta(&mut self, dx: f32, dy: f32) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    pub fn add_scroll(&mut self, dx: f32, dy: f32) {
        self.scroll_delta.0 += dx;
        self.scroll_delta.1 += dy;
    }

    pub fn set_mouse_button(&mut self, left: bool, middle: bool, right: bool) {
        self.mouse_left = left;
        self.mouse_middle = middle;
        self.mouse_right = right;
    }

    pub fn mouse_position(&self) -> (f32, f32) {
        self.mouse_position
    }

    pub fn mouse_delta(&self) -> (f32, f32) {
        self.mouse_delta
    }

    pub fn scroll_delta(&self) -> (f32, f32) {
        self.scroll_delta
    }

    /// Unity-style `Input.GetAxis` for two opposing keys (−1..1).
    pub fn get_axis(&self, negative_code: &str, positive_code: &str) -> f32 {
        let neg = self.is_key_down(negative_code) as i32 as f32;
        let pos = self.is_key_down(positive_code) as i32 as f32;
        pos - neg
    }

    /// Default WASD horizontal (A = −1, D = +1).
    pub fn axis_horizontal(&self) -> f32 {
        self.get_axis("KeyA", "KeyD")
    }

    /// Default WASD vertical (S = −1, W = +1) to match Unity `vertical` with W forward.
    pub fn axis_vertical(&self) -> f32 {
        self.get_axis("KeyS", "KeyW")
    }

    /// Iterator for syncing to JS: (code, down) for all currently held keys.
    pub fn keys_down_iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.keys_down.iter().map(|s| s.as_str())
    }

    pub fn mouse_left(&self) -> bool {
        self.mouse_left
    }

    pub fn mouse_middle(&self) -> bool {
        self.mouse_middle
    }

    pub fn mouse_right(&self) -> bool {
        self.mouse_right
    }
}
