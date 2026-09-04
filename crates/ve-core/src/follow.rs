//! Objects that follow other objects (spec.md 9.3, M13).
//!
//! A follower's `position` or `rotation_deg` is not its own: it is derived
//! from another object's, keeping the offset it had when the link was made.
//! The offset is kept **in the primary's frame**, so a follower orbits a
//! turning primary as a rigid part of it rather than sliding beside it.
//!
//! The link lives on the follower's [`Animatable`](crate::keyframe::Animatable)
//! and the follower's own keys are kept, dormant, beneath it — so unlinking is
//! an exact reversal and a link is never a destructive edit.
//!
//! **Resolution is a pre-pass over the whole project, once per step.** A
//! follower cannot be resolved from itself: it needs its primary resolved
//! first, and its primary may follow something in turn. Walking the chain at
//! every sample would be quadratic and would need a cycle guard at every
//! level; doing it once, in dependency order, needs one guard and gives every
//! consumer — the field, the map's outlines, hit testing — the same answer.

use std::collections::BTreeMap;

use crate::angle::Angle;
use crate::document::Object;
use crate::id::Id;
use crate::keyframe::{Follow, FollowOffset};
use crate::project::Project;
use crate::schema::PropId;
use crate::value::PropValue;
use crate::{LonLat, geo};

/// What an object's own keys say about it, before any link is applied.
fn own_position(object: &Object, step: u32) -> Option<LonLat> {
    object
        .props
        .value_at(object.tool, PropId::Position, step)
        .and_then(PropValue::as_lonlat)
}

fn own_rotation(object: &Object, step: u32) -> f64 {
    object
        .props
        .value_at(object.tool, PropId::RotationDeg, step)
        .and_then(PropValue::as_angle)
        .map_or(0.0, |a| a.degrees())
}

/// The position and rotation a following property derives.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Derived {
    /// The anchor a `position` link produces, if the object has one.
    pub position: Option<LonLat>,
    /// The rotation a `rotation_deg` link produces, if the object has one.
    pub rotation: Option<f64>,
}

impl Derived {
    /// Whether anything was derived, which is false for most objects.
    pub fn is_empty(&self) -> bool {
        self.position.is_none() && self.rotation.is_none()
    }
}

/// Every object's derived values at one step.
///
/// Empty for a project with no links at all, which is every project until
/// someone makes one — so the common case costs one `is_empty` check.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    derived: BTreeMap<Id, Derived>,
}

impl Resolved {
    /// What an object derives, or nothing.
    pub fn of(&self, object: Id) -> Derived {
        self.derived.get(&object).copied().unwrap_or_default()
    }

    /// Whether any object in the project follows another.
    pub fn is_empty(&self) -> bool {
        self.derived.is_empty()
    }
}

/// The link on one of an object's two followable properties.
pub fn follow_of(object: &Object, id: PropId) -> Option<Follow> {
    if !followable(id) {
        return None;
    }
    object.props.get(id).and_then(|anim| anim.follow())
}

/// Whether a property can follow another object's at all.
///
/// Position and rotation only. A speed that followed another object's speed
/// would be a different feature with a different meaning, and every other
/// property is not a place in the world for an offset to be kept in.
pub fn followable(id: PropId) -> bool {
    matches!(id, PropId::Position | PropId::RotationDeg)
}

/// Resolves every link in a project at one step.
///
/// Objects are visited in dependency order by recursion with a visiting set,
/// which is also the cycle guard: a link that would close a loop is refused
/// when it is made ([`would_cycle`]), and this is the second line of defence
/// for a file that arrived with one anyway.
pub fn resolve(project: &Project, step: u32) -> Resolved {
    let objects: BTreeMap<Id, &Object> = project
        .layers
        .iter()
        .flat_map(|layer| layer.objects.iter())
        .map(|object| (object.id, object))
        .collect();
    // Nothing follows anything: the overwhelmingly common case, and worth not
    // walking the project twice for.
    if !objects.values().any(|object| {
        follow_of(object, PropId::Position).is_some()
            || follow_of(object, PropId::RotationDeg).is_some()
    }) {
        return Resolved::default();
    }

    let mut done: BTreeMap<Id, Place> = BTreeMap::new();
    let mut visiting: Vec<Id> = Vec::new();
    for &id in objects.keys() {
        place_of(id, &objects, step, &mut done, &mut visiting);
    }

    let derived = done
        .into_iter()
        .filter_map(|(id, place)| {
            let object = objects.get(&id)?;
            let d = Derived {
                position: follow_of(object, PropId::Position).map(|_| place.position),
                rotation: follow_of(object, PropId::RotationDeg).map(|_| place.rotation),
            };
            (!d.is_empty()).then_some((id, d))
        })
        .collect();
    Resolved { derived }
}

/// Where an object ends up once its links are applied.
#[derive(Debug, Clone, Copy)]
struct Place {
    position: LonLat,
    rotation: f64,
}

/// Resolves one object, resolving whatever it follows first.
///
/// A cycle, or a link to an object that is not there, leaves the property on
/// the object's own keys: a broken link is inert rather than fatal, which is
/// what lets a follower survive its primary being deleted from under it.
fn place_of(
    id: Id,
    objects: &BTreeMap<Id, &Object>,
    step: u32,
    done: &mut BTreeMap<Id, Place>,
    visiting: &mut Vec<Id>,
) -> Option<Place> {
    if let Some(place) = done.get(&id) {
        return Some(*place);
    }
    let object = objects.get(&id)?;
    let own = Place {
        position: own_position(object, step)?,
        rotation: own_rotation(object, step),
    };
    if visiting.contains(&id) {
        // A cycle. Everything on it falls back to its own keys.
        return Some(own);
    }
    visiting.push(id);

    let mut place = own;
    if let Some(follow) = follow_of(object, PropId::Position)
        && let FollowOffset::Position {
            distance_m,
            bearing_deg,
        } = follow.offset
        && let Some(primary) = place_of(follow.primary, objects, step, done, visiting)
    {
        // The offset is measured against the primary's rotation, so it turns
        // with the primary: a follower is a rigid part of it.
        place.position = primary
            .position
            .destination(Angle::new(bearing_deg + primary.rotation), distance_m);
    }
    if let Some(follow) = follow_of(object, PropId::RotationDeg)
        && let FollowOffset::Rotation { delta_deg } = follow.offset
        && let Some(primary) = place_of(follow.primary, objects, step, done, visiting)
    {
        place.rotation = primary.rotation + delta_deg;
    }

    visiting.pop();
    done.insert(id, place);
    Some(place)
}

/// The offset to record when `follower` is linked to `primary` at `step`.
///
/// Read from where the two objects actually are, so making a link never moves
/// anything: the follower's derived value at the moment of linking is exactly
/// where it already was.
pub fn offset_at(
    project: &Project,
    follower: Id,
    primary: Id,
    id: PropId,
    step: u32,
) -> Option<FollowOffset> {
    let resolved = resolve(project, step);
    let place = |object: Id| -> Option<(LonLat, f64)> {
        let found = project.object(object)?;
        let derived = resolved.of(object);
        Some((
            derived.position.or_else(|| own_position(found, step))?,
            derived
                .rotation
                .unwrap_or_else(|| own_rotation(found, step)),
        ))
    };
    let (follower_at, follower_rot) = place(follower)?;
    let (primary_at, primary_rot) = place(primary)?;
    Some(match id {
        PropId::Position => FollowOffset::Position {
            distance_m: geo::LonLat::distance_m(primary_at, follower_at),
            bearing_deg: primary_at.initial_bearing(follower_at).degrees() - primary_rot,
        },
        PropId::RotationDeg => FollowOffset::Rotation {
            delta_deg: follower_rot - primary_rot,
        },
        _ => return None,
    })
}

/// Whether linking `follower` to `primary` on `id` would close a loop.
///
/// Refused at the command rather than tolerated at resolution: a cycle has no
/// meaning — every object in it would be defined by the others — and a user
/// who made one by accident would see a set of objects stop responding with
/// nothing to point at.
pub fn would_cycle(project: &Project, follower: Id, primary: Id, id: PropId) -> bool {
    if follower == primary {
        return true;
    }
    let mut at = primary;
    // The chain cannot be longer than the project, so this terminates even if
    // a file arrived with a loop already in it.
    for _ in 0..project.object_count().saturating_add(1) {
        let Some(object) = project.object(at) else {
            return false;
        };
        let Some(follow) = follow_of(object, id) else {
            return false;
        };
        if follow.primary == follower {
            return true;
        }
        at = follow.primary;
    }
    true
}
