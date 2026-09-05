//! Flattening a project into an evaluable scene.
//!
//! For one time step, every animatable property is resolved to a concrete
//! value and every object becomes a [`FlatObject`] carrying its frame, its
//! shape, and its resolved parameters (spec.md 7.1). Nothing downstream of
//! this sees a keyframe, a layer, or a property id.

use std::sync::Arc;

use ve_core::angle::Angle;
use ve_core::capture::{Capture, FramePick, Resample};
use ve_core::document::{Geometry, Object, PathNode, SpeedRange};
use ve_core::project::Project;
use ve_core::raster::RasterGrid;
use ve_core::schema::{PropId, ToolKind};
use ve_core::vector::Uv;
use ve_core::{EARTH_RADIUS_M, LonLat, PropValue};

use ve_core::follow::{Derived, Resolved};

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
    /// Drags the field along a stroke (spec.md 6.3, M17).
    ///
    /// No payload: the displacement is the object's own [`FlatObject::smear`],
    /// one delta per stamp, and the read position at a cell is the cell minus
    /// the feathered sum of the deltas of the stamps that cover it. A warp with
    /// a displacement that varies along the stroke instead of across a region.
    Smear,
}

/// Whether a feathered edge blends with what is beneath it (decision D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeMode {
    /// Fade into the underlying field.
    Blend,
    /// Overwrite, fading the object's own speed toward zero.
    Replace,
}

/// An object's own movement, as the field sees it (spec.md 9.3, M13).
///
/// **A translation and a rotation are each one angular velocity.** A position
/// segment is a great-circle slerp — a rotation of the sphere — so the
/// translation velocity at every cell of a footprint is exactly `Ω × p` for
/// one 3-vector, and a turn about the anchor is `ω` about the anchor's own
/// unit vector. The two add, and one cross product then gives the velocity at
/// any cell, exactly, at the poles and across the seam alike.
///
/// The obvious alternative — take the anchor's speed and bearing and apply it
/// uniformly — is wrong by the frame's own drift a few thousand kilometres
/// out, which is the mistake `aeqd.rs` exists to warn about.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Motion {
    /// Angular velocity of the frame, radians per second, in earth-centred
    /// cartesian coordinates.
    pub omega: [f64; 3],
    /// Relative rate of change of scale, per second: `ṡ / s`. A cell `r`
    /// metres from the anchor moves outward at `scale_rate · r`.
    pub scale_rate: f64,
}

impl Motion {
    /// Whether the object is standing still, which most objects are.
    pub fn is_still(&self) -> bool {
        self.omega == [0.0; 3] && self.scale_rate == 0.0
    }

    /// The velocity this adds at a position, in m/s eastward and northward.
    ///
    /// `Ω × p` is a rate in radians per second on the unit sphere; the earth's
    /// radius turns it into metres. The scale term is radial from the anchor,
    /// along the frame's own radial bearing — the same one the divergence
    /// modifier uses, so the two agree about which way "out" is.
    pub fn velocity_at(&self, frame: &Frame, position: LonLat) -> Uv {
        let mut uv = Uv::default();
        if self.omega != [0.0; 3] {
            let p = unit_vector(position);
            let w = self.omega;
            // Ω × p, then scaled to metres per second.
            let v = [
                (w[1] * p[2] - w[2] * p[1]) * EARTH_RADIUS_M,
                (w[2] * p[0] - w[0] * p[2]) * EARTH_RADIUS_M,
                (w[0] * p[1] - w[1] * p[0]) * EARTH_RADIUS_M,
            ];
            let (east, north) = east_north(position);
            uv.u = (v[0] * east[0] + v[1] * east[1] + v[2] * east[2]) as f32;
            uv.v = (v[0] * north[0] + v[1] * north[1] + v[2] * north[2]) as f32;
        }
        if self.scale_rate != 0.0 {
            let radial = self.scale_rate * frame.distance_m(position);
            let bearing = frame.radial_bearing(position).radians();
            uv.u += (radial * bearing.sin()) as f32;
            uv.v += (radial * bearing.cos()) as f32;
        }
        uv
    }
}

/// A position as a unit vector in earth-centred cartesian coordinates.
fn unit_vector(position: LonLat) -> [f64; 3] {
    let (lat, lon) = (position.lat.to_radians(), position.lon.to_radians());
    let c = lat.cos();
    [c * lon.cos(), c * lon.sin(), lat.sin()]
}

/// The eastward and northward unit vectors at a position.
///
/// At a pole east is undefined; the returned pair is then the meridian of the
/// stated longitude, which is the same choice every other bearing here makes.
fn east_north(position: LonLat) -> ([f64; 3], [f64; 3]) {
    let (lat, lon) = (position.lat.to_radians(), position.lon.to_radians());
    let (sin_lon, cos_lon) = lon.sin_cos();
    let (sin_lat, cos_lat) = lat.sin_cos();
    (
        [-sin_lon, cos_lon, 0.0],
        [-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat],
    )
}

/// A patch's samples, resolved for one step.
///
/// Shared rather than owned: the document is cloned freely — into history,
/// into render snapshots — and a captured field can run to megabytes.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatCapture {
    /// The whole capture, for its lattice geometry.
    pub capture: Arc<Capture>,
    /// Which of its frames this step shows, and how far between two of them
    /// (spec.md 8.7). A patch has one frame and picks it; a macro's run of
    /// them lands on the project's steps by its own rule.
    pub pick: FramePick,
    /// How far the capture's region had moved by this frame, in degrees of
    /// the map — zero unless the capture recorded movement.
    pub shift_deg: [f64; 2],
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
    /// A liquify stroke's deltas, one per stamp of each chain, in local metres
    /// (spec.md 6.3, M17). Empty for everything else.
    ///
    /// Parallel to the capsule's chains in [`Self::shape`]: stamp `i` of chain
    /// `c` is at `shape.chains[c][i]` and carries `smear[c][i]`. Kept beside
    /// the shape rather than in it so the footprint stays the one capsule
    /// every swept tool shares — coverage, outlines and culling know nothing
    /// about the smear.
    pub smear: Vec<Vec<Local>>,
    /// The captured field this object replays, if it is a patch
    /// (spec.md 8.5, M14).
    ///
    /// The lattice is in the object's **own frame**, measured in degrees of
    /// the map: a patch is captured over a region, a region is map space, and
    /// the frame carries the anchor, the rotation and the scale. So a patch
    /// moves, turns and grows like any object and the samples never move at
    /// all.
    pub capture: Option<FlatCapture>,
    /// Whether the object *removes* field rather than writing one.
    ///
    /// True for the mask, and only for it. The mask writes calm, which is what
    /// the field shows — but a cell it removed is **undefined** in a capture
    /// rather than a real zero, so that a patch taken over one is transparent
    /// there instead of painting a hole of dead air (spec.md 8.5, D58).
    ///
    /// Read by the capture and by nothing else, so it is deliberately **not**
    /// in the render cache's key: it changes no frame, and §7.10's rule is
    /// that a key changes exactly when a frame does.
    pub erases: bool,
    /// The object's own movement, added to what it paints (spec.md 9.3, M13).
    ///
    /// Still unless the user asked for it, and set by [`flatten`] rather than
    /// by [`flatten_object`]: the velocity needs the step size and the
    /// neighbouring steps, which only a whole-project flatten has. Every other
    /// caller wants a footprint and gets a still one.
    pub motion: Motion,
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
        // A liquify's footprint is the capsule every swept tool has; the
        // deltas ride beside it on the flat object (`smear_of`), so nothing
        // that measures coverage, outlines or culls learns a new shape.
        Geometry::Smear { chains } => {
            let chains: Vec<Vec<Local>> = chains
                .iter()
                .map(|chain| chain.iter().map(|p| [p.x, p.y]).collect())
                .collect();
            let path = chains.first().cloned().unwrap_or_default();
            Some((
                Shape::Capsule {
                    chains,
                    radius_m: km(PropId::SizeKm) / 2.0,
                },
                path,
            ))
        }
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
        ToolKind::Liquify => Some(Modifier::Smear),
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

/// A liquify stroke's deltas, or nothing for any other geometry.
///
/// The deltas were scaled by the stroke's strength when it was drawn, so
/// there is nothing to consult here: the geometry is the displacement.
fn smear_of(object: &Object) -> Vec<Vec<Local>> {
    match &object.geometry {
        Geometry::Smear { chains } => chains
            .iter()
            .map(|chain| chain.iter().map(|p| [p.dx, p.dy]).collect())
            .collect(),
        _ => Vec::new(),
    }
}

/// Flattens one object for a time step, or `None` if it contributes nothing.
pub fn flatten_object(object: &Object, step: u32) -> Option<FlatObject> {
    flatten_object_at(object, step, Derived::default())
}

/// [`flatten_object`], with the anchor and rotation a link derives
/// (spec.md 9.3, M13).
///
/// A follower's `position` or `rotation_deg` is not its own, and every caller
/// that asks where an object *is* — the field, the map's outlines, hit
/// testing — has to ask the same question. `Derived::default()` is "follows
/// nothing", which is what a freshly drawn object always is.
pub fn flatten_object_at(object: &Object, step: u32, derived: Derived) -> Option<FlatObject> {
    if !object.is_active_at(step) {
        return None;
    }
    let anchor = derived
        .position
        .or_else(|| position(object, PropId::Position, step))?;
    let rotation = derived
        .rotation
        .unwrap_or_else(|| bearing(object, PropId::RotationDeg, step).map_or(0.0, |a| a.degrees()));
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
        smear: smear_of(object),
        capture: None,
        erases: object.tool == ToolKind::Mask,
        motion: Motion::default(),
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

/// The object's own movement at a step, for the tracks it was told to use
/// (spec.md 9.3, M13).
///
/// **The velocity comes from the track the field already uses**: the central
/// difference of the property's own sampled value at `step - 1` and
/// `step + 1`, one-sided at the ends, never a second interpolation of the
/// keys. A held segment contributes nothing — a value that jumps is a
/// teleport — and so does a property with no keys.
/// A project's links resolved at the three steps a velocity needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Links {
    /// At the step being flattened.
    pub at: Resolved,
    /// At the step before it.
    pub before: Resolved,
    /// And the one after.
    pub after: Resolved,
}

impl Links {
    /// Resolves a project's links at a step and at its two neighbours.
    ///
    /// Three passes rather than one because a follower's *velocity* is the
    /// velocity of the value it derives, not of the keys it is ignoring. Each
    /// costs one walk of the objects and returns immediately when nothing
    /// follows anything, which is every project until someone makes a link.
    pub fn resolve(project: &Project, step: u32) -> Self {
        let last = project.settings.last_step();
        Self {
            at: ve_core::follow::resolve(project, step),
            before: ve_core::follow::resolve(project, step.saturating_sub(1)),
            after: ve_core::follow::resolve(project, (step + 1).min(last)),
        }
    }

    /// Whether nothing in the project follows anything.
    pub fn is_empty(&self) -> bool {
        self.at.is_empty() && self.before.is_empty() && self.after.is_empty()
    }
}

pub fn motion_of(
    object: &Object,
    step: u32,
    step_hours: u32,
    last_step: u32,
    links: &Links,
) -> Motion {
    let flags = object.motion;
    if !flags.any() || step_hours == 0 {
        return Motion::default();
    }
    // The span the difference is taken over: two steps in the middle of the
    // timeline, one at either end.
    let before = step.saturating_sub(1);
    let after = (step + 1).min(last_step);
    if after == before {
        return Motion::default();
    }
    let dt = f64::from(after - before) * f64::from(step_hours) * 3600.0;

    // A followed property's own keys are dormant, so "does it hold?" is a
    // question about the primary and not about them: the derived values speak
    // for themselves, and a primary that stands still differences to nothing.
    let animated = |id: PropId| -> bool {
        ve_core::follow::follow_of(object, id).is_some()
            || object
                .props
                .get(id)
                .is_some_and(|a| !a.holds_across(before, after))
    };
    let anchor_at = |s: u32, resolved: &Resolved| -> Option<LonLat> {
        resolved
            .of(object.id)
            .position
            .or_else(|| position(object, PropId::Position, s))
    };
    let turn_at = |s: u32, resolved: &Resolved| -> Option<f64> {
        resolved
            .of(object.id)
            .rotation
            .or_else(|| bearing(object, PropId::RotationDeg, s).map(|a| a.degrees()))
    };

    let mut omega = [0.0f64; 3];
    if flags.position
        && animated(PropId::Position)
        && let (Some(from), Some(to)) = (
            anchor_at(before, &links.before),
            anchor_at(after, &links.after),
        )
    {
        omega = add(omega, translation_omega(from, to, dt));
    }
    if flags.rotation
        && animated(PropId::RotationDeg)
        && let (Some(from), Some(to), Some(anchor)) = (
            turn_at(before, &links.before),
            turn_at(after, &links.after),
            anchor_at(step, &links.at),
        )
    {
        // A bearing increases clockwise, and a clockwise turn seen from
        // outside the sphere is a *negative* rotation about the outward
        // normal by the right-hand rule.
        let rate = -shortest_arc(from, to).to_radians() / dt;
        let axis = unit_vector(anchor);
        omega = add(omega, [axis[0] * rate, axis[1] * rate, axis[2] * rate]);
    }
    let mut scale_rate = 0.0;
    if flags.scale && animated(PropId::ScalePct) {
        let at = |s: u32| number(object, PropId::ScalePct, s).unwrap_or(100.0);
        let (from, to, now) = (at(before), at(after), at(step));
        // The relative rate: a cell `r` from the anchor sits at local radius
        // `r / s`, so its ground speed is `r · ṡ / s`.
        if now.abs() > 1e-9 {
            scale_rate = (to - from) / (now * dt);
        }
    }
    Motion { omega, scale_rate }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Degrees from `from` to `to` the short way round, in `(-180, 180]`.
fn shortest_arc(from: f64, to: f64) -> f64 {
    let d = (to - from + 180.0).rem_euclid(360.0) - 180.0;
    if d <= -180.0 { d + 360.0 } else { d }
}

/// The angular velocity of the great-circle move from `from` to `to`.
///
/// A position segment interpolates along a great circle, which *is* a rotation
/// of the sphere: the axis is the normal of the plane through both points and
/// the rate is the angle between them over the time. Two coincident points
/// have no axis and no motion, and two antipodal ones have no unique axis —
/// which is a 20,000 km jump, and not a wind.
fn translation_omega(from: LonLat, to: LonLat, dt: f64) -> [f64; 3] {
    let (a, b) = (unit_vector(from), unit_vector(to));
    let axis = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let sin = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if sin < 1e-15 {
        return [0.0; 3];
    }
    let cos = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let rate = sin.atan2(cos) / dt / sin;
    [axis[0] * rate, axis[1] * rate, axis[2] * rate]
}

/// The captured field a patch shows at a step (spec.md 8.5, M14).
///
/// A still capture shows its one frame at every step. An animated one is
/// aligned with the project's own hours from the object's first active step,
/// so a captured run of frames plays where the patch was put rather than
/// where it was taken.
fn capture_of(project: &Project, object: &Object, step: u32) -> Option<FlatCapture> {
    let hash = object.capture.as_deref()?;
    let capture = project.captures.get(hash)?;
    if capture.frames.len() == 1 {
        return Some(FlatCapture {
            capture: Arc::clone(capture),
            pick: FramePick {
                frame: 0,
                next: 0,
                blend: 0.0,
            },
            shift_deg: [0.0, 0.0],
        });
    }
    // A run of frames lands on the project's steps by the object's own rule
    // (spec.md 8.7): a macro is a thing the user *placed*, so it holds like a
    // keyframe rather than vanishing like a message — §4.8's rule against
    // holding a measurement forward is about a forecast, and this is not one.
    let hours_per_step = f64::from(project.settings.step_hours.hours());
    let elapsed = f64::from(step.saturating_sub(object.active_range.start)) * hours_per_step;
    let resample = if choice(object, PropId::Resample, step) == 1 {
        Resample::Interpolate
    } else {
        Resample::Hold
    };
    let looping = object
        .props
        .value_at(object.tool, PropId::LoopMacro, step)
        .and_then(PropValue::as_bool)
        .unwrap_or(false);
    // A patch is a copy of what was painted, and what was painted does not
    // vanish when its run ends: past its last frame a patch holds that frame
    // (D65). A macro is placed with a `loop` switch of its own and keeps
    // spec.md 8.7's rule — nothing past the end unless it loops.
    let pick = capture.pick(elapsed, resample, looping).or_else(|| {
        (object.tool == ToolKind::Patch && elapsed >= 0.0).then(|| FramePick {
            frame: capture.frames.len() - 1,
            next: capture.frames.len() - 1,
            blend: 0.0,
        })
    })?;
    let (dx, dy) = capture.displacement(pick);
    Some(FlatCapture {
        capture: Arc::clone(capture),
        pick,
        shift_deg: [dx, dy],
    })
}

/// Flattens a whole project for one time step, in z-order.
///
/// A layer's imported field, if it has one, goes in beneath the layer's own
/// objects — at the steps the file has a message for, and at no others: a step
/// whose time the file says nothing about has no imported field at all, and
/// what was painted on the layer stands alone there (spec.md 4.8).
///
/// Which frame that is can be the user's choice rather than the file's, where
/// a step carries an override (M20). What arrives here either way is a frame
/// the *file* holds, so nothing below this line — the cache key, the
/// readiness probe, either kernel — needs to know the choice was made.
pub fn flatten(project: &Project, step: u32) -> Scene {
    // Links are resolved once for the whole project, in dependency order: a
    // follower needs its primary placed first, and its primary may follow
    // something in turn (spec.md 9.3).
    let links = Links::resolve(project, step);
    let mut scene = Scene::default();
    for layer in project.layers.iter().filter(|l| l.visible) {
        if let Some(frame) = layer.imported_frame(&project.settings, step) {
            scene.rasters.push(FlatRaster {
                z: scene.objects.len(),
                grid: Arc::clone(&frame.grid),
                speed_range: layer.speed_range,
            });
        }
        let hours = project.settings.step_hours.hours();
        let last = project.settings.last_step();
        scene
            .objects
            .extend(layer.objects.iter().filter_map(|object| {
                // A patch replays a captured field (spec.md 8.5). The samples
                // live beside the project, keyed by hash, so one whose entry
                // is missing simply has none — it draws nothing, exactly as a
                // GRIB layer whose file has gone.
                let patch = capture_of(project, object, step);
                let mut derived = links.at.of(object.id);
                // A macro that recorded a moving region moves the **whole
                // object**: its anchor, and with it its footprint, its outline
                // and what a click selects (spec.md 8.7). Shifting only the
                // lattice lookup would leave the field trying to draw outside
                // the shape that admits it, and nothing would appear.
                if let Some(shift) = patch.as_ref().map(|p| p.shift_deg)
                    && shift != [0.0, 0.0]
                {
                    let base = derived
                        .position
                        .or_else(|| position(object, PropId::Position, step))?;
                    derived.position = LonLat::new(
                        ve_core::geo::normalize_lon(base.lon + shift[0]),
                        (base.lat + shift[1]).clamp(-90.0, 90.0),
                    )
                    .ok();
                }
                let mut flat = flatten_object_at(object, step, derived)?;
                flat.motion = motion_of(object, step, hours, last, &links);
                flat.capture = patch;
                Some(flat)
            }));
    }
    scene
}
