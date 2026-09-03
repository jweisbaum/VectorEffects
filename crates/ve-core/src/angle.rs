//! Angles in degrees, normalised to `[0, 360)`.
//!
//! All interpolation between angles takes the shortest arc (`CLAUDE.md`,
//! conventions). Linear interpolation on raw degrees would send 350 -> 10 the
//! long way round through 180, which is both wrong and highly visible in an
//! animated wind field.

use serde::{Deserialize, Serialize};

/// A bearing or direction in degrees, always normalised to `[0, 360)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Angle(#[serde(with = "crate::canonical::degrees_field")] f64);

impl Angle {
    /// Creates an angle, normalising into `[0, 360)`.
    pub fn new(degrees: f64) -> Self {
        Self(degrees.rem_euclid(360.0))
    }

    /// The normalised value in degrees, `[0, 360)`.
    pub const fn degrees(self) -> f64 {
        self.0
    }

    /// The value in radians.
    pub fn radians(self) -> f64 {
        self.0.to_radians()
    }

    /// Signed shortest angular distance from `self` to `other`, in `(-180, 180]`.
    ///
    /// Positive is clockwise (increasing bearing).
    pub fn shortest_delta_to(self, other: Self) -> f64 {
        let raw = (other.0 - self.0).rem_euclid(360.0);
        if raw > 180.0 { raw - 360.0 } else { raw }
    }

    /// Interpolates toward `other` along the shortest arc.
    pub fn lerp_shortest(self, other: Self, t: f64) -> Self {
        Self::new(self.0 + self.shortest_delta_to(other) * t)
    }

    /// The reciprocal bearing, 180 degrees opposed.
    ///
    /// This is the only operation that converts between "wind from" and
    /// "wind toward"; it belongs at the UI boundary and nowhere below it.
    pub fn reciprocal(self) -> Self {
        Self::new(self.0 + 180.0)
    }
}

impl From<f64> for Angle {
    fn from(degrees: f64) -> Self {
        Self::new(degrees)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn normalises_into_range() {
        close(Angle::new(370.0).degrees(), 10.0);
        close(Angle::new(-10.0).degrees(), 350.0);
        close(Angle::new(-720.5).degrees(), 359.5);
        close(Angle::new(360.0).degrees(), 0.0);
    }

    #[test]
    fn shortest_delta_wraps_through_zero() {
        close(Angle::new(350.0).shortest_delta_to(Angle::new(10.0)), 20.0);
        close(Angle::new(10.0).shortest_delta_to(Angle::new(350.0)), -20.0);
        close(Angle::new(0.0).shortest_delta_to(Angle::new(180.0)), 180.0);
    }

    /// The case named explicitly in `plan.md` M1 acceptance: 350 -> 10 must
    /// pass through 0, never through 180.
    #[test]
    fn lerp_350_to_10_passes_through_zero() {
        close(
            Angle::new(350.0)
                .lerp_shortest(Angle::new(10.0), 0.5)
                .degrees(),
            0.0,
        );
        close(
            Angle::new(350.0)
                .lerp_shortest(Angle::new(10.0), 0.25)
                .degrees(),
            355.0,
        );
        close(
            Angle::new(350.0)
                .lerp_shortest(Angle::new(10.0), 1.0)
                .degrees(),
            10.0,
        );
    }

    #[test]
    fn reciprocal_is_involutive() {
        close(Angle::new(110.0).reciprocal().degrees(), 290.0);
        close(Angle::new(110.0).reciprocal().reciprocal().degrees(), 110.0);
    }
}
