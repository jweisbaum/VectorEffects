//! Document editing commands.
//!
//! Every mutation goes through the undo stack (`ve-core::history`), so undo is
//! available from the moment the first tool exists rather than being retrofitted
//! once several tools have their own bespoke paths.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_core::angle::Angle;
use ve_core::command::Command;
use ve_core::document::{Geometry, Layer, LocalPoint, Object};
use ve_core::schema::{PropId, ToolKind};
use ve_core::{LonLat, PropValue};
use ve_render::aeqd::{Frame, Local, Space};
use ve_render::scene::flatten_object;
use ve_render::sdf::chains_distance;

use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::{ProjectSummary, with_session};

/// Which stamp the brush sweeps along the stroke (spec.md 6.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "BrushShape.ts")]
pub enum BrushShape {
    /// A round stamp: the footprint is a swept disc.
    #[default]
    Circle,
    /// A square stamp, axis-aligned in the object's own frame — so rotating
    /// the object turns the stamp with it.
    Square,
}

impl BrushShape {
    /// Index into [`ve_core::schema::BRUSH_SHAPES`].
    fn variant(self) -> u8 {
        match self {
            Self::Circle => 0,
            Self::Square => 1,
        }
    }
}

/// How the brush decides each cell's direction (spec.md 6.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "BrushDirectionMode.ts")]
pub enum BrushDirectionMode {
    /// One bearing across the whole stroke.
    #[default]
    Constant,
    /// Every cell takes the initial great-circle bearing to a fixed point, so
    /// the stroke converges on it.
    TowardPoint,
    /// The reciprocal: every cell points directly away from a fixed point, so
    /// the stroke radiates out of it.
    AwayFromPoint,
}

impl BrushDirectionMode {
    /// Index into [`ve_core::schema::DIRECTION_MODES`].
    fn variant(self) -> u8 {
        match self {
            Self::Constant => 0,
            Self::TowardPoint => 1,
            Self::AwayFromPoint => 2,
        }
    }

    /// Whether the mode needs a target to mean anything.
    fn needs_target(self) -> bool {
        matches!(self, Self::TowardPoint | Self::AwayFromPoint)
    }
}

/// Which space a stamp's footprint is a circle in (spec.md 3.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "StampSpace.ts")]
pub enum StampSpace {
    /// A shape on the ground: a disc of `size_km` at any latitude, which the
    /// map draws as an ellipse widening away from the equator.
    #[default]
    Geodesic,
    /// A shape on the map: a circle on screen at any latitude and any zoom,
    /// which covers `cos(lat)` as much ground east-west as it does north-south.
    /// What a size entered in pixels asks for.
    Projected,
}

impl StampSpace {
    /// Index into [`ve_core::schema::STAMP_SPACES`].
    fn variant(self) -> u8 {
        match self {
            Self::Geodesic => 0,
            Self::Projected => 1,
        }
    }

    /// The render frame's matching space.
    fn frame_space(self) -> Space {
        match self {
            Self::Geodesic => Space::Geodesic,
            Self::Projected => Space::Projected,
        }
    }
}

/// A brush stroke, as painted on the map.
///
/// [`Default`] is derived so a caller — a test, or an example — can name only
/// the options it is about, rather than restating every one of them.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[ts(export, export_to = "BrushStroke.ts")]
pub struct BrushStroke {
    /// Path in geographic coordinates, as `[lon, lat]` pairs.
    pub points: Vec<[f64; 2]>,
    /// Brush size in kilometres: the disc's diameter, or the square's side.
    ///
    /// Kilometres, never pixels. A size entered in pixels is resolved against
    /// the map scale before it gets here, and never revisited, so zooming
    /// afterwards cannot resize an existing stroke (spec.md 3.5).
    pub size_km: f32,
    /// Speed in metres per second.
    pub speed_mps: f32,
    /// Direction the flow points toward, degrees clockwise from north.
    ///
    /// Already converted from the project's display convention by the caller;
    /// nothing below the IPC boundary sees a "from" bearing (spec.md 3.3).
    /// Ignored by the modes that aim at a target.
    pub direction_toward_deg: f64,
    /// Edge falloff, 0 to 1.
    pub feather: f32,
    /// Circular or square stamp.
    #[serde(default)]
    pub shape: BrushShape,
    /// Whether the stamp is a shape on the ground or a shape on the map.
    ///
    /// `size_km` means the same number either way: it is the footprint's
    /// north-south ground extent, which is the one axis the projection leaves
    /// alone (spec.md 3.5).
    #[serde(default)]
    pub space: StampSpace,
    /// A constant bearing, or every vector aimed at or away from `target`.
    #[serde(default)]
    pub direction_mode: BrushDirectionMode,
    /// Where the vectors aim, as `[lon, lat]`. Required by the aimed modes.
    #[serde(default)]
    #[ts(optional)]
    pub target: Option<[f64; 2]>,
    /// Which layer receives it. `None` means the top layer.
    #[ts(optional)]
    pub layer: Option<u64>,
}

/// Adds a painted stroke to the topmost layer.
#[tauri::command]
pub fn add_brush_stroke(
    state: tauri::State<'_, AppState>,
    stroke: BrushStroke,
) -> Result<ProjectSummary> {
    paint(&state, stroke)
}

/// How far a merged stroke may reach from its anchor, in metres.
///
/// Geometry lives in an azimuthal-equidistant frame, which is well conditioned
/// near its anchor and degenerates at the antipode. A quarter of the earth's
/// circumference is a wide margin from that, and a stroke wanting to reach
/// further is better off as its own object with its own anchor.
const MERGE_MAX_RADIUS_M: f64 = 10_007_543.0;

/// Implementation of [`add_brush_stroke`], callable without a Tauri handle.
pub fn paint(state: &AppState, stroke: BrushStroke) -> Result<ProjectSummary> {
    let first = *stroke
        .points
        .first()
        .ok_or_else(|| AppError::Internal("a stroke needs at least one point".to_owned()))?;
    let anchor = LonLat::new(first[0], first[1])?;

    // A stroke aimed at nothing has no direction to paint, and falling back to
    // the constant bearing would quietly paint a stroke nobody asked for. So
    // the mode and its point are validated together, before anything is
    // written.
    let target = if stroke.direction_mode.needs_target() {
        let point = stroke.target.ok_or(AppError::BadOption {
            field: "target",
            value: "absent, but the direction mode aims at a point".to_owned(),
        })?;
        Some(LonLat::new(point[0], point[1])?)
    } else {
        None
    };

    // Geographic points are kept alongside the object: a merge re-expresses the
    // stroke in the *target's* frame, which is not known until a target is
    // found.
    let mut positions = Vec::with_capacity(stroke.points.len());
    for point in &stroke.points {
        positions.push(LonLat::new(point[0], point[1])?);
    }

    with_session(state, |session| {
        let open = session.require_open()?;
        let step_count = open.project.settings.step_count;

        // Geometry is stored in the object's own frame, in metres, so the
        // stroke stays pinned to the earth under pan and zoom (spec.md 7.2).
        // Which frame depends on what the stamp is a circle in: a projected
        // stroke's points are map-space metres, and converting them with a
        // ground frame would bend the stroke away from where it was drawn.
        let frame = Frame::in_space(anchor, 0.0, 100.0, stroke.space.frame_space());
        let chain: Vec<LocalPoint> = positions
            .iter()
            .map(|position| {
                let local = frame.to_local(*position);
                LocalPoint::new(local[0], local[1])
            })
            .collect();

        let index = open.project.layers.len();
        let mut object = Object::new(ToolKind::Brush, format!("Stroke {index}"), step_count);
        object.geometry = Geometry::Stroke {
            chains: vec![chain],
        };

        let mut set = |id: PropId, value: PropValue| {
            if let Some(prop) = object.props.get_mut(id) {
                prop.set_base(value);
            }
        };
        set(PropId::Position, PropValue::LonLat(anchor));
        set(PropId::SizeKm, PropValue::F32(stroke.size_km.max(1.0)));
        set(PropId::Speed, PropValue::F32(stroke.speed_mps.max(0.0)));
        set(
            PropId::Direction,
            PropValue::Angle(Angle::new(stroke.direction_toward_deg)),
        );
        set(
            PropId::Feather,
            PropValue::F32(stroke.feather.clamp(0.0, 1.0)),
        );
        set(PropId::BrushShape, PropValue::Enum(stroke.shape.variant()));
        set(PropId::StampSpace, PropValue::Enum(stroke.space.variant()));
        set(
            PropId::DirectionMode,
            PropValue::Enum(stroke.direction_mode.variant()),
        );
        // Left at its default in constant mode, so two strokes painted with the
        // same options still compare equal and can merge.
        if let Some(target) = target {
            set(PropId::Target, PropValue::LonLat(target));
        }

        // A new object joins the layer that was selected when it was created
        // (spec.md 6.1); with no selection that is the top of the stack.
        let layer = match stroke.layer {
            Some(raw) => open
                .project
                .layers
                .iter()
                .find(|layer| layer.id.raw() == raw)
                .filter(|layer| !layer.locked)
                .ok_or(AppError::Core(ve_core::CoreError::MissingLayer(raw)))?,
            None => open
                .project
                .layers
                .last()
                .ok_or_else(|| AppError::Internal("project has no layers".to_owned()))?,
        };

        // A stroke that lands on an identical one becomes another chain of it
        // rather than a second object, so repeatedly going over an area leaves
        // one thing to select, move and animate (spec.md 6.1).
        let command = match merge_target(layer, &object, &positions) {
            Some((target, merged)) => Command::SetGeometry {
                object: target,
                before: Box::new(
                    layer
                        .objects
                        .iter()
                        .find(|o| o.id == target)
                        .map(|o| o.geometry.clone())
                        .ok_or_else(|| AppError::Internal("merge target vanished".to_owned()))?,
                ),
                after: Box::new(merged),
            },
            None => Command::AddObject {
                layer: layer.id,
                index: layer.objects.len(),
                object: Box::new(object),
            },
        };

        let open = session.require_open()?;
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();

        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// Finds the object `stroke` should be merged into, with the merged geometry.
///
/// Merging is only safe when it changes nothing about how the layer composites.
/// That needs three things, checked from the top of the stack down:
///
///  * the target renders identically to the new stroke apart from where it sits
///    (every property equal, keyframes included);
///  * their footprints actually overlap, or the two would read as one object
///    while looking like two;
///  * nothing between them in z-order overlaps the new stroke, or absorbing it
///    downwards would move it beneath something it was painted on top of.
fn merge_target(
    layer: &Layer,
    stroke: &Object,
    positions: &[LonLat],
) -> Option<(ve_core::id::Id, Geometry)> {
    let flat_stroke = flatten_object(stroke, 0)?;
    // Both brush shapes sweep the same chains; the reach that decides whether
    // two strokes touch is the stamp's half-width either way. For a square that
    // is measured across the flats rather than the diagonal, so the overlap
    // test below is slightly strict — and strict is the safe direction, since
    // refusing a merge only costs tidiness while a wrong one changes the layer.
    let reach_m = match flat_stroke.shape {
        ve_render::sdf::Shape::Capsule { radius_m, .. } => radius_m,
        ve_render::sdf::Shape::SweptSquare { half_size_m, .. } => half_size_m,
        _ => return None,
    };

    for candidate in layer.objects.iter().rev() {
        if let Some(merged) = merged_geometry(candidate, stroke, positions, reach_m) {
            return Some((candidate.id, merged));
        }
        // Not a merge target: if it overlaps the new stroke it stands between
        // the stroke and anything below, and the search stops.
        if blocks(candidate, &flat_stroke) {
            return None;
        }
    }
    None
}

/// The geometry `candidate` would have with `stroke` merged into it, if the two
/// may be merged at all.
fn merged_geometry(
    candidate: &Object,
    stroke: &Object,
    positions: &[LonLat],
    reach_m: f64,
) -> Option<Geometry> {
    if candidate.tool != stroke.tool || candidate.active_range != stroke.active_range {
        return None;
    }
    let Geometry::Stroke { chains } = &candidate.geometry else {
        return None;
    };

    // Position differs by definition; everything else, keyframes included, must
    // match or the merged object could not render both strokes as they were
    // painted.
    if candidate.props.len() != stroke.props.len() {
        return None;
    }
    for (id, anim) in candidate.props.iter() {
        if *id == PropId::Position {
            // An animated position moves the whole object, so the frame the new
            // stroke would be expressed in is not the frame it was painted in.
            if anim.is_animated() {
                return None;
            }
            continue;
        }
        if stroke.props.get(*id) != Some(anim) {
            return None;
        }
    }

    // Re-express the new stroke in the candidate's frame. Identical properties
    // mean identical scale and rotation, so only the anchor differs.
    let flat = flatten_object(candidate, 0)?;
    let chain: Vec<Local> = positions
        .iter()
        .map(|position| flat.frame.to_local(*position))
        .collect();
    if chain
        .iter()
        .any(|p| p[0].hypot(p[1]) * flat.frame.scale > MERGE_MAX_RADIUS_M)
    {
        return None;
    }

    let existing: Vec<Vec<Local>> = chains
        .iter()
        .map(|chain| chain.iter().map(|p| [p.x, p.y]).collect())
        .collect();
    if chains_distance(&existing, std::slice::from_ref(&chain)) > reach_m * 2.0 {
        return None;
    }

    let mut merged = chains.clone();
    merged.push(chain.iter().map(|p| LocalPoint::new(p[0], p[1])).collect());
    Some(Geometry::Stroke { chains: merged })
}

/// Whether `candidate` stands between the new stroke and anything below it.
///
/// Checked over the candidate's whole lifetime, not just the step being edited:
/// an object that only appears later, or that moves, still covers the stroke
/// when it does, and the stack order has to hold at every step.
fn blocks(candidate: &Object, stroke: &ve_render::scene::FlatObject) -> bool {
    let range = candidate.active_range;
    // A still object has one footprint, so one step answers for every step.
    let last = if candidate.props.iter().any(|(_, anim)| anim.is_animated()) {
        range.end
    } else {
        range.start
    };
    (range.start..=last).any(|step| {
        flatten_object(candidate, step).is_some_and(|flat| footprints_may_overlap(&flat, stroke))
    })
}

/// Whether two footprints can touch, judged by bounding circles.
///
/// Deliberately conservative: an approximation that says "maybe" too often
/// costs a merge, while one that says "no" too often would reorder the stack.
fn footprints_may_overlap(
    a: &ve_render::scene::FlatObject,
    b: &ve_render::scene::FlatObject,
) -> bool {
    let reach = |o: &ve_render::scene::FlatObject| o.shape.bounding_radius_m() * o.frame.scale;
    a.frame.anchor.distance_m(b.frame.anchor) <= reach(a) + reach(b)
}

/// Reverses the most recent change.
#[tauri::command]
pub fn undo(state: tauri::State<'_, AppState>) -> Result<ProjectSummary> {
    step_history(&state, true)
}

/// Reapplies the most recently undone change.
#[tauri::command]
pub fn redo(state: tauri::State<'_, AppState>) -> Result<ProjectSummary> {
    step_history(&state, false)
}

/// Reverses the most recent change. Exposed for tests, which drive the same
/// path the command does.
pub fn undo_for_test(state: &AppState) -> Result<ProjectSummary> {
    step_history(state, true)
}

fn step_history(state: &AppState, backwards: bool) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let (project, history) = (&mut open.project, &mut open.history);
        let moved = if backwards {
            history.undo(project)?
        } else {
            history.redo(project)?
        };
        if moved.is_some() {
            open.touch();
        }
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::schema::{BRUSH_SHAPES, DIRECTION_MODES};
    use ve_core::value::Interpolation;

    /// The wire enums are indices into the schema's variant lists. Written as
    /// literals for legibility, so something has to notice when a list is
    /// reordered.
    #[test]
    fn the_wire_enums_index_the_schema_variants() {
        assert_eq!(
            BRUSH_SHAPES[BrushShape::Circle.variant() as usize],
            "circle"
        );
        assert_eq!(
            BRUSH_SHAPES[BrushShape::Square.variant() as usize],
            "square"
        );
        assert_eq!(
            DIRECTION_MODES[BrushDirectionMode::Constant.variant() as usize],
            "constant"
        );
        assert_eq!(
            DIRECTION_MODES[BrushDirectionMode::TowardPoint.variant() as usize],
            "toward_point"
        );
        assert_eq!(
            DIRECTION_MODES[BrushDirectionMode::AwayFromPoint.variant() as usize],
            "away_from_point"
        );
    }

    fn brush(anchor: LonLat, chain: Vec<[f64; 2]>) -> Object {
        let mut object = Object::new(ToolKind::Brush, "s", 12);
        object.geometry = Geometry::Stroke {
            chains: vec![
                chain
                    .into_iter()
                    .map(|p| LocalPoint::new(p[0], p[1]))
                    .collect(),
            ],
        };
        if let Some(prop) = object.props.get_mut(PropId::Position) {
            prop.set_base(PropValue::LonLat(anchor));
        }
        object
    }

    /// The frame a stroke is expressed in is the frame it was painted in. An
    /// animated position moves that frame, so the merged stroke would travel
    /// somewhere it was never painted.
    #[test]
    fn a_moving_object_does_not_absorb_a_stroke() {
        let anchor = LonLat::new(0.0, 0.0).unwrap();
        let target = brush(anchor, vec![[0.0, 0.0], [200_000.0, 0.0]]);
        let painted = brush(
            LonLat::new(1.0, 0.0).unwrap(),
            vec![[0.0, 0.0], [200_000.0, 0.0]],
        );
        let positions = [
            LonLat::new(1.0, 0.0).unwrap(),
            LonLat::new(2.0, 0.0).unwrap(),
        ];

        let mut layer = Layer::new("L");
        layer.objects.push(target.clone());
        assert!(
            merge_target(&layer, &painted, &positions).is_some(),
            "the two overlap and match, so they would otherwise merge"
        );

        let mut moving = target;
        moving.props.get_mut(PropId::Position).unwrap().set_key(
            6,
            PropValue::LonLat(LonLat::new(40.0, 20.0).unwrap()),
            Interpolation::Linear,
        );
        let mut layer = Layer::new("L");
        layer.objects.push(moving);
        assert!(merge_target(&layer, &painted, &positions).is_none());
    }
}
