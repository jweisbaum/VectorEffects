//! Fits an image warp from user-placed control points (spec.md §4.9).
//!
//! A control point pairs a pixel in a picture with where it belongs on the
//! earth. Given zero, one, two, three, or four such pairs, this module picks
//! the least-surprising *exact* fit: the stored placement, a translation, a
//! similarity, a full affine, or a homography. A fifth pair and beyond bends
//! the picture further with a thin-plate spline laid on top of a
//! least-squares homography — the residual a straight projective map cannot
//! reach, damped back to that homography outside the pairs' hull so a far
//! corner of the image does not run away.
//!
//! **The degenerate rule**, followed throughout: wherever a fit's own linear
//! system turns out singular — control points on a line, or two pairs on the
//! same pixel — fall back to the next simpler fit rather than returning a
//! warp that collapses the picture to a line or a point. Homography degrades
//! to affine, affine to similarity, similarity to translation; translation
//! never fails, since one pair has nothing to solve.
//!
//! A homography can also be *solvable* and still unusable: four (or more)
//! pairs placed in a crossing order — or merely close to one — fit a
//! homography whose vanishing line lies on or near the image's own
//! rectangle, so evaluating it anywhere near that line divides by something
//! close enough to zero to send the answer to the thousands of degrees
//! `Warp::mesh` then narrows to `f32`. `homography_covers` checks the fitted
//! homography against the image's four corners — sufficient by convexity,
//! see its own doc — and a homography that fails it degrades exactly like a
//! singular one: to the affine tail, never to a warp that quietly ships a
//! blown-up mesh.

use crate::document::{ControlPoint, Placement};

/// The most control points a warp will fit.
///
/// Past this, the fit solves an `n`-by-`n` dense system and a later change's
/// mesh is re-evaluated against every pair, so the cost is quadratic in one
/// place and linear in a hot one. `Warp::fit` silently truncates to the first
/// `MAX_CONTROL_POINTS` of whatever slice it is given — it does see the rest,
/// as far as the argument goes, but never fits them — which is why the
/// caller that lets the user place them is what has to refuse a fifty-first
/// click with a hint; by the time `fit` runs, silence is all a point past
/// this count would get.
pub const MAX_CONTROL_POINTS: usize = 50;

/// Below this, a pixel-space quantity with units of pixels² is treated as
/// zero and the fit degrades to a simpler one rather than dividing by it:
/// `similarity_through`'s squared distance between its two pixels, or
/// `best_affine_3`'s 2×2 determinant of pixel vectors — the signed area,
/// twice over, of the triangle its three pairs form.
///
/// Scaled by the image's own area rather than fixed, because "nearly
/// coincident" or "nearly collinear" should mean the same fraction of the
/// picture at any resolution. A user's two control points landing a couple
/// of pixels apart is most likely a missed second click on an 80×60
/// thumbnail — a real, if extreme, similarity — and almost certainly a
/// mis-click on the same pixel, or three points that are truly collinear,
/// on a 4000×3000 scan. A threshold fixed in raw pixels² would have to pick
/// one of those readings and be wrong for the other; scaling by
/// `width * height` keeps the same *relative* tolerance at every image
/// size.
fn degeneracy_threshold(width: u32, height: u32) -> f64 {
    f64::from(width.max(1)) * f64::from(height.max(1)) * 1e-12
}

/// How an image's pixels are mapped onto the earth.
///
/// Opaque: a caller constructs one with [`Warp::fit`] and reads it with
/// [`Warp::place`]. [`Warp::is_identity_affine`] and [`Warp::as_placement`]
/// exist so a caller that only cares whether the warp reduces to a plain
/// six-number [`Placement`] — a later change's choice of render path — never
/// has to look inside.
#[derive(Debug)]
pub struct Warp {
    kind: Kind,
    /// The placement `fit` was given. Read only by [`Warp::place`]'s
    /// projective arm, as the answer for a pixel on the homography's
    /// vanishing line (see there); every other arm answers entirely from
    /// `kind`.
    base: Placement,
}

/// The fitted transform. Never matched on outside this module.
#[derive(Debug)]
enum Kind {
    /// A translation, a similarity, or a full affine — the three differ only
    /// in which of the six numbers a fit was free to choose, so one variant
    /// holds all of them.
    Affine(Placement),
    /// A homography's nine matrix entries, row-major, with the ninth fixed
    /// at 1 (see [`Warp::place`]).
    Projective([f64; 9]),
    /// A least-squares homography (`projective`) plus a thin-plate-spline
    /// correction fit through five or more pairs.
    ///
    /// The spline is the standard decomposition into `n` radial weights and
    /// a linear tail: `weights_lon[i]`/`weights_lat[i]` for `i < n` multiply
    /// `phi(|scaled pixel - centers[i]|)`, and `weights_*[n..n+3]` are the
    /// tail's `[a0, a1, a2]` evaluated at the same scaled pixel. `centers`
    /// holds the `n` pairs' pixels already divided by `scale` (the image's
    /// larger side) — see `Warp::fit`'s doc for why the division has to
    /// happen before any distance is computed. `centroid` and `hull_radius`
    /// are in raw, unscaled pixels: they exist only to measure how far a
    /// query pixel is from the pairs, for [`Warp::place`]'s damping.
    Spline {
        projective: [f64; 9],
        scale: f64,
        centers: Vec<(f64, f64)>,
        weights_lon: Vec<f64>,
        weights_lat: Vec<f64>,
        centroid: (f64, f64),
        hull_radius: f64,
    },
}

impl Warp {
    /// Fits a warp through `points`, degrading by count and by degeneracy
    /// exactly as the module doc describes.
    ///
    /// `width` and `height` are the image's pixel dimensions, used only to
    /// scale the degeneracy checks to its size (`degeneracy_threshold`).
    /// `base` is returned unchanged when `points` is empty, and is the
    /// fallback both for a single pair's translation and for a projective
    /// warp's vanishing line.
    pub fn fit(points: &[ControlPoint], width: u32, height: u32, base: Placement) -> Warp {
        let n = points.len().min(MAX_CONTROL_POINTS);
        let points = &points[..n];
        let kind = match n {
            0 => Kind::Affine(base),
            1 => Kind::Affine(translation_through(points[0], base)),
            2 => Kind::Affine(
                similarity_through(points[0], points[1], width, height, base)
                    .unwrap_or_else(|| translation_through(points[0], base)),
            ),
            3 => Kind::Affine(best_affine(points, width, height, base)),
            4 => homography_through(points)
                .filter(|h| homography_covers(h, width, height))
                .map(Kind::Projective)
                .unwrap_or_else(|| Kind::Affine(best_affine(points, width, height, base))),
            _ => spline_through(points, width, height, base),
        };
        Warp { kind, base }
    }

    /// Where a pixel lands, in degrees.
    ///
    /// The longitude is not normalised, for the same reason `Placement::place`
    /// does not: a warped image spanning the antimeridian must keep its right
    /// edge to the right of its left one.
    pub fn place(&self, u: f64, v: f64) -> (f64, f64) {
        match &self.kind {
            Kind::Affine(placement) => placement.place(u, v),
            Kind::Projective(h) => project_through(h, self.base, u, v),
            Kind::Spline {
                projective,
                scale,
                centers,
                weights_lon,
                weights_lat,
                centroid,
                hull_radius,
            } => {
                let (plon, plat) = project_through(projective, self.base, u, v);
                let scale = *scale;
                let uc = u / scale;
                let vc = v / scale;
                let n = centers.len();
                let (mut radial_lon, mut radial_lat) = (0.0, 0.0);
                for (i, c) in centers.iter().enumerate() {
                    let dx = uc - c.0;
                    let dy = vc - c.1;
                    let basis = phi((dx * dx + dy * dy).sqrt());
                    radial_lon += weights_lon[i] * basis;
                    radial_lat += weights_lat[i] * basis;
                }
                let affine_lon = weights_lon[n] + weights_lon[n + 1] * uc + weights_lon[n + 2] * vc;
                let affine_lat = weights_lat[n] + weights_lat[n + 1] * uc + weights_lat[n + 2] * vc;
                // Damping: undamped, `phi`'s r^2*ln(r) growth would send a
                // pixel far outside the pairs' hull to an enormous or
                // infinite correction (spec's mitigation for the spline's
                // documented failure mode). `d <= hull_radius` covers every
                // pair itself — `hull_radius` is defined as the greatest
                // such `d` over the pairs — so this never softens the fit at
                // the pairs the caller asked to land exactly.
                let d = ((u - centroid.0).powi(2) + (v - centroid.1).powi(2)).sqrt();
                let damping = if d <= *hull_radius {
                    1.0
                } else {
                    (*hull_radius / d).powi(2)
                };
                (
                    plon + affine_lon + damping * radial_lon,
                    plat + affine_lat + damping * radial_lat,
                )
            }
        }
    }

    /// True when the warp is exactly a [`Placement`] — zero, one, two, or
    /// three well-placed pairs, never four or more. A later change's render
    /// path uses this to choose between the cheap affine path and the warp
    /// mesh.
    pub fn is_identity_affine(&self) -> bool {
        matches!(self.kind, Kind::Affine(_))
    }

    /// The warp's [`Placement`], when it has one.
    pub fn as_placement(&self) -> Option<Placement> {
        match self.kind {
            Kind::Affine(placement) => Some(placement),
            Kind::Projective(_) | Kind::Spline { .. } => None,
        }
    }

    /// Samples the warp on a `(cells + 1)` by `(cells + 1)` grid over the
    /// image, in the order a vertex buffer wants it: row by row from the
    /// top-left, `v = 0` first and each row running `u = 0` to `u = width`.
    ///
    /// This is a rebuild-time cost, not a per-frame one: the caller re-runs
    /// it when the control points change and hands the renderer lon/lat per
    /// vertex, so the vertex shader only projects what it is given and needs
    /// no per-projection warp logic. `f32` because that is what reaches a
    /// vertex buffer; `place` itself stays `f64` throughout.
    ///
    /// Not normalised, matching `place`: a mesh over an image spanning the
    /// antimeridian must keep its right edge to the right of its left one.
    pub fn mesh(&self, width: u32, height: u32, cells: u32) -> Vec<[f32; 2]> {
        let mut out = Vec::with_capacity((cells as usize + 1) * (cells as usize + 1));
        for row in 0..=cells {
            let v = f64::from(row) / f64::from(cells) * f64::from(height);
            for col in 0..=cells {
                let u = f64::from(col) / f64::from(cells) * f64::from(width);
                let (lon, lat) = self.place(u, v);
                out.push([lon as f32, lat as f32]);
            }
        }
        out
    }

    /// How far each of the image's four corners moves under the full warp
    /// versus under its own plain projective fit, in degrees.
    ///
    /// Top-left, top-right, bottom-right, bottom-left — the same order
    /// `view` builds `ImageLayerView::corners` in. Zero at every corner for
    /// an `Affine` or a plain `Projective` warp: neither has a spline
    /// correction to differ from itself by. Only `Kind::Spline` can move a
    /// corner away from its own least-squares homography, and it typically
    /// moves one most at a far corner outside the pairs' hull — which is
    /// exactly what this reports (spec.md §2, §5): a caution that the spline
    /// is reaching, not an error.
    pub fn corner_residual_deg(&self, width: u32, height: u32) -> [f64; 4] {
        let w = f64::from(width);
        let h = f64::from(height);
        [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(u, v)| {
            let (lon, lat) = self.place(u, v);
            let (base_lon, base_lat) = match &self.kind {
                Kind::Affine(_) | Kind::Projective(_) => (lon, lat),
                Kind::Spline { projective, .. } => project_through(projective, self.base, u, v),
            };
            ((lon - base_lon).powi(2) + (lat - base_lat).powi(2)).sqrt()
        })
    }
}

/// Below this, a corner's homography denominator `w` is close enough to the
/// vanishing line (`w == 0`) that dividing by it inflates whatever numerator
/// remains into millions of degrees before `Warp::mesh` narrows the result to
/// `f32` — no more usable than landing on the line outright, so it is
/// rejected the same way. Six orders of magnitude looser than
/// `project_through`'s own `1e-12`, which exists only to protect that one
/// division from a literal zero and says nothing about whether the *result*
/// stays sane; a corner at `w = 1e-10` clears `1e-12` and still produces the
/// roughly-1e10-degree value this threshold exists to catch.
const HOMOGRAPHY_MIN_W: f64 = 1e-6;

/// Whether a fitted homography is usable across the image's own rectangle —
/// `(0, 0)` to `(width, height)`, regardless of where the control points
/// themselves fell — rather than only at the pairs it was fit through.
///
/// `w = h6*u + h7*v + h8` is linear in `(u, v)`, so its extreme values over a
/// convex region are attained at the region's corners; checking only the
/// four image corners therefore bounds `|w|` everywhere inside the
/// rectangle, not just at the corners themselves. A sign that differs
/// between corners means the vanishing line passes through the rectangle's
/// interior — exactly the fit a four-pair set placed in a crossing order
/// produces — and no bound on `|w|` at the corners helps there, since some
/// point *between* them still lands on the line.
fn homography_covers(h: &[f64; 9], width: u32, height: u32) -> bool {
    let w = f64::from(width);
    let ht = f64::from(height);
    let mut sign = 0.0_f64;
    for (u, v) in [(0.0, 0.0), (w, 0.0), (w, ht), (0.0, ht)] {
        let denom = h[6] * u + h[7] * v + h[8];
        if !denom.is_finite() || denom.abs() < HOMOGRAPHY_MIN_W {
            return false;
        }
        if sign == 0.0 {
            sign = denom.signum();
        } else if denom.signum() != sign {
            return false;
        }
    }
    true
}

/// A translation: `base` shifted so pixel `p.u, p.v` lands exactly on `p`'s
/// target and nothing else changes. This is what makes one pair move the
/// picture without touching its scale or rotation.
fn translation_through(p: ControlPoint, base: Placement) -> Placement {
    let (lon, lat) = base.place(p.u, p.v);
    Placement {
        c: base.c + (p.lon - lon),
        f: base.f + (p.lat - lat),
        ..base
    }
}

/// The similarity (translate, rotate, uniform scale — never shear) through
/// two pairs, solved in closed form.
///
/// A similarity's matrix is always a scalar multiple of a rotation, so it has
/// the constrained form `[[A, -B], [B, A]]`. Writing the map as
/// `lon = A*u - B*v + tx`, `lat = B*u + A*v + ty` and subtracting the first
/// pair's equations from the second's cancels `tx` and `ty`, leaving a 2×2
/// linear system in `A` and `B` alone:
///
/// ```text
/// A*du - B*dv = dlon
/// B*du + A*dv = dlat
/// ```
///
/// Pixel v increases down, whereas latitude increases up. Give v the base
/// placement's handedness before fitting, then restore it in the matrix.
/// Otherwise a second alignment pair silently reflects a north-up image.
///
/// where `du, dv` and `dlon, dlat` are the pixel and target vectors between
/// the two points. Its determinant is `du² + dv²`, so it is singular only
/// when the two pixels coincide — the "two pairs on the same pixel" case the
/// caller falls back from.
fn similarity_through(
    p0: ControlPoint,
    p1: ControlPoint,
    width: u32,
    height: u32,
    base: Placement,
) -> Option<Placement> {
    let handedness = if base.a * base.e - base.b * base.d < 0.0 {
        -1.0
    } else {
        1.0
    };
    let du = p1.u - p0.u;
    let dv = (p1.v - p0.v) * handedness;
    let denom = du * du + dv * dv;
    if denom < degeneracy_threshold(width, height) {
        return None;
    }
    let dlon = p1.lon - p0.lon;
    let dlat = p1.lat - p0.lat;
    let a = (dlon * du + dlat * dv) / denom;
    let b = (dlat * du - dlon * dv) / denom;
    Some(Placement {
        a,
        b: -b * handedness,
        c: p0.lon - a * p0.u + b * p0.v * handedness,
        d: b,
        e: a * handedness,
        f: p0.lat - b * p0.u - a * p0.v * handedness,
    })
}

/// The affine through three pairs, solved in closed form.
///
/// An affine map is linear plus a translation, so subtracting the first
/// pair's equations from the other two removes the translation and leaves a
/// plain 2×2 linear map `M` between pixel-space vectors and target-space
/// vectors: `M * e1 = f1`, `M * e2 = f2`, where `e1, e2` are the pixel
/// offsets of the second and third pairs from the first, and `f1, f2` their
/// target offsets. Two vector equations in a 2×2 unknown give
/// `M = F * E⁻¹`, `E⁻¹` by the ordinary 2×2 cofactor formula. `E`'s
/// determinant is the (signed) area of the pixel triangle the three pairs
/// form; it is singular exactly when they are collinear, which is the
/// degenerate input this function declines.
fn best_affine_3(
    p0: ControlPoint,
    p1: ControlPoint,
    p2: ControlPoint,
    width: u32,
    height: u32,
) -> Option<Placement> {
    let e1 = (p1.u - p0.u, p1.v - p0.v);
    let e2 = (p2.u - p0.u, p2.v - p0.v);
    let det = e1.0 * e2.1 - e2.0 * e1.1;
    if det.abs() < degeneracy_threshold(width, height) {
        return None;
    }
    let inv = 1.0 / det;
    // E^{-1}, by the cofactor formula for a 2x2 matrix with columns e1, e2.
    let inv00 = e2.1 * inv;
    let inv01 = -e2.0 * inv;
    let inv10 = -e1.1 * inv;
    let inv11 = e1.0 * inv;
    let f1 = (p1.lon - p0.lon, p1.lat - p0.lat);
    let f2 = (p2.lon - p0.lon, p2.lat - p0.lat);
    // M = F * E^{-1}, F's columns are f1, f2.
    let a = f1.0 * inv00 + f2.0 * inv10;
    let b = f1.0 * inv01 + f2.0 * inv11;
    let d = f1.1 * inv00 + f2.1 * inv10;
    let e = f1.1 * inv01 + f2.1 * inv11;
    Some(Placement {
        a,
        b,
        c: p0.lon - a * p0.u - b * p0.v,
        d,
        e,
        f: p0.lat - d * p0.u - e * p0.v,
    })
}

/// The best affine fit that `points` supports: the exact affine through its
/// first three pairs if they are not collinear, else the similarity through
/// its first two if they are not coincident, else the translation through
/// its first — the affine tail of the module's degenerate rule, shared by
/// the three-pair path and by a homography's own fallback.
fn best_affine(points: &[ControlPoint], width: u32, height: u32, base: Placement) -> Placement {
    if points.len() >= 3
        && let Some(p) = best_affine_3(points[0], points[1], points[2], width, height)
    {
        return p;
    }
    if points.len() >= 2
        && let Some(p) = similarity_through(points[0], points[1], width, height, base)
    {
        return p;
    }
    translation_through(points[0], base)
}

/// The homography through exactly four pairs.
///
/// A homography maps `(u, v)` to `((h0*u + h1*v + h2) / w, (h3*u + h4*v + h5)
/// / w)` with `w = h6*u + h7*v + 1` — nine matrix entries with the ninth
/// fixed at 1, since scaling every entry by the same nonzero number leaves
/// the map unchanged. Clearing each pair's denominator turns
/// `lon = (...)/ w` into one linear equation in `h0..h7`.
/// Four pairs give eight such equations (two per pair), solved as one 8×8
/// system. `solve` reports `None` when the four points cannot determine a
/// homography — three or more of them collinear, most commonly — which the
/// caller answers by falling back to the affine tail.
fn homography_through(points: &[ControlPoint]) -> Option<[f64; 9]> {
    debug_assert_eq!(points.len(), 4);
    let mut a = [0.0_f64; 64];
    let mut b = [0.0_f64; 8];
    for (i, p) in points.iter().enumerate() {
        let x_row = 2 * i;
        let y_row = 2 * i + 1;
        // lon*w = h0*u + h1*v + h2  =>  h0*u + h1*v + h2 - lon*h6*u - lon*h7*v = lon
        a[x_row * 8] = p.u;
        a[x_row * 8 + 1] = p.v;
        a[x_row * 8 + 2] = 1.0;
        a[x_row * 8 + 6] = -p.lon * p.u;
        a[x_row * 8 + 7] = -p.lon * p.v;
        b[x_row] = p.lon;
        // lat*w = h3*u + h4*v + h5, the same shape one row down.
        a[y_row * 8 + 3] = p.u;
        a[y_row * 8 + 4] = p.v;
        a[y_row * 8 + 5] = 1.0;
        a[y_row * 8 + 6] = -p.lat * p.u;
        a[y_row * 8 + 7] = -p.lat * p.v;
        b[y_row] = p.lat;
    }
    solve(&mut a, &mut b, 8)?;
    Some([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], 1.0])
}

/// Evaluates a homography's nine entries at pixel `(u, v)`, falling back to
/// `base` on the vanishing line — see [`Warp::place`]'s `Projective` arm for
/// why. Shared with [`spline_through`], which needs the same projection to
/// compute the residual its spline has to make up at each pair.
fn project_through(h: &[f64; 9], base: Placement, u: f64, v: f64) -> (f64, f64) {
    let w = h[6] * u + h[7] * v + h[8];
    if w.abs() < 1e-12 {
        return base.place(u, v);
    }
    (
        (h[0] * u + h[1] * v + h[2]) / w,
        (h[3] * u + h[4] * v + h[5]) / w,
    )
}

/// The homography through five or more pairs, fit by least squares.
///
/// With more than four pairs the eight unknowns are over-determined, so this
/// minimises the sum of squared per-row residuals via the normal equations
/// `AᵀA h = Aᵀb`, accumulated from the same per-pair rows
/// [`homography_through`] solves exactly for four. `solve` reports `None`
/// when `AᵀA` is singular — every pair collinear, most commonly — which the
/// caller answers by falling back to an affine fit, the module's usual
/// degenerate rule.
fn homography_least_squares(points: &[ControlPoint]) -> Option<[f64; 9]> {
    let mut ata = [0.0_f64; 64];
    let mut atb = [0.0_f64; 8];
    for p in points {
        let rows = [
            // lon*w = h0*u + h1*v + h2, the same row `homography_through`
            // builds for one pair.
            [p.u, p.v, 1.0, 0.0, 0.0, 0.0, -p.lon * p.u, -p.lon * p.v],
            [0.0, 0.0, 0.0, p.u, p.v, 1.0, -p.lat * p.u, -p.lat * p.v],
        ];
        for (row, target) in rows.iter().zip([p.lon, p.lat]) {
            for i in 0..8 {
                atb[i] += row[i] * target;
                for j in 0..8 {
                    ata[i * 8 + j] += row[i] * row[j];
                }
            }
        }
    }
    solve(&mut ata, &mut atb, 8)?;
    Some([
        atb[0], atb[1], atb[2], atb[3], atb[4], atb[5], atb[6], atb[7], 1.0,
    ])
}

/// The thin-plate-spline radial basis function `r² ln r`, the kernel a
/// rubber sheet bends by: it is the unique function (up to the fit's own
/// affine tail) that minimises total bending energy for a surface pinned at
/// scattered points in the plane, which is why the pairs "bend" rather than
/// merely interpolate along one axis at a time.
///
/// Zero at `r = 0`: the literal formula is `0 * -inf` there, but the
/// function's limit as `r -> 0` is `0`, so it is defined that way rather than
/// evaluated as written.
fn phi(r: f64) -> f64 {
    if r <= 0.0 { 0.0 } else { r * r * r.ln() }
}

/// The thin-plate-spline fit through five or more pairs (module doc):
/// [`homography_least_squares`] for the bulk of the map, plus a spline
/// correction — [`Kind::Spline`] — for the residual a straight projective
/// map cannot reach.
///
/// Degrades twice, both by the module's usual rule of falling back rather
/// than returning a warp built on a `None`: to the affine tail when the
/// homography's own normal equations are singular, and to the bare
/// homography (no spline correction) when the spline's `(n+3)`-by-`(n+3)`
/// system is — which happens when two pairs share a pixel, since then two
/// of `K`'s rows are identical.
fn spline_through(points: &[ControlPoint], width: u32, height: u32, base: Placement) -> Kind {
    let Some(projective) =
        homography_least_squares(points).filter(|h| homography_covers(h, width, height))
    else {
        return Kind::Affine(best_affine(points, width, height, base));
    };
    let n = points.len();

    // Scaled into roughly 0..1 by the image's larger side: `phi`'s `r^2*ln
    // r` on raw pixel distances of a few hundred to a few thousand makes the
    // matrix below badly conditioned, and `solve` reports singular on a
    // perfectly good set of points. Distances computed from these scaled
    // coordinates are still all that `phi` sees, consistently, in both the
    // fit here and `Warp::place`'s evaluation of it.
    let scale = f64::from(width.max(height).max(1));
    let centers: Vec<(f64, f64)> = points.iter().map(|p| (p.u / scale, p.v / scale)).collect();

    // The residual the spline has to supply at each pair: how far the
    // least-squares homography alone misses the target.
    let mut b_lon = vec![0.0; n + 3];
    let mut b_lat = vec![0.0; n + 3];
    for (i, p) in points.iter().enumerate() {
        let (plon, plat) = project_through(&projective, base, p.u, p.v);
        b_lon[i] = p.lon - plon;
        b_lat[i] = p.lat - plat;
    }

    // The [[K, P], [Pᵀ, 0]] system shared by both axes: K's radial entries,
    // P's columns [1, u, v] beside them, and Pᵀ's rows below — the standard
    // thin-plate-spline layout, whose bottom-right 3x3 block stays zero.
    let dim = n + 3;
    let mut k = vec![0.0; dim * dim];
    for (i, ci) in centers.iter().enumerate() {
        for (j, cj) in centers.iter().enumerate() {
            let dx = ci.0 - cj.0;
            let dy = ci.1 - cj.1;
            k[i * dim + j] = phi((dx * dx + dy * dy).sqrt());
        }
        k[i * dim + n] = 1.0;
        k[i * dim + n + 1] = ci.0;
        k[i * dim + n + 2] = ci.1;
        k[n * dim + i] = 1.0;
        k[(n + 1) * dim + i] = ci.0;
        k[(n + 2) * dim + i] = ci.1;
    }
    let mut k_lat = k.clone();
    if solve(&mut k, &mut b_lon, dim).is_none() || solve(&mut k_lat, &mut b_lat, dim).is_none() {
        return Kind::Projective(projective);
    }

    let (su, sv) = points
        .iter()
        .fold((0.0, 0.0), |(su, sv), p| (su + p.u, sv + p.v));
    let centroid = (su / n as f64, sv / n as f64);
    let hull_radius = points
        .iter()
        .map(|p| ((p.u - centroid.0).powi(2) + (p.v - centroid.1).powi(2)).sqrt())
        .fold(0.0_f64, f64::max);

    Kind::Spline {
        projective,
        scale,
        centers,
        weights_lon: b_lon,
        weights_lat: b_lat,
        centroid,
        hull_radius,
    }
}

/// Solves `a x = b` in place by Gaussian elimination with partial pivoting.
///
/// `a` is row-major and `n` by `n`; the solution is written back into `b`.
/// `None` means the system is singular — for this module's callers that is
/// control points on a line, or two pairs on the same pixel, and every
/// caller answers it by falling back to a simpler fit rather than failing.
///
/// Written here because the largest system this ever sees is 53 by 53 (the
/// fifty-pair cap plus the spline's three affine terms), which is far too
/// small to justify a linear-algebra dependency — and several of them bring
/// a `-sys` crate, which invariant 5 and the three-platform build forbid.
fn solve(a: &mut [f64], b: &mut [f64], n: usize) -> Option<()> {
    debug_assert_eq!(a.len(), n * n);
    debug_assert_eq!(b.len(), n);
    for column in 0..n {
        // Partial pivoting: the largest remaining magnitude in this column.
        // Without it a zero on the diagonal stops an otherwise fine system,
        // and a small one loses most of its digits.
        let mut pivot = column;
        for row in (column + 1)..n {
            if a[row * n + column].abs() > a[pivot * n + column].abs() {
                pivot = row;
            }
        }
        if a[pivot * n + column].abs() < 1e-12 {
            return None;
        }
        if pivot != column {
            for k in 0..n {
                a.swap(column * n + k, pivot * n + k);
            }
            b.swap(column, pivot);
        }
        let diagonal = a[column * n + column];
        for row in (column + 1)..n {
            let factor = a[row * n + column] / diagonal;
            if factor == 0.0 {
                continue;
            }
            for k in column..n {
                a[row * n + k] -= factor * a[column * n + k];
            }
            b[row] -= factor * b[column];
        }
    }
    // Back-substitution.
    for row in (0..n).rev() {
        let mut sum = b[row];
        for k in (row + 1)..n {
            sum -= a[row * n + k] * b[k];
        }
        b[row] = sum / a[row * n + row];
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ControlPoint, Placement};

    fn base() -> Placement {
        // 800x600 pixels over one degree square, north up.
        Placement::spanning(-71.0, 42.0, -70.0, 41.0, 800, 600)
    }

    fn at(u: f64, v: f64, lon: f64, lat: f64) -> ControlPoint {
        ControlPoint { u, v, lon, lat }
    }

    /// The defining property, and the one that means "perfectly": whatever
    /// the pairs, the fitted warp puts each pixel exactly on its target.
    fn assert_exact(points: &[ControlPoint], warp: &Warp) {
        for (n, p) in points.iter().enumerate() {
            let (lon, lat) = warp.place(p.u, p.v);
            assert!(
                (lon - p.lon).abs() < 1e-9 && (lat - p.lat).abs() < 1e-9,
                "pair {n} landed at {lon}, {lat} and not at {}, {}",
                p.lon,
                p.lat
            );
        }
    }

    /// No pairs is today's behaviour: the stored placement, untouched.
    #[test]
    fn no_pairs_is_the_placement_unchanged() {
        let warp = Warp::fit(&[], 800, 600, base());
        assert_eq!(warp.place(0.0, 0.0), base().place(0.0, 0.0));
        assert_eq!(warp.place(800.0, 600.0), base().place(800.0, 600.0));
        assert!(warp.is_identity_affine());
    }

    /// One pair moves the picture and changes nothing else. Checked against
    /// a second, independent point: it must move by exactly the same offset.
    #[test]
    fn one_pair_is_a_translation() {
        let (lon0, lat0) = base().place(400.0, 300.0);
        let points = [at(400.0, 300.0, lon0 + 0.25, lat0 - 0.5)];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        let (was_lon, was_lat) = base().place(0.0, 0.0);
        let (now_lon, now_lat) = warp.place(0.0, 0.0);
        assert!((now_lon - was_lon - 0.25).abs() < 1e-12);
        assert!((now_lat - was_lat + 0.5).abs() < 1e-12);
    }

    /// Two pairs rotate and scale but never shear: a square stays square.
    /// Checked by the property that defines a similarity — every distance
    /// is scaled by the same factor.
    #[test]
    fn two_pairs_are_a_similarity_and_keep_the_aspect() {
        let points = [
            at(0.0, 0.0, 0.0, 0.0),
            at(800.0, 0.0, 0.0, 1.0), // the top edge now runs north
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        let a = warp.place(0.0, 0.0);
        let b = warp.place(800.0, 0.0);
        let c = warp.place(0.0, 800.0);
        let ab = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let ac = ((c.0 - a.0).powi(2) + (c.1 - a.1).powi(2)).sqrt();
        assert!((ab - ac).abs() < 1e-9, "a square became {ab} by {ac}");
    }

    #[test]
    fn two_pairs_keep_the_images_handedness_including_rotated_and_south_up_files() {
        let north_up = Placement::spanning(-10.0, 5.0, 10.0, -5.0, 80, 40);
        let points = [at(0.0, 0.0, -10.0, 5.0), at(80.0, 40.0, 10.0, -5.0)];
        let warp = Warp::fit(&points, 80, 40, north_up);
        assert_eq!(warp.place(80.0, 0.0), (10.0, 5.0));
        assert_eq!(warp.place(0.0, 40.0), (-10.0, -5.0));
        let rotated = [at(0.0, 0.0, 0.0, 0.0), at(80.0, 0.0, 0.0, 20.0)];
        assert_eq!(
            Warp::fit(&rotated, 80, 40, north_up).place(0.0, 40.0),
            (10.0, 0.0)
        );
        let south_up = Placement::spanning(-10.0, -5.0, 10.0, 5.0, 80, 40);
        let points = [at(0.0, 0.0, -10.0, -5.0), at(80.0, 0.0, 10.0, -5.0)];
        assert_eq!(
            Warp::fit(&points, 80, 40, south_up).place(0.0, 40.0),
            (-10.0, 5.0)
        );
    }

    /// Three points determine an affine exactly. The expected answer here is
    /// arithmetic worked by hand, not a second copy of the fit: the image's
    /// top-left, top-right and bottom-left are sent to three chosen places,
    /// so the centre must land at their mean.
    #[test]
    fn three_pairs_are_the_affine_through_them() {
        let points = [
            at(0.0, 0.0, -10.0, 5.0),
            at(800.0, 0.0, -8.0, 5.0),
            at(0.0, 600.0, -10.0, 3.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        let (lon, lat) = warp.place(400.0, 300.0);
        assert!((lon - -9.0).abs() < 1e-9, "{lon}");
        assert!((lat - 4.0).abs() < 1e-9, "{lat}");
        assert!(warp.is_identity_affine(), "three pairs are still an affine");
    }

    /// Four pairs are a homography, which an affine cannot be: the image's
    /// four corners go to a trapezium. The property that proves it is
    /// projective and not affine: a homography sends a line through a point
    /// to a line through its image, so the pixel rectangle's own diagonal
    /// crossing — its centre — must land exactly where the target
    /// trapezium's diagonals cross, not at the corners' plain mean.
    ///
    /// The crossing point is worked out here from the two diagonal lines by
    /// hand, independently of the fit: `(-2,1)-(1,-1)` reaches `x = 0` at
    /// `s = 2/3` along itself, where `y = 1 - 2s = -1/3`; `(2,1)-(-1,-1)`
    /// reaches `x = 0` at `t = 2/3` along itself, where `y = 1 - 2t = -1/3`
    /// too, as any two diagonals of one quadrilateral must agree. That
    /// point is *below* the corners' mean of 0, which is the brief's
    /// original assertion inverted — this was checked independently against
    /// the line-intersection formula above (not against this module's own
    /// fit) before concluding the brief's inequality had the wrong sign.
    #[test]
    fn four_pairs_are_a_homography_that_no_affine_could_be() {
        let points = [
            at(0.0, 0.0, -2.0, 1.0),
            at(800.0, 0.0, 2.0, 1.0),
            at(800.0, 600.0, 1.0, -1.0),
            at(0.0, 600.0, -1.0, -1.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        let (lon, lat) = warp.place(400.0, 300.0);
        assert!(
            lon.abs() < 1e-9,
            "the centre should stay on the axis: {lon}"
        );
        assert!(
            (lat - (-1.0 / 3.0)).abs() < 1e-9,
            "expected the diagonals' crossing at -1/3, got {lat}"
        );
        let mean_lat = (1.0 + 1.0 - 1.0 - 1.0) / 4.0;
        assert!(
            (lat - mean_lat).abs() > 1e-6,
            "an affine would give the corners' mean, {mean_lat}; a homography must not"
        );
        assert!(!warp.is_identity_affine(), "a homography is not an affine");
    }

    /// Pairs on a line cannot determine an area. The fit must say so rather
    /// than return a degenerate warp that collapses the picture.
    #[test]
    fn pairs_on_a_line_fall_back_rather_than_collapsing_the_image() {
        let points = [
            at(0.0, 0.0, 0.0, 0.0),
            at(100.0, 0.0, 1.0, 0.0),
            at(200.0, 0.0, 2.0, 0.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        // Not asserted to be exact — it cannot be — but it must still be a
        // usable warp with area, not a collapse to a line.
        let a = warp.place(0.0, 0.0);
        let b = warp.place(0.0, 600.0);
        assert!((a.1 - b.1).abs() > 1e-6, "the image collapsed to a line");
    }

    /// Four pairs placed in a "crossing order" — the target quad's diagonal
    /// pair swapped relative to the picture's — fit a homography whose
    /// vanishing line runs straight through the image: `w` is `+1` at two
    /// corners and `-1` at the other two (worked by hand with a computer
    /// algebra check, not by trusting this module's own fit). Review
    /// finding 3(a): before `homography_covers` existed, this produced a
    /// `Kind::Projective` that sent nearby pixels to on the order of
    /// `1/w -> infinity` degrees. It must fall back to the affine tail
    /// instead, the same way a collinear or coincident input does.
    #[test]
    fn four_pairs_in_a_crossing_order_fall_back_rather_than_diverging() {
        let points = [
            at(0.0, 0.0, -10.0, 5.0),
            at(800.0, 0.0, -8.0, 3.0),
            at(800.0, 600.0, -8.0, 5.0),
            at(0.0, 600.0, -10.0, 3.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert!(
            warp.is_identity_affine(),
            "a crossing-order homography must fall back to the affine tail"
        );
        // The affine tail is still exact through three of the four pairs
        // (the fourth cannot be, or it would not be an affine).
        for p in &points[..3] {
            let (lon, lat) = warp.place(p.u, p.v);
            assert!((lon - p.lon).abs() < 1e-6 && (lat - p.lat).abs() < 1e-6);
        }
        // And every point inside the rectangle stays a sane geographic
        // number — the regression this closes returned values in the
        // billions of degrees for points right beside the vanishing line.
        for (u, v) in [(400.0, 300.0), (799.0, 1.0), (1.0, 599.0)] {
            let (lon, lat) = warp.place(u, v);
            assert!(lon.abs() < 1000.0 && lat.abs() < 1000.0, "{lon}, {lat}");
        }
    }

    /// A homography fit through points placed so that the image's own far
    /// corners land almost exactly on the vanishing line: consistent sign
    /// (no crossing) but `w` on the order of `1e-9` at two corners, computed
    /// independently in Python before being hard-coded here as the pairs.
    /// Review finding 3(a)'s other half — the near-singular case the
    /// original `|w| < 1e-12` guard let through, since `1e-9 > 1e-12`.
    #[test]
    fn a_near_singular_homography_falls_back_rather_than_diverging() {
        let points = [
            at(50.0, 50.0, -11.199999999253333, 11.73333333255111),
            at(750.0, 50.0, 167.99999748000042, 175.99999736000044),
            at(750.0, 550.0, 167.99999748000042, 15.99999976000004),
            at(50.0, 550.0, -11.199999999253333, 1.0666666665955555),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert!(
            warp.is_identity_affine(),
            "a near-singular homography must fall back to the affine tail"
        );
        for (u, v) in [(0.0, 0.0), (800.0, 0.0), (800.0, 600.0), (0.0, 600.0)] {
            let (lon, lat) = warp.place(u, v);
            assert!(lon.is_finite() && lat.is_finite(), "{lon}, {lat}");
            assert!(lon.abs() < 1000.0 && lat.abs() < 1000.0, "{lon}, {lat}");
        }
    }

    /// Five or more pairs bend, and every one of them still lands exactly.
    /// This is what "perfectly warped" means and is the test that would
    /// catch a wrong spline.
    #[test]
    fn every_pair_lands_exactly_however_many_there_are() {
        let mut points = Vec::new();
        for i in 0..7 {
            for j in 0..7 {
                let u = i as f64 * 100.0;
                let v = j as f64 * 80.0;
                // A target that no projective map could reach: a sine ripple
                // across the sheet, which is exactly the paper distortion a
                // rubber sheet exists to correct.
                let lon = -71.0 + u / 800.0 + 0.02 * (v / 100.0).sin();
                let lat = 42.0 - v / 600.0 + 0.02 * (u / 100.0).cos();
                points.push(at(u, v, lon, lat));
            }
        }
        assert!(points.len() <= MAX_CONTROL_POINTS);
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        assert!(!warp.is_identity_affine());
    }

    /// Exactly four pairs must stay a pure homography — `n == 4` never
    /// reaches `spline_through` at all, so there is no spline residual to be
    /// zero, only a projective map to check is still projective. A
    /// homography sends straight lines to straight lines, so three collinear
    /// pixels staying collinear here is the property that would catch a
    /// spline term sneaking in; a picture taken at an angle should not
    /// acquire bending it did not ask for.
    #[test]
    fn four_pairs_bend_not_at_all() {
        let points = [
            at(0.0, 0.0, -2.0, 1.0),
            at(800.0, 0.0, 2.0, 1.0),
            at(800.0, 600.0, 1.0, -1.0),
            at(0.0, 600.0, -1.0, -1.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        // A projective map sends straight lines to straight lines. Three
        // collinear pixels must stay collinear; a spline term would bow them.
        let a = warp.place(0.0, 0.0);
        let m = warp.place(400.0, 300.0);
        let b = warp.place(800.0, 600.0);
        let cross = (m.0 - a.0) * (b.1 - a.1) - (m.1 - a.1) * (b.0 - a.0);
        assert!(cross.abs() < 1e-9, "the diagonal bowed by {cross}");
    }

    /// Outside the hull of the points the spline must not run away: the
    /// radial term grows like r^2 log r, so a far corner has to fall back
    /// towards the projective base rather than diverge.
    #[test]
    fn a_point_far_outside_the_hull_stays_near_the_projective_base() {
        let points: Vec<_> = (0..6)
            .map(|i| {
                let u = 300.0 + (i % 3) as f64 * 100.0;
                let v = 200.0 + (i / 3) as f64 * 100.0;
                at(u, v, -70.5 + u / 4000.0, 41.5 - v / 4000.0)
            })
            .collect();
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        // Ten image-widths away, far outside the hull.
        let (lon, lat) = warp.place(8000.0, 6000.0);
        assert!(lon.is_finite() && lat.is_finite(), "{lon}, {lat}");
        assert!(
            lon.abs() <= 360.0 && lat.abs() <= 180.0,
            "ran away to {lon}, {lat}"
        );
    }

    /// Pairs that straddle the antimeridian. Longitudes are not normalised
    /// inside a warp — an image spanning the seam runs past 180 so its right
    /// edge stays to the right of its left one, exactly as `Placement::place`
    /// documents — so the fit must not fold it.
    #[test]
    fn pairs_across_the_antimeridian_are_not_folded() {
        let points = [
            at(0.0, 0.0, 178.0, 10.0),
            at(800.0, 0.0, 182.0, 10.0),
            at(0.0, 600.0, 178.0, 8.0),
            at(800.0, 600.0, 182.0, 8.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        let (lon, _) = warp.place(400.0, 300.0);
        assert!(
            (lon - 180.0).abs() < 1e-6,
            "the seam folded the image: {lon}"
        );
    }

    /// An image against the pole. The warp is in degree space, so a latitude
    /// past 90 is a real answer for a pixel off the top of the sheet and must
    /// not be clamped inside the fit — but a pair at the pole must still land.
    #[test]
    fn a_pair_at_the_pole_lands_on_the_pole() {
        let points = [
            at(400.0, 0.0, 0.0, 90.0),
            at(0.0, 600.0, -20.0, 80.0),
            at(800.0, 600.0, 20.0, 80.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
    }

    /// The antimeridian case above uses four pairs, which never reaches
    /// `Kind::Spline` — it is Task 2's exact homography, unchanged by this
    /// module's spline code. This is the same property proved through the
    /// spline arm instead: nine pairs (forcing `Kind::Spline`), a target with
    /// a genuine sine curl no homography could reach (so this is not a
    /// homography that merely happens to still fit), longitudes that run
    /// past 180 on both sides of the seam, and every pair still landing
    /// exactly. The fold check is the regression this exists to catch: if
    /// `Warp::place`'s `Spline` arm ever normalised longitude the way
    /// `Placement::place` deliberately does not, a pixel between two pairs
    /// whose longitudes are, say, 177 and 180 would jump to something near
    /// -180 instead of landing between them.
    #[test]
    fn a_spline_across_the_antimeridian_is_not_folded() {
        let mut points = Vec::new();
        for &u in &[0.0, 400.0, 800.0] {
            for &v in &[0.0, 300.0, 600.0] {
                let lon = 180.0 + (u - 400.0) / 400.0 * 3.0 + 0.4 * (v / 300.0_f64).sin();
                let lat = 10.0 - (v - 300.0) / 300.0 + 0.3 * (u / 400.0_f64).cos();
                points.push(at(u, v, lon, lat));
            }
        }
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        assert!(!warp.is_identity_affine());

        // A pixel strictly between the u=0 and u=400 columns (row v=300, an
        // untouched interior point) must land between those two columns' own
        // longitudes — not 360 degrees away on the far side of the seam.
        let left = warp.place(0.0, 300.0).0;
        let mid = warp.place(200.0, 300.0).0;
        let right = warp.place(400.0, 300.0).0;
        assert!(
            (left.min(right)..=left.max(right)).contains(&mid),
            "the seam folded the image: {left}, {mid}, {right}"
        );
    }

    /// The pole case above uses three pairs — `best_affine_3`, unchanged by
    /// this task. Proved again through the spline arm: nine pairs (forcing
    /// `Kind::Spline`) with a target that curls in both pixel directions —
    /// unreachable by any homography — one of them pinned exactly at the
    /// pole. `assert_exact` is what would catch a spline that reaches the
    /// pole only approximately, the way a naive residual-then-clamp
    /// implementation might.
    #[test]
    fn a_spline_at_the_pole_lands_on_the_pole() {
        let mut points = Vec::new();
        for &u in &[0.0, 400.0, 800.0] {
            for &v in &[0.0, 300.0, 600.0] {
                let lon = (u - 400.0) / 40.0 + 0.5 * (v / 300.0_f64).sin();
                let lat = 90.0 - v / 8.0 + 0.4 * (u / 400.0_f64).cos();
                points.push(at(u, v, lon, lat));
            }
        }
        // Pin the first pair exactly at the pole, in place of its formula
        // value (which would land a fraction of a degree short of it).
        points[0] = at(0.0, 0.0, 0.0, 90.0);
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        assert!(!warp.is_identity_affine());
    }

    /// The solver against a system whose answer is known by inspection,
    /// and one that is singular and must say so rather than return noise.
    #[test]
    fn the_solver_solves_and_reports_a_singular_system() {
        // 2x + y = 5, x + 3y = 10  =>  x = 1, y = 3.
        let mut a = [2.0, 1.0, 1.0, 3.0];
        let mut b = [5.0, 10.0];
        solve(&mut a, &mut b, 2).expect("solvable");
        assert!((b[0] - 1.0).abs() < 1e-12, "{b:?}");
        assert!((b[1] - 3.0).abs() < 1e-12, "{b:?}");

        // Two copies of the same equation determine nothing.
        let mut a = [1.0, 2.0, 2.0, 4.0];
        let mut b = [3.0, 6.0];
        assert!(solve(&mut a, &mut b, 2).is_none());

        // A zero in the first pivot position is fine once rows are swapped.
        let mut a = [0.0, 1.0, 1.0, 0.0];
        let mut b = [2.0, 3.0];
        solve(&mut a, &mut b, 2).expect("pivoting handles a zero diagonal");
        assert!(
            (b[0] - 3.0).abs() < 1e-12 && (b[1] - 2.0).abs() < 1e-12,
            "{b:?}"
        );
    }

    /// The mesh is the warp sampled on a grid, in the order the vertex
    /// buffer wants: row by row from the image's top-left. Checked against
    /// `place` at the same pixels, which is the definition.
    #[test]
    fn the_mesh_is_the_warp_sampled_row_by_row() {
        let points = [
            at(0.0, 0.0, -2.0, 1.0),
            at(800.0, 0.0, 2.0, 1.0),
            at(800.0, 600.0, 1.0, -1.0),
            at(0.0, 600.0, -1.0, -1.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        let cells = 4u32;
        let mesh = warp.mesh(800, 600, cells);
        assert_eq!(mesh.len() as u32, (cells + 1) * (cells + 1));
        for row in 0..=cells {
            for col in 0..=cells {
                let u = f64::from(col) / f64::from(cells) * 800.0;
                let v = f64::from(row) / f64::from(cells) * 600.0;
                let (lon, lat) = warp.place(u, v);
                let got = mesh[(row * (cells + 1) + col) as usize];
                assert!((f64::from(got[0]) - lon).abs() < 1e-4, "{got:?} vs {lon}");
                assert!((f64::from(got[1]) - lat).abs() < 1e-4, "{got:?} vs {lat}");
            }
        }
        // The first vertex is the image's top-left corner.
        let corner = warp.place(0.0, 0.0);
        assert!((f64::from(mesh[0][0]) - corner.0).abs() < 1e-4);
    }

    /// A pure affine or homography has no spline term to differ from itself
    /// by, so every corner's residual is exactly zero however far the
    /// pairs push the picture.
    #[test]
    fn an_affine_or_homography_has_zero_corner_residual() {
        let points = [
            at(0.0, 0.0, -2.0, 1.0),
            at(800.0, 0.0, 2.0, 1.0),
            at(800.0, 600.0, 1.0, -1.0),
            at(0.0, 600.0, -1.0, -1.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        for r in warp.corner_residual_deg(800, 600) {
            assert!(r < 1e-9, "{r}");
        }

        let three = &points[..3];
        let warp = Warp::fit(three, 800, 600, base());
        for r in warp.corner_residual_deg(800, 600) {
            assert!(r < 1e-9, "{r}");
        }
    }

    /// A spline's residual at a corner outside the pairs' hull is the
    /// straight-line distance between the full warp and the homography it
    /// was laid on top of — checked independently by evaluating both
    /// placements by hand rather than trusting the same subtraction twice.
    #[test]
    fn a_spline_corner_residual_is_the_distance_to_the_projective_base() {
        let mut points = Vec::new();
        for i in 0..7 {
            for j in 0..7 {
                let u = i as f64 * 100.0;
                let v = j as f64 * 80.0;
                let lon = -71.0 + u / 800.0 + 0.02 * (v / 100.0).sin();
                let lat = 42.0 - v / 600.0 + 0.02 * (u / 100.0).cos();
                points.push(at(u, v, lon, lat));
            }
        }
        let warp = Warp::fit(&points, 800, 600, base());
        assert!(!warp.is_identity_affine());

        let residual = warp.corner_residual_deg(800, 600);
        let Kind::Spline { projective, .. } = &warp.kind else {
            panic!("nine pairs must fit a spline");
        };
        for (i, (u, v)) in [(0.0, 0.0), (800.0, 0.0), (800.0, 600.0), (0.0, 600.0)]
            .into_iter()
            .enumerate()
        {
            let (lon, lat) = warp.place(u, v);
            let (base_lon, base_lat) = project_through(projective, warp.base, u, v);
            let expected = ((lon - base_lon).powi(2) + (lat - base_lat).powi(2)).sqrt();
            assert!(
                (residual[i] - expected).abs() < 1e-12,
                "corner {i}: {} vs {expected}",
                residual[i]
            );
        }
        // At least one corner actually differs: the spline is doing
        // something, or this test would pass for a residual that is
        // silently always zero.
        assert!(residual.iter().any(|r| *r > 1e-6), "{residual:?}");
    }
}
