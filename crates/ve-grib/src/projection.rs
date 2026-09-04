//! The map projections a GRIB grid can be laid out on.
//!
//! A regular lat/lon grid (template 3.0) carries its own geometry: the value
//! at row `j`, column `i` is at a longitude and a latitude that follow from
//! two increments. Every other grid in this module is a regular lattice in
//! **some other plane**, and turning a node index into a position on the
//! earth means inverting a projection. That is the whole of what this module
//! does, plus the one thing that falls out of it and is easy to get silently
//! wrong: the angle between the grid's north and the earth's.
//!
//! Three plane projections and one sphere rotation cover everything the
//! forecast centres put regional models on:
//!
//! | Template | Projection | Plane unit |
//! |---|---|---|
//! | 3.10 | Mercator | metres |
//! | 3.20 | Polar stereographic | metres |
//! | 3.30 | Lambert conformal conic | metres |
//! | 3.1, 3.32769 | Rotated lat/lon | degrees of the rotated sphere |
//!
//! **The formulas are ellipsoidal, and reduce to the spherical ones exactly
//! when the eccentricity is zero.** That is not a flourish: code table 3.2
//! lets a file name a sphere (most do) or WGS 84 (NCEP's wave grids do), and
//! carrying one code path rather than two means the sphere is tested by every
//! file. Snyder's *Map Projections — A Working Manual* is the reference;
//! equation numbers are cited where a constant would otherwise be a mystery.
//!
//! **`convergence` is the reason this module is not just geometry.** GRIB
//! flag table 3.5 lets a message resolve `u` and `v` along the *grid* axes
//! rather than along east and north, and most regional models do. Reading
//! those components as if they were eastward and northward is not a small
//! error — it is 30° or more over the width of a Lambert CONUS grid, and it
//! looks entirely plausible on screen.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

use crate::error::{GribError, Result};

/// The earth a grid's projection is defined on (code table 3.2).
///
/// Only two numbers of it matter here: the semi-major axis and the
/// eccentricity. A sphere is the `e2 == 0` case and needs no separate path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Earth {
    /// Semi-major axis, metres.
    pub a: f64,
    /// First eccentricity squared, `(a² - b²) / a²`. Zero for a sphere.
    pub e2: f64,
}

impl Earth {
    /// A sphere of the given radius.
    pub const fn sphere(radius: f64) -> Self {
        Self { a: radius, e2: 0.0 }
    }

    /// An ellipsoid from its two axes, in metres.
    fn ellipsoid(a: f64, b: f64) -> Result<Self> {
        if !(a.is_finite() && b.is_finite()) || a <= 0.0 || b <= 0.0 || b > a {
            return Err(GribError::Malformed(format!(
                "an earth with axes {a} and {b} metres"
            )));
        }
        Ok(Self {
            a,
            e2: (a * a - b * b) / (a * a),
        })
    }

    /// First eccentricity.
    pub fn e(&self) -> f64 {
        self.e2.sqrt()
    }

    /// The earth code table 3.2 describes, from section 3's octets 15-30.
    ///
    /// The scaled fields are only read for the codes that use them, because a
    /// file that names a fixed shape is free to leave them missing — and many
    /// do, filling all fifteen octets with `0xff`.
    pub fn of_code(
        shape: u8,
        radius_scale: u8,
        radius: u32,
        major_scale: u8,
        major: u32,
        minor_scale: u8,
        minor: u32,
    ) -> Result<Self> {
        // A scale factor is a negative power of ten: value = scaled / 10^s.
        // Both are "missing" as all-ones, which is not a number at all.
        let scaled = |scale: u8, value: u32, what: &str| -> Result<f64> {
            if scale == 0xff || value == u32::MAX {
                return Err(GribError::Malformed(format!(
                    "an earth whose {what} is missing"
                )));
            }
            Ok(f64::from(value) / 10f64.powi(i32::from(scale)))
        };
        Ok(match shape {
            0 => Self::sphere(6_367_470.0),
            1 => Self::sphere(scaled(radius_scale, radius, "radius")?),
            2 => Self::ellipsoid(6_378_160.0, 6_356_775.0)?,
            // Code 3 states its axes in kilometres, code 7 in metres. The
            // only difference between the two is the factor of a thousand.
            3 => Self::ellipsoid(
                scaled(major_scale, major, "major axis")? * 1000.0,
                scaled(minor_scale, minor, "minor axis")? * 1000.0,
            )?,
            4 | 5 => {
                // IAG-GRS80 and WGS 84 differ in the eleventh digit of the
                // flattening, which is far below anything a grid can express.
                Self::ellipsoid(6_378_137.0, 6_356_752.314_245)?
            }
            6 => Self::sphere(6_371_229.0),
            7 => Self::ellipsoid(
                scaled(major_scale, major, "major axis")?,
                scaled(minor_scale, minor, "minor axis")?,
            )?,
            8 => Self::sphere(6_371_200.0),
            9 => Self::ellipsoid(6_377_563.396, 6_356_256.909)?,
            other => {
                return Err(GribError::Unsupported(format!(
                    "shape of the earth {other} (code table 3.2)"
                )));
            }
        })
    }
}

// --- Snyder's conformal helpers ---------------------------------------------

/// Snyder eq. 15-9: the conformal latitude factor the conic and stereographic
/// projections are both built from. Falls to `tan(π/4 - φ/2)` on a sphere,
/// and to zero at the north pole.
fn snyder_t(e: f64, lat: f64) -> f64 {
    let s = lat.sin();
    (FRAC_PI_4 - lat / 2.0).tan() / ((1.0 - e * s) / (1.0 + e * s)).powf(e / 2.0)
}

/// Snyder eq. 14-15: the radius of the parallel at `lat`, as a fraction of
/// the semi-major axis. `cos φ` on a sphere.
fn snyder_m(e2: f64, lat: f64) -> f64 {
    lat.cos() / (1.0 - e2 * lat.sin().powi(2)).sqrt()
}

/// Inverts [`snyder_t`] (Snyder eq. 7-9), by the iteration Snyder gives.
///
/// One pass is exact on a sphere; on WGS 84 it converges to the last bit of
/// an `f64` in four or five, and the loop is bounded so a pathological
/// eccentricity cannot hang an import.
fn lat_from_t(e: f64, t: f64) -> f64 {
    let mut lat = FRAC_PI_2 - 2.0 * t.atan();
    for _ in 0..16 {
        let s = lat.sin();
        let next = FRAC_PI_2 - 2.0 * (t * ((1.0 - e * s) / (1.0 + e * s)).powf(e / 2.0)).atan();
        let done = (next - lat).abs() < 1e-15;
        lat = next;
        if done {
            break;
        }
    }
    lat
}

/// Wraps a longitude difference into `(-180, 180]`, in degrees.
///
/// Every projection here measures longitude from a reference meridian, and
/// the difference has to take the short way round or a grid that straddles
/// the antimeridian folds in half.
pub fn wrap180(degrees: f64) -> f64 {
    let wrapped = (degrees + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped <= -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

// --- The rotated sphere ------------------------------------------------------

/// The rotated frame templates 3.1 and 3.32769 lay their grids out on.
///
/// Held as an orthonormal basis rather than as two angles: the forward and
/// the inverse are then three dot products and a linear combination, with no
/// trigonometric identity to get backwards and no special case at the seam.
///
/// * `z` is the rotated frame's north pole.
/// * `x` is its `(0°, 0°)`, which the templates place on the meridian of the
///   projection's southern pole, 90° from it.
/// * `y = z × x` is its `(0°, 90°E)`, which makes the frame right-handed and
///   rotated longitude increase eastward like the real one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotatedPole {
    z: [f64; 3],
    x: [f64; 3],
    y: [f64; 3],
}

fn unit(lat_deg: f64, lon_deg: f64) -> [f64; 3] {
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalized(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(v, v).sqrt();
    (length > 1e-12).then(|| [v[0] / length, v[1] / length, v[2] / length])
}

impl RotatedPole {
    /// From the **southern pole of the projection**, which is how both
    /// templates name the rotation.
    ///
    /// The rotated north pole is that point's antipode, and rotated `(0, 0)`
    /// is the point 90° from it along the southern pole's own meridian — so
    /// `(-90, 0)`, the pole where it already is, gives back the identity.
    pub fn from_south_pole(lat_sp: f64, lon_sp: f64) -> Self {
        let z = unit(-lat_sp, lon_sp + 180.0);
        let x = unit(90.0 + lat_sp, lon_sp);
        Self {
            z,
            x,
            y: cross(z, x),
        }
    }

    /// From the **centre of the grid**, which is how NCEP's template 3.32769
    /// names the same rotation: the centre is rotated `(0, 0)`.
    pub fn from_centre(lat: f64, lon: f64) -> Self {
        Self::from_south_pole(lat - 90.0, lon)
    }

    /// Geographic degrees to rotated degrees, longitude first.
    pub fn to_rotated(&self, lat: f64, lon: f64) -> (f64, f64) {
        let p = unit(lat, lon);
        let lat_r = dot(p, self.z).clamp(-1.0, 1.0).asin().to_degrees();
        let lon_r = dot(p, self.y).atan2(dot(p, self.x)).to_degrees();
        (lon_r, lat_r)
    }

    /// Rotated degrees back to geographic ones, latitude first.
    pub fn from_rotated(&self, lon_r: f64, lat_r: f64) -> (f64, f64) {
        let (lat_r, lon_r) = (lat_r.to_radians(), lon_r.to_radians());
        let (c, s) = (lat_r.cos(), lat_r.sin());
        let (cx, cy) = (c * lon_r.cos(), c * lon_r.sin());
        let mut p = [0.0; 3];
        for (k, slot) in p.iter_mut().enumerate() {
            *slot = cx * self.x[k] + cy * self.y[k] + s * self.z[k];
        }
        (
            p[2].clamp(-1.0, 1.0).asin().to_degrees(),
            p[1].atan2(p[0]).to_degrees(),
        )
    }

    /// The angle from true north to rotated north at a point, clockwise, in
    /// radians.
    ///
    /// Both directions are the component of a pole's direction perpendicular
    /// to the point, so this is one subtraction and two dot products and has
    /// no seam and no hemisphere case. It is zero where either direction is
    /// undefined — at the poles themselves, where a `u`/`v` pair means
    /// nothing anyway.
    fn convergence(&self, lat: f64, lon: f64) -> f64 {
        let p = unit(lat, lon);
        let Some(east) = normalized(cross([0.0, 0.0, 1.0], p)) else {
            return 0.0;
        };
        let north = cross(p, east);
        let Some(grid_north) = normalized([
            self.z[0] - dot(self.z, p) * p[0],
            self.z[1] - dot(self.z, p) * p[1],
            self.z[2] - dot(self.z, p) * p[2],
        ]) else {
            return 0.0;
        };
        dot(grid_north, east).atan2(dot(grid_north, north))
    }
}

// --- The projections ---------------------------------------------------------

/// A grid's plane, and the map between it and the earth.
///
/// The plane's unit is metres for the three map projections and degrees of
/// the rotated sphere for [`Projection::Rotated`]; a grid's increments are in
/// whichever its projection uses, so nothing outside this module needs to
/// know which.
///
/// The two projections whose `x` is periodic in longitude — Mercator and the
/// rotated sphere — measure it from a reference meridian **the grid itself
/// supplies**, so that `x = 0` at the grid's first column. The conic and
/// stereographic ones cannot: their plane is a fan about `lov`, single-valued
/// everywhere but the one meridian opposite it, which no grid can cross
/// without overlapping itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    /// Template 3.10. `lad` is the parallel the grid length is true at.
    Mercator {
        /// True-scale latitude, degrees.
        lad: f64,
        /// The meridian `x = 0` sits on, degrees.
        lon_ref: f64,
    },
    /// Template 3.20.
    PolarStereographic {
        /// Orientation: the meridian that runs straight up the grid.
        lov: f64,
        /// True-scale latitude, degrees, as a magnitude.
        lad: f64,
        /// Whether the projection is centred on the north pole.
        north: bool,
    },
    /// Template 3.30.
    LambertConformal {
        /// Orientation: the meridian that runs straight up the grid.
        lov: f64,
        /// First standard parallel, degrees.
        latin1: f64,
        /// Second standard parallel, degrees. Equal to the first for a
        /// tangent cone.
        latin2: f64,
        /// Whether the cone's apex is the north pole.
        north: bool,
    },
    /// Templates 3.1 and 3.32769: not a plane projection at all, but a
    /// sphere with its pole moved.
    Rotated {
        /// The rotation.
        pole: RotatedPole,
        /// The rotated meridian `x = 0` sits on, degrees.
        lon_ref: f64,
    },
}

/// The constants a conic or stereographic projection is evaluated through,
/// worked out once when the grid is parsed rather than per point.
#[derive(Debug, Clone, Copy)]
struct Conic {
    /// Cone constant, always positive: `sin(latin)` for a tangent cone, 1
    /// for the stereographic limit.
    n: f64,
    /// `ρ = a · f · t^n`.
    f: f64,
    /// `+1` for a projection about the north pole, `-1` about the south.
    hemisphere: f64,
    /// Orientation meridian, degrees.
    lov: f64,
}

impl Conic {
    /// `ρ`, the distance from the cone's apex, for a geographic latitude.
    fn rho(&self, earth: &Earth, lat_deg: f64) -> f64 {
        let lat = (self.hemisphere * lat_deg).to_radians();
        earth.a * self.f * snyder_t(earth.e(), lat).powf(self.n)
    }

    /// `θ`, the angle about the apex, in radians. This is also the grid
    /// convergence up to the hemisphere's sign.
    fn theta(&self, lon_deg: f64) -> f64 {
        self.n * wrap180(lon_deg - self.lov).to_radians()
    }

    fn forward(&self, earth: &Earth, lat: f64, lon: f64) -> (f64, f64) {
        let (rho, theta) = (self.rho(earth, lat), self.theta(lon));
        (rho * theta.sin(), -self.hemisphere * rho * theta.cos())
    }

    /// The longitude alone, which is all the convergence needs and costs no
    /// iteration.
    fn lon_of(&self, x: f64, y: f64) -> f64 {
        if x == 0.0 && y == 0.0 {
            return self.lov;
        }
        self.lov + x.atan2(-self.hemisphere * y).to_degrees() / self.n
    }

    fn inverse(&self, earth: &Earth, x: f64, y: f64) -> (f64, f64) {
        let rho = (x * x + y * y).sqrt();
        let lon = self.lon_of(x, y);
        if rho <= 0.0 {
            return (self.hemisphere * 90.0, lon);
        }
        let t = (rho / (earth.a * self.f)).powf(1.0 / self.n);
        let lat = lat_from_t(earth.e(), t).to_degrees();
        (self.hemisphere * lat, lon)
    }
}

/// The stereographic limit of the cone: `n = 1`, with the scale pinned by
/// making `ρ` the true radius of the standard parallel there (Snyder
/// eq. 21-32, rearranged).
fn stereographic(earth: &Earth, lov: f64, lad: f64, north: bool) -> Conic {
    let lat_c = lad.abs().to_radians();
    Conic {
        n: 1.0,
        f: snyder_m(earth.e2, lat_c) / snyder_t(earth.e(), lat_c),
        hemisphere: if north { 1.0 } else { -1.0 },
        lov,
    }
}

/// The cone through two standard parallels (Snyder eq. 15-8), of which a
/// tangent cone is the limit as they meet.
fn lambert(earth: &Earth, lov: f64, latin1: f64, latin2: f64, north: bool) -> Conic {
    let hemisphere = if north { 1.0 } else { -1.0 };
    let (p1, p2) = (
        (hemisphere * latin1).to_radians(),
        (hemisphere * latin2).to_radians(),
    );
    let (m1, m2) = (snyder_m(earth.e2, p1), snyder_m(earth.e2, p2));
    let (t1, t2) = (snyder_t(earth.e(), p1), snyder_t(earth.e(), p2));
    let n = if (p1 - p2).abs() < 1e-9 {
        p1.sin()
    } else {
        (m1.ln() - m2.ln()) / (t1.ln() - t2.ln())
    };
    Conic {
        n,
        f: m1 / (n * t1.powf(n)),
        hemisphere,
        lov,
    }
}

/// A projection with its derived constants already worked out.
///
/// A Lambert cone costs two logarithms and two powers to *describe* and none
/// to evaluate, and a resample evaluates it a few million times. Building one
/// of these per point — which is what [`Projection`]'s own methods do, and is
/// right for a handful of them — spends most of the run rebuilding the same
/// four numbers. Everything on the hot path prepares once and evaluates per
/// node.
#[derive(Debug, Clone, Copy)]
pub struct Prepared {
    earth: Earth,
    kind: Kind,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    /// Mercator, with the plane scale `a·m(lad)` folded in.
    Mercator { k: f64, lon_ref: f64 },
    /// The conic and stereographic projections, which differ only in how
    /// their two constants are arrived at.
    Conic(Conic),
    /// The rotated sphere, whose constants are the basis itself.
    Rotated { pole: RotatedPole, lon_ref: f64 },
}

impl Prepared {
    /// Earth to plane. Longitude first, as plane coordinates are `(x, y)`.
    pub fn forward(&self, lat: f64, lon: f64) -> (f64, f64) {
        match self.kind {
            Kind::Mercator { k, lon_ref } => (
                k * wrap180(lon - lon_ref).to_radians(),
                // The isometric latitude is the log of the reciprocal of
                // Snyder's t, so the one helper serves both projections.
                -k * snyder_t(self.earth.e(), lat.to_radians()).ln(),
            ),
            Kind::Rotated { pole, lon_ref } => {
                let (lon_r, lat_r) = pole.to_rotated(lat, lon);
                (wrap180(lon_r - lon_ref), lat_r)
            }
            Kind::Conic(conic) => conic.forward(&self.earth, lat, lon),
        }
    }

    /// Plane to earth. Latitude first, as positions are `(lat, lon)`.
    pub fn inverse(&self, x: f64, y: f64) -> (f64, f64) {
        match self.kind {
            Kind::Mercator { k, lon_ref } => (
                lat_from_t(self.earth.e(), (-y / k).exp()).to_degrees(),
                lon_ref + (x / k).to_degrees(),
            ),
            Kind::Rotated { pole, lon_ref } => pole.from_rotated(x + lon_ref, y),
            Kind::Conic(conic) => conic.inverse(&self.earth, x, y),
        }
    }

    /// The angle from true north to the grid's `+y` axis at a plane point,
    /// clockwise, in radians.
    ///
    /// This is what turns grid-resolved `u`/`v` into eastward and northward
    /// components:
    ///
    /// ```text
    /// u_east  =  u_grid·cos γ + v_grid·sin γ
    /// v_north = -u_grid·sin γ + v_grid·cos γ
    /// ```
    ///
    /// For the conic and stereographic projections it is `n·(λ - λ₀)`, which
    /// needs the longitude and not the latitude — so it costs an `atan2` and
    /// none of the inverse's iteration. Mercator's is zero: its meridians are
    /// the plane's own vertical lines.
    pub fn convergence(&self, x: f64, y: f64) -> f64 {
        match self.kind {
            Kind::Mercator { .. } => 0.0,
            Kind::Rotated { pole, lon_ref } => {
                let (lat, lon) = pole.from_rotated(x + lon_ref, y);
                pole.convergence(lat, lon)
            }
            Kind::Conic(conic) => conic.hemisphere * conic.theta(conic.lon_of(x, y)),
        }
    }

    /// Where a geographic pole lands in the plane, if it lands anywhere.
    ///
    /// A grid that contains this point contains the pole, and its longitudes
    /// therefore run all the way round — which no walk of its boundary can
    /// discover. Mercator sends both poles to infinity and has none.
    pub fn pole(&self, north: bool) -> Option<(f64, f64)> {
        let (x, y) = self.forward(if north { 90.0 } else { -90.0 }, 0.0);
        (x.is_finite() && y.is_finite() && x.abs() < 1e12 && y.abs() < 1e12).then_some((x, y))
    }
}

impl Projection {
    /// The conic form of the two projections that have one.
    fn conic(&self, earth: &Earth) -> Option<Conic> {
        match *self {
            Self::Mercator { .. } | Self::Rotated { .. } => None,
            Self::PolarStereographic { lov, lad, north } => {
                Some(stereographic(earth, lov, lad, north))
            }
            Self::LambertConformal {
                lov,
                latin1,
                latin2,
                north,
            } => Some(lambert(earth, lov, latin1, latin2, north)),
        }
    }

    /// This projection with its constants worked out, for evaluating in bulk.
    pub fn prepared(&self, earth: &Earth) -> Prepared {
        let kind = match *self {
            Self::Mercator { lad, lon_ref } => Kind::Mercator {
                k: earth.a * snyder_m(earth.e2, lad.to_radians()),
                lon_ref,
            },
            Self::Rotated { pole, lon_ref } => Kind::Rotated { pole, lon_ref },
            Self::PolarStereographic { lov, lad, north } => {
                Kind::Conic(stereographic(earth, lov, lad, north))
            }
            Self::LambertConformal {
                lov,
                latin1,
                latin2,
                north,
            } => Kind::Conic(lambert(earth, lov, latin1, latin2, north)),
        };
        Prepared {
            earth: *earth,
            kind,
        }
    }

    /// Checks the projection is one this grid can actually be evaluated on.
    fn validate(&self, earth: &Earth) -> Result<()> {
        let bad = |what: String| Err(GribError::Malformed(what));
        match *self {
            Self::Mercator { lad, .. } => {
                if !(-89.9..=89.9).contains(&lad) {
                    return bad(format!("a Mercator grid true at latitude {lad}"));
                }
            }
            Self::PolarStereographic { lad, .. } => {
                if !(0.1..=89.9).contains(&lad.abs()) {
                    return bad(format!("a polar stereographic grid true at latitude {lad}"));
                }
            }
            Self::LambertConformal { latin1, latin2, .. } => {
                if !(-89.9..=89.9).contains(&latin1) || !(-89.9..=89.9).contains(&latin2) {
                    return bad(format!(
                        "a Lambert grid with standard parallels {latin1} and {latin2}"
                    ));
                }
                let n = self.conic(earth).map(|c| c.n).unwrap_or(0.0);
                if !n.is_finite() || n.abs() < 1e-6 {
                    return bad(format!(
                        "a Lambert grid whose parallels {latin1} and {latin2} make no cone"
                    ));
                }
            }
            Self::Rotated { .. } => {}
        }
        Ok(())
    }

    /// Earth to plane, for a point or two. In bulk, use [`Self::prepared`].
    pub fn forward(&self, earth: &Earth, lat: f64, lon: f64) -> (f64, f64) {
        self.prepared(earth).forward(lat, lon)
    }

    /// Plane to earth, for a point or two. In bulk, use [`Self::prepared`].
    pub fn inverse(&self, earth: &Earth, x: f64, y: f64) -> (f64, f64) {
        self.prepared(earth).inverse(x, y)
    }

    /// The grid convergence at a plane point. See [`Prepared::convergence`].
    pub fn convergence(&self, earth: &Earth, x: f64, y: f64) -> f64 {
        self.prepared(earth).convergence(x, y)
    }

    /// Where a geographic pole lands in the plane. See [`Prepared::pole`].
    pub fn pole(&self, earth: &Earth, north: bool) -> Option<(f64, f64)> {
        self.prepared(earth).pole(north)
    }

    /// Checks the projection is usable and reports why if it is not.
    pub fn checked(self, earth: &Earth) -> Result<Self> {
        self.validate(earth)?;
        Ok(self)
    }
}

/// Rotates a grid-resolved vector into eastward and northward components.
///
/// The identity at `γ = 0`, so a caller need not special-case an
/// earth-resolved message.
pub fn to_earth_relative(u: f32, v: f32, convergence: f64) -> (f32, f32) {
    let (s, c) = convergence.sin_cos();
    let (u, v) = (f64::from(u), f64::from(v));
    ((u * c + v * s) as f32, (-u * s + v * c) as f32)
}

/// Great-circle distance in metres on a sphere of the app's radius.
///
/// Used only by the tests, which check a projection against a distance the
/// projection had no part in computing.
#[cfg(test)]
fn haversine_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lat1, lat2) = (a.0.to_radians(), b.0.to_radians());
    let dlat = lat2 - lat1;
    let dlon = (b.1 - a.1).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * ve_core::EARTH_RADIUS_M * h.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPHERE: Earth = Earth::sphere(6_371_229.0);

    fn wgs84() -> Earth {
        Earth::of_code(5, 0, 0, 0, 0, 0, 0).expect("wgs84")
    }

    /// Every projection must invert to what it projected, everywhere its
    /// grids reach.
    #[test]
    fn every_projection_round_trips() {
        let cases: Vec<Projection> = vec![
            Projection::Mercator {
                lad: 20.0,
                lon_ref: 198.475,
            },
            Projection::PolarStereographic {
                lov: 210.0,
                lad: 60.0,
                north: true,
            },
            Projection::PolarStereographic {
                lov: 0.0,
                lad: 60.0,
                north: false,
            },
            Projection::LambertConformal {
                lov: 262.5,
                latin1: 33.0,
                latin2: 45.0,
                north: true,
            },
            Projection::LambertConformal {
                lov: 145.0,
                latin1: -30.0,
                latin2: -50.0,
                north: false,
            },
            Projection::Rotated {
                pole: RotatedPole::from_south_pole(-36.0885, 245.305),
                lon_ref: -14.81,
            },
        ];
        for earth in [SPHERE, wgs84()] {
            for projection in &cases {
                for lat in [-80.0, -45.0, -5.0, 0.0, 12.5, 45.0, 79.0] {
                    for lon in [-179.0, -90.0, -0.5, 0.0, 47.0, 179.5, 210.0] {
                        // A conic projection is a fan about its orientation
                        // meridian and cannot represent the antipodal one; no
                        // grid crosses that cut, and neither does this test.
                        if let Some(conic) = projection.conic(&earth) {
                            if wrap180(lon - conic.lov).abs() > 179.0 {
                                continue;
                            }
                            // The far hemisphere is where a conic projection
                            // stops being a map at all.
                            if conic.hemisphere * lat < -70.0 {
                                continue;
                            }
                        }
                        let (x, y) = projection.forward(&earth, lat, lon);
                        let (back_lat, back_lon) = projection.inverse(&earth, x, y);
                        assert!(
                            (back_lat - lat).abs() < 1e-8,
                            "{projection:?} at ({lon}, {lat}): latitude came back {back_lat}"
                        );
                        assert!(
                            wrap180(back_lon - lon).abs() < 1e-8,
                            "{projection:?} at ({lon}, {lat}): longitude came back {back_lon}"
                        );
                    }
                }
            }
        }
    }

    /// A conformal projection is true to scale on its standard parallel: one
    /// grid step along it must be one grid length on the ground.
    ///
    /// The distance on the ground is a haversine, which knows nothing about
    /// any of these projections — so this checks the projection's scale
    /// against geodesy rather than against itself.
    #[test]
    fn the_standard_parallel_is_true_to_scale() {
        let step = 10_000.0;
        let cases: Vec<(Projection, f64)> = vec![
            (
                Projection::Mercator {
                    lad: 20.0,
                    lon_ref: 0.0,
                },
                20.0,
            ),
            (
                Projection::PolarStereographic {
                    lov: 0.0,
                    lad: 60.0,
                    north: true,
                },
                60.0,
            ),
            (
                Projection::PolarStereographic {
                    lov: 0.0,
                    lad: 60.0,
                    north: false,
                },
                -60.0,
            ),
            (
                Projection::LambertConformal {
                    lov: 0.0,
                    latin1: 25.0,
                    latin2: 25.0,
                    north: true,
                },
                25.0,
            ),
            (
                Projection::LambertConformal {
                    lov: 0.0,
                    latin1: 33.0,
                    latin2: 45.0,
                    north: true,
                },
                45.0,
            ),
            (
                Projection::LambertConformal {
                    lov: 0.0,
                    latin1: -30.0,
                    latin2: -50.0,
                    north: false,
                },
                -30.0,
            ),
        ];
        for (projection, parallel) in cases {
            let (x, y) = projection.forward(&SPHERE, parallel, 0.0);
            // A step along the plane's x axis is a step along the parallel,
            // because every one of these projections has its x axis there.
            let a = projection.inverse(&SPHERE, x - step / 2.0, y);
            let b = projection.inverse(&SPHERE, x + step / 2.0, y);
            let measured = haversine_m(a, b);
            assert!(
                (measured - step).abs() < step * 2e-3,
                "{projection:?}: {step} m on the plane at {parallel}° measured {measured} m"
            );
        }
    }

    /// The convergence must be the angle the projection's own geometry has,
    /// which a finite difference of the inverse measures without using the
    /// closed form at all.
    #[test]
    fn convergence_matches_the_grid_north_the_inverse_describes() {
        let cases: Vec<(Projection, f64, f64)> = vec![
            (
                Projection::PolarStereographic {
                    lov: 210.0,
                    lad: 60.0,
                    north: true,
                },
                55.0,
                190.0,
            ),
            (
                Projection::PolarStereographic {
                    lov: 0.0,
                    lad: 60.0,
                    north: false,
                },
                -62.0,
                40.0,
            ),
            (
                Projection::LambertConformal {
                    lov: 262.5,
                    latin1: 33.0,
                    latin2: 45.0,
                    north: true,
                },
                48.0,
                285.0,
            ),
            (
                Projection::LambertConformal {
                    lov: 145.0,
                    latin1: -30.0,
                    latin2: -50.0,
                    north: false,
                },
                -42.0,
                110.0,
            ),
            (
                Projection::Rotated {
                    pole: RotatedPole::from_south_pole(-36.0885, 245.305),
                    lon_ref: 0.0,
                },
                52.0,
                300.0,
            ),
            (
                Projection::Mercator {
                    lad: 20.0,
                    lon_ref: 0.0,
                },
                30.0,
                40.0,
            ),
        ];
        for earth in [SPHERE, wgs84()] {
            for &(projection, lat, lon) in &cases {
                let (x, y) = projection.forward(&earth, lat, lon);
                // Where is the meridian in the plane? Step a little way due
                // north on the earth and see which way the plane point moved.
                // Grid north is `+y`, and the convergence is the angle from
                // one to the other — so this measures the same angle with
                // nothing but the forward projection, on an ellipsoid as
                // readily as on a sphere.
                let step = 1e-4;
                let (x_up, y_up) = projection.forward(&earth, lat + step, lon);
                let (x_dn, y_dn) = projection.forward(&earth, lat - step, lon);
                // The three map projections measure their plane in metres,
                // where an angle is an angle. The rotated sphere measures it
                // in its own degrees, which are equirectangular: a degree of
                // its longitude is `cos φr` of a degree of its latitude.
                let east = match projection {
                    Projection::Rotated { .. } => (x_dn - x_up) * y.to_radians().cos(),
                    _ => x_dn - x_up,
                };
                let measured = east.atan2(y_up - y_dn);
                let closed = projection.convergence(&earth, x, y);
                assert!(
                    (measured - closed).abs() < 1e-6,
                    "{projection:?} at ({lon}, {lat}): closed form {closed}, measured {measured}"
                );
            }
        }
    }

    /// The sign of the convergence, stated as geometry rather than as
    /// algebra: on a grid whose meridians converge at the north pole, a point
    /// east of the orientation meridian has the grid's north turned
    /// **clockwise** of the true one — so a wind blowing along the grid's
    /// `+y` axis is blowing east of true north.
    #[test]
    fn grid_north_leans_the_way_the_meridians_do() {
        let north = Projection::LambertConformal {
            lov: 260.0,
            latin1: 25.0,
            latin2: 25.0,
            north: true,
        };
        let (x, y) = north.forward(&SPHERE, 40.0, 280.0);
        assert!(north.convergence(&SPHERE, x, y) > 0.0);
        let (x, y) = north.forward(&SPHERE, 40.0, 240.0);
        assert!(north.convergence(&SPHERE, x, y) < 0.0);

        // South of the equator the apex is the south pole and the lean is the
        // other way.
        let south = Projection::LambertConformal {
            lov: 145.0,
            latin1: -30.0,
            latin2: -30.0,
            north: false,
        };
        let (x, y) = south.forward(&SPHERE, -40.0, 165.0);
        assert!(south.convergence(&SPHERE, x, y) < 0.0);
        let (x, y) = south.forward(&SPHERE, -40.0, 125.0);
        assert!(south.convergence(&SPHERE, x, y) > 0.0);
    }

    /// A vector resolved along the grid axes, rotated, must be the same
    /// vector: same length, and pointing where the grid's axes say.
    #[test]
    fn rotation_preserves_a_vector_and_turns_it_the_right_way() {
        // 30° of convergence: the grid's north is 30° east of true north, so
        // a wind blowing along the grid's +y axis is blowing toward 30°.
        let (u, v) = to_earth_relative(0.0, 10.0, 30f64.to_radians());
        assert!((u - 5.0).abs() < 1e-5, "eastward came out {u}");
        assert!((v - 8.660_254).abs() < 1e-5, "northward came out {v}");
        assert!((f64::from(u).hypot(f64::from(v)) - 10.0).abs() < 1e-5);

        // And the grid's +x axis is 30° east of true east.
        let (u, v) = to_earth_relative(10.0, 0.0, 30f64.to_radians());
        assert!((u - 8.660_254).abs() < 1e-5, "eastward came out {u}");
        assert!((v + 5.0).abs() < 1e-5, "northward came out {v}");
    }

    /// The rotated frame's identity case, and the one real file's pole.
    #[test]
    fn a_rotation_to_the_pole_it_already_has_changes_nothing() {
        let pole = RotatedPole::from_south_pole(-90.0, 0.0);
        for (lat, lon) in [(0.0, 0.0), (45.0, 100.0), (-30.0, -170.0)] {
            let (lon_r, lat_r) = pole.to_rotated(lat, lon);
            assert!((lat_r - lat).abs() < 1e-9);
            assert!(wrap180(lon_r - lon).abs() < 1e-9);
            assert!(pole.convergence(lat, lon).abs() < 1e-9);
        }
    }

    /// NCEP names the same rotation by the grid's centre; the centre must
    /// land on the rotated origin, and the two spellings must agree.
    #[test]
    fn a_centre_and_a_southern_pole_name_the_same_rotation() {
        let by_centre = RotatedPole::from_centre(54.0, 254.0);
        let by_pole = RotatedPole::from_south_pole(54.0 - 90.0, 254.0);
        let (lon_r, lat_r) = by_centre.to_rotated(54.0, 254.0);
        assert!(lon_r.abs() < 1e-9 && lat_r.abs() < 1e-9);
        for (lat, lon) in [(20.0, 200.0), (60.0, 300.0), (-10.0, 220.9)] {
            let a = by_centre.to_rotated(lat, lon);
            let b = by_pole.to_rotated(lat, lon);
            assert!((a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12);
        }
    }

    /// Code table 3.2's fixed shapes, and the two that read their own axes.
    #[test]
    fn the_earth_comes_from_the_shape_code() {
        assert_eq!(
            Earth::of_code(6, 0xff, u32::MAX, 0xff, u32::MAX, 0xff, u32::MAX)
                .expect("shape 6 needs no axes"),
            Earth::sphere(6_371_229.0)
        );
        assert_eq!(
            Earth::of_code(1, 0, 6_371_200, 0, 0, 0, 0).expect("a stated radius"),
            Earth::sphere(6_371_200.0)
        );
        let wgs = wgs84();
        assert!((wgs.a - 6_378_137.0).abs() < 1e-6);
        assert!((1.0 / (1.0 - (1.0 - wgs.e2).sqrt()) - 298.257_223_563).abs() < 1e-6);
        // Code 3 states its axes in kilometres.
        let km = Earth::of_code(3, 0, 0, 0, 6_378_137, 0, 6_356_752).expect("axes in km");
        assert!((km.a - 6_378_137_000.0).abs() < 1.0);
        assert!(Earth::of_code(200, 0, 0, 0, 0, 0, 0).is_err());
    }
}
