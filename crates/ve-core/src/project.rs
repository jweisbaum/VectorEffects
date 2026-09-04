//! The project: settings, layers, and everything saved to a `.veproj`.
//!
//! Grid resolution, field kind, and time-step size are immutable after creation
//! (spec.md 4.1, decision D8). Changing any of them would invalidate every
//! object's relationship to the grid and every rendered frame, so a different
//! resolution means a different project.

use serde::{Deserialize, Serialize};

use crate::document::{Layer, Object, StepRange};
use crate::error::{CoreError, Result};
use crate::geo::LonLat;
use crate::id::Id;
use crate::vector::DirectionConvention;

/// The document schema version this build writes.
///
/// Opening a newer version is refused; older versions migrate forward on open.
pub const SCHEMA_VERSION: u32 = 10;

/// The largest number of time steps a project may have.
pub const MAX_STEPS: u32 = 240;

/// Whether a project describes wind or ocean current.
///
/// Fixed at creation: it selects the GRIB discipline, parameter numbers, and
/// level encoding, and gates the sailboat route feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// Wind at 10 m. GRIB discipline 0, UGRD/VGRD.
    Wind,
    /// Ocean surface current. GRIB discipline 10, UOGRD/VOGRD.
    Current,
}

/// Global grid resolution (spec.md 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    /// 1 degree: 360 x 181.
    #[serde(rename = "1.0")]
    Deg1,
    /// 0.5 degree: 720 x 361.
    #[serde(rename = "0.5")]
    Deg05,
    /// 0.25 degree: 1440 x 721.
    #[serde(rename = "0.25")]
    Deg025,
    /// 0.1 degree: 3600 x 1801.
    #[serde(rename = "0.1")]
    Deg01,
}

impl Resolution {
    /// Every resolution, coarsest first.
    pub const ALL: [Self; 4] = [Self::Deg1, Self::Deg05, Self::Deg025, Self::Deg01];

    /// Grid spacing in degrees.
    pub fn degrees(self) -> f64 {
        match self {
            Self::Deg1 => 1.0,
            Self::Deg05 => 0.5,
            Self::Deg025 => 0.25,
            Self::Deg01 => 0.1,
        }
    }

    /// Grid spacing in micro-degrees, the unit GRIB2 encodes with.
    ///
    /// Integer rather than derived from [`Self::degrees`] so the GRIB writer
    /// never rounds a float into the file.
    pub fn micro_degrees(self) -> u32 {
        match self {
            Self::Deg1 => 1_000_000,
            Self::Deg05 => 500_000,
            Self::Deg025 => 250_000,
            Self::Deg01 => 100_000,
        }
    }

    /// Points along a parallel. No duplicated column at longitude 360.
    pub fn ni(self) -> u32 {
        360_000_000 / self.micro_degrees()
    }

    /// Points along a meridian, including both poles.
    pub fn nj(self) -> u32 {
        180_000_000 / self.micro_degrees() + 1
    }

    /// Total grid points.
    pub fn point_count(self) -> u64 {
        u64::from(self.ni()) * u64::from(self.nj())
    }

    /// This resolution as a lattice to resample an imported grid onto.
    ///
    /// The same geometry the exporter writes: north-west first, rows running
    /// south, no duplicated column at 360°. An unstructured import is
    /// resampled onto exactly this, so what a GRIB layer holds lines up with
    /// what the project exports (spec §4.8).
    pub fn target_grid(self) -> crate::regrid::TargetGrid {
        crate::regrid::TargetGrid {
            ni: self.ni(),
            nj: self.nj(),
            lon0: -180.0,
            lat0: 90.0,
            dlon: self.degrees(),
            dlat: self.degrees(),
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Deg1 => "1°",
            Self::Deg05 => "0.5°",
            Self::Deg025 => "0.25°",
            Self::Deg01 => "0.1°",
        }
    }
}

/// Hours between time steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepHours {
    /// Hourly.
    #[serde(rename = "1")]
    H1,
    /// Three-hourly.
    #[serde(rename = "3")]
    H3,
    /// Six-hourly.
    #[serde(rename = "6")]
    H6,
    /// Daily.
    #[serde(rename = "24")]
    H24,
}

impl StepHours {
    /// Every step size.
    pub const ALL: [Self; 4] = [Self::H1, Self::H3, Self::H6, Self::H24];

    /// The interval in hours.
    pub fn hours(self) -> u32 {
        match self {
            Self::H1 => 1,
            Self::H3 => 3,
            Self::H6 => 6,
            Self::H24 => 24,
        }
    }
}

/// What a shrink of the timeline would delete (spec.md 4.1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShrinkImpact {
    /// Keyframes past the new end, across every object.
    pub keyframes: u32,
    /// Objects whose `active_range` reaches past the new end and will be clamped.
    pub clamped_ranges: u32,
    /// Every object touched, by id and name, in z-order.
    pub objects: Vec<(Id, String)>,
}

/// Project-wide settings (spec.md 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// Wind or current. Immutable after creation.
    pub field_kind: FieldKind,
    /// Grid resolution. Immutable after creation.
    pub resolution: Resolution,
    /// Hours per step. Immutable after creation.
    pub step_hours: StepHours,
    /// Number of time steps. May be changed.
    pub step_count: u32,
    /// How directions are shown. Display only.
    ///
    /// There is no companion speed setting: speed is stored in m/s and always
    /// shown in knots (`ve_core::units`).
    pub direction_convention: DirectionConvention,
    /// When step 0 is, as seconds since the Unix epoch, UTC (spec.md 3).
    ///
    /// Unset until the user sets it: the ruler labels steps by forecast hour
    /// either way, and by absolute time once this is known (spec.md 9.1).
    /// Seconds rather than a date type, so the file needs no calendar library
    /// and the value survives a round trip exactly. Display only for now — the
    /// GRIB reference time is chosen at export (spec.md 12.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_unix_s: Option<i64>,
}

impl ProjectSettings {
    /// Sensible defaults for a new project of `field_kind`.
    ///
    /// The direction convention follows the field kind: wind is conventionally
    /// given as the direction it blows *from*, current as where it flows *to*.
    pub fn new(
        field_kind: FieldKind,
        resolution: Resolution,
        step_hours: StepHours,
        step_count: u32,
    ) -> Self {
        Self {
            field_kind,
            resolution,
            step_hours,
            step_count: step_count.clamp(1, MAX_STEPS),
            start_unix_s: None,
            direction_convention: match field_kind {
                FieldKind::Wind => DirectionConvention::From,
                FieldKind::Current => DirectionConvention::Toward,
            },
        }
    }

    /// The last valid step index.
    pub fn last_step(self) -> u32 {
        self.step_count.saturating_sub(1)
    }

    /// Forecast hour of `step`.
    pub fn forecast_hour(self, step: u32) -> u32 {
        step * self.step_hours.hours()
    }
}

/// Camera and playhead state. A convenience, not part of the field.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    /// Map centre.
    pub center: LonLat,
    /// Zoom level.
    #[serde(with = "crate::canonical::ratio_field")]
    pub zoom: f64,
    /// The step being edited.
    pub current_step: u32,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            center: LonLat { lon: 0.0, lat: 0.0 },
            zoom: 1.0,
            current_step: 0,
        }
    }
}

/// Measurement overlays (spec.md 10).
///
/// Saved with the project but never contributing to the field or the export.
/// Defined now, populated in M8; having the slot means adding measurements
/// later needs no schema migration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct Annotations {}

/// Sailboat route data (spec.md 11). Populated in M9.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct RouteData {}

/// A complete project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Neighbour sets for resampling an unstructured imported grid onto this
    /// project's own (spec §4.8).
    ///
    /// Derived, not authored: the search that produces one takes half a
    /// second at 0.1° and depends only on the mesh and this project's
    /// resolution, so it is kept beside the project rather than recomputed on
    /// every open. It is **not** JSON — a global 0.1° set is millions of
    /// indices — but an entry of its own in the archive, keyed by mesh and
    /// target. Losing it costs time and nothing else.
    #[serde(skip)]
    pub regrid: std::collections::BTreeMap<String, std::sync::Arc<crate::regrid::Neighbours>>,
    /// Document schema version. See [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Stable identity.
    pub id: Id,
    /// Display name.
    pub name: String,
    /// Grid and display settings.
    pub settings: ProjectSettings,
    /// Layers in z-order; index 0 is the bottom.
    pub layers: Vec<Layer>,
    /// Measurement overlays.
    #[serde(default)]
    pub annotations: Annotations,
    /// Route data, for wind projects.
    #[serde(default)]
    pub routes: RouteData,
    /// Camera and playhead.
    #[serde(default)]
    pub view: ViewState,
}

impl Project {
    /// A new project with one empty layer.
    pub fn new(name: impl Into<String>, settings: ProjectSettings) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            id: Id::new(),
            name: name.into(),
            settings,
            layers: vec![Layer::new("Layer 1")],
            annotations: Annotations::default(),
            routes: RouteData::default(),
            view: ViewState::default(),
            regrid: std::collections::BTreeMap::new(),
        }
    }

    /// The last valid step index.
    pub fn last_step(&self) -> u32 {
        self.settings.last_step()
    }

    /// Finds a layer by id.
    pub fn layer(&self, id: Id) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// Finds a layer by id, mutably.
    pub fn layer_mut(&mut self, id: Id) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// The index of a layer within the stack.
    pub fn layer_index(&self, id: Id) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// Locates an object as `(layer index, object index)`.
    pub fn locate(&self, object: Id) -> Option<(usize, usize)> {
        self.layers
            .iter()
            .enumerate()
            .find_map(|(li, l)| l.index_of(object).map(|oi| (li, oi)))
    }

    /// Finds an object by id.
    pub fn object(&self, id: Id) -> Option<&Object> {
        self.layers
            .iter()
            .find_map(|l| l.objects.iter().find(|o| o.id == id))
    }

    /// Finds an object by id, mutably.
    pub fn object_mut(&mut self, id: Id) -> Option<&mut Object> {
        self.layers
            .iter_mut()
            .find_map(|l| l.objects.iter_mut().find(|o| o.id == id))
    }

    /// Every object in z-order, bottom first, paired with its layer.
    ///
    /// Invisible layers are skipped. This is the order the evaluator composites
    /// in (spec.md 7.6).
    pub fn objects_in_z_order(&self) -> impl Iterator<Item = (&Layer, &Object)> {
        self.layers
            .iter()
            .filter(|l| l.visible)
            .flat_map(|l| l.objects.iter().map(move |o| (l, o)))
    }

    /// Total object count across every layer.
    pub fn object_count(&self) -> usize {
        self.layers.iter().map(|l| l.objects.len()).sum()
    }

    /// How many keyframes sit beyond `last_step`, and how many objects hold them.
    ///
    /// Backs the quantified confirmation required before shrinking a project
    /// (spec.md 4.1, decision D13): the dialog must say how many keyframes and
    /// name the objects rather than warning vaguely.
    pub fn keyframes_after(&self, last_step: u32) -> (usize, Vec<&str>) {
        let mut total = 0;
        let mut names = Vec::new();
        for layer in &self.layers {
            for object in &layer.objects {
                let n = object.props.count_after(last_step);
                if n > 0 {
                    total += n;
                    names.push(object.name.as_str());
                }
            }
        }
        (total, names)
    }

    /// Removes every keyframe past `last_step` and clamps active ranges.
    ///
    /// Returns the number of keyframes deleted.
    pub fn truncate_to(&mut self, last_step: u32) -> usize {
        let mut removed = 0;
        for layer in &mut self.layers {
            for object in &mut layer.objects {
                removed += object.props.truncate_to(last_step);
                object.active_range = object.active_range.clamped_to(last_step);
            }
        }
        removed
    }

    /// Repairs a freshly-loaded document.
    ///
    /// Run after every load. It reserves ids so the allocator cannot reissue
    /// one the document already uses, backfills properties added since the file
    /// was written, and clamps ranges into the project's step count.
    pub fn normalize(&mut self) {
        Id::reserve_above(self.id.raw());
        let last = self.settings.last_step();

        for layer in &mut self.layers {
            Id::reserve_above(layer.id.raw());
            for object in &mut layer.objects {
                Id::reserve_above(object.id.raw());
                object.props.backfill(object.tool);
                object.active_range = object.active_range.clamped_to(last);
            }
        }

        if self.view.current_step > last {
            self.view.current_step = last;
        }
    }

    /// What reducing the timeline to `step_count` steps would delete.
    ///
    /// Spec 4.1 gates the shrink behind a confirmation that states the exact
    /// count and names the objects, so the number has to be computed *before*
    /// the command runs and has to be what the command then deletes. Both read
    /// the same predicate.
    pub fn shrink_impact(&self, step_count: u32) -> ShrinkImpact {
        let last = step_count.clamp(1, MAX_STEPS).saturating_sub(1);
        let mut impact = ShrinkImpact::default();
        for object in self.layers.iter().flat_map(|layer| &layer.objects) {
            let keys = object.props.count_after(last);
            let clamped = object.active_range.end > last;
            if keys > 0 || clamped {
                impact.keyframes += keys as u32;
                impact.clamped_ranges += u32::from(clamped);
                impact.objects.push((object.id, object.name.clone()));
            }
        }
        impact
    }

    /// Checks the document is internally consistent and saveable.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version > SCHEMA_VERSION {
            return Err(CoreError::SchemaTooNew {
                found: self.schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        if self.settings.step_count == 0 || self.settings.step_count > MAX_STEPS {
            return Err(CoreError::InvalidStepCount(self.settings.step_count));
        }

        let last = self.settings.last_step();
        for layer in &self.layers {
            for object in &layer.objects {
                if !object.geometry.is_finite() {
                    return Err(CoreError::NonFiniteCoordinate("object geometry"));
                }
                if object.active_range.end > last {
                    return Err(CoreError::RangeOutOfBounds {
                        object: object.name.clone(),
                        end: object.active_range.end,
                        last,
                    });
                }
                for (id, anim) in object.props.iter() {
                    if !anim.base().is_finite() || anim.keys().iter().any(|k| !k.value.is_finite())
                    {
                        return Err(CoreError::NonFiniteProperty {
                            object: object.name.clone(),
                            property: format!("{id:?}"),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// A default active range covering the whole project.
    pub fn full_range(&self) -> StepRange {
        StepRange::full(self.settings.step_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Geometry, LocalPoint, Object};
    use crate::schema::{PropId, ToolKind};
    use crate::value::{Interpolation, PropValue};

    fn settings() -> ProjectSettings {
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 24)
    }

    /// Checked against the table in spec.md 4.2. If the grid dimensions are
    /// wrong, every exported GRIB is wrong, so these are literal expectations
    /// rather than anything derived from the implementation.
    #[test]
    fn grid_dimensions_match_the_specification() {
        let expected = [
            (Resolution::Deg1, 360, 181, 65_160, 1_000_000),
            (Resolution::Deg05, 720, 361, 259_920, 500_000),
            (Resolution::Deg025, 1440, 721, 1_038_240, 250_000),
            (Resolution::Deg01, 3600, 1801, 6_483_600, 100_000),
        ];
        for (res, ni, nj, points, micro) in expected {
            assert_eq!(res.ni(), ni, "{res:?} ni");
            assert_eq!(res.nj(), nj, "{res:?} nj");
            assert_eq!(res.point_count(), points, "{res:?} point count");
            assert_eq!(res.micro_degrees(), micro, "{res:?} micro-degrees");
        }
    }

    /// Longitude has no duplicated column at 360; latitude includes both poles.
    #[test]
    fn the_grid_spans_the_globe_exactly_once() {
        for res in Resolution::ALL {
            assert_eq!(
                f64::from(res.ni()) * res.degrees(),
                360.0,
                "{res:?} must wrap the globe exactly once"
            );
            assert_eq!(
                f64::from(res.nj() - 1) * res.degrees(),
                180.0,
                "{res:?} must span pole to pole"
            );
        }
    }

    #[test]
    fn step_hours_report_their_interval() {
        assert_eq!(StepHours::H1.hours(), 1);
        assert_eq!(StepHours::H24.hours(), 24);
    }

    #[test]
    fn forecast_hours_follow_the_step_size() {
        let s = ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H6, 10);
        assert_eq!(s.forecast_hour(0), 0);
        assert_eq!(s.forecast_hour(4), 24);
        assert_eq!(s.last_step(), 9);
    }

    /// Wind is conventionally reported as the direction it blows *from*;
    /// current as where it flows *to*.
    #[test]
    fn the_default_convention_follows_the_field_kind() {
        let wind = ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H1, 5);
        assert_eq!(wind.direction_convention, DirectionConvention::From);

        let current = ProjectSettings::new(FieldKind::Current, Resolution::Deg1, StepHours::H1, 5);
        assert_eq!(current.direction_convention, DirectionConvention::Toward);
    }

    #[test]
    fn step_count_is_clamped_at_creation() {
        let zero = ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H1, 0);
        assert_eq!(zero.step_count, 1);
        let huge = ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H1, 9_999);
        assert_eq!(huge.step_count, MAX_STEPS);
    }

    #[test]
    fn a_new_project_has_one_layer_and_no_objects() {
        let p = Project::new("Test", settings());
        assert_eq!(p.layers.len(), 1);
        assert_eq!(p.object_count(), 0);
        assert_eq!(p.schema_version, SCHEMA_VERSION);
        p.validate().unwrap();
    }

    #[test]
    fn objects_and_layers_are_findable_by_id() {
        let mut p = Project::new("Test", settings());
        let obj = Object::new(ToolKind::Brush, "b", 24);
        let (oid, lid) = (obj.id, p.layers[0].id);
        p.layers[0].objects.push(obj);

        assert_eq!(p.locate(oid), Some((0, 0)));
        assert_eq!(p.object(oid).map(|o| o.name.as_str()), Some("b"));
        assert!(p.object_mut(oid).is_some());
        assert_eq!(p.layer_index(lid), Some(0));
        assert!(p.layer(lid).is_some());
        assert_eq!(p.locate(Id::from_raw(u64::MAX)), None);
    }

    /// Compositing order is layer order then object order, and an invisible
    /// layer contributes nothing (spec.md 7.6).
    #[test]
    fn z_order_is_bottom_first_and_skips_hidden_layers() {
        let mut p = Project::new("Test", settings());
        p.layers[0]
            .objects
            .push(Object::new(ToolKind::Brush, "bottom", 24));
        p.layers[0]
            .objects
            .push(Object::new(ToolKind::Brush, "above", 24));

        let mut hidden = Layer::new("hidden");
        hidden.visible = false;
        hidden
            .objects
            .push(Object::new(ToolKind::Brush, "invisible", 24));
        p.layers.push(hidden);

        let mut top = Layer::new("top");
        top.objects
            .push(Object::new(ToolKind::Brush, "topmost", 24));
        p.layers.push(top);

        let order: Vec<&str> = p
            .objects_in_z_order()
            .map(|(_, o)| o.name.as_str())
            .collect();
        assert_eq!(order, vec!["bottom", "above", "topmost"]);
    }

    #[test]
    fn shrinking_reports_an_exact_count_before_deleting() {
        let mut p = Project::new("Test", settings());
        let mut obj = Object::new(ToolKind::Brush, "gusty", 24);
        let speed = obj.props.get_mut(PropId::Speed).expect("present");
        speed.set_key(0, PropValue::F32(5.0), Interpolation::Linear);
        speed.set_key(10, PropValue::F32(20.0), Interpolation::Linear);
        speed.set_key(20, PropValue::F32(5.0), Interpolation::Linear);
        p.layers[0].objects.push(obj);
        p.layers[0]
            .objects
            .push(Object::new(ToolKind::Circle, "calm", 24));

        let (count, names) = p.keyframes_after(9);
        assert_eq!(count, 2);
        assert_eq!(
            names,
            vec!["gusty"],
            "only objects that lose keys are named"
        );

        assert_eq!(p.truncate_to(9), 2);
        assert_eq!(p.keyframes_after(9).0, 0);
        assert_eq!(
            p.layers[0].objects[0].active_range.end, 9,
            "ranges clamp too"
        );
    }

    /// Ids come from the file, but the allocator restarts at 1 each run. Without
    /// reserving, a newly created object could collide with a loaded one.
    #[test]
    fn normalising_reserves_loaded_ids() {
        let mut p = Project::new("Test", settings());
        p.id = Id::from_raw(5_000_000);
        p.layers[0].id = Id::from_raw(5_000_001);
        let mut obj = Object::new(ToolKind::Brush, "b", 24);
        obj.id = Id::from_raw(5_000_002);
        p.layers[0].objects.push(obj);

        p.normalize();
        assert!(Id::new().raw() > 5_000_002);
    }

    #[test]
    fn normalising_clamps_ranges_and_the_playhead() {
        let mut p = Project::new("Test", settings());
        p.settings.step_count = 5;
        p.view.current_step = 99;
        let mut obj = Object::new(ToolKind::Brush, "b", 24);
        obj.active_range = StepRange::new(0, 23);
        p.layers[0].objects.push(obj);

        p.normalize();
        assert_eq!(p.view.current_step, 4);
        assert_eq!(p.layers[0].objects[0].active_range.end, 4);
        p.validate().unwrap();
    }

    /// Loading an older file must not need a bespoke migration just because a
    /// property was added.
    #[test]
    fn normalising_backfills_properties_added_since_the_file_was_written() {
        let mut p = Project::new("Test", settings());
        let mut obj = Object::new(ToolKind::Brush, "b", 24);
        obj.props.remove(PropId::Feather);
        p.layers[0].objects.push(obj);

        p.normalize();
        assert!(p.layers[0].objects[0].props.get(PropId::Feather).is_some());
    }

    #[test]
    fn validation_rejects_a_newer_schema() {
        let mut p = Project::new("Test", settings());
        p.schema_version = SCHEMA_VERSION + 1;
        assert!(matches!(p.validate(), Err(CoreError::SchemaTooNew { .. })));
    }

    #[test]
    fn validation_rejects_an_impossible_step_count() {
        let mut p = Project::new("Test", settings());
        p.settings.step_count = 0;
        assert!(matches!(p.validate(), Err(CoreError::InvalidStepCount(0))));
    }

    #[test]
    fn validation_rejects_a_range_past_the_end() {
        let mut p = Project::new("Test", settings());
        let mut obj = Object::new(ToolKind::Brush, "b", 24);
        obj.active_range = StepRange::new(0, 99);
        p.layers[0].objects.push(obj);
        assert!(matches!(
            p.validate(),
            Err(CoreError::RangeOutOfBounds { .. })
        ));
    }

    /// JSON has no NaN, so a non-finite value must be caught before it makes a
    /// project unsaveable.
    #[test]
    fn validation_rejects_non_finite_values() {
        let mut p = Project::new("Test", settings());
        let mut obj = Object::new(ToolKind::Brush, "b", 24);
        obj.props
            .get_mut(PropId::Speed)
            .expect("present")
            .set_base(PropValue::F32(f32::NAN));
        p.layers[0].objects.push(obj);
        assert!(matches!(
            p.validate(),
            Err(CoreError::NonFiniteProperty { .. })
        ));

        let mut p2 = Project::new("Test", settings());
        let mut obj2 = Object::new(ToolKind::Brush, "b", 24);
        obj2.geometry = Geometry::Stroke {
            chains: vec![vec![LocalPoint::new(f64::NAN, 0.0)]],
        };
        p2.layers[0].objects.push(obj2);
        assert!(matches!(
            p2.validate(),
            Err(CoreError::NonFiniteCoordinate(_))
        ));
    }

    #[test]
    fn projects_round_trip_through_json() {
        let mut p = Project::new("Round trip", settings());
        p.layers[0]
            .objects
            .push(Object::new(ToolKind::Brush, "b", 24));
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
