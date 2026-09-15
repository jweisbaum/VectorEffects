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
    /// Whether this track can put the object's own movement into the field
    /// (spec.md 9.3, M13).
    ///
    /// Only position, rotation and scale move an object, and only the tools
    /// that paint a vector have a field to add it to: a modifier writes what
    /// it read and a mask writes calm, so neither has anything to carry.
    pub motion_available: bool,
    /// Whether it is doing so.
    pub motion: bool,
    /// Whether this track can follow another object's (spec.md 9.3, M13).
    ///
    /// Position and rotation only: a speed that followed another object's
    /// speed would be a different feature, and no other property is a place
    /// in the world for an offset to be kept in.
    pub can_follow: bool,
    /// The object this track follows, if it follows one. Its keys are then
    /// dormant: the value comes from the link.
    pub follows: Option<u64>,
    /// That object's name, for the row.
    pub follows_name: Option<String>,
    /// The keys this track actually moves by, when it follows another (M65).
    ///
    /// A follower's own keys go dormant under the link, so its track has none
    /// to draw and the object moves anyway. What moves it is the primary's —
    /// or the primary's primary's, if the link is a chain — so those are shown
    /// here, to be drawn as borrowed and edited on the row that owns them.
    /// Empty for a track that follows nothing.
    pub inherited: Vec<KeyframeView>,
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

/// One numeric component of a property, sampled at every step.
///
/// A scalar or an angle has one; a position has two, longitude and latitude,
/// which share an axis because they share a unit.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "TrackSeries.ts")]
pub struct TrackSeries {
    /// Empty for a property with a single component; otherwise which one.
    pub label: String,
    /// Display unit, in [`crate::document::PropertyView`]'s vocabulary. The
    /// frontend converts — a speed is sampled in m/s and graphed in the preferred speed unit.
    pub unit: String,
    /// The value at every step of the project, in step order.
    pub values: Vec<f64>,
}

/// A property's value at every step, for the timeline's value graph (spec.md 9.3).
///
/// Sampled here rather than interpolated in the frontend: easing curves, the
/// shortest-arc angle and the great-circle position are the model's, and a
/// graph drawn from a second implementation of them would eventually disagree
/// with the field it claims to describe.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "TrackSamples.ts")]
pub struct TrackSamples {
    /// Which object.
    pub object: u64,
    /// The property id, as the inspector spells it.
    pub property: String,
    /// Its label.
    pub label: String,
    /// One series per numeric component, empty for a kind that has no
    /// magnitude — a boolean or a choice graphs nothing.
    pub series: Vec<TrackSeries>,
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
fn track_of(
    tool: ToolKind,
    id: PropId,
    anim: &Animatable,
    step: u32,
    motion: ve_core::document::MotionFlags,
    project: &ve_core::project::Project,
) -> Option<TrackView> {
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
        can_follow: ve_core::follow::followable(id),
        follows: anim.follow().map(|f| f.primary.raw()),
        follows_name: anim
            .follow()
            .and_then(|f| project.object(f.primary))
            .map(|primary| primary.name.clone()),
        inherited: anim
            .follow()
            .map(|link| inherited_keys(project, link.primary, id))
            .unwrap_or_default(),
        motion_available: motion_track(tool, id).is_some(),
        motion: match motion_track(tool, id) {
            Some(MotionTrack::Position) => motion.position,
            Some(MotionTrack::Rotation) => motion.rotation,
            Some(MotionTrack::Scale) => motion.scale,
            None => false,
        },
    })
}

/// Which of the three movements a track is, if it is one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionTrack {
    /// The anchor's travel.
    Position,
    /// The frame's turn.
    Rotation,
    /// The geometry's growth.
    Scale,
}

/// The movement a property carries, for the tools that paint a vector.
///
/// A modifier writes what it read and a mask writes calm, so neither has a
/// field of its own for a velocity to be added to (spec.md 6.3, 9.3).
pub fn motion_track(tool: ToolKind, id: PropId) -> Option<MotionTrack> {
    if tool.is_modifier() || tool == ToolKind::Mask {
        return None;
    }
    match id {
        PropId::Position => Some(MotionTrack::Position),
        PropId::RotationDeg => Some(MotionTrack::Rotation),
        PropId::ScalePct => Some(MotionTrack::Scale),
        _ => None,
    }
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
        let mut tracks: Vec<TrackView> = schema::all_specs(target.tool)
            // Not merely editable: keyable (M60). A mode that decides which
            // other properties are live cannot be animated, because keying it
            // changes what the object has half way along the timeline.
            .filter(|spec| schema::animatable(target.tool, spec.id))
            .filter(|spec| schema::is_live(target.tool, spec.id, choice_of))
            .filter_map(|spec| {
                target.props.get(spec.id).and_then(|anim| {
                    track_of(target.tool, spec.id, anim, step, target.motion, project)
                })
            })
            .collect();
        if target.tool.can_animate_shape() {
            tracks.insert(0, crate::shape_animation::track(target, step));
        }
        Ok(ObjectTracks {
            object,
            name: target.name.clone(),
            start_step: target.active_range.start,
            end_step: target.active_range.end,
            tracks,
        })
    })
}

/// The keys a following property is actually moved by (M65).
///
/// Walks up the chain from `primary`: an object whose own link is set has
/// dormant keys of its own, so what moves it is *its* primary's, and so on
/// until an object that follows nothing. The visited set is the same guard
/// `ve_core::follow` uses — a chain built into a ring must end the walk rather
/// than run forever.
///
/// Empty where there is nothing to show: a primary that has been deleted, or
/// one whose property has no keys, in which case the follower is rigid and
/// does not move.
fn inherited_keys(
    project: &ve_core::project::Project,
    primary: ve_core::Id,
    id: PropId,
) -> Vec<KeyframeView> {
    let mut at = primary;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        if !seen.insert(at) {
            return Vec::new();
        }
        let Some(object) = project.object(at) else {
            return Vec::new();
        };
        let Some(anim) = object.props.get(id) else {
            return Vec::new();
        };
        match anim.follow() {
            Some(link) => at = link.primary,
            None => {
                return anim
                    .keys()
                    .iter()
                    .map(|key| KeyframeView {
                        step: key.step,
                        value: PropertyValue::of(key.value),
                        interp: InterpolationView::of(key.interp),
                    })
                    .collect();
            }
        }
    }
}

/// Turns one of an object's own movements into the field it paints
/// (spec.md 9.3, M13).
///
/// One track at a time, because the checkbox is on the track row: a spinning
/// system that also travels may want its spin in the wind and not its
/// translation, and one switch per object cannot say which (D57).
#[tauri::command]
pub fn set_motion(
    state: tauri::State<'_, AppState>,
    object: u64,
    property: String,
    on: bool,
) -> Result<ProjectSummary> {
    motion_set(&state, object, &property, on)
}

/// Implementation of [`set_motion`].
pub fn motion_set(
    state: &AppState,
    object: u64,
    property: &str,
    on: bool,
) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let target = open
            .project
            .object(object_id(object))
            .ok_or(AppError::Core(ve_core::CoreError::MissingObject(object)))?;
        let bad = || AppError::BadOption {
            field: "property",
            value: property.to_owned(),
        };
        let id = schema::all_specs(target.tool)
            .map(|spec| spec.id)
            .find(|id| format!("{id:?}") == property)
            .ok_or_else(bad)?;
        let track = motion_track(target.tool, id).ok_or_else(bad)?;
        let mut after = target.motion;
        match track {
            MotionTrack::Position => after.position = on,
            MotionTrack::Rotation => after.rotation = on,
            MotionTrack::Scale => after.scale = on,
        }
        if after == target.motion {
            return Ok(ProjectSummary::of(open));
        }
        let command = Command::SetMotion {
            object: target.id,
            before: target.motion,
            after,
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// Makes one object's position or rotation follow another's (spec.md 9.3,
/// M13), or clears the link when `primary` is `null`.
///
/// Linking never moves anything: the offset is read from where the two
/// objects actually are, so the derived value at the moment of linking is
/// exactly where the follower already was.
#[tauri::command]
pub fn set_follow(
    state: tauri::State<'_, AppState>,
    object: u64,
    property: String,
    primary: Option<u64>,
    step: u32,
) -> Result<ProjectSummary> {
    follow_set(&state, object, &property, primary, step)
}

/// Implementation of [`set_follow`].
pub fn follow_set(
    state: &AppState,
    object: u64,
    property: &str,
    primary: Option<u64>,
    step: u32,
) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let bad = |why: String| AppError::BadOption {
            field: "follow",
            value: why,
        };
        let id = object_id(object);
        let target = open
            .project
            .object(id)
            .ok_or(AppError::Core(ve_core::CoreError::MissingObject(object)))?;
        let prop = schema::all_specs(target.tool)
            .map(|spec| spec.id)
            .find(|p| format!("{p:?}") == property)
            .filter(|p| ve_core::follow::followable(*p))
            .ok_or_else(|| {
                bad(format!(
                    "{property} is not a property that can follow another"
                ))
            })?;
        let before = target
            .props
            .get(prop)
            .ok_or_else(|| bad(property.to_owned()))?
            .clone();

        let mut after = before.clone();
        match primary {
            Some(raw) => {
                let primary_id = object_id(raw);
                if open.project.object(primary_id).is_none() {
                    return Err(AppError::Core(ve_core::CoreError::MissingObject(raw)));
                }
                // A cycle has no meaning — every object in it would be defined
                // by the others — and a user who made one by accident would
                // see a set of objects stop responding with nothing to point
                // at. Refused here rather than tolerated at resolution.
                if ve_core::follow::would_cycle(&open.project, id, primary_id, prop) {
                    return Err(bad(
                        "that would make a loop: the object you picked already follows this one"
                            .to_owned(),
                    ));
                }
                let offset = ve_core::follow::offset_at(&open.project, id, primary_id, prop, step)
                    .ok_or_else(|| bad("neither object is anywhere at this step".to_owned()))?;
                after.set_follow(Some(ve_core::keyframe::Follow {
                    primary: primary_id,
                    offset,
                }));
            }
            None => {
                if before.follow().is_none() {
                    return Ok(ProjectSummary::of(open));
                }
                // Unlinking wakes the follower's own keys, which may say it is
                // somewhere else entirely. Writing the derived value at the
                // current step keeps it where it is (D42): a key is the only
                // write that shows once a property has keys at all.
                let derived = ve_core::follow::resolve(&open.project, step).of(id);
                let held = match prop {
                    PropId::Position => derived.position.map(ve_core::PropValue::LonLat),
                    _ => derived
                        .rotation
                        .map(|d| ve_core::PropValue::Angle(ve_core::angle::Angle::new(d))),
                };
                after.set_follow(None);
                if let Some(value) = held {
                    // `true` for auto-key: the value has to land as a *key*
                    // whether or not auto-key is on, because a base write is
                    // invisible once the property has keys of its own and
                    // those are exactly what unlinking wakes up.
                    after = written(&after, step, true, value);
                    after.set_follow(None);
                }
            }
        }
        if after == before {
            return Ok(ProjectSummary::of(open));
        }
        let command = Command::SetFollow {
            object: id,
            prop,
            before: Box::new(before),
            after: Box::new(after),
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// One property's value at every step, for the timeline's value graph.
#[tauri::command]
pub fn track_samples(
    state: tauri::State<'_, AppState>,
    object: u64,
    property: String,
) -> Result<TrackSamples> {
    samples_of(&state, object, &property)
}

/// Implementation of [`track_samples`], callable without a Tauri handle.
///
/// Every step is asked of the model, keys and gaps alike, so the graph shows
/// the same numbers the evaluator will use — including the held value outside
/// the keyed range, which is the nearest key's and not the base (spec.md 4.5).
pub fn samples_of(state: &AppState, object: u64, property: &str) -> Result<TrackSamples> {
    with_session(state, |session| {
        let project = &session.require_open()?.project;
        let target = project
            .object(object_id(object))
            .ok_or(AppError::Core(ve_core::CoreError::MissingObject(object)))?;
        let spec = schema::all_specs(target.tool)
            .find(|spec| format!("{:?}", spec.id) == property)
            .ok_or_else(|| AppError::BadOption {
                field: "property",
                value: property.to_owned(),
            })?;
        let anim = target
            .props
            .get(spec.id)
            .ok_or_else(|| AppError::BadOption {
                field: "property",
                value: property.to_owned(),
            })?;
        let unit = crate::document::unit_name(spec.unit).to_owned();
        let values: Vec<PropValue> = (0..project.settings.step_count)
            .map(|step| anim.value_at(step))
            .collect();
        let component = |label: &str, unit: &str, of: fn(PropValue) -> Option<f64>| TrackSeries {
            label: label.to_owned(),
            unit: unit.to_owned(),
            values: values.iter().copied().filter_map(of).collect(),
        };
        let series = match anim.kind() {
            PropKind::F32 => vec![component("", &unit, |v| v.as_f32().map(f64::from))],
            PropKind::Angle => vec![component("", &unit, |v| v.as_angle().map(|a| a.degrees()))],
            // Both halves of a position are degrees whatever the property's
            // own unit says, and they share the axis.
            PropKind::LonLat => vec![
                component("Lon", "degrees", |v| v.as_lonlat().map(|p| p.lon)),
                component("Lat", "degrees", |v| v.as_lonlat().map(|p| p.lat)),
            ],
            PropKind::Bool | PropKind::Enum => Vec::new(),
        };
        Ok(TrackSamples {
            object,
            property: property.to_owned(),
            label: spec.label.to_owned(),
            series,
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
        // A key is an edit spread over time, so the property has to admit an
        // edit and has to be one whose value does not change what else the
        // object has (spec.md 6.1, 9.3, M60).
        if !schema::animatable(target.tool, prop) {
            let why = if schema::spec_for(target.tool, prop).is_some_and(|s| s.creation_only) {
                "is fixed when the object is created"
            } else {
                "cannot be animated"
            };
            return Err(AppError::BadOption {
                field: "property",
                value: format!("{property} {why}"),
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
#[tauri::command]
pub fn add_constant_motion(
    state: tauri::State<'_, AppState>,
    object: u64,
    step: u32,
    direction: f64,
    speed_mps: f64,
    overwrite: bool,
) -> Result<ProjectSummary> {
    constant_motion(&state, object, step, direction, speed_mps, overwrite)
}

/// Generate one position per project frame at a constant compass bearing.
/// Existing keys outside the segment remain intact; one undo restores all keys.
pub fn constant_motion(
    state: &AppState,
    object: u64,
    step: u32,
    direction: f64,
    speed_mps: f64,
    overwrite: bool,
) -> Result<ProjectSummary> {
    let bad = |value: &str| AppError::BadOption {
        field: "constant motion",
        value: value.to_owned(),
    };
    if !direction.is_finite() || !speed_mps.is_finite() || speed_mps < 0.0 {
        return Err(bad("enter a finite direction and a non-negative speed"));
    }
    let (seconds, end) = with_session(state, |session| {
        let open = session.require_open()?;
        let target = open
            .project
            .object(object_id(object))
            .ok_or(AppError::Core(ve_core::CoreError::MissingObject(object)))?;
        Ok((
            f64::from(open.project.settings.step_hours.hours()) * 3600.0,
            target.active_range.end.min(open.project.last_step()),
        ))
    })?;
    rewrite(state, object, "Position", None, |anim, _, last| {
        if anim.follow().is_some() {
            return Err(bad(
                "unlink the followed position before adding constant motion",
            ));
        }
        within(step, last)?;
        let end = anim
            .keys()
            .iter()
            .find(|key| key.step > step)
            .map_or(end, |key| key.step.min(end));
        if end <= step {
            return Err(bad(
                "select a frame before the end of the object's lifetime",
            ));
        }
        if !overwrite
            && anim
                .keys()
                .iter()
                .any(|key| key.step >= step && key.step <= end)
        {
            return Err(bad(
                "existing position keyframes would be overwritten; confirm to continue",
            ));
        }
        let start = anim
            .value_at(step)
            .as_lonlat()
            .ok_or_else(|| bad("position is unavailable"))?;
        for at in step..=end {
            let point = ve_core::geo::rhumb_destination(
                start,
                ve_core::angle::Angle::new(direction),
                speed_mps * seconds * f64::from(at - step),
            )
            .ok_or_else(|| {
                bad("this course reaches a pole; reduce the speed or change direction")
            })?;
            anim.set_key(at, PropValue::LonLat(point), Interpolation::Linear);
        }
        Ok(())
    })
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
    if property == "shape" {
        return crate::shape_animation::key_at(state, object, step);
    }
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
    if property == "shape" {
        return crate::shape_animation::unkey_at(state, object, step);
    }
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
    if property == "shape" {
        return crate::shape_animation::move_key(state, object, from, to, gesture);
    }
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
    if property == "shape" {
        return crate::shape_animation::ease_from(state, object, step, interp);
    }
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
#[tauri::command(async)]
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
