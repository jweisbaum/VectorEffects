//! Canonical numeric precision for the document.
//!
//! # Why this exists
//!
//! `serde_json`'s float *parser* is not correctly rounded: it lands one ULP
//! away from the true nearest `f64` for roughly 10% of values. Its writer is
//! fine and agrees with `std`, so the asymmetry means a raw `f64` written to a
//! project file does not always read back as the same value.
//!
//! Measured on 200,000 random values in each range:
//!
//! | value                      | round-trip failures |
//! |----------------------------|---------------------|
//! | raw `f64`, metres          | 19,953 / 200,000    |
//! | raw `f64`, degrees         | 23,839 / 200,000    |
//! | rounded `f64` (any below)  | 0 / 200,000         |
//! | `f32` (every scalar prop)  | 0 / 200,000         |
//!
//! Numerically the error is irrelevant -- one ULP of 380 km is about 6e-11 m.
//! What it breaks is *identity*: a project would not equal itself across a save
//! and load, which defeats byte-reproducible export (invariant 4) and makes
//! "did this file change?" unanswerable.
//!
//! The fix is to quantise at the serialisation boundary. The precisions below
//! are far finer than anything the application can express -- the finest grid
//! cell is 11 km wide, and geometry is stored to the millimetre -- so nothing
//! observable is lost.
//!
//! `f32` needs none of this, which is why scalar properties are left alone.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Decimal places kept for distances in metres. A millimetre.
pub const METRE_PLACES: i32 = 3;
/// Decimal places kept for angles and coordinates in degrees. About 0.1 mm.
pub const DEGREE_PLACES: i32 = 9;
/// Decimal places kept for dimensionless quantities such as zoom.
pub const RATIO_PLACES: i32 = 6;

/// Rounds to `places` decimal places.
///
/// Values that are not finite, or large enough that scaling would overflow,
/// are returned unchanged: they cannot be represented in JSON anyway and are
/// rejected by validation before they reach a file.
pub fn round(value: f64, places: i32) -> f64 {
    if !value.is_finite() || value.abs() >= 1e15 {
        return value;
    }
    let factor = 10f64.powi(places);
    (value * factor).round() / factor
}

/// Rounds a distance in metres to canonical precision.
pub fn metres(value: f64) -> f64 {
    round(value, METRE_PLACES)
}

/// Rounds an angle or coordinate in degrees to canonical precision.
pub fn degrees(value: f64) -> f64 {
    round(value, DEGREE_PLACES)
}

/// Rounds a dimensionless value to canonical precision.
pub fn ratio(value: f64) -> f64 {
    round(value, RATIO_PLACES)
}

/// Builds a serde helper module that rounds in both directions.
///
/// Rounding on read as well as write means the in-memory document matches the
/// file exactly after a load, so the two can never disagree.
macro_rules! rounding_serde {
    ($name:ident, $round:path, $doc:literal) => {
        #[doc = $doc]
        pub mod $name {
            use super::*;

            /// Rounds, then serialises.
            pub fn serialize<S: Serializer>(value: &f64, s: S) -> Result<S::Ok, S::Error> {
                $round(*value).serialize(s)
            }

            /// Deserialises, then rounds.
            pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
                Ok($round(f64::deserialize(d)?))
            }
        }
    };
}

rounding_serde!(
    metres_field,
    metres,
    "Serde helper for a distance in metres."
);
rounding_serde!(
    degrees_field,
    degrees,
    "Serde helper for an angle in degrees."
);
rounding_serde!(
    ratio_field,
    ratio,
    "Serde helper for a dimensionless value."
);

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole module exists to guarantee.
    #[test]
    fn rounded_values_survive_a_json_round_trip() {
        let mut state: u64 = 0x243f_6a88_85a3_08d3;
        let mut checked = 0;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let unit = state as f64 / u64::MAX as f64;

            for (value, rounder) in [
                (unit * 1.0e7 - 5.0e6, metres as fn(f64) -> f64),
                (unit * 360.0 - 180.0, degrees as fn(f64) -> f64),
                (unit * 24.0, ratio as fn(f64) -> f64),
            ] {
                let canonical = rounder(value);
                let json = serde_json::to_string(&canonical).expect("finite");
                let back: f64 = serde_json::from_str(&json).expect("valid json");
                assert_eq!(
                    back.to_bits(),
                    canonical.to_bits(),
                    "{canonical} did not survive as {json}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 60_000);
    }

    /// Documents the upstream defect this module works around. If a future
    /// serde_json fixes its parser, this test fails and the module can go.
    #[test]
    fn raw_floats_still_need_the_workaround() {
        let value: f64 = 380_812.358_415_356_84;
        let json = serde_json::to_string(&value).expect("finite");
        let back: f64 = serde_json::from_str(&json).expect("valid json");
        assert_ne!(
            back.to_bits(),
            value.to_bits(),
            "serde_json now round-trips raw f64 correctly; canonical.rs may be removable"
        );
    }

    #[test]
    fn rounding_keeps_the_value_close() {
        assert!((metres(380_812.358_415_356_84) - 380_812.358_415_356_84).abs() < 1e-3);
        assert!((degrees(-107.943_262_213_740_03) + 107.943_262_213_740_03).abs() < 1e-9);
    }

    #[test]
    fn rounding_is_idempotent() {
        for v in [0.0, -1.5, 1234.56789, -98765.4321] {
            assert_eq!(metres(metres(v)).to_bits(), metres(v).to_bits());
            assert_eq!(degrees(degrees(v)).to_bits(), degrees(v).to_bits());
        }
    }

    /// Values that cannot be scaled must pass through rather than becoming
    /// infinity or NaN.
    #[test]
    fn extreme_and_non_finite_values_pass_through() {
        assert!(round(f64::NAN, 3).is_nan());
        assert_eq!(round(f64::INFINITY, 3), f64::INFINITY);
        assert_eq!(round(1e300, 3), 1e300);
        assert_eq!(round(-1e300, 9), -1e300);
    }

    #[test]
    fn zero_and_small_values_are_preserved() {
        assert_eq!(metres(0.0), 0.0);
        assert_eq!(metres(0.0005), 0.001);
        assert_eq!(metres(0.0004), 0.0);
    }
}
