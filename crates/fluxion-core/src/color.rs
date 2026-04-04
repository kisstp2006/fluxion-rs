// ============================================================
// fluxion-core — Color enum
//
// Engine-wide named color presets + Custom(r, g, b, a) variant.
// All values are linear RGBA in [0, 1].
//
// Unity equivalents: Color.red, Color.green, Color.white, …
// ============================================================

use glam::Vec4;

/// Engine-wide color type.
///
/// Named variants are linear RGBA presets; use [`Color::Custom`] for
/// arbitrary values.  Convert to raw arrays / Vec4 via [`Color::rgba`]
/// / [`Color::vec4`].
///
/// # Example
/// ```rust
/// use fluxion_core::Color;
/// let c: [f32; 4] = Color::Red.rgba();
/// let v: glam::Vec4 = Color::Cyan.with_alpha(0.5).vec4();
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Color {
    White,
    Black,
    Gray,
    Red,
    Green,
    Blue,
    Yellow,
    Cyan,
    Magenta,
    Orange,
    Purple,
    Lime,
    Pink,
    Aqua,
    Maroon,
    /// Arbitrary linear RGBA value.
    Custom(f32, f32, f32, f32),
}

impl Color {
    /// Convert to a linear `[r, g, b, a]` array.
    #[inline]
    pub fn rgba(self) -> [f32; 4] {
        match self {
            Color::White   => [1.00, 1.00, 1.000, 1.0],
            Color::Black   => [0.00, 0.00, 0.000, 1.0],
            Color::Gray    => [0.50, 0.50, 0.500, 1.0],
            Color::Red     => [1.00, 0.00, 0.000, 1.0],
            Color::Green   => [0.00, 0.80, 0.000, 1.0],
            Color::Blue    => [0.00, 0.00, 1.000, 1.0],
            Color::Yellow  => [1.00, 0.92, 0.016, 1.0],
            Color::Cyan    => [0.00, 1.00, 1.000, 1.0],
            Color::Magenta => [1.00, 0.00, 1.000, 1.0],
            Color::Orange  => [1.00, 0.50, 0.000, 1.0],
            Color::Purple  => [0.50, 0.00, 0.500, 1.0],
            Color::Lime    => [0.20, 0.90, 0.200, 1.0],
            Color::Pink    => [1.00, 0.40, 0.700, 1.0],
            Color::Aqua    => [0.00, 0.80, 0.800, 1.0],
            Color::Maroon  => [0.50, 0.00, 0.000, 1.0],
            Color::Custom(r, g, b, a) => [r, g, b, a],
        }
    }

    /// Convert to a [`glam::Vec4`] (x=r, y=g, z=b, w=a).
    #[inline]
    pub fn vec4(self) -> Vec4 {
        let [r, g, b, a] = self.rgba();
        Vec4::new(r, g, b, a)
    }

    /// Return a copy of this color with a different alpha value.
    #[inline]
    pub fn with_alpha(self, a: f32) -> Color {
        let [r, g, b, _] = self.rgba();
        Color::Custom(r, g, b, a)
    }
}

impl From<Color> for [f32; 4] {
    fn from(c: Color) -> [f32; 4] { c.rgba() }
}

impl From<Color> for Vec4 {
    fn from(c: Color) -> Vec4 { c.vec4() }
}

impl Default for Color {
    fn default() -> Self { Color::White }
}
