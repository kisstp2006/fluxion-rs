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
use serde::{Deserialize, Serialize};

// ── Input Action Map ──────────────────────────────────────────────────────────

/// One concrete binding that can trigger an `InputAction`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputBinding {
    /// Keyboard key code string (e.g. "Space", "KeyW").
    Key { code: String },
    /// Gamepad button bit-index (0=South … 11=DRight).
    GamepadButton { index: u32 },
    /// Gamepad axis: "LeftX", "LeftY", "RightX", "RightY", "LT", "RT".
    /// Positive threshold fires the action; negative threshold gives −1 for analog.
    GamepadAxis { axis: String, threshold: f32 },
}

impl InputBinding {
    /// Human-readable label used in the settings UI.
    pub fn label(&self) -> String {
        match self {
            InputBinding::Key { code }                   => code.clone(),
            InputBinding::GamepadButton { index }        => format!("Button {}", index),
            InputBinding::GamepadAxis { axis, threshold } =>
                format!("Axis {} ({}{})", axis,
                    if *threshold >= 0.0 { "+" } else { "" }, threshold),
        }
    }
}

/// A named logical input action (e.g. "Jump", "Fire", "MoveHorizontal").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputAction {
    /// Logical name used in scripts (`action_pressed("Jump")`).
    pub name:     String,
    /// Bindings that trigger this action.
    pub bindings: Vec<InputBinding>,
    /// If true, returns an analog value in [-1, 1] via `action_value()`.
    /// If false, `action_pressed()` returns a bool.
    pub analog:   bool,
}

impl InputAction {
    pub fn new_digital(name: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            name:     name.into(),
            bindings: vec![InputBinding::Key { code: key.into() }],
            analog:   false,
        }
    }
    pub fn new_analog(name: impl Into<String>) -> Self {
        Self { name: name.into(), bindings: Vec::new(), analog: true }
    }
}

/// Default built-in actions added to every new project.
pub fn default_input_actions() -> Vec<InputAction> {
    vec![
        InputAction::new_digital("Jump",   "Space"),
        InputAction::new_digital("Fire",   "MouseLeft"),
        InputAction::new_digital("Sprint", "ShiftLeft"),
        InputAction::new_digital("Pause",  "Escape"),
        InputAction { name: "MoveHorizontal".into(), bindings: vec![
            InputBinding::Key { code: "KeyA".into() },
            InputBinding::Key { code: "KeyD".into() },
        ], analog: true },
        InputAction { name: "MoveVertical".into(), bindings: vec![
            InputBinding::Key { code: "KeyS".into() },
            InputBinding::Key { code: "KeyW".into() },
        ], analog: true },
    ]
}

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

    /// Returns an iterator over all currently-held key codes.
    /// Used by the editor to push the held set into the Rune `INPUT_SNAPSHOT_CELL`.
    pub fn held_keys(&self) -> impl Iterator<Item = &str> {
        self.keys_down.iter().map(|s| s.as_str())
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

    // ── Action Map evaluation ──────────────────────────────────────────────────

    /// Returns true if any binding of the named action is currently active.
    pub fn action_pressed(&self, actions: &[InputAction], name: &str) -> bool {
        let Some(action) = actions.iter().find(|a| a.name == name) else { return false };
        for b in &action.bindings {
            if self.eval_binding_digital(b) { return true; }
        }
        false
    }

    /// Returns the analog value [-1, 1] for the named action.
    /// For digital actions this is 0.0 or 1.0.
    pub fn action_value(&self, actions: &[InputAction], name: &str) -> f32 {
        let Some(action) = actions.iter().find(|a| a.name == name) else { return 0.0 };
        if !action.analog {
            return if self.action_pressed(actions, name) { 1.0 } else { 0.0 };
        }
        // Analog: for paired keys the convention is [negative_key, positive_key].
        let mut val = 0.0f32;
        for (i, b) in action.bindings.iter().enumerate() {
            let sign = if i % 2 == 0 { -1.0f32 } else { 1.0f32 };
            if self.eval_binding_digital(b) { val += sign; }
        }
        val.clamp(-1.0, 1.0)
    }

    fn eval_binding_digital(&self, b: &InputBinding) -> bool {
        match b {
            InputBinding::Key { code } => {
                if code == "MouseLeft"   { return self.mouse_left; }
                if code == "MouseMiddle" { return self.mouse_middle; }
                if code == "MouseRight"  { return self.mouse_right; }
                self.is_key_down(code)
            }
            InputBinding::GamepadButton { index } => {
                self.gamepad_connected && (self.gamepad_buttons >> index) & 1 == 1
            }
            InputBinding::GamepadAxis { axis, threshold } => {
                let v = self.gamepad_axis_value(axis);
                if *threshold >= 0.0 { v >= *threshold } else { v <= *threshold }
            }
        }
    }

    fn gamepad_axis_value(&self, axis: &str) -> f32 {
        match axis {
            "LeftX"  => self.gamepad_left_stick.0,
            "LeftY"  => self.gamepad_left_stick.1,
            "RightX" => self.gamepad_right_stick.0,
            "RightY" => self.gamepad_right_stick.1,
            "LT"     => self.gamepad_left_trigger,
            "RT"     => self.gamepad_right_trigger,
            _        => 0.0,
        }
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
