//! Freeze a pixel eraser's perimeter in the edited object's frame.

use ve_core::LonLat;
use ve_core::document::{Erasure, LocalPoint};
use ve_render::aeqd::{Frame, Local, Space};

fn hull(mut points: Vec<Local>) -> Vec<Local> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    let cross = |a: Local, b: Local, c: Local| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let mut out: Vec<Local> = Vec::new();
    for p in &points {
        while out.len() >= 2 && cross(out[out.len() - 2], out[out.len() - 1], *p) <= 0.0 {
            out.pop();
        }
        out.push(*p);
    }
    let lower = out.len();
    for p in points.iter().rev().skip(1) {
        while out.len() > lower && cross(out[out.len() - 2], out[out.len() - 1], *p) <= 0.0 {
            out.pop();
        }
        out.push(*p);
    }
    out.pop();
    out
}

/// Each swept segment is a convex cut. Separate cuts preserve bends and loops.
pub fn cuts(
    points: &[LonLat],
    radius: f64,
    square: bool,
    space: Space,
    projection_origin: Option<LonLat>,
    target: Frame,
) -> Vec<Erasure> {
    let Some(&origin) = points.first() else {
        return Vec::new();
    };
    let source = Frame::in_space(origin, 0.0, 100.0, space)
        .with_projection_origin(projection_origin.unwrap_or(origin));
    let centres: Vec<Local> = points.iter().map(|p| source.to_local(*p)).collect();
    centres
        .windows(2)
        .chain((centres.len() == 1).then_some(centres.as_slice()))
        .map(|pair| {
            let mut rim = Vec::new();
            for centre in pair {
                if square {
                    for [x, y] in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
                        rim.push([centre[0] + x * radius, centre[1] + y * radius]);
                    }
                } else {
                    for i in 0..64 {
                        let t = f64::from(i) * std::f64::consts::TAU / 64.0;
                        rim.push([centre[0] + t.cos() * radius, centre[1] + t.sin() * radius]);
                    }
                }
            }
            let rim = hull(rim);
            let mut contour = Vec::new();
            for (a, b) in rim.iter().zip(rim.iter().cycle().skip(1)).take(rim.len()) {
                // Long straight map edges can curve in a ground object's frame.
                let subdivisions = (((a[0] - b[0]).hypot(a[1] - b[1]) / radius.max(1.0)).ceil()
                    as usize)
                    .clamp(1, 128);
                for i in 0..subdivisions {
                    let t = i as f64 / subdivisions as f64;
                    let local = target.to_local(
                        source.to_global([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]),
                    );
                    contour.push(LocalPoint {
                        x: local[0],
                        y: local[1],
                    });
                }
            }
            let centre = target.to_local(origin);
            let rim_point = target.to_local(source.to_global([radius, 0.0]));
            Erasure {
                contour,
                chains: Vec::new(),
                radius_m: (rim_point[0] - centre[0]).hypot(rim_point[1] - centre[1]),
                square: false,
                feather: 0.0,
                step: None,
            }
        })
        .collect()
}
