//! Keyframes, as the timeline and the inspector edit them (spec.md 9.3).
//!
//! Every edit here is a [`Command::SetProperty`], which swaps a property's
//! whole [`Animatable`] — keys included — for another. That is what makes
//! undo trivially exact: there is no per-key command to get the inverse of,
//! and a drag of many moves coalesces into one entry like every other gesture.
//!
//! The model already enforces what a key may hold: `Animatable::set_key`
//! coerces an interpolation an enum or a bool cannot use back to `Step`, and
//! keeps the key list sorted and unique. Nothing here re-checks those; a rule
//! that lived in two places would eventually be two rules.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_core::command::Command;
use ve_core::keyframe::Animatable;
use ve_core::schema::{self, PropId, ToolKind};
use ve_core::value::{Interpolation, PropKind, PropValue};

use crate::commands::AppState;
use crate::document::{PropertyValue, object_id};
use crate::error::{AppError, Result};
use crate::projects::{ProjectSummary, with_session};

/// How a keyframe's outgoing segment is eased, on the wire.
///
/// A mirror of [`Interpolation`], which cannot derive `TS` from another crate.
/// The hand-authored curve is carried whole so a project that has one survives
/// the timeline reading and writing its neighbours; the panel offers the named
/// presets.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[ts(export, export_to = "InterpolationView.ts")]
pub enum InterpolationView {
    /// Hold until the next key.
    Step,
    /// Constant rate.
    Linear,
    /// Slow start.
    EaseIn,
    /// Slow finish.
    EaseOut,
    /// Slow at both ends.
    EaseInOut,
    /// A hand-authored cubic.
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

impl InterpolationView {
    /// The model's interpolation.
    pub fn into_model(self) -> Interpolation {
        match self {
            Self::Step => Interpolation::Step,
            Self::Linear => Interpolation::Linear,
            Self::EaseIn => Interpolation::EaseIn,
            Self::EaseOut => Interpolation::EaseOut,
            Self::EaseInOut => Interpolation::EaseInOut,
            Self::Bezier { x1, y1, x2, y2 } => Interpolation::Bezier { x1, y1, x2, y2 },
        }
    }

    /// The wire form of a model interpolation.
    pub fn of(interp: Interpolation) -> Self {
        match interp {
            Interpolation::Step => Self::Step,
            Interpolation::Linear => Self::Linear,
            Interpolation::EaseIn => Self::EaseIn,
            Interpolation::EaseOut => Self::EaseOut,
            Interpolation::EaseInOut => Self::EaseInOut,
            Interpolation::Bezier { x1, y1, x2, y2 } => Self::Bezier { x1, y1, x2, y2 },
        }
    }
}

/// The interpolation a new key starts with.
///
/// Linear wherever the kind allows one: a keyed speed or position is nearly
/// always meant to move between its keys. Enums and booleans have no midpoint
/// and hold (spec.md 4.5).
pub fn default_interpolation(kind: PropKind) -> Interpolation {
    if Interpolation::Linear.is_valid_for(kind) {
        Interpolation::Linear
    } else {
        Interpolation::Step
    }
}

/// Where a change to a property goes (spec.md 9.3, D42).
///
/// A property with keys always keys the current step: once a key exists its
/// base is unreachable — the nearest key holds outside the keyed range — so a
/// base write would be an edit that changes nothing on screen. A property with
/// no keys edits its base, unless auto-key is on. An edit at a keyed step keeps
/// that key's interpolation; a new key takes the kind's default.
///
/// The inspector and the map's transform drags both write through here, so the
/// two cannot disagree about what a change at step 6 means.
pub fn written(before: &Animatable, step: u32, auto_key: bool, value: PropValue) -> Animatable {
    let mut after = before.clone();
    if auto_key || before.is_animated() {
        let interp = before
            .keys()
            .iter()
            .find(|key| key.step == step)
            .map_or_else(|| default_interpolation(before.kind()), |key| key.interp);
        after.set_key(step, value, interp);
    } else {
        after.set_base(value);
    }
    after
}

/// One keyframe, on the wire.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "KeyframeView.ts")]
pub struct KeyframeView {
    /// The step the key is pinned to.
    pub step: u32,
    /// Its value.
    pub value: PropertyValue,
    /// How the segment leaving it is eased.
    pub interp: InterpolationView,
}

/// One property's track: its base, its keys, and what the timeline may do to it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "TrackView.ts")]
pub struct TrackView {
    /// The property id, as the inspector spells it.
    pub property: String,
    /// Its label.
    pub label: String,
    /// The value used when there are no keys.
    pub base: PropertyValue,
    /// Every key, in step order.
    pub keys: Vec<KeyframeView>,
    /// The interpolations this property's kind allows (spec.md 4.5).
    pub interpolations: Vec<InterpolationView>,
    /// Whether a key sits on the step being viewed.
    pub keyed_here: bool,
    /// Whether the value at the step being viewed comes from interpolation
    /// between keys rather than from a key or the base (spec.md 9.3).
    pub interpolated_here: bool,
}

/// An object's tracks, for the timeline's tree.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ObjectTracks.ts")]
pub struct ObjectTracks {
    /// Which object.
    pub object: u64,
    /// Its name.
    pub name: String,
    /// First step of its lifetime.
    pub start_step: u32,
    /// Last step, inclusive.
    pub end_step: u32,
    /// Its editable properties, in schema order.
    pub tracks: Vec<TrackView>,
}

/// What reducing the timeline would delete (spec.md 4.1).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ShrinkImpact.ts")]
pub struct ShrinkImpact {
    /// Keyframes past the new end.
    pub keyframes: u32,
    /// Objects whose lifetime will be clamped.
    pub clamped_ranges: u32,
    /// Every object touched, by name.
    pub objects: Vec<String>,
}

/// The interpolations valid for a kind, in the order the panel offers them.
fn interpolations_for(kind: PropKind) -> Vec<InterpolationView> {
    [
        Interpolation::Step,
        Interpolation::Linear,
        Interpolation::EaseIn,
        Interpolation::EaseOut,
        Interpolation::EaseInOut,
    ]
    .into_iter()
    .filter(|interp| interp.is_valid_for(kind))
    .map(InterpolationView::of)
    .collect()
}

/// Reads one property's track at `step`.
fn track_of(tool: ToolKind, id: PropId, anim: &Animatable, step: u32) -> Option<TrackView> {
    let spec = schema::spec_for(tool, id)?;
    let keys = anim.keys();
    let keyed_here = keys.iter().any(|key| key.step == step);
    // Interpolated only strictly between two keys: on a key the value is the
    // key's, and outside the range the nearest key holds (spec.md 4.5).
    let interpolated_here = !keyed_here
        && keys.first().is_some_and(|first| first.step < step)
        && keys.last().is_some_and(|last| last.step > step);
    Some(TrackView {
        property: format!("{id:?}"),
        label: spec.label.to_owned(),
        base: PropertyValue::of(anim.base()),
        keys: keys
            .iter()
            .map(|key| KeyframeView {
                step: key.step,
                value: PropertyValue::of(key.value),
                interp: InterpolationView::of(key.interp),
            })
            .collect(),
        interpolations: interpolations_for(anim.kind()),
        keyed_here,
        interpolated_here,
    })
}

/// An object's tracks, as the timeline draws them.
#[tauri::command]
pub fn object_tracks(
    state: tauri::State<'_, AppState>,
    object: u64,
    step: u32,
) -> Result<ObjectTracks> {
    tracks_of(&state, object, step)
}

/// Implementation of [`object_tracks`], callable without a Tauri handle.
///
/// Lists exactly the properties the inspector edits — live ones, and not the
/// creation-only ones — so the two panels never disagree about what an object
/// has (spec.md 6.1). A creation-only property cannot be keyed: a key is an
/// edit spread over time.
pub fn tracks_of(state: &AppState, object: u64, step: u32) -> Result<ObjectTracks> {
    with_session(state, |session| {
        let project = &session.require_open()?.project;
        let target = project
            .object(object_id(object))
            .ok_or(AppError::Core(ve_core::CoreError::MissingObject(object)))?;
        let choice_of = |id: PropId| {
            target
                .props
                .value_at(target.tool, id, step)
                .and_then(PropValue::as_enum)
                .unwrap_or(0)
        };
        let tracks = schema::all_specs(target.tool)
            .filter(|spec| !spec.creation_only)
            .filter(|spec| schema::is_live(target.tool, spec.id, choice_of))
            .filter_map(|spec| {
                target
                    .props
                    .get(spec.id)
                    .and_then(|anim| track_of(target.tool, spec.id, anim, step))
            })
            .collect();
        Ok(ObjectTracks {
            object,
            name: target.name.clone(),
            start_step: target.active_range.start,
            end_step: target.active_range.end,
            tracks,
        })
    })
}

/// Rewrites one property's animatable through the undo stack.
///
/// The shape every keyframe edit takes: read the property, change a copy, and
/// commit the pair. `gesture` coalesces a drag into one history entry.
fn rewrite(
    state: &AppState,
    object: u64,
    property: &str,
    gesture: Option<String>,
    change: impl FnOnce(&mut Animatable, PropKind, u32) -> Result<()>,
) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let last = open.project.last_step();
        let target = open
            .project
            .object(object_id(object))
            .ok_or(AppError::Core(ve_core::CoreError::MissingObject(object)))?;
        let prop = schema::all_specs(target.tool)
            .map(|spec| spec.id)
            .find(|id| format!("{id:?}") == property)
            .ok_or_else(|| AppError::BadOption {
                field: "property",
                value: property.to_owned(),
            })?;
        // A key is an edit spread over time, and a creation-only property
        // admits no edit at all (spec.md 6.1).
        if schema::spec_for(target.tool, prop).is_some_and(|spec| spec.creation_only) {
            return Err(AppError::BadOption {
                field: "property",
                value: format!("{property} is fixed when the object is created"),
            });
        }
        let before = target
            .props
            .get(prop)
            .ok_or_else(|| AppError::BadOption {
                field: "property",
                value: property.to_owned(),
            })?
            .clone();

        let mut after = before.clone();
        change(&mut after, before.kind(), last)?;
        if after == before {
            // Nothing to record: a no-op in the history would be an undo step
            // that undoes nothing.
            return Ok(ProjectSummary::of(session.require_open()?));
        }

        let command = Command::SetProperty {
            object: object_id(object),
            prop,
            before: Box::new(before),
            after: Box::new(after),
        };
        let open = session.require_open()?;
        let (project, history) = (&mut open.project, &mut open.history);
        match gesture {
            Some(key) => history.push_coalesced(project, command, key)?,
            None => history.push(project, command)?,
        }
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// A step the timeline can hold, or an error naming why not.
fn within(step: u32, last: u32) -> Result<()> {
    if step > last {
        return Err(AppError::BadOption {
            field: "step",
            value: format!("{step} is past the last step, {last}"),
        });
    }
    Ok(())
}

/// Adds or replaces a key at `step`.
///
/// Without a value, the key takes the property's value *at that step* — which
/// for a step between two keys is the interpolated one, so "key this here"
/// pins what is on screen rather than snapping it to something else.
#[tauri::command]
pub fn set_keyframe(
    state: tauri::State<'_, AppState>,
    object: u64,
    property: String,
    step: u32,
    value: Option<PropertyValue>,
) -> Result<ProjectSummary> {
    key_at(&state, object, &property, step, value)
}

/// Implementation of [`set_keyframe`], callable without a Tauri handle.
pub fn key_at(
    state: &AppState,
    object: u64,
    property: &str,
    step: u32,
    value: Option<PropertyValue>,
) -> Result<ProjectSummary> {
    rewrite(state, object, property, None, |anim, kind, last| {
        within(step, last)?;
        let value = match value {
            Some(value) => value.into_prop(kind)?,
            None => anim.value_at(step),
        };
        let interp = anim
            .keys()
            .iter()
            .find(|key| key.step == step)
            .map_or_else(|| default_interpolation(kind), |key| key.interp);
        anim.set_key(step, value, interp);
        Ok(())
    })
}

/// Removes the key at `step`, if there is one.
#[tauri::command]
pub fn remove_keyframe(
    state: tauri::State<'_, AppState>,
    object: u64,
    property: String,
    step: u32,
) -> Result<ProjectSummary> {
    unkey_at(&state, object, &property, step)
}

/// Implementation of [`remove_keyframe`], callable without a Tauri handle.
pub fn unkey_at(
    state: &AppState,
    object: u64,
    property: &str,
    step: u32,
) -> Result<ProjectSummary> {
    rewrite(state, object, property, None, |anim, _, _| {
        anim.remove_key(step);
        Ok(())
    })
}

/// Moves a key from one step to another.
///
/// `gesture` is a drag's key: every move in the drag shares it, so the drag is
/// one history entry (spec.md 9.3, 8.4). Moving onto an occupied step replaces
/// the key there, which is what the model's `move_key` does and what dragging
/// one diamond onto another means.
#[tauri::command]
pub fn move_keyframe(
    state: tauri::State<'_, AppState>,
    object: u64,
    property: String,
    from: u32,
    to: u32,
    gesture: Option<String>,
) -> Result<ProjectSummary> {
    move_key(&state, object, &property, from, to, gesture)
}

/// Implementation of [`move_keyframe`], callable without a Tauri handle.
pub fn move_key(
    state: &AppState,
    object: u64,
    property: &str,
    from: u32,
    to: u32,
    gesture: Option<String>,
) -> Result<ProjectSummary> {
    rewrite(state, object, property, gesture, |anim, _, last| {
        within(to, last)?;
        if !anim.move_key(from, to) {
            return Err(AppError::BadOption {
                field: "step",
                value: format!("no key at step {from}"),
            });
        }
        Ok(())
    })
}

/// Sets how the segment leaving the key at `step` is eased.
#[tauri::command]
pub fn set_interpolation(
    state: tauri::State<'_, AppState>,
    object: u64,
    property: String,
    step: u32,
    interp: InterpolationView,
) -> Result<ProjectSummary> {
    ease_from(&state, object, &property, step, interp)
}

/// Implementation of [`set_interpolation`], callable without a Tauri handle.
///
/// An interpolation the kind cannot use is refused rather than coerced. The
/// model would coerce it to `Step` silently; a panel that offered "linear" on
/// a boolean and had it accepted would be lying about what it did.
pub fn ease_from(
    state: &AppState,
    object: u64,
    property: &str,
    step: u32,
    interp: InterpolationView,
) -> Result<ProjectSummary> {
    rewrite(state, object, property, None, |anim, kind, _| {
        let interp = interp.into_model();
        if !interp.is_valid_for(kind) {
            return Err(AppError::BadOption {
                field: "interp",
                value: format!("{interp:?} cannot ease a {kind:?}"),
            });
        }
        if !anim.set_key_interp(step, interp) {
            return Err(AppError::BadOption {
                field: "step",
                value: format!("no key at step {step}"),
            });
        }
        Ok(())
    })
}

/// What reducing the timeline to `step_count` would delete (spec.md 4.1).
#[tauri::command]
pub fn step_count_impact(
    state: tauri::State<'_, AppState>,
    step_count: u32,
) -> Result<ShrinkImpact> {
    shrink_impact(&state, step_count)
}

/// Implementation of [`step_count_impact`], callable without a Tauri handle.
pub fn shrink_impact(state: &AppState, step_count: u32) -> Result<ShrinkImpact> {
    with_session(state, |session| {
        let impact = session.require_open()?.project.shrink_impact(step_count);
        Ok(ShrinkImpact {
            keyframes: impact.keyframes,
            clamped_ranges: impact.clamped_ranges,
            objects: impact.objects.into_iter().map(|(_, name)| name).collect(),
        })
    })
}

/// Changes the number of time steps.
///
/// Growing is free. Shrinking deletes keyframes past the new end and clamps
/// lifetimes, undoably within the session (decision D13); the confirmation
/// that gates it is the frontend's, built from [`step_count_impact`].
#[tauri::command]
pub fn set_step_count(
    state: tauri::State<'_, AppState>,
    step_count: u32,
) -> Result<ProjectSummary> {
    resize_steps(&state, step_count)
}

/// Implementation of [`set_step_count`], callable without a Tauri handle.
pub fn resize_steps(state: &AppState, step_count: u32) -> Result<ProjectSummary> {
    if step_count == 0 || step_count > ve_core::project::MAX_STEPS {
        return Err(AppError::Core(ve_core::CoreError::InvalidStepCount(
            step_count,
        )));
    }
    with_session(state, |session| {
        let open = session.require_open()?;
        let before = open.project.settings.step_count;
        if before == step_count {
            return Ok(ProjectSummary::of(session.require_open()?));
        }
        let command = Command::SetStepCount {
            before,
            after: step_count,
            restore: Vec::new(),
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// Sets when step 0 is, or clears it (spec.md 9.1).
#[tauri::command]
pub fn set_start_time(
    state: tauri::State<'_, AppState>,
    start_unix_s: Option<i64>,
) -> Result<ProjectSummary> {
    start_at(&state, start_unix_s)
}

/// Implementation of [`set_start_time`], callable without a Tauri handle.
pub fn start_at(state: &AppState, start_unix_s: Option<i64>) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let before = open.project.settings.start_unix_s;
        if before == start_unix_s {
            return Ok(ProjectSummary::of(session.require_open()?));
        }
        let command = Command::SetStartTime {
            before,
            after: start_unix_s,
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_interpolations_round_trip_through_the_model() {
        for interp in [
            Interpolation::Step,
            Interpolation::Linear,
            Interpolation::EaseIn,
            Interpolation::EaseOut,
            Interpolation::EaseInOut,
            Interpolation::Bezier {
                x1: 0.1,
                y1: 0.2,
                x2: 0.3,
                y2: 0.4,
            },
        ] {
            assert_eq!(InterpolationView::of(interp).into_model(), interp);
        }
    }

    /// Spec 4.5: a bool or an enum can only hold. The panel is told so rather
    /// than left to find out from a refused write.
    #[test]
    fn a_holding_kind_is_offered_only_step() {
        assert_eq!(
            interpolations_for(PropKind::Bool),
            vec![InterpolationView::Step]
        );
        assert_eq!(
            interpolations_for(PropKind::Enum),
            vec![InterpolationView::Step]
        );
        assert!(interpolations_for(PropKind::F32).len() > 1);
        assert!(interpolations_for(PropKind::LonLat).contains(&InterpolationView::EaseInOut));
    }

    #[test]
    fn a_new_key_moves_where_it_can_and_holds_where_it_cannot() {
        assert_eq!(default_interpolation(PropKind::F32), Interpolation::Linear);
        assert_eq!(
            default_interpolation(PropKind::Angle),
            Interpolation::Linear
        );
        assert_eq!(
            default_interpolation(PropKind::LonLat),
            Interpolation::Linear
        );
        assert_eq!(default_interpolation(PropKind::Bool), Interpolation::Step);
        assert_eq!(default_interpolation(PropKind::Enum), Interpolation::Step);
    }
}
