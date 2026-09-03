//! Property values and the interpolation rules for each value kind.
//!
//! Every property of every object is animatable (spec.md 4.4), including enums
//! and booleans. Storage is therefore dynamic: one `PropValue` type covering
//! every kind, with a schema (see [`crate::schema`]) declaring what each
//! property actually is. That is what lets the timeline iterate an object's
//! keyframe tracks and the inspector build itself, without either one carrying
//! a hand-written case per tool.

use serde::{Deserialize, Serialize};

use crate::angle::Angle;
use crate::geo::LonLat;

/// A property's runtime value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropValue {
    /// A scalar: speed, size, feather, divergence, curl.
    F32(f32),
    /// A toggle.
    Bool(bool),
    /// A direction or bearing. Interpolates along the shortest arc.
    Angle(Angle),
    /// A geographic position. Interpolates along a great circle.
    LonLat(LonLat),
    /// A discriminant into the schema's `variants` list. Never interpolates.
    Enum(u8),
}

/// The kind of a [`PropValue`], without the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropKind {
    /// See [`PropValue::F32`].
    F32,
    /// See [`PropValue::Bool`].
    Bool,
    /// See [`PropValue::Angle`].
    Angle,
    /// See [`PropValue::LonLat`].
    LonLat,
    /// See [`PropValue::Enum`].
    Enum,
}

impl PropValue {
    /// The kind of this value.
    pub fn kind(self) -> PropKind {
        match self {
            Self::F32(_) => PropKind::F32,
            Self::Bool(_) => PropKind::Bool,
            Self::Angle(_) => PropKind::Angle,
            Self::LonLat(_) => PropKind::LonLat,
            Self::Enum(_) => PropKind::Enum,
        }
    }

    /// Whether the value can be written to JSON.
    ///
    /// JSON has no NaN or infinity, so a non-finite value would make a project
    /// unsaveable. Rejected at the edit boundary rather than at save time,
    /// where the user has already lost the context that produced it.
    pub fn is_finite(self) -> bool {
        match self {
            Self::F32(v) => v.is_finite(),
            Self::Angle(a) => a.degrees().is_finite(),
            Self::LonLat(p) => p.lon.is_finite() && p.lat.is_finite(),
            Self::Bool(_) | Self::Enum(_) => true,
        }
    }

    /// Extracts an `f32`, or `None` if this is another kind.
    pub fn as_f32(self) -> Option<f32> {
        match self {
            Self::F32(v) => Some(v),
            _ => None,
        }
    }

    /// Extracts a `bool`, or `None` if this is another kind.
    pub fn as_bool(self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(v),
            _ => None,
        }
    }

    /// Extracts an [`Angle`], or `None` if this is another kind.
    pub fn as_angle(self) -> Option<Angle> {
        match self {
            Self::Angle(v) => Some(v),
            _ => None,
        }
    }

    /// Extracts a [`LonLat`], or `None` if this is another kind.
    pub fn as_lonlat(self) -> Option<LonLat> {
        match self {
            Self::LonLat(v) => Some(v),
            _ => None,
        }
    }

    /// Extracts an enum discriminant, or `None` if this is another kind.
    pub fn as_enum(self) -> Option<u8> {
        match self {
            Self::Enum(v) => Some(v),
            _ => None,
        }
    }
}

/// How a keyframe's outgoing segment reaches the next keyframe.
///
/// The named presets are the CSS timing functions, expressed as the same cubic
/// Bézier the custom variant uses. One solver, one behaviour, no drift between
/// "ease-in" as a preset and "ease-in" drawn by hand in a curve editor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    /// Hold the value until the next keyframe. The only mode for enums and bools.
    Step,
    /// Constant rate.
    Linear,
    /// `cubic-bezier(0.42, 0, 1, 1)`
    EaseIn,
    /// `cubic-bezier(0, 0, 0.58, 1)`
    EaseOut,
    /// `cubic-bezier(0.42, 0, 0.58, 1)`
    EaseInOut,
    /// A hand-authored timing curve.
    Bezier {
        /// First control point, x.
        x1: f32,
        /// First control point, y.
        y1: f32,
        /// Second control point, x.
        x2: f32,
        /// Second control point, y.
        y2: f32,
    },
}

impl Interpolation {
    /// Maps a normalised segment position to an eased position.
    ///
    /// `Step` returns 0: the segment holds the earlier keyframe's value.
    pub fn ease(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Step => 0.0,
            Self::Linear => t,
            Self::EaseIn => solve_bezier(0.42, 0.0, 1.0, 1.0, t),
            Self::EaseOut => solve_bezier(0.0, 0.0, 0.58, 1.0, t),
            Self::EaseInOut => solve_bezier(0.42, 0.0, 0.58, 1.0, t),
            Self::Bezier { x1, y1, x2, y2 } => solve_bezier(
                f64::from(x1),
                f64::from(y1),
                f64::from(x2),
                f64::from(y2),
                t,
            ),
        }
    }

    /// Whether this mode is valid for values of `kind`.
    ///
    /// Enums and booleans have no meaningful midpoint, so they hold.
    pub fn is_valid_for(self, kind: PropKind) -> bool {
        match kind {
            PropKind::Bool | PropKind::Enum => self == Self::Step,
            PropKind::F32 | PropKind::Angle | PropKind::LonLat => true,
        }
    }
}

/// Evaluates a cubic Bézier timing curve at `x`, returning `y`.
///
/// The curve runs from (0,0) to (1,1) with the two given control points. `x` is
/// the segment position, so the parameter `u` must be recovered from it first.
fn solve_bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    fn cubic(a: f64, b: f64, u: f64) -> f64 {
        let inv = 1.0 - u;
        3.0 * inv * inv * u * a + 3.0 * inv * u * u * b + u * u * u
    }
    fn cubic_derivative(a: f64, b: f64, u: f64) -> f64 {
        let inv = 1.0 - u;
        3.0 * inv * inv * a + 6.0 * inv * u * (b - a) + 3.0 * u * u * (1.0 - b)
    }

    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }

    // Newton-Raphson converges in a few steps for well-formed curves.
    let mut u = x;
    for _ in 0..8 {
        let err = cubic(x1, x2, u) - x;
        if err.abs() < 1e-9 {
            return cubic(y1, y2, u);
        }
        let slope = cubic_derivative(x1, x2, u);
        if slope.abs() < 1e-12 {
            break;
        }
        u -= err / slope;
    }

    // Bisection fallback for pathological control points, where the derivative
    // vanishes and Newton stalls.
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    let mut u = x;
    for _ in 0..32 {
        let err = cubic(x1, x2, u) - x;
        if err.abs() < 1e-9 {
            break;
        }
        if err > 0.0 {
            hi = u;
        } else {
            lo = u;
        }
        u = (lo + hi) / 2.0;
    }
    cubic(y1, y2, u)
}

/// Interpolates between two values of the same kind.
///
/// Mismatched kinds, and kinds that cannot interpolate, hold `from`. That is a
/// deliberate fallback rather than an error: a mismatch can only arise from a
/// corrupt or hand-edited project, and holding produces a usable document
/// instead of an unopenable one.
pub fn interpolate(from: PropValue, to: PropValue, t: f64, interp: Interpolation) -> PropValue {
    if interp == Interpolation::Step {
        return from;
    }
    let t = interp.ease(t);

    match (from, to) {
        (PropValue::F32(a), PropValue::F32(b)) => {
            PropValue::F32((f64::from(a) + (f64::from(b) - f64::from(a)) * t) as f32)
        }
        (PropValue::Angle(a), PropValue::Angle(b)) => PropValue::Angle(a.lerp_shortest(b, t)),
        (PropValue::LonLat(a), PropValue::LonLat(b)) => PropValue::LonLat(slerp(a, b, t)),
        _ => from,
    }
}

/// Interpolates between two positions along the great circle joining them.
///
/// Component-wise interpolation of longitude and latitude would cut the corner,
/// drift off the shorter path near the poles, and unwind the wrong way across
/// the antimeridian. Interpolating the 3D unit vectors avoids all three.
pub fn slerp(from: LonLat, to: LonLat, t: f64) -> LonLat {
    fn to_vec(p: LonLat) -> [f64; 3] {
        let (lat, lon) = (p.lat.to_radians(), p.lon.to_radians());
        [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
    }

    let (a, b) = (to_vec(from), to_vec(to));
    let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0);
    let omega = dot.acos();

    // Coincident (or very nearly so): the great circle is undefined and
    // unnecessary. Antipodal points leave the path genuinely ambiguous; any
    // great circle is as correct as any other, so linear blending picks one.
    let v = if omega.abs() < 1e-9 {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    } else {
        let sin_omega = omega.sin();
        let (wa, wb) = (
            ((1.0 - t) * omega).sin() / sin_omega,
            (t * omega).sin() / sin_omega,
        );
        [
            a[0] * wa + b[0] * wb,
            a[1] * wa + b[1] * wb,
            a[2] * wa + b[2] * wb,
        ]
    };

    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-12 {
        return from;
    }
    let (x, y, z) = (v[0] / len, v[1] / len, v[2] / len);
    LonLat {
        lon: crate::geo::normalize_lon(y.atan2(x).to_degrees()),
        lat: z.clamp(-1.0, 1.0).asin().to_degrees(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() < tol, "{a} != {b} (tol {tol})");
    }

    fn ll(lon: f64, lat: f64) -> LonLat {
        LonLat::new(lon, lat).unwrap()
    }

    #[test]
    fn easing_is_pinned_at_both_ends() {
        for interp in [
            Interpolation::Linear,
            Interpolation::EaseIn,
            Interpolation::EaseOut,
            Interpolation::EaseInOut,
            Interpolation::Bezier {
                x1: 0.1,
                y1: 0.9,
                x2: 0.9,
                y2: 0.1,
            },
        ] {
            close(interp.ease(0.0), 0.0, 1e-9);
            close(interp.ease(1.0), 1.0, 1e-9);
        }
    }

    #[test]
    fn linear_easing_is_the_identity() {
        for t in [0.1, 0.25, 0.5, 0.75, 0.9] {
            close(Interpolation::Linear.ease(t), t, 1e-9);
        }
    }

    #[test]
    fn easing_presets_are_monotonic_and_bounded() {
        for interp in [
            Interpolation::EaseIn,
            Interpolation::EaseOut,
            Interpolation::EaseInOut,
        ] {
            let mut prev = -1.0;
            for i in 0..=50 {
                let y = interp.ease(f64::from(i) / 50.0);
                assert!((0.0..=1.0).contains(&y), "{interp:?} left [0,1]: {y}");
                assert!(y >= prev - 1e-9, "{interp:?} went backwards at {i}");
                prev = y;
            }
        }
    }

    /// Ease-in starts slow, ease-out starts fast. If these are swapped the
    /// animation looks wrong in a way that is easy to miss by eye.
    #[test]
    fn ease_in_and_out_bend_opposite_ways() {
        assert!(Interpolation::EaseIn.ease(0.25) < 0.25);
        assert!(Interpolation::EaseOut.ease(0.25) > 0.25);
    }

    #[test]
    fn step_holds_the_earlier_value() {
        close(Interpolation::Step.ease(0.99), 0.0, 1e-12);
        let held = interpolate(
            PropValue::F32(10.0),
            PropValue::F32(20.0),
            0.99,
            Interpolation::Step,
        );
        assert_eq!(held, PropValue::F32(10.0));
    }

    #[test]
    fn only_step_is_valid_for_discrete_kinds() {
        for kind in [PropKind::Bool, PropKind::Enum] {
            assert!(Interpolation::Step.is_valid_for(kind));
            assert!(!Interpolation::Linear.is_valid_for(kind));
            assert!(!Interpolation::EaseInOut.is_valid_for(kind));
        }
        for kind in [PropKind::F32, PropKind::Angle, PropKind::LonLat] {
            assert!(Interpolation::Linear.is_valid_for(kind));
        }
    }

    #[test]
    fn scalars_interpolate_linearly() {
        let v = interpolate(
            PropValue::F32(10.0),
            PropValue::F32(20.0),
            0.5,
            Interpolation::Linear,
        );
        close(f64::from(v.as_f32().unwrap()), 15.0, 1e-6);
    }

    #[test]
    fn angles_take_the_shortest_arc() {
        let v = interpolate(
            PropValue::Angle(Angle::new(350.0)),
            PropValue::Angle(Angle::new(10.0)),
            0.5,
            Interpolation::Linear,
        );
        close(v.as_angle().unwrap().degrees(), 0.0, 1e-9);
    }

    #[test]
    fn discrete_kinds_hold_rather_than_blend() {
        let b = interpolate(
            PropValue::Bool(false),
            PropValue::Bool(true),
            0.9,
            Interpolation::Linear,
        );
        assert_eq!(b, PropValue::Bool(false));

        let e = interpolate(
            PropValue::Enum(0),
            PropValue::Enum(3),
            0.9,
            Interpolation::Linear,
        );
        assert_eq!(e, PropValue::Enum(0));
    }

    /// A corrupt project must open, not explode.
    #[test]
    fn mismatched_kinds_hold() {
        let v = interpolate(
            PropValue::F32(1.0),
            PropValue::Bool(true),
            0.5,
            Interpolation::Linear,
        );
        assert_eq!(v, PropValue::F32(1.0));
    }

    #[test]
    fn slerp_endpoints_are_exact() {
        let (a, b) = (ll(-30.0, 40.0), ll(120.0, -20.0));
        let start = slerp(a, b, 0.0);
        close(start.lon, a.lon, 1e-9);
        close(start.lat, a.lat, 1e-9);
        let end = slerp(a, b, 1.0);
        close(end.lon, b.lon, 1e-9);
        close(end.lat, b.lat, 1e-9);
    }

    #[test]
    fn slerp_midpoint_on_the_equator() {
        let mid = slerp(ll(0.0, 0.0), ll(90.0, 0.0), 0.5);
        close(mid.lon, 45.0, 1e-9);
        close(mid.lat, 0.0, 1e-9);
    }

    /// The case component-wise interpolation gets wrong: it would unwind the
    /// long way round through longitude 0 instead of crossing the dateline.
    #[test]
    fn slerp_crosses_the_antimeridian_the_short_way() {
        let mid = slerp(ll(170.0, 0.0), ll(-170.0, 0.0), 0.5);
        close(mid.lon.abs(), 180.0, 1e-6);
        close(mid.lat, 0.0, 1e-9);
    }

    /// Two points on opposite meridians at high latitude are joined over the
    /// pole, not down across the equator.
    #[test]
    fn slerp_goes_over_the_pole() {
        let mid = slerp(ll(0.0, 80.0), ll(180.0, 80.0), 0.5);
        close(mid.lat, 90.0, 1e-6);
    }

    /// Every intermediate point must stay on the sphere and inside valid
    /// coordinate ranges, including for awkward pairs.
    #[test]
    fn slerp_stays_on_the_globe() {
        let pairs = [
            (ll(0.0, 0.0), ll(0.0, 0.0)),
            (ll(-179.9, -89.0), ll(179.9, 89.0)),
            (ll(0.0, 90.0), ll(0.0, -90.0)),
            (ll(45.0, 12.0), ll(-135.0, -12.0)),
        ];
        for (a, b) in pairs {
            for i in 0..=10 {
                let p = slerp(a, b, f64::from(i) / 10.0);
                assert!(
                    p.lat.is_finite() && p.lon.is_finite(),
                    "{a:?}->{b:?} produced {p:?}"
                );
                assert!((-90.0..=90.0).contains(&p.lat), "lat out of range: {p:?}");
                assert!((-180.0..180.0).contains(&p.lon), "lon out of range: {p:?}");
            }
        }
    }

    #[test]
    fn non_finite_values_are_detected() {
        assert!(PropValue::F32(1.0).is_finite());
        assert!(!PropValue::F32(f32::NAN).is_finite());
        assert!(!PropValue::F32(f32::INFINITY).is_finite());
        assert!(PropValue::Bool(true).is_finite());
    }

    #[test]
    fn values_round_trip_through_json() {
        let values = [
            PropValue::F32(12.5),
            PropValue::Bool(true),
            PropValue::Angle(Angle::new(110.0)),
            PropValue::LonLat(ll(-45.0, 30.0)),
            PropValue::Enum(2),
        ];
        for v in values {
            let json = serde_json::to_string(&v).unwrap();
            let back: PropValue = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back, "round trip failed for {json}");
        }
    }
}
