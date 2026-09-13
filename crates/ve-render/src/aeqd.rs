//! Object-local azimuthal-equidistant frames.
//!
//! Every object defines a frame centred on its anchor (spec.md 7.2). Geometry
//! lives in that frame, in metres, and a grid cell is tested against it by
//! measuring the geodesic distance and bearing from the anchor. That single
//! construction gives three things for free:
//!
//! - The antimeridian and the poles stop being special cases, because distance
//!   and bearing are continuous across both.
//! - Rotation is a true bearing rotation rather than a shear of lat/lon space.
//! - Scale is in true metres, so a 500 km brush is 500 km at any latitude.
//!
//! # Geometry here, directions elsewhere
//!
//! **This frame is used for geometry only — never to derive a direction.**
//!
//! An azimuthal-equidistant projection preserves bearings *from its centre*,
//! but local grid north at a distant point is not true north; the two diverge
//! by the meridian convergence, which reaches tens of degrees a few thousand
//! kilometres out. Taking a vector's heading from a local frame angle would
//! therefore be quietly wrong for large objects, in a way no small test would
//! catch.
//!
//! So every direction the evaluator produces comes from a geographic bearing
//! instead: a constant heading is a true azimuth already, "point at a target"
//! is the bearing from the cell to the target, and a circle's rotation is
//! defined against the bearing from the anchor. All exact, everywhere.
//!
//! # The projected frame
//!
//! One object kind does not want a ground frame at all. A stamp sized in
//! pixels is asking for a shape on the *map*, and a ground disc is an ellipse
//! there — wider than it is tall by `1 / cos(lat)`, which at 60 degrees is
//! twice (spec.md 3.5, 5.1). [`Space::Projected`] is for those: its local
//! metres are degrees times the metres-per-degree of latitude, so a circle in
//! the frame is a circle on the map at every latitude and every zoom.
//!
//! The AEQD rule above still governs every geodesic object, which is all of
//! them by default. What the projected frame gives up is ground fidelity:
//! `Space::Projected` metres are north-equivalent, so an eastward metre in the
//! frame is `cos(lat)` metres of ground. It gives up nothing at the poles or
//! the antimeridian — there is no cosine to divide by, and a longitude delta is
//! normalised the same way distance and bearing normalise it.

use ve_core::LonLat;
use ve_core::angle::Angle;
use ve_core::geo::{EARTH_RADIUS_M, normalize_lon};

/// A point in an object's local frame, in metres.
pub type Local = [f64; 2];

/// Metres per degree of latitude, and per degree of longitude at the equator.
///
/// Derived from the one earth radius rather than written out, so the frame, the
/// GRIB shape-of-earth code and every distance stay the same earth.
pub const M_PER_DEGREE: f64 = EARTH_RADIUS_M * std::f64::consts::PI / 180.0;

/// Which space an object's geometry is defined in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Space {
    /// Ground space: local metres are true metres, in an azimuthal-equidistant
    /// frame about the anchor. A circle here is a geodesic cap.
    #[default]
    Geodesic,
    /// Map space: local metres are degrees scaled by [`M_PER_DEGREE`]. A circle
    /// here is a circle in equirectangular projection at every latitude — and
    /// an ellipse on the ground, narrowed east-west by `cos(lat)`.
    ///
    /// Equirectangular specifically, and not whichever projection the map
    /// happens to be showing: the space is frozen into the object and reaches
    /// the export, so it cannot depend on a view setting (spec §5.1, D63).
    Projected,
    /// Mercator map metres, frozen when a pixel tool creates the object.
    Mercator,
    /// Miller map metres, frozen when a pixel tool creates the object.
    Miller,
}

impl Space {
    /// Stable index shared with the schema, file format, and compute shader.
    pub fn from_choice(index: u8) -> Self {
        match index {
            1 => Self::Projected,
            2 => Self::Mercator,
            3 => Self::Miller,
            _ => Self::Geodesic,
        }
    }

    /// Latitude in equatorial projected degrees.
    pub fn y_of(self, lat: f64) -> f64 {
        let (lat, factor) = match self {
            Self::Mercator => (lat.clamp(-89.999, 89.999), 1.0),
            Self::Miller => (lat.clamp(-90.0, 90.0), 0.8),
            _ => return lat,
        };
        ((std::f64::consts::FRAC_PI_4 + factor * lat.to_radians() / 2.0)
            .tan()
            .ln()
            / factor)
            .to_degrees()
    }

    /// Inverse of the frozen cylindrical projection.
    pub fn lat_of(self, y: f64) -> f64 {
        let factor = match self {
            Self::Mercator => 1.0,
            Self::Miller => 0.8,
            _ => return y.clamp(-90.0, 90.0),
        };
        ((2.0 * (factor * y.to_radians()).exp().atan() - std::f64::consts::FRAC_PI_2) / factor)
            .to_degrees()
            .clamp(-90.0, 90.0)
    }
}

/// An object's local frame: azimuthal-equidistant, or projected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// Where the frame is centred.
    pub anchor: LonLat,
    /// True-bearing rotation applied to the geometry, degrees.
    pub rotation_deg: f64,
    /// Geometry scale, as a multiplier. 1.0 is unscaled.
    pub scale: f64,
    /// Which space the geometry is defined in.
    pub space: Space,
}

impl Frame {
    /// Builds a ground frame. A non-positive or non-finite scale falls back to 1.
    pub fn new(anchor: LonLat, rotation_deg: f64, scale_pct: f64) -> Self {
        Self::in_space(anchor, rotation_deg, scale_pct, Space::Geodesic)
    }

    /// Builds a frame in `space`.
    pub fn in_space(anchor: LonLat, rotation_deg: f64, scale_pct: f64, space: Space) -> Self {
        let scale = scale_pct / 100.0;
        let scale = if scale.is_finite() && scale > 1e-6 {
            scale
        } else {
            1.0
        };
        Self {
            anchor,
            rotation_deg,
            scale,
            space,
        }
    }

    /// Projects a geographic position into the frame.
    ///
    /// Returns metres in unscaled geometry units, so the result can be compared
    /// directly against the object's stored geometry.
    pub fn to_local(&self, position: LonLat) -> Local {
        if self.space != Space::Geodesic {
            // Map space: the offset in degrees, scaled to metres. No cosine, so
            // a circle here is a circle on the map wherever the object sits.
            let east = normalize_lon(position.lon - self.anchor.lon) * M_PER_DEGREE;
            let north =
                (self.space.y_of(position.lat) - self.space.y_of(self.anchor.lat)) * M_PER_DEGREE;
            return self.unrotate([east / self.scale, north / self.scale]);
        }
        let distance = self.anchor.distance_m(position);
        if distance < 1e-9 {
            return [0.0, 0.0];
        }
        let bearing = self.anchor.initial_bearing(position).degrees();
        let local = (bearing - self.rotation_deg).to_radians();
        let radius = distance / self.scale;
        [radius * local.sin(), radius * local.cos()]
    }

    /// Lifts a local point back to a geographic position.
    pub fn to_global(&self, local: Local) -> LonLat {
        if self.space != Space::Geodesic {
            let [east, north] = self.rotate(local);
            let lon = self.anchor.lon + east * self.scale / M_PER_DEGREE;
            let lat = self
                .space
                .lat_of(self.space.y_of(self.anchor.lat) + north * self.scale / M_PER_DEGREE);
            // Latitude is clamped rather than wrapped: a shape reaching past a
            // pole flattens against it, which is what it looks like on the map.
            // `new` normalises the longitude and only rejects non-finite input.
            return LonLat::new(lon, lat.clamp(-90.0, 90.0)).unwrap_or(self.anchor);
        }
        let radius = local[0].hypot(local[1]);
        if radius < 1e-9 {
            return self.anchor;
        }
        let bearing = Angle::new(local[0].atan2(local[1]).to_degrees() + self.rotation_deg);
        self.anchor.destination(bearing, radius * self.scale)
    }

    /// Turns a map-space offset into the object's rotated frame.
    ///
    /// The rotation is a bearing, clockwise from north, so it turns the axes
    /// the same way [`Self::to_local`] does in a ground frame.
    fn unrotate(&self, offset: Local) -> Local {
        let theta = self.rotation_deg.to_radians();
        let (sin, cos) = theta.sin_cos();
        [
            offset[0] * cos - offset[1] * sin,
            offset[0] * sin + offset[1] * cos,
        ]
    }

    /// The inverse of [`Self::unrotate`].
    fn rotate(&self, local: Local) -> Local {
        let theta = self.rotation_deg.to_radians();
        let (sin, cos) = theta.sin_cos();
        [
            local[0] * cos + local[1] * sin,
            -local[0] * sin + local[1] * cos,
        ]
    }

    /// True bearing from the anchor to `position`.
    ///
    /// The outward bearing from the anchor, which a circle's tangential flow is
    /// a quarter turn from. Exact at any distance, which is why directions are
    /// taken from here rather than from a local frame angle.
    pub fn radial_bearing(&self, position: LonLat) -> Angle {
        self.anchor.initial_bearing(position)
    }

    /// Distance from the anchor, in metres on the globe.
    pub fn distance_m(&self, position: LonLat) -> f64 {
        self.anchor.distance_m(position)
    }

    /// The greatest geographic distance a point at local radius `r` can reach.
    ///
    /// Used to build the spherical cap the evaluator culls against.
    ///
    /// The same bound serves both spaces. A projected metre is north-equivalent:
    /// due north it is exactly a ground metre, and in every other direction it
    /// is fewer, so `r * scale` over-estimates the reach and the cull stays
    /// conservative — which is the only direction a cull may err in.
    pub fn reach_m(&self, local_radius: f64) -> f64 {
        local_radius * self.scale
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

    #[test]
    fn the_anchor_is_the_origin() {
        let frame = Frame::new(ll(-30.0, 45.0), 0.0, 100.0);
        let local = frame.to_local(frame.anchor);
        close(local[0], 0.0, 1e-6);
        close(local[1], 0.0, 1e-6);
    }

    /// Local +y is north and +x is east when the object is unrotated.
    #[test]
    fn local_axes_point_north_and_east() {
        let frame = Frame::new(ll(0.0, 0.0), 0.0, 100.0);
        let north = frame.to_local(frame.anchor.destination(Angle::new(0.0), 100_000.0));
        close(north[0], 0.0, 1.0);
        close(north[1], 100_000.0, 1.0);

        let east = frame.to_local(frame.anchor.destination(Angle::new(90.0), 100_000.0));
        close(east[0], 100_000.0, 1.0);
        close(east[1], 0.0, 1.0);
    }

    #[test]
    fn projecting_and_lifting_round_trips() {
        let frame = Frame::new(ll(140.0, -25.0), 37.0, 160.0);
        for point in [
            ll(140.0, -25.0),
            ll(141.0, -24.0),
            ll(150.0, -30.0),
            ll(120.0, -10.0),
        ] {
            let back = frame.to_global(frame.to_local(point));
            close(back.lon, point.lon, 1e-6);
            close(back.lat, point.lat, 1e-6);
        }
    }

    /// Rotation turns the geometry by a true bearing: a point due north of the
    /// anchor lands on local +y only when the object is unrotated.
    #[test]
    fn rotation_turns_the_geometry() {
        let anchor = ll(0.0, 0.0);
        let north = anchor.destination(Angle::new(0.0), 200_000.0);

        let turned = Frame::new(anchor, 90.0, 100.0);
        let local = turned.to_local(north);
        // Rotating the object 90 degrees clockwise puts true north on local -x.
        close(local[0], -200_000.0, 2.0);
        close(local[1], 0.0, 2.0);
    }

    #[test]
    fn scale_divides_the_local_radius() {
        let anchor = ll(0.0, 0.0);
        let point = anchor.destination(Angle::new(45.0), 400_000.0);

        let doubled = Frame::new(anchor, 0.0, 200.0);
        let local = doubled.to_local(point);
        // At 200% the same ground distance is half as far in geometry units,
        // so a fixed shape covers twice the ground.
        close(local[0].hypot(local[1]), 200_000.0, 1.0);
    }

    #[test]
    fn a_degenerate_scale_falls_back_to_unscaled() {
        for pct in [0.0, -50.0, f64::NAN] {
            assert_eq!(Frame::new(ll(0.0, 0.0), 0.0, pct).scale, 1.0);
        }
    }

    /// The property the whole construction exists for: a fixed local radius is
    /// the same ground distance wherever the object sits, including at a pole
    /// and across the dateline.
    #[test]
    fn a_local_radius_is_the_same_distance_at_any_latitude() {
        for anchor in [
            ll(0.0, 0.0),
            ll(0.0, 70.0),
            ll(179.5, 45.0),
            ll(0.0, 89.5),
            ll(-120.0, -80.0),
        ] {
            let frame = Frame::new(anchor, 0.0, 100.0);
            for bearing in [0.0, 45.0, 90.0, 180.0, 300.0] {
                let point = anchor.destination(Angle::new(bearing), 500_000.0);
                let radius = {
                    let l = frame.to_local(point);
                    l[0].hypot(l[1])
                };
                close(radius, 500_000.0, 1.0);
            }
        }
    }

    /// Crossing the dateline must not produce a discontinuity in local space.
    #[test]
    fn local_coordinates_are_continuous_across_the_antimeridian() {
        let frame = Frame::new(ll(179.8, 0.0), 0.0, 100.0);
        let mut previous: Option<Local> = None;
        for step in 0..20 {
            let lon = 179.0 + f64::from(step) * 0.1;
            let local = frame.to_local(ll(lon, 0.0));
            if let Some(prev) = previous {
                let jump = (local[0] - prev[0]).hypot(local[1] - prev[1]);
                // 0.1 degrees at the equator is about 11 km; a seam would be
                // thousands.
                assert!(jump < 20_000.0, "jump of {jump} m at lon {lon}");
            }
            previous = Some(local);
        }
    }

    #[test]
    fn the_radial_bearing_is_a_true_azimuth() {
        let anchor = ll(10.0, 20.0);
        let frame = Frame::new(anchor, 45.0, 250.0);
        for bearing in [0.0, 73.0, 180.0, 271.0] {
            let point = anchor.destination(Angle::new(bearing), 300_000.0);
            // Rotation and scale must not disturb it: a circle's rotation is
            // defined against the ground, not against the object's geometry.
            close(frame.radial_bearing(point).degrees(), bearing, 1e-6);
        }
    }

    // --- The projected frame ---

    fn projected(anchor: LonLat, rotation: f64) -> Frame {
        Frame::in_space(anchor, rotation, 100.0, Space::Projected)
    }

    /// The property the projected frame exists for: a fixed local radius is the
    /// same number of *degrees* at every latitude, so it is the same number of
    /// pixels on an equirectangular map. The ground frame is the opposite by
    /// construction, which is the whole distinction.
    #[test]
    fn a_projected_radius_is_the_same_angle_at_any_latitude() {
        let radius = 2.0 * M_PER_DEGREE;
        for lat in [0.0, 45.0, 70.0, -80.0, 89.0] {
            let frame = projected(ll(0.0, lat), 0.0);
            // Two degrees east and two degrees north both land on the circle of
            // radius `radius`, however far from the equator that is.
            let east = frame.to_local(ll(2.0, lat));
            let north = frame.to_local(ll(0.0, lat - 2.0));
            close(east[0].hypot(east[1]), radius, 1.0);
            close(north[0].hypot(north[1]), radius, 1.0);
        }
    }

    /// The same circle covers less ground east-west the further from the
    /// equator: that is the trade the projected frame makes, stated as a test
    /// so it cannot be mistaken for a bug.
    #[test]
    fn a_projected_circle_narrows_on_the_ground() {
        let frame = projected(ll(0.0, 60.0), 0.0);
        let radius = M_PER_DEGREE;

        // A point one degree east is on the circle in the frame...
        let east = frame.to_local(ll(1.0, 60.0));
        close(east[0].hypot(east[1]), radius, 1.0);
        // ...but only about half a degree's worth of ground away, because
        // cos(60) is a half.
        let ground = frame.anchor.distance_m(ll(1.0, 60.0));
        close(ground, radius * 60.0_f64.to_radians().cos(), 500.0);
    }

    #[test]
    fn projected_axes_point_north_and_east() {
        let frame = projected(ll(0.0, 40.0), 0.0);
        let north = frame.to_local(ll(0.0, 41.0));
        close(north[0], 0.0, 1e-6);
        close(north[1], M_PER_DEGREE, 1e-6);

        let east = frame.to_local(ll(1.0, 40.0));
        close(east[0], M_PER_DEGREE, 1e-6);
        close(east[1], 0.0, 1e-6);
    }

    /// Rotation is a bearing here too, so a rotated projected object turns the
    /// same way a rotated ground object does.
    #[test]
    fn projected_rotation_turns_the_geometry() {
        let frame = projected(ll(0.0, 0.0), 90.0);
        let north = frame.to_local(ll(0.0, 2.0));
        close(north[0], -2.0 * M_PER_DEGREE, 1e-6);
        close(north[1], 0.0, 1e-6);
    }

    #[test]
    fn projecting_and_lifting_round_trips_in_map_space() {
        let frame = projected(ll(140.0, -25.0), 37.0);
        for point in [
            ll(140.0, -25.0),
            ll(141.0, -24.0),
            ll(150.0, -30.0),
            ll(120.0, -10.0),
        ] {
            let back = frame.to_global(frame.to_local(point));
            close(back.lon, point.lon, 1e-9);
            close(back.lat, point.lat, 1e-9);
        }
    }

    /// Longitude deltas are normalised, so the dateline is an ordinary case in
    /// map space as well.
    #[test]
    fn projected_coordinates_are_continuous_across_the_antimeridian() {
        let frame = projected(ll(179.8, 0.0), 0.0);
        let mut previous: Option<Local> = None;
        for step in 0..20 {
            let lon = 179.0 + f64::from(step) * 0.1;
            let local = frame.to_local(ll(lon, 0.0));
            if let Some(prev) = previous {
                let jump = (local[0] - prev[0]).hypot(local[1] - prev[1]);
                assert!(jump < 20_000.0, "jump of {jump} m at lon {lon}");
            }
            previous = Some(local);
        }
        // ...and the far side of it is where it should be, not a world away.
        let across = frame.to_local(ll(-179.8, 0.0));
        close(across[0], 0.4 * M_PER_DEGREE, 1.0);
    }

    /// Nothing divides by `cos(lat)`, so the pole is an ordinary point rather
    /// than a singularity.
    #[test]
    fn the_projected_frame_is_finite_at_the_pole() {
        let frame = projected(ll(0.0, 89.5), 0.0);
        for lon in [0.0, 90.0, 179.0, -90.0] {
            let local = frame.to_local(ll(lon, 89.5));
            assert!(local[0].is_finite() && local[1].is_finite());
            assert!(local[0].abs() <= 180.0 * M_PER_DEGREE);
        }
        // Lifting past the pole flattens against it rather than wrapping over.
        let over = frame.to_global([0.0, 2.0 * M_PER_DEGREE]);
        close(over.lat, 90.0, 1e-9);
    }

    #[test]
    fn projected_scale_divides_the_local_offset() {
        let doubled = Frame::in_space(ll(0.0, 0.0), 0.0, 200.0, Space::Projected);
        let local = doubled.to_local(ll(4.0, 0.0));
        close(local[0], 2.0 * M_PER_DEGREE, 1e-6);
    }

    /// Directions never come from the frame, so a circle's rotation is
    /// unaffected by the space the geometry lives in.
    #[test]
    fn the_radial_bearing_is_a_true_azimuth_in_either_space() {
        let anchor = ll(10.0, 60.0);
        for space in [Space::Geodesic, Space::Projected] {
            let frame = Frame::in_space(anchor, 45.0, 250.0, space);
            for bearing in [0.0, 73.0, 180.0, 271.0] {
                let point = anchor.destination(Angle::new(bearing), 300_000.0);
                close(frame.radial_bearing(point).degrees(), bearing, 1e-6);
            }
        }
    }

    /// The cull must never cut a shape short. A projected metre is at most a
    /// ground metre, so the bound holds in every direction.
    #[test]
    fn projected_reach_bounds_the_ground_distance() {
        let radius = 3.0 * M_PER_DEGREE;
        for lat in [0.0, 45.0, 80.0] {
            let frame = projected(ll(0.0, lat), 0.0);
            let reach = frame.reach_m(radius);
            for bearing in [0.0, 45.0, 90.0, 135.0, 180.0, 270.0] {
                // Walk to the edge of the shape in map space, then measure how
                // far that actually is on the ground.
                let theta = f64::from(bearing as i32).to_radians();
                let edge = frame.to_global([radius * theta.sin(), radius * theta.cos()]);
                assert!(
                    frame.anchor.distance_m(edge) <= reach + 1.0,
                    "reach {reach} too small at lat {lat}, bearing {bearing}"
                );
            }
        }
    }

    #[test]
    fn reach_accounts_for_scale() {
        let frame = Frame::new(ll(0.0, 0.0), 0.0, 300.0);
        close(frame.reach_m(100_000.0), 300_000.0, 1e-6);
    }
}

#[cfg(test)]
mod cylindrical_tests {
    use super::*;
    #[test]
    fn pixel_perimeters_are_circular_at_every_latitude_in_their_projection() {
        for space in [Space::Projected, Space::Mercator, Space::Miller] {
            for lat in [-75.0, 0.0, 60.0, 75.0] {
                let frame = Frame::in_space(LonLat::new(179.0, lat).unwrap(), 0.0, 100.0, space);
                for i in 0..64 {
                    let t = f64::from(i) * std::f64::consts::TAU / 64.0;
                    let local = [t.cos() * M_PER_DEGREE, t.sin() * M_PER_DEGREE];
                    let global = frame.to_global(local);
                    let x = normalize_lon(global.lon - frame.anchor.lon);
                    let y = space.y_of(global.lat) - space.y_of(lat);
                    assert!((x.hypot(y) - 1.0).abs() < 1e-9);
                    let back = frame.to_local(global);
                    assert!((back[0] - local[0]).hypot(back[1] - local[1]) < 1e-6);
                }
            }
        }
    }
}
