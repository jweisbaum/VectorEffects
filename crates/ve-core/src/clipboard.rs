//! Copying and pasting objects.
//!
//! The clipboard holds whole objects, keyframes included, so a pasted copy
//! carries its animation rather than a snapshot of one step (spec.md 8.5).
//!
//! Copies always receive fresh identities. Two objects sharing an id would be
//! the same object to every command that follows, so an edit to one would move
//! both — and undo would put back whichever the history happened to name.

use serde::{Deserialize, Serialize};

use crate::document::{MotionFlags, Object, StepRange};
use crate::id::Id;
use crate::keyframe::Animatable;
use crate::schema::PropId;
use crate::value::PropValue;

/// How pasted keyframes are placed in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteTiming {
    /// Shift the animation so it begins at the paste step. The default: a copy
    /// pasted at step 12 usually means "do that again, here".
    Relative,
    /// Keep the original step numbers.
    Absolute,
    /// No animation at all: the copy as it was at the step it was copied
    /// from, at every step (M27). Every property is frozen at that step's
    /// value with its keys dropped, the motion switches are off, a follower
    /// stands alone, and the range is the whole timeline — a still has no
    /// timing to shift.
    Still,
}

/// Degrees the anchor moves when a copy would land exactly on its original.
///
/// Enough to be visible and grabbable at a working zoom, small enough not to
/// look like a mistake. Only applied when the copy would otherwise be hidden
/// underneath the object it came from.
const NUDGE_DEGREES: f64 = 2.0;

/// Objects held for pasting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clipboard {
    objects: Vec<Object>,
    /// The step they were copied from, for relative timing.
    source_step: u32,
}

impl Clipboard {
    /// Takes a copy of `objects`, as they were at `step`.
    pub fn copy(objects: Vec<Object>, step: u32) -> Self {
        Self {
            objects,
            source_step: step,
        }
    }

    /// Whether there is anything to paste.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// How many objects are held.
    /// The tools the held objects were drawn with, in order: what a paste
    /// puts into a layer, for the layer to accept or refuse (M31).
    pub fn tools(&self) -> impl Iterator<Item = crate::schema::ToolKind> + '_ {
        self.objects.iter().map(|object| object.tool)
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Builds the objects to insert.
    ///
    /// `nudge` moves the copies slightly, for pasting back where they came
    /// from — otherwise the copy sits exactly under the original and looks like
    /// nothing happened.
    pub fn paste(
        &self,
        step: u32,
        timing: PasteTiming,
        last_step: u32,
        nudge: bool,
    ) -> Vec<Object> {
        // Fresh ids first, so a link inside the copied set can be remapped
        // onto the copies rather than left pointing at the originals.
        let ids: Vec<Id> = self.objects.iter().map(|_| Id::new()).collect();
        let remap: std::collections::BTreeMap<Id, Id> = self
            .objects
            .iter()
            .zip(&ids)
            .map(|(source, &fresh)| (source.id, fresh))
            .collect();

        self.objects
            .iter()
            .zip(&ids)
            .map(|(source, &fresh)| {
                let mut copy = source.clone();
                copy.id = fresh;
                copy.name = format!("{} copy", source.name);

                // A copied pair keeps its link, remapped onto the copies; a
                // follower copied without its primary stands alone, because a
                // link to an object the paste did not bring is a link to
                // whatever happens to be there (spec.md 9.3, M13).
                for (_, animatable) in copy.props.iter_mut() {
                    if let Some(mut follow) = animatable.follow() {
                        match remap.get(&follow.primary) {
                            Some(&fresh_primary) => {
                                follow.primary = fresh_primary;
                                animatable.set_follow(Some(follow));
                            }
                            None => animatable.set_follow(None),
                        }
                    }
                }

                if timing == PasteTiming::Still {
                    for (_, animatable) in copy.props.iter_mut() {
                        *animatable = Animatable::constant(animatable.value_at(self.source_step));
                    }
                    copy.motion = MotionFlags::default();
                    copy.active_range = StepRange::new(0, last_step);
                }

                if timing == PasteTiming::Relative {
                    let delta = i64::from(step) - i64::from(copy.active_range.start);
                    let start = (i64::from(copy.active_range.start) + delta)
                        .clamp(0, i64::from(last_step)) as u32;
                    let end = (i64::from(copy.active_range.end) + delta)
                        .clamp(0, i64::from(last_step)) as u32;
                    copy.active_range = crate::document::StepRange::new(start, end);

                    for (_, animatable) in copy.props.iter_mut() {
                        animatable.shift_keys(delta, last_step);
                    }
                }

                if nudge
                    && let Some(position) = copy.props.get_mut(PropId::Position)
                    && let PropValue::LonLat(anchor) = position.base()
                {
                    position.set_base(PropValue::LonLat(crate::geo::LonLat {
                        lon: crate::geo::normalize_lon(anchor.lon + NUDGE_DEGREES),
                        lat: anchor.lat,
                    }));
                }

                copy
            })
            .collect()
    }

    /// The step the objects were copied from.
    pub fn source_step(&self) -> u32 {
        self.source_step
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::StepRange;
    use crate::schema::ToolKind;
    use crate::value::Interpolation;

    fn object(name: &str, start: u32, end: u32) -> Object {
        let mut object = Object::new(ToolKind::Brush, name, 24);
        object.active_range = StepRange::new(start, end);
        if let Some(speed) = object.props.get_mut(PropId::Speed) {
            speed.set_key(start, PropValue::F32(5.0), Interpolation::Linear);
            speed.set_key(end, PropValue::F32(20.0), Interpolation::Linear);
        }
        object
    }

    #[test]
    fn an_empty_clipboard_pastes_nothing() {
        let clipboard = Clipboard::default();
        assert!(clipboard.is_empty());
        assert!(
            clipboard
                .paste(0, PasteTiming::Relative, 23, false)
                .is_empty()
        );
    }

    /// Two objects sharing an id would be the same object to every later
    /// command, so a copy must always get a fresh one.
    #[test]
    fn copies_get_fresh_identities() {
        let source = object("stroke", 0, 5);
        let original = source.id;
        let clipboard = Clipboard::copy(vec![source], 0);

        let pasted = clipboard.paste(0, PasteTiming::Absolute, 23, false);
        assert_eq!(pasted.len(), 1);
        assert_ne!(pasted[0].id, original);
        assert_eq!(pasted[0].name, "stroke copy");

        // And two pastes are distinct from each other.
        let again = clipboard.paste(0, PasteTiming::Absolute, 23, false);
        assert_ne!(pasted[0].id, again[0].id);
    }

    #[test]
    fn absolute_timing_keeps_the_original_steps() {
        let clipboard = Clipboard::copy(vec![object("a", 4, 9)], 4);
        let pasted = clipboard.paste(15, PasteTiming::Absolute, 23, false);

        assert_eq!(pasted[0].active_range, StepRange::new(4, 9));
        let steps: Vec<u32> = pasted[0]
            .props
            .get(PropId::Speed)
            .expect("speed")
            .keys()
            .iter()
            .map(|k| k.step)
            .collect();
        assert_eq!(steps, vec![4, 9]);
    }

    /// The default: a copy pasted at step 15 begins there, animation intact.
    #[test]
    fn relative_timing_moves_the_animation_to_the_paste_step() {
        let clipboard = Clipboard::copy(vec![object("a", 4, 9)], 4);
        let pasted = clipboard.paste(15, PasteTiming::Relative, 23, false);

        assert_eq!(
            pasted[0].active_range,
            StepRange::new(15, 20),
            "shifted by 11"
        );
        let steps: Vec<u32> = pasted[0]
            .props
            .get(PropId::Speed)
            .expect("speed")
            .keys()
            .iter()
            .map(|k| k.step)
            .collect();
        assert_eq!(steps, vec![15, 20], "the keyframes travel with it");
    }

    /// A still paste is the copy as it was at the copy step, everywhere:
    /// no keys, no motion, no timing, the whole timeline.
    #[test]
    fn a_still_paste_freezes_the_copy_at_the_step_it_was_copied_from() {
        // Speed runs 5 -> 20 over steps 4..9, so at step 6 it is 11.
        let mut source = object("a", 4, 9);
        source.motion.position = true;
        let clipboard = Clipboard::copy(vec![source], 6);
        let pasted = clipboard.paste(15, PasteTiming::Still, 23, false);

        let speed = pasted[0].props.get(PropId::Speed).expect("speed");
        assert!(speed.keys().is_empty(), "no keys survive a still paste");
        assert_eq!(speed.base(), PropValue::F32(11.0));
        assert_eq!(pasted[0].active_range, StepRange::new(0, 23));
        assert!(!pasted[0].motion.any(), "the motion switches are off");
    }

    /// Pasting near the end must not silently discard the animation.
    #[test]
    fn a_paste_near_the_end_clamps_rather_than_dropping_keys() {
        let clipboard = Clipboard::copy(vec![object("a", 0, 10)], 0);
        let pasted = clipboard.paste(20, PasteTiming::Relative, 23, false);

        assert_eq!(pasted[0].active_range, StepRange::new(20, 23));
        assert!(
            !pasted[0]
                .props
                .get(PropId::Speed)
                .expect("speed")
                .keys()
                .is_empty(),
            "the keys must survive, clamped"
        );
    }

    /// A copy pasted exactly on top of its original looks like nothing
    /// happened, so it is nudged aside.
    #[test]
    fn a_nudged_paste_lands_beside_the_original() {
        let mut source = object("a", 0, 5);
        source
            .props
            .get_mut(PropId::Position)
            .expect("position")
            .set_base(PropValue::LonLat(crate::geo::LonLat {
                lon: 10.0,
                lat: 20.0,
            }));
        let clipboard = Clipboard::copy(vec![source], 0);

        let plain = clipboard.paste(0, PasteTiming::Relative, 23, false);
        let nudged = clipboard.paste(0, PasteTiming::Relative, 23, true);

        let anchor = |object: &Object| {
            object
                .props
                .get(PropId::Position)
                .expect("position")
                .base()
                .as_lonlat()
                .expect("a position")
        };
        assert_eq!(anchor(&plain[0]).lon, 10.0);
        assert_eq!(anchor(&nudged[0]).lon, 12.0);
        assert_eq!(anchor(&nudged[0]).lat, 20.0, "only longitude moves");
    }

    #[test]
    fn a_nudge_wraps_across_the_antimeridian() {
        let mut source = object("a", 0, 5);
        source
            .props
            .get_mut(PropId::Position)
            .expect("position")
            .set_base(PropValue::LonLat(crate::geo::LonLat {
                lon: 179.0,
                lat: 0.0,
            }));
        let clipboard = Clipboard::copy(vec![source], 0);

        let pasted = clipboard.paste(0, PasteTiming::Relative, 23, true);
        let lon = pasted[0]
            .props
            .get(PropId::Position)
            .expect("position")
            .base()
            .as_lonlat()
            .expect("a position")
            .lon;
        assert!((lon + 179.0).abs() < 1e-9, "expected -179, got {lon}");
    }

    #[test]
    fn several_objects_paste_together() {
        let clipboard = Clipboard::copy(vec![object("a", 0, 3), object("b", 2, 8)], 0);
        assert_eq!(clipboard.len(), 2);

        let pasted = clipboard.paste(10, PasteTiming::Relative, 23, false);
        assert_eq!(pasted.len(), 2);
        // Each is shifted by its own offset from the paste step.
        assert_eq!(pasted[0].active_range, StepRange::new(10, 13));
        assert_eq!(pasted[1].active_range, StepRange::new(10, 16));
    }
}
