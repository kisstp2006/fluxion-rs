// ============================================================
// spinner.js — FluxionRS demo script
//
// Rotates the "Cube" entity at a configurable speed.
// This script demonstrates the FluxionBehaviour lifecycle API.
//
// The sandbox loads this file automatically if it exists at
// assets/scripts/spinner.js relative to the working directory.
// ============================================================

class Spinner extends FluxionBehaviour {
    constructor() {
        super();
        this.speed = 90.0; // degrees per second
    }

    start() {
        console.log("Spinner script started — rotating at " + this.speed + " deg/s");
    }

    update(dt) {
        // Rotate around Y axis at `speed` degrees per second.
        // Transform rotation is stored as radians internally.
        if (this.transform) {
            this.transform.rotationY = (this.transform.rotationY || 0) + this.speed * Mathf.DEG2RAD * dt;
        }
    }
}

// Register and attach to the cube entity via the global registry.
// In the full engine, you would do: world.find("Cube").addScript(new Spinner())
// For the Phase 1 sandbox we register directly since world bindings are not yet wired.
const spinner = new Spinner();
__fluxion_register(spinner);

console.log("spinner.js loaded — Spinner registered");
