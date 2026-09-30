//! Basic math types.

use std::fmt;

/// A 3D vector.
///
/// Used for unit and doodad positions and scales. Warcraft III is Z-up, but
/// that convention belongs to the renderer, not here.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
}

impl Vec3 {
    /// The origin.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// All ones.
    pub const ONE: Self = Self {
        x: 1.0,
        y: 1.0,
        z: 1.0,
    };

    /// Builds a vector.
    #[inline]
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Builds a vector from plane coordinates, leaving Z at zero.
    #[inline]
    #[must_use]
    pub const fn new_xy(x: f32, y: f32) -> Self {
        Self { x, y, z: 0.0 }
    }

    /// Dot product.
    #[inline]
    #[must_use]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// Cross product.
    #[inline]
    #[must_use]
    pub fn cross(self, o: Self) -> Self {
        Self {
            x: self.y * o.z - self.z * o.y,
            y: self.z * o.x - self.x * o.z,
            z: self.x * o.y - self.y * o.x,
        }
    }

    /// Length.
    #[inline]
    #[must_use]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// Component-wise addition.
    #[inline]
    #[must_use]
    pub fn plus(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    /// Component-wise subtraction.
    #[inline]
    #[must_use]
    pub fn minus(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    /// Whether every component is finite.
    ///
    /// Parsers use this to reject corrupted coordinates: a single NaN reaching
    /// the renderer fails in ways that are hard to trace back.
    #[inline]
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl fmt::Display for Vec3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:.2}, {:.2}, {:.2})", self.x, self.y, self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_follows_right_hand_rule() {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(x.cross(y), Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn length_of_unit_diagonal() {
        assert!((Vec3::new(1.0, 1.0, 1.0).length() - 3f32.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn nan_is_not_finite() {
        assert!(!Vec3::new(f32::NAN, 0.0, 0.0).is_finite());
        assert!(!Vec3::new(f32::INFINITY, 0.0, 0.0).is_finite());
        assert!(Vec3::ZERO.is_finite());
    }

    #[test]
    fn add_and_sub_round_trip() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(-4.0, 5.5, 0.25);
        assert_eq!(a.plus(b).minus(b), a);
    }
}
