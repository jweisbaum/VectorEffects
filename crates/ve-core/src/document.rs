//! Objects, layers, and the geometry they carry.
//!
//! All geometry is stored in the object's local azimuthal-equidistant frame, in
//! **metres** (spec.md 7.2). Nothing here is in lat/lon degrees, and nothing is
//! in pixels: a pixel-valued size is resolved to kilometres the moment an
//! object is created and never revisited (spec.md 3.5). That is what keeps
//! objects pinned to the earth under pan and zoom.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::id::Id;
use crate::project::FieldKind;
use crate::raster::RasterSequence;
use crate::schema::{PropertyMap, ToolKind};

/// A point in an object's local AEQD frame, in metres from the anchor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LocalPoint {
    /// Eastward offset in metres, before rotation and scale.
    #[serde(with = "crate::canonical::metres_field")]
    pub x: f64,
    /// Northward offset in metres, before rotation and scale.
    #[serde(with = "crate::canonical::metres_field")]
    pub y: f64,
}

impl LocalPoint {
    /// A point at `(x, y)` metres, quantised to canonical precision.
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            x: crate::canonical::metres(x),
            y: crate::canonical::metres(y),
        }
    }

    /// Distance from the anchor, in metres.
    pub fn radius(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// Whether both components are finite.
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// One node of a curve path.
///
/// A node with no handles is a polyline corner; one with handles is a cubic
/// Bézier control. The curve's kind is therefore implied by its geometry rather
/// than stored as a separate property that could contradict it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathNode {
    /// The on-curve point.
    pub point: LocalPoint,
    /// Incoming control handle, if this node is a Bézier control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_handle: Option<LocalPoint>,
    /// Outgoing control handle, if this node is a Bézier control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_handle: Option<LocalPoint>,
}

/// An object's shape, in its local AEQD frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Geometry {
    /// Swept brush stamps: brush, eraser, and clone stamp. The radius comes
    /// from the object's size property, not from the geometry.
    ///
    /// *Chains*, plural, because overlapping strokes with identical properties
    /// merge into one object (spec.md 6.1). They cannot share a single polyline:
    /// joining one stroke's end to the next one's start would sweep the brush
    /// across the gap between them and paint a line that was never drawn.
    Stroke {
        /// One polyline per stroke that has been merged into this object.
        chains: Vec<Vec<LocalPoint>>,
    },
    /// A circle centred on the anchor.
    ///
    /// The radius comes from one of two places, which is what the `Option`
    /// records. The circle *stamp* types a diameter, so its size is the
    /// animatable `diameter_km` property and the geometry carries `None`. The
    /// shape fill's circle preset is *dragged out* on the map, so its size is
    /// part of what the user drew and lives here — the same place a dragged
    /// rectangle's extents live, and resized afterwards by the same scale
    /// handle rather than by a number nobody typed.
    Disc {
        /// Radius in metres, for a circle whose size was dragged out.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::canonical::optional_metres_field"
        )]
        radius_m: Option<f64>,
    },
    /// An arbitrary closed polygon.
    Polygon {
        /// Vertices in order. Implicitly closed.
        points: Vec<LocalPoint>,
    },
    /// An axis-aligned rectangle centred on the anchor, before rotation.
    Rect {
        /// Half-extent along local x, in metres.
        #[serde(with = "crate::canonical::metres_field")]
        half_width_m: f64,
        /// Half-extent along local y, in metres.
        #[serde(with = "crate::canonical::metres_field")]
        half_height_m: f64,
    },
    /// A polyline or Bézier path.
    Path {
        /// Path nodes in order.
        nodes: Vec<PathNode>,
    },
}

impl Geometry {
    /// The default geometry for a tool, used when creating an empty object.
    pub fn default_for(tool: ToolKind) -> Self {
        match tool {
            // Everything painted along a polyline: the brush, the mask, the
            // clone stamp, and the modifiers, which are swept the same way so
            // that two strokes of one merge like two of a brush (spec.md 6.3).
            ToolKind::Brush
            | ToolKind::Mask
            | ToolKind::CloneStamp
            | ToolKind::Intensity
            | ToolKind::Divergence
            | ToolKind::Turn
            | ToolKind::Warp => Self::Stroke { chains: Vec::new() },
            ToolKind::Circle => Self::Disc { radius_m: None },
            // A patch is pasted with the shape it was captured over, which is
            // one of the region's three; an empty polygon is what it has
            // before that shape arrives (spec.md 8.5).
            // A patch is pasted and a macro placed, each with the shape it
            // was captured over; an empty polygon is what they have before
            // that shape arrives (spec.md 8.5, 8.7).
            ToolKind::ShapeFill | ToolKind::Patch | ToolKind::Macro => {
                Self::Polygon { points: Vec::new() }
            }
            ToolKind::Curve => Self::Path { nodes: Vec::new() },
        }
    }

    /// Every point across every chain, for a stroke.
    pub fn stroke_chains(&self) -> &[Vec<LocalPoint>] {
        match self {
            Self::Stroke { chains } => chains,
            _ => &[],
        }
    }

    /// The furthest any geometry point sits from the anchor, in metres.
    ///
    /// The evaluator adds the object's own radius and feather to this to build
    /// the spherical cap it culls against (spec.md 7.3).
    pub fn bounding_radius_m(&self) -> f64 {
        match self {
            // A stamp's radius comes from its property, which the geometry
            // does not see; a dragged-out circle carries its own.
            Self::Disc { radius_m } => radius_m.unwrap_or(0.0),
            Self::Stroke { chains } => chains
                .iter()
                .flatten()
                .map(|p| p.radius())
                .fold(0.0, f64::max),
            Self::Polygon { points } => points.iter().map(|p| p.radius()).fold(0.0, f64::max),
            Self::Rect {
                half_width_m,
                half_height_m,
            } => half_width_m.hypot(*half_height_m),
            Self::Path { nodes } => nodes
                .iter()
                .flat_map(|n| {
                    [Some(n.point), n.in_handle, n.out_handle]
                        .into_iter()
                        .flatten()
                })
                .map(|p| p.radius())
                .fold(0.0, f64::max),
        }
    }

    /// Whether every coordinate is finite, so the object can be saved.
    pub fn is_finite(&self) -> bool {
        match self {
            Self::Disc { radius_m } => radius_m.is_none_or(|r| r.is_finite()),
            Self::Stroke { chains } => chains.iter().flatten().all(|p| p.is_finite()),
            Self::Polygon { points } => points.iter().all(|p| p.is_finite()),
            Self::Rect {
                half_width_m,
                half_height_m,
            } => half_width_m.is_finite() && half_height_m.is_finite(),
            Self::Path { nodes } => nodes.iter().all(|n| {
                n.point.is_finite()
                    && n.in_handle.is_none_or(LocalPoint::is_finite)
                    && n.out_handle.is_none_or(LocalPoint::is_finite)
            }),
        }
    }
}

/// An inclusive range of time steps.
///
/// This is the object's coarse lifetime. The per-step `Enabled` property is the
/// fine-grained switch; both must be satisfied for an object to contribute
/// (spec.md 4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRange {
    /// First step, inclusive.
    pub start: u32,
    /// Last step, inclusive.
    pub end: u32,
}

impl StepRange {
    /// A range covering `start..=end`, ordered so `start <= end`.
    pub fn new(start: u32, end: u32) -> Self {
        if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }

    /// A range covering every step of a project with `step_count` steps.
    pub fn full(step_count: u32) -> Self {
        Self {
            start: 0,
            end: step_count.saturating_sub(1),
        }
    }

    /// Whether `step` falls inside the range.
    pub fn contains(self, step: u32) -> bool {
        step >= self.start && step <= self.end
    }

    /// How many steps the range spans.
    pub fn len(self) -> u32 {
        self.end - self.start + 1
    }

    /// Always false; a range is inclusive and so always covers at least a step.
    pub fn is_empty(self) -> bool {
        false
    }

    /// Clamps the range to fit a project with `last_step` as its final step.
    pub fn clamped_to(self, last_step: u32) -> Self {
        Self {
            start: self.start.min(last_step),
            end: self.end.min(last_step),
        }
    }
}

/// One invocation of a vector-creation tool (spec.md 6.1).
///
/// One gesture makes one object, and the tool's options are frozen onto it at
/// creation. Changing a tool option afterwards affects only later objects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Object {
    /// Stable identity.
    pub id: Id,
    /// User-editable name.
    pub name: String,
    /// Which tool made it.
    pub tool: ToolKind,
    /// Shape, in the object's local AEQD frame.
    pub geometry: Geometry,
    /// Coarse lifetime.
    pub active_range: StepRange,
    /// Every property, animatable.
    pub props: PropertyMap,
    /// The captured field this object replays, by content hash
    /// (spec.md 8.5, M14).
    ///
    /// `Some` only for a patch. The samples themselves are **not** here and
    /// never in the JSON: they live in the project's own `captures/` archive
    /// entry, keyed by this hash, and are loaded into
    /// [`Project::captures`](crate::project::Project::captures) on open. A
    /// patch whose entry is missing draws nothing and is marked, exactly as a
    /// GRIB layer whose file has gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<String>,
    /// Which of the object's own movements reach the field it paints
    /// (spec.md 9.3, M13).
    ///
    /// Off for every object that has not been told otherwise, which is what
    /// every object was before the flags existed, so an old project opens
    /// unchanged and needs no migration.
    #[serde(default, skip_serializing_if = "is_still")]
    pub motion: MotionFlags,
}

/// Whether an object contributes no motion, for skipping the field on save.
fn is_still(motion: &MotionFlags) -> bool {
    !motion.any()
}

impl Object {
    /// A new object of `tool`, with schema defaults and default geometry.
    pub fn new(tool: ToolKind, name: impl Into<String>, step_count: u32) -> Self {
        Self {
            id: Id::new(),
            name: name.into(),
            tool,
            geometry: Geometry::default_for(tool),
            active_range: StepRange::full(step_count),
            props: PropertyMap::for_tool(tool),
            capture: None,
            motion: MotionFlags::default(),
        }
    }

    /// Whether the object contributes anything at `step`.
    ///
    /// Checks the coarse lifetime and the animated `Enabled` switch together.
    pub fn is_active_at(&self, step: u32) -> bool {
        use crate::schema::PropId;
        if !self.active_range.contains(step) {
            return false;
        }
        self.props
            .value_at(self.tool, PropId::Enabled, step)
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }
}

/// An ordered group of objects (spec.md 4.3).
///
/// Layers persist across every time step. The basemap is not a layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Stable identity.
    pub id: Id,
    /// User-editable name.
    pub name: String,
    /// Whether the layer contributes to the field and is drawn.
    pub visible: bool,
    /// Whether the layer's objects can be selected and edited.
    pub locked: bool,
    /// Objects in z-order; index 0 is the bottom.
    pub objects: Vec<Object>,
    /// Where the layer's field comes from besides its objects.
    ///
    /// Absent from the file for an ordinary painted layer, so older projects
    /// open unchanged.
    #[serde(default, skip_serializing_if = "LayerSource::is_painted")]
    pub source: LayerSource,
    /// The decoded field of a [`LayerSource::Grib`] layer.
    ///
    /// **Never serialised** (invariants 1 and 2): the file keeps the path in
    /// `source` and the app re-reads the GRIB when the project opens. `None`
    /// on a GRIB layer means the file could not be read, and the layer then
    /// contributes nothing until it can be. Shared rather than owned because
    /// the document is cloned freely — into history, into render snapshots —
    /// and a decoded field can run to hundreds of megabytes.
    #[serde(skip)]
    pub raster: Option<Arc<RasterSequence>>,
    /// Which speeds of an imported field to keep (spec.md 4.8).
    ///
    /// `None` keeps every sample, which is what a layer has until the user
    /// says otherwise. A range drops the samples outside it — the field beneath
    /// then shows through, exactly as it does where the file has no value at
    /// all — which is how one band of a forecast is isolated: the calms, the
    /// gale, the jet.
    ///
    /// A property of the *layer* rather than of the lattice, because it is a
    /// choice about what to show and not a fact about the file. It therefore
    /// costs nothing on disk beyond two numbers and survives the file being
    /// re-read on open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_range: Option<SpeedRange>,
    /// Which of the file's messages a step shows, where the user has said
    /// (spec.md 4.8, M20).
    ///
    /// A forecast file rarely lines up with a timeline: a 6-hourly file in a
    /// 3-hourly project has a message on every other step, a file ends before
    /// the timeline does, and a message is sometimes simply bad. An override
    /// is the choice a keyframe gives — *this* step shows *that* message — as
    /// an instruction rather than a measurement, which is why it survives
    /// where §4.8's hold rule was removed.
    ///
    /// **A step number, never a sample.** Invariants 1 and 2 stand: what is
    /// stored is which of the file's own frames to serve, so the frame that
    /// reaches the scene is one the file already holds and the render cache,
    /// readiness and both kernels are right without knowing this exists.
    ///
    /// Sorted by step and unique in it, which [`Layer::set_frame_overrides`]
    /// maintains, so the lookup is a binary search.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frame_overrides: Vec<FrameOverride>,
}

/// One step's answer to "which of the file's messages does this show?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameOverride {
    /// The step whose content is being decided.
    pub step: u32,
    /// The step whose message it shows, or `None` to show nothing at all.
    ///
    /// The source is resolved against the **file**, not against another
    /// override, so a chain of them is impossible and the frame served is
    /// always one the file holds.
    pub source: Option<u32>,
}

/// A band of speeds to keep from an imported field, in metres per second.
///
/// `f32` and not `f64`: it is a threshold on a field stored in `f32`, and a
/// `f32` needs no canonical serde helper to survive a round trip
/// (`crate::canonical`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpeedRange {
    /// Slowest speed kept.
    pub min_mps: f32,
    /// Fastest speed kept.
    pub max_mps: f32,
}

impl SpeedRange {
    /// Whether a speed is inside the band.
    ///
    /// Inclusive at both ends: a range set to exactly the speeds on screen
    /// should keep them.
    pub fn keeps(self, speed_mps: f32) -> bool {
        speed_mps >= self.min_mps && speed_mps <= self.max_mps
    }
}

/// What a layer's field is built from besides the objects painted on it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerSource {
    /// Only what the user paints. The ordinary case.
    #[default]
    Painted,
    /// A field imported from a GRIB2 file, drawn beneath the layer's objects.
    ///
    /// The file's first message is aligned with the project's first step, and
    /// each later step shows the message valid at its own forecast hour — or
    /// no imported field at all, where the file has none for that time
    /// (spec.md 4.8).
    Grib {
        /// The file, as the user chose it. Not copied into the project.
        path: PathBuf,
        /// Which of the file's fields this layer carries: a file holding both
        /// wind and currents imports as two layers.
        field: FieldKind,
    },
}

impl LayerSource {
    /// Whether this is the default, unwritten source.
    pub fn is_painted(&self) -> bool {
        matches!(self, Self::Painted)
    }
}

/// Which of an object's own movements reach the field it paints (spec.md 9.3,
/// M13).
///
/// One flag per animated track rather than one per object: a rotating system
/// that also travels may want its spin in the wind and not its translation,
/// or the reverse, and a single switch cannot say which (D57). The checkbox
/// sits on the track row for the same reason — the track is where the
/// movement is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MotionFlags {
    /// The anchor's travel between steps.
    pub position: bool,
    /// The frame's turn about its anchor.
    pub rotation: bool,
    /// The geometry's growth or shrink.
    pub scale: bool,
}

impl MotionFlags {
    /// Whether any of the three is on, which is what makes an object worth
    /// differentiating at all.
    pub fn any(self) -> bool {
        self.position || self.rotation || self.scale
    }
}

impl Layer {
    /// An empty, visible, unlocked layer.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Id::new(),
            name: name.into(),
            visible: true,
            locked: false,
            objects: Vec::new(),
            source: LayerSource::Painted,
            raster: None,
            speed_range: None,
            frame_overrides: Vec::new(),
        }
    }

    /// A layer carrying an imported GRIB field, hidden or shown as asked.
    pub fn from_grib(
        name: impl Into<String>,
        path: PathBuf,
        raster: Arc<RasterSequence>,
        visible: bool,
    ) -> Self {
        Self {
            id: Id::new(),
            name: name.into(),
            visible,
            locked: false,
            objects: Vec::new(),
            source: LayerSource::Grib {
                path,
                field: raster.kind,
            },
            raster: Some(raster),
            speed_range: None,
            frame_overrides: Vec::new(),
        }
    }

    /// Whether the layer carries an imported field.
    pub fn is_grib(&self) -> bool {
        !self.source.is_painted()
    }

    /// The override on a step, if the user has set one.
    pub fn frame_override(&self, step: u32) -> Option<FrameOverride> {
        self.frame_overrides
            .binary_search_by_key(&step, |o| o.step)
            .ok()
            .map(|at| self.frame_overrides[at])
    }

    /// Replaces the overrides, keeping them sorted and unique in `step`.
    ///
    /// Later entries win over earlier ones for the same step, which is what a
    /// paste that lands on a step it also overrode should do.
    pub fn set_frame_overrides(&mut self, mut overrides: Vec<FrameOverride>) {
        overrides.sort_by_key(|o| o.step);
        overrides.dedup_by_key(|o| o.step);
        self.frame_overrides = overrides;
    }

    /// The imported frame a step shows, after the user's overrides.
    ///
    /// Three answers, in order: an override naming a source step serves the
    /// message the **file** has at that step's time, wherever the file put
    /// it; an override naming nothing serves nothing, which is what a bad
    /// message needs; and an unoverridden step serves the file's own message
    /// for its own time, or nothing (spec.md 4.8, D48).
    pub fn imported_frame(
        &self,
        settings: &crate::project::ProjectSettings,
        step: u32,
    ) -> Option<&crate::raster::RasterFrame> {
        let sequence = self.raster.as_deref()?;
        let at = |s: u32| sequence.frame_at(f64::from(settings.forecast_hour(s)));
        match self.frame_override(step) {
            Some(FrameOverride {
                source: Some(s), ..
            }) => at(s),
            Some(FrameOverride { source: None, .. }) => None,
            None => at(step),
        }
    }

    /// Finds an object's index within this layer.
    pub fn index_of(&self, object: Id) -> Option<usize> {
        self.objects.iter().position(|o| o.id == object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::PropId;
    use crate::value::{Interpolation, PropValue};

    #[test]
    fn step_ranges_order_their_endpoints() {
        assert_eq!(StepRange::new(9, 2), StepRange { start: 2, end: 9 });
        assert_eq!(StepRange::new(2, 9), StepRange { start: 2, end: 9 });
    }

    #[test]
    fn step_ranges_are_inclusive() {
        let r = StepRange::new(3, 5);
        assert!(!r.contains(2));
        assert!(r.contains(3) && r.contains(4) && r.contains(5));
        assert!(!r.contains(6));
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn a_full_range_covers_every_step() {
        let r = StepRange::full(24);
        assert_eq!(r, StepRange { start: 0, end: 23 });
        assert_eq!(r.len(), 24);
    }

    #[test]
    fn clamping_pulls_both_ends_inside() {
        assert_eq!(
            StepRange::new(10, 40).clamped_to(20),
            StepRange { start: 10, end: 20 }
        );
        assert_eq!(
            StepRange::new(30, 40).clamped_to(20),
            StepRange { start: 20, end: 20 }
        );
        assert_eq!(
            StepRange::new(1, 5).clamped_to(20),
            StepRange { start: 1, end: 5 }
        );
    }

    #[test]
    fn bounding_radius_covers_every_geometry() {
        assert_eq!(Geometry::Disc { radius_m: None }.bounding_radius_m(), 0.0);
        assert_eq!(
            Geometry::Disc {
                radius_m: Some(7.0)
            }
            .bounding_radius_m(),
            7.0
        );

        let stroke = Geometry::Stroke {
            chains: vec![vec![LocalPoint::new(0.0, 0.0), LocalPoint::new(3.0, 4.0)]],
        };
        assert_eq!(stroke.bounding_radius_m(), 5.0);

        let rect = Geometry::Rect {
            half_width_m: 3.0,
            half_height_m: 4.0,
        };
        assert_eq!(rect.bounding_radius_m(), 5.0);

        // A handle reaching further than its node must widen the bound, or the
        // evaluator's cull would clip the drawn curve.
        let path = Geometry::Path {
            nodes: vec![PathNode {
                point: LocalPoint::new(1.0, 0.0),
                in_handle: None,
                out_handle: Some(LocalPoint::new(0.0, 12.0)),
            }],
        };
        assert_eq!(path.bounding_radius_m(), 12.0);
    }

    #[test]
    fn empty_geometry_has_zero_radius() {
        assert_eq!(Geometry::Stroke { chains: vec![] }.bounding_radius_m(), 0.0);
        assert_eq!(Geometry::Path { nodes: vec![] }.bounding_radius_m(), 0.0);
    }

    #[test]
    fn non_finite_geometry_is_detected() {
        let bad = Geometry::Stroke {
            chains: vec![vec![LocalPoint::new(f64::NAN, 0.0)]],
        };
        assert!(!bad.is_finite());

        let bad_handle = Geometry::Path {
            nodes: vec![PathNode {
                point: LocalPoint::new(0.0, 0.0),
                in_handle: Some(LocalPoint::new(f64::INFINITY, 0.0)),
                out_handle: None,
            }],
        };
        assert!(!bad_handle.is_finite());

        assert!(Geometry::Disc { radius_m: None }.is_finite());
        assert!(
            !Geometry::Disc {
                radius_m: Some(f64::NAN)
            }
            .is_finite()
        );
    }

    #[test]
    fn each_tool_gets_a_sensible_default_geometry() {
        for tool in ToolKind::ALL {
            let g = Geometry::default_for(tool);
            assert!(g.is_finite());
            match tool {
                ToolKind::Circle => assert_eq!(g, Geometry::Disc { radius_m: None }),
                ToolKind::Curve => assert!(matches!(g, Geometry::Path { .. })),
                _ => {}
            }
        }
    }

    /// Both gates must pass: the coarse lifetime and the animated switch.
    #[test]
    fn activity_needs_range_and_enabled_together() {
        let mut obj = Object::new(ToolKind::Brush, "stroke", 10);
        obj.active_range = StepRange::new(2, 5);

        assert!(!obj.is_active_at(1), "outside the range");
        assert!(obj.is_active_at(3), "inside the range and enabled");
        assert!(!obj.is_active_at(6), "outside the range");

        let enabled = obj.props.get_mut(PropId::Enabled).expect("present");
        enabled.set_key(0, PropValue::Bool(true), Interpolation::Step);
        enabled.set_key(4, PropValue::Bool(false), Interpolation::Step);

        assert!(obj.is_active_at(3), "still enabled at 3");
        assert!(!obj.is_active_at(4), "disabled from step 4");
        assert!(!obj.is_active_at(5), "step holds the disabled value");
    }

    #[test]
    fn new_objects_get_unique_ids_and_full_ranges() {
        let a = Object::new(ToolKind::Brush, "a", 12);
        let b = Object::new(ToolKind::Brush, "b", 12);
        assert_ne!(a.id, b.id);
        assert_eq!(a.active_range, StepRange { start: 0, end: 11 });
    }

    #[test]
    fn layers_find_their_objects() {
        let mut layer = Layer::new("L");
        let obj = Object::new(ToolKind::Circle, "c", 4);
        let id = obj.id;
        layer.objects.push(Object::new(ToolKind::Brush, "b", 4));
        layer.objects.push(obj);

        assert_eq!(layer.index_of(id), Some(1));
        assert_eq!(layer.index_of(Id::from_raw(u64::MAX)), None);
        assert!(layer.visible && !layer.locked);
    }

    #[test]
    fn objects_round_trip_through_json() {
        let mut obj = Object::new(ToolKind::Curve, "leg 1", 8);
        obj.geometry = Geometry::Path {
            nodes: vec![PathNode {
                point: LocalPoint::new(1.0, 2.0),
                in_handle: None,
                out_handle: Some(LocalPoint::new(3.0, 4.0)),
            }],
        };
        let json = serde_json::to_string(&obj).unwrap();
        let back: Object = serde_json::from_str(&json).unwrap();
        assert_eq!(obj, back);
    }
}
