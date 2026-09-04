//! Flattening a project into an evaluable scene.
//!
//! For one time step, every animatable property is resolved to a concrete
//! value and every object becomes a [`FlatObject`] carrying its frame, its
//! shape, and its resolved parameters (spec.md 7.1). Nothing downstream of
//! this sees a keyframe, a layer, or a property id.

use std::sync::Arc;

use ve_core::angle::Angle;
use ve_core::document::{Geometry, Object, PathNode, SpeedRange};
use ve_core::project::Project;
use ve_core::raster::RasterGrid;
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
    /// A direction mode of its own rather than a component added to a base
    /// bearing: a circle has no bearing to add to, so "rotating" would have
    /// meant "north, plus a turn", and came out drifting northward. This is the
    /// whole of a circle's flow — no tool has a radial or tangential component
    /// on top of its direction any more (spec.md 7.5).
    Tangential {
        /// True for clockwise, false for counter-clockwise.
        clockwise: bool,
    },
}

/// Where a clone stamp's source sits as the brush moves (spec.md 6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetMode {
    /// The source moves with the brush, preserving the offset the gesture
    /// began with. Painting a long stroke copies a correspondingly long band
    /// of the field, shifted by that offset.
    Aligned,
    /// The source stays where it was put, so every stamp along the stroke reads
    /// the same neighbourhood of it. Painting a long stroke repeats one patch
    /// rather than copying a band.
    Fixed,
}

/// How a warp displaces the position it reads from (spec.md 6.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Warp {
    /// A constant displacement in the object's local frame, in geometry units.
    ///
    /// Local rather than a bearing and a distance, so the object's own rotation
    /// and scale turn and size the push with it, exactly as they do its
    /// footprint — a transform acts on the frame (spec.md 7.2).
    Push {
        /// Local x, positive toward the frame's east.
        x: f64,
        /// Local y, positive toward the frame's north.
        y: f64,
    },
    /// A rotation of the read position about the anchor, degrees clockwise.
    Twist {
        /// Degrees.
        degrees: f64,
    },
}

/// What a modifier does to the field beneath it (spec.md 6.3).
///
/// A modifier has no field of its own: it reads the composite below it in
/// z-order and writes back a transformed version of it, inside its footprint
/// and faded by its feather. Every variant is therefore a function of what was
/// already there — which is what makes them safe to stack in any order the
/// z-order allows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Modifier {
    /// Scales the speed by `1 + gain`, leaving the direction alone.
    ///
    /// A fraction, not a percentage: the option is in percent and is divided
    /// once, here, so nothing downstream has to remember which it holds.
    Gain(f64),
    /// Adds a radial component of `fraction` times the local speed, outward
    /// from the anchor when positive and inward when negative.
    Radial(f64),
    /// Turns every vector clockwise by this many degrees.
    Turn(f64),
    /// Reads the field from a displaced position, which is what makes a warp a
    /// warp: nothing about the vectors changes, only where they are read.
    Warp(Warp),
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
    /// Whether that source travels with the brush.
    ///
    /// Meaningless without [`Self::clone_source`], and carried beside it rather
    /// than inside it so the evaluator's "is this a clone stamp" test stays one
    /// `Option` check on the hot path.
    pub clone_offset: OffsetMode,
    /// Whether the object writes everywhere *except* its footprint.
    ///
    /// The mask's invert (spec.md 6.2). It turns the coverage test inside out
    /// — including the spherical-cap cull, which is why it is a field on the
    /// object rather than a property the evaluator has to look up: a cell a
    /// thousand kilometres away is *inside* an inverted mask, and the cull that
    /// makes every other object cheap would otherwise skip it.
    pub invert: bool,
    /// What this object does to the field beneath it, if it is a modifier.
    ///
    /// `Some` excludes [`Self::speed`] and [`Self::direction`] from meaning
    /// anything: a modifier writes what it read, changed. The evaluator tests
    /// this before it tests anything else about the object.
    pub modifier: Option<Modifier>,
}

/// An imported field, resolved to the one time slice this step shows.
///
/// A raster sits in the z-order like anything else: it is drawn beneath the
/// objects of its own layer and over everything in the layers below. Where
/// the grid has a value it overwrites; where it has none — outside a regional
/// grid, or under a bitmap's gaps — it leaves the field beneath alone. No
/// feather: a lattice has no edge to soften.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatRaster {
    /// How many objects lie beneath it. The raster is applied before object
    /// `z`, or after the last object when `z` equals the object count.
    pub z: usize,
    /// The lattice, shared with the layer that owns it.
    pub grid: Arc<RasterGrid>,
    /// Which speeds the layer keeps, if it filters (spec.md 4.8).
    ///
    /// A sample outside the band is treated exactly as a missing one: the
    /// field beneath shows through. Carried per raster rather than applied to
    /// the lattice, because the lattice is shared, content-hashed and re-read
    /// from the file — the band is a choice about what to show.
    pub speed_range: Option<SpeedRange>,
}

/// A whole time step, ready to evaluate. Objects are in z-order, bottom first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    /// Objects, bottom of the stack first.
    pub objects: Vec<FlatObject>,
    /// Imported fields, in increasing `z`.
    pub rasters: Vec<FlatRaster>,
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
            // Shape 1 is the square stamp. The mask and the clone stamp share
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
        // The mask writes calm; that is the whole of its behaviour.
        ToolKind::Mask => SpeedMode::Constant(0.0),
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

/// What a modifier object does, or `None` for a tool that paints a field.
///
/// The percentages become fractions here, and the warp's bearing and distance
/// become a local displacement, so the evaluator receives numbers it can use
/// rather than options it has to interpret. A push is turned into the local
/// frame by construction: local angles are measured from the frame's own north,
/// which is already the object's rotation (spec.md 7.2), so an object rotated
/// 30° pushes 30° further round without the bearing being touched here.
fn modifier_of(object: &Object, step: u32) -> Option<Modifier> {
    // The frame a warp's target is expressed in. Built the same way
    // `flatten_object` builds it, and only where it is needed.
    fn frame_for(object: &Object, step: u32) -> Option<Frame> {
        let anchor = position(object, PropId::Position, step)?;
        let rotation = bearing(object, PropId::RotationDeg, step).map_or(0.0, |a| a.degrees());
        let scale_pct = number(object, PropId::ScalePct, step).unwrap_or(100.0);
        let space = if choice(object, PropId::StampSpace, step) == 1 {
            Space::Projected
        } else {
            Space::Geodesic
        };
        Some(Frame::in_space(anchor, rotation, scale_pct, space))
    }

    let get = |id: PropId| number(object, id, step).unwrap_or(0.0);
    match object.tool {
        ToolKind::Intensity => Some(Modifier::Gain(get(PropId::Gain) / 100.0)),
        ToolKind::Divergence => Some(Modifier::Radial(get(PropId::Radial) / 100.0)),
        ToolKind::Turn => Some(Modifier::Turn(get(PropId::TurnDeg))),
        ToolKind::Warp => {
            // Mode 1 twists about the anchor; 0 pushes along a bearing.
            if choice(object, PropId::WarpMode, step) == 1 {
                Some(Modifier::Warp(Warp::Twist {
                    degrees: get(PropId::TwistDeg),
                }))
            } else {
                // The push is the target's own place in the object's frame:
                // the field under the anchor is dragged to the target, and the
                // frame does the geodesy. Both ends are animatable positions,
                // so a warp that grows or travels is two keyframed points
                // (spec.md 6.3).
                let target = position(object, PropId::PushTo, step)?;
                let local = frame_for(object, step)?.to_local(target);
                Some(Modifier::Warp(Warp::Push {
                    x: local[0],
                    y: local[1],
                }))
            }
        }
        _ => None,
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
        // Offset mode 1 is the fixed source.
        clone_offset: if choice(object, PropId::OffsetMode, step) == 1 {
            OffsetMode::Fixed
        } else {
            OffsetMode::Aligned
        },
        modifier: modifier_of(object, step),
        // Only the mask has the property; everything else reads `false` and
        // covers what it is drawn over, as it always did.
        invert: object
            .props
            .value_at(object.tool, PropId::Invert, step)
            .and_then(PropValue::as_bool)
            .unwrap_or(false),
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
///
/// A layer's imported field, if it has one, goes in beneath the layer's own
/// objects — at the steps the file has a message for, and at no others: a step
/// whose time the file says nothing about has no imported field at all, and
/// what was painted on the layer stands alone there (spec.md 4.8).
pub fn flatten(project: &Project, step: u32) -> Scene {
    let hour = f64::from(project.settings.forecast_hour(step));
    let mut scene = Scene::default();
    for layer in project.layers.iter().filter(|l| l.visible) {
        if let Some(frame) = layer.raster.as_ref().and_then(|s| s.frame_at(hour)) {
            scene.rasters.push(FlatRaster {
                z: scene.objects.len(),
                grid: Arc::clone(&frame.grid),
                speed_range: layer.speed_range,
            });
        }
        scene.objects.extend(
            layer
                .objects
                .iter()
                .filter_map(|object| flatten_object(object, step)),
        );
    }
    scene
}
