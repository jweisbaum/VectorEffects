//! The property schema: what properties each tool has, and what they mean.
//!
//! Storage is dynamic ([`crate::keyframe::Animatable`] keyed by [`PropId`]) so
//! the timeline and the inspector can iterate an object's properties without a
//! hand-written case per tool. This module is the static counterpart that gives
//! those dynamic values their meaning: label, kind, default, range, unit, and
//! for enums the variant names.
//!
//! **Adding a property is one line in one table here**, plus the evaluator work
//! listed in `CLAUDE.md`. If it ever takes more than that on the model side,
//! fix this module rather than working around it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::angle::Angle;
use crate::geo::LonLat;
use crate::keyframe::Animatable;
use crate::value::{PropKind, PropValue};

/// Which vector-creation tool produced an object (spec.md 6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// Freehand painting.
    Brush,
    /// Rotating circle stamp.
    Circle,
    /// Polygon or preset shape filled with a vector field.
    ShapeFill,
    /// Writes zero-speed vectors: a mask over what is beneath it.
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
    /// Reads the field beneath it from a displaced position.
    Warp,
    /// Drags the field beneath it along a stroke, each stamp carrying the
    /// pointer's own movement (spec.md 6.3, M17).
    ///
    /// The forward warp of a paint program. A warp moves a *placed* region's
    /// field as one block; a liquify smears it along the hand.
    Liquify,
    /// Replays a captured field from the macro library (spec.md 8.7, M16).
    ///
    /// A creation tool with a field: unlike a patch, it *is* placed by a
    /// gesture — the insert tool's click — and it carries the resample
    /// options a run of frames needs.
    Macro,
    /// Replays a field captured from a region of the map (spec.md 8.5, M14).
    ///
    /// Not in the palette: a patch is not *drawn*, it is pasted. It is a tool
    /// kind all the same, because it is an object in the document with a
    /// lifetime, a frame, keyframes and an inspector row like any other.
    Patch,
}

impl ToolKind {
    /// Every tool, in palette order.
    ///
    /// The tools that lay a field down first, in the order a stroke, a stamp,
    /// a fill and a path suggest; then the mask, which takes one away; then
    /// the modifiers, which change one — because there has to be a field
    /// before either of those does anything.
    pub const ALL: [Self; 13] = [
        Self::Brush,
        Self::Circle,
        Self::ShapeFill,
        Self::CloneStamp,
        Self::Curve,
        Self::Mask,
        Self::Intensity,
        Self::Divergence,
        Self::Turn,
        Self::Warp,
        Self::Liquify,
        Self::Patch,
        Self::Macro,
    ];

    /// Whether the palette offers this tool.
    ///
    /// Every kind but the patch, which is pasted rather than drawn: there is
    /// no gesture that makes one, so a palette entry would be a button that
    /// does nothing (spec.md 8.5).
    pub fn in_palette(self) -> bool {
        // The patch is pasted and the macro is placed by the insert tool,
        // which carries the library rather than a schema bar.
        !matches!(self, Self::Patch | Self::Macro)
    }

    /// Whether the tool modifies the field beneath it rather than adding one
    /// (spec.md 6.3).
    ///
    /// A modifier has no speed and no direction of its own: it reads the
    /// composite below it in z-order, transforms it, and writes it back inside
    /// its footprint. What follows from that is worth stating once here rather
    /// than being rediscovered per call site — it has no `edge_mode`, because
    /// there is nothing to replace *with*, and nothing to preview as a colour,
    /// because its result is whatever it was given.
    pub fn is_modifier(self) -> bool {
        matches!(
            self,
            Self::Intensity | Self::Divergence | Self::Turn | Self::Warp | Self::Liquify
        )
    }

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Brush => "Brush",
            Self::Circle => "Circle",
            Self::ShapeFill => "Shape fill",
            Self::Mask => "Mask",
            Self::CloneStamp => "Clone stamp",
            Self::Curve => "Curve",
            Self::Intensity => "Intensify / reduce",
            Self::Divergence => "Diverge / converge",
            Self::Turn => "Rotate flow",
            Self::Warp => "Warp",
            Self::Liquify => "Liquify",
            Self::Patch => "Patch",
            Self::Macro => "Macro",
        }
    }
}

/// Identifies a property. Ordering is declaration order, which gives the
/// document a canonical, stable property order for free (invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PropId {
    // --- Common to every object (spec.md 4.4) ---
    /// How a macro's frames land on the project's steps (spec.md 8.7).
    Resample,
    /// Whether a macro repeats past its last frame.
    LoopMacro,
    /// The AEQD anchor: where the object sits.
    Position,
    /// Scales geometry radii, in percent.
    ScalePct,
    /// True-bearing rotation about the anchor.
    RotationDeg,
    /// Per-step on/off switch.
    Enabled,
    /// Whether a feathered edge blends with what is beneath it (decision D12).
    EdgeMode,

    // --- Sizes ---
    /// Brush, eraser, and clone-stamp footprint.
    SizeKm,
    /// Circle stamp diameter.
    DiameterKm,
    /// Curve corridor width.
    WidthKm,
    /// Circle perimeter thickness.
    RingWidthKm,

    // --- Speeds ---
    /// Constant speed.
    Speed,
    /// Gradient minimum speed.
    SpeedMin,
    /// Gradient maximum speed.
    SpeedMax,
    /// Gradient start speed.
    SpeedStart,
    /// Gradient end speed.
    SpeedEnd,

    // --- Directions ---
    /// Constant direction.
    Direction,
    /// Gradient start direction.
    DirectionStart,
    /// Gradient end direction.
    DirectionEnd,
    /// Bearing along which a gradient ramps.
    GradientAxis,
    /// Constant, or point at a target.
    DirectionMode,
    /// The point vectors aim at in `toward_point` mode.
    Target,

    // --- Field shaping ---
    /// Edge falloff, 0 to 1.
    Feather,

    // --- Brush ---
    /// Circular or square brush footprint.
    ///
    /// Its own id rather than a share of [`Self::FillMode`]: the circle reads
    /// `FillMode` to choose between a filled disc, a ring and a gradient, and a
    /// brush answering that question with "square" is a trap waiting for the
    /// first piece of code that reads the property without checking the tool.
    BrushShape,
    /// Whether a shape is defined on the ground or on the map.
    ///
    /// A shape sized in pixels is asking for one on the map: a ground disc
    /// projects to an ellipse, twice as wide as tall at 60 degrees (spec.md
    /// 3.5). `projected` makes the footprint a circle on screen at every
    /// latitude, at the cost of covering less ground east-west.
    ///
    /// **Shared by every tool with a size**, not the brush's alone: the circle
    /// stamp's disc and the shape fill's presets ask the same question, and one
    /// name for it is what stops "px" meaning two things (spec.md 6.1).
    StampSpace,

    // --- Circle ---
    /// Filled, perimeter, or filled with a radial gradient.
    FillMode,
    /// Clockwise or counter-clockwise.
    RotationSense,

    // --- Shape fill ---
    /// Constant vector, or a gradient.
    VectorMode,
    /// Which geometry the shape fill *is*: a freehand polygon or a preset.
    ///
    /// Creation-only, like every property that decides what an object is: a
    /// polygon and a dragged-out rectangle are different geometries, and
    /// switching between them afterwards has no honest answer for the
    /// vertices the user placed.
    ShapeSource,

    // --- Clone stamp ---
    /// Where the clone samples from.
    SourcePoint,
    /// Whether the source follows the brush.
    OffsetMode,

    // --- Curve ---
    /// Absolute bearing, or relative to the path tangent.
    CurveDirectionMode,
    /// Whether the path's segments are straight or cubic Béziers.
    ///
    /// Implied by the geometry — a node with handles is a Bézier control —
    /// but carried as a property too, because it is what the *tool* was set to
    /// when the gesture was drawn and what the option bar has to remember.
    /// Creation-only for the same reason [`Self::ShapeSource`] is.
    CurveKind,

    // --- Mask ---
    /// Whether the mask covers everything but its footprint.
    Invert,

    // --- Modifiers (spec.md 6.3) ---
    /// How much a modifier changes the speed beneath it, in percent.
    Gain,
    /// How strongly a modifier bends the field beneath it outward, in percent
    /// of the local speed. Negative converges.
    Radial,
    /// How far a modifier turns the field beneath it, in degrees clockwise.
    ///
    /// Its own id rather than a share of [`Self::RotationDeg`], which every
    /// object has and which turns the object's *frame*. Turning the frame of a
    /// disc does nothing at all; this turns the vectors.
    TurnDeg,
    /// Whether a warp pushes the field to a place or twists it.
    WarpMode,
    /// Where a warp drags the field under its anchor to.
    ///
    /// Its own id rather than a share of [`Self::Target`], which is the point
    /// an *aimed direction mode* points every vector at. Both are positions and
    /// both are placed by pointing, but they answer different questions, and an
    /// id shared between two questions is a trap for the first piece of code
    /// that reads it without checking the tool (compare [`Self::BrushShape`]).
    PushTo,
    /// How far a warp twists the field about its anchor, in degrees.
    TwistDeg,
    /// How much of the pointer's own movement a liquify stroke applies, in
    /// percent (spec.md 6.3, M17). At 100 the field is dragged as far as the
    /// hand moved; at 50, half as far.
    Strength,
    // Where `distance_km` and `push_bearing` were, before a warp pushed to a
    // *place* (spec.md 6.3). The ids are gone with the table rows: a property
    // no tool declares can still be held by an object from an older file, and
    // `value_at` returns whatever the map holds — so the migration removes the
    // entries, and removing the ids is what stops anything reading them again
    // (the same reasoning as D38, schema version 10).
}

/// A unit, for display and for the inspector's suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// Dimensionless.
    None,
    /// Stored in metres per second, always displayed in knots
    /// (`crate::units`).
    Speed,
    /// Kilometres. Pixel input resolves to this at creation (spec.md 3.5).
    Kilometres,
    /// Degrees. A geometric bearing: an object's rotation, a gradient's axis.
    Degrees,
    /// Degrees, and a **flow** direction.
    ///
    /// Stored as an azimuth-toward like every other direction (spec.md 3.3),
    /// and displayed in the project's convention — which for a "from" project
    /// is the reciprocal. The distinction from [`Self::Degrees`] is what stops
    /// an object's *rotation* being flipped by the same conversion.
    Direction,
    /// Percent.
    Percent,
}

/// A default value, in a form that can be written in a `const` table.
///
/// [`PropValue`] cannot be built in const context because `Angle` and `LonLat`
/// normalise on construction, so the table stores the raw numbers instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropDefault {
    /// A scalar default.
    F32(f32),
    /// A boolean default.
    Bool(bool),
    /// An angle in degrees, normalised on use.
    Angle(f64),
    /// A position as (lon, lat).
    LonLat(f64, f64),
    /// An index into [`PropSpec::variants`].
    Enum(u8),
}

impl PropDefault {
    /// Builds the runtime value.
    pub fn value(self) -> PropValue {
        match self {
            Self::F32(v) => PropValue::F32(v),
            Self::Bool(v) => PropValue::Bool(v),
            Self::Angle(d) => PropValue::Angle(Angle::new(d)),
            // Latitude is clamped rather than validated: these are compile-time
            // constants in this file, so an out-of-range value is a typo here,
            // not user input, and clamping keeps the accessor infallible.
            Self::LonLat(lon, lat) => PropValue::LonLat(LonLat {
                lon: crate::geo::normalize_lon(lon),
                lat: lat.clamp(-90.0, 90.0),
            }),
            Self::Enum(v) => PropValue::Enum(v),
        }
    }

    /// The kind this default produces.
    pub fn kind(self) -> PropKind {
        match self {
            Self::F32(_) => PropKind::F32,
            Self::Bool(_) => PropKind::Bool,
            Self::Angle(_) => PropKind::Angle,
            Self::LonLat(_, _) => PropKind::LonLat,
            Self::Enum(_) => PropKind::Enum,
        }
    }
}

/// Everything the UI and the evaluator need to know about one property.
#[derive(Debug, Clone, Copy)]
pub struct PropSpec {
    /// Identity.
    pub id: PropId,
    /// Inspector label.
    pub label: &'static str,
    /// Value used when the object is created.
    pub default: PropDefault,
    /// Inclusive bounds for numeric properties, for slider ranges and clamping.
    pub range: Option<(f32, f32)>,
    /// Display unit.
    pub unit: Unit,
    /// Variant names for enum properties; empty otherwise.
    pub variants: &'static [&'static str],
    /// Whether the property is fixed once the object exists.
    ///
    /// Set from the tool's options at creation and not editable afterwards, so
    /// the inspector does not offer it and a write is refused. Some options
    /// describe *what the object is* rather than a parameter of it — a brush
    /// stroke's stamp is part of the geometry it painted — and changing one
    /// after the fact re-makes the object into something the user did not
    /// draw. Getting a different one means painting a different stroke.
    ///
    /// A creation-only property is not animatable either: a keyframe is an edit
    /// spread over time, and the two rules would otherwise contradict.
    pub creation_only: bool,
    /// Edited by a centred slider rather than a typed number (M29): a signed
    /// amount whose zero is "do nothing" and whose two ends mean opposite
    /// things. The labels name the ends; `reversed` puts the positive end on
    /// the left, for a tool whose positive sense reads as the left-hand word.
    pub slider: Option<Slider>,
}

/// How a centred slider is labelled (M29).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slider {
    /// The label at the slider's left end.
    pub low_label: &'static str,
    /// And at its right end.
    pub high_label: &'static str,
    /// Whether the stored value's positive end is the *left* one.
    pub reversed: bool,
}

impl PropSpec {
    /// The kind of value this property holds.
    pub fn kind(self) -> PropKind {
        self.default.kind()
    }
}

// --- Table constructors ------------------------------------------------------
// One line per property at the call sites below. That is the whole point.

const fn num(
    id: PropId,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    unit: Unit,
) -> PropSpec {
    PropSpec {
        id,
        label,
        default: PropDefault::F32(default),
        range: Some((min, max)),
        unit,
        variants: &[],
        creation_only: false,
        slider: None,
    }
}

const fn ang(id: PropId, label: &'static str, default: f64) -> PropSpec {
    PropSpec {
        id,
        label,
        default: PropDefault::Angle(default),
        range: Some((0.0, 360.0)),
        unit: Unit::Degrees,
        variants: &[],
        creation_only: false,
        slider: None,
    }
}

/// Gives a signed amount a centred slider (M29): zero in the middle, the
/// ends named, and the positive sense on the left when `reversed`.
const fn slider(
    spec: PropSpec,
    low_label: &'static str,
    high_label: &'static str,
    reversed: bool,
) -> PropSpec {
    PropSpec {
        slider: Some(Slider {
            low_label,
            high_label,
            reversed,
        }),
        ..spec
    }
}

/// Marks a property as fixed once the object exists.
///
/// Wraps a constructor rather than adding an argument to each, so the tables
/// read as "this one is frozen" at the point it matters.
const fn frozen(spec: PropSpec) -> PropSpec {
    PropSpec {
        creation_only: true,
        ..spec
    }
}

/// An angle that is a flow direction, displayed in the project's convention.
const fn dir(id: PropId, label: &'static str, default: f64) -> PropSpec {
    PropSpec {
        unit: Unit::Direction,
        ..ang(id, label, default)
    }
}

const fn pos(id: PropId, label: &'static str) -> PropSpec {
    PropSpec {
        id,
        label,
        default: PropDefault::LonLat(0.0, 0.0),
        range: None,
        unit: Unit::None,
        variants: &[],
        creation_only: false,
        slider: None,
    }
}

const fn flag(id: PropId, label: &'static str, default: bool) -> PropSpec {
    PropSpec {
        id,
        label,
        default: PropDefault::Bool(default),
        range: None,
        unit: Unit::None,
        variants: &[],
        creation_only: false,
        slider: None,
    }
}

const fn choice(
    id: PropId,
    label: &'static str,
    default: u8,
    variants: &'static [&'static str],
) -> PropSpec {
    PropSpec {
        id,
        label,
        default: PropDefault::Enum(default),
        range: None,
        unit: Unit::None,
        variants,
        creation_only: false,
        slider: None,
    }
}

// --- Shared variant sets -----------------------------------------------------

/// Variants of [`PropId::DirectionMode`].
///
/// Appended to rather than reordered: the stored value is the index, so moving
/// one would silently change what every existing object aims at.
pub const DIRECTION_MODES: &[&str] = &["constant", "toward_point", "away_from_point"];
/// Variants of [`PropId::EdgeMode`]. Index 0 is the default (decision D12).
pub const EDGE_MODES: &[&str] = &["blend", "replace"];
/// Variants of [`PropId::BrushShape`].
pub const BRUSH_SHAPES: &[&str] = &["circle", "square"];
/// Variants of [`PropId::StampSpace`]. Index 0 keeps existing objects on the
/// ground, which is what they were painted as.
pub const STAMP_SPACES: &[&str] = &["geodesic", "projected"];
/// Variants of [`PropId::FillMode`].
pub const FILL_MODES: &[&str] = &["filled", "perimeter", "filled_gradient"];
/// Variants of [`PropId::RotationSense`].
pub const ROTATION_SENSES: &[&str] = &["cw", "ccw"];
/// Variants of [`PropId::VectorMode`].
pub const VECTOR_MODES: &[&str] = &["constant", "gradient"];
/// Variants of [`PropId::ShapeSource`]. Index 0 is the freehand polygon, which
/// is the only one with no size to drag out.
pub const SHAPE_SOURCES: &[&str] = &["polygon", "square", "rectangle", "circle"];
/// Variants of [`PropId::Resample`] (spec.md 8.7).
pub const RESAMPLE_MODES: &[&str] = &["hold", "interpolate"];
/// Variants of [`PropId::CurveKind`].
pub const CURVE_KINDS: &[&str] = &["polyline", "bezier"];
/// Variants of [`PropId::OffsetMode`].
pub const OFFSET_MODES: &[&str] = &["aligned", "fixed"];
/// Variants of [`PropId::CurveDirectionMode`]. Renamed in place, not reordered:
/// index 0 was called `absolute` and is the same fixed bearing every other tool
/// calls `constant`, which is what a reader of two option bars expects.
pub const CURVE_DIRECTION_MODES: &[&str] = &["constant", "relative_to_path"];
/// Variants of [`PropId::WarpMode`].
pub const WARP_MODES: &[&str] = &["push", "twist"];

// --- The tables --------------------------------------------------------------

/// Properties every object carries, regardless of tool (spec.md 4.4).
pub const COMMON: &[PropSpec] = &[
    pos(PropId::Position, "Position"),
    num(
        PropId::ScalePct,
        "Scale",
        100.0,
        1.0,
        10_000.0,
        Unit::Percent,
    ),
    ang(PropId::RotationDeg, "Rotation", 0.0),
    flag(PropId::Enabled, "Enabled", true),
    choice(PropId::EdgeMode, "Edge", 0, EDGE_MODES),
];

/// [`COMMON`] without the edge mode, for the modifiers (spec.md 6.3).
///
/// `edge_mode` chooses between blending an object's own field into what is
/// beneath and replacing it. A modifier has no field of its own — its output
/// *is* what was beneath, changed — so "replace" has nothing to name, and an
/// option that cannot do anything is worse than an absent one (D30).
const COMMON_MODIFIER: &[PropSpec] = &[
    pos(PropId::Position, "Position"),
    num(
        PropId::ScalePct,
        "Scale",
        100.0,
        1.0,
        10_000.0,
        Unit::Percent,
    ),
    ang(PropId::RotationDeg, "Rotation", 0.0),
    flag(PropId::Enabled, "Enabled", true),
];

/// The size and space every modifier shares.
///
/// They are painted, like the brush and the mask: a stamp swept along a
/// polyline, so a swathe of the field can be intensified or turned in one
/// gesture and two strokes of the same settings merge into one object
/// (spec.md 6.1). They ask the same question about their size that every sized
/// tool asks: px is a footprint that keeps its shape on the map, km one that
/// keeps it on the ground (spec.md 3.5).
macro_rules! modifier_stamp {
    () => {
        frozen(choice(PropId::StampSpace, "Stamp space", 0, STAMP_SPACES))
    };
}

/// The stamp a modifier sweeps, in the same terms the brush and the mask use.
macro_rules! modifier_size {
    () => {
        num(
            PropId::SizeKm,
            "Size",
            1_500.0,
            1.0,
            20_000.0,
            Unit::Kilometres,
        )
    };
}

const INTENSITY: &[PropSpec] = &[
    modifier_stamp!(),
    modifier_size!(),
    // Signed, so one tool intensifies and reduces: +100% doubles the speed
    // beneath, -100% takes it to calm. The floor is exactly -100 because a
    // negative speed is a reversed vector, which is what the turn tool is for.
    // A centred slider from -100% to +200% with 0 — do nothing — in the
    // middle, and 0 as the default so the thumb starts there (M29).
    slider(
        num(PropId::Gain, "Amount", 0.0, -100.0, 200.0, Unit::Percent),
        "−100%",
        "+200%",
        false,
    ),
    num(PropId::Feather, "Feather", 0.5, 0.0, 1.0, Unit::None),
];

const DIVERGENCE: &[PropSpec] = &[
    modifier_stamp!(),
    modifier_size!(),
    // Signed: positive diverges (outward), negative converges.
    num(PropId::Radial, "Amount", 50.0, -400.0, 400.0, Unit::Percent),
    num(PropId::Feather, "Feather", 0.5, 0.0, 1.0, Unit::None),
];

const TURN: &[PropSpec] = &[
    modifier_stamp!(),
    modifier_size!(),
    // A number of degrees rather than an `Angle`, because it is an amount and
    // not a bearing: it is signed, it may exceed a turn, and animating it from
    // -170 to 170 should unwind through zero rather than take the short way
    // round as a bearing would (spec.md 3.3).
    num(PropId::TurnDeg, "Turn", 30.0, -180.0, 180.0, Unit::Degrees),
    num(PropId::Feather, "Feather", 0.5, 0.0, 1.0, Unit::None),
];

const LIQUIFY: &[PropSpec] = &[
    modifier_stamp!(),
    modifier_size!(),
    num(
        PropId::Strength,
        "Strength",
        100.0,
        0.0,
        100.0,
        Unit::Percent,
    ),
    num(PropId::Feather, "Feather", 0.5, 0.0, 1.0, Unit::None),
];

const WARP: &[PropSpec] = &[
    modifier_stamp!(),
    modifier_size!(),
    choice(PropId::WarpMode, "Warp", 0, WARP_MODES),
    // Where the field under the anchor is dragged to. A *position*, not a
    // distance and a bearing: a warp is set by pulling the field where you want
    // it (shift-drag, spec.md 6.3), and the two ends of that pull are the
    // object's own anchor and this — both animatable, so a warp that grows or
    // travels is two keyframed points and nothing else.
    pos(PropId::PushTo, "Push to"),
    num(
        PropId::TwistDeg,
        "Twist",
        60.0,
        -360.0,
        360.0,
        Unit::Degrees,
    ),
    num(PropId::Feather, "Feather", 0.5, 0.0, 1.0, Unit::None),
];

const BRUSH: &[PropSpec] = &[
    // The stamp is part of the geometry the stroke painted: swapping it after
    // the fact re-rasterises the object into one nobody drew.
    frozen(choice(PropId::BrushShape, "Brush shape", 0, BRUSH_SHAPES)),
    // Also the stamp's geometry: swapping the space re-rasterises the stroke
    // into a different shape on the ground, exactly as the shape itself does.
    frozen(choice(PropId::StampSpace, "Stamp space", 0, STAMP_SPACES)),
    num(
        PropId::SizeKm,
        "Size",
        500.0,
        1.0,
        20_000.0,
        Unit::Kilometres,
    ),
    num(PropId::Speed, "Speed", 10.0, 0.0, 120.0, Unit::Speed),
    choice(PropId::DirectionMode, "Direction mode", 0, DIRECTION_MODES),
    dir(PropId::Direction, "Direction", 0.0),
    pos(PropId::Target, "Target"),
    num(PropId::Feather, "Feather", 0.2, 0.0, 1.0, Unit::None),
    // No `divergence` or `curl`: the brush paints a flow along a stroke, and a
    // radial or rotational component belongs to the tools that have a centre to
    // define it about. The other tools keep theirs (spec.md 7.5).
];

const CIRCLE: &[PropSpec] = &[
    choice(PropId::FillMode, "Fill", 0, FILL_MODES),
    num(
        PropId::RingWidthKm,
        "Ring width",
        100.0,
        1.0,
        5_000.0,
        Unit::Kilometres,
    ),
    num(
        PropId::DiameterKm,
        "Diameter",
        1_000.0,
        1.0,
        20_000.0,
        Unit::Kilometres,
    ),
    num(PropId::Speed, "Speed", 15.0, 0.0, 120.0, Unit::Speed),
    num(
        PropId::SpeedMin,
        "Speed (centre)",
        0.0,
        0.0,
        120.0,
        Unit::Speed,
    ),
    num(
        PropId::SpeedMax,
        "Speed (edge)",
        25.0,
        0.0,
        120.0,
        Unit::Speed,
    ),
    choice(PropId::RotationSense, "Rotation", 0, ROTATION_SENSES),
    // The same property the brush uses, deliberately: "px" must mean one
    // thing across the catalogue (spec.md 3.5, 6.1).
    frozen(choice(PropId::StampSpace, "Stamp space", 0, STAMP_SPACES)),
    num(PropId::Feather, "Feather", 0.2, 0.0, 1.0, Unit::None),
    // No `divergence` or `curl`: no tool has them (spec.md 6.2, 7.5). A circle
    // turns about its centre and that is the whole of its flow.
];

const SHAPE_FILL: &[PropSpec] = &[
    // Which geometry the object is. Frozen for the same reason the brush's
    // stamp is: a polygon's vertices and a dragged-out rectangle's extents are
    // not convertible into one another.
    frozen(choice(PropId::ShapeSource, "Shape", 0, SHAPE_SOURCES)),
    // Applies to the presets, which have a size dragged out on the map. A
    // freehand polygon's vertices are placed geographically one by one, so
    // there is no size for a space to interpret and `DEPENDENCIES` hides it.
    frozen(choice(PropId::StampSpace, "Stamp space", 0, STAMP_SPACES)),
    choice(PropId::VectorMode, "Vector mode", 0, VECTOR_MODES),
    num(PropId::Speed, "Speed", 10.0, 0.0, 120.0, Unit::Speed),
    num(
        PropId::SpeedStart,
        "Speed (start)",
        5.0,
        0.0,
        120.0,
        Unit::Speed,
    ),
    num(
        PropId::SpeedEnd,
        "Speed (end)",
        20.0,
        0.0,
        120.0,
        Unit::Speed,
    ),
    dir(PropId::DirectionStart, "Direction (start)", 0.0),
    dir(PropId::DirectionEnd, "Direction (end)", 90.0),
    ang(PropId::GradientAxis, "Gradient axis", 90.0),
    choice(PropId::DirectionMode, "Direction mode", 0, DIRECTION_MODES),
    dir(PropId::Direction, "Direction", 0.0),
    pos(PropId::Target, "Target"),
    num(PropId::Feather, "Feather", 0.1, 0.0, 1.0, Unit::None),
];

// "Identical interaction to the brush" is binding (spec.md 6.2): the mask
// sweeps the same stamp along the same kind of polyline, so it carries the same
// stamp properties. Anything else and a px-sized mask would cover an ellipse
// over a stroke that is a circle on the map.
/// The patch (spec.md 8.5, M14).
///
/// It paints a *captured* field, so it has no speed and no direction of its
/// own — the samples are the field, and there is nothing for a bar to set.
/// What is left is the edge: how the patch meets what is beneath it, which is
/// exactly the question every other tool answers with these two properties.
///
/// Its shape came from the region it was captured over, so the stamp space is
/// frozen at `projected` (D28, D55) and its size is the geometry rather than a
/// number anyone typed.
/// The macro (spec.md 8.7, M16).
///
/// A captured field with the two questions a *run* of frames raises that a
/// single one does not: how its frames land on the project's steps, and what
/// happens after the last of them. Both per object rather than per library
/// entry, because the same macro dropped into a 1-hourly and a 6-hourly
/// project wants different answers.
const MACRO: &[PropSpec] = &[
    frozen(choice(PropId::StampSpace, "Stamp space", 1, STAMP_SPACES)),
    choice(PropId::Resample, "Between frames", 0, RESAMPLE_MODES),
    flag(PropId::LoopMacro, "Loop", false),
    num(PropId::Feather, "Feather", 0.0, 0.0, 1.0, Unit::None),
];

/// Nothing depends on anything: `loop` is asked whichever resample is chosen,
/// because the two are different questions — one is what happens *between*
/// frames and the other what happens *after* them.
const MACRO_DEPENDENCIES: &[Dependency] = &[];

const PATCH: &[PropSpec] = &[
    frozen(choice(PropId::StampSpace, "Stamp space", 1, STAMP_SPACES)),
    num(PropId::Feather, "Feather", 0.0, 0.0, 1.0, Unit::None),
];

const MASK: &[PropSpec] = &[
    frozen(choice(PropId::BrushShape, "Brush shape", 0, BRUSH_SHAPES)),
    frozen(choice(PropId::StampSpace, "Stamp space", 0, STAMP_SPACES)),
    num(
        PropId::SizeKm,
        "Size",
        500.0,
        1.0,
        20_000.0,
        Unit::Kilometres,
    ),
    num(PropId::Feather, "Feather", 0.2, 0.0, 1.0, Unit::None),
    // Which side of the footprint is masked. Off, the mask covers what it is
    // drawn over; on, it covers everything *except* that — which is how a
    // field is confined to a region rather than cut out of one (spec.md 6.2).
    flag(PropId::Invert, "Invert", false),
];

const CLONE_STAMP: &[PropSpec] = &[
    frozen(choice(PropId::BrushShape, "Brush shape", 0, BRUSH_SHAPES)),
    frozen(choice(PropId::StampSpace, "Stamp space", 0, STAMP_SPACES)),
    num(
        PropId::SizeKm,
        "Size",
        500.0,
        1.0,
        20_000.0,
        Unit::Kilometres,
    ),
    pos(PropId::SourcePoint, "Source"),
    choice(PropId::OffsetMode, "Offset", 0, OFFSET_MODES),
    num(PropId::Feather, "Feather", 0.2, 0.0, 1.0, Unit::None),
];

const CURVE: &[PropSpec] = &[
    frozen(choice(PropId::CurveKind, "Curve", 0, CURVE_KINDS)),
    // The corridor has a width, so it asks the same question every sized tool
    // asks: a width in px is a corridor that keeps its thickness on the map.
    frozen(choice(PropId::StampSpace, "Stamp space", 0, STAMP_SPACES)),
    num(
        PropId::WidthKm,
        "Width",
        300.0,
        1.0,
        20_000.0,
        Unit::Kilometres,
    ),
    num(PropId::Speed, "Speed", 12.0, 0.0, 120.0, Unit::Speed),
    choice(
        PropId::CurveDirectionMode,
        "Direction mode",
        1,
        CURVE_DIRECTION_MODES,
    ),
    // `Degrees` rather than `Direction`, deliberately, and only until the curve
    // tool ships: in `relative_to_path` mode this is an *offset* from the path's
    // tangent, not an azimuth, and showing an offset of 0 as 180 in a "from"
    // project would be nonsense. It is a flow direction in `absolute` mode only,
    // so the honest answer is mode-dependent — like `Direction` on the brush,
    // which `DEPENDENCIES` handles by hiding rather than converting.
    ang(PropId::Direction, "Direction", 0.0),
    num(PropId::Feather, "Feather", 0.3, 0.0, 1.0, Unit::None),
];

/// A property that only does anything when another property has certain values.
///
/// Some options are read in one mode and ignored in the others: a brush aimed
/// at a point never reads its constant bearing, and one on a constant bearing
/// never reads its target. Showing an inert value invites editing it and
/// watching nothing happen, which reads as a broken control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dependency {
    /// The property that may be inert.
    pub prop: PropId,
    /// The choice property that decides.
    pub on: PropId,
    /// The values of `on` for which `prop` is read.
    pub live_for: &'static [u8],
}

const fn dep(prop: PropId, on: PropId, live_for: &'static [u8]) -> Dependency {
    Dependency { prop, on, live_for }
}

/// Which of the brush's properties depend on another (spec.md 6.2).
///
/// Scoped per tool rather than by property id alone: the curve reads
/// [`PropId::Direction`] in both of *its* modes, and a global rule keyed on the
/// property would hide it there for a reason that has nothing to do with it.
const BRUSH_DEPENDENCIES: &[Dependency] = &[
    // Mode 0 is the constant bearing; 1 and 2 aim at a target instead.
    dep(PropId::Direction, PropId::DirectionMode, &[0]),
    dep(PropId::Target, PropId::DirectionMode, &[1, 2]),
];

/// Which of the circle stamp's properties depend on its fill mode.
///
/// Fill modes are `filled`, `perimeter`, `filled_gradient` in that order. A
/// filled disc has no ring to be thick, and a gradient reads its two ends
/// rather than the single speed — so the panel offering a ring width for a
/// filled disc is offering a control that does nothing.
const CIRCLE_DEPENDENCIES: &[Dependency] = &[
    dep(PropId::RingWidthKm, PropId::FillMode, &[1]),
    dep(PropId::Speed, PropId::FillMode, &[0, 1]),
    dep(PropId::SpeedMin, PropId::FillMode, &[2]),
    dep(PropId::SpeedMax, PropId::FillMode, &[2]),
];

/// Which of the shape fill's properties depend on another.
///
/// Two axes cross here. `vector_mode` chooses between one vector everywhere and
/// a ramp across the shape, and the constant branch then has the brush's three
/// aim modes inside it — so `direction` is live only when the object is both
/// constant *and* on a fixed bearing. [`is_live`] ands the rules together,
/// which is what lets that be written as two independent lines.
const SHAPE_FILL_DEPENDENCIES: &[Dependency] = &[
    // Vector mode 0 is the constant vector; 1 is the gradient.
    dep(PropId::Speed, PropId::VectorMode, &[0]),
    dep(PropId::DirectionMode, PropId::VectorMode, &[0]),
    dep(PropId::Direction, PropId::VectorMode, &[0]),
    dep(PropId::Target, PropId::VectorMode, &[0]),
    dep(PropId::SpeedStart, PropId::VectorMode, &[1]),
    dep(PropId::SpeedEnd, PropId::VectorMode, &[1]),
    dep(PropId::DirectionStart, PropId::VectorMode, &[1]),
    dep(PropId::DirectionEnd, PropId::VectorMode, &[1]),
    dep(PropId::GradientAxis, PropId::VectorMode, &[1]),
    // ...and inside the constant branch, the same rule the brush has.
    dep(PropId::Direction, PropId::DirectionMode, &[0]),
    dep(PropId::Target, PropId::DirectionMode, &[1, 2]),
    // Shape source 0 is the freehand polygon, whose vertices are placed
    // geographically one at a time. It has no size, so no space to size it in.
    dep(PropId::StampSpace, PropId::ShapeSource, &[1, 2, 3]),
];

/// The dependencies among `tool`'s properties.
///
/// The mask has none — every option it has is read in every mode — and
/// neither does the clone stamp: both of its offset modes read `source_point`,
/// one as a fixed sample centre and the other as the origin of the offset. The
/// curve reads `direction` in both of its modes too, which is exactly why the
/// rule is scoped per tool rather than keyed on the property alone.
pub fn dependencies(tool: ToolKind) -> &'static [Dependency] {
    match tool {
        ToolKind::Brush => BRUSH_DEPENDENCIES,
        ToolKind::Circle => CIRCLE_DEPENDENCIES,
        ToolKind::ShapeFill => SHAPE_FILL_DEPENDENCIES,
        ToolKind::Warp => WARP_DEPENDENCIES,
        ToolKind::Mask
        | ToolKind::CloneStamp
        | ToolKind::Curve
        | ToolKind::Intensity
        | ToolKind::Divergence
        | ToolKind::Turn
        | ToolKind::Liquify
        | ToolKind::Patch => &[],
        ToolKind::Macro => MACRO_DEPENDENCIES,
    }
}

/// Whether `id` is read at all, given how `choice_of` resolves the mode it
/// depends on. Properties with no dependency are always live.
pub fn is_live(tool: ToolKind, id: PropId, choice_of: impl Fn(PropId) -> u8) -> bool {
    dependencies(tool)
        .iter()
        .filter(|d| d.prop == id)
        .all(|d| d.live_for.contains(&choice_of(d.on)))
}

/// A warp's two modes read different options: one pushes the field to a place,
/// the other twists it about the anchor, and neither reads the other's.
const WARP_DEPENDENCIES: &[Dependency] = &[
    dep(PropId::PushTo, PropId::WarpMode, &[0]),
    dep(PropId::TwistDeg, PropId::WarpMode, &[1]),
];

/// The pair of properties an eyedropper writes, and when it is offered.
///
/// A tool that paints a *constant* vector has one speed and one bearing, and
/// both can be read off the field the user is looking at. A tool in a gradient
/// mode has two of each and no single answer, and a curve aiming relative to
/// its own path has a bearing that is an offset rather than a direction — so
/// the eyedropper is offered exactly where those two properties mean what the
/// map is showing (spec.md 6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Eyedropper {
    /// The speed property it writes, in m/s.
    pub speed: PropId,
    /// The direction property it writes, as an azimuth-toward.
    pub direction: PropId,
    /// Conditions beyond those the two properties already carry.
    ///
    /// Empty for the brush and the shape fill, whose own dependencies already
    /// say when a constant vector is what the tool paints.
    pub only_when: &'static [Dependency],
}

/// The curve reads `direction` in both of its modes, so the rule that the
/// eyedropper is for a *bearing* and not an offset has to be stated here.
const CURVE_EYEDROPPER: &[Dependency] = &[dep(PropId::Direction, PropId::CurveDirectionMode, &[0])];

/// The eyedropper `tool` offers, if it paints a constant vector at all.
///
/// Not the mask (no field of its own), not the clone stamp (its field is what
/// it reads), not the circle (its flow is tangential and has no bearing to
/// take), and not a modifier (spec.md 6.3).
pub fn eyedropper(tool: ToolKind) -> Option<Eyedropper> {
    match tool {
        ToolKind::Brush | ToolKind::ShapeFill => Some(Eyedropper {
            speed: PropId::Speed,
            direction: PropId::Direction,
            only_when: &[],
        }),
        ToolKind::Curve => Some(Eyedropper {
            speed: PropId::Speed,
            direction: PropId::Direction,
            only_when: CURVE_EYEDROPPER,
        }),
        _ => None,
    }
}

/// The tool-specific properties for `tool`, excluding the common ones.
pub fn tool_specs(tool: ToolKind) -> &'static [PropSpec] {
    match tool {
        ToolKind::Brush => BRUSH,
        ToolKind::Circle => CIRCLE,
        ToolKind::ShapeFill => SHAPE_FILL,
        ToolKind::Mask => MASK,
        ToolKind::CloneStamp => CLONE_STAMP,
        ToolKind::Curve => CURVE,
        ToolKind::Intensity => INTENSITY,
        ToolKind::Divergence => DIVERGENCE,
        ToolKind::Turn => TURN,
        ToolKind::Warp => WARP,
        ToolKind::Liquify => LIQUIFY,
        ToolKind::Patch => PATCH,
        ToolKind::Macro => MACRO,
    }
}

/// The properties `tool` carries regardless of what it draws.
///
/// [`COMMON`] for everything that paints a field; [`COMMON_MODIFIER`] — the
/// same without `edge_mode` — for the modifiers (spec.md 6.3).
pub fn common_specs(tool: ToolKind) -> &'static [PropSpec] {
    if tool.is_modifier() {
        COMMON_MODIFIER
    } else {
        COMMON
    }
}

/// Every property of `tool`, common first then tool-specific.
pub fn all_specs(tool: ToolKind) -> impl Iterator<Item = &'static PropSpec> {
    common_specs(tool).iter().chain(tool_specs(tool))
}

/// Looks up one property's spec for a tool.
pub fn spec_for(tool: ToolKind, id: PropId) -> Option<&'static PropSpec> {
    all_specs(tool).find(|s| s.id == id)
}

/// An object's properties, keyed by id.
///
/// `BTreeMap` rather than a hash map: iteration order is [`PropId`] declaration
/// order, which makes serialisation canonical without any extra sorting step
/// (invariant 4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PropertyMap(BTreeMap<PropId, Animatable>);

impl PropertyMap {
    /// Builds a map populated with every default for `tool`.
    pub fn for_tool(tool: ToolKind) -> Self {
        Self(
            all_specs(tool)
                .map(|s| (s.id, Animatable::constant(s.default.value())))
                .collect(),
        )
    }

    /// The property, if present.
    pub fn get(&self, id: PropId) -> Option<&Animatable> {
        self.0.get(&id)
    }

    /// The property for in-place editing, if present.
    pub fn get_mut(&mut self, id: PropId) -> Option<&mut Animatable> {
        self.0.get_mut(&id)
    }

    /// Inserts or replaces a property, returning what was there.
    pub fn insert(&mut self, id: PropId, anim: Animatable) -> Option<Animatable> {
        self.0.insert(id, anim)
    }

    /// Removes a property, returning it.
    pub fn remove(&mut self, id: PropId) -> Option<Animatable> {
        self.0.remove(&id)
    }

    /// The value of `id` at `step`.
    ///
    /// Falls back to the schema default when the property is absent, which is
    /// how a project saved before a property existed opens without a migration.
    pub fn value_at(&self, tool: ToolKind, id: PropId, step: u32) -> Option<PropValue> {
        match self.0.get(&id) {
            Some(anim) => Some(anim.value_at(step)),
            None => spec_for(tool, id).map(|s| s.default.value()),
        }
    }

    /// Iterates properties for in-place editing, in canonical order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&PropId, &mut Animatable)> {
        self.0.iter_mut()
    }

    /// Iterates properties in canonical order.
    pub fn iter(&self) -> impl Iterator<Item = (&PropId, &Animatable)> {
        self.0.iter()
    }

    /// How many properties are present.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Adds any property the schema defines but this object lacks.
    ///
    /// Called after loading an older project: a property added in a later
    /// version simply appears at its default, with no bespoke migration step.
    pub fn backfill(&mut self, tool: ToolKind) -> usize {
        let mut added = 0;
        for spec in all_specs(tool) {
            self.0.entry(spec.id).or_insert_with(|| {
                added += 1;
                Animatable::constant(spec.default.value())
            });
        }
        added
    }

    /// Drops keyframes past `last_step` across every property.
    ///
    /// Returns the number removed, for the shrink confirmation (decision D13).
    pub fn truncate_to(&mut self, last_step: u32) -> usize {
        self.0.values_mut().map(|a| a.truncate_to(last_step)).sum()
    }

    /// Counts keyframes past `last_step` without removing them.
    pub fn count_after(&self, last_step: u32) -> usize {
        self.0.values().map(|a| a.count_after(last_step)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Interpolation;

    /// The tables are hand-written, so the realistic failure is a typo in one
    /// row. These walk every property of every tool and check the row is
    /// internally consistent.
    #[test]
    fn every_spec_is_self_consistent() {
        for tool in ToolKind::ALL {
            for spec in all_specs(tool) {
                let ctx = format!("{tool:?}/{:?}", spec.id);
                assert!(!spec.label.is_empty(), "{ctx}: empty label");
                assert_eq!(
                    spec.default.value().kind(),
                    spec.kind(),
                    "{ctx}: kind mismatch"
                );
                assert!(
                    spec.default.value().is_finite(),
                    "{ctx}: non-finite default"
                );

                if spec.kind() == PropKind::Enum {
                    assert!(!spec.variants.is_empty(), "{ctx}: enum with no variants");
                    let PropDefault::Enum(idx) = spec.default else {
                        panic!("{ctx}: enum kind without an enum default");
                    };
                    assert!(
                        (idx as usize) < spec.variants.len(),
                        "{ctx}: default {idx} is outside {} variants",
                        spec.variants.len()
                    );
                } else {
                    assert!(spec.variants.is_empty(), "{ctx}: non-enum carries variants");
                }

                if let (Some((min, max)), PropDefault::F32(d)) = (spec.range, spec.default) {
                    assert!(min <= max, "{ctx}: inverted range");
                    assert!(
                        (min..=max).contains(&d),
                        "{ctx}: default {d} outside [{min}, {max}]"
                    );
                }
            }
        }
    }

    #[test]
    fn no_tool_declares_a_property_twice() {
        for tool in ToolKind::ALL {
            let mut seen = Vec::new();
            for spec in all_specs(tool) {
                assert!(
                    !seen.contains(&spec.id),
                    "{tool:?} declares {:?} twice",
                    spec.id
                );
                seen.push(spec.id);
            }
        }
    }

    /// Decision D12: blending is the default, so it must be variant 0.
    #[test]
    fn edge_mode_defaults_to_blend() {
        let spec = spec_for(ToolKind::Brush, PropId::EdgeMode).unwrap();
        let PropDefault::Enum(idx) = spec.default else {
            panic!("not an enum")
        };
        assert_eq!(spec.variants[idx as usize], "blend");
    }

    /// The dependency tables are hand-written and name properties by id, so
    /// the realistic failure is a rule about a property the tool does not have,
    /// or one keyed on a variant index that does not exist. Both make an option
    /// silently vanish from the panel rather than crashing.
    #[test]
    fn every_dependency_names_properties_its_tool_actually_has() {
        for tool in ToolKind::ALL {
            for rule in dependencies(tool) {
                let ctx = format!("{tool:?}: {:?} on {:?}", rule.prop, rule.on);
                assert!(
                    spec_for(tool, rule.prop).is_some(),
                    "{ctx}: the dependent property is not one of {tool:?}'s"
                );
                let on = spec_for(tool, rule.on)
                    .unwrap_or_else(|| panic!("{ctx}: the deciding property is not {tool:?}'s"));
                assert_eq!(
                    on.kind(),
                    PropKind::Enum,
                    "{ctx}: only a choice can decide what is live"
                );
                assert!(!rule.live_for.is_empty(), "{ctx}: live for nothing at all");
                for index in rule.live_for {
                    assert!(
                        (*index as usize) < on.variants.len(),
                        "{ctx}: variant {index} is outside {:?}",
                        on.variants
                    );
                }
                assert_ne!(rule.prop, rule.on, "{ctx}: a property cannot hide itself");
            }
        }
    }

    /// A creation-only property is set once and never edited, so a rule that
    /// hides it would hide a value nothing can change anyway — and, worse, one
    /// that the tool's own option bar has to keep offering. The one exception
    /// is a property whose *whole* meaning depends on what the object is: the
    /// shape fill's stamp space, which a freehand polygon has no size for.
    #[test]
    fn a_hidden_creation_only_property_depends_on_another_creation_only_one() {
        for tool in ToolKind::ALL {
            for rule in dependencies(tool) {
                let Some(spec) = spec_for(tool, rule.prop) else {
                    continue;
                };
                if !spec.creation_only {
                    continue;
                }
                let decider = spec_for(tool, rule.on).expect("checked above");
                assert!(
                    decider.creation_only,
                    "{tool:?}: {:?} is frozen but hidden by {:?}, which can change \
                     afterwards — the option would appear and disappear on an \
                     object nobody can edit",
                    rule.prop, rule.on
                );
            }
        }
    }

    /// Every mode of a choice must leave something to edit. A variant that
    /// hides every one of a tool's options is a mode with an empty panel, which
    /// is always a mistake in the table rather than an intended design.
    #[test]
    fn no_mode_hides_every_option_a_tool_has() {
        for tool in ToolKind::ALL {
            let deciders: Vec<PropId> = dependencies(tool).iter().map(|d| d.on).collect();
            for decider in deciders {
                let spec = spec_for(tool, decider).expect("checked above");
                for index in 0..spec.variants.len() as u8 {
                    let live = tool_specs(tool)
                        .iter()
                        .filter(|s| {
                            is_live(tool, s.id, |id| {
                                if id == decider {
                                    index
                                } else {
                                    spec_for(tool, id)
                                        .and_then(|s| match s.default {
                                            PropDefault::Enum(v) => Some(v),
                                            _ => None,
                                        })
                                        .unwrap_or(0)
                                }
                            })
                        })
                        .count();
                    assert!(
                        live > 0,
                        "{tool:?}: {decider:?} = {:?} leaves nothing editable",
                        spec.variants[index as usize]
                    );
                }
            }
        }
    }

    /// Spec 6.1: an option that can be neither set at creation nor edited
    /// afterwards can only ever hold its default, and does not belong on the
    /// tool. Creation-only is therefore a claim about the *tool's* options, and
    /// this is the half of it the schema can check: that such a property is one
    /// of the tool's own rather than inherited from `COMMON`, which the tools
    /// do not offer at creation.
    #[test]
    fn nothing_common_is_creation_only() {
        for spec in COMMON {
            assert!(
                !spec.creation_only,
                "{:?} is common to every tool, so no tool's option bar owns it",
                spec.id
            );
        }
    }

    /// Spec 6.1: a creation-only property is not animatable, because a keyframe
    /// is an edit spread over time and the two rules would contradict. The
    /// document cannot enforce that on its own — `Animatable` holds keys for any
    /// property — so the rule lives at the write path, and this records which
    /// properties it has to cover.
    #[test]
    fn the_frozen_properties_are_the_ones_that_decide_what_an_object_is() {
        let mut frozen: Vec<(ToolKind, PropId)> = Vec::new();
        for tool in ToolKind::ALL {
            for spec in all_specs(tool) {
                if spec.creation_only {
                    frozen.push((tool, spec.id));
                }
            }
        }

        // Named rather than counted: each of these is a claim that the property
        // is part of the geometry the gesture laid down (spec.md 6.1).
        assert_eq!(
            frozen,
            vec![
                (ToolKind::Brush, PropId::BrushShape),
                (ToolKind::Brush, PropId::StampSpace),
                (ToolKind::Circle, PropId::StampSpace),
                (ToolKind::ShapeFill, PropId::ShapeSource),
                (ToolKind::ShapeFill, PropId::StampSpace),
                (ToolKind::CloneStamp, PropId::BrushShape),
                (ToolKind::CloneStamp, PropId::StampSpace),
                (ToolKind::Curve, PropId::CurveKind),
                (ToolKind::Curve, PropId::StampSpace),
                (ToolKind::Mask, PropId::BrushShape),
                (ToolKind::Mask, PropId::StampSpace),
                (ToolKind::Intensity, PropId::StampSpace),
                (ToolKind::Divergence, PropId::StampSpace),
                (ToolKind::Turn, PropId::StampSpace),
                (ToolKind::Warp, PropId::StampSpace),
                (ToolKind::Liquify, PropId::StampSpace),
                // A patch's space came from the region it was captured over,
                // which is map space and cannot be reconsidered afterwards
                // (spec.md 8.5, D55).
                (ToolKind::Patch, PropId::StampSpace),
                // A macro's space came from the region it was captured over,
                // exactly as a patch's did (spec.md 8.7, D55).
                (ToolKind::Macro, PropId::StampSpace),
            ]
        );
    }

    /// Spec 3.5 and 6.1: px selects `projected` for *every* tool with a size,
    /// so every tool that has one must carry the property that records the
    /// choice. Without it the tool's px option would have nowhere to land and
    /// would quietly paint on the ground instead.
    ///
    /// One direction only. The shape fill has a stamp space and no size
    /// property, because its presets are dragged out and carry their extents in
    /// the geometry — but the space still decides whether what was dragged is a
    /// shape on the map or one on the ground.
    #[test]
    fn every_tool_with_a_size_property_carries_a_stamp_space() {
        const SIZES: [PropId; 4] = [
            PropId::SizeKm,
            PropId::DiameterKm,
            PropId::WidthKm,
            PropId::RingWidthKm,
        ];
        for tool in ToolKind::ALL {
            if !tool_specs(tool).iter().any(|s| SIZES.contains(&s.id)) {
                continue;
            }
            assert!(
                spec_for(tool, PropId::StampSpace).is_some(),
                "{tool:?} has a size but no space to size it in"
            );
        }
    }

    /// Spec 6.1: the three shared aim modes mean the same thing for every tool
    /// that has a target, and are the same property with the same variant
    /// indices. A tool that offers `toward_point` without a `target` would aim
    /// at nothing.
    #[test]
    fn every_tool_with_an_aim_mode_has_a_target() {
        for tool in ToolKind::ALL {
            let Some(mode) = spec_for(tool, PropId::DirectionMode) else {
                assert!(
                    spec_for(tool, PropId::Target).is_none(),
                    "{tool:?} has a target but no mode that reads it"
                );
                continue;
            };
            assert_eq!(
                mode.variants, DIRECTION_MODES,
                "{tool:?} redefines the modes"
            );
            assert!(
                spec_for(tool, PropId::Target).is_some(),
                "{tool:?} can aim at a point but has no point to aim at"
            );
        }
    }

    #[test]
    fn every_tool_carries_the_common_properties() {
        for tool in ToolKind::ALL {
            for common in common_specs(tool) {
                assert!(
                    spec_for(tool, common.id).is_some(),
                    "{tool:?} is missing common property {:?}",
                    common.id
                );
            }
        }
    }

    /// Spec 6.3: a modifier has no field of its own, so it has no `edge_mode`
    /// — there is nothing to replace what is beneath *with*. Everything else in
    /// [`COMMON`] it does carry, placement and lifetime alike.
    #[test]
    fn a_modifier_carries_every_common_property_but_the_edge_mode() {
        for tool in ToolKind::ALL.into_iter().filter(|t| t.is_modifier()) {
            assert!(
                spec_for(tool, PropId::EdgeMode).is_none(),
                "{tool:?} offers an edge mode it cannot use"
            );
            for common in COMMON.iter().filter(|s| s.id != PropId::EdgeMode) {
                assert!(
                    spec_for(tool, common.id).is_some(),
                    "{tool:?} is missing common property {:?}",
                    common.id
                );
            }
        }
        // And a tool that paints a field still has one.
        assert!(spec_for(ToolKind::Brush, PropId::EdgeMode).is_some());
    }

    /// Spec 6.3: every modifier is painted and sized the same way, and every
    /// one of them has a feather. Stated as a rule so a fifth modifier cannot
    /// ship with a different set by accident.
    #[test]
    fn every_modifier_is_a_feathered_swept_stamp() {
        for tool in ToolKind::ALL.into_iter().filter(|t| t.is_modifier()) {
            for id in [
                PropId::Position,
                PropId::ScalePct,
                PropId::RotationDeg,
                PropId::Enabled,
                PropId::SizeKm,
                PropId::StampSpace,
                PropId::Feather,
            ] {
                assert!(spec_for(tool, id).is_some(), "{tool:?} is missing {id:?}");
            }
            // And none of them carries a speed or a direction of its own: what
            // it writes is what it read.
            for id in [PropId::Speed, PropId::Direction, PropId::DirectionMode] {
                assert!(
                    spec_for(tool, id).is_none(),
                    "{tool:?} declares {id:?}, but a modifier has no field of its own"
                );
            }
        }
    }

    #[test]
    fn a_new_map_holds_every_default() {
        for tool in ToolKind::ALL {
            let map = PropertyMap::for_tool(tool);
            assert_eq!(map.len(), all_specs(tool).count());
            for spec in all_specs(tool) {
                let anim = map.get(spec.id).expect("property present");
                assert_eq!(anim.base(), spec.default.value());
                assert!(!anim.is_animated(), "a new object must have no keyframes");
            }
        }
    }

    /// A project saved before a property existed must open, with the new
    /// property simply at its default. That is what avoids a bespoke migration
    /// for the common case of adding a property.
    #[test]
    fn a_missing_property_falls_back_to_its_default() {
        let mut map = PropertyMap::for_tool(ToolKind::Brush);
        map.remove(PropId::Feather);
        assert!(map.get(PropId::Feather).is_none());

        let value = map.value_at(ToolKind::Brush, PropId::Feather, 0);
        assert_eq!(value, Some(PropValue::F32(0.2)));

        assert_eq!(map.backfill(ToolKind::Brush), 1);
        assert!(map.get(PropId::Feather).is_some());
        assert_eq!(
            map.backfill(ToolKind::Brush),
            0,
            "backfill must be idempotent"
        );
    }

    #[test]
    fn an_unknown_property_for_a_tool_has_no_value() {
        let map = PropertyMap::for_tool(ToolKind::Mask);
        assert_eq!(map.value_at(ToolKind::Mask, PropId::Speed, 0), None);
    }

    /// Spec 7.5: no tool has a radial or tangential component of its own. The
    /// ids are gone as well as the table rows — a property no tool declares can
    /// still be *held* by an object loaded from an older file, and `value_at`
    /// returns whatever the map holds (D30).
    #[test]
    fn no_tool_shapes_its_flow_with_divergence_or_curl() {
        for tool in ToolKind::ALL {
            for spec in all_specs(tool) {
                let name = format!("{:?}", spec.id);
                assert!(
                    name != "Divergence" && name != "Curl",
                    "{tool:?} still declares {name}"
                );
            }
        }
    }

    #[test]
    fn truncation_spans_every_property() {
        let mut map = PropertyMap::for_tool(ToolKind::Brush);
        for id in [PropId::Speed, PropId::Feather] {
            let anim = map.get_mut(id).expect("present");
            anim.set_key(0, PropValue::F32(1.0), Interpolation::Linear);
            anim.set_key(50, PropValue::F32(2.0), Interpolation::Linear);
            anim.set_key(80, PropValue::F32(3.0), Interpolation::Linear);
        }
        assert_eq!(map.count_after(40), 4);
        assert_eq!(map.truncate_to(40), 4);
        assert_eq!(map.count_after(40), 0);
    }

    /// Iteration order must be declaration order so the JSON is canonical.
    #[test]
    fn iteration_is_in_declaration_order() {
        let map = PropertyMap::for_tool(ToolKind::Brush);
        let ids: Vec<PropId> = map.iter().map(|(id, _)| *id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn property_maps_round_trip_through_json() {
        let mut map = PropertyMap::for_tool(ToolKind::Circle);
        map.get_mut(PropId::Speed).expect("present").set_key(
            3,
            PropValue::F32(22.0),
            Interpolation::EaseOut,
        );

        let json = serde_json::to_string(&map).unwrap();
        assert!(
            json.contains("\"speed\""),
            "ids must serialise as snake_case names: {json}"
        );
        let back: PropertyMap = serde_json::from_str(&json).unwrap();
        assert_eq!(map, back);
    }

    /// Serialising twice must produce identical bytes; that property is what
    /// the save/load/save acceptance test rests on.
    #[test]
    fn serialisation_is_stable_across_runs() {
        let map = PropertyMap::for_tool(ToolKind::ShapeFill);
        let a = serde_json::to_string(&map).unwrap();
        let b = serde_json::to_string(&map).unwrap();
        assert_eq!(a, b);
    }
}
