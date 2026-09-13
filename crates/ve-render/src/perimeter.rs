//! Editable closed boundaries for all supported object footprints.
//!
//! Primitive rings are exact (circles use 48 samples). Swept stamps are
//! contoured on a sparse lattice at one sixth of the stamp radius. Sampling
//! only beside each segment keeps long, thin and disconnected strokes intact.

use std::collections::{BTreeMap, BTreeSet};

use crate::{aeqd::Local, sdf::Shape};

type Edge = (i64, i64, u8);

/// Extracts outer boundaries and holes without joining separate strokes.
pub fn rings(shape: &Shape) -> Vec<Vec<Local>> {
    let circle = |r: f64| {
        (0..48)
            .map(|i| {
                let a = std::f64::consts::TAU * f64::from(i) / 48.0;
                [r * a.cos(), r * a.sin()]
            })
            .collect::<Vec<_>>()
    };
    match shape {
        Shape::Contours { rings, .. } => rings.clone(),
        Shape::Polygon { ring } => vec![subdivide(ring)],
        Shape::Rect {
            half_width_m: x,
            half_height_m: y,
        } => vec![subdivide(&[[-x, -y], [*x, -y], [*x, *y], [-x, *y]])],
        Shape::Disc { radius_m } => vec![circle(*radius_m)],
        Shape::Annulus {
            radius_m,
            half_width_m,
        } => {
            let mut out = vec![circle(radius_m + half_width_m)];
            if radius_m > half_width_m {
                out.push(circle(radius_m - half_width_m));
            }
            out
        }
        Shape::Capsule { chains, radius_m } => swept(chains, *radius_m, false),
        Shape::SweptSquare {
            chains,
            half_size_m,
        } => swept(chains, *half_size_m, true),
    }
}

fn subdivide(ring: &[Local]) -> Vec<Local> {
    if ring.len() < 3 {
        return ring.to_vec();
    }
    let edges = || {
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
    };
    let length: f64 = edges().map(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1])).sum();
    let mut out = Vec::new();
    for (a, b) in edges() {
        let n = ((a[0] - b[0]).hypot(a[1] - b[1]) / (length / 32.0).max(1e-6))
            .ceil()
            .max(1.0) as usize;
        for i in 0..n {
            let t = i as f64 / n as f64;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    out
}

fn swept(chains: &[Vec<Local>], radius: f64, square: bool) -> Vec<Vec<Local>> {
    if radius <= 0.0 || !radius.is_finite() {
        return Vec::new();
    }
    let h = radius / 6.0;
    let mut values = BTreeMap::<(i64, i64), f64>::new();
    for chain in chains {
        for (a, b) in chain.iter().zip(chain.iter().skip(1).chain(chain.last())) {
            let length = (a[0] - b[0]).hypot(a[1] - b[1]);
            let n = (length / radius).ceil().max(1.0) as usize;
            for k in 0..n {
                let at = |i: usize| {
                    let t = i as f64 / n as f64;
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                };
                let (a, b) = (at(k), at(k + 1));
                let segment = if square {
                    Shape::SweptSquare {
                        chains: vec![vec![a, b]],
                        half_size_m: radius,
                    }
                } else {
                    Shape::Capsule {
                        chains: vec![vec![a, b]],
                        radius_m: radius,
                    }
                };
                let lo = |i: usize| ((a[i].min(b[i]) - radius) / h).floor() as i64 - 1;
                let hi = |i: usize| ((a[i].max(b[i]) + radius) / h).ceil() as i64 + 1;
                for y in lo(1)..=hi(1) {
                    for x in lo(0)..=hi(0) {
                        let d = segment.distance([x as f64 * h, y as f64 * h]);
                        values
                            .entry((x, y))
                            .and_modify(|v| *v = v.min(d))
                            .or_insert(d);
                    }
                }
            }
        }
    }
    let cells: BTreeSet<_> = values
        .keys()
        .flat_map(|&(x, y)| [(x, y), (x - 1, y), (x, y - 1), (x - 1, y - 1)])
        .collect();
    let mut positions = BTreeMap::<Edge, Local>::new();
    let mut links = BTreeMap::<Edge, Vec<Edge>>::new();
    for (x, y) in cells {
        let corners = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)];
        let d = corners.map(|p| values.get(&p).copied().unwrap_or(h));
        let edges = [(x, y, 0), (x + 1, y, 1), (x, y + 1, 0), (x, y, 1)];
        let mut crossed = Vec::new();
        for i in 0..4 {
            let j = (i + 1) % 4;
            if (d[i] < 0.0) != (d[j] < 0.0) {
                let t = d[i] / (d[i] - d[j]);
                let (a, b) = (corners[i], corners[j]);
                positions.insert(
                    edges[i],
                    [
                        (a.0 as f64 + (b.0 - a.0) as f64 * t) * h,
                        (a.1 as f64 + (b.1 - a.1) as f64 * t) * h,
                    ],
                );
                crossed.push(edges[i]);
            }
        }
        if crossed.len() == 4 {
            // Resolve a saddle through its bilinear centre, preserving holes.
            if (d.iter().sum::<f64>() < 0.0) != (d[0] < 0.0) {
                crossed.rotate_left(1);
            }
        }
        for pair in crossed.chunks_exact(2) {
            links.entry(pair[0]).or_default().push(pair[1]);
            links.entry(pair[1]).or_default().push(pair[0]);
        }
    }
    let mut out = Vec::new();
    while let Some((&start, _)) = links.first_key_value() {
        let mut ring = Vec::new();
        let mut at = start;
        while let Some(neighbours) = links.remove(&at) {
            ring.push(positions[&at]);
            let Some(next) = neighbours.into_iter().find(|p| links.contains_key(p)) else {
                break;
            };
            at = next;
        }
        if ring.len() >= 3 {
            // Collapse lattice collinearity and small curve errors without
            // dropping a component or changing which ring is a hole.
            simplify(&mut ring, radius * 0.008);
            out.push(ring);
        }
    }
    out
}

fn simplify(ring: &mut Vec<Local>, tolerance: f64) {
    // Remove only points whose neighbouring chord lies within tolerance. A
    // second pass checks against the new neighbours, so error cannot build up
    // by deleting every point on a shallow curve at once.
    let mut i = 0;
    let mut unchanged = 0;
    while ring.len() > 3 && unchanged < ring.len() {
        let n = ring.len();
        i %= n;
        let (a, p, b) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy).max(1e-12))
            .clamp(0.0, 1.0);
        if (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy) < tolerance {
            ring.remove(i);
            unchanged = 0;
        } else {
            i += 1;
            unchanged += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agrees(source: Shape) -> Shape {
        let contours = rings(&source);
        assert!(!contours.is_empty());
        let converted = Shape::Contours {
            rings: contours,
            source: Box::new(source.clone()),
        };
        let reach = source.bounding_radius_m();
        let tolerance = source.feather_reference_m() * 0.06;
        for y in -55..=55 {
            for x in -55..=55 {
                let p = [f64::from(x) / 55.0 * reach, f64::from(y) / 55.0 * reach];
                let d = source.distance(p);
                if d.abs() > tolerance {
                    assert_eq!(
                        d < 0.0,
                        converted.distance(p) < 0.0,
                        "footprint changed at {p:?}: {d}"
                    );
                }
            }
        }
        converted
    }

    #[test]
    fn overlapping_stamps_are_one_boundary_and_disconnected_strokes_stay_disconnected() {
        for square in [false, true] {
            let chains = vec![
                vec![[-4.0, 0.0], [-1.0, 1.0], [0.0, 0.0]],
                vec![[-2.0, 0.0], [0.0, -1.0]],
                vec![[4.0, 0.0], [5.0, 0.0]],
            ];
            let source = if square {
                Shape::SweptSquare {
                    chains,
                    half_size_m: 0.75,
                }
            } else {
                Shape::Capsule {
                    chains,
                    radius_m: 0.75,
                }
            };
            let converted = agrees(source);
            let Shape::Contours { rings, .. } = converted else {
                unreachable!()
            };
            assert_eq!(
                rings.len(),
                2,
                "overlapping strokes must be unioned before keying"
            );
        }
    }

    #[test]
    fn circles_annuli_polygons_and_looped_strokes_keep_their_holes() {
        agrees(Shape::Disc { radius_m: 2.0 });
        agrees(Shape::Rect {
            half_width_m: 2.0,
            half_height_m: 0.5,
        });
        agrees(Shape::Polygon {
            ring: vec![
                [-3.0, -2.0],
                [3.0, -2.0],
                [3.0, 2.0],
                [1.0, 0.0],
                [-3.0, 2.0],
            ],
        });
        for source in [
            Shape::Annulus {
                radius_m: 2.0,
                half_width_m: 0.25,
            },
            Shape::Capsule {
                chains: vec![vec![
                    [-2.0, -2.0],
                    [2.0, -2.0],
                    [2.0, 2.0],
                    [-2.0, 2.0],
                    [-2.0, -2.0],
                ]],
                radius_m: 0.4,
            },
        ] {
            let converted = agrees(source);
            assert!(converted.distance([0.0, 0.0]) > 0.0);
            let Shape::Contours { rings, .. } = converted else {
                unreachable!()
            };
            assert_eq!(rings.len(), 2);
        }
    }

    #[test]
    fn isolated_single_stamps_and_thin_distant_parts_are_not_lost() {
        let source = Shape::Capsule {
            chains: vec![vec![[0.0, 0.0]], vec![[10000.0, 0.0], [10010.0, 0.0]]],
            radius_m: 0.1,
        };
        let converted = Shape::Contours {
            rings: rings(&source),
            source: Box::new(source),
        };
        assert!(converted.distance([0.0, 0.0]) < 0.0);
        assert!(converted.distance([10005.0, 0.0]) < 0.0);
        assert!(converted.distance([5000.0, 0.0]) > 0.0);
    }

    #[test]
    fn size_animation_changes_width_without_stretching_a_stroke() {
        let shape = Shape::Capsule {
            chains: vec![vec![[0.0, 0.0], [10.0, 0.0]]],
            radius_m: 1.0,
        };
        assert_eq!(shape.rescale_perimeter_point([5.0, 1.0], 2.0), [5.0, 2.0]);
        assert_eq!(shape.rescale_perimeter_point([11.0, 0.0], 2.0), [12.0, 0.0]);
    }
}
