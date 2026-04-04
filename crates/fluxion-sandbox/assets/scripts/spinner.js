// ============================================================
// spinner.js — FluxionRS demo script
//
// Local Y rotation via quaternion (matches glam Quat xyzw). Euler round-trip
// was removed on the Rust bridge to avoid jitter / stepped motion.
// ============================================================

function quat_mul(a, b) {
    const ax = a[0], ay = a[1], az = a[2], aw = a[3];
    const bx = b[0], by = b[1], bz = b[2], bw = b[3];
    return [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ];
}

function quat_normalize(q) {
    const x = q[0], y = q[1], z = q[2], w = q[3];
    const l = Math.sqrt(x * x + y * y + z * z + w * w);
    if (l < 1e-10) return [0, 0, 0, 1];
    return [x / l, y / l, z / l, w / l];
}

/** Quaternion for rotation of `angle` radians around +Y (xyzw, same as glam). */
function quat_rotate_y(angle) {
    const h = angle * 0.5;
    return [0, Math.sin(h), 0, Math.cos(h)];
}

class Spinner extends FluxionBehaviour {
    constructor() {
        super();
        this.speed = 90.0; // degrees per second
    }

    start() {
        if (!this.transform) {
            console.error(
                "Spinner: no transform — use __fluxion_register(spinner, \"EntityName\") with a matching name"
            );
            return;
        }
        console.log("Spinner script started — rotating at " + this.speed + " deg/s");
    }

    update(dt) {
        if (!this.transform) return;
        const r = this.transform.rotation;
        if (!Array.isArray(r) || r.length !== 4) {
            console.error("Spinner: expected transform.rotation as [x,y,z,w] quaternion");
            return;
        }
        const dq = quat_rotate_y(this.speed * Mathf.DEG2RAD * dt);
        this.transform.rotation = quat_normalize(quat_mul(r, dq));
    }
}

const spinner = new Spinner();
__fluxion_register(spinner, "Cube");

console.log("spinner.js loaded — Spinner on \"Cube\" (quat Y spin)");
