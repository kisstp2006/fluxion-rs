// ============================================================
// fluxion-core — Time
//
// Tracks frame timing and drives the fixed-timestep physics loop.
// Ported from Time.ts in the TypeScript engine.
//
// Usage in the main loop:
//
//   let (fixed_steps, dt) = time.tick();
//   for _ in 0..fixed_steps {
//       // fixed update (physics, deterministic systems)
//       physics_system.step(time.fixed_dt);
//   }
//   // variable update (rendering, scripts)
//   scripts.update(dt);
//
// The fixed accumulator algorithm:
//   accumulator += raw_dt (capped at MAX_DELTA to avoid "spiral of death")
//   while accumulator >= fixed_dt:
//     fixed_step_count += 1
//     accumulator -= fixed_dt
//   alpha = accumulator / fixed_dt  (interpolation factor for rendering)
//
// "Spiral of death": if frames take longer than MAX_DELTA seconds,
// we cap the delta so physics doesn't try to simulate 100 steps in
// one frame and make things worse. Matches TS engine behavior.
// ============================================================

use std::time::Instant;

/// Maximum delta time (seconds) accepted per frame.
/// If a frame takes longer than this (e.g. debugger pause), we clamp it.
/// Prevents the "spiral of death" where long frames cause more physics steps
/// which cause longer frames etc.
const MAX_DELTA: f32 = 0.25;

/// Number of fixed steps per second.
const DEFAULT_FIXED_RATE: f32 = 60.0;

/// Maximum fixed steps per frame. Safety valve against runaway accumulation.
const MAX_FIXED_STEPS: u32 = 8;

/// Frame timing and fixed-timestep accumulator.
///
/// Create one per engine, tick it every frame.
pub struct Time {
    // ── Fixed timestep ─────────────────────────────────────────────────────────
    /// Duration of each fixed update step, in seconds. Default: 1/60 ≈ 16.67ms.
    pub fixed_dt: f32,

    /// Accumulator for the fixed-timestep algorithm (seconds).
    accumulator: f32,

    // ── Variable timestep ──────────────────────────────────────────────────────
    /// Elapsed time since the previous frame, after time scale (seconds).
    pub dt: f32,

    /// Elapsed time since the previous frame, ignoring time scale (seconds).
    pub unscaled_dt: f32,

    // ── Time scale ─────────────────────────────────────────────────────────────
    /// Multiply all variable deltas by this factor.
    /// 1.0 = real time, 0.5 = half speed, 0.0 = paused.
    /// Does NOT affect fixed_dt (physics should remain deterministic).
    pub time_scale: f32,

    // ── Totals ─────────────────────────────────────────────────────────────────
    /// Total scaled time since engine start (seconds).
    pub elapsed: f32,

    /// Total unscaled time since engine start (seconds).
    pub unscaled_elapsed: f32,

    // ── FPS tracking ───────────────────────────────────────────────────────────
    /// Instantaneous frames per second (1 / unscaled_dt).
    pub fps: f32,

    /// Exponentially smoothed FPS. Less jittery than raw fps.
    pub smooth_fps: f32,

    /// Number of frames rendered since engine start.
    pub frame_count: u64,

    // ── Rendering interpolation ────────────────────────────────────────────────
    /// Interpolation factor [0..1] for lerping physics state between fixed steps.
    /// Use this to smooth rendered positions: pos_render = lerp(pos_prev, pos_curr, alpha).
    pub fixed_alpha: f32,

    // ── Internal ───────────────────────────────────────────────────────────────
    #[cfg(not(target_arch = "wasm32"))]
    last_instant: Instant,

    #[cfg(target_arch = "wasm32")]
    last_timestamp_ms: f64,
}

impl Time {
    pub fn new() -> Self {
        Self {
            fixed_dt:         1.0 / DEFAULT_FIXED_RATE,
            accumulator:      0.0,
            dt:               0.0,
            unscaled_dt:      0.0,
            time_scale:       1.0,
            elapsed:          0.0,
            unscaled_elapsed: 0.0,
            fps:              0.0,
            smooth_fps:       60.0,
            frame_count:      0,
            fixed_alpha:      0.0,

            #[cfg(not(target_arch = "wasm32"))]
            last_instant: Instant::now(),

            #[cfg(target_arch = "wasm32")]
            last_timestamp_ms: 0.0,
        }
    }

    /// Set the fixed timestep rate. For example, 120.0 = 120 physics steps per second.
    pub fn set_fixed_rate(&mut self, hz: f32) {
        assert!(hz > 0.0, "Fixed rate must be positive");
        self.fixed_dt = 1.0 / hz;
    }

    /// Advance time by one frame. Call this at the start of each game loop iteration.
    ///
    /// Returns `(fixed_step_count, variable_dt)`:
    ///   - `fixed_step_count`: number of fixed updates to run this frame
    ///   - `variable_dt`:      scaled delta time for variable-rate systems
    ///
    /// # Example
    /// ```rust
    /// let (fixed_steps, dt) = time.tick();
    /// for _ in 0..fixed_steps {
    ///     physics.step(time.fixed_dt);
    /// }
    /// scripts.update(dt);
    /// ```
    #[cfg(not(target_arch = "wasm32"))]
    pub fn tick(&mut self) -> (u32, f32) {
        let now     = Instant::now();
        let raw_dt  = now.duration_since(self.last_instant).as_secs_f32();
        self.last_instant = now;
        self.advance(raw_dt)
    }

    /// WASM variant: caller passes the high-resolution timestamp from
    /// `performance.now()` (milliseconds).
    #[cfg(target_arch = "wasm32")]
    pub fn tick_wasm(&mut self, timestamp_ms: f64) -> (u32, f32) {
        let raw_dt = if self.last_timestamp_ms == 0.0 {
            0.0
        } else {
            ((timestamp_ms - self.last_timestamp_ms) / 1000.0) as f32
        };
        self.last_timestamp_ms = timestamp_ms;
        self.advance(raw_dt)
    }

    /// Internal: advance internal state by `raw_dt` seconds.
    fn advance(&mut self, raw_dt: f32) -> (u32, f32) {
        // Cap to prevent spiral of death
        let clamped_dt = raw_dt.min(MAX_DELTA);

        self.unscaled_dt      = clamped_dt;
        self.dt               = clamped_dt * self.time_scale;
        self.unscaled_elapsed += clamped_dt;
        self.elapsed          += self.dt;
        self.frame_count      += 1;

        // FPS tracking
        if clamped_dt > 0.0 {
            self.fps       = 1.0 / clamped_dt;
            // Exponential moving average: alpha = 0.1 gives smooth result
            self.smooth_fps = self.smooth_fps * 0.9 + self.fps * 0.1;
        }

        // Fixed timestep accumulation
        self.accumulator += clamped_dt;
        let mut fixed_steps = 0u32;
        while self.accumulator >= self.fixed_dt && fixed_steps < MAX_FIXED_STEPS {
            self.accumulator -= self.fixed_dt;
            fixed_steps      += 1;
        }

        // Interpolation factor for rendering (blend between last and next physics state)
        self.fixed_alpha = self.accumulator / self.fixed_dt;

        (fixed_steps, self.dt)
    }
}

impl Default for Time {
    fn default() -> Self { Self::new() }
}
