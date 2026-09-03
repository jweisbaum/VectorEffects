//! Flattening a project into an evaluable scene.
//!
//! For one time step, every animatable property is resolved to a concrete
//! value and every object becomes a [`FlatObject`] carrying its frame, its
//! shape, and its resolved parameters (spec.md 7.1). Nothing downstream of
//! this sees a keyframe, a layer, or a property id.

use ve_core::angle::Angle;
use ve_core::document::{Geometry, Object, PathNode};
use ve_core::project::Project;
use ve_core::schema::{PropId, ToolKind};
use ve_core::{LonLat, PropValue};

use crate::aeqd::{Frame, Local, Space};
use crate::sdf::Shape;

/// How an object's speed varies across its footprint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpeedMode {
    /// One speed everywhere.
    Constant(f64),
    /// Ramps from the anchor outward: the circle tool's gradient fill.
    Radial {
        /// Speed at the anchor.
        centre: f64,
        /// Speed at the outer edge.
        edge: f64,
        /// Radius the ramp spans, in geometry units.
        extent: f64,
    },
    /// Ramps along a bearing across the footprint: the shape-fill gradient.
    Axis {
        /// Speed at the start of the ramp.
        start: f64,
        /// Speed at the end.
        end: f64,
    },
}

/// How an object's direction is decided at a point.
#[derive(Debug, Clone, PartialEq)]
pub enum DirectionMode {
    /// One bearing everywhere.
    Constant(Angle),
    /// Every vector aims at a fixed position.
    Toward(LonLat),
    /// Every vector aims directly away from a fixed position, so the field
    /// radiates outward from it.
    ///
    /// The opposite tangent to the same great circle, not the bearing measured
    /// at the point itself: at a cell, "away" is exactly the reciprocal of
    /// "toward", and the two therefore cancel to calm if stacked.
    Away(LonLat),
    /// Ramps between two bearings along the gradient axis.
    Axis {
        /// Bearing at the start of the ramp.
        start: Angle,
        /// Bearing at the end.
        end: Angle,
    },
    /// Offset from the local tangent of the object's path.
    AlongPath {
        /// Added to the path's bearing.
        offset: Angle,
    },
    /// Flow around the anchor: the circle tool's rotation.
    ///
    /// A separate mode rather than sugar over `curl`, because curl *adds* to
    /// the base direction. With the circle having no direction property of its
    /// own, that base defaulted to north and a "rotating" circle came out
    /// drifting northward. Rotation is the primary flow here, and divergence
    /// and curl remain available on top to make it spiral.
    Tangential {
        /// True for clockwise, false for counter-clockwise.
        clockwise: bool,
    },
}

/// Whether a feathered edge blends with what is beneath it (decision D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeMode {
    /// Fade into the underlying field.
    Blend,
    /// Overwrite, fading the object's own speed toward zero.
    Replace,
}

/// One object with every property resolved for a single time step.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatObject {
    /// The object's local frame.
    pub frame: Frame,
    /// Its footprint, in local geometry units.
    pub shape: Shape,
    /// Ground radius beyond which the object cannot reach, for culling.
    pub cap_radius_m: f64,
    /// Speed behaviour.
    pub speed: SpeedMode,
    /// Direction behaviour.
    pub direction: DirectionMode,
    /// Edge falloff, 0 to 1.
    pub feather: f64,
    /// Radial component, -1 to 1.
    pub divergence: f64,
    /// Tangential component, -1 to 1.
    pub curl: f64,
    /// How the edge combines with what is beneath.
    pub edge_mode: EdgeMode,
    /// Bearing the gradient ramps along, for the axis modes.
    pub gradient_axis: Angle,
    /// Half-extent of the gradient ramp, in geometry units.
    pub gradient_extent: f64,
    /// Path points in local units, for [`DirectionMode::AlongPath`].
    pub path: Vec<Local>,
    /// Where a clone stamp samples from.
    ///
    /// `Some` only for the clone stamp, which is the one operator that reads
    /// the composite beneath it rather than producing a field of its own
    /// (spec.md 7.6).
    pub clone_source: Option<LonLat>,
}

/// A whole time step, ready to evaluate. Objects are in z-order, bottom first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    /// Objects, bottom of the stack first.
    pub objects: Vec<FlatObject>,
}

/// Maximum error when turning a Bézier segment into a polyline, in metres.
///
/// Well below the finest grid cell (about 11 km), so flattening is never what
/// limits accuracy.
const CURVE_TOLERANCE_M: f64 = 250.0;

fn number(object: &Object, id: PropId, step: u32) -> Option<f64> {
    object
        .props
        .value_at(object.tool, id, step)
        .and_then(|v| v.as_f32())
        .map(f64::from)
}

fn bearing(object: &Object, id: PropId, step: u32) -> Option<Angle> {
    object
        .props
        .value_at(object.tool, id, step)
        .and_then(PropValue::as_angle)
}

fn position(object: &Object, id: PropId, step: u32) -> Option<LonLat> {
    object
        .props
        .value_at(object.tool, id, step)
        .and_then(PropValue::as_lonlat)
}

fn choice(object: &Object, id: PropId, step: u32) -> u8 {
    object
        .props
        .value_at(object.tool, id, step)
        .and_then(PropValue::as_enum)
        .unwrap_or(0)
}

/// Subdivides a cubic Bézier until it is flat to within the tolerance.
fn flatten_cubic(from: Local, c1: Local, c2: Local, to: Local, out: &mut Vec<Local>) {
    // Distance of the control points from the chord: a standard flatness test.
    let chord = [to[0] - from[0], to[1] - from[1]];
    let length = chord[0].hypot(chord[1]);
    let deviation = if length < 1e-9 {
        (c1[0] - from[0]).hypot(c1[1] - from[1]) + (c2[0] - to[0]).hypot(c2[1] - to[1])
    } else {
        let cross =
            |p: Local| ((p[0] - from[0]) * chord[1] - (p[1] - from[1]) * chord[0]).abs() / length;
        cross(c1).max(cross(c2))
    };

    // Cap the depth so a pathological control net cannot recurse forever.
    if deviation <= CURVE_TOLERANCE_M || out.len() > 4096 {
        out.push(to);
        return;
    }

    let mid = |a: Local, b: Local| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let (ab, bc, cd) = (mid(from, c1), mid(c1, c2), mid(c2, to));
    let (abc, bcd) = (mid(ab, bc), mid(bc, cd));
    let middle = mid(abc, bcd);

    flatten_cubic(from, ab, abc, middle, out);
    flatten_cubic(middle, bcd, cd, to, out);
}

/// Turns path nodes into a polyline, expanding any Bézier segments.
pub fn flatten_path(nodes: &[PathNode]) -> Vec<Local> {
    let mut out: Vec<Local> = Vec::new();
    let Some(first) = nodes.first() else {
        return out;
    };
    out.push([first.point.x, first.point.y]);

    for pair in nodes.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let from = [a.point.x, a.point.y];
        let to = [b.point.x, b.point.y];
        match (a.out_handle, b.in_handle) {
            (None, None) => out.push(to),
            (out_handle, in_handle) => {
                let c1 = out_handle.map_or(from, |h| [h.x, h.y]);
                let c2 = in_handle.map_or(to, |h| [h.x, h.y]);
                flatten_cubic(from, c1, c2, to, &mut out);
            }
        }
    }
    out
}

/// Builds the object's footprint from its geometry and size properties.
fn shape_of(object: &Object, step: u32) -> Option<(Shape, Vec<Local>)> {
    let km = |id: PropId| number(object, id, step).unwrap_or(0.0) * 1000.0;

    match &object.geometry {
        Geometry::Stroke { chains } => {
            let chains: Vec<Vec<Local>> = chains
                .iter()
                .map(|chain| chain.iter().map(|p| [p.x, p.y]).collect())
                .collect();
            // Size is a diameter, as in every other paint tool — so it is the
            // square's side just as it is the disc's width, and switching
            // shapes keeps the stroke the same size across.
            let half_m = km(PropId::SizeKm) / 2.0;
            // A brush has no path direction mode, so the first chain suffices
            // for anything that asks for one.
            let path = chains.first().cloned().unwrap_or_default();
            // Shape 1 is the square stamp. The eraser and the clone stamp share
            // this geometry but declare no `BrushShape`, so they read 0 and
            // stay round.
            let shape = if choice(object, PropId::BrushShape, step) == 1 {
                Shape::SweptSquare {
                    chains,
                    half_size_m: half_m,
                }
            } else {
                Shape::Capsule {
                    chains,
                    radius_m: half_m,
                }
            };
            Some((shape, path))
        }
        Geometry::Disc { radius_m } => {
            // A dragged-out circle carries its own radius; a stamp reads the
            // diameter it was given. Which of the two an object is was decided
            // when it was drawn, so there is no mode to consult here.
            let radius_m = radius_m.unwrap_or_else(|| km(PropId::DiameterKm) / 2.0);
            // Fill mode 1 is the perimeter ring; 0 and 2 are filled.
            let shape = if choice(object, PropId::FillMode, step) == 1 {
                Shape::Annulus {
                    radius_m,
                    half_width_m: km(PropId::RingWidthKm) / 2.0,
                }
            } else {
                Shape::Disc { radius_m }
            };
            Some((shape, Vec::new()))
        }
        Geometry::Polygon { points } => Some((
            Shape::Polygon {
                ring: points.iter().map(|p| [p.x, p.y]).collect(),
            },
            Vec::new(),
        )),
        Geometry::Rect {
            half_width_m,
            half_height_m,
        } => Some((
            Shape::Rect {
                half_width_m: *half_width_m,
                half_height_m: *half_height_m,
            },
            Vec::new(),
        )),
        Geometry::Path { nodes } => {
            let path = flatten_path(nodes);
            let radius_m = km(PropId::WidthKm) / 2.0;
            Some((
                Shape::Capsule {
                    chains: vec![path.clone()],
                    radius_m,
                },
                path,
            ))
        }
    }
}

/// Resolves the object's speed behaviour.
fn speed_of(object: &Object, step: u32, extent: f64) -> SpeedMode {
    let get = |id: PropId| number(object, id, step).unwrap_or(0.0);

    match object.tool {
        // The eraser writes calm; that is the whole of its behaviour.
        ToolKind::Eraser => SpeedMode::Constant(0.0),
        ToolKind::Circle if choice(object, PropId::FillMode, step) == 2 => SpeedMode::Radial {
            centre: get(PropId::SpeedMin),
            edge: get(PropId::SpeedMax),
            extent,
        },
        ToolKind::ShapeFill if choice(object, PropId::VectorMode, step) == 1 => SpeedMode::Axis {
            start: get(PropId::SpeedStart),
            end: get(PropId::SpeedEnd),
        },
        _ => SpeedMode::Constant(get(PropId::Speed)),
    }
}

/// Resolves the object's direction behaviour.
fn direction_of(object: &Object, step: u32, rotation: f64) -> DirectionMode {
    let turned = |a: Angle| Angle::new(a.degrees() + rotation);
    let constant =
        |id: PropId| turned(bearing(object, id, step).unwrap_or_else(|| Angle::new(0.0)));
    let aim = choice(object, PropId::DirectionMode, step);

    match object.tool {
        ToolKind::Circle => DirectionMode::Tangential {
            // Variant 0 is clockwise.
            clockwise: choice(object, PropId::RotationSense, step) == 0,
        },
        ToolKind::Curve => {
            // Mode 1 is relative to the path tangent; 0 is an absolute bearing.
            if choice(object, PropId::CurveDirectionMode, step) == 1 {
                DirectionMode::AlongPath {
                    offset: bearing(object, PropId::Direction, step)
                        .unwrap_or_else(|| Angle::new(0.0)),
                }
            } else {
                DirectionMode::Constant(constant(PropId::Direction))
            }
        }
        ToolKind::ShapeFill if choice(object, PropId::VectorMode, step) == 1 => {
            DirectionMode::Axis {
                start: constant(PropId::DirectionStart),
                end: constant(PropId::DirectionEnd),
            }
        }
        // Aim modes 1 and 2 both point every vector along the great circle
        // through a target: mode 1 at it, mode 2 directly away from it. With no
        // target there is nothing to aim at, so the constant bearing stands in.
        _ if aim == 1 || aim == 2 => match position(object, PropId::Target, step) {
            Some(target) if aim == 1 => DirectionMode::Toward(target),
            Some(target) => DirectionMode::Away(target),
            None => DirectionMode::Constant(constant(PropId::Direction)),
        },
        _ => DirectionMode::Constant(constant(PropId::Direction)),
    }
}

/// Flattens one object for a time step, or `None` if it contributes nothing.
pub fn flatten_object(object: &Object, step: u32) -> Option<FlatObject> {
    if !object.is_active_at(step) {
        return None;
    }
    let anchor = position(object, PropId::Position, step)?;
    let rotation = bearing(object, PropId::RotationDeg, step).map_or(0.0, |a| a.degrees());
    let scale_pct = number(object, PropId::ScalePct, step).unwrap_or(100.0);
    // Space 1 is the map-space stamp. Tools that declare no `StampSpace` read
    // 0 and stay on the ground, which is what every object was before the
    // property existed (spec.md 3.5).
    let space = if choice(object, PropId::StampSpace, step) == 1 {
        Space::Projected
    } else {
        Space::Geodesic
    };
    let frame = Frame::in_space(anchor, rotation, scale_pct, space);

    let (shape, path) = shape_of(object, step)?;
    if shape.is_empty() {
        return None;
    }

    let extent = shape.bounding_radius_m();

    Some(FlatObject {
        cap_radius_m: frame.reach_m(extent),
        speed: speed_of(object, step, extent),
        direction: direction_of(object, step, rotation),
        feather: number(object, PropId::Feather, step)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0),
        divergence: number(object, PropId::Divergence, step)
            .unwrap_or(0.0)
            .clamp(-1.0, 1.0),
        curl: number(object, PropId::Curl, step)
            .unwrap_or(0.0)
            .clamp(-1.0, 1.0),
        edge_mode: if choice(object, PropId::EdgeMode, step) == 1 {
            EdgeMode::Replace
        } else {
            EdgeMode::Blend
        },
        gradient_axis: bearing(object, PropId::GradientAxis, step)
            .map_or_else(|| Angle::new(90.0), |a| Angle::new(a.degrees() + rotation)),
        gradient_extent: extent,
        frame,
        shape,
        path,
        clone_source: if object.tool == ToolKind::CloneStamp {
            // Without a source point there is nothing to copy, so the object
            // contributes nothing rather than sampling itself.
            Some(position(object, PropId::SourcePoint, step)?)
        } else {
            None
        },
    })
}

/// Whether an object covers a position.
///
/// The same cull and signed-distance test the evaluator uses, so what a click
/// selects is exactly what the object paints — no second notion of "inside" to
/// drift out of step with the first.
pub fn covers(object: &FlatObject, position: LonLat) -> bool {
    if object.frame.distance_m(position) > object.cap_radius_m {
        return false;
    }
    object.shape.distance(object.frame.to_local(position)) <= 0.0
}

/// Flattens a whole project for one time step, in z-order.
pub fn flatten(project: &Project, step: u32) -> Scene {
    Scene {
        objects: project
            .objects_in_z_order()
            .filter_map(|(_, object)| flatten_object(object, step))
            .collect(),
    }
}
