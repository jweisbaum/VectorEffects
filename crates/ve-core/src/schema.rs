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
    /// Writes zero-speed vectors.
    Eraser,
    /// Samples the composite below it in z-order.
    CloneStamp,
    /// A vector field along a path.
    Curve,
}

impl ToolKind {
    /// Every tool, in palette order.
    pub const ALL: [Self; 6] = [
        Self::Brush,
        Self::Circle,
        Self::ShapeFill,
        Self::Eraser,
        Self::CloneStamp,
        Self::Curve,
    ];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Brush => "Brush",
            Self::Circle => "Circle",
            Self::ShapeFill => "Shape fill",
            Self::Eraser => "Eraser",
            Self::CloneStamp => "Clone stamp",
            Self::Curve => "Curve",
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
    /// Radial component, -1 to 1.
    Divergence,
    /// Tangential component, -1 to 1.
    Curl,

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

    // --- Clone stamp ---
    /// Where the clone samples from.
    SourcePoint,
    /// Whether the source follows the brush.
    OffsetMode,

    // --- Curve ---
    /// Absolute bearing, or relative to the path tangent.
    CurveDirectionMode,
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
/// Variants of [`PropId::OffsetMode`].
pub const OFFSET_MODES: &[&str] = &["aligned", "fixed"];
/// Variants of [`PropId::CurveDirectionMode`].
pub const CURVE_DIRECTION_MODES: &[&str] = &["absolute", "relative_to_path"];

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
    // Neutral by default: the circle's rotation is its direction mode, not a
    // curl term, so a non-zero default here would spiral every new circle.
    num(PropId::Divergence, "Divergence", 0.0, -1.0, 1.0, Unit::None),
    num(PropId::Curl, "Curl", 0.0, -1.0, 1.0, Unit::None),
];

const SHAPE_FILL: &[PropSpec] = &[
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
    num(PropId::Divergence, "Divergence", 0.0, -1.0, 1.0, Unit::None),
    num(PropId::Curl, "Curl", 0.0, -1.0, 1.0, Unit::None),
];

const ERASER: &[PropSpec] = &[
    num(
        PropId::SizeKm,
        "Size",
        500.0,
        1.0,
        20_000.0,
        Unit::Kilometres,
    ),
    num(PropId::Feather, "Feather", 0.2, 0.0, 1.0, Unit::None),
];

const CLONE_STAMP: &[PropSpec] = &[
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

/// The dependencies among `tool`'s properties.
pub fn dependencies(tool: ToolKind) -> &'static [Dependency] {
    match tool {
        ToolKind::Brush => BRUSH_DEPENDENCIES,
        _ => &[],
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

/// The tool-specific properties for `tool`, excluding [`COMMON`].
pub fn tool_specs(tool: ToolKind) -> &'static [PropSpec] {
    match tool {
        ToolKind::Brush => BRUSH,
        ToolKind::Circle => CIRCLE,
        ToolKind::ShapeFill => SHAPE_FILL,
        ToolKind::Eraser => ERASER,
        ToolKind::CloneStamp => CLONE_STAMP,
        ToolKind::Curve => CURVE,
    }
}

/// Every property of `tool`, common first then tool-specific.
pub fn all_specs(tool: ToolKind) -> impl Iterator<Item = &'static PropSpec> {
    COMMON.iter().chain(tool_specs(tool))
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

    #[test]
    fn every_tool_carries_the_common_properties() {
        for tool in ToolKind::ALL {
            for common in COMMON {
                assert!(
                    spec_for(tool, common.id).is_some(),
                    "{tool:?} is missing common property {:?}",
                    common.id
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
        let map = PropertyMap::for_tool(ToolKind::Eraser);
        assert_eq!(map.value_at(ToolKind::Eraser, PropId::Curl, 0), None);
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
