//! Keyframed properties.
//!
//! An [`Animatable`] holds a base value plus zero or more keyframes. With no
//! keyframes it is a constant; with keyframes it is sampled per time step
//! (spec.md 4.4, 4.5).
//!
//! Outside the keyframed range the nearest keyframe's value is held. There is
//! no extrapolation: a field that ramps off to infinity past the last keyframe
//! is never what an author wants, and would silently produce absurd wind speeds
//! at the end of a forecast.

use serde::{Deserialize, Serialize};

use crate::value::{Interpolation, PropKind, PropValue, interpolate};

/// A value pinned to a specific time step.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// The time-step index this key is pinned to.
    pub step: u32,
    /// The value at that step.
    pub value: PropValue,
    /// Governs the segment *leaving* this key, toward the next one.
    pub interp: Interpolation,
}

/// Serialised form. Kept separate so deserialisation can re-establish the
/// sorted-and-unique invariant on data that may have been hand-edited.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnimatableRepr {
    base: PropValue,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    keys: Vec<Keyframe>,
}

/// A property value that may vary across time steps.
///
/// The keyframe list is private because every operation must preserve two
/// invariants: keys are sorted by step, and no two keys share a step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "AnimatableRepr", into = "AnimatableRepr")]
pub struct Animatable {
    base: PropValue,
    keys: Vec<Keyframe>,
}

impl From<AnimatableRepr> for Animatable {
    fn from(repr: AnimatableRepr) -> Self {
        let mut anim = Self {
            base: repr.base,
            keys: repr.keys,
        };
        // A hand-edited or corrupt file may carry unsorted or duplicated keys.
        // Normalising here means no other code has to defend against it.
        anim.keys.sort_by_key(|k| k.step);
        anim.keys.dedup_by_key(|k| k.step);
        anim
    }
}

impl From<Animatable> for AnimatableRepr {
    fn from(anim: Animatable) -> Self {
        Self {
            base: anim.base,
            keys: anim.keys,
        }
    }
}

impl Animatable {
    /// A constant property with no keyframes.
    pub fn constant(base: PropValue) -> Self {
        Self {
            base,
            keys: Vec::new(),
        }
    }

    /// The value used when no keyframes exist.
    pub fn base(&self) -> PropValue {
        self.base
    }

    /// Replaces the base value.
    pub fn set_base(&mut self, value: PropValue) {
        self.base = value;
    }

    /// The keyframes, sorted by step.
    pub fn keys(&self) -> &[Keyframe] {
        &self.keys
    }

    /// Whether this property varies over time.
    pub fn is_animated(&self) -> bool {
        !self.keys.is_empty()
    }

    /// The kind of value this property holds.
    pub fn kind(&self) -> PropKind {
        self.base.kind()
    }

    /// Adds a keyframe, replacing any existing key at the same step.
    ///
    /// The interpolation is coerced to `Step` for kinds that cannot blend, so
    /// an invalid combination cannot be stored in the first place.
    pub fn set_key(&mut self, step: u32, value: PropValue, interp: Interpolation) {
        let interp = if interp.is_valid_for(value.kind()) {
            interp
        } else {
            Interpolation::Step
        };
        let key = Keyframe {
            step,
            value,
            interp,
        };
        match self.keys.binary_search_by_key(&step, |k| k.step) {
            Ok(i) => self.keys[i] = key,
            Err(i) => self.keys.insert(i, key),
        }
    }

    /// Removes the keyframe at `step`, returning it if there was one.
    pub fn remove_key(&mut self, step: u32) -> Option<Keyframe> {
        match self.keys.binary_search_by_key(&step, |k| k.step) {
            Ok(i) => Some(self.keys.remove(i)),
            Err(_) => None,
        }
    }

    /// Moves a keyframe to a new step, overwriting any key already there.
    ///
    /// Returns `false` if there was no key at `from`.
    pub fn move_key(&mut self, from: u32, to: u32) -> bool {
        let Some(mut key) = self.remove_key(from) else {
            return false;
        };
        key.step = to;
        self.set_key(to, key.value, key.interp);
        true
    }

    /// Changes the interpolation of the segment leaving `step`.
    ///
    /// Returns `false` if there was no key at `step`.
    pub fn set_key_interp(&mut self, step: u32, interp: Interpolation) -> bool {
        let kind = self.base.kind();
        match self.keys.binary_search_by_key(&step, |k| k.step) {
            Ok(i) => {
                if interp.is_valid_for(kind) {
                    self.keys[i].interp = interp;
                }
                true
            }
            Err(_) => false,
        }
    }

    /// Drops every keyframe after `last_step`, returning how many were removed.
    ///
    /// Used when the project's `step_count` shrinks (spec.md 4.1, decision D13).
    pub fn truncate_to(&mut self, last_step: u32) -> usize {
        let before = self.keys.len();
        self.keys.retain(|k| k.step <= last_step);
        before - self.keys.len()
    }

    /// How many keyframes sit beyond `last_step`.
    ///
    /// Lets the shrink confirmation quote an exact count before deleting
    /// anything.
    pub fn count_after(&self, last_step: u32) -> usize {
        self.keys.iter().filter(|k| k.step > last_step).count()
    }

    /// Shifts every keyframe by `delta` steps, clamped to `[0, last]`.
    ///
    /// Used when pasting with relative timing (spec.md 8.5). Keys that would
    /// land outside the project are clamped rather than dropped: losing an
    /// animation because it was pasted near the end of the timeline would be a
    /// surprise, and clamping is recoverable by moving them.
    pub fn shift_keys(&mut self, delta: i64, last: u32) {
        for key in &mut self.keys {
            let moved = i64::from(key.step) + delta;
            key.step = moved.clamp(0, i64::from(last)) as u32;
        }
        // Clamping can collide two keys onto one step; the invariant is that
        // steps are unique, so the later one wins.
        self.keys.sort_by_key(|key| key.step);
        self.keys.dedup_by_key(|key| key.step);
    }

    /// The value at `step`.
    pub fn value_at(&self, step: u32) -> PropValue {
        let keys = &self.keys;
        let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
            return self.base;
        };
        if step <= first.step {
            return first.value;
        }
        if step >= last.step {
            return last.value;
        }

        // `partition_point` gives the index of the first key strictly after
        // `step`; the segment we want starts at the key before it. Both
        // neighbours exist because `step` is strictly inside the range.
        let next = keys.partition_point(|k| k.step <= step);
        let (Some(a), Some(b)) = (keys.get(next - 1), keys.get(next)) else {
            return self.base;
        };

        let span = f64::from(b.step - a.step);
        let t = if span > 0.0 {
            f64::from(step - a.step) / span
        } else {
            0.0
        };
        interpolate(a.value, b.value, t, a.interp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::angle::Angle;

    fn f32_anim(base: f32) -> Animatable {
        Animatable::constant(PropValue::F32(base))
    }

    fn at(anim: &Animatable, step: u32) -> f32 {
        anim.value_at(step).as_f32().unwrap()
    }

    #[test]
    fn constant_returns_base_at_every_step() {
        let anim = f32_anim(7.0);
        assert!(!anim.is_animated());
        for step in [0, 1, 100, u32::MAX] {
            assert_eq!(at(&anim, step), 7.0);
        }
    }

    #[test]
    fn keys_are_kept_sorted_regardless_of_insertion_order() {
        let mut anim = f32_anim(0.0);
        for step in [30, 10, 20, 5] {
            anim.set_key(step, PropValue::F32(step as f32), Interpolation::Linear);
        }
        let steps: Vec<u32> = anim.keys().iter().map(|k| k.step).collect();
        assert_eq!(steps, vec![5, 10, 20, 30]);
    }

    #[test]
    fn setting_a_key_twice_replaces_it() {
        let mut anim = f32_anim(0.0);
        anim.set_key(5, PropValue::F32(1.0), Interpolation::Linear);
        anim.set_key(5, PropValue::F32(9.0), Interpolation::Linear);
        assert_eq!(anim.keys().len(), 1);
        assert_eq!(at(&anim, 5), 9.0);
    }

    /// No extrapolation in either direction.
    #[test]
    fn outside_the_key_range_the_nearest_value_holds() {
        let mut anim = f32_anim(0.0);
        anim.set_key(10, PropValue::F32(100.0), Interpolation::Linear);
        anim.set_key(20, PropValue::F32(200.0), Interpolation::Linear);

        assert_eq!(at(&anim, 0), 100.0);
        assert_eq!(at(&anim, 9), 100.0);
        assert_eq!(at(&anim, 21), 200.0);
        assert_eq!(at(&anim, 9999), 200.0);
    }

    #[test]
    fn interpolates_between_neighbouring_keys() {
        let mut anim = f32_anim(0.0);
        anim.set_key(0, PropValue::F32(0.0), Interpolation::Linear);
        anim.set_key(10, PropValue::F32(100.0), Interpolation::Linear);

        assert_eq!(at(&anim, 0), 0.0);
        assert_eq!(at(&anim, 5), 50.0);
        assert_eq!(at(&anim, 10), 100.0);
    }

    /// The interpolation belongs to the key the segment *leaves*, so a `Step`
    /// key holds its value right up to the next one.
    #[test]
    fn the_leaving_key_governs_the_segment() {
        let mut anim = f32_anim(0.0);
        anim.set_key(0, PropValue::F32(0.0), Interpolation::Step);
        anim.set_key(10, PropValue::F32(100.0), Interpolation::Linear);
        anim.set_key(20, PropValue::F32(200.0), Interpolation::Linear);

        assert_eq!(at(&anim, 9), 0.0, "step segment must hold");
        assert_eq!(at(&anim, 15), 150.0, "linear segment must blend");
    }

    #[test]
    fn three_keys_use_the_correct_segment() {
        let mut anim = f32_anim(0.0);
        anim.set_key(0, PropValue::F32(0.0), Interpolation::Linear);
        anim.set_key(10, PropValue::F32(100.0), Interpolation::Linear);
        anim.set_key(30, PropValue::F32(0.0), Interpolation::Linear);

        assert_eq!(at(&anim, 5), 50.0);
        assert_eq!(at(&anim, 20), 50.0);
    }

    #[test]
    fn angle_keys_take_the_shortest_arc() {
        let mut anim = Animatable::constant(PropValue::Angle(Angle::new(0.0)));
        anim.set_key(
            0,
            PropValue::Angle(Angle::new(350.0)),
            Interpolation::Linear,
        );
        anim.set_key(
            10,
            PropValue::Angle(Angle::new(10.0)),
            Interpolation::Linear,
        );

        let mid = anim.value_at(5).as_angle().unwrap().degrees();
        assert!(mid.abs() < 1e-9 || (mid - 360.0).abs() < 1e-9, "got {mid}");
    }

    /// Discrete kinds cannot store a blending interpolation even if asked.
    #[test]
    fn discrete_kinds_coerce_to_step() {
        let mut anim = Animatable::constant(PropValue::Bool(false));
        anim.set_key(0, PropValue::Bool(false), Interpolation::EaseInOut);
        assert_eq!(anim.keys()[0].interp, Interpolation::Step);

        anim.set_key(10, PropValue::Bool(true), Interpolation::Linear);
        assert_eq!(anim.value_at(9).as_bool(), Some(false));
        assert_eq!(anim.value_at(10).as_bool(), Some(true));
    }

    #[test]
    fn moving_a_key_preserves_its_value() {
        let mut anim = f32_anim(0.0);
        anim.set_key(5, PropValue::F32(42.0), Interpolation::Linear);
        assert!(anim.move_key(5, 15));
        assert_eq!(anim.keys().len(), 1);
        assert_eq!(anim.keys()[0].step, 15);
        assert_eq!(at(&anim, 15), 42.0);
        assert!(
            !anim.move_key(99, 1),
            "moving a missing key must report failure"
        );
    }

    #[test]
    fn removing_a_key_returns_it() {
        let mut anim = f32_anim(0.0);
        anim.set_key(5, PropValue::F32(42.0), Interpolation::Linear);
        assert_eq!(
            anim.remove_key(5).map(|k| k.value),
            Some(PropValue::F32(42.0))
        );
        assert!(anim.remove_key(5).is_none());
    }

    /// Backs the quantified confirmation required when `step_count` shrinks.
    #[test]
    fn truncation_counts_before_it_deletes() {
        let mut anim = f32_anim(0.0);
        for step in [0, 10, 20, 30, 40] {
            anim.set_key(step, PropValue::F32(1.0), Interpolation::Linear);
        }
        assert_eq!(anim.count_after(20), 2);
        assert_eq!(anim.truncate_to(20), 2);
        assert_eq!(anim.keys().len(), 3);
        assert_eq!(anim.count_after(20), 0);
    }

    #[test]
    fn shifting_moves_every_key() {
        let mut anim = f32_anim(0.0);
        anim.set_key(2, PropValue::F32(1.0), Interpolation::Linear);
        anim.set_key(6, PropValue::F32(2.0), Interpolation::Linear);

        anim.shift_keys(3, 20);
        let steps: Vec<u32> = anim.keys().iter().map(|k| k.step).collect();
        assert_eq!(steps, vec![5, 9]);
    }

    #[test]
    fn shifting_backwards_clamps_at_zero() {
        let mut anim = f32_anim(0.0);
        anim.set_key(1, PropValue::F32(1.0), Interpolation::Linear);
        anim.set_key(8, PropValue::F32(2.0), Interpolation::Linear);

        anim.shift_keys(-4, 20);
        let steps: Vec<u32> = anim.keys().iter().map(|k| k.step).collect();
        assert_eq!(
            steps,
            vec![0, 4],
            "the first key clamps rather than vanishing"
        );
    }

    /// Clamping can push two keys onto the same step; steps stay unique.
    #[test]
    fn a_collision_from_clamping_keeps_one_key() {
        let mut anim = f32_anim(0.0);
        anim.set_key(0, PropValue::F32(1.0), Interpolation::Linear);
        anim.set_key(3, PropValue::F32(2.0), Interpolation::Linear);

        anim.shift_keys(-10, 20);
        assert_eq!(anim.keys().len(), 1);
        assert_eq!(anim.keys()[0].step, 0);
    }

    #[test]
    fn round_trips_through_json() {
        let mut anim = f32_anim(1.5);
        anim.set_key(0, PropValue::F32(0.0), Interpolation::EaseInOut);
        anim.set_key(7, PropValue::F32(9.0), Interpolation::Step);

        let json = serde_json::to_string(&anim).unwrap();
        let back: Animatable = serde_json::from_str(&json).unwrap();
        assert_eq!(anim, back);
    }

    /// A constant property must not carry an empty `keys` array in the file.
    #[test]
    fn constants_serialise_without_a_keys_field() {
        let json = serde_json::to_string(&f32_anim(1.0)).unwrap();
        assert!(!json.contains("keys"), "got {json}");
    }

    /// Hand-edited files may carry unsorted or duplicated keys; loading must
    /// normalise rather than propagate a broken invariant.
    #[test]
    fn deserialisation_repairs_unsorted_and_duplicate_keys() {
        let json = r#"{"base":{"f32":0.0},"keys":[
            {"step":20,"value":{"f32":2.0},"interp":"linear"},
            {"step":5,"value":{"f32":1.0},"interp":"linear"},
            {"step":20,"value":{"f32":3.0},"interp":"linear"}
        ]}"#;
        let anim: Animatable = serde_json::from_str(json).unwrap();
        let steps: Vec<u32> = anim.keys().iter().map(|k| k.step).collect();
        assert_eq!(steps, vec![5, 20]);
    }
}
