//! Canonical conversions used by wind barbs and legacy colour-scale fields.
//!
//! Vector data remains in m/s. Global UI preferences select kt, mph, or km/h
//! at the view boundary; wind-barb flags always encode 5-knot increments.

/// Knots per metre per second.
pub const KNOTS_PER_MPS: f64 = 1.943_844_492_440_605;

/// Converts a stored speed to knots.
pub fn knots_from_mps(mps: f64) -> f64 {
    mps * KNOTS_PER_MPS
}

/// Converts knots to the stored m/s unit.
pub fn mps_from_knots(knots: f64) -> f64 {
    knots / KNOTS_PER_MPS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_metre_per_second_is_about_two_knots() {
        assert!((knots_from_mps(1.0) - 1.943_844).abs() < 1e-6);
    }

    /// The conventional reference point: a gale is 34 knots.
    #[test]
    fn known_reference_points_convert() {
        assert!(
            (mps_from_knots(34.0) - 17.49).abs() < 0.01,
            "{}",
            mps_from_knots(34.0)
        );
        assert!((knots_from_mps(10.0) - 19.438).abs() < 0.01);
    }

    #[test]
    fn conversion_round_trips() {
        for mps in [0.0, 0.5, 12.5, 33.3, 120.0] {
            assert!((mps_from_knots(knots_from_mps(mps)) - mps).abs() < 1e-9);
        }
    }

    #[test]
    fn zero_stays_zero() {
        assert_eq!(knots_from_mps(0.0), 0.0);
        assert_eq!(mps_from_knots(0.0), 0.0);
    }
}
