//! Signed distance functions for object geometry.
//!
//! Every shape reports, for a point in the object's local frame, its signed
//! distance to the shape's boundary in metres — negative inside (spec.md 7.3).
//!
//! One scalar answers everything the evaluator needs. Coverage is `d <= 0`.
//! Feather is a falloff over the last stretch of `d` inside the edge. Culling
//! is a bound on the largest `d` that can be negative. There is no separate
//! inside-test, edge-test or bounds-test to keep consistent with each other.

use crate::aeqd::Local;

/// A shape in an object's local frame. Distances are metres.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A swept disc along one or more polylines: brush, eraser, clone stamp,
    /// and the corridor along a curve. A single point is a plain disc.
    ///
    /// Several chains, because merged strokes share one object and must not be
    /// joined end to start — that would sweep the brush across the gap.
    Capsule {
        /// Paths the disc is swept along.
        chains: Vec<Vec<Local>>,
        /// Swept radius.
        radius_m: f64,
    },
    /// A swept *square* along one or more polylines: the square brush.
    ///
    /// The same chains a [`Self::Capsule`] holds, with the stamp swapped for an
    /// axis-aligned square of side `2 * half_size_m` in the object's local
    /// frame — so the object's rotation turns the stamp with it.
    SweptSquare {
        /// Paths the square is swept along.
        chains: Vec<Vec<Local>>,
        /// Half the stamp's side.
        half_size_m: f64,
    },
    /// A filled circle centred on the anchor.
    Disc {
        /// Radius.
        radius_m: f64,
    },
    /// A ring centred on the anchor.
    Annulus {
        /// Radius of the ring's centreline.
        radius_m: f64,
        /// Half the ring's thickness.
        half_width_m: f64,
    },
    /// An axis-aligned rectangle centred on the anchor, before rotation.
    Rect {
        /// Half-extent along local x.
        half_width_m: f64,
        /// Half-extent along local y.
        half_height_m: f64,
    },
    /// A closed polygon. May be concave.
    Polygon {
        /// Vertices in order; the closing edge is implied.
        ring: Vec<Local>,
    },
}

/// Distance from `p` to the segment `a`–`b`.
fn segment_distance(p: Local, a: Local, b: Local) -> f64 {
    let (pax, pay) = (p[0] - a[0], p[1] - a[1]);
    let (bax, bay) = (b[0] - a[0], b[1] - a[1]);
    let denom = bax * bax + bay * bay;
    // A zero-length segment is a point, which is a legitimate one-click stroke.
    let t = if denom <= f64::EPSILON {
        0.0
    } else {
        ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0)
    };
    (pax - bax * t).hypot(pay - bay * t)
}

impl Shape {
    /// Signed distance to the boundary, in metres. Negative inside.
    pub fn distance(&self, p: Local) -> f64 {
        match self {
            Self::Capsule { chains, radius_m } => capsule_distance(chains, p) - radius_m.max(0.0),
            Self::SweptSquare {
                chains,
                half_size_m,
            } => swept_square_distance(chains, p) - half_size_m.max(0.0),
            Self::Disc { radius_m } => p[0].hypot(p[1]) - radius_m.max(0.0),
            Self::Annulus {
                radius_m,
                half_width_m,
            } => (p[0].hypot(p[1]) - radius_m.max(0.0)).abs() - half_width_m.max(0.0),
            Self::Rect {
                half_width_m,
                half_height_m,
            } => rect_distance(p, half_width_m.max(0.0), half_height_m.max(0.0)),
            Self::Polygon { ring } => polygon_distance(ring, p),
        }
    }

    /// The largest distance from the origin at which the shape can be inside.
    ///
    /// The evaluator turns this into a spherical cap and rejects cells outside
    /// it before doing any distance work at all.
    pub fn bounding_radius_m(&self) -> f64 {
        match self {
            Self::Capsule { chains, radius_m } => {
                chains
                    .iter()
                    .flatten()
                    .map(|p| p[0].hypot(p[1]))
                    .fold(0.0, f64::max)
                    + radius_m.max(0.0)
            }
            // The stamp's corner, not its edge: a square reaches
            // `half_size * sqrt(2)` diagonally, and a cull that used the edge
            // would clip the four corners of every square stroke.
            Self::SweptSquare {
                chains,
                half_size_m,
            } => {
                chains
                    .iter()
                    .flatten()
                    .map(|p| p[0].hypot(p[1]))
                    .fold(0.0, f64::max)
                    + half_size_m.max(0.0) * std::f64::consts::SQRT_2
            }
            Self::Disc { radius_m } => radius_m.max(0.0),
            Self::Annulus {
                radius_m,
                half_width_m,
            } => radius_m.max(0.0) + half_width_m.max(0.0),
            Self::Rect {
                half_width_m,
                half_height_m,
            } => half_width_m.max(0.0).hypot(half_height_m.max(0.0)),
            Self::Polygon { ring } => ring.iter().map(|p| p[0].hypot(p[1])).fold(0.0, f64::max),
        }
    }

    /// A characteristic size, used to scale the feather band (spec.md 7.4).
    pub fn feather_reference_m(&self) -> f64 {
        match self {
            Self::Capsule { radius_m, .. } => radius_m.max(0.0),
            Self::SweptSquare { half_size_m, .. } => half_size_m.max(0.0),
            Self::Disc { radius_m } => radius_m.max(0.0),
            Self::Annulus { half_width_m, .. } => half_width_m.max(0.0),
            Self::Rect {
                half_width_m,
                half_height_m,
            } => half_width_m.max(0.0).min(half_height_m.max(0.0)),
            // A polygon has no radius, so use its extent. Half is close enough
            // for a falloff band and is stable under editing.
            Self::Polygon { .. } => self.bounding_radius_m() * 0.5,
        }
    }

    /// Whether the shape can cover anything at all.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Capsule { chains, radius_m } => {
                chains.iter().all(Vec::is_empty) || *radius_m <= 0.0
            }
            Self::SweptSquare {
                chains,
                half_size_m,
            } => chains.iter().all(Vec::is_empty) || *half_size_m <= 0.0,
            Self::Disc { radius_m } => *radius_m <= 0.0,
            Self::Annulus {
                radius_m,
                half_width_m,
            } => *radius_m <= 0.0 || *half_width_m <= 0.0,
            Self::Rect {
                half_width_m,
                half_height_m,
            } => *half_width_m <= 0.0 || *half_height_m <= 0.0,
            Self::Polygon { ring } => ring.len() < 3,
        }
    }
}

/// Distance to the nearest of several polylines.
///
/// Taking the minimum across chains — rather than concatenating them — is what
/// keeps a merged stroke from painting a segment across the gap between the two
/// gestures it was merged from.
fn capsule_distance(chains: &[Vec<Local>], p: Local) -> f64 {
    chains
        .iter()
        .map(|chain| match chain.as_slice() {
            [] => f64::INFINITY,
            [only] => (p[0] - only[0]).hypot(p[1] - only[1]),
            _ => chain
                .windows(2)
                .map(|pair| segment_distance(p, pair[0], pair[1]))
                .fold(f64::INFINITY, f64::min),
        })
        .fold(f64::INFINITY, f64::min)
}

/// Chebyshev distance from `p` to the nearest of several polylines.
fn swept_square_distance(chains: &[Vec<Local>], p: Local) -> f64 {
    chains
        .iter()
        .map(|chain| match chain.as_slice() {
            [] => f64::INFINITY,
            [only] => (p[0] - only[0]).abs().max((p[1] - only[1]).abs()),
            _ => chain
                .windows(2)
                .map(|pair| segment_distance_inf(p, pair[0], pair[1]))
                .fold(f64::INFINITY, f64::min),
        })
        .fold(f64::INFINITY, f64::min)
}

/// Chebyshev (L-infinity) distance from `p` to the segment `a`-`b`.
///
/// The square swept along a segment is exactly the set of points within
/// `half_size` of it *in this metric*, because the metric's unit ball is the
/// stamp. Measuring the sweep that way makes coverage exact, where a circular
/// distance would have to approximate the square as a polygon and disagree
/// with the GPU about its corners.
///
/// Inside the footprint this is also the true distance to the boundary, which
/// is what the feather band needs: the inward offset of a square is a smaller
/// square. Outside it under-reports along the diagonals, and nothing reads it
/// there — coverage stops at zero and culling uses the corner radius.
fn segment_distance_inf(p: Local, a: Local, b: Local) -> f64 {
    let (ex, ey) = (a[0] - p[0], a[1] - p[1]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let g = |t: f64| (ex + t * dx).abs().max((ey + t * dy).abs());

    // `g` is the maximum of four linear functions of `t`, so it is convex and
    // piecewise linear: its minimum over the segment sits either at an end or
    // where two of those lines cross. These four crossings plus the two ends
    // are every candidate there is, so the answer is exact rather than
    // iterative — which is what lets the shader compute the same number.
    let mut best = g(0.0).min(g(1.0));
    let mut consider = |numerator: f64, denominator: f64| {
        if denominator.abs() > f64::EPSILON {
            best = best.min(g((numerator / denominator).clamp(0.0, 1.0)));
        }
    };
    consider(-ex, dx); // the x term changes sign
    consider(-ey, dy); // the y term changes sign
    consider(ey - ex, dx - dy); // the two terms meet, same sign
    consider(-(ex + ey), dx + dy); // the two terms meet, opposite signs
    best
}

/// Minimum distance between two sets of polylines, in metres.
///
/// Two capsules overlap when this is no greater than the sum of their radii.
/// Segment-to-segment rather than vertex-to-vertex, because two long strokes
/// can cross in the middle of a span with every vertex far from the crossing.
pub fn chains_distance(a: &[Vec<Local>], b: &[Vec<Local>]) -> f64 {
    let mut best = f64::INFINITY;
    for left in a {
        for right in b {
            best = best.min(polyline_distance(left, right));
            if best == 0.0 {
                return 0.0;
            }
        }
    }
    best
}

fn polyline_distance(a: &[Local], b: &[Local]) -> f64 {
    // A one-point chain is a point, which the segment cases handle as a
    // zero-length segment.
    let segments = |chain: &[Local]| -> Vec<(Local, Local)> {
        match chain {
            [] => Vec::new(),
            [only] => vec![(*only, *only)],
            _ => chain.windows(2).map(|w| (w[0], w[1])).collect(),
        }
    };
    let (left, right) = (segments(a), segments(b));
    let mut best = f64::INFINITY;
    for &(p0, p1) in &left {
        for &(q0, q1) in &right {
            best = best.min(segment_to_segment(p0, p1, q0, q1));
            if best == 0.0 {
                return 0.0;
            }
        }
    }
    best
}

fn segment_to_segment(p0: Local, p1: Local, q0: Local, q1: Local) -> f64 {
    if segments_cross(p0, p1, q0, q1) {
        return 0.0;
    }
    segment_distance(p0, q0, q1)
        .min(segment_distance(p1, q0, q1))
        .min(segment_distance(q0, p0, p1))
        .min(segment_distance(q1, p0, p1))
}

fn segments_cross(p0: Local, p1: Local, q0: Local, q1: Local) -> bool {
    let cross = |o: Local, a: Local, b: Local| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let d1 = cross(p0, p1, q0);
    let d2 = cross(p0, p1, q1);
    let d3 = cross(q0, q1, p0);
    let d4 = cross(q0, q1, p1);
    // Strict signs only: collinear and touching cases fall through to the
    // endpoint distances, which report zero for them anyway.
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

fn rect_distance(p: Local, half_w: f64, half_h: f64) -> f64 {
    let dx = p[0].abs() - half_w;
    let dy = p[1].abs() - half_h;
    // Outside contributes the corner distance; inside, the nearest edge.
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0)
}

/// Signed distance to a closed polygon, negative inside.
///
/// The sign comes from a crossing count rather than a convexity assumption, so
/// a hand-drawn concave outline behaves correctly.
fn polygon_distance(points: &[Local], p: Local) -> f64 {
    if points.len() < 3 {
        return f64::INFINITY;
    }

    let mut squared = f64::INFINITY;
    let mut inside = false;

    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];

        let distance = segment_distance(p, a, b);
        squared = squared.min(distance * distance);

        // Ray crossing to the +x side.
        let crosses = (a[1] > p[1]) != (b[1] > p[1]);
        if crosses {
            let t = (p[1] - a[1]) / (b[1] - a[1]);
            if p[0] < a[0] + t * (b[0] - a[0]) {
                inside = !inside;
            }
        }
    }

    let distance = squared.sqrt();
    if inside { -distance } else { distance }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() < tol, "{a} != {b} (tol {tol})");
    }

    #[test]
    fn a_disc_measures_from_its_edge() {
        let disc = Shape::Disc { radius_m: 1000.0 };
        close(disc.distance([0.0, 0.0]), -1000.0, 1e-9);
        close(disc.distance([1000.0, 0.0]), 0.0, 1e-9);
        close(disc.distance([2000.0, 0.0]), 1000.0, 1e-9);
        close(disc.distance([0.0, -1500.0]), 500.0, 1e-9);
        assert_eq!(disc.bounding_radius_m(), 1000.0);
    }

    #[test]
    fn an_annulus_is_inside_only_near_its_ring() {
        let ring = Shape::Annulus {
            radius_m: 1000.0,
            half_width_m: 100.0,
        };
        assert!(ring.distance([0.0, 0.0]) > 0.0, "the hole is outside");
        close(ring.distance([1000.0, 0.0]), -100.0, 1e-9);
        close(ring.distance([900.0, 0.0]), 0.0, 1e-9);
        close(ring.distance([1100.0, 0.0]), 0.0, 1e-9);
        assert_eq!(ring.bounding_radius_m(), 1100.0);
    }

    #[test]
    fn a_capsule_follows_its_path() {
        let stroke = Shape::Capsule {
            chains: vec![vec![[0.0, 0.0], [1000.0, 0.0]]],
            radius_m: 200.0,
        };
        // On the path, at the swept radius.
        close(stroke.distance([500.0, 0.0]), -200.0, 1e-9);
        // Perpendicular offset.
        close(stroke.distance([500.0, 200.0]), 0.0, 1e-9);
        close(stroke.distance([500.0, 400.0]), 200.0, 1e-9);
        // Rounded ends, not square ones.
        close(stroke.distance([1200.0, 0.0]), 0.0, 1e-9);
        close(stroke.distance([-300.0, 0.0]), 100.0, 1e-9);
        close(stroke.bounding_radius_m(), 1200.0, 1e-9);
    }

    /// A single click is a legitimate stroke and must produce a disc, not a
    /// division by zero.
    #[test]
    fn a_one_point_capsule_is_a_disc() {
        let dot = Shape::Capsule {
            chains: vec![vec![[0.0, 0.0]]],
            radius_m: 300.0,
        };
        close(dot.distance([0.0, 0.0]), -300.0, 1e-9);
        close(dot.distance([300.0, 0.0]), 0.0, 1e-9);
    }

    #[test]
    fn a_repeated_point_does_not_divide_by_zero() {
        let stroke = Shape::Capsule {
            chains: vec![vec![[10.0, 10.0], [10.0, 10.0]]],
            radius_m: 50.0,
        };
        let d = stroke.distance([10.0, 60.0]);
        assert!(d.is_finite(), "got {d}");
        close(d, 0.0, 1e-9);
    }

    /// The whole reason a capsule holds several chains: merging two strokes
    /// must not paint the gap between them.
    #[test]
    fn separate_chains_are_not_joined_end_to_start() {
        let merged = Shape::Capsule {
            chains: vec![
                vec![[0.0, 0.0], [1000.0, 0.0]],
                vec![[5000.0, 0.0], [6000.0, 0.0]],
            ],
            radius_m: 200.0,
        };
        // Both chains are covered.
        assert!(merged.distance([500.0, 0.0]) < 0.0);
        assert!(merged.distance([5500.0, 0.0]) < 0.0);
        // The gap between them is not.
        assert!(
            merged.distance([3000.0, 0.0]) > 0.0,
            "a chain boundary must not sweep the brush across the gap"
        );
        close(merged.distance([3000.0, 0.0]), 1800.0, 1e-9);
    }

    #[test]
    fn chain_distance_finds_crossings_between_vertices() {
        let horizontal = vec![vec![[-1000.0, 0.0], [1000.0, 0.0]]];
        let vertical = vec![vec![[0.0, -1000.0], [0.0, 1000.0]]];
        // The crossing is a long way from every vertex, so a vertex-to-vertex
        // test would report 1000 m here.
        close(chains_distance(&horizontal, &vertical), 0.0, 1e-9);

        let offset = vec![vec![[-1000.0, 300.0], [1000.0, 300.0]]];
        close(chains_distance(&horizontal, &offset), 300.0, 1e-9);
    }

    #[test]
    fn chain_distance_takes_the_nearest_pair_of_chains() {
        let a = vec![vec![[0.0, 0.0]], vec![[10_000.0, 0.0]]];
        let b = vec![vec![[10_000.0, 400.0]]];
        close(chains_distance(&a, &b), 400.0, 1e-9);
    }

    /// A square brush stamped once must be exactly the square, not a circle
    /// approximated by one.
    #[test]
    fn a_one_point_swept_square_is_a_square() {
        let stamp = Shape::SweptSquare {
            chains: vec![vec![[0.0, 0.0]]],
            half_size_m: 100.0,
        };
        close(stamp.distance([0.0, 0.0]), -100.0, 1e-9);
        // The edge, at the same distance along either axis.
        close(stamp.distance([100.0, 0.0]), 0.0, 1e-9);
        close(stamp.distance([0.0, -100.0]), 0.0, 1e-9);
        // The corner is inside, where a disc of the same size would not be.
        assert!(stamp.distance([99.0, 99.0]) < 0.0, "the corner is covered");
        assert!(Shape::Disc { radius_m: 100.0 }.distance([99.0, 99.0]) > 0.0);
        // Just outside along both axes at once is still outside.
        assert!(stamp.distance([101.0, 101.0]) > 0.0);
    }

    /// The whole point of sweeping in the Chebyshev metric: the footprint of a
    /// stroke is the union of the squares stamped along it, corners included.
    #[test]
    fn a_swept_square_covers_the_union_of_its_stamps() {
        let stroke = Shape::SweptSquare {
            chains: vec![vec![[0.0, 0.0], [1000.0, 0.0]]],
            half_size_m: 200.0,
        };
        // Hand-computed: the sweep of a 400 m square along the x axis is the
        // rectangle [-200, 1200] x [-200, 200].
        for (point, inside) in [
            ([500.0, 199.0], true),
            ([500.0, 201.0], false),
            ([1199.0, 199.0], true), // the far corner, square not rounded
            ([1201.0, 0.0], false),
            ([-199.0, -199.0], true), // the near corner
            ([-201.0, 0.0], false),
        ] {
            assert_eq!(
                stroke.distance(point) <= 0.0,
                inside,
                "{point:?} should be {}",
                if inside { "inside" } else { "outside" }
            );
        }
        // A capsule of the same radius rounds the ends off, so the corners of
        // the swept square are exactly where the two disagree.
        let round = Shape::Capsule {
            chains: vec![vec![[0.0, 0.0], [1000.0, 0.0]]],
            radius_m: 200.0,
        };
        assert!(round.distance([1199.0, 199.0]) > 0.0);
    }

    /// Inside the footprint the value is the true distance to the boundary,
    /// which is what the feather band is measured in.
    #[test]
    fn a_swept_square_measures_its_inside_from_the_nearest_edge() {
        let stroke = Shape::SweptSquare {
            chains: vec![vec![[0.0, 0.0], [1000.0, 0.0]]],
            half_size_m: 200.0,
        };
        close(stroke.distance([500.0, 0.0]), -200.0, 1e-9);
        close(stroke.distance([500.0, 150.0]), -50.0, 1e-9);
        close(stroke.distance([1100.0, 0.0]), -100.0, 1e-9);
    }

    /// Chains stay separate here for the same reason they do for a capsule.
    #[test]
    fn separate_square_chains_are_not_joined_end_to_start() {
        let merged = Shape::SweptSquare {
            chains: vec![
                vec![[0.0, 0.0], [1000.0, 0.0]],
                vec![[5000.0, 0.0], [6000.0, 0.0]],
            ],
            half_size_m: 200.0,
        };
        assert!(merged.distance([500.0, 0.0]) < 0.0);
        assert!(merged.distance([5500.0, 0.0]) < 0.0);
        close(merged.distance([3000.0, 0.0]), 1800.0, 1e-9);
    }

    /// A diagonal stroke is where a naive minimum would go wrong: the closest
    /// point in the Euclidean sense is not the one whose stamp reaches
    /// furthest.
    #[test]
    fn a_diagonal_swept_square_agrees_with_its_stamps() {
        let half = 300.0;
        let stroke = Shape::SweptSquare {
            chains: vec![vec![[0.0, 0.0], [2000.0, 2000.0]]],
            half_size_m: half,
        };
        // Independent reference: a point is covered exactly when some point on
        // the segment has it inside that point's square.
        let covered = |p: [f64; 2]| {
            (0..=20_000).any(|i| {
                let t = f64::from(i) / 20_000.0;
                let c = [2000.0 * t, 2000.0 * t];
                (p[0] - c[0]).abs() <= half && (p[1] - c[1]).abs() <= half
            })
        };
        for i in -12..24 {
            for j in -12..24 {
                let p = [f64::from(i) * 200.0, f64::from(j) * 200.0];
                assert_eq!(
                    stroke.distance(p) <= 1e-6,
                    covered(p),
                    "disagreed at {p:?}: distance {}",
                    stroke.distance(p)
                );
            }
        }
    }

    #[test]
    fn a_rectangle_measures_edges_and_corners() {
        let rect = Shape::Rect {
            half_width_m: 200.0,
            half_height_m: 100.0,
        };
        close(rect.distance([0.0, 0.0]), -100.0, 1e-9);
        close(rect.distance([200.0, 0.0]), 0.0, 1e-9);
        close(rect.distance([300.0, 0.0]), 100.0, 1e-9);
        // Outside a corner, the distance is the diagonal to it.
        close(rect.distance([500.0, 400.0]), (300.0f64).hypot(300.0), 1e-9);
        close(rect.bounding_radius_m(), (200.0f64).hypot(100.0), 1e-9);
    }

    #[test]
    fn a_convex_polygon_signs_correctly() {
        let square = Shape::Polygon {
            ring: vec![
                [-100.0, -100.0],
                [100.0, -100.0],
                [100.0, 100.0],
                [-100.0, 100.0],
            ],
        };
        assert!(square.distance([0.0, 0.0]) < 0.0);
        close(square.distance([0.0, 0.0]), -100.0, 1e-9);
        close(square.distance([100.0, 0.0]), 0.0, 1e-9);
        assert!(square.distance([200.0, 0.0]) > 0.0);
        close(square.distance([200.0, 0.0]), 100.0, 1e-9);
    }

    /// The reason the sign comes from a crossing count: a hand-drawn outline is
    /// often concave, and a convexity assumption would fill the notch.
    #[test]
    fn a_concave_polygon_excludes_its_notch() {
        // An L shape occupying three quadrants of a square.
        let shape = Shape::Polygon {
            ring: vec![
                [0.0, 0.0],
                [200.0, 0.0],
                [200.0, 100.0],
                [100.0, 100.0],
                [100.0, 200.0],
                [0.0, 200.0],
            ],
        };
        assert!(shape.distance([50.0, 50.0]) < 0.0, "inside the corner");
        assert!(shape.distance([150.0, 50.0]) < 0.0, "inside the foot");
        assert!(shape.distance([50.0, 150.0]) < 0.0, "inside the upright");
        assert!(
            shape.distance([150.0, 150.0]) > 0.0,
            "the notch must be outside"
        );
    }

    #[test]
    fn a_degenerate_polygon_covers_nothing() {
        let line = Shape::Polygon {
            ring: vec![[0.0, 0.0], [100.0, 0.0]],
        };
        assert!(line.is_empty());
        assert!(line.distance([50.0, 0.0]).is_infinite());
    }

    #[test]
    fn empty_shapes_are_recognised() {
        assert!(
            Shape::Capsule {
                chains: vec![],
                radius_m: 10.0
            }
            .is_empty()
        );
        assert!(
            Shape::Capsule {
                chains: vec![vec![[0.0, 0.0]]],
                radius_m: 0.0
            }
            .is_empty()
        );
        assert!(
            Shape::SweptSquare {
                chains: vec![vec![[0.0, 0.0]]],
                half_size_m: 0.0
            }
            .is_empty()
        );
        assert!(Shape::Disc { radius_m: 0.0 }.is_empty());
        assert!(!Shape::Disc { radius_m: 1.0 }.is_empty());
    }

    /// Nothing may be inside a shape beyond its own bounding radius, or the
    /// evaluator's cull would clip it.
    #[test]
    fn nothing_is_inside_beyond_the_bounding_radius() {
        let shapes = [
            Shape::Disc { radius_m: 900.0 },
            Shape::Annulus {
                radius_m: 700.0,
                half_width_m: 120.0,
            },
            Shape::Rect {
                half_width_m: 300.0,
                half_height_m: 800.0,
            },
            Shape::Capsule {
                chains: vec![vec![[-400.0, 0.0], [600.0, 250.0]]],
                radius_m: 150.0,
            },
            Shape::SweptSquare {
                chains: vec![vec![[-400.0, 0.0], [600.0, 250.0]]],
                half_size_m: 150.0,
            },
            Shape::Polygon {
                ring: vec![[-500.0, -200.0], [400.0, -300.0], [100.0, 600.0]],
            },
        ];

        for shape in &shapes {
            let bound = shape.bounding_radius_m();
            for i in 0..360 {
                let angle = f64::from(i).to_radians();
                let r = bound + 1.0;
                let p = [r * angle.cos(), r * angle.sin()];
                assert!(
                    shape.distance(p) > 0.0,
                    "{shape:?} claims a point at {r} is inside, bound is {bound}"
                );
            }
        }
    }

    /// A signed distance must not jump: neighbouring points differ by at most
    /// the distance between them.
    #[test]
    fn distances_are_continuous() {
        let shapes = [
            Shape::Disc { radius_m: 500.0 },
            Shape::Rect {
                half_width_m: 200.0,
                half_height_m: 400.0,
            },
            Shape::Capsule {
                chains: vec![vec![[0.0, 0.0], [300.0, 300.0]]],
                radius_m: 100.0,
            },
            Shape::SweptSquare {
                chains: vec![vec![[0.0, 0.0], [300.0, 300.0]]],
                half_size_m: 100.0,
            },
        ];
        for shape in &shapes {
            let mut previous = shape.distance([-1200.0, 37.0]);
            for step in 1..2400 {
                let x = -1200.0 + f64::from(step);
                let d = shape.distance([x, 37.0]);
                assert!(
                    (d - previous).abs() <= 1.0 + 1e-6,
                    "{shape:?} jumped by {} at x={x}",
                    (d - previous).abs()
                );
                previous = d;
            }
        }
    }
}
