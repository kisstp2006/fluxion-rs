// ============================================================
// fluxion-scripting — FluxionBehaviour JS base class
//
// Injected into the JS global scope before any user scripts load.
// User scripts extend this class to get the Unity-like lifecycle.
//
// This is the JS-side equivalent of FluxionBehaviour.ts from the
// TypeScript engine — the same lifecycle hooks, the same API.
//
// Usage in a user script:
//
//   // my_script.js (or my_script.ts → transpiled to JS)
//   class Spinner extends FluxionBehaviour {
//     constructor() {
//       super();
//       this.speed = 90.0; // degrees per second
//     }
//
//     start() {
//       console.log("Spinner started on entity:", this.entity.name);
//     }
//
//     update(dt) {
//       this.transform.rotation.y += this.speed * dt * Math.PI / 180;
//       this.transform.dirty = true;
//     }
//   }
//
//   // Attach to an entity
//   const cube = world.find("Cube");
//   cube.addScript(new Spinner());
// ============================================================

/// The JavaScript source for the FluxionBehaviour base class.
/// Injected as a global before user scripts load.
pub const FLUXION_BEHAVIOUR_JS: &str = r#"
// ── FluxionBehaviour base class ───────────────────────────────────────────────
// All user scripts extend this. Do not instantiate directly.
class FluxionBehaviour {
    constructor() {
        // Set by the engine when attached to an entity:
        this.entity    = null;  // EntityRef — name, tags, active state
        this.transform = null;  // TransformRef — position, rotation, scale (world + local)
        this.enabled   = true;
        this._started  = false; // tracks whether start() has been called
    }

    // ── Lifecycle hooks — override in your script ──────────────────────────────
    // These match the names from the TypeScript engine's FluxionBehaviour.

    /** Called once before the first update(). Initialize your state here. */
    start() {}

    /** Called every frame. dt = delta time in seconds. */
    update(dt) {}

    /** Called every frame after all update() calls. */
    lateUpdate(dt) {}

    /** Called at a fixed rate (default 60 Hz) for physics integration. */
    fixedUpdate(dt) {}

    /** Called when this component is enabled. */
    onEnable() {}

    /** Called when this component is disabled. */
    onDisable() {}

    /** Called when the entity is destroyed. Clean up resources here. */
    onDestroy() {}

    // ── Internal tick (called by the engine — do not override) ────────────────
    _tick(dt) {
        if (!this.enabled) return;
        if (!this._started) {
            this._started = true;
            try { this.start(); } catch(e) { console.error("start() error:", e); }
        }
        try { this.update(dt); } catch(e) { console.error("update() error:", e); }
    }

    _lateTick(dt) {
        if (!this.enabled || !this._started) return;
        try { this.lateUpdate(dt); } catch(e) { console.error("lateUpdate() error:", e); }
    }

    _fixedTick(dt) {
        if (!this.enabled) return;
        try { this.fixedUpdate(dt); } catch(e) { console.error("fixedUpdate() error:", e); }
    }
}

// ── Script registry ──────────────────────────────────────────────────────────
// The engine calls __fluxion_tick(dt) each frame.
// Scripts registered with __fluxion_register() are ticked automatically.

const __behaviours = [];

function __fluxion_register(behaviour) {
    __behaviours.push(behaviour);
}

function __fluxion_unregister(behaviour) {
    const idx = __behaviours.indexOf(behaviour);
    if (idx !== -1) __behaviours.splice(idx, 1);
}

function __fluxion_tick(dt) {
    for (const b of __behaviours) {
        b._tick(dt);
    }
    for (const b of __behaviours) {
        b._lateTick(dt);
    }
}

function __fluxion_fixed_tick(dt) {
    for (const b of __behaviours) {
        b._fixedTick(dt);
    }
}

// ── Math helpers (mirrors the TS engine's injected shortcuts) ─────────────────
const Mathf = {
    PI:     Math.PI,
    TAU:    Math.PI * 2,
    DEG2RAD: Math.PI / 180,
    RAD2DEG: 180 / Math.PI,
    lerp:   (a, b, t) => a + (b - a) * t,
    clamp:  (v, lo, hi) => Math.max(lo, Math.min(hi, v)),
    sin:    Math.sin,
    cos:    Math.cos,
    sqrt:   Math.sqrt,
    abs:    Math.abs,
    floor:  Math.floor,
    ceil:   Math.ceil,
    round:  Math.round,
};
"#;

/// Inject the FluxionBehaviour base class and math helpers into the VM.
pub fn inject_base_classes(vm: &crate::vm::JsVm) -> anyhow::Result<()> {
    vm.eval(FLUXION_BEHAVIOUR_JS, "<fluxion-behaviour-stdlib>")
}
