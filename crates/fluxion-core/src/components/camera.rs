// ============================================================
// Camera component
//
// Marks an entity as a camera viewpoint. The renderer uses the
// active camera's Transform + Camera component to build the view
// and projection matrices for each frame.
//
// Only one camera can be active at a time (the first one found
// with `is_active = true`).
// ============================================================

use serde::{Deserialize, Serialize};
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

/// Camera component.
///
/// Attach to an entity that also has a `Transform`. The entity's world
/// position and rotation become the camera's eye position and look direction.
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
pub struct Camera {
    /// Vertical field of view in degrees. Only used for Perspective mode.
    /// Typical values: 60–90. Default: 70.
    #[reflect(range(min = 1.0, max = 180.0))]
    pub fov: f32,

    /// Near clipping plane distance (meters). Objects closer than this are clipped.
    /// Default: 0.1 (10 cm).
    #[reflect(range(min = 0.001, max = 100.0))]
    pub near: f32,

    /// Far clipping plane distance (meters). Objects further than this are clipped.
    /// Default: 1000.0 (1 km).
    #[reflect(range(min = 1.0, max = 100000.0))]
    pub far: f32,

    /// Projection mode. Default: Perspective.
    pub projection_mode: ProjectionMode,

    /// Half-height of the orthographic view volume. Only used in Orthographic mode.
    /// The width is derived from this and the viewport aspect ratio.
    #[reflect(range(min = 0.1, max = 1000.0))]
    pub ortho_size: f32,

    /// If `true`, this camera is the active camera used for rendering the scene.
    /// Set exactly one camera active per scene.
    pub is_active: bool,

    /// If set, render to a texture instead of the screen.
    /// The texture path is a key in the renderer's RenderTarget registry.
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
            is_active:        true,
            render_to_texture: None,
        }
    }

    /// Compute the projection matrix for this camera given the viewport dimensions.
    pub fn projection_matrix(&self, width: u32, height: u32) -> glam::Mat4 {
        let aspect = width as f32 / height.max(1) as f32;
        match self.projection_mode {
            ProjectionMode::Perspective => {
                glam::Mat4::perspective_rh(
                    self.fov.to_radians(),
                    aspect,
                    self.near,
                    self.far,
                )
            }
            ProjectionMode::Orthographic => {
                let half_h = self.ortho_size;
                let half_w = half_h * aspect;
                glam::Mat4::orthographic_rh(
                    -half_w, half_w,
                    -half_h, half_h,
                    self.near, self.far,
                )
            }
        }
    }
}

impl Default for Camera {
    fn default() -> Self { Self::new() }
}

impl Component for Camera {}
