//! Creating objects from tool gestures.
//!
//! One command for every tool, deliberately. Spec 6.1's shared behaviour is
//! shared behaviour and not the brush's, and the cheapest way to make that true
//! is to give the tools one write path rather than six that resemble each
//! other: the aim-mode check, the frozen-option rule, the anchor, the frame,
//! the layer choice and the merge test are written once and every tool gets
//! them. A tool that wants different behaviour has to say so here, in front of
//! the rule it is breaking.
//!
//! What a tool supplies is its *gesture* — the geometry the pointer drew — and
//! its *options*, which are property values keyed by the same ids the inspector
//! uses. Neither is tool-specific code.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_core::command::Command;
use ve_core::document::{Geometry, Layer, LocalPoint, Object, PathNode};
use ve_core::schema::{self, PropId, PropertyMap, ToolKind};
use ve_core::value::PropKind;
use ve_core::{LonLat, PropValue};
use ve_render::aeqd::{Frame, Local, Space};
use ve_render::scene::flatten_object;

use crate::commands::AppState;
use crate::document::PropertyValue;
use crate::error::{AppError, Result};
use crate::merge::merge_target;
use crate::projects::{ProjectSummary, with_session};

/// Which tool made the gesture (spec.md 6.2).
///
/// A wire mirror of [`ToolKind`], which cannot derive `TS` from another crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "Tool.ts")]
pub enum Tool {
    /// Freehand painting.
    Brush,
    /// A click-placed rotating disc.
    Circle,
    /// A polygon or a dragged-out preset, filled with a vector field.
    ShapeFill,
    /// Writes calm over what is beneath it.
    Mask,
    /// Samples the composite below it in z-order.
    CloneStamp,
    /// A vector field along a path.
    Curve,
    /// Scales the speed of the field beneath it.
    Intensity,
    /// Bends the field beneath it toward or away from its anchor.
    Divergence,
    /// Turns the field beneath it by a fixed angle.
    Turn,
    /// Displaces the field beneath it.
    Warp,
    /// Drags the field along the stroke (spec.md 6.3, M17).
    Liquify,
    /// Replays a field captured from a region (spec.md 8.5, M14).
    ///
    /// In the wire enum but not in the palette: a patch is pasted rather than
    /// drawn, and the panels still have to name it.
    Patch,
    /// Replays a captured field from the macro library (spec.md 8.7, M16).
    Macro,
}

impl Tool {
    /// The model's tool kind.
    pub fn kind(self) -> ToolKind {
        match self {
            Self::Brush => ToolKind::Brush,
            Self::Circle => ToolKind::Circle,
            Self::ShapeFill => ToolKind::ShapeFill,
            Self::Mask => ToolKind::Mask,
            Self::CloneStamp => ToolKind::CloneStamp,
            Self::Curve => ToolKind::Curve,
            Self::Intensity => ToolKind::Intensity,
            Self::Divergence => ToolKind::Divergence,
            Self::Turn => ToolKind::Turn,
            Self::Warp => ToolKind::Warp,
            Self::Liquify => ToolKind::Liquify,
            Self::Patch => ToolKind::Patch,
            Self::Macro => ToolKind::Macro,
        }
    }

    /// The wire tool for a model kind.
    pub fn of(kind: ToolKind) -> Self {
        match kind {
            ToolKind::Brush => Self::Brush,
            ToolKind::Circle => Self::Circle,
            ToolKind::ShapeFill => Self::ShapeFill,
            ToolKind::Mask => Self::Mask,
            ToolKind::CloneStamp => Self::CloneStamp,
            ToolKind::Curve => Self::Curve,
            ToolKind::Intensity => Self::Intensity,
            ToolKind::Divergence => Self::Divergence,
            ToolKind::Turn => Self::Turn,
            ToolKind::Warp => Self::Warp,
            ToolKind::Liquify => Self::Liquify,
            ToolKind::Patch => Self::Patch,
            ToolKind::Macro => Self::Macro,
        }
    }

    /// The stem new objects of this tool are named from.
    fn noun(self) -> &'static str {
        match self {
            Self::Brush => "Stroke",
            Self::Circle => "Circle",
            Self::ShapeFill => "Shape",
            Self::Mask => "Mask",
            Self::CloneStamp => "Clone",
            Self::Curve => "Curve",
            Self::Intensity => "Intensity",
            Self::Divergence => "Divergence",
            Self::Turn => "Rotation",
            Self::Warp => "Warp",
            Self::Liquify => "Liquify",
            Self::Patch => "Patch",
            Self::Macro => "Macro",
        }
    }
}

/// One node of a drawn path, in geographic coordinates.
#[derive(Debug, Clone, Copy, Deserialize, TS)]
#[ts(export, export_to = "PathPoint.ts")]
pub struct PathPoint {
    /// The on-curve point, as `[lon, lat]`.
    pub at: [f64; 2],
    /// Incoming control handle, if this node is a Bézier control.
    #[serde(default)]
    #[ts(optional)]
    pub in_handle: Option<[f64; 2]>,
    /// Outgoing control handle, if this node is a Bézier control.
    #[serde(default)]
    #[ts(optional)]
    pub out_handle: Option<[f64; 2]>,
}

/// The geometry a gesture drew, in geographic coordinates.
///
/// Deliberately a small closed set rather than one variant per tool: two tools
/// that draw the same way should share the same gesture, so that anything true
/// of one gesture is true for every tool that uses it. The mask and the clone
/// stamp are brush-like *because* all three send a [`Self::Stroke`], not
/// because three separate code paths were written to match.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "Gesture.ts")]
pub enum Gesture {
    /// A freehand polyline: the brush, the mask and the clone stamp.
    Stroke {
        /// Pointer positions as `[lon, lat]`, in the order they were drawn.
        points: Vec<[f64; 2]>,
    },
    /// A single click: the circle stamp.
    Point {
        /// Where it was clicked, as `[lon, lat]`.
        at: [f64; 2],
    },
    /// A drag from a centre outward: the shape fill's presets.
    ///
    /// Centre-out rather than corner-to-corner, so that all three presets have
    /// the same anchor as the shape they produce — the pivot their handles turn
    /// them about.
    Extent {
        /// Where the drag began, as `[lon, lat]`. The shape's centre.
        centre: [f64; 2],
        /// Where it ended, as `[lon, lat]`. A point on the shape's edge.
        rim: [f64; 2],
    },
    /// A closed ring of placed vertices: the shape fill's freehand polygon.
    Ring {
        /// Vertices in order, as `[lon, lat]`. The closing edge is implied.
        points: Vec<[f64; 2]>,
    },
    /// A path of nodes, straight or Bézier: the curve.
    Path {
        /// Nodes in order.
        nodes: Vec<PathPoint>,
    },
}

impl Gesture {
    /// A name for the gesture, for error messages.
    fn label(&self) -> &'static str {
        match self {
            Self::Stroke { .. } => "a stroke",
            Self::Point { .. } => "a click",
            Self::Extent { .. } => "a drag",
            Self::Ring { .. } => "a polygon",
            Self::Path { .. } => "a path",
        }
    }
}

/// One tool option, frozen onto the object at creation (spec.md 6.1).
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "ToolOption.ts")]
pub struct ToolOption {
    /// The property id, spelled as the inspector spells it.
    pub property: String,
    /// The value, in the same shape the inspector sends.
    pub value: PropertyValue,
}

/// A gesture ready to become an object.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "NewObject.ts")]
pub struct NewObject {
    /// Which tool drew it.
    pub tool: Tool,
    /// What it drew.
    pub gesture: Gesture,
    /// The tool's options at the moment of the gesture.
    ///
    /// Anything omitted takes the schema default, which is what makes an
    /// option a tool never offers still exist on the object at a known value.
    #[serde(default)]
    pub options: Vec<ToolOption>,
    /// Which layer receives it. `None` means the top layer.
    #[ts(optional)]
    pub layer: Option<u64>,
}

/// Adds an object drawn with any tool.
#[tauri::command]
pub fn create_object(
    state: tauri::State<'_, AppState>,
    object: NewObject,
) -> Result<ProjectSummary> {
    create(&state, object)
}

/// Reads an enum option, falling back to the schema default.
///
/// Every mode question below asks this rather than the object, because the
/// object does not exist yet when the geometry has to be built.
/// A numeric option the gesture is being drawn with, if it holds one.
fn number_of(props: &PropertyMap, tool: ToolKind, id: PropId) -> Option<f64> {
    props
        .value_at(tool, id, 0)
        .and_then(PropValue::as_f32)
        .map(f64::from)
}

fn choice(props: &PropertyMap, tool: ToolKind, id: PropId) -> u8 {
    props
        .value_at(tool, id, 0)
        .and_then(PropValue::as_enum)
        .unwrap_or(0)
}

/// Turns the wire options into a property map, checking each against the
/// schema.
///
/// Three things are refused. A property the tool does not declare, because
/// accepting it would put a value on the object that nothing reads and that
/// stops two otherwise identical gestures merging. A value of the wrong shape,
/// which [`PropertyValue::into_prop`] catches. And a variant index outside the
/// property's list, which would otherwise render as whichever mode the
/// evaluator's `else` branch happens to be.
fn resolve_options(tool: ToolKind, options: &[ToolOption]) -> Result<PropertyMap> {
    let mut props = PropertyMap::for_tool(tool);

    for option in options {
        let id = schema::all_specs(tool)
            .map(|spec| spec.id)
            .find(|id| format!("{id:?}") == option.property)
            .ok_or_else(|| AppError::BadOption {
                field: "property",
                value: format!("{tool:?} has no property {}", option.property),
            })?;

        // The gesture places the object; an option claiming to do it as well
        // would be silently overruled a few lines below.
        if id == PropId::Position {
            return Err(AppError::BadOption {
                field: "property",
                value: "position comes from the gesture, not from an option".to_owned(),
            });
        }

        let spec = schema::spec_for(tool, id)
            .ok_or_else(|| AppError::Internal(format!("{tool:?}/{id:?} has an id but no spec")))?;
        let value = option.value.clone().into_prop(spec.kind())?;

        if let (PropValue::Enum(index), PropKind::Enum) = (value, spec.kind())
            && usize::from(index) >= spec.variants.len()
        {
            return Err(AppError::BadOption {
                field: "property",
                value: format!(
                    "{} has no variant {index}; it has {}",
                    option.property,
                    spec.variants.len()
                ),
            });
        }

        if let Some(prop) = props.get_mut(id) {
            prop.set_base(value);
        }
    }

    Ok(props)
}

/// Checks the options that only mean something together (spec.md 6.1).
///
/// An aimed direction with nothing to aim at would fall back to the constant
/// bearing and quietly paint a stroke nobody asked for; a clone stamp with no
/// source has nothing to copy and contributes nothing at all. Both are caught
/// before anything is written, and for every tool at once — the rule belongs to
/// the aim modes, not to the brush.
fn check_modes(tool: ToolKind, props: &PropertyMap, options: &[ToolOption]) -> Result<()> {
    let given = |id: PropId| options.iter().any(|o| o.property == format!("{id:?}"));

    if schema::spec_for(tool, PropId::DirectionMode).is_some() {
        let aims = matches!(choice(props, tool, PropId::DirectionMode), 1 | 2);
        // Only when the mode is live: a shape fill on a gradient never reads
        // either of them, so an aim mode left over from the constant branch is
        // not a reason to refuse the gesture.
        let live = schema::is_live(tool, PropId::DirectionMode, |id| choice(props, tool, id));
        if aims && live && !given(PropId::Target) {
            return Err(AppError::BadOption {
                field: "target",
                value: "absent, but the direction mode aims at a point".to_owned(),
            });
        }
    }

    if tool == ToolKind::CloneStamp && !given(PropId::SourcePoint) {
        return Err(AppError::BadOption {
            field: "source_point",
            value: "absent, but a clone stamp has nothing to copy without one".to_owned(),
        });
    }

    Ok(())
}

/// The frame a gesture's geometry is expressed in.
///
/// Which space depends on what the shape is a circle in: a projected gesture's
/// points are map-space metres, and converting them with a ground frame would
/// bend the drawing away from where it was made (spec.md 7.2).
fn frame_at(anchor: LonLat, props: &PropertyMap, tool: ToolKind) -> Frame {
    let space = if choice(props, tool, PropId::StampSpace) == 1 {
        Space::Projected
    } else {
        Space::Geodesic
    };
    Frame::in_space(anchor, 0.0, 100.0, space)
}

/// Reads a `[lon, lat]` pair.
fn point(raw: [f64; 2]) -> Result<LonLat> {
    Ok(LonLat::new(raw[0], raw[1])?)
}

/// Reads every `[lon, lat]` pair, refusing an empty gesture.
fn points(raw: &[[f64; 2]], least: usize, what: &'static str) -> Result<Vec<LonLat>> {
    if raw.len() < least {
        return Err(AppError::BadOption {
            field: what,
            value: format!("{} points, but at least {least} are needed", raw.len()),
        });
    }
    raw.iter().copied().map(point).collect()
}

/// The gesture's anchor and geometry, in the object's own frame.
///
/// The anchor is the point the object's frame is built at, so it is also the
/// pivot its handles turn it about, and the centre a circle's rotation and its
/// radial speed ramp are measured from. Each tool's choice is the point the *user*
/// would call the object's place: where a stroke began, where a stamp was
/// clicked, the middle of a dragged shape, the middle of a placed polygon.
fn geometry_of(
    tool: ToolKind,
    gesture: &Gesture,
    props: &PropertyMap,
) -> Result<(LonLat, Geometry, Vec<LonLat>)> {
    let wrong = || AppError::BadOption {
        field: "gesture",
        value: format!("{tool:?} cannot be drawn with {}", gesture.label()),
    };

    match (tool, gesture) {
        // The brush and four of the operators also take a *region* (spec.md
        // 8.2): with one selected, a click inside it makes that kind of object
        // from the region's boundary — a brush paints the region, a mask masks
        // it. The geometry is the region's own — a polygon, or a disc — and the
        // evaluator keys on geometry rather than on tool, so a brush that is a
        // polygon paints a polygon and a mask that is one masks one.
        //
        // A rectangle arrives as a ring of its four corners, so an extent here
        // can only mean a disc: these tools have no `shape_source` to say
        // otherwise, and giving them one would be a control for a single
        // gesture. The clone stamp and the warp are left out on purpose — both
        // are measured from their anchor to somewhere else, and "the region"
        // does not say where.
        (
            ToolKind::Brush
            | ToolKind::Mask
            | ToolKind::Intensity
            | ToolKind::Divergence
            | ToolKind::Turn,
            Gesture::Ring { points: raw },
        ) => ring_geometry(raw, props, tool),
        (
            ToolKind::Brush
            | ToolKind::Mask
            | ToolKind::Intensity
            | ToolKind::Divergence
            | ToolKind::Turn,
            Gesture::Extent { centre, rim },
        ) => {
            let anchor = point(*centre)?;
            let edge = point(*rim)?;
            let frame = frame_at(anchor, props, tool);
            let [dx, dy] = frame.to_local(edge);
            Ok((
                anchor,
                Geometry::Disc {
                    radius_m: Some(dx.hypot(dy)),
                },
                vec![anchor, edge],
            ))
        }

        // Every painted tool, sharing one gesture and therefore one geometry:
        // the brush, the mask, the clone stamp and the four modifiers. Their
        // anchor is where the gesture began.
        (
            ToolKind::Brush
            | ToolKind::Mask
            | ToolKind::CloneStamp
            | ToolKind::Intensity
            | ToolKind::Divergence
            | ToolKind::Turn
            | ToolKind::Warp,
            Gesture::Stroke { points: raw },
        ) => {
            let positions = points(raw, 1, "points")?;
            let anchor = positions[0];
            let frame = frame_at(anchor, props, tool);
            Ok((
                anchor,
                Geometry::Stroke {
                    chains: vec![local_chain(&frame, &positions)],
                },
                positions,
            ))
        }

        // A liquify is the painted gesture with the pointer's movement kept:
        // each stamp carries the step that reached it, scaled by the strength,
        // so the geometry *is* the displacement (spec.md 6.3, M17). Measured in
        // the local frame after the chain is, so a stamp's delta is the
        // difference of the same numbers its position is.
        (ToolKind::Liquify, Gesture::Stroke { points: raw }) => {
            let positions = points(raw, 1, "points")?;
            let anchor = positions[0];
            let frame = frame_at(anchor, props, tool);
            let chain = local_chain(&frame, &positions);
            let strength = number_of(props, tool, PropId::Strength).unwrap_or(100.0) / 100.0;
            let stamps = chain
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let (dx, dy) = if i == 0 {
                        (0.0, 0.0)
                    } else {
                        let q = chain[i - 1];
                        ((p.x - q.x) * strength, (p.y - q.y) * strength)
                    };
                    ve_core::document::SmearPoint::new(p.x, p.y, dx, dy)
                })
                .collect();
            Ok((
                anchor,
                Geometry::Smear {
                    chains: vec![stamps],
                },
                positions,
            ))
        }

        // A stamp's size is typed, so its geometry carries none.
        (ToolKind::Circle, Gesture::Point { at }) => {
            let anchor = point(*at)?;
            Ok((anchor, Geometry::Disc { radius_m: None }, vec![anchor]))
        }

        (ToolKind::ShapeFill, gesture) => shape_fill_geometry(gesture, props),

        (ToolKind::Curve, Gesture::Path { nodes }) => {
            let first = nodes.first().ok_or_else(|| AppError::BadOption {
                field: "nodes",
                value: "a curve needs at least two nodes".to_owned(),
            })?;
            if nodes.len() < 2 {
                return Err(AppError::BadOption {
                    field: "nodes",
                    value: format!("{} nodes, but a curve needs two", nodes.len()),
                });
            }
            let anchor = point(first.at)?;
            let frame = frame_at(anchor, props, tool);

            let mut converted = Vec::with_capacity(nodes.len());
            let mut positions = Vec::with_capacity(nodes.len());
            for node in nodes {
                let at = point(node.at)?;
                positions.push(at);
                let handle = |raw: Option<[f64; 2]>| -> Result<Option<LocalPoint>> {
                    raw.map(|h| Ok(local(&frame, point(h)?))).transpose()
                };
                converted.push(PathNode {
                    point: local(&frame, at),
                    in_handle: handle(node.in_handle)?,
                    out_handle: handle(node.out_handle)?,
                });
            }
            Ok((anchor, Geometry::Path { nodes: converted }, positions))
        }

        _ => Err(wrong()),
    }
}

/// A geographic position in a frame, as document geometry.
fn local(frame: &Frame, position: LonLat) -> LocalPoint {
    let p = frame.to_local(position);
    LocalPoint::new(p[0], p[1])
}

/// A whole polyline in a frame.
fn local_chain(frame: &Frame, positions: &[LonLat]) -> Vec<LocalPoint> {
    positions.iter().map(|p| local(frame, *p)).collect()
}

/// The middle of a ring of vertices.
///
/// The mean of the local coordinates is only the middle in the frame it was
/// taken in, and an AEQD frame is exact only at its own anchor: taken at the
/// first vertex, a 400 km square's mean lands about 90 m off centre. So the
/// mean is re-taken in the frame its own answer builds, which is a fixed-point
/// iteration that converges immediately — the error is second order in the
/// offset, and the first step has already removed almost all of it.
fn centroid_of(positions: &[LonLat], props: &PropertyMap, tool: ToolKind) -> LonLat {
    /// Enough to reach the metre for any polygon that fits in a frame.
    const PASSES: usize = 3;

    let mut anchor = positions[0];
    for _ in 0..PASSES {
        let frame = frame_at(anchor, props, tool);
        let n = positions.len() as f64;
        let mean: Local = positions.iter().fold([0.0, 0.0], |sum, p| {
            let local = frame.to_local(*p);
            [sum[0] + local[0] / n, sum[1] + local[1] / n]
        });
        anchor = frame.to_global(mean);
    }
    anchor
}

/// A polygon from a ring of placed vertices, for any tool that takes one.
///
/// Its anchor is the mean of its vertices, so that a shape drawn anywhere turns
/// about its own middle rather than about whichever corner happened to be
/// clicked first. The points are re-expressed in the frame that anchor builds,
/// not shifted within the first vertex's frame: the two differ by the AEQD
/// distortion between the points, which is what would otherwise pull a large
/// polygon out of shape.
fn ring_geometry(
    raw: &[[f64; 2]],
    props: &PropertyMap,
    tool: ToolKind,
) -> Result<(LonLat, Geometry, Vec<LonLat>)> {
    let positions = points(raw, 3, "points")?;
    let anchor = centroid_of(&positions, props, tool);
    let frame = frame_at(anchor, props, tool);
    Ok((
        anchor,
        Geometry::Polygon {
            points: local_chain(&frame, &positions),
        },
        positions,
    ))
}

/// The shape fill's geometry, which its `shape_source` decides.
///
/// The freehand polygon and the three presets are different geometries drawn in
/// different ways, which is exactly why `shape_source` is creation-only: there
/// is no honest conversion between a ring of placed vertices and a pair of
/// dragged half-extents.
fn shape_fill_geometry(
    gesture: &Gesture,
    props: &PropertyMap,
) -> Result<(LonLat, Geometry, Vec<LonLat>)> {
    let tool = ToolKind::ShapeFill;
    let source = choice(props, tool, PropId::ShapeSource);

    match (source, gesture) {
        // A freehand polygon. Its anchor is the mean of its vertices, so that a
        // shape drawn anywhere turns about its own middle rather than about
        // whichever corner happened to be clicked first.
        (0, Gesture::Ring { points: raw }) => ring_geometry(raw, props, tool),

        (1..=3, Gesture::Extent { centre, rim }) => {
            let anchor = point(*centre)?;
            let edge = point(*rim)?;
            let frame = frame_at(anchor, props, tool);
            let [dx, dy] = frame.to_local(edge);

            let geometry = match source {
                // A square takes the larger reach, so a drag that is mostly
                // sideways still produces the square it looks like it is
                // producing rather than collapsing to the smaller axis.
                1 => {
                    let half = dx.abs().max(dy.abs());
                    Geometry::Rect {
                        half_width_m: half,
                        half_height_m: half,
                    }
                }
                2 => Geometry::Rect {
                    half_width_m: dx.abs(),
                    half_height_m: dy.abs(),
                },
                _ => Geometry::Disc {
                    radius_m: Some(dx.hypot(dy)),
                },
            };
            Ok((anchor, geometry, vec![anchor, edge]))
        }

        (0, other) => Err(AppError::BadOption {
            field: "gesture",
            value: format!(
                "a polygon is drawn vertex by vertex, not with {}",
                other.label()
            ),
        }),
        (_, other) => Err(AppError::BadOption {
            field: "gesture",
            value: format!(
                "a preset shape is dragged out, not drawn with {}",
                other.label()
            ),
        }),
    }
}

/// Builds and commits an object. The implementation of [`create_object`],
/// callable without a Tauri handle.
pub fn create(state: &AppState, new: NewObject) -> Result<ProjectSummary> {
    let tool = new.tool.kind();
    let props = resolve_options(tool, &new.options)?;
    check_modes(tool, &props, &new.options)?;
    let (anchor, geometry, positions) = geometry_of(tool, &new.gesture, &props)?;

    if !geometry.is_finite() {
        return Err(AppError::BadOption {
            field: "gesture",
            value: "the drawn geometry is not finite".to_owned(),
        });
    }

    with_session(state, |session| {
        let open = session.require_open()?;
        let step_count = open.project.settings.step_count;

        // A new object joins the layer that was selected when it was created
        // (spec.md 6.1); with no selection that is the top of the stack. An
        // imported layer refuses it (D66).
        let layer = crate::document::creation_layer(&open.project, new.layer)?;

        let mut object = Object::new(tool, name_for(&open.project, new.tool), step_count);
        object.geometry = geometry;
        object.props = props;
        if let Some(prop) = object.props.get_mut(PropId::Position) {
            prop.set_base(PropValue::LonLat(anchor));
        }
        // A warp starts pushing nowhere unless the caller said otherwise: its
        // push is measured *from* the anchor, which does not exist until the
        // gesture does, so there is nothing a push could have meant before this
        // point. It is set afterwards by pulling the warp where it should go
        // (spec.md 6.3), which is why the option bar does not offer it — but an
        // option that names it explicitly is still honoured, since by then the
        // caller knows where the gesture went.
        let pushed = new
            .options
            .iter()
            .any(|option| option.property == format!("{:?}", PropId::PushTo));
        if tool == ToolKind::Warp
            && !pushed
            && let Some(prop) = object.props.get_mut(PropId::PushTo)
        {
            prop.set_base(PropValue::LonLat(anchor));
        }

        let command = match merge_into(layer, &object, &positions)? {
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

/// Whether this gesture is absorbed by an existing object, and into what.
///
/// Merging is a property of the *geometry*, so it is offered to every tool that
/// draws a stroke rather than to the brush alone — with one exception stated
/// here rather than buried in the merge test.
fn merge_into(
    layer: &Layer,
    object: &Object,
    positions: &[LonLat],
) -> Result<Option<(ve_core::id::Id, Geometry)>> {
    // A merge re-expresses the new chain in the *target's* frame, and therefore
    // under the target's anchor. A tool whose field is measured from that
    // anchor would paint something different afterwards, which is the one thing
    // a merge may never do (spec.md 6.1): the clone stamp reads from a
    // displacement off it, a divergence radiates from it, and a warp both
    // twists about it and pushes from it. Two strokes of any of those stay two
    // objects.
    //
    // The rest are safe because nothing they do refers to the anchor: an
    // intensity scales and a turn rotates, wherever the frame is centred.
    // A warp is anchored in both of its modes now: a twist turns about the
    // anchor, and a push carries the field from the anchor to a place, so the
    // displacement is the offset between the two and moving the anchor changes
    // it (spec.md 6.3).
    // A liquify merges with nothing: its deltas are its own, and re-expressing
    // two smears under one frame would add the second's movement to the
    // first's stamps (spec.md 6.3, M17).
    let anchored = matches!(
        object.tool,
        ToolKind::CloneStamp | ToolKind::Divergence | ToolKind::Warp | ToolKind::Liquify
    );
    if anchored {
        return Ok(None);
    }
    Ok(merge_target(layer, object, positions))
}

/// A name for a new object: the tool's noun and how many of them there are.
fn name_for(project: &ve_core::project::Project, tool: Tool) -> String {
    let kind = tool.kind();
    let count = project
        .layers
        .iter()
        .flat_map(|layer| &layer.objects)
        .filter(|object| object.tool == kind)
        .count();
    format!("{} {}", tool.noun(), count + 1)
}

/// Whether the object a gesture would create contributes anything at all.
///
/// Used by the tests, which need to know that a gesture produced a footprint
/// rather than an object the evaluator culls.
pub fn is_drawable(object: &Object) -> bool {
    flatten_object(object, 0).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire tools and the model's kinds must stay in step in both
    /// directions, or a gesture would be attributed to the wrong tool.
    #[test]
    fn the_wire_tools_round_trip_through_the_model() {
        for kind in ToolKind::ALL {
            assert_eq!(Tool::of(kind).kind(), kind);
        }
    }

    /// The option names on the wire are the inspector's own spelling, so a
    /// rename in the schema has to break loudly rather than silently dropping
    /// whatever the tool bar was sending.
    #[test]
    fn options_are_named_the_way_the_inspector_names_them() {
        let props = resolve_options(
            ToolKind::Circle,
            &[ToolOption {
                property: "FillMode".to_owned(),
                value: PropertyValue::Choice { index: 2 },
            }],
        )
        .expect("a real property");
        assert_eq!(
            props.value_at(ToolKind::Circle, PropId::FillMode, 0),
            Some(PropValue::Enum(2))
        );

        assert!(
            resolve_options(
                ToolKind::Circle,
                &[ToolOption {
                    property: "fill_mode".to_owned(),
                    value: PropertyValue::Choice { index: 2 },
                }],
            )
            .is_err(),
            "the id is spelled as the inspector spells it"
        );
    }

    #[test]
    fn a_property_another_tool_owns_is_refused() {
        // The mask has no speed: writing calm is the whole of what it does.
        let err = resolve_options(
            ToolKind::Mask,
            &[ToolOption {
                property: "Speed".to_owned(),
                value: PropertyValue::Number { value: 12.0 },
            }],
        )
        .expect_err("the mask has no speed");
        assert!(matches!(err, AppError::BadOption { .. }));
    }

    /// A variant index past the end of the list would otherwise reach the
    /// evaluator, where every mode switch has a fallback branch — so it would
    /// render as some *other* mode rather than failing.
    #[test]
    fn a_variant_that_does_not_exist_is_refused() {
        let err = resolve_options(
            ToolKind::Circle,
            &[ToolOption {
                property: "FillMode".to_owned(),
                value: PropertyValue::Choice { index: 9 },
            }],
        )
        .expect_err("there are three fill modes");
        assert!(matches!(err, AppError::BadOption { .. }));
    }

    #[test]
    fn the_gesture_places_the_object_and_an_option_may_not() {
        let err = resolve_options(
            ToolKind::Brush,
            &[ToolOption {
                property: "Position".to_owned(),
                value: PropertyValue::Position {
                    lon: 10.0,
                    lat: 20.0,
                },
            }],
        )
        .expect_err("position comes from the gesture");
        assert!(matches!(err, AppError::BadOption { .. }));
    }

    /// Spec 6.1: the aim modes mean the same thing for every tool that has a
    /// target, and that includes refusing to aim at nothing. Checked across the
    /// catalogue rather than on the brush, which is the tool it was written for.
    #[test]
    fn no_tool_may_aim_at_a_point_it_was_not_given() {
        for kind in ToolKind::ALL {
            if schema::spec_for(kind, PropId::Target).is_none() {
                continue;
            }
            let options = vec![ToolOption {
                property: "DirectionMode".to_owned(),
                value: PropertyValue::Choice { index: 1 },
            }];
            let props = resolve_options(kind, &options).expect("a real property");
            // The shape fill only reads an aim mode in its constant branch,
            // which is its default, so every tool with a target refuses here.
            assert!(
                check_modes(kind, &props, &options).is_err(),
                "{kind:?} accepted an aim mode with no target"
            );
        }
    }

    #[test]
    fn a_clone_stamp_needs_somewhere_to_copy_from() {
        let props = resolve_options(ToolKind::CloneStamp, &[]).expect("defaults");
        assert!(check_modes(ToolKind::CloneStamp, &props, &[]).is_err());
    }

    /// A gesture belongs to the geometry it draws, not to a tool, so sending
    /// the wrong one has to be refused rather than half-interpreted.
    #[test]
    fn a_tool_refuses_a_gesture_it_does_not_draw() {
        let props = PropertyMap::for_tool(ToolKind::Circle);
        let err = geometry_of(
            ToolKind::Circle,
            &Gesture::Stroke {
                points: vec![[0.0, 0.0], [1.0, 1.0]],
            },
            &props,
        )
        .expect_err("a circle is clicked, not dragged");
        assert!(matches!(err, AppError::BadOption { .. }));
    }

    /// The three presets are one gesture with three readings, and each has to
    /// produce the geometry its name promises.
    #[test]
    fn each_preset_reads_the_same_drag_as_its_own_shape() {
        let drag = Gesture::Extent {
            centre: [0.0, 0.0],
            rim: [3.0, 1.0],
        };

        for (source, check) in [(1u8, "square"), (2, "rectangle"), (3, "circle")] {
            let mut props = PropertyMap::for_tool(ToolKind::ShapeFill);
            props
                .get_mut(PropId::ShapeSource)
                .expect("present")
                .set_base(PropValue::Enum(source));
            let (anchor, geometry, _) =
                geometry_of(ToolKind::ShapeFill, &drag, &props).expect("a preset drag");

            assert_eq!(anchor.lon, 0.0, "{check}: the drag begins at the centre");
            match (source, &geometry) {
                (
                    1,
                    Geometry::Rect {
                        half_width_m,
                        half_height_m,
                    },
                ) => assert_eq!(half_width_m, half_height_m, "a square's sides are equal"),
                (
                    2,
                    Geometry::Rect {
                        half_width_m,
                        half_height_m,
                    },
                ) => assert!(
                    half_width_m > half_height_m,
                    "the drag was wider than it was tall"
                ),
                (3, Geometry::Disc { radius_m }) => {
                    assert!(
                        radius_m.is_some_and(|r| r > 0.0),
                        "a dragged circle carries its radius"
                    );
                }
                _ => panic!("{check}: got {geometry:?}"),
            }
        }
    }

    /// A polygon turns about its own middle. Anchoring it at the first vertex
    /// instead would put the pivot on the rim, so every rotation and scale
    /// would swing the shape about one of its own corners.
    #[test]
    fn a_polygon_is_anchored_at_its_middle() {
        let mut props = PropertyMap::for_tool(ToolKind::ShapeFill);
        props
            .get_mut(PropId::ShapeSource)
            .expect("present")
            .set_base(PropValue::Enum(0));

        let (anchor, geometry, _) = geometry_of(
            ToolKind::ShapeFill,
            &Gesture::Ring {
                points: vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]],
            },
            &props,
        )
        .expect("a square ring");

        assert!(
            anchor.lon.abs() < 1e-6 && anchor.lat.abs() < 1e-6,
            "{anchor:?}"
        );
        let Geometry::Polygon { points } = geometry else {
            panic!("a ring makes a polygon");
        };
        // The four corners sit symmetrically about the anchor, which is the
        // property the mean was computed to give.
        let x: f64 = points.iter().map(|p| p.x).sum();
        let y: f64 = points.iter().map(|p| p.y).sum();
        assert!(x.abs() < 1.0 && y.abs() < 1.0, "corners are not balanced");
    }

    #[test]
    fn a_polygon_needs_three_vertices() {
        let props = PropertyMap::for_tool(ToolKind::ShapeFill);
        assert!(
            geometry_of(
                ToolKind::ShapeFill,
                &Gesture::Ring {
                    points: vec![[0.0, 0.0], [1.0, 0.0]],
                },
                &props,
            )
            .is_err()
        );
    }

    #[test]
    fn a_curve_needs_two_nodes() {
        let props = PropertyMap::for_tool(ToolKind::Curve);
        assert!(
            geometry_of(
                ToolKind::Curve,
                &Gesture::Path {
                    nodes: vec![PathPoint {
                        at: [0.0, 0.0],
                        in_handle: None,
                        out_handle: None,
                    }],
                },
                &props,
            )
            .is_err()
        );
    }

    /// A Bézier node's handles are geographic on the wire and local in the
    /// document, like every other point, so they have to go through the same
    /// frame — a handle left in degrees would bend the curve differently at
    /// every latitude.
    #[test]
    fn a_bezier_handle_is_converted_into_the_object_frame() {
        let props = PropertyMap::for_tool(ToolKind::Curve);
        let (_, geometry, _) = geometry_of(
            ToolKind::Curve,
            &Gesture::Path {
                nodes: vec![
                    PathPoint {
                        at: [0.0, 0.0],
                        in_handle: None,
                        out_handle: Some([1.0, 0.0]),
                    },
                    PathPoint {
                        at: [2.0, 0.0],
                        in_handle: Some([1.5, 0.0]),
                        out_handle: None,
                    },
                ],
            },
            &props,
        )
        .expect("a two-node bezier");

        let Geometry::Path { nodes } = geometry else {
            panic!("a path gesture makes a path");
        };
        let handle = nodes[0].out_handle.expect("kept");
        // One degree of longitude at the equator, in metres, and not the number
        // 1: a handle that stayed in degrees would be a millionth of this.
        assert!(
            (handle.x - 111_195.0).abs() < 500.0,
            "handle at {} m is not one degree east",
            handle.x
        );
    }
}
