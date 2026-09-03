//! A synthetic field, used to build and verify the preview pipeline before the
//! real evaluator exists.
//!
//! M2 needs something to render that has *recognisable structure*: uniform
//! noise would prove tiles arrive but would not reveal a transposed axis, an
//! inverted latitude, or a rotation sense that is backwards. This field has
//! zonal bands whose direction reverses with latitude, plus a cyclone that
//! drifts east with time -- so a wrong axis, a flipped hemisphere, or a frozen
//! playhead all show up immediately.
//!
//! Replaced by the real evaluator in M3. Nothing depends on its exact shape.

use ve_core::angle::Angle;
use ve_core::geo::LonLat;
use ve_core::vector::{Uv, uv_from_speed_azimuth};

/// Peak zonal wind speed, m/s.
const ZONAL_PEAK_MPS: f64 = 12.0;
/// Peak cyclone wind speed, m/s.
const CYCLONE_PEAK_MPS: f64 = 30.0;
/// Radius of peak cyclone wind, metres.
const CYCLONE_RADIUS_M: f64 = 900_000.0;
/// Latitude the cyclone sits at.
const CYCLONE_LAT: f64 = 35.0;
/// Where the cyclone starts, and how fast it tracks east per step.
const CYCLONE_START_LON: f64 = -40.0;
/// Degrees of longitude the cyclone moves per time step.
const CYCLONE_DRIFT_DEG: f64 = 6.0;

/// Where the cyclone sits at `step`.
pub fn cyclone_centre(step: u32) -> LonLat {
    let lon = ve_core::geo::normalize_lon(CYCLONE_START_LON + f64::from(step) * CYCLONE_DRIFT_DEG);
    LonLat {
        lon,
        lat: CYCLONE_LAT,
    }
}

/// Samples the field at `position` and `step`.
pub fn sample(position: LonLat, step: u32) -> Uv {
    // Zonal bands: three reversals between pole and pole, so a flipped
    // latitude axis is obvious rather than subtle.
    let zonal = ZONAL_PEAK_MPS * (3.0 * position.lat.to_radians()).sin();

    let centre = cyclone_centre(step);
    let distance = centre.distance_m(position);
    // A Rankine-style profile: calm at the eye, peaking at the radius of
    // maximum wind, decaying outward. Finite everywhere, including the eye.
    let x = distance / CYCLONE_RADIUS_M;
    let cyclone_speed = CYCLONE_PEAK_MPS * x * (1.0 - x).exp();

    let cyclone = if cyclone_speed > 0.01 {
        // Counter-clockwise, the cyclonic sense in the northern hemisphere:
        // 90 degrees left of the outward bearing from the eye.
        let outward = centre.initial_bearing(position).degrees();
        uv_from_speed_azimuth(cyclone_speed, Angle::new(outward - 90.0))
    } else {
        Uv::default()
    };

    Uv {
        u: (zonal as f32) + cyclone.u,
        v: cyclone.v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::vector::speed_azimuth_from_uv;

    fn ll(lon: f64, lat: f64) -> LonLat {
        LonLat::new(lon, lat).unwrap()
    }

    /// Nothing may produce a NaN, including at the poles and in the eye.
    #[test]
    fn the_field_is_finite_everywhere() {
        for step in [0u32, 1, 12, 59] {
            for lat in [-90.0, -89.9, -45.0, 0.0, 45.0, 89.9, 90.0] {
                for lon in [-180.0, -179.9, -90.0, 0.0, 90.0, 179.9] {
                    let uv = sample(ll(lon, lat), step);
                    assert!(
                        uv.u.is_finite() && uv.v.is_finite(),
                        "step {step} at {lon},{lat} produced {uv:?}"
                    );
                }
            }
            let eye = cyclone_centre(step);
            let uv = sample(eye, step);
            assert!(uv.u.is_finite() && uv.v.is_finite());
        }
    }

    #[test]
    fn the_eye_is_calm() {
        let eye = cyclone_centre(0);
        let (speed, _) = speed_azimuth_from_uv(sample(eye, 0));
        // Only the zonal background remains at the centre.
        let zonal = ZONAL_PEAK_MPS * (3.0 * eye.lat.to_radians()).sin();
        assert!(
            (speed - zonal.abs()).abs() < 0.5,
            "eye speed {speed}, zonal {zonal}"
        );
    }

    /// Counter-clockwise: due east of the eye the flow must run northward.
    #[test]
    fn the_cyclone_turns_the_right_way() {
        let eye = cyclone_centre(0);
        let east = eye.destination(Angle::new(90.0), CYCLONE_RADIUS_M);
        let west = eye.destination(Angle::new(270.0), CYCLONE_RADIUS_M);

        assert!(sample(east, 0).v > 5.0, "east of the eye must flow north");
        assert!(sample(west, 0).v < -5.0, "west of the eye must flow south");
    }

    /// The bands must reverse with latitude, or a flipped axis goes unnoticed.
    #[test]
    fn zonal_bands_reverse_across_the_equator() {
        // Far from the cyclone so only the zonal term matters.
        let north = sample(ll(150.0, 30.0), 0);
        let south = sample(ll(150.0, -30.0), 0);
        assert!(north.u > 8.0, "30N should blow eastward, got {north:?}");
        assert!(south.u < -8.0, "30S should blow westward, got {south:?}");
    }

    #[test]
    fn the_cyclone_moves_with_time() {
        assert!(cyclone_centre(0).lon < cyclone_centre(1).lon);
        let point = ll(-40.0, 35.0);
        let a = sample(point, 0);
        let b = sample(point, 8);
        assert!(
            (a.u - b.u).abs() > 1.0 || (a.v - b.v).abs() > 1.0,
            "the field must change between steps"
        );
    }

    /// It must stay inside the tile encoding's full scale.
    #[test]
    fn speeds_stay_within_the_encoding_range() {
        for step in [0u32, 30] {
            for lat in (-90..=90).step_by(5) {
                for lon in (-180..180).step_by(5) {
                    let uv = sample(ll(f64::from(lon), f64::from(lat)), step);
                    let (speed, _) = speed_azimuth_from_uv(uv);
                    assert!(
                        speed < f64::from(crate::tile::SPEED_SCALE_MPS),
                        "speed {speed} exceeds the tile encoding scale"
                    );
                }
            }
        }
    }
}
