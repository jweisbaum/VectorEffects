//! Vector representation and the wind/current direction conventions.
//!
//! Every vector in VectorEffects is stored as speed plus **azimuth-toward**:
//! the direction the vector points, clockwise from true north. Conversion to
//! and from the meteorological "direction it comes from" convention happens
//! exactly once, at the IPC boundary, and never below it (spec.md 3.3).
//!
//! This module exists because a sign error here would produce GRIB files that
//! parse cleanly, look plausible, and are 180 degrees wrong.

use serde::{Deserialize, Serialize};

use crate::angle::Angle;

/// A vector sample: eastward and northward components, in m/s.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Uv {
    /// Eastward component, m/s. GRIB UGRD / UOGRD.
    pub u: f32,
    /// Northward component, m/s. GRIB VGRD / VOGRD.
    pub v: f32,
}

/// Converts speed and azimuth-toward into earth-relative `u`/`v` components.
pub fn uv_from_speed_azimuth(speed_mps: f64, azimuth_toward: Angle) -> Uv {
    let theta = azimuth_toward.radians();
    Uv {
        u: (speed_mps * theta.sin()) as f32,
        v: (speed_mps * theta.cos()) as f32,
    }
}

/// Recovers speed (m/s) and azimuth-toward from `u`/`v` components.
///
/// A zero vector has no meaningful direction; azimuth 0 is returned.
pub fn speed_azimuth_from_uv(uv: Uv) -> (f64, Angle) {
    let (u, v) = (f64::from(uv.u), f64::from(uv.v));
    let speed = u.hypot(v);
    if speed == 0.0 {
        return (0.0, Angle::new(0.0));
    }
    (speed, Angle::new(u.atan2(v).to_degrees()))
}

/// How a direction is presented to the user.
///
/// This affects display and tool input only. Storage is always azimuth-toward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectionConvention {
    /// Meteorological: the direction the flow comes *from*. Conventional for wind.
    From,
    /// Oceanographic: the direction the flow goes *toward*. Conventional for currents.
    Toward,
}

impl DirectionConvention {
    /// Converts stored azimuth-toward into the value shown to the user.
    pub fn to_display(self, stored: Angle) -> Angle {
        match self {
            Self::From => stored.reciprocal(),
            Self::Toward => stored,
        }
    }

    /// Converts a user-entered direction into stored azimuth-toward.
    pub fn from_display(self, entered: Angle) -> Angle {
        // Both conventions are their own inverse, but spelling this out
        // separately keeps the call sites readable at the IPC boundary.
        match self {
            Self::From => entered.reciprocal(),
            Self::Toward => entered,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    /// The cardinal cases. A `u`/`v` swap or a sign flip fails here, which is
    /// the single most likely silent bug in the codebase.
    #[test]
    fn components_follow_the_toward_convention() {
        let s = 10.0;

        let north = uv_from_speed_azimuth(s, Angle::new(0.0));
        close(f64::from(north.u), 0.0);
        close(f64::from(north.v), 10.0);

        let east = uv_from_speed_azimuth(s, Angle::new(90.0));
        close(f64::from(east.u), 10.0);
        close(f64::from(east.v), 0.0);

        let south = uv_from_speed_azimuth(s, Angle::new(180.0));
        close(f64::from(south.u), 0.0);
        close(f64::from(south.v), -10.0);

        let west = uv_from_speed_azimuth(s, Angle::new(270.0));
        close(f64::from(west.u), -10.0);
        close(f64::from(west.v), 0.0);
    }

    /// `Uv` stores `f32` because the export grid holds 6.5M of them; a
    /// round-trip through it therefore loses roughly 1e-6 degrees of angle.
    /// The tolerance here is the precision of the storage type, not a fudge:
    /// 1e-4 degrees is five orders of magnitude below anything the packing
    /// resolves (spec.md 12.3).
    #[test]
    fn speed_azimuth_round_trips() {
        fn close_f32(a: f64, b: f64) {
            assert!((a - b).abs() < 1e-4, "{a} != {b}");
        }
        for az in [0.0, 37.0, 90.0, 179.0, 200.0, 359.5] {
            let uv = uv_from_speed_azimuth(12.5, Angle::new(az));
            let (speed, back) = speed_azimuth_from_uv(uv);
            close_f32(speed, 12.5);
            close_f32(back.degrees(), az);
        }
    }

    #[test]
    fn zero_vector_has_no_direction() {
        let (speed, az) = speed_azimuth_from_uv(Uv { u: 0.0, v: 0.0 });
        close(speed, 0.0);
        close(az.degrees(), 0.0);
    }

    /// A northerly wind blows *from* the north, so it points south: stored
    /// azimuth 180, displayed as 0 under the meteorological convention.
    #[test]
    fn wind_display_uses_the_from_convention() {
        let stored = Angle::new(180.0);
        close(DirectionConvention::From.to_display(stored).degrees(), 0.0);
        close(
            DirectionConvention::Toward.to_display(stored).degrees(),
            180.0,
        );
    }

    #[test]
    fn display_conversion_round_trips() {
        for conv in [DirectionConvention::From, DirectionConvention::Toward] {
            for az in [0.0, 45.0, 110.0, 271.0] {
                let stored = Angle::new(az);
                let shown = conv.to_display(stored);
                close(conv.from_display(shown).degrees(), az);
            }
        }
    }
}
