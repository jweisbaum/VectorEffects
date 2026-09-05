//! Transforming a selection of one or more objects.
//!
//! Every drag on the map — move, rotate, scale, or repositioning an anchor —
//! comes through here. Three things make this its own module rather than a
//! branch in [`crate::document`]:
//!
//! * **Multi-selection transforms are about a collective centroid** (spec.md
//!   8.2), so the maths is not "set this object's property" but "place every
//!   object relative to a pivot".
//! * **A drag must be idempotent.** The pointer reports an absolute position
//!   many times a second, and each report must produce the same result as if it
//!   were the first. So every update is computed from a *baseline* captured at
//!   pointer-down, never from the current state — applying a delta to a value
//!   that already includes it is how a drag runs away.
//! * **All of it is geodesy**, and geodesy belongs on this side of the IPC
//!   boundary (spec.md 3.3). Rotating a selection at 60°N is not rotating in
//!   lat/lon space.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_core::angle::Angle;
use ve_core::command::Command;
use ve_core::document::{Geometry, LocalPoint};
use ve_core::keyframe::Animatable;
use ve_core::schema::PropId;

use crate::create::Tool;
use ve_core::{LonLat, PropValue};
use ve_render::aeqd::{Frame, Local, Space};
use ve_render::scene::flatten_object_at;
use ve_render::sdf::Shape;

use crate::commands::AppState;
use crate::document::object_id;
use crate::edit::StampSpace;
use crate::error::{AppError, Result};
use crate::projects::{ProjectSummary, with_session};
use crate::session::{TransformBaseline, TransformGesture};

/// The smallest scale a drag may produce, as a fraction. Matches the property
/// schema's floor, so a drag cannot ask for something the inspector rejects.
const MIN_SCALE_PCT: f64 = 1.0;

/// What a drag is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export, export_to = "TransformKind.ts")]
#[serde(rename_all = "snake_case")]
pub enum TransformKind {
    /// Move the selection, keeping its shape and orientation.
    Move,
    /// Rotate it about the pivot.
    Rotate,
    /// Scale it about the pivot.
    Scale,
    /// Move one object's anchor without moving its geometry.
    Anchor,
}

/// Where a selection's handles go.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "SelectionTransform.ts")]
pub struct SelectionTransform {
    /// Pivot longitude: the object's anchor, or the collective centroid.
    pub lon: f64,
    /// Pivot latitude.
    pub lat: f64,
    /// True-bearing rotation of the one selected object; 0 for a group, which
    /// has no orientation of its own until it is rotated.
    pub rotation_deg: f64,
    /// Scale of the one selected object; 100 for a group.
    pub scale_pct: f64,
    /// How far the selection reaches from the pivot, in metres.
    pub radius_m: f64,
    /// How many objects the handles act on.
    pub count: u32,
}

/// An object's footprint in its local frame, captured when a drag begins.
///
/// Enough to redraw the object under the pointer without re-reading the
/// document. Built from the flattened [`Shape`], which is the authority on what
/// an object covers, so the preview cannot describe a shape the evaluator does
/// not paint.
#[derive(Debug, Clone)]
pub enum BaselineOutline {
    /// A stamp swept along one or more chains: every painted geometry, and a
    /// lone disc, which is a chain of one point.
    Swept {
        /// Paths the stamp is swept along, in local metres.
        chains: Vec<Vec<Local>>,
        /// The stamp's radius, in local metres.
        radius_m: f64,
        /// Whether the stamp is a square rather than a disc.
        square: bool,
    },
    /// A closed ring, in local metres: a filled shape's own boundary.
    Ring(Vec<Local>),
}

/// How many points a preview outline sends per chain.
///
/// The preview is a proxy (spec.md 1.5, invariant 3), and a thinned chain is
/// indistinguishable from the full one at any zoom a drag happens at. It also
/// bounds the work per pointer report: a stroke of ten thousand points costs
/// the same as one of two hundred.
const PREVIEW_POINTS: usize = 192;

/// How many points a sampled circle becomes.
const RING_SEGMENTS: usize = 48;

impl BaselineOutline {
    /// Reads an object's footprint off its flattened shape.
    fn of(shape: &Shape) -> Self {
        match shape {
            Shape::Capsule { chains, radius_m } => Self::Swept {
                chains: chains.clone(),
                radius_m: *radius_m,
                square: false,
            },
            Shape::SweptSquare {
                chains,
                half_size_m,
            } => Self::Swept {
                chains: chains.clone(),
                radius_m: *half_size_m,
                square: true,
            },
            // A stamp with nowhere to travel is a disc, so one construction
            // covers both.
            Shape::Disc { radius_m } => Self::Swept {
                chains: vec![vec![[0.0, 0.0]]],
                radius_m: *radius_m,
                square: false,
            },
            // The outer edge stands for the ring: a preview says where the
            // object is, and the hole is not what a drag is aiming.
            Shape::Annulus {
                radius_m,
                half_width_m,
            } => Self::Ring(circle(radius_m + half_width_m)),
            Shape::Rect {
                half_width_m,
                half_height_m,
            } => Self::Ring(vec![
                [-half_width_m, -half_height_m],
                [*half_width_m, -half_height_m],
                [*half_width_m, *half_height_m],
                [-half_width_m, *half_height_m],
            ]),
            Shape::Polygon { ring } => Self::Ring(ring.clone()),
        }
    }
}

/// A closed ring of `RING_SEGMENTS` points at `radius_m`.
fn circle(radius_m: f64) -> Vec<Local> {
    (0..RING_SEGMENTS)
        .map(|i| {
            let theta = std::f64::consts::TAU * i as f64 / RING_SEGMENTS as f64;
            [radius_m * theta.cos(), radius_m * theta.sin()]
        })
        .collect()
}

/// Every `stride`-th point, and always the last one.
///
/// Keeping the last point matters: it is the end of the stroke, and a chain
/// that stopped short of it would preview a shorter object than the one being
/// dragged.
fn thinned(points: &[Local]) -> Vec<Local> {
    if points.len() <= PREVIEW_POINTS {
        return points.to_vec();
    }
    let stride = points.len().div_ceil(PREVIEW_POINTS);
    let mut out: Vec<Local> = points.iter().step_by(stride).copied().collect();
    if let (Some(last), Some(end)) = (points.last(), out.last())
        && last != end
    {
        out.push(*last);
    }
    out
}

/// One object's footprint under a drag, in geographic coordinates.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ObjectOutline.ts")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectOutline {
    /// Sweep the stamp along these chains.
    Swept {
        /// Chains of `[lon, lat]` points.
        chains: Vec<Vec<[f64; 2]>>,
        /// The stamp's radius in kilometres, as the map draws sizes.
        radius_km: f64,
        /// Whether the stamp is square.
        square: bool,
        /// Which space the stamp is a circle in (spec.md 3.5).
        space: StampSpace,
    },
    /// Fill this closed ring of `[lon, lat]` points.
    Ring {
        /// The ring, in order; the closing edge is implied.
        points: Vec<[f64; 2]>,
    },
}

/// Where a drag would put the selection, without putting it there.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "TransformPreview.ts")]
pub struct TransformPreview {
    /// Where the handles go while the pointer is here.
    pub handles: SelectionTransform,
    /// The selection's footprints, as they would be if the drag ended here.
    pub outlines: Vec<ObjectOutline>,
}

/// Handle geometry for a selection, without starting a drag.
#[tauri::command]
pub fn selection_transform(
    state: tauri::State<'_, AppState>,
    objects: Vec<u64>,
    step: u32,
) -> Result<Option<SelectionTransform>> {
    transform_of(&state, &objects, step)
}

/// Implementation of [`selection_transform`], callable without a Tauri handle.
pub fn transform_of(
    state: &AppState,
    objects: &[u64],
    step: u32,
) -> Result<Option<SelectionTransform>> {
    with_session(state, |session| {
        let project = &session.require_open()?.project;
        Ok(baseline_of(project, objects, step).map(|baseline| baseline.handles()))
    })
}

/// Captures the baseline for a drag and reports where the handles are.
///
/// `lon`/`lat` is where the pointer went down: rotation and scale are measured
/// against it, so grabbing a handle never snaps the selection to the cursor.
#[tauri::command]
pub fn begin_transform(
    state: tauri::State<'_, AppState>,
    objects: Vec<u64>,
    step: u32,
    kind: TransformKind,
    lon: f64,
    lat: f64,
    auto_key: Option<bool>,
) -> Result<Option<SelectionTransform>> {
    start_transform(
        &state,
        &objects,
        step,
        kind,
        lon,
        lat,
        auto_key.unwrap_or(false),
    )
}

/// Implementation of [`begin_transform`], callable without a Tauri handle.
///
/// `auto_key` is the frontend's switch (spec.md 9.3). A property that already
/// has keys is keyed at `step` whatever the switch says: its base shows at no
/// step, so changing the base would be an edit nobody could see.
pub fn start_transform(
    state: &AppState,
    objects: &[u64],
    step: u32,
    kind: TransformKind,
    lon: f64,
    lat: f64,
    auto_key: bool,
) -> Result<Option<SelectionTransform>> {
    let pointer = LonLat::new(lon, lat)?;
    with_session(state, |session| {
        let project = &session.require_open()?.project;
        let Some(baseline) = baseline_of(project, objects, step) else {
            return Ok(None);
        };
        let handles = baseline.handles();

        session.transform = Some(TransformGesture {
            // The key is what collapses the drag into one history entry. It
            // names the gesture, not the moment, so every update in the drag
            // shares it and none is shared with the next drag.
            key: format!("transform:{kind:?}:{}", session.next_gesture_id()),
            kind,
            step,
            auto_key,
            pointer,
            pivot: baseline.pivot,
            pointer_bearing: bearing_or(baseline.pivot, pointer, 0.0),
            pointer_distance_m: baseline.pivot.distance_m(pointer),
            items: baseline.items,
        });
        Ok(Some(handles))
    })
}

/// Where the drag in progress would leave the selection, without writing.
///
/// **This is what the pointer follows.** Applying the drag on every pointer
/// report instead bumps the document revision, and the revision addresses every
/// tile, so each report invalidated the whole visible field and asked for a
/// re-render costing tens to hundreds of milliseconds (spec.md 13). The renders
/// never finished before the next one replaced them, so the object appeared to
/// move only when the drag ended. Reading the destination and drawing it as an
/// overlay costs no evaluation at all, and the document is written once, when
/// the pointer comes up.
#[tauri::command]
pub fn preview_transform(
    state: tauri::State<'_, AppState>,
    lon: f64,
    lat: f64,
) -> Result<Option<TransformPreview>> {
    peek_transform(&state, lon, lat)
}

/// Implementation of [`preview_transform`], callable without a Tauri handle.
pub fn peek_transform(state: &AppState, lon: f64, lat: f64) -> Result<Option<TransformPreview>> {
    let pointer = LonLat::new(lon, lat)?;
    with_session(state, |session| {
        let Some(gesture) = session.transform.clone() else {
            return Ok(None);
        };
        let motion = motion_of(&gesture, pointer);

        let mut placed = Vec::with_capacity(gesture.items.len());
        let mut outlines = Vec::with_capacity(gesture.items.len());
        for item in &gesture.items {
            let to = placement_of(&gesture, &motion, item, pointer);
            // Repin leaves the geometry on the ground, so its outline is drawn
            // in the frame the object still has, not the one it is acquiring.
            let frame = if matches!(motion, Motion::Repin) {
                Frame::in_space(item.anchor, item.rotation_deg, item.scale_pct, item.space)
            } else {
                Frame::in_space(to.anchor, to.rotation_deg, to.scale_pct, item.space)
            };
            outlines.push(outline_of(&item.outline, &frame));
            // The reach as it would be at 100%, so that `handles_for` can apply
            // the *placed* scale to it. `reach_m` was captured from the flat
            // object and already carries the scale the object had; applying
            // the new scale on top of that multiplied the two, and the handles
            // of any object not at 100% jumped the moment it was grabbed.
            placed.push((to, item.reach_m * 100.0 / item.scale_pct.max(1.0)));
        }

        Ok(Some(TransformPreview {
            handles: handles_for(&placed, gesture.items.len()),
            outlines,
        }))
    })
}

/// One object's footprint, for the map to draw (spec.md 6.1, 6.2, 6.3).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "OperatorOutline.ts")]
pub struct OperatorOutline {
    /// Which object.
    pub object: u64,
    /// Which tool drew it, so the map can say what it has found.
    pub tool: Tool,
    /// Whether it covers everything *but* its footprint. Masks only.
    pub inverted: bool,
    /// Its anchor, as `[lon, lat]`.
    ///
    /// The end of a warp's push that a pull does not move (spec.md 6.3), and
    /// the frame every one of these outlines was lifted through.
    pub anchor: [f64; 2],
    /// Where its edge is.
    pub outline: ObjectOutline,
}

/// The footprints the map may need to draw at `step`.
///
/// Two audiences, one round trip. The **edge under the pointer**, which is
/// highlighted while a tool is in hand that draws objects like the one beneath
/// it — so `tool` selects them, and the answer is bounded by how many objects
/// that tool has drawn rather than by how many the project holds. And the
/// **selection**, whose operators are outlined because they have no field to
/// show where they are (spec.md 6.1, 6.2, 6.3).
///
/// Sent once per revision, step, tool and selection rather than per frame; the
/// hit test that decides which edge the pointer is near is the map's own, on
/// the outline it is already drawing.
///
/// Only what is visible: a hidden layer's objects are not drawn and an object
/// outside its active range at this step is not there to draw.
#[tauri::command]
pub fn object_outlines(
    state: tauri::State<'_, AppState>,
    step: u32,
    tool: Option<Tool>,
    objects: Vec<u64>,
) -> Result<Vec<OperatorOutline>> {
    outlines_at(&state, step, tool, &objects)
}

/// Implementation of [`object_outlines`], callable without a Tauri handle.
pub fn outlines_at(
    state: &AppState,
    step: u32,
    tool: Option<Tool>,
    objects: &[u64],
) -> Result<Vec<OperatorOutline>> {
    let wanted = tool.map(Tool::kind);
    with_session(state, |session| {
        let project = &session.require_open()?.project;
        // A follower is where its link puts it, not where its dormant keys
        // say (spec.md 9.3): the outline has to agree with the field.
        let links = ve_core::follow::resolve(project, step);
        let mut out = Vec::new();
        for layer in &project.layers {
            if !layer.visible {
                continue;
            }
            for object in &layer.objects {
                if Some(object.tool) != wanted && !objects.contains(&object.id.raw()) {
                    continue;
                }
                let Some(flat) = flatten_object_at(object, step, links.of(object.id)) else {
                    continue;
                };
                out.push(OperatorOutline {
                    object: object.id.raw(),
                    tool: Tool::of(object.tool),
                    inverted: flat.invert,
                    anchor: [flat.frame.anchor.lon, flat.frame.anchor.lat],
                    outline: outline_of(&BaselineOutline::of(&flat.shape), &flat.frame),
                });
            }
        }
        Ok(out)
    })
}

/// Lifts a captured outline into geographic coordinates through `frame`.
fn outline_of(outline: &BaselineOutline, frame: &Frame) -> ObjectOutline {
    let lift = |points: &[Local]| -> Vec<[f64; 2]> {
        thinned(points)
            .into_iter()
            .map(|p| {
                let global = frame.to_global(p);
                [global.lon, global.lat]
            })
            .collect()
    };

    match outline {
        BaselineOutline::Swept {
            chains,
            radius_m,
            square,
        } => ObjectOutline::Swept {
            chains: chains.iter().map(|chain| lift(chain)).collect(),
            // The stamp scales with the object, and the frame is what carries
            // the scale, so the radius has to be scaled here rather than sent
            // as it was captured.
            radius_km: radius_m * frame.scale / 1000.0,
            square: *square,
            space: match frame.space {
                Space::Geodesic => StampSpace::Geodesic,
                Space::Projected => StampSpace::Projected,
            },
        },
        BaselineOutline::Ring(ring) => ObjectOutline::Ring { points: lift(ring) },
    }
}

/// Handles for a set of placements, the same shape [`Baseline::handles`] makes.
///
/// Each placement carries its reach *at 100%*; the placed scale is applied
/// here. That is the one place a drag's handles can disagree with the
/// selection's, and `selection.rs` holds the two together at several scales.
fn handles_for(placed: &[(Placement, f64)], count: usize) -> SelectionTransform {
    let pivot = centroid(placed.iter().map(|(to, _)| to.anchor));
    let single = if placed.len() == 1 {
        placed.first()
    } else {
        None
    };
    let radius_m = placed
        .iter()
        .map(|(to, reach)| pivot.distance_m(to.anchor) + reach * to.scale_pct / 100.0)
        .fold(0.0, f64::max);
    SelectionTransform {
        lon: pivot.lon,
        lat: pivot.lat,
        rotation_deg: single.map_or(0.0, |(to, _)| to.rotation_deg),
        scale_pct: single.map_or(100.0, |(to, _)| to.scale_pct),
        radius_m: radius_m.max(50_000.0),
        count: count as u32,
    }
}

/// Applies the drag as the pointer moves.
#[tauri::command]
pub fn drag_transform(
    state: tauri::State<'_, AppState>,
    lon: f64,
    lat: f64,
) -> Result<ProjectSummary> {
    update_transform(&state, lon, lat)
}

/// Implementation of [`drag_transform`], callable without a Tauri handle.
pub fn update_transform(state: &AppState, lon: f64, lat: f64) -> Result<ProjectSummary> {
    let pointer = LonLat::new(lon, lat)?;
    with_session(state, |session| {
        let Some(gesture) = session.transform.clone() else {
            return Err(AppError::Internal(
                "a transform update arrived with no drag in progress".to_owned(),
            ));
        };

        let commands = commands_for(&gesture, pointer);
        let open = session.require_open()?;
        if !commands.is_empty() {
            let command = Command::Batch {
                label: label_for(gesture.kind, gesture.items.len()),
                commands,
            };
            let (project, history) = (&mut open.project, &mut open.history);
            history.push_coalesced(project, command, gesture.key.clone())?;
            open.touch();
        }
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

fn label_for(kind: TransformKind, count: usize) -> String {
    let what = match kind {
        TransformKind::Move => "Move",
        TransformKind::Rotate => "Rotate",
        TransformKind::Scale => "Scale",
        TransformKind::Anchor => "Move anchor",
    };
    if count > 1 {
        format!("{what} {count} objects")
    } else {
        what.to_owned()
    }
}

/// The baseline for a whole selection.
struct Baseline {
    pivot: LonLat,
    items: Vec<TransformBaseline>,
    radius_m: f64,
}

impl Baseline {
    fn handles(&self) -> SelectionTransform {
        // A group has no orientation or scale of its own: its handles start
        // level, and a drag adds to each member's own values.
        let single = if self.items.len() == 1 {
            self.items.first()
        } else {
            None
        };
        SelectionTransform {
            lon: self.pivot.lon,
            lat: self.pivot.lat,
            rotation_deg: single.map_or(0.0, |item| item.rotation_deg),
            scale_pct: single.map_or(100.0, |item| item.scale_pct),
            // A degenerate selection would stack its handles on one point.
            radius_m: self.radius_m.max(50_000.0),
            count: self.items.len() as u32,
        }
    }
}

/// Captures every selected object's transform, and the pivot they share.
fn baseline_of(
    project: &ve_core::project::Project,
    objects: &[u64],
    step: u32,
) -> Option<Baseline> {
    let links = ve_core::follow::resolve(project, step);
    let mut items = Vec::new();
    for raw in objects {
        let Some(object) = project.object(object_id(*raw)) else {
            continue;
        };
        // No flat object means it is outside its lifetime or covers nothing,
        // and there is nothing to put a handle on. A follower's handles go
        // where its link puts it (spec.md 9.3).
        let Some(flat) = flatten_object_at(object, step, links.of(object.id)) else {
            continue;
        };
        // The stored properties, keys and all, so a drag can write *into* an
        // animation rather than over it. Absent from the map only if the
        // object predates the property; the schema default stands in.
        let stored = |id: PropId| {
            object.props.get(id).cloned().unwrap_or_else(|| {
                ve_core::keyframe::Animatable::constant(
                    ve_core::schema::spec_for(object.tool, id)
                        .map(|spec| spec.default.value())
                        .unwrap_or(PropValue::F32(0.0)),
                )
            })
        };
        items.push(TransformBaseline {
            object: object.id,
            outline: BaselineOutline::of(&flat.shape),
            anchor: flat.frame.anchor,
            rotation_deg: flat.frame.rotation_deg,
            scale_pct: flat.frame.scale * 100.0,
            space: flat.frame.space,
            reach_m: flat.cap_radius_m,
            geometry: object.geometry.clone(),
            position: stored(PropId::Position),
            rotation: stored(PropId::RotationDeg),
            scale: stored(PropId::ScalePct),
        });
    }
    if items.is_empty() {
        return None;
    }

    let pivot = centroid(items.iter().map(|item| item.anchor));
    // The radius has to cover every member's own reach, not just how far apart
    // their anchors are, or the handles land inside the selection.
    let radius_m = items
        .iter()
        .map(|item| pivot.distance_m(item.anchor) + item.reach_m)
        .fold(0.0, f64::max);
    Some(Baseline {
        pivot,
        items,
        radius_m,
    })
}

/// The centroid of several positions on the sphere.
///
/// Averaged as 3-D unit vectors, not as longitudes and latitudes: the mean of
/// 179° and -179° is 0° in degree space and 180° on the globe, and the second
/// one is where the objects actually are.
fn centroid(positions: impl Iterator<Item = LonLat>) -> LonLat {
    let mut sum = [0.0f64; 3];
    let mut count = 0.0f64;
    let mut last = LonLat { lon: 0.0, lat: 0.0 };
    for position in positions {
        let v = to_vector(position);
        sum[0] += v[0];
        sum[1] += v[1];
        sum[2] += v[2];
        count += 1.0;
        last = position;
    }
    if count == 0.0 {
        return last;
    }
    let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
    if length < 1e-12 {
        // Antipodal or evenly spread: no meaningful centroid, so pivot about a
        // member rather than about an arbitrary point.
        return last;
    }
    from_vector([sum[0] / length, sum[1] / length, sum[2] / length])
}

fn to_vector(p: LonLat) -> [f64; 3] {
    let (lat, lon) = (p.lat.to_radians(), p.lon.to_radians());
    let (clat, slat) = (lat.cos(), lat.sin());
    [clat * lon.cos(), clat * lon.sin(), slat]
}

fn from_vector(v: [f64; 3]) -> LonLat {
    let lat = v[2].clamp(-1.0, 1.0).asin().to_degrees();
    let lon = v[1].atan2(v[0]).to_degrees();
    LonLat::new(lon, lat).unwrap_or(LonLat { lon: 0.0, lat })
}

/// Bearing from `from` to `to`, or `fallback` when they coincide.
fn bearing_or(from: LonLat, to: LonLat, fallback: f64) -> f64 {
    if from.distance_m(to) < 1.0 {
        fallback
    } else {
        from.initial_bearing(to).degrees()
    }
}

/// What the pointer is doing to the selection as a whole.
///
/// Derived once per update: every member is placed by the same motion, which is
/// what keeps a group rigid rather than letting each member answer the pointer
/// on its own.
enum Motion {
    /// A rigid rotation of the sphere carrying the pivot to the pointer.
    ///
    /// Moving a selection is a rotation rather than an offset because a
    /// rotation preserves the distances and bearings between its members
    /// exactly. Shifting each anchor by a "longitude and latitude offset" does
    /// not: near a pole the same degree offset is a different distance for
    /// every member, and the selection shears.
    Rigid(SphereRotation),
    /// A turn about the pivot, in degrees.
    Turn(f64),
    /// A scale about the pivot, as a multiplier.
    Resize(f64),
    /// The anchor moves to the pointer; the geometry stays on the ground.
    Repin,
}

/// Where one object ends up under a motion.
#[derive(Debug, Clone, Copy)]
struct Placement {
    anchor: LonLat,
    rotation_deg: f64,
    scale_pct: f64,
}

fn motion_of(gesture: &TransformGesture, pointer: LonLat) -> Motion {
    match gesture.kind {
        TransformKind::Move => Motion::Rigid(SphereRotation::carrying(gesture.pivot, pointer)),
        TransformKind::Rotate => Motion::Turn(
            bearing_or(gesture.pivot, pointer, gesture.pointer_bearing) - gesture.pointer_bearing,
        ),
        TransformKind::Scale => {
            // Measured against where the pointer went down, so grabbing the
            // handle does not itself resize anything.
            let reference = gesture.pointer_distance_m.max(1.0);
            Motion::Resize(
                (gesture.pivot.distance_m(pointer) / reference).max(MIN_SCALE_PCT / 10_000.0),
            )
        }
        TransformKind::Anchor => Motion::Repin,
    }
}

/// Where the motion puts one object.
///
/// **The one answer both the writes and the drag preview are built from.** The
/// outline that follows the pointer is the object's own destination rather than
/// a second calculation of it, so what is previewed cannot land anywhere but
/// where the release puts it.
fn placement_of(
    gesture: &TransformGesture,
    motion: &Motion,
    item: &TransformBaseline,
    pointer: LonLat,
) -> Placement {
    let unmoved = Placement {
        anchor: item.anchor,
        rotation_deg: item.rotation_deg,
        scale_pct: item.scale_pct,
    };
    match motion {
        Motion::Rigid(rotation) => Placement {
            anchor: rotation.apply(item.anchor),
            ..unmoved
        },
        Motion::Turn(theta) => {
            let distance = gesture.pivot.distance_m(item.anchor);
            let bearing = bearing_or(gesture.pivot, item.anchor, 0.0);
            Placement {
                anchor: gesture
                    .pivot
                    .destination(Angle::new(bearing + theta), distance),
                // The member turns with the group as well as travelling around
                // it: a rigid rotation, not an orbit.
                rotation_deg: item.rotation_deg + theta,
                ..unmoved
            }
        }
        Motion::Resize(factor) => {
            let distance = gesture.pivot.distance_m(item.anchor) * factor;
            let bearing = bearing_or(gesture.pivot, item.anchor, 0.0);
            Placement {
                anchor: gesture.pivot.destination(Angle::new(bearing), distance),
                scale_pct: (item.scale_pct * factor).max(MIN_SCALE_PCT),
                ..unmoved
            }
        }
        // The geometry is re-expressed about the new anchor, so the object does
        // not move: only the pivot everything else turns about does.
        Motion::Repin => Placement {
            anchor: pointer,
            ..unmoved
        },
    }
}

/// The writes this drag produces, all computed from the baseline.
fn commands_for(gesture: &TransformGesture, pointer: LonLat) -> Vec<Command> {
    let motion = motion_of(gesture, pointer);
    if matches!(motion, Motion::Repin) {
        return anchor_commands(gesture, pointer);
    }
    gesture
        .items
        .iter()
        .flat_map(|item| {
            let to = placement_of(gesture, &motion, item, pointer);
            // The same commands every update, whatever the values happen to be.
            // Batches coalesce element-wise, so a batch that changed length
            // partway through a drag would split it into several history
            // entries -- a rotation that starts at exactly zero degrees is the
            // case that would find it.
            match gesture.kind {
                TransformKind::Move => vec![set_position(gesture, item, to.anchor)],
                TransformKind::Rotate => vec![
                    set_position(gesture, item, to.anchor),
                    set_rotation(gesture, item, to.rotation_deg),
                ],
                TransformKind::Scale => vec![
                    set_position(gesture, item, to.anchor),
                    set_scale(gesture, item, to.scale_pct),
                ],
                // Handled above: it rewrites geometry as well.
                TransformKind::Anchor => Vec::new(),
            }
        })
        .collect()
}

/// The property as the drag leaves it: the stored animatable with `value`
/// written into it — as a key at the gesture's step when the property is
/// animated or auto-key is on, as the base otherwise (spec.md 9.3).
///
/// A key already at that step keeps its easing; a new one takes the kind's
/// default. `before` is the stored animatable itself, so undo restores keys
/// and base exactly.
/// The drag's value, written by the one rule (spec.md 9.3): into the current
/// step's key when the property is animated or auto-key is on, else the base.
fn written(gesture: &TransformGesture, before: &Animatable, value: PropValue) -> Animatable {
    crate::animation::written(before, gesture.step, gesture.auto_key, value)
}

fn set_property(
    gesture: &TransformGesture,
    item: &TransformBaseline,
    prop: PropId,
    before: &Animatable,
    value: PropValue,
) -> Command {
    Command::SetProperty {
        object: item.object,
        prop,
        before: Box::new(before.clone()),
        after: Box::new(written(gesture, before, value)),
    }
}

fn set_position(gesture: &TransformGesture, item: &TransformBaseline, to: LonLat) -> Command {
    set_property(
        gesture,
        item,
        PropId::Position,
        &item.position,
        PropValue::LonLat(to),
    )
}

fn set_rotation(gesture: &TransformGesture, item: &TransformBaseline, degrees: f64) -> Command {
    set_property(
        gesture,
        item,
        PropId::RotationDeg,
        &item.rotation,
        PropValue::Angle(Angle::new(degrees)),
    )
}

fn set_scale(gesture: &TransformGesture, item: &TransformBaseline, percent: f64) -> Command {
    set_property(
        gesture,
        item,
        PropId::ScalePct,
        &item.scale,
        PropValue::F32(percent as f32),
    )
}

/// Moves the anchor and leaves the geometry where it is on the ground.
///
/// The anchor is the pivot everything else turns and scales about, so being
/// able to put it somewhere other than where the first brush stamp landed is
/// what makes rotation useful (spec.md 8.2). Re-expressing the geometry in the
/// new frame is the whole operation: the points are stored relative to the
/// anchor, so without it the shape would jump by however far the anchor moved.
fn anchor_commands(gesture: &TransformGesture, pointer: LonLat) -> Vec<Command> {
    let mut commands = Vec::new();
    for item in &gesture.items {
        let before = Frame::in_space(item.anchor, item.rotation_deg, item.scale_pct, item.space);
        let after = Frame::in_space(pointer, item.rotation_deg, item.scale_pct, item.space);
        let reframe = |p: LocalPoint| {
            let global = before.to_global([p.x, p.y]);
            let local = after.to_local(global);
            LocalPoint::new(local[0], local[1])
        };

        let geometry = match &item.geometry {
            Geometry::Stroke { chains } => Some(Geometry::Stroke {
                chains: chains
                    .iter()
                    .map(|chain| chain.iter().copied().map(reframe).collect())
                    .collect(),
            }),
            Geometry::Polygon { points } => Some(Geometry::Polygon {
                points: points.iter().copied().map(reframe).collect(),
            }),
            // The stamps move with the frame; the deltas are directions in it
            // and are left alone, as a push's local displacement is.
            Geometry::Smear { chains } => Some(Geometry::Smear {
                chains: chains
                    .iter()
                    .map(|chain| {
                        chain
                            .iter()
                            .map(|s| {
                                let moved = reframe(LocalPoint::new(s.x, s.y));
                                ve_core::document::SmearPoint::new(moved.x, moved.y, s.dx, s.dy)
                            })
                            .collect()
                    })
                    .collect(),
            }),
            Geometry::Path { nodes } => Some(Geometry::Path {
                nodes: nodes
                    .iter()
                    .map(|node| ve_core::document::PathNode {
                        point: reframe(node.point),
                        in_handle: node.in_handle.map(reframe),
                        out_handle: node.out_handle.map(reframe),
                    })
                    .collect(),
            }),
            // A disc or a rectangle *is* centred on its anchor. There is no
            // geometry to leave behind, so moving the anchor moves the object,
            // which is the only answer that keeps the two consistent.
            Geometry::Disc { .. } | Geometry::Rect { .. } => None,
        };

        commands.push(set_position(gesture, item, pointer));
        if let Some(geometry) = geometry {
            commands.push(Command::SetGeometry {
                object: item.object,
                before: Box::new(item.geometry.clone()),
                after: Box::new(geometry),
            });
        }
    }
    commands
}

/// The rotation of the sphere that carries one point to another.
struct SphereRotation {
    axis: [f64; 3],
    cos: f64,
    sin: f64,
}

impl SphereRotation {
    fn carrying(from: LonLat, to: LonLat) -> Self {
        let a = to_vector(from);
        let b = to_vector(to);
        let axis = cross(a, b);
        let length = norm(axis);
        let cos = dot(a, b).clamp(-1.0, 1.0);
        if length < 1e-12 {
            // Coincident (or antipodal, which no drag can reach in one step):
            // the identity is the only sane answer.
            return Self {
                axis: [0.0, 0.0, 1.0],
                cos: 1.0,
                sin: 0.0,
            };
        }
        Self {
            axis: [axis[0] / length, axis[1] / length, axis[2] / length],
            cos,
            sin: length,
        }
    }

    fn apply(&self, position: LonLat) -> LonLat {
        // Rodrigues' rotation formula.
        let v = to_vector(position);
        let k = self.axis;
        let kv = cross(k, v);
        let kdv = dot(k, v);
        from_vector([
            v[0] * self.cos + kv[0] * self.sin + k[0] * kdv * (1.0 - self.cos),
            v[1] * self.cos + kv[1] * self.sin + k[1] * kdv * (1.0 - self.cos),
            v[2] * self.cos + kv[2] * self.sin + k[2] * kdv * (1.0 - self.cos),
        ])
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}

// --- Marquee ----------------------------------------------------------------

/// Objects whose footprint meets a lat/lon rectangle.
///
/// `layer` scopes the search: selection is layer-scoped by default, and a
/// modifier passes `None` to reach across layers (spec.md 8.2).
#[tauri::command]
pub fn objects_in_region(
    state: tauri::State<'_, AppState>,
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    step: u32,
    layer: Option<u64>,
) -> Result<Vec<u64>> {
    region_objects(&state, west, south, east, north, step, layer)
}

/// Implementation of [`objects_in_region`], callable without a Tauri handle.
#[allow(clippy::too_many_arguments, reason = "a rectangle is four numbers")]
pub fn region_objects(
    state: &AppState,
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    step: u32,
    layer: Option<u64>,
) -> Result<Vec<u64>> {
    with_session(state, |session| {
        let project = &session.require_open()?.project;
        let wanted = layer.map(object_id);

        let links = ve_core::follow::resolve(project, step);
        let mut found = Vec::new();
        for candidate in &project.layers {
            if !candidate.visible || candidate.locked {
                continue;
            }
            if wanted.is_some_and(|id| id != candidate.id) {
                continue;
            }
            for object in &candidate.objects {
                let Some(flat) = flatten_object_at(object, step, links.of(object.id)) else {
                    continue;
                };
                // Distance from the anchor to the nearest point of the
                // rectangle, against the object's own reach: a cap-versus-box
                // test. It errs towards selecting, which is the right way to be
                // wrong — a marquee that misses what it visibly enclosed is far
                // more annoying than one that catches a near miss.
                let nearest = nearest_in_rect(flat.frame.anchor, west, south, east, north);
                if flat.frame.anchor.distance_m(nearest) <= flat.cap_radius_m {
                    found.push(object.id.raw());
                }
            }
        }
        Ok(found)
    })
}

/// The point of a lat/lon rectangle closest to `p`.
fn nearest_in_rect(p: LonLat, west: f64, south: f64, east: f64, north: f64) -> LonLat {
    let lat = p.lat.clamp(south.min(north), south.max(north));
    // Longitude is clamped on the circle, not the number line: a rectangle
    // spanning the antimeridian has west > east, and its inside is the part
    // that wraps.
    let lon = if contains_lon(west, east, p.lon) {
        p.lon
    } else {
        let to_west = angular_gap(p.lon, west);
        let to_east = angular_gap(p.lon, east);
        if to_west <= to_east { west } else { east }
    };
    LonLat::new(lon, lat).unwrap_or(p)
}

/// Whether `lon` lies in the arc running east from `west` to `east`.
fn contains_lon(west: f64, east: f64, lon: f64) -> bool {
    let span = (east - west).rem_euclid(360.0);
    let offset = (lon - west).rem_euclid(360.0);
    offset <= span
}

/// The smaller angle between two longitudes, in degrees.
fn angular_gap(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(lon: f64, lat: f64) -> LonLat {
        LonLat::new(lon, lat).expect("valid")
    }

    /// The mean of 179° and -179° is 180°, not 0°: the objects are next to each
    /// other across the dateline, not on the far side of the planet.
    #[test]
    fn a_centroid_crosses_the_antimeridian() {
        let c = centroid([at(179.0, 10.0), at(-179.0, 10.0)].into_iter());
        assert!(c.lon.abs() > 179.0, "centroid landed at {}", c.lon);
        assert!((c.lat - 10.0).abs() < 0.1);
    }

    #[test]
    fn a_centroid_of_one_is_itself() {
        let c = centroid([at(-40.0, 62.0)].into_iter());
        assert!((c.lon + 40.0).abs() < 1e-9 && (c.lat - 62.0).abs() < 1e-9);
    }

    /// A rigid rotation is what keeps a group's shape: the distance between two
    /// members must survive being dragged across the pole.
    #[test]
    fn moving_a_group_preserves_the_distances_within_it() {
        let (a, b) = (at(0.0, 0.0), at(10.0, 5.0));
        let before = a.distance_m(b);

        let rotation = SphereRotation::carrying(at(5.0, 2.5), at(-120.0, 78.0));
        let after = rotation.apply(a).distance_m(rotation.apply(b));

        assert!(
            (after - before).abs() < 1.0,
            "{before} m became {after} m across the move"
        );
    }

    #[test]
    fn a_move_puts_the_pivot_exactly_on_the_pointer() {
        let pivot = at(-30.0, 12.0);
        let target = at(140.0, -55.0);
        let moved = SphereRotation::carrying(pivot, target).apply(pivot);
        assert!(moved.distance_m(target) < 1.0, "landed at {moved:?}");
    }

    #[test]
    fn a_zero_length_move_changes_nothing() {
        let p = at(23.0, -4.0);
        let moved = SphereRotation::carrying(p, p).apply(at(80.0, 40.0));
        assert!(moved.distance_m(at(80.0, 40.0)) < 1.0);
    }

    #[test]
    fn a_rectangle_contains_its_own_points() {
        let inside = at(10.0, 20.0);
        let nearest = nearest_in_rect(inside, 0.0, 10.0, 20.0, 30.0);
        assert!(
            nearest.distance_m(inside) < 1.0,
            "an inside point is its own nearest"
        );
    }

    #[test]
    fn a_rectangle_clamps_to_its_nearest_edge() {
        let nearest = nearest_in_rect(at(30.0, 20.0), 0.0, 10.0, 20.0, 30.0);
        assert!(
            (nearest.lon - 20.0).abs() < 1e-9,
            "clamped to {}",
            nearest.lon
        );
        assert!((nearest.lat - 20.0).abs() < 1e-9);
    }

    /// A marquee dragged across the dateline has west > east, and everything
    /// between them the long way round is *outside* it.
    #[test]
    fn a_rectangle_may_wrap_the_antimeridian() {
        assert!(contains_lon(170.0, -170.0, 179.0));
        assert!(contains_lon(170.0, -170.0, -175.0));
        assert!(!contains_lon(170.0, -170.0, 0.0));

        let nearest = nearest_in_rect(at(179.0, 5.0), 170.0, 0.0, -170.0, 10.0);
        assert!(
            (nearest.lon - 179.0).abs() < 1e-9,
            "a point inside a wrapped rectangle is its own nearest, got {}",
            nearest.lon
        );
    }
}
