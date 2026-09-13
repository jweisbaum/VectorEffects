//! Independently keyed perimeter vertices, in the object's local frame.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Interpolation, LocalPoint};

/// A position key on one perimeter vertex.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointKey {
    /// Position in unscaled local metres.
    pub point: LocalPoint,
    /// Easing leaving this key.
    pub interp: Interpolation,
}

/// One stable vertex identity. Editing another vertex never adds keys here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapePoint {
    /// Position when there are no keys.
    pub base: LocalPoint,
    /// Sorted, unique frame positions.
    #[serde(default)]
    pub keys: BTreeMap<u32, PointKey>,
}

impl ShapePoint {
    /// Samples each coordinate independently, holding outside the keyed range.
    pub fn at(&self, step: u32) -> LocalPoint {
        let before = self.keys.range(..=step).next_back();
        let after = self.keys.range(step..).next();
        match (before, after) {
            (Some((&a, from)), Some((&b, to))) if a != b => {
                let t = from.interp.ease(f64::from(step - a) / f64::from(b - a));
                LocalPoint {
                    x: from.point.x + (to.point.x - from.point.x) * t,
                    y: from.point.y + (to.point.y - from.point.y) * t,
                }
            }
            (Some((_, key)), _) | (_, Some((_, key))) => key.point,
            _ => self.base,
        }
    }

    /// Adds a key, retaining its outgoing easing if it already exists.
    pub fn set(&mut self, step: u32, point: LocalPoint) {
        let interp = self
            .keys
            .get(&step)
            .map_or(Interpolation::Linear, |k| k.interp);
        self.keys.insert(step, PointKey { point, interp });
    }
}

/// Closed contours with stable vertex counts. Even/odd filling keeps holes
/// and disconnected parts. Erasures remain separate destructive cuts on the
/// object and are applied after this footprint has been sampled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeAnimation {
    /// Outer boundaries and holes, implicitly closed.
    pub rings: Vec<Vec<ShapePoint>>,
    /// Original stamp size: existing size animation scales the contour.
    #[serde(with = "crate::canonical::metres_field")]
    pub reference_size_m: f64,
}

impl ShapeAnimation {
    /// Captures an editable perimeter without keying its vertices yet.
    pub fn new(rings: Vec<Vec<LocalPoint>>, reference_size_m: f64) -> Self {
        Self {
            rings: rings
                .into_iter()
                .map(|r| {
                    r.into_iter()
                        .map(|base| ShapePoint {
                            base,
                            keys: BTreeMap::new(),
                        })
                        .collect()
                })
                .collect(),
            reference_size_m,
        }
    }

    /// The sampled perimeter in local metres.
    pub fn at(&self, step: u32) -> Vec<Vec<LocalPoint>> {
        self.rings
            .iter()
            .map(|r| r.iter().map(|p| p.at(step)).collect())
            .collect()
    }

    /// The timeline's aggregate shape keys, with one marker per step.
    pub fn keys(&self) -> BTreeMap<u32, Interpolation> {
        let mut keys = BTreeMap::new();
        for point in self.rings.iter().flatten() {
            for (&step, key) in &point.keys {
                keys.entry(step)
                    .and_modify(|interp| {
                        if *interp != key.interp {
                            *interp = Interpolation::Linear;
                        }
                    })
                    .or_insert(key.interp);
            }
        }
        keys
    }

    /// Counts aggregate shape keys beyond the timeline.
    pub fn count_after(&self, last: u32) -> usize {
        self.keys().keys().filter(|&&s| s > last).count()
    }

    /// Removes keys outside a shortened project.
    pub fn truncate_to(&mut self, last: u32) -> usize {
        let count = self.count_after(last);
        for p in self.rings.iter_mut().flatten() {
            p.keys.retain(|&s, _| s <= last);
        }
        count
    }

    /// Freezes a still paste at the copied frame.
    pub fn freeze(&mut self, step: u32) {
        for p in self.rings.iter_mut().flatten() {
            p.base = p.at(step);
            p.keys.clear();
        }
    }

    /// Moves keys with a relative paste.
    pub fn shift_keys(&mut self, delta: i64, last: u32) {
        for p in self.rings.iter_mut().flatten() {
            p.keys = std::mem::take(&mut p.keys)
                .into_iter()
                .map(|(s, k)| ((i64::from(s) + delta).clamp(0, i64::from(last)) as u32, k))
                .collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{Clipboard, PasteTiming};
    use crate::{Object, ToolKind};

    #[test]
    fn independent_points_hold_and_ease_in_local_metres() {
        let mut p = ShapePoint {
            base: LocalPoint::new(10.0, 20.0),
            keys: BTreeMap::new(),
        };
        p.set(2, p.base);
        p.set(6, LocalPoint::new(30.0, -20.0));
        assert_eq!(p.at(0), p.base);
        assert_eq!(p.at(4), LocalPoint::new(20.0, 0.0));
        assert_eq!(p.at(10), LocalPoint::new(30.0, -20.0));
        p.keys.get_mut(&2).unwrap().interp = Interpolation::Step;
        assert_eq!(p.at(5), p.base);
        assert_eq!(p.at(6), LocalPoint::new(30.0, -20.0));
    }

    #[test]
    fn shape_timing_follows_still_and_relative_pastes() {
        let mut object = Object::new(ToolKind::ShapeFill, "shape", 11);
        let mut a = ShapeAnimation::new(
            vec![vec![
                LocalPoint::new(0.0, 0.0),
                LocalPoint::new(10.0, 0.0),
                LocalPoint::new(0.0, 10.0),
            ]],
            1.0,
        );
        a.rings[0][0].set(0, LocalPoint::new(0.0, 0.0));
        a.rings[0][0].set(10, LocalPoint::new(20.0, 0.0));
        object.shape_animation = Some(a);
        let clipboard = Clipboard::copy(vec![object], 5);
        let still = clipboard.paste(0, PasteTiming::Still, 20, false);
        let still_shape = still[0].shape_animation.as_ref().unwrap();
        assert_eq!(still_shape.at(0)[0][0], LocalPoint::new(10.0, 0.0));
        assert!(still_shape.keys().is_empty());
        let moved = clipboard.paste(3, PasteTiming::Relative, 20, false);
        assert_eq!(
            moved[0]
                .shape_animation
                .as_ref()
                .unwrap()
                .keys()
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![3, 13]
        );
    }
}
