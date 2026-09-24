//! Liquify's transient harmonic transition. Only geometry and parameters are
//! document data; this lattice is rebuilt from the layer beneath the object.
use std::sync::{Arc, OnceLock};

use ve_core::vector::Uv;

use crate::aeqd::Local;
use crate::sdf::Shape;

/// Shared by flattened scene copies, including spatially culled tile scenes.
#[derive(Debug, Clone, Default)]
pub struct TransitionCache(Arc<OnceLock<Transition>>);

// Cache warmth is not part of a scene's value.
impl PartialEq for TransitionCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl TransitionCache {
    /// Build at most once, including when tiles arrive on several workers.
    pub fn get_or_init(&self, build: impl FnOnce() -> Transition) -> &Transition {
        self.0.get_or_init(build)
    }
}

/// The destination takes precedence when source and destination overlap.
pub fn destination_point(p: Local, displacement: Local) -> Local {
    [p[0] - displacement[0], p[1] - displacement[1]]
}

/// Outside this union the original field remains bit-for-bit unchanged.
pub fn affected(shape: &Shape, p: Local, displacement: Local, distance: f64) -> bool {
    shape.distance(p) <= 0.0 || shape.distance(destination_point(p, displacement)) <= distance
}

/// A bounded, deterministic Dirichlet solve of premultiplied u/v and coverage.
#[derive(Debug)]
pub struct Transition {
    origin: Local,
    spacing: f64,
    width: usize,
    height: usize,
    values: Vec<[f64; 3]>,
    secondary: Option<Box<Transition>>,
    source_radius: f64,
}

impl Transition {
    /// Keep the moved interior and unchanged exterior as boundary conditions;
    /// solve every cell in the vacated source and the destination's outer band.
    pub fn build(
        shape: &Shape,
        displacement: Local,
        distance: f64,
        feather: f64,
        sample: impl Fn(Local) -> (Uv, f32),
    ) -> Self {
        let radius = shape.bounding_radius_m().max(1.0);
        let low = [
            -radius + displacement[0].min(0.0) - distance,
            -radius + displacement[1].min(0.0) - distance,
        ];
        let high = [
            radius + displacement[0].max(0.0) + distance,
            radius + displacement[1].max(0.0) + distance,
        ];
        // Disconnected source/destination domains get their own resolution.
        // A very long move must not coarsen a small interpolation band.
        if displacement[0].hypot(displacement[1]) > 2.0 * radius + distance {
            let mut source = Self::solve(
                shape,
                displacement,
                distance,
                feather,
                &sample,
                [-radius; 2],
                [radius; 2],
            );
            let r = radius + distance;
            source.secondary = Some(Box::new(Self::solve(
                shape,
                displacement,
                distance,
                feather,
                &sample,
                [displacement[0] - r, displacement[1] - r],
                [displacement[0] + r, displacement[1] + r],
            )));
            source.source_radius = radius;
            return source;
        }
        Self::solve(shape, displacement, distance, feather, &sample, low, high)
    }

    fn solve(
        shape: &Shape,
        displacement: Local,
        distance: f64,
        feather: f64,
        sample: &impl Fn(Local) -> (Uv, f32),
        low: Local,
        high: Local,
    ) -> Self {
        let spacing = ((high[0] - low[0]).max(high[1] - low[1]) / 192.0).max(0.01);
        let origin = [low[0] - spacing, low[1] - spacing];
        let width = ((high[0] - low[0]) / spacing).ceil() as usize + 3;
        let height = ((high[1] - low[1]) / spacing).ceil() as usize + 3;
        let mut values = Vec::with_capacity(width * height);
        let mut free = Vec::new();
        let mut scale = 1.0f64;
        let mut boundary_sum = [0.0; 3];
        let mut boundary_count = 0.0;
        for y in 0..height {
            for x in 0..width {
                let p = [
                    origin[0] + x as f64 * spacing,
                    origin[1] + y as f64 * spacing,
                ];
                let source = destination_point(p, displacement);
                let edge = shape.distance(source);
                let moved = edge <= 0.0;
                let feather_width = feather.clamp(0.0, 1.0) * shape.feather_reference_m();
                let pinned = edge <= -feather_width;
                let unknown = !pinned
                    && affected(shape, p, displacement, distance)
                    && x > 0
                    && y > 0
                    && x + 1 < width
                    && y + 1 < height;
                // Unknown cells are initialised below from fixed boundary values,
                // never from the source detail that is being vacated.
                let value = if unknown {
                    [0.0; 3]
                } else {
                    let (uv, coverage) = sample(if moved { source } else { p });
                    [f64::from(uv.u), f64::from(uv.v), f64::from(coverage)]
                };
                scale = scale.max(value[0].abs()).max(value[1].abs());
                if unknown {
                    // A screened-Poisson constraint fades continuously from free
                    // interpolation at the rim to the exact source at the core.
                    // Scale by cell area so feather width is a physical distance.
                    let t = if moved && feather_width > 0.0 {
                        (-edge / feather_width).clamp(0.0, 1.0 - 1e-9)
                    } else {
                        0.0
                    };
                    let lambda = if t > 0.0 {
                        (8.0 * t * spacing / (feather_width * (1.0 - t))).powi(2)
                    } else {
                        0.0
                    };
                    let weight = lambda / (4.0 + lambda);
                    let target = if weight > 0.0 {
                        let (uv, c) = sample(source);
                        [f64::from(uv.u), f64::from(uv.v), f64::from(c)]
                    } else {
                        [0.0; 3]
                    };
                    free.push((y * width + x, weight, target));
                } else {
                    for (sum, component) in boundary_sum.iter_mut().zip(value) {
                        *sum += component;
                    }
                    boundary_count += 1.0;
                }
                values.push(value);
            }
        }
        // A constant from the fixed boundary is a faster initial guess for
        // calm/uniform surroundings. It carries no vacated source detail.
        if boundary_count > 0.0 {
            let initial = boundary_sum.map(|sum| sum / boundary_count);
            for &(i, _, _) in &free {
                values[i] = initial;
            }
        }
        // SOR uses a fixed traversal/order and no parallel reduction, so
        // exports and tile requests see the same solution on every machine.
        for _ in 0..1400 {
            let mut change = 0.0f64;
            for &(i, weight, target) in &free {
                let neighbors = [
                    values[i - 1],
                    values[i + 1],
                    values[i - width],
                    values[i + width],
                ];
                for (channel, value) in values[i].iter_mut().enumerate() {
                    let mean = (neighbors[0][channel]
                        + neighbors[1][channel]
                        + neighbors[2][channel]
                        + neighbors[3][channel])
                        * 0.25;
                    let goal = mean * (1.0 - weight) + target[channel] * weight;
                    let delta = 1.92 * (goal - *value);
                    *value += delta;
                    change = change.max(delta.abs() / if channel == 2 { 1.0 } else { scale });
                }
            }
            if change < 1e-6 {
                break;
            }
        }
        Self {
            origin,
            spacing,
            width,
            height,
            values,
            secondary: None,
            source_radius: 0.0,
        }
    }

    /// Bilinear reading keeps transition edges smooth at any output resolution.
    pub fn sample(&self, p: Local) -> (Uv, f32) {
        if let Some(secondary) = &self.secondary
            && p[0].hypot(p[1]) > self.source_radius
        {
            return secondary.sample(p);
        }

        let x = ((p[0] - self.origin[0]) / self.spacing).clamp(0.0, (self.width - 1) as f64);
        let y = ((p[1] - self.origin[1]) / self.spacing).clamp(0.0, (self.height - 1) as f64);
        let ix = (x.floor() as usize).min(self.width - 2);
        let iy = (y.floor() as usize).min(self.height - 2);
        let fx = x - ix as f64;
        let fy = y - iy as f64;
        let mut value = [0.0; 3];
        for (i, w) in [
            (iy * self.width + ix, (1.0 - fx) * (1.0 - fy)),
            (iy * self.width + ix + 1, fx * (1.0 - fy)),
            ((iy + 1) * self.width + ix, (1.0 - fx) * fy),
            ((iy + 1) * self.width + ix + 1, fx * fy),
        ] {
            for (channel, v) in value.iter_mut().enumerate() {
                *v += self.values[i][channel] * w;
            }
        }
        (
            Uv {
                u: value[0] as f32,
                v: value[1] as f32,
            },
            value[2].clamp(0.0, 1.0) as f32,
        )
    }
}
