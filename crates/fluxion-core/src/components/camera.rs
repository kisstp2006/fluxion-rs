// ============================================================
// Camera component
//
// Marks an entity as a camera viewpoint. The renderer uses the
// active camera's Transform + Camera component to build the view
// and projection matrices for each frame.
//
// Only one camera can be active at a time (the first one found
// with `is_active = true` and the lowest `depth` value).
// ============================================================

use serde::{Deserialize, Serialize};
use glam::{Mat4, Vec3, Vec4};
use fluxion_reflect_derive::Reflect;

use crate::ecs::component::Component;

/// Whether the camera uses perspective (3D) or orthographic (2D / UI) projection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ProjectionMode {
    /// Standard 3D perspective projection.
    Perspective,
    /// Parallel orthographic projection (isometric games, 2D, UI cameras).
    Orthographic,
}

/// How the camera clears the render target before drawing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ClearFlags {
    /// Clear with the skybox / sky gradient (default).
    Skybox,
    /// Clear with a solid `background_color`.
    SolidColor,
    /// Clear depth only; colour is preserved from the previous camera.
    DepthOnly,
    /// Don't clear anything (useful for overlay cameras).
    Nothing,
}

impl Default for ClearFlags {
    fn default() -> Self { Self::Skybox }
}

impl ClearFlags {
    pub fn from_str(s: &str) -> Self {
        match s {
            "SolidColor" => Self::SolidColor,
            "DepthOnly"  => Self::DepthOnly,
            "Nothing"    => Self::Nothing,
            _            => Self::Skybox,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skybox     => "Skybox",
            Self::SolidColor => "SolidColor",
            Self::DepthOnly  => "DepthOnly",
            Self::Nothing    => "Nothing",
        }
    }
}

/// Camera component.
///
/// Attach to an entity that also has a `Transform`. The entity's world
/// position and rotation become the camera's eye position and look direction.
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
pub struct Camera {
    // ── Projection ────────────────────────────────────────────────────────────

    /// Vertical field of view in degrees. Only used for Perspective mode.
    /// Typical values: 60–90. Default: 70.
    #[reflect(range(min = 1.0, max = 180.0))]
    pub fov: f32,

    /// Near clipping plane distance (meters). Default: 0.1.
    #[reflect(range(min = 0.001, max = 100.0))]
    pub near: f32,

    /// Far clipping plane distance (meters). Default: 1000.0.
    #[reflect(range(min = 1.0, max = 100000.0))]
    pub far: f32,

    /// Projection mode. Default: Perspective.
    pub projection_mode: ProjectionMode,

    /// Half-height of the orthographic view volume (Orthographic mode only).
    #[reflect(range(min = 0.1, max = 1000.0))]
    pub ortho_size: f32,

    // ── Culling & ordering ────────────────────────────────────────────────────

    /// Layer bitmask: only entities whose `(1 << layer)` bit is set are rendered.
    /// Default: `0xFFFF_FFFF` (all layers).
    pub culling_mask: u32,

    /// Render order. Cameras with lower depth render first (background).
    /// Default: 0.
    pub depth: i32,

    // ── Clear ─────────────────────────────────────────────────────────────────

    /// How the camera clears the render target each frame.
    pub clear_flags: ClearFlags,

    /// Solid background colour used when `clear_flags == SolidColor`.
    /// RGBA, linear, 0–1.
    pub background_color: [f32; 4],

    // ── Viewport ──────────────────────────────────────────────────────────────

    /// Normalized screen rectangle `[x, y, width, height]` (0–1).
    /// Default: `[0, 0, 1, 1]` (full screen).
    pub viewport_rect: [f32; 4],

    // ── Quality flags ─────────────────────────────────────────────────────────

    /// Allow HDR render target. Default: true.
    pub allow_hdr: bool,

    /// Allow MSAA antialiasing. Default: false.
    pub allow_msaa: bool,

    // ── Physical camera ───────────────────────────────────────────────────────

    /// When true, FOV is derived from `focal_length` + `sensor_size`.
    pub use_physical: bool,

    /// Focal length in millimetres. Default: 50 mm.
    #[reflect(range(min = 1.0, max = 300.0))]
    pub focal_length: f32,

    /// Sensor size `[width, height]` in millimetres. Default: full-frame 36×24.
    pub sensor_size: [f32; 2],

    /// Lens shift `[x, y]` as a fraction of the sensor size. Default: [0, 0].
    pub lens_shift: [f32; 2],

    // ── Custom projection ─────────────────────────────────────────────────────

    /// Override the projection matrix entirely.  Row-major 4×4.
    /// When set, all other projection fields are ignored.
    #[serde(skip)]
    #[reflect(skip)]
    pub custom_projection: Option<[[f32; 4]; 4]>,

    // ── Misc ──────────────────────────────────────────────────────────────────

    /// If `true`, this camera renders the scene this frame.
    pub is_active: bool,

    /// If set, render to a named render-texture instead of the screen.
    pub render_to_texture: Option<String>,
}

impl Camera {
    /// Create a standard perspective camera with sensible defaults.
    pub fn new() -> Self {
        Camera {
            fov:              70.0,
            near:             0.1,
            far:              1000.0,
            projection_mode:  ProjectionMode::Perspective,
            ortho_size:       5.0,
            culling_mask:     0xFFFF_FFFF,
            depth:            0,
            clear_flags:      ClearFlags::Skybox,
            background_color: [0.1, 0.1, 0.1, 1.0],
            viewport_rect:    [0.0, 0.0, 1.0, 1.0],
            allow_hdr:        true,
            allow_msaa:       false,
            use_physical:     false,
            focal_length:     50.0,
            sensor_size:      [36.0, 24.0],
            lens_shift:       [0.0, 0.0],
            custom_projection: None,
            is_active:        true,
            render_to_texture: None,
        }
    }

    // ── Projection helpers ────────────────────────────────────────────────────

    /// Compute the projection matrix, respecting `custom_projection` → physical → standard.
    pub fn projection_matrix_ex(&self, width: u32, height: u32) -> Mat4 {
        if let Some(m) = self.custom_projection {
            return Mat4::from_cols_array_2d(&m);
        }
        let aspect = width as f32 / height.max(1) as f32;
        match self.projection_mode {
            ProjectionMode::Perspective => {
                let fov_rad = if self.use_physical {
                    // Derive vertical FOV from focal length + sensor height.
                    2.0 * (self.sensor_size[1] / (2.0 * self.focal_length)).atan()
                } else {
                    self.fov.to_radians()
                };
                let proj = Mat4::perspective_rh(fov_rad, aspect, self.near, self.far);
                // Apply lens shift as a post-projection translation.
                let shift_x = self.lens_shift[0] * 2.0;
                let shift_y = self.lens_shift[1] * 2.0;
                if shift_x == 0.0 && shift_y == 0.0 {
                    proj
                } else {
                    let shift = Mat4::from_translation(glam::Vec3::new(shift_x, shift_y, 0.0));
                    shift * proj
                }
            }
            ProjectionMode::Orthographic => {
                let half_h = self.ortho_size;
                let half_w = half_h * aspect;
                Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, self.near, self.far)
            }
        }
    }

    /// Legacy alias — uses `projection_matrix_ex` internally.
    pub fn projection_matrix(&self, width: u32, height: u32) -> Mat4 {
        self.projection_matrix_ex(width, height)
    }

    // ── Screen ↔ World conversions ────────────────────────────────────────────

    /// Convert a screen-space point `(px, py)` and a linear depth value (0 = near, 1 = far)
    /// into a world-space position.
    ///
    /// `vp_w` / `vp_h` are the actual pixel dimensions of the viewport.
    pub fn screen_to_world(
        screen_x:     f32,
        screen_y:     f32,
        depth:        f32,
        inv_view_proj: Mat4,
        vp_w:         u32,
        vp_h:         u32,
    ) -> Vec3 {
        let ndc_x =  (screen_x / vp_w as f32) * 2.0 - 1.0;
        let ndc_y = -(screen_y / vp_h as f32) * 2.0 + 1.0;
        let ndc_z = depth * 2.0 - 1.0;
        let clip  = Vec4::new(ndc_x, ndc_y, ndc_z, 1.0);
        let world = inv_view_proj * clip;
        world.truncate() / world.w
    }

    /// Project a world-space point into screen space.
    ///
    /// Returns `(screen_x, screen_y, depth_01)`.  
    /// `depth_01 < 0` means the point is behind the camera.
    pub fn world_to_screen(
        world_pos: Vec3,
        view_proj: Mat4,
        vp_w:      u32,
        vp_h:      u32,
    ) -> Vec3 {
        let clip = view_proj * Vec4::new(world_pos.x, world_pos.y, world_pos.z, 1.0);
        if clip.w.abs() < 1e-7 {
            return Vec3::ZERO;
        }
        let ndc = clip.truncate() / clip.w;
        Vec3::new(
            (ndc.x * 0.5 + 0.5) * vp_w  as f32,
            (1.0 - (ndc.y * 0.5 + 0.5)) * vp_h as f32,
            ndc.z * 0.5 + 0.5,
        )
    }

    /// Generate a world-space ray from a screen pixel.
    ///
    /// Returns `(ray_origin, ray_direction)` (direction is normalized).
    pub fn screen_point_to_ray(
        screen_x:     f32,
        screen_y:     f32,
        inv_view_proj: Mat4,
        cam_pos:       Vec3,
        vp_w:          u32,
        vp_h:          u32,
    ) -> (Vec3, Vec3) {
        let world_near = Self::screen_to_world(screen_x, screen_y, 0.0, inv_view_proj, vp_w, vp_h);
        let world_far  = Self::screen_to_world(screen_x, screen_y, 1.0, inv_view_proj, vp_w, vp_h);
        let dir = (world_far - world_near).normalize_or_zero();
        (cam_pos, dir)
    }
}

impl Default for Camera {
    fn default() -> Self { Self::new() }
}

impl Component for Camera {}
