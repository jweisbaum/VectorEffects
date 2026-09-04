//! Display unit conversion.
//!
//! **Speed is stored in metres per second and shown in knots, always.**
//!
//! m/s is what GRIB2 encodes, so it is what the document holds and what the
//! evaluator works in. Knots is what the audience for this tool reads: a
//! sailing forecast and a wind barb are both in knots, and a barb is
//! *defined* in 5-knot increments.
//!
//! The unit is not configurable. An earlier draft made it a project setting,
//! which bought nothing — nobody wants half their speeds in km/h — and cost a
//! branch at every display site plus a field in the file format.
//!
//! Conversion happens at the UI boundary and nowhere else, the same discipline
//! as the direction convention (spec.md 3.3, 3.4).

/// Knots per metre per second.
pub const KNOTS_PER_MPS: f64 = 1.943_844_492_440_605;

/// Converts a stored speed into the displayed unit.
pub fn knots_from_mps(mps: f64) -> f64 {
    mps * KNOTS_PER_MPS
}

/// Converts an entered speed into the stored unit.
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
