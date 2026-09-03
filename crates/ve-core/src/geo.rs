//! Geographic coordinates and spherical geodesy.
//!
//! One earth model, used everywhere including GRIB encoding. `EARTH_RADIUS_M`
//! matches GRIB2 shape-of-earth code 6, so the sphere the user draws on and the
//! sphere declared in the exported file are the same sphere (spec.md 3.1).

use serde::{Deserialize, Serialize};

use crate::angle::Angle;
use crate::error::{CoreError, Result};

/// Earth radius in metres. Matches GRIB2 shape-of-earth code 6.
pub const EARTH_RADIUS_M: f64 = 6_371_229.0;

/// A geographic position. Longitude is always normalised to `[-180, 180)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LonLat {
    /// Longitude in degrees, `[-180, 180)`.
    #[serde(with = "crate::canonical::degrees_field")]
    pub lon: f64,
    /// Latitude in degrees, `[-90, 90]`.
    #[serde(with = "crate::canonical::degrees_field")]
    pub lat: f64,
}

/// Normalises a longitude into `[-180, 180)`.
pub fn normalize_lon(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

impl LonLat {
    /// Creates a position, normalising longitude and validating latitude.
    pub fn new(lon: f64, lat: f64) -> Result<Self> {
        if !lon.is_finite() {
            return Err(CoreError::NonFiniteCoordinate("lon"));
        }
        if !lat.is_finite() {
            return Err(CoreError::NonFiniteCoordinate("lat"));
        }
        if !(-90.0..=90.0).contains(&lat) {
            return Err(CoreError::LatitudeOutOfRange(lat));
        }
        Ok(Self {
            lon: normalize_lon(lon),
            lat,
        })
    }

    /// Great-circle distance to `other`, in metres.
    pub fn distance_m(self, other: Self) -> f64 {
        let (phi1, phi2) = (self.lat.to_radians(), other.lat.to_radians());
        let dphi = phi2 - phi1;
        // Normalising the longitude delta is what makes the antimeridian an
        // ordinary case: 179E -> 179W is 2 degrees apart, not 358.
        let dlambda = normalize_lon(other.lon - self.lon).to_radians();

        let a =
            (dphi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_M * a.sqrt().clamp(-1.0, 1.0).asin()
    }

    /// Initial great-circle bearing toward `other`, as an azimuth clockwise
    /// from true north.
    pub fn initial_bearing(self, other: Self) -> Angle {
        let (phi1, phi2) = (self.lat.to_radians(), other.lat.to_radians());
        let dlambda = normalize_lon(other.lon - self.lon).to_radians();

        let y = dlambda.sin() * phi2.cos();
        let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * dlambda.cos();
        Angle::new(y.atan2(x).to_degrees())
    }

    /// The point reached by travelling `distance_m` along `bearing`.
    ///
    /// Together with [`Self::distance_m`] and [`Self::initial_bearing`] this
    /// forms the object-local AEQD frame described in spec.md 7.2.
    pub fn destination(self, bearing: Angle, distance_m: f64) -> Self {
        let delta = distance_m / EARTH_RADIUS_M;
        let theta = bearing.radians();
        let phi1 = self.lat.to_radians();
        let lambda1 = self.lon.to_radians();

        let sin_phi2 = phi1.sin() * delta.cos() + phi1.cos() * delta.sin() * theta.cos();
        let phi2 = sin_phi2.clamp(-1.0, 1.0).asin();
        let lambda2 = lambda1
            + (theta.sin() * delta.sin() * phi1.cos()).atan2(delta.cos() - phi1.sin() * sin_phi2);

        Self {
            lon: normalize_lon(lambda2.to_degrees()),
            lat: phi2.to_degrees(),
        }
    }
}

/// Rhumb-line (constant-bearing) distance to `other`, in metres.
///
/// A rhumb line is the path of constant compass bearing. It is longer than the
/// great circle but is what a hand-steered course actually follows, which is
/// why the measurement tools draw both (spec.md 10).
pub fn rhumb_distance_m(from: LonLat, to: LonLat) -> f64 {
    let (phi1, phi2) = (from.lat.to_radians(), to.lat.to_radians());
    let dphi = phi2 - phi1;
    let dlambda = normalize_lon(to.lon - from.lon).to_radians();

    // The stretched latitude difference from the Mercator projection, which is
    // what makes a constant bearing a straight line.
    let dpsi = ((phi2 / 2.0 + std::f64::consts::FRAC_PI_4).tan()
        / (phi1 / 2.0 + std::f64::consts::FRAC_PI_4).tan())
    .ln();

    // Near an east-west course dpsi tends to zero and the ratio is unstable;
    // fall back to the cosine of the mean latitude.
    let q = if dpsi.abs() > 1e-12 {
        dphi / dpsi
    } else {
        phi1.cos()
    };

    EARTH_RADIUS_M * dphi.hypot(q * dlambda)
}

/// Constant bearing that follows the rhumb line to `other`.
pub fn rhumb_bearing(from: LonLat, to: LonLat) -> Angle {
    let (phi1, phi2) = (from.lat.to_radians(), to.lat.to_radians());
    let dlambda = normalize_lon(to.lon - from.lon).to_radians();
    let dpsi = ((phi2 / 2.0 + std::f64::consts::FRAC_PI_4).tan()
        / (phi1 / 2.0 + std::f64::consts::FRAC_PI_4).tan())
    .ln();
    Angle::new(dlambda.atan2(dpsi).to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ll(lon: f64, lat: f64) -> LonLat {
        LonLat::new(lon, lat).unwrap()
    }

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() < tol, "{a} != {b} (tol {tol})");
    }

    #[test]
    fn longitude_normalises_half_open() {
        close(normalize_lon(181.0), -179.0, 1e-9);
        close(normalize_lon(-181.0), 179.0, 1e-9);
        close(normalize_lon(180.0), -180.0, 1e-9);
        close(normalize_lon(-180.0), -180.0, 1e-9);
        close(normalize_lon(540.0), -180.0, 1e-9);
    }

    #[test]
    fn latitude_out_of_range_is_rejected() {
        assert!(LonLat::new(0.0, 91.0).is_err());
        assert!(LonLat::new(0.0, f64::NAN).is_err());
        assert!(LonLat::new(0.0, 90.0).is_ok());
    }

    /// Analytic references, independent of the implementation: a quarter of the
    /// equator, and a pole-to-pole meridian.
    #[test]
    fn distance_matches_analytic_cases() {
        close(
            ll(0.0, 0.0).distance_m(ll(90.0, 0.0)),
            EARTH_RADIUS_M * std::f64::consts::FRAC_PI_2,
            1e-3,
        );
        close(
            ll(0.0, -90.0).distance_m(ll(0.0, 90.0)),
            EARTH_RADIUS_M * std::f64::consts::PI,
            1e-3,
        );
        // One degree along a meridian is exactly R * 1 degree in radians.
        close(
            ll(0.0, 0.0).distance_m(ll(0.0, 1.0)),
            EARTH_RADIUS_M * 1.0_f64.to_radians(),
            1e-6,
        );
    }

    /// The antimeridian must be an ordinary case, not 358 degrees of travel.
    #[test]
    fn distance_crosses_the_antimeridian() {
        let two_degrees = EARTH_RADIUS_M * 2.0_f64.to_radians();
        close(
            ll(179.0, 0.0).distance_m(ll(-179.0, 0.0)),
            two_degrees,
            1e-6,
        );
    }

    #[test]
    fn bearings_point_the_right_way() {
        close(
            ll(0.0, 0.0).initial_bearing(ll(0.0, 10.0)).degrees(),
            0.0,
            1e-9,
        );
        close(
            ll(0.0, 0.0).initial_bearing(ll(10.0, 0.0)).degrees(),
            90.0,
            1e-9,
        );
        close(
            ll(0.0, 0.0).initial_bearing(ll(0.0, -10.0)).degrees(),
            180.0,
            1e-9,
        );
        close(
            ll(0.0, 0.0).initial_bearing(ll(-10.0, 0.0)).degrees(),
            270.0,
            1e-9,
        );
    }

    /// Heading "east" along a northern parallel, the great circle initially
    /// bears north of east. This catches a swapped atan2 that still passes the
    /// cardinal-direction cases above.
    #[test]
    fn great_circle_bulges_poleward() {
        let b = ll(0.0, 60.0).initial_bearing(ll(10.0, 60.0)).degrees();
        assert!(b < 90.0 && b > 80.0, "expected north of east, got {b}");
    }

    #[test]
    fn destination_round_trips() {
        let start = ll(-30.0, 45.0);
        let bearing = Angle::new(117.0);
        let dist = 1_234_567.0;
        let end = start.destination(bearing, dist);

        close(start.distance_m(end), dist, 1e-3);
        close(
            start.initial_bearing(end).degrees(),
            bearing.degrees(),
            1e-9,
        );
    }

    /// Travelling across the antimeridian must wrap rather than run off the end.
    #[test]
    fn destination_wraps_across_the_antimeridian() {
        let start = ll(179.0, 0.0);
        let end = start.destination(Angle::new(90.0), EARTH_RADIUS_M * 2.0_f64.to_radians());
        close(end.lon, -179.0, 1e-6);
        close(end.lat, 0.0, 1e-9);
    }

    /// A polar case, where naive lat/lon arithmetic breaks down.
    /// Along a meridian the rhumb line and the great circle are the same path.
    #[test]
    fn rhumb_matches_the_great_circle_along_a_meridian() {
        let (a, b) = (ll(10.0, -20.0), ll(10.0, 40.0));
        close(rhumb_distance_m(a, b), a.distance_m(b), 1e-3);
        close(rhumb_bearing(a, b).degrees(), 0.0, 1e-9);
    }

    /// Along the equator they also coincide, and the bearing is due east.
    #[test]
    fn rhumb_matches_the_great_circle_along_the_equator() {
        let (a, b) = (ll(-10.0, 0.0), ll(30.0, 0.0));
        close(rhumb_distance_m(a, b), a.distance_m(b), 1e-3);
        close(rhumb_bearing(a, b).degrees(), 90.0, 1e-9);
    }

    /// Everywhere else the rhumb line is the longer path -- that difference is
    /// the whole reason the measurement tool draws both.
    #[test]
    fn rhumb_is_longer_than_the_great_circle() {
        let (a, b) = (ll(-74.0, 40.7), ll(-0.1, 51.5));
        let (rhumb, great) = (rhumb_distance_m(a, b), a.distance_m(b));
        assert!(
            rhumb > great,
            "rhumb {rhumb} should exceed great circle {great}"
        );
        // A few percent for a transatlantic leg, not orders of magnitude.
        assert!(rhumb / great < 1.05, "ratio {}", rhumb / great);
    }

    /// A due-east course along a parallel has constant bearing 90 and a length
    /// of the parallel's own circumference fraction, not the great circle's.
    #[test]
    fn rhumb_follows_a_parallel() {
        let lat = 60.0;
        let (a, b) = (ll(0.0, lat), ll(90.0, lat));
        close(rhumb_bearing(a, b).degrees(), 90.0, 1e-6);

        let expected = EARTH_RADIUS_M * 90.0_f64.to_radians() * lat.to_radians().cos();
        close(rhumb_distance_m(a, b), expected, 1.0);
        assert!(rhumb_distance_m(a, b) > a.distance_m(b));
    }

    #[test]
    fn rhumb_handles_the_antimeridian() {
        let (a, b) = (ll(179.0, 10.0), ll(-179.0, 10.0));
        let two_degrees_at_lat =
            EARTH_RADIUS_M * 2.0_f64.to_radians() * 10.0_f64.to_radians().cos();
        close(rhumb_distance_m(a, b), two_degrees_at_lat, 1.0);
        close(rhumb_bearing(a, b).degrees(), 90.0, 1e-6);
    }

    #[test]
    fn destination_crosses_the_pole() {
        let start = ll(0.0, 89.0);
        let end = start.destination(Angle::new(0.0), EARTH_RADIUS_M * 2.0_f64.to_radians());
        close(end.lat, 89.0, 1e-6);
        close(end.lon.abs(), 180.0, 1e-6);
    }
}
