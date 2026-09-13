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
    let dlambda = normalize_lon(to.lon - from.lon).to_radians();
    let dpsi = isometric_lat(to.lat) - isometric_lat(from.lat);
    Angle::new(dlambda.atan2(dpsi).to_degrees())
}

/// Walk a constant true bearing and distance. A rhumb line cannot cross a pole.
pub fn rhumb_destination(from: LonLat, bearing: Angle, distance_m: f64) -> Option<LonLat> {
    if !distance_m.is_finite() || distance_m < 0.0 {
        return None;
    }
    if distance_m == 0.0 {
        return Some(from);
    }
    let phi1 = from.lat.to_radians();
    let theta = bearing.degrees().to_radians();
    let delta = distance_m / EARTH_RADIUS_M;
    let dphi = delta * theta.cos();
    let phi2 = phi1 + dphi;
    if phi1.abs() >= std::f64::consts::FRAC_PI_2 || phi2.abs() >= std::f64::consts::FRAC_PI_2 {
        return None;
    }
    let dpsi = ((std::f64::consts::FRAC_PI_4 + phi2 / 2.0).tan()
        / (std::f64::consts::FRAC_PI_4 + phi1 / 2.0).tan())
    .ln();
    let q = if dpsi.abs() > 1e-12 {
        dphi / dpsi
    } else {
        phi1.cos()
    };
    LonLat::new(
        from.lon + (delta * theta.sin() / q).to_degrees(),
        phi2.to_degrees(),
    )
    .ok()
}

/// Isometric latitude: the Mercator projection's stretched latitude.
///
/// The coordinate a rhumb line is straight in, which is what both the rhumb
/// functions above are built on and what [`rhumb_path`] steps along. Infinite
/// at the poles, so it is clamped there — a rhumb line reaching a pole spirals
/// into it through infinitely many turns and has no last point to draw.
fn isometric_lat(lat_deg: f64) -> f64 {
    let phi = lat_deg.clamp(-89.9999, 89.9999).to_radians();
    (phi / 2.0 + std::f64::consts::FRAC_PI_4).tan().ln()
}

/// How finely a measured path is drawn: one vertex per degree of arc.
///
/// The paths are geographic, so this is a property of the path and not of the
/// zoom: densifying by the camera would rebuild every polyline on every wheel
/// event, and a degree of arc is about a pixel at the sharpest zoom the map
/// offers over a path long enough for the curvature to show at all.
const PATH_STEP_DEG: f64 = 1.0;

/// Vertices for a path spanning `span_deg` of arc, at [`PATH_STEP_DEG`].
///
/// At least two, so a path is always drawable, and capped: a great circle is
/// at most 180° of arc and a rhumb line near a pole can run much further in
/// longitude than it does in distance.
fn path_vertices(span_deg: f64) -> usize {
    ((span_deg / PATH_STEP_DEG).ceil() as usize).clamp(1, 512) + 1
}

/// The great-circle path from `from` to `to`, as a polyline.
///
/// Spherical linear interpolation, in Cartesian space: the great circle
/// *is* the plane through the two points and the centre, so rotating one
/// vector toward the other in that plane is the path itself rather than an
/// approximation of it.
///
/// Antipodal points have no unique great circle between them — every plane
/// through the centre contains both — so the pair is returned unjoined rather
/// than one of the infinitely many answers being invented.
pub fn great_circle_path(from: LonLat, to: LonLat) -> Vec<LonLat> {
    let a = unit(from);
    let b = unit(to);
    let dot = (a.0 * b.0 + a.1 * b.1 + a.2 * b.2).clamp(-1.0, 1.0);
    let omega = dot.acos();
    let sin_omega = omega.sin();
    if sin_omega.abs() < 1e-12 {
        return vec![from, to];
    }

    let count = path_vertices(omega.to_degrees());
    // The ends are the endpoints themselves, not what a round trip through
    // Cartesian gives back: a divider's handle sits on the end of its own
    // segment, and a last-bit difference there is a handle beside the line.
    (0..count)
        .map(|i| match i {
            0 => from,
            _ if i == count - 1 => to,
            _ => {
                let t = i as f64 / (count - 1) as f64;
                let ca = ((1.0 - t) * omega).sin() / sin_omega;
                let cb = (t * omega).sin() / sin_omega;
                from_unit((
                    ca * a.0 + cb * b.0,
                    ca * a.1 + cb * b.1,
                    ca * a.2 + cb * b.2,
                ))
            }
        })
        .collect()
}

/// The rhumb-line path from `from` to `to`, as a polyline.
///
/// Straight in the isometric latitude, which is the defining property: a
/// constant bearing means longitude advances in proportion to `psi`, so
/// stepping both linearly *is* the constant-bearing path. It is drawn as a
/// polyline all the same, because it is a curve in every projection the map
/// offers except Mercator.
///
/// The longitude difference is taken the short way round, so a course crossing
/// the antimeridian is the ordinary case rather than a trip round the world.
pub fn rhumb_path(from: LonLat, to: LonLat) -> Vec<LonLat> {
    let dlambda = normalize_lon(to.lon - from.lon);
    let psi1 = isometric_lat(from.lat);
    let dpsi = isometric_lat(to.lat) - psi1;

    // A path measured in degrees of *arc*, so the two axes are comparable:
    // longitude counts for less the further from the equator, and a course
    // along a parallel at 80° covers a sixth of the ground its degrees suggest.
    let mean_lat = (from.lat + to.lat) / 2.0;
    let span = (to.lat - from.lat).hypot(dlambda * mean_lat.to_radians().cos());
    let count = path_vertices(span);

    // Stepped in *latitude*, not in the isometric latitude the line is
    // straight in. The two trace the same curve, but a rhumb line's distance
    // is proportional to its change in latitude, so this one puts its vertices
    // an equal distance apart — which is what a drawn path wants, and what
    // makes the midpoint of the list the midpoint of the sail.
    (0..count)
        .map(|i| match i {
            0 => from,
            _ if i == count - 1 => to,
            _ => {
                let t = i as f64 / (count - 1) as f64;
                if dpsi.abs() <= 1e-12 {
                    // Due east or west: the latitude does not change, and the
                    // isometric step is zero rather than small — dividing by it
                    // would be noise.
                    return LonLat {
                        lon: normalize_lon(from.lon + t * dlambda),
                        lat: from.lat,
                    };
                }
                let lat = from.lat + t * (to.lat - from.lat);
                let lon = from.lon + (isometric_lat(lat) - psi1) / dpsi * dlambda;
                LonLat {
                    lon: normalize_lon(lon),
                    lat: lat.clamp(-90.0, 90.0),
                }
            }
        })
        .collect()
}

/// A geodesic circle of `radius_m` about `centre`, as a closed polyline.
///
/// Every vertex is [`LonLat::destination`] at one bearing, so the ring is a
/// set of points at a true distance rather than a shape in degrees. Near a
/// pole — or at a radius large enough to reach one — that is a very different
/// figure from a circle on the map, which is the whole reason to draw it.
pub fn geodesic_ring(centre: LonLat, radius_m: f64) -> Vec<LonLat> {
    const VERTICES: usize = 180;
    (0..=VERTICES)
        .map(|i| {
            let bearing = Angle::new(i as f64 * 360.0 / VERTICES as f64);
            centre.destination(bearing, radius_m)
        })
        .collect()
}

/// A position as a unit vector on the sphere.
fn unit(p: LonLat) -> (f64, f64, f64) {
    let (phi, lambda) = (p.lat.to_radians(), p.lon.to_radians());
    (
        phi.cos() * lambda.cos(),
        phi.cos() * lambda.sin(),
        phi.sin(),
    )
}

/// And back. The vector need not be normalised: only its direction is read.
fn from_unit(v: (f64, f64, f64)) -> LonLat {
    let lat = v.2.atan2(v.0.hypot(v.1)).to_degrees();
    LonLat {
        lon: normalize_lon(v.1.atan2(v.0).to_degrees()),
        lat: lat.clamp(-90.0, 90.0),
    }
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

    /// Every vertex of a great-circle path is on the great circle.
    ///
    /// Checked against the defining property rather than against the slerp
    /// that made it: a point is on the great circle through A and B exactly
    /// when its distance from A plus its distance to B is the distance from A
    /// to B. Any point off the arc makes that sum larger.
    #[test]
    fn a_great_circle_path_lies_on_the_great_circle() {
        for (a, b) in [
            (ll(-73.78, 40.64), ll(-0.46, 51.47)), // JFK to Heathrow
            (ll(174.8, -36.9), ll(-58.4, -34.6)),  // Auckland to Buenos Aires
            (ll(179.0, 60.0), ll(-179.0, 62.0)),   // across the antimeridian
            (ll(10.0, 85.0), ll(-170.0, 85.0)),    // over the pole
        ] {
            let path = great_circle_path(a, b);
            let whole = a.distance_m(b);
            assert!(path.len() >= 2, "a path needs two points");
            assert_eq!((path[0].lon, path[0].lat), (a.lon, a.lat));
            for point in &path {
                let via = a.distance_m(*point) + point.distance_m(b);
                // A metre over ten thousand kilometres.
                close(via, whole, whole * 1e-7 + 1.0);
            }
        }
    }

    /// A rhumb line is *defined* as the path of constant bearing, so the
    /// closed form is checked against walking that bearing.
    ///
    /// The walk takes two thousand short steps with [`LonLat::destination`],
    /// each along the same compass bearing — an integration of the defining
    /// property, and a different computation from the isometric-latitude
    /// formula the path is built with. If they agree, the formula is right.
    #[test]
    fn a_rhumb_path_is_where_walking_the_bearing_arrives() {
        for (a, b) in [
            (ll(-73.78, 40.64), ll(-0.46, 51.47)),
            (ll(20.0, -30.0), ll(-40.0, -55.0)),
            (ll(178.0, 20.0), ll(-176.0, 44.0)), // across the antimeridian
        ] {
            let bearing = rhumb_bearing(a, b);
            let total = rhumb_distance_m(a, b);
            const STEPS: usize = 20_000;
            let mut walked = a;
            for _ in 0..STEPS {
                walked = walked.destination(bearing, total / STEPS as f64);
            }
            // A kilometre after ten thousand, which is the integration's own
            // error: each step is a great-circle hop and the bearing drifts
            // across it.
            // 134 m over 5,758 km, and it halves when the step count doubles:
            // the error is the walk's, not the formula's.
            close(walked.distance_m(b), 0.0, total * 1e-4);

            // And the closed form agrees with the walk in the middle, not only
            // at the ends — which is a statement about the path's shape, and
            // about its vertices being spaced by distance: the vertex a third
            // of the way along the list is a third of the way along the sail.
            let path = rhumb_path(a, b);
            let third = path.len() / 3;
            let fraction = third as f64 / (path.len() - 1) as f64;
            let mut walked_part = a;
            for _ in 0..(fraction * STEPS as f64).round() as usize {
                walked_part = walked_part.destination(bearing, total / STEPS as f64);
            }
            close(path[third].distance_m(walked_part), 0.0, total * 1e-4);
        }
    }

    /// The rhumb line is straight in Mercator, and only there.
    ///
    /// Straightness is checked in the isometric latitude — which is Mercator's
    /// `y` — against the chord between the endpoints. The same path is then
    /// shown to bow away from its chord in plain latitude, so the test says
    /// which projection the property belongs to rather than assuming it.
    #[test]
    fn a_rhumb_path_is_straight_in_mercator_and_curved_in_a_flat_map() {
        let (a, b) = (ll(-73.78, 40.64), ll(-0.46, 51.47));
        let path = rhumb_path(a, b);
        let dlon = normalize_lon(b.lon - a.lon);
        let (psi_a, psi_b) = (isometric_lat(a.lat), isometric_lat(b.lat));

        let mut worst_flat: f64 = 0.0;
        for point in &path {
            let t = normalize_lon(point.lon - a.lon) / dlon;
            close(isometric_lat(point.lat), psi_a + t * (psi_b - psi_a), 1e-9);
            worst_flat = worst_flat.max((point.lat - (a.lat + t * (b.lat - a.lat))).abs());
        }
        assert!(
            worst_flat > 0.2,
            "the path should bow off a straight line in latitude, worst was {worst_flat}"
        );
    }

    /// A ring is at a true distance from its centre at every bearing.
    ///
    /// Including one that swallows a pole, which is where a circle on the
    /// ground stops being anything like a circle on the map: every longitude
    /// is inside it, and the ring's northmost point is south of its centre.
    #[test]
    fn a_geodesic_ring_is_everywhere_the_same_distance_from_its_centre() {
        for (centre, radius_m) in [
            (ll(0.0, 0.0), 500_000.0),
            (ll(-30.0, 62.0), 2_000_000.0),
            (ll(140.0, 78.0), 2_000_000.0), // reaches past the pole
        ] {
            let ring = geodesic_ring(centre, radius_m);
            for point in &ring {
                close(centre.distance_m(*point), radius_m, 1e-3);
            }
            assert_eq!(
                ring.first().map(|p| p.lat),
                ring.last().map(|p| p.lat),
                "the ring must close"
            );
        }

        // Past the pole: the ring covers every longitude, and its far side has
        // come back down the other meridian.
        let over = geodesic_ring(ll(140.0, 78.0), 2_000_000.0);
        let north = over.iter().fold(f64::MIN, |m, p| m.max(p.lat));
        assert!(
            north < 90.0,
            "a ring cannot reach past the pole, got {north}"
        );
        let spread = over
            .iter()
            .map(|p| normalize_lon(p.lon - 140.0))
            .fold(f64::MIN, f64::max)
            - over
                .iter()
                .map(|p| normalize_lon(p.lon - 140.0))
                .fold(f64::MAX, f64::min);
        assert!(
            spread > 300.0,
            "a ring over the pole spans every longitude, got {spread}"
        );
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
