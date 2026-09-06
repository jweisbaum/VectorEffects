//! The document tree and per-object properties, for the editing panels.
//!
//! The frontend never sees `ve-core` types. It receives a flat tree of layers
//! and objects, and a list of property descriptions built from the schema — so
//! adding a property to a tool makes it appear in the inspector with no
//! frontend change at all (`CLAUDE.md`, "adding a property").

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_core::angle::Angle;
use ve_core::project::Project;
use ve_core::schema::{PropId, ToolKind, all_specs};
use ve_core::value::PropKind;
use ve_core::{LonLat, PropValue};

use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::{ProjectSummary, with_session};

/// One object, as the layer panel sees it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ObjectNode.ts")]
pub struct ObjectNode {
    /// Stable identity.
    pub id: u64,
    /// User-editable name.
    pub name: String,
    /// Which tool made it, e.g. `"brush"`.
    pub tool: String,
    /// Display name of the tool.
    pub tool_label: String,
    /// Whether it contributes at the step being viewed.
    pub active_here: bool,
    /// First step of its lifetime, inclusive.
    pub start_step: u32,
    /// Last step of its lifetime, inclusive.
    pub end_step: u32,
}

/// One layer, with its objects in z-order.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "LayerNode.ts")]
pub struct LayerNode {
    /// Stable identity.
    pub id: u64,
    /// User-editable name.
    pub name: String,
    /// Whether the layer contributes and is drawn.
    pub visible: bool,
    /// Whether its objects can be selected and edited.
    pub locked: bool,
    /// Objects, bottom of the layer first.
    pub objects: Vec<ObjectNode>,
    /// The imported field beneath the objects, for a GRIB layer.
    pub grib: Option<GribLayerInfo>,
    /// The picture beneath everything, for an image layer (spec.md 4.9, M18).
    pub image: Option<crate::image::ImageLayerView>,
    /// What the layer is (M29): `"painted"`, `"raster"` for an imported GRIB,
    /// `"image"` for a picture. Says which controls the panel offers.
    pub source: String,
    /// Which field the layer is part of — `"wind"` or `"current"`: its
    /// file's for a raster layer, none that matters for an image (M29).
    pub parameter: String,
}

/// What one step of a GRIB layer shows (spec.md 4.8, M20).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "GribStepView.ts")]
pub struct GribStepView {
    /// The file has a message of its own for this step's time.
    pub in_file: bool,
    /// The step whose message this one shows, when the user pasted one here.
    ///
    /// `null` when the step is not overridden at all — which is not the same
    /// as being overridden to show nothing, and `hidden` is what says which.
    pub source: Option<u32>,
    /// The step is overridden to show no imported field, which is what a bad
    /// message needs.
    pub hidden: bool,
    /// A frame reaches the map here, from the file or from a paste.
    pub shown: bool,
}

/// What the panel says about a layer's imported field (spec.md 4.8).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "GribLayerInfo.ts")]
pub struct GribLayerInfo {
    /// The file the field is read from.
    pub path: String,
    /// `"wind"` or `"current"`.
    pub field_kind: String,
    /// Whether the file was read; false when it is missing or unreadable,
    /// in which case the layer contributes nothing.
    pub loaded: bool,
    /// Time slices the file holds, or 0 when not loaded.
    pub frame_count: u32,
    /// Hours from the first slice to the last, or 0 when not loaded.
    pub span_hours: f64,
    /// The band of speeds the layer keeps, in m/s, if it filters (spec.md 4.8).
    pub speed_min_mps: Option<f32>,
    /// The top of that band.
    pub speed_max_mps: Option<f32>,
    /// The fastest speed the file holds, in m/s, over every message.
    ///
    /// What the panel's slider runs to: a filter is set by looking at the
    /// field, and a scale that ended at a number the file never reaches would
    /// spend most of its travel on nothing.
    pub speed_ceiling_mps: f32,
    /// Which of the project's steps the file has a message for.
    ///
    /// One entry per step, in step order. A step the file says nothing about
    /// shows no imported field at all (spec.md 4.8), and a timeline that did
    /// not say which those were would leave the user to work it out from a
    /// field that comes and goes. The timeline marks them.
    pub covered_steps: Vec<bool>,
    /// What each of the project's steps actually shows, after the user's
    /// frame overrides (M20).
    ///
    /// One entry per step, in step order. `covered_steps` beside it is what
    /// the *file* says; this is what the map will draw, and the timeline
    /// needs both to tell a message from a pasted one.
    pub steps: Vec<GribStepView>,
}

/// The whole document, for the panel.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "DocumentTree.ts")]
pub struct DocumentTree {
    /// Layers, bottom of the stack first — the order they composite in.
    pub layers: Vec<LayerNode>,
}

/// A property value crossing IPC.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "PropertyValue.ts")]
pub enum PropertyValue {
    /// A scalar.
    Number {
        /// The value.
        value: f64,
    },
    /// A toggle.
    Bool {
        /// The value.
        value: bool,
    },
    /// A bearing in degrees.
    Angle {
        /// Degrees clockwise from north.
        degrees: f64,
    },
    /// A geographic position.
    Position {
        /// Longitude.
        lon: f64,
        /// Latitude.
        lat: f64,
    },
    /// An index into the property's variants.
    Choice {
        /// Selected variant.
        index: u8,
    },
}

impl PropertyValue {
    /// The wire form of a stored value.
    pub fn of(value: PropValue) -> Self {
        match value {
            PropValue::F32(v) => Self::Number {
                value: f64::from(v),
            },
            PropValue::Bool(v) => Self::Bool { value: v },
            PropValue::Angle(a) => Self::Angle {
                degrees: a.degrees(),
            },
            PropValue::LonLat(p) => Self::Position {
                lon: p.lon,
                lat: p.lat,
            },
            PropValue::Enum(v) => Self::Choice { index: v },
        }
    }

    /// Converts back, checking the shape matches what the property holds.
    pub fn into_prop(self, expected: PropKind) -> Result<PropValue> {
        let bad = |got: &str| AppError::BadOption {
            field: "property value",
            value: format!("{got} where {expected:?} was expected"),
        };
        match (self, expected) {
            (Self::Number { value }, PropKind::F32) => Ok(PropValue::F32(value as f32)),
            (Self::Bool { value }, PropKind::Bool) => Ok(PropValue::Bool(value)),
            (Self::Angle { degrees }, PropKind::Angle) => Ok(PropValue::Angle(Angle::new(degrees))),
            (Self::Position { lon, lat }, PropKind::LonLat) => {
                Ok(PropValue::LonLat(LonLat::new(lon, lat)?))
            }
            (Self::Choice { index }, PropKind::Enum) => Ok(PropValue::Enum(index)),
            (Self::Number { .. }, _) => Err(bad("a number")),
            (Self::Bool { .. }, _) => Err(bad("a boolean")),
            (Self::Angle { .. }, _) => Err(bad("an angle")),
            (Self::Position { .. }, _) => Err(bad("a position")),
            (Self::Choice { .. }, _) => Err(bad("a choice")),
        }
    }
}

/// One property, described well enough for the inspector to render it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "PropertyView.ts")]
pub struct PropertyView {
    /// Identifier, e.g. `"speed"`.
    pub id: String,
    /// Inspector label.
    pub label: String,
    /// Value at the step being viewed.
    pub value: PropertyValue,
    /// Display unit: `"speed"`, `"kilometres"`, `"degrees"`, `"percent"` or `"none"`.
    ///
    /// A speed is stored in m/s and shown in knots; the frontend converts
    /// (`ve_core::units`).
    pub unit: String,
    /// Lower bound, for numeric properties.
    pub min: Option<f32>,
    /// Upper bound, for numeric properties.
    pub max: Option<f32>,
    /// Variant names, for choices.
    pub variants: Vec<String>,
    /// Whether the property has keyframes.
    pub animated: bool,
    /// Whether a key sits on the step being viewed (spec.md 9.3).
    pub keyed_here: bool,
    /// Whether the value shown is interpolated between keys rather than keyed
    /// or held (spec.md 9.3).
    pub interpolated_here: bool,
    /// Edited by a centred slider rather than a typed number (M29).
    pub slider: Option<SliderView>,
}

/// A centred slider's labelling (M29), as the schema declares it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "SliderView.ts")]
pub struct SliderView {
    /// The label at the left end.
    pub low_label: String,
    /// And at the right end.
    pub high_label: String,
    /// Whether the stored value's positive end is the left one.
    pub reversed: bool,
}

impl SliderView {
    pub(crate) fn of(slider: ve_core::schema::Slider) -> Self {
        Self {
            low_label: slider.low_label.to_owned(),
            high_label: slider.high_label.to_owned(),
            reversed: slider.reversed,
        }
    }
}

fn tool_name(tool: ToolKind) -> &'static str {
    match tool {
        ToolKind::Brush => "brush",
        ToolKind::Circle => "circle",
        ToolKind::ShapeFill => "shape_fill",
        ToolKind::Mask => "mask",
        ToolKind::CloneStamp => "clone_stamp",
        ToolKind::Curve => "curve",
        ToolKind::Intensity => "intensity",
        ToolKind::Divergence => "divergence",
        ToolKind::Turn => "turn",
        ToolKind::Warp => "warp",
        ToolKind::Liquify => "liquify",
        ToolKind::Patch => "patch",
        ToolKind::Macro => "macro",
    }
}

pub(crate) fn unit_name(unit: ve_core::schema::Unit) -> &'static str {
    use ve_core::schema::Unit;
    match unit {
        Unit::None => "none",
        Unit::Speed => "speed",
        Unit::Kilometres => "kilometres",
        Unit::Degrees => "degrees",
        Unit::Direction => "direction",
        Unit::Percent => "percent",
    }
}

fn tree_of(project: &Project, step: u32) -> DocumentTree {
    DocumentTree {
        layers: project
            .layers
            .iter()
            .map(|layer| LayerNode {
                id: layer.id.raw(),
                name: layer.name.clone(),
                visible: layer.visible,
                locked: layer.locked,
                source: match &layer.source {
                    ve_core::document::LayerSource::Painted => "painted",
                    ve_core::document::LayerSource::Grib { .. } => "raster",
                    ve_core::document::LayerSource::Image { .. } => "image",
                }
                .to_owned(),
                parameter: crate::projects::kind_name(layer.parameter()).to_owned(),
                image: crate::image::view(layer.id, &layer.source),
                grib: match &layer.source {
                    // An image contributes no field, so it has no GRIB view; it
                    // has an `image` one instead (spec.md 4.9, M18).
                    ve_core::document::LayerSource::Painted
                    | ve_core::document::LayerSource::Image { .. } => None,
                    ve_core::document::LayerSource::Grib { path, field } => Some(GribLayerInfo {
                        path: path.to_string_lossy().into_owned(),
                        field_kind: match field {
                            ve_core::FieldKind::Wind => "wind".to_owned(),
                            ve_core::FieldKind::Current => "current".to_owned(),
                        },
                        loaded: layer.raster.is_some(),
                        frame_count: layer.raster.as_ref().map_or(0, |r| r.frames.len() as u32),
                        span_hours: layer.raster.as_ref().map_or(0.0, |r| r.span_hours()),
                        speed_min_mps: layer.speed_range.map(|b| b.min_mps),
                        speed_max_mps: layer.speed_range.map(|b| b.max_mps),
                        speed_ceiling_mps: layer
                            .raster
                            .as_ref()
                            .map_or(0.0, |sequence| sequence.fastest_mps()),
                        covered_steps: (0..project.settings.step_count)
                            .map(|s| {
                                let hour = f64::from(project.settings.forecast_hour(s));
                                layer
                                    .raster
                                    .as_ref()
                                    .is_some_and(|seq| seq.frame_at(hour).is_some())
                            })
                            .collect(),
                        steps: (0..project.settings.step_count)
                            .map(|s| {
                                let hour = f64::from(project.settings.forecast_hour(s));
                                let over = layer.frame_override(s);
                                GribStepView {
                                    in_file: layer
                                        .raster
                                        .as_ref()
                                        .is_some_and(|seq| seq.frame_at(hour).is_some()),
                                    source: over.and_then(|o| o.source),
                                    hidden: over.is_some_and(|o| o.source.is_none()),
                                    shown: layer.imported_frame(&project.settings, s).is_some(),
                                }
                            })
                            .collect(),
                    }),
                },
                objects: layer
                    .objects
                    .iter()
                    .map(|object| ObjectNode {
                        id: object.id.raw(),
                        name: object.name.clone(),
                        tool: tool_name(object.tool).to_owned(),
                        tool_label: object.tool.label().to_owned(),
                        active_here: object.is_active_at(step),
                        start_step: object.active_range.start,
                        end_step: object.active_range.end,
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// The document tree, for the layer panel.
#[tauri::command]
pub fn document_tree(state: tauri::State<'_, AppState>, step: u32) -> Result<DocumentTree> {
    tree(&state, step)
}

/// Implementation of [`document_tree`], callable without a Tauri handle.
pub fn tree(state: &AppState, step: u32) -> Result<DocumentTree> {
    with_session(state, |session| {
        Ok(tree_of(&session.require_open()?.project, step))
    })
}

/// Every property of one object, in schema order.
#[tauri::command]
pub fn object_properties(
    state: tauri::State<'_, AppState>,
    object: u64,
    step: u32,
) -> Result<Vec<PropertyView>> {
    properties(&state, object, step)
}

/// Implementation of [`object_properties`], callable without a Tauri handle.
pub fn properties(state: &AppState, object: u64, step: u32) -> Result<Vec<PropertyView>> {
    with_session(state, |session| {
        let id = ve_core::Id::from_raw(object);
        let open = session.require_open()?;
        let object = open
            .project
            .object(id)
            .ok_or(AppError::Core(ve_core::CoreError::MissingObject(id.raw())))?;

        // A property the object's own mode makes inert is left out entirely.
        // Editing one and watching nothing happen reads as a broken control,
        // and this is the only place that knows the resolved values.
        let choice_of = |id: PropId| {
            object
                .props
                .value_at(object.tool, id, step)
                .and_then(ve_core::PropValue::as_enum)
                .unwrap_or(0)
        };

        // Driven by the schema, so a new property appears here with no change
        // to this function or to the frontend.
        Ok(all_specs(object.tool)
            // The panel edits; a property that cannot be edited has no row in
            // it (spec.md 6.1).
            .filter(|spec| !spec.creation_only)
            .filter(|spec| ve_core::schema::is_live(object.tool, spec.id, choice_of))
            .filter_map(|spec| {
                let animatable = object.props.get(spec.id)?;
                Some(PropertyView {
                    id: format!("{:?}", spec.id),
                    label: spec.label.to_owned(),
                    value: PropertyValue::of(animatable.value_at(step)),
                    unit: unit_name(spec.unit).to_owned(),
                    min: spec.range.map(|(low, _)| low),
                    max: spec.range.map(|(_, high)| high),
                    variants: spec.variants.iter().map(|v| (*v).to_owned()).collect(),
                    animated: animatable.is_animated(),
                    keyed_here: animatable.keys().iter().any(|key| key.step == step),
                    interpolated_here: !animatable.keys().iter().any(|key| key.step == step)
                        && animatable
                            .keys()
                            .first()
                            .is_some_and(|first| first.step < step)
                        && animatable
                            .keys()
                            .last()
                            .is_some_and(|last| last.step > step),
                    slider: spec.slider.map(SliderView::of),
                })
            })
            .collect())
    })
}

/// Finds a property id by the name the inspector sent back.
fn prop_id_named(tool: ToolKind, name: &str) -> Option<PropId> {
    all_specs(tool)
        .map(|spec| spec.id)
        .find(|id| format!("{id:?}") == name)
}

/// Changes one property of one object.
///
/// `gesture` groups consecutive changes into a single undo entry. A drag passes
/// the same key throughout and calls [`end_gesture`] on release, so one drag is
/// one undo rather than one per pointer event.
#[tauri::command]
pub fn set_object_property(
    state: tauri::State<'_, AppState>,
    object: u64,
    property: String,
    value: PropertyValue,
    step: u32,
    auto_key: bool,
    gesture: Option<String>,
) -> Result<ProjectSummary> {
    set_property_with(&state, object, &property, value, gesture, step, auto_key)
}

/// Implementation of [`set_object_property`] without a gesture.
pub fn set_property(
    state: &AppState,
    object: u64,
    property: &str,
    value: PropertyValue,
) -> Result<ProjectSummary> {
    set_property_with(state, object, property, value, None, 0, false)
}

/// Implementation of [`set_object_property`], callable without a Tauri handle.
///
/// `step` and `auto_key` decide where the change goes — the current step's key
/// or the base — by the rule in [`crate::animation::written`] (spec.md 9.3).
pub fn set_property_with(
    state: &AppState,
    object: u64,
    property: &str,
    value: PropertyValue,
    gesture: Option<String>,
    step: u32,
    auto_key: bool,
) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let id = ve_core::Id::from_raw(object);
        let open = session.require_open()?;
        let target = open
            .project
            .object(id)
            .ok_or(AppError::Core(ve_core::CoreError::MissingObject(id.raw())))?;

        let prop = prop_id_named(target.tool, property).ok_or_else(|| AppError::BadOption {
            field: "property",
            value: property.to_owned(),
        })?;
        // Refused here rather than only hidden in the inspector: a rule the
        // document does not enforce is decorative, and every other caller —
        // the timeline, a script, a future panel — would still get through.
        if ve_core::schema::spec_for(target.tool, prop).is_some_and(|s| s.creation_only) {
            return Err(AppError::BadOption {
                field: "property",
                value: format!("{property} is fixed when the object is created"),
            });
        }
        let before = target
            .props
            .get(prop)
            .ok_or_else(|| AppError::BadOption {
                field: "property",
                value: property.to_owned(),
            })?
            .clone();

        let value = value.into_prop(before.kind())?;
        let after = crate::animation::written(&before, step, auto_key, value);

        let command = ve_core::command::Command::SetProperty {
            object: id,
            prop,
            before: Box::new(before),
            after: Box::new(after),
        };

        let open = session.require_open()?;
        let (project, history) = (&mut open.project, &mut open.history);
        match gesture {
            Some(key) => history.push_coalesced(project, command, key)?,
            None => history.push(project, command)?,
        }
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// Ends the current gesture, so the next change starts a new undo entry.
#[tauri::command]
pub fn end_gesture(state: tauri::State<'_, AppState>) -> Result<()> {
    finish_gesture(&state)
}

/// Implementation of [`end_gesture`], callable without a Tauri handle.
pub fn finish_gesture(state: &AppState) -> Result<()> {
    with_session(state, |session| {
        // Dropping the baseline matters as much as closing the entry: a stale
        // one would make the next drag compute from where the last one started.
        session.transform = None;
        session.require_open()?.history.break_coalescing();
        Ok(())
    })
}

// --- Structural edits -------------------------------------------------------

use ve_core::command::Command;
use ve_core::document::{Layer, StepRange};

/// Applies a command through the undo stack and reports the new state.
fn apply(
    state: &AppState,
    build: impl FnOnce(&Project) -> Result<Command>,
) -> Result<ProjectSummary> {
    apply_with(state, None, build)
}

/// [`apply`], with a coalescing key for an edit that is part of a drag.
fn apply_with(
    state: &AppState,
    gesture: Option<String>,
    build: impl FnOnce(&Project) -> Result<Command>,
) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let command = build(&session.require_open()?.project)?;
        let open = session.require_open()?;
        let (project, history) = (&mut open.project, &mut open.history);
        match gesture {
            Some(key) => history.push_coalesced(project, command, key)?,
            None => history.push(project, command)?,
        }
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

pub(crate) fn object_id(raw: u64) -> ve_core::Id {
    ve_core::Id::from_raw(raw)
}

fn missing_object(raw: u64) -> AppError {
    AppError::Core(ve_core::CoreError::MissingObject(raw))
}

fn missing_layer(raw: u64) -> AppError {
    AppError::Core(ve_core::CoreError::MissingLayer(raw))
}

/// The layer a new object joins (spec.md 6.1, D66).
///
/// **One rule for every path that adds an object** — creation, paste, the
/// pasted patch, an inserted macro, a duplicate and the panel's drag-drop.
/// Each of these chose a layer its own way before: the top of the stack, the
/// active one, whatever it was handed. `None` means the top of the stack,
/// which is where a project with no layer chosen puts things.
///
/// **An imported layer takes nothing.** A GRIB layer's field is its file, and
/// an object composited above that file inside the same layer would be
/// painted onto a forecast that is not the user's to paint on; the refusal
/// names the layer so the hint area can say which one to pick instead. A
/// locked layer is refused for the reason it is locked.
pub(crate) fn creation_layer(project: &Project, layer: Option<u64>) -> Result<&Layer> {
    let found = match layer {
        Some(raw) => project
            .layer(object_id(raw))
            .ok_or_else(|| missing_layer(raw))?,
        None => project
            .layers
            .last()
            .ok_or_else(|| AppError::Internal("project has no layers".to_owned()))?,
    };
    if !found.source.is_painted() {
        return Err(AppError::BadOption {
            field: "layer",
            value: format!(
                "\"{}\" is an imported field and cannot hold objects; pick a painted layer",
                found.name
            ),
        });
    }
    if found.locked {
        return Err(AppError::BadOption {
            field: "layer",
            value: format!("\"{}\" is locked", found.name),
        });
    }
    Ok(found)
}

/// Renames the project (M25): a document write, undoable.
#[tauri::command]
pub fn rename_project(state: tauri::State<'_, AppState>, name: String) -> Result<ProjectSummary> {
    project_rename(&state, name)
}

/// Implementation of [`rename_project`].
///
/// The name is the project's, so it is a history entry like a layer's
/// rename; an empty name is refused rather than written, since the title bar
/// and the recent list would then show nothing to click on.
pub fn project_rename(state: &AppState, name: String) -> Result<ProjectSummary> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(AppError::BadOption {
            field: "name",
            value: "a project needs a name".to_owned(),
        });
    }
    apply(state, |project| {
        Ok(Command::SetProjectName {
            before: project.name.clone(),
            after: name,
        })
    })
}

/// Adds a layer above the current top.
#[tauri::command]
pub fn add_layer(state: tauri::State<'_, AppState>, name: String) -> Result<ProjectSummary> {
    layer_add(&state, name)
}

/// Implementation of [`add_layer`].
pub fn layer_add(state: &AppState, name: String) -> Result<ProjectSummary> {
    apply(state, |project| {
        let name = if name.trim().is_empty() {
            format!("Layer {}", project.layers.len() + 1)
        } else {
            name.trim().to_owned()
        };
        Ok(Command::AddLayer {
            index: project.layers.len(),
            layer: Box::new(Layer::of_kind(name, project.settings.field_kind)),
        })
    })
}

/// Removes a layer and everything in it.
#[tauri::command]
pub fn remove_layer(state: tauri::State<'_, AppState>, layer: u64) -> Result<ProjectSummary> {
    layer_remove(&state, layer)
}

/// Implementation of [`remove_layer`].
pub fn layer_remove(state: &AppState, layer: u64) -> Result<ProjectSummary> {
    apply(state, |project| {
        // A project always has somewhere to put an object, so the last layer
        // cannot be removed.
        if project.layers.len() <= 1 {
            return Err(AppError::Internal(
                "a project must keep at least one layer".to_owned(),
            ));
        }
        let index = project
            .layer_index(object_id(layer))
            .ok_or_else(|| missing_layer(layer))?;
        Ok(Command::RemoveLayer {
            index,
            layer: Box::new(project.layers[index].clone()),
        })
    })
}

/// Renames a layer.
#[tauri::command]
pub fn rename_layer(
    state: tauri::State<'_, AppState>,
    layer: u64,
    name: String,
) -> Result<ProjectSummary> {
    layer_rename(&state, layer, name)
}

/// Implementation of [`rename_layer`].
pub fn layer_rename(state: &AppState, layer: u64, name: String) -> Result<ProjectSummary> {
    apply(state, |project| {
        let found = project
            .layer(object_id(layer))
            .ok_or_else(|| missing_layer(layer))?;
        Ok(Command::RenameLayer {
            layer: found.id,
            before: found.name.clone(),
            after: name,
        })
    })
}

/// Sets which speeds an imported field keeps, or clears the filter (spec.md 4.8).
///
/// In m/s, like everything below the IPC boundary; the panel shows knots.
/// Passing `null` for either end clears the band, which is what "keep
/// everything" means — a range with no ends is not a range.
#[tauri::command]
pub fn set_layer_speed_range(
    state: tauri::State<'_, AppState>,
    layer: u64,
    min_mps: Option<f32>,
    max_mps: Option<f32>,
    gesture: Option<String>,
) -> Result<ProjectSummary> {
    layer_speed_range(&state, layer, min_mps, max_mps, gesture)
}

/// Implementation of [`set_layer_speed_range`].
///
/// `gesture` is the coalescing key a slider drag sends on every tick, so the
/// drag is one history entry and one undo returns the band to where the drag
/// began; a typed value sends none and is its own entry.
pub fn layer_speed_range(
    state: &AppState,
    layer: u64,
    min_mps: Option<f32>,
    max_mps: Option<f32>,
    gesture: Option<String>,
) -> Result<ProjectSummary> {
    let after = match (min_mps, max_mps) {
        (Some(min), Some(max)) if min.is_finite() && max.is_finite() => {
            // Ordered here rather than refused: the panel has a field for each
            // end, and a low end typed above the high one is a band written
            // backwards, not a mistake. (The sliders cannot cross: a dragged
            // thumb stops at the other one.)
            Some(ve_core::document::SpeedRange {
                min_mps: min.min(max).max(0.0),
                max_mps: max.max(min),
            })
        }
        _ => None,
    };
    apply_with(state, gesture, |project| {
        let found = project
            .layer(object_id(layer))
            .ok_or_else(|| missing_layer(layer))?;
        Ok(Command::SetLayerSpeedRange {
            layer: found.id,
            before: found.speed_range,
            after,
        })
    })
}

/// Shows or hides a layer.
#[tauri::command]
pub fn set_layer_visible(
    state: tauri::State<'_, AppState>,
    layer: u64,
    visible: bool,
) -> Result<ProjectSummary> {
    layer_visibility(&state, layer, visible)
}

/// Implementation of [`set_layer_visible`].
pub fn layer_visibility(state: &AppState, layer: u64, visible: bool) -> Result<ProjectSummary> {
    apply(state, |project| {
        let found = project
            .layer(object_id(layer))
            .ok_or_else(|| missing_layer(layer))?;
        Ok(Command::SetLayerVisible {
            layer: found.id,
            before: found.visible,
            after: visible,
        })
    })
}

/// Says which field a painted layer is part of (M29): `"wind"` or `"current"`.
#[tauri::command]
pub fn set_layer_parameter(
    state: tauri::State<'_, AppState>,
    layer: u64,
    parameter: String,
) -> Result<ProjectSummary> {
    layer_parameter(&state, layer, &parameter)
}

/// Implementation of [`set_layer_parameter`]. A raster layer's kind is its
/// file's and an image has none, so only a painted layer takes it.
pub fn layer_parameter(state: &AppState, layer: u64, parameter: &str) -> Result<ProjectSummary> {
    let after = crate::projects::parse_field_kind(parameter)?;
    apply(state, |project| {
        let found = project
            .layer(object_id(layer))
            .ok_or_else(|| missing_layer(layer))?;
        if !matches!(found.source, ve_core::document::LayerSource::Painted) {
            return Err(AppError::BadOption {
                field: "layer",
                value: "an imported layer's field is its file's".to_owned(),
            });
        }
        Ok(Command::SetLayerParameter {
            layer: found.id,
            before: found.parameter,
            after,
        })
    })
}

/// Locks or unlocks a layer.
#[tauri::command]
pub fn set_layer_locked(
    state: tauri::State<'_, AppState>,
    layer: u64,
    locked: bool,
) -> Result<ProjectSummary> {
    layer_lock(&state, layer, locked)
}

/// Implementation of [`set_layer_locked`].
pub fn layer_lock(state: &AppState, layer: u64, locked: bool) -> Result<ProjectSummary> {
    apply(state, |project| {
        let found = project
            .layer(object_id(layer))
            .ok_or_else(|| missing_layer(layer))?;
        Ok(Command::SetLayerLocked {
            layer: found.id,
            before: found.locked,
            after: locked,
        })
    })
}

/// Reorders the layer stack. Index 0 is the bottom.
#[tauri::command]
pub fn move_layer(
    state: tauri::State<'_, AppState>,
    from: usize,
    to: usize,
) -> Result<ProjectSummary> {
    layer_move(&state, from, to)
}

/// Implementation of [`move_layer`].
pub fn layer_move(state: &AppState, from: usize, to: usize) -> Result<ProjectSummary> {
    apply(state, |project| {
        let len = project.layers.len();
        if from >= len || to >= len {
            return Err(AppError::Core(ve_core::CoreError::IndexOutOfBounds {
                index: from.max(to),
                len,
            }));
        }
        Ok(Command::MoveLayer { from, to })
    })
}

/// Renames an object.
#[tauri::command]
pub fn rename_object(
    state: tauri::State<'_, AppState>,
    object: u64,
    name: String,
) -> Result<ProjectSummary> {
    object_rename(&state, object, name)
}

/// Implementation of [`rename_object`].
pub fn object_rename(state: &AppState, object: u64, name: String) -> Result<ProjectSummary> {
    apply(state, |project| {
        let found = project
            .object(object_id(object))
            .ok_or_else(|| missing_object(object))?;
        Ok(Command::RenameObject {
            object: found.id,
            before: found.name.clone(),
            after: name,
        })
    })
}

/// Deletes an object.
#[tauri::command]
pub fn remove_object(state: tauri::State<'_, AppState>, object: u64) -> Result<ProjectSummary> {
    object_remove(&state, object)
}

/// Implementation of [`remove_object`].
///
/// An object that others follow is deleted with them unlinked first, in the
/// same history entry: the followers keep where they stood rather than
/// snapping back to the dormant keys underneath (spec.md 9.3, M13). One undo
/// puts the object back and re-links them.
pub fn object_remove(state: &AppState, object: u64) -> Result<ProjectSummary> {
    apply(state, |project| {
        let id = object_id(object);
        let (layer_index, index) = project.locate(id).ok_or_else(|| missing_object(object))?;
        let remove = Command::RemoveObject {
            layer: project.layers[layer_index].id,
            index,
            object: Box::new(project.layers[layer_index].objects[index].clone()),
        };
        let mut commands = unlink_followers_of(project, id);
        if commands.is_empty() {
            return Ok(remove);
        }
        commands.push(remove);
        Ok(Command::Batch {
            label: "Delete object".to_owned(),
            commands,
        })
    })
}

/// Deletes several objects as one history entry.
#[tauri::command]
pub fn remove_objects(
    state: tauri::State<'_, AppState>,
    objects: Vec<u64>,
) -> Result<ProjectSummary> {
    objects_remove(&state, &objects)
}

/// Implementation of [`remove_objects`].
///
/// `Delete` on a selection: every member goes in one `Command::Batch`, so one
/// undo returns all of them (spec.md 8.4), with their followers freed first
/// as [`object_remove`] frees them. The removals are ordered **bottom of
/// each layer last** — highest index first — because each one shifts the
/// indices above it, and a batch applies in order.
pub fn objects_remove(state: &AppState, objects: &[u64]) -> Result<ProjectSummary> {
    if objects.is_empty() {
        return with_session(state, |session| {
            Ok(ProjectSummary::of(session.require_open()?))
        });
    }
    apply(state, |project| {
        let mut located = Vec::new();
        for raw in objects {
            let id = object_id(*raw);
            let (layer_index, index) = project.locate(id).ok_or_else(|| missing_object(*raw))?;
            if !located
                .iter()
                .any(|&(_, l, i)| (l, i) == (layer_index, index))
            {
                located.push((id, layer_index, index));
            }
        }
        let mut commands = Vec::new();
        for (id, _, _) in &located {
            commands.extend(unlink_followers_of(project, *id));
        }
        located.sort_by_key(|entry| std::cmp::Reverse((entry.1, entry.2)));
        for (_, layer_index, index) in located {
            commands.push(Command::RemoveObject {
                layer: project.layers[layer_index].id,
                index,
                object: Box::new(project.layers[layer_index].objects[index].clone()),
            });
        }
        if commands.len() == 1 {
            return Ok(commands.remove(0));
        }
        Ok(Command::Batch {
            label: if objects.len() == 1 {
                "Delete object".to_owned()
            } else {
                format!("Delete {} objects", objects.len())
            },
            commands,
        })
    })
}

/// The eraser's stroke (spec.md 8.1, M29): a brush that takes away.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "EraseStroke.ts")]
pub struct EraseStroke {
    /// Pointer positions as `[lon, lat]`, in the order they were drawn.
    pub points: Vec<[f64; 2]>,
    /// The stamp's radius on the ground, in kilometres. A size in pixels
    /// has already become kilometres at the latitude the stroke began.
    pub radius_km: f64,
    /// A square stamp rather than a disc.
    pub square: bool,
    /// Edge falloff, 0 to 1.
    pub feather: f32,
    /// The one step to erase from, or every step.
    pub step: Option<u32>,
    /// The step the stroke was drawn at: the frame each object is measured
    /// in, since an object that moves is somewhere else at every step.
    pub at_step: u32,
    /// The layer it acts on — the active one; the creation rule's when absent.
    pub layer: Option<u64>,
}

/// Erases what the stroke covers in one layer (spec.md 8.1, M29).
#[tauri::command(async)]
pub fn erase_stroke(
    state: tauri::State<'_, AppState>,
    stroke: EraseStroke,
) -> Result<ProjectSummary> {
    stroke_erase(&state, stroke)
}

/// Implementation of [`erase_stroke`].
///
/// The eraser makes no object. What it does depends on what is under it:
///
/// - a **painted object** gains an [`Erasure`](ve_core::document::Erasure) —
///   the stamp in the object's own frame, so it travels with the object —
///   and its coverage is multiplied by what the erasures leave; an object
///   with nothing left of it is **deleted** instead;
/// - a **patch or macro** has its captured samples rewritten to undefined
///   where the stamp falls, as a new capture with its own hash, so the erase
///   is in the data and not layered over it;
/// - an **imported layer** gains a stamp in geographic space, applied when
///   its lattice is sampled, since its samples are its file's and are read
///   back on open (invariants 1 and 2).
///
/// One `Batch` per sweep, so one undo returns the whole stroke.
pub fn stroke_erase(state: &AppState, stroke: EraseStroke) -> Result<ProjectSummary> {
    use ve_core::capture::{Capture, CaptureLattice, UNDEFINED};
    use ve_core::document::{Erasure, LayerSource, LocalPoint, RasterErasure};
    use ve_render::aeqd::M_PER_DEGREE;
    use ve_render::cpu::{erased_factor, raster_erased};
    use ve_render::scene::{FlatObject, FlatRasterErasure, flatten_object};

    let points: Vec<LonLat> = stroke
        .points
        .iter()
        .map(|pair| LonLat::new(pair[0], pair[1]).map_err(AppError::Core))
        .collect::<Result<_>>()?;
    let radius_m = stroke.radius_km * 1000.0;
    if points.is_empty() || !radius_m.is_finite() || radius_m <= 0.0 {
        return with_session(state, |session| {
            Ok(ProjectSummary::of(session.require_open()?))
        });
    }
    let feather = stroke.feather.clamp(0.0, 1.0);

    /// Whether anything of the object is left: its footprint, sampled on a
    /// lattice over its bounding radius, has no point the erasures leave
    /// more than a trace of.
    fn fully_erased(flat: &FlatObject) -> bool {
        let reach = flat.shape.bounding_radius_m();
        if !reach.is_finite() || reach <= 0.0 {
            return true;
        }
        const N: i32 = 40;
        for j in -N..=N {
            for i in -N..=N {
                let local = [
                    f64::from(i) / f64::from(N) * reach,
                    f64::from(j) / f64::from(N) * reach,
                ];
                if flat.shape.distance(local) <= 0.0 && erased_factor(&flat.erased, local) > 0.02 {
                    return false;
                }
            }
        }
        true
    }

    with_session(state, |session| {
        let open = session.require_open()?;
        let (mut commands, mut removals, new_captures) = {
            let project = &open.project;
            let layer = match stroke.layer {
                Some(raw) => project
                    .layer(object_id(raw))
                    .ok_or_else(|| missing_layer(raw))?,
                None => creation_layer(project, None)?,
            };
            if layer.locked {
                return Err(AppError::BadOption {
                    field: "layer",
                    value: format!("{} is locked", layer.name),
                });
            }
            let mut commands = Vec::new();
            let mut removals: Vec<(usize, Command)> = Vec::new();
            let mut new_captures: Vec<std::sync::Arc<Capture>> = Vec::new();

            if matches!(layer.source, LayerSource::Grib { .. }) {
                let mut after = layer.erased.clone();
                after.push(RasterErasure {
                    chains: vec![points.clone()],
                    radius_m,
                    square: stroke.square,
                    feather,
                    step: stroke.step,
                });
                commands.push(Command::SetRasterErasures {
                    layer: layer.id,
                    before: layer.erased.clone(),
                    after,
                });
            } else if !matches!(layer.source, LayerSource::Image { .. }) {
                let on_ground = FlatRasterErasure {
                    chains: vec![points.clone()],
                    radius_m,
                    square: stroke.square,
                    feather: f64::from(feather),
                };
                for (index, object) in layer.objects.iter().enumerate() {
                    let Some(flat) = flatten_object(object, stroke.at_step) else {
                        continue;
                    };
                    if let Some(hash) = &object.capture {
                        // A patch or a macro: the samples themselves.
                        let Some(capture) = project.captures.get(hash) else {
                            continue;
                        };
                        let hours = f64::from(project.settings.step_hours.hours());
                        let only_offset = stroke.step.map(|at| {
                            f64::from(at.saturating_sub(object.active_range.start)) * hours
                        });
                        let mut frames = capture.frames.clone();
                        let mut touched = false;
                        let mut anything_left = false;
                        for frame in &mut frames {
                            let this_frame = only_offset
                                .is_none_or(|offset| (frame.offset_hours - offset).abs() < 1e-6);
                            for j in 0..capture.nj {
                                for i in 0..capture.ni {
                                    let at = (j * capture.ni + i) as usize;
                                    let Some(uv) = frame.uv.get_mut(at) else {
                                        continue;
                                    };
                                    if this_frame {
                                        let local = [
                                            (capture.x0_deg + f64::from(i) * capture.spacing_deg)
                                                * M_PER_DEGREE,
                                            (capture.y0_deg - f64::from(j) * capture.spacing_deg)
                                                * M_PER_DEGREE,
                                        ];
                                        let here = flat.frame.to_global(local);
                                        if raster_erased(std::slice::from_ref(&on_ground), here)
                                            && !uv[0].is_nan()
                                        {
                                            *uv = UNDEFINED;
                                            touched = true;
                                        }
                                    }
                                    if !uv[0].is_nan() {
                                        anything_left = true;
                                    }
                                }
                            }
                        }
                        if !touched {
                            continue;
                        }
                        if !anything_left {
                            removals.push((
                                index,
                                Command::RemoveObject {
                                    layer: layer.id,
                                    index,
                                    object: Box::new(object.clone()),
                                },
                            ));
                            continue;
                        }
                        let rewritten = Capture::new(
                            capture.kind,
                            CaptureLattice {
                                ni: capture.ni,
                                nj: capture.nj,
                                spacing_deg: capture.spacing_deg,
                                x0_deg: capture.x0_deg,
                                y0_deg: capture.y0_deg,
                            },
                            capture.seconds_per_frame,
                            capture.shape.clone(),
                            frames,
                        )
                        .map_err(AppError::Core)?;
                        commands.push(Command::SetCapture {
                            object: object.id,
                            before: Some(hash.clone()),
                            after: Some(rewritten.hash.clone()),
                        });
                        new_captures.push(std::sync::Arc::new(rewritten));
                        continue;
                    }

                    // A painted object: the stamp in its own frame.
                    let chain: Vec<[f64; 2]> =
                        points.iter().map(|p| flat.frame.to_local(*p)).collect();
                    let radius_local = radius_m / flat.frame.scale;
                    let reach = radius_local * (1.0 + f64::from(feather));
                    // Touched if any point along the stroke, sampled at half a
                    // radius, comes within the stamp of the footprint.
                    let mut touched = false;
                    'chain: for pair in chain
                        .windows(2)
                        .chain(std::iter::once(&chain[..1.min(chain.len())]))
                    {
                        let (a, b) = (pair[0], *pair.get(1).unwrap_or(&pair[0]));
                        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
                        let steps = (length / (radius_local / 2.0)).ceil().max(1.0) as usize;
                        for k in 0..=steps {
                            let t = k as f64 / steps as f64;
                            let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                            if flat.shape.distance(q) <= reach {
                                touched = true;
                                break 'chain;
                            }
                        }
                    }
                    if !touched {
                        continue;
                    }
                    let mut after = object.erased.clone();
                    after.push(Erasure {
                        chains: vec![
                            chain
                                .iter()
                                .map(|p| LocalPoint { x: p[0], y: p[1] })
                                .collect(),
                        ],
                        radius_m: radius_local,
                        square: stroke.square,
                        feather,
                        step: stroke.step,
                    });
                    // Nothing left of it, at every step, is a deletion.
                    let mut probe = object.clone();
                    probe.erased = after.clone();
                    let gone = stroke.step.is_none()
                        && flatten_object(&probe, stroke.at_step).is_none_or(|f| fully_erased(&f));
                    if gone {
                        commands.extend(unlink_followers_of(project, object.id));
                        removals.push((
                            index,
                            Command::RemoveObject {
                                layer: layer.id,
                                index,
                                object: Box::new(object.clone()),
                            },
                        ));
                    } else {
                        commands.push(Command::SetErasures {
                            object: object.id,
                            before: object.erased.clone(),
                            after,
                        });
                    }
                }
            }
            (commands, removals, new_captures)
        };

        // Removals last and from the bottom of the layer up: each shifts the
        // indices above it, and a batch applies in order.
        removals.sort_by_key(|(index, _)| std::cmp::Reverse(*index));
        commands.extend(removals.into_iter().map(|(_, command)| command));
        if commands.is_empty() {
            return Ok(ProjectSummary::of(session.require_open()?));
        }
        // The rewritten samples go into the project before the command that
        // names them, keyed by hash, like a pasted capture's.
        for capture in new_captures {
            open.project
                .captures
                .entry(capture.hash.clone())
                .or_insert(capture);
        }
        let command = if commands.len() == 1 {
            commands.remove(0)
        } else {
            Command::Batch {
                label: "Erase".to_owned(),
                commands,
            }
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// The commands that free every follower of `primary`, holding each where it
/// stands.
///
/// The step used is 0: a follower's dormant keys are woken by the unlink, and
/// whichever step the value is written at, the rest of its own animation
/// resumes from there. Step 0 is the one step every project has.
fn unlink_followers_of(project: &Project, primary: ve_core::Id) -> Vec<Command> {
    use ve_core::follow;
    let resolved = follow::resolve(project, 0);
    let mut out = Vec::new();
    for layer in &project.layers {
        for object in &layer.objects {
            for prop in [PropId::Position, PropId::RotationDeg] {
                let Some(link) = follow::follow_of(object, prop) else {
                    continue;
                };
                if link.primary != primary {
                    continue;
                }
                let Some(before) = object.props.get(prop) else {
                    continue;
                };
                let derived = resolved.of(object.id);
                let held = match prop {
                    PropId::Position => derived.position.map(ve_core::PropValue::LonLat),
                    _ => derived
                        .rotation
                        .map(|d| ve_core::PropValue::Angle(ve_core::angle::Angle::new(d))),
                };
                let mut after = before.clone();
                after.set_follow(None);
                if let Some(value) = held {
                    after = crate::animation::written(&after, 0, true, value);
                    after.set_follow(None);
                }
                out.push(Command::SetFollow {
                    object: object.id,
                    prop,
                    before: Box::new(before.clone()),
                    after: Box::new(after),
                });
            }
        }
    }
    out
}

/// Moves an object to a position in a layer. Index 0 is the bottom.
#[tauri::command]
pub fn move_object(
    state: tauri::State<'_, AppState>,
    object: u64,
    layer: u64,
    index: usize,
) -> Result<ProjectSummary> {
    object_move(&state, object, layer, index)
}

/// Implementation of [`move_object`].
pub fn object_move(
    state: &AppState,
    object: u64,
    layer: u64,
    index: usize,
) -> Result<ProjectSummary> {
    apply(state, |project| {
        let id = object_id(object);
        let (from_layer, from_index) = project.locate(id).ok_or_else(|| missing_object(object))?;
        // The destination is a creation target like any other (D66): an
        // object dragged onto an imported layer is refused the same way one
        // painted onto it is.
        let destination = creation_layer(project, Some(layer))?;
        Ok(Command::MoveObject {
            object: id,
            from: (project.layers[from_layer].id, from_index),
            to: (destination.id, index),
        })
    })
}

/// Duplicates an object, placing the copy directly above the original.
#[tauri::command]
pub fn duplicate_object(state: tauri::State<'_, AppState>, object: u64) -> Result<ProjectSummary> {
    object_duplicate(&state, object)
}

/// Implementation of [`duplicate_object`].
pub fn object_duplicate(state: &AppState, object: u64) -> Result<ProjectSummary> {
    apply(state, |project| {
        let id = object_id(object);
        let (layer_index, index) = project.locate(id).ok_or_else(|| missing_object(object))?;

        let target = creation_layer(project, Some(project.layers[layer_index].id.raw()))?;

        let mut copy = project.layers[layer_index].objects[index].clone();
        // A fresh identity, or the two would be the same object to every
        // command that follows.
        copy.id = ve_core::Id::new();
        copy.name = format!("{} copy", copy.name);

        Ok(Command::AddObject {
            layer: target.id,
            index: index + 1,
            object: Box::new(copy),
        })
    })
}

/// Changes an object's lifetime.
#[tauri::command]
pub fn set_active_range(
    state: tauri::State<'_, AppState>,
    object: u64,
    start: u32,
    end: u32,
) -> Result<ProjectSummary> {
    object_range(&state, object, start, end)
}

/// Implementation of [`set_active_range`].
pub fn object_range(state: &AppState, object: u64, start: u32, end: u32) -> Result<ProjectSummary> {
    apply(state, |project| {
        let last = project.last_step();
        let found = project
            .object(object_id(object))
            .ok_or_else(|| missing_object(object))?;
        Ok(Command::SetActiveRange {
            object: found.id,
            before: found.active_range,
            after: StepRange::new(start.min(last), end.min(last)),
        })
    })
}

/// The topmost object covering a position, if any.
#[tauri::command]
pub fn object_at(
    state: tauri::State<'_, AppState>,
    lon: f64,
    lat: f64,
    step: u32,
) -> Result<Option<u64>> {
    hit_test(&state, lon, lat, step)
}

/// Implementation of [`object_at`], callable without a Tauri handle.
///
/// Searches top-down and stops at the first hit, so a click selects what the
/// user can see rather than something buried beneath it. Objects on hidden or
/// locked layers are skipped: if you cannot see it or edit it, you cannot
/// select it by clicking.
pub fn hit_test(state: &AppState, lon: f64, lat: f64, step: u32) -> Result<Option<u64>> {
    let position = LonLat::new(lon, lat)?;
    with_session(state, |session| {
        let project = &session.require_open()?.project;
        let links = ve_core::follow::resolve(project, step);

        for layer in project.layers.iter().rev() {
            if !layer.visible || layer.locked {
                continue;
            }
            for object in layer.objects.iter().rev() {
                // A follower is where its link puts it, not where its dormant
                // keys say (spec.md 9.3): picking has to agree with drawing.
                let Some(flat) =
                    ve_render::scene::flatten_object_at(object, step, links.of(object.id))
                else {
                    continue;
                };
                if ve_render::scene::covers(&flat, position) {
                    return Ok(Some(object.id.raw()));
                }
            }
        }
        Ok(None)
    })
}

// --- Clipboard and history --------------------------------------------------

use ve_core::clipboard::{Clipboard, PasteTiming};

/// What the clipboard holds.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ClipboardState.ts")]
pub struct ClipboardState {
    /// Objects available to paste.
    pub count: usize,
}

/// One entry in the undo history.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "HistoryEntry.ts")]
pub struct HistoryEntry {
    /// Position in the stack, oldest first.
    pub index: usize,
    /// What the change was.
    pub label: String,
    /// Whether it is currently applied.
    pub applied: bool,
}

/// The undo history, for the history panel.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "HistoryView.ts")]
pub struct HistoryView {
    /// Entries, oldest first.
    pub entries: Vec<HistoryEntry>,
    /// How many are applied. Entries at or past this index can be redone.
    pub cursor: usize,
}

/// Copies objects to the clipboard.
#[tauri::command]
pub fn copy_objects(
    state: tauri::State<'_, AppState>,
    objects: Vec<u64>,
    step: u32,
) -> Result<ClipboardState> {
    clipboard_copy(&state, &objects, step)
}

/// Implementation of [`copy_objects`], callable without a Tauri handle.
pub fn clipboard_copy(state: &AppState, objects: &[u64], step: u32) -> Result<ClipboardState> {
    with_session(state, |session| {
        let project = &session.require_open()?.project;
        let copied: Vec<_> = objects
            .iter()
            .filter_map(|raw| project.object(object_id(*raw)).cloned())
            .collect();
        let count = copied.len();
        session.clipboard = Clipboard::copy(copied, step);
        // One clipboard (spec.md 8.5): what was copied last is what pastes.
        // A capture taken earlier would otherwise keep answering `Cmd`-`V`
        // for the rest of the session, which is how objects stopped copying.
        session.capture.held = None;
        Ok(ClipboardState { count })
    })
}

/// Copies objects and then deletes them.
#[tauri::command]
pub fn cut_objects(
    state: tauri::State<'_, AppState>,
    objects: Vec<u64>,
    step: u32,
) -> Result<ProjectSummary> {
    clipboard_cut(&state, &objects, step)
}

/// Implementation of [`cut_objects`], callable without a Tauri handle.
pub fn clipboard_cut(state: &AppState, objects: &[u64], step: u32) -> Result<ProjectSummary> {
    clipboard_copy(state, objects, step)?;
    let mut summary = None;
    for raw in objects {
        summary = Some(object_remove(state, *raw)?);
    }
    match summary {
        Some(summary) => Ok(summary),
        None => with_session(state, |session| {
            Ok(ProjectSummary::of(session.require_open()?))
        }),
    }
}

/// Pastes the clipboard into a layer at a step.
#[tauri::command]
pub fn paste_objects(
    state: tauri::State<'_, AppState>,
    layer: Option<u64>,
    step: u32,
    absolute_timing: bool,
    still: bool,
) -> Result<ProjectSummary> {
    clipboard_paste(&state, layer, step, absolute_timing, still)
}

/// Implementation of [`paste_objects`], callable without a Tauri handle.
///
/// `still` pastes the copy as it was at the step it was copied from, with no
/// animation at all (spec.md 8.5, M27); it wins over `absolute_timing`, since
/// a still has no timing to keep.
pub fn clipboard_paste(
    state: &AppState,
    layer: Option<u64>,
    step: u32,
    absolute_timing: bool,
    still: bool,
) -> Result<ProjectSummary> {
    with_session(state, |session| {
        if session.clipboard.is_empty() {
            return Err(AppError::Internal("the clipboard is empty".to_owned()));
        }

        let timing = if still {
            PasteTiming::Still
        } else if absolute_timing {
            PasteTiming::Absolute
        } else {
            PasteTiming::Relative
        };

        // Read the clipboard before borrowing the project mutably.
        let nudge = session.clipboard.source_step() == step;

        let (target, index, last_step) = {
            let project = &session.require_open()?.project;
            let target = creation_layer(project, layer)?;
            (target.id, target.objects.len(), project.last_step())
        };

        // Pasting back where it came from would hide the copy underneath the
        // original, so it is nudged aside (spec.md 8.5).
        let objects = session.clipboard.paste(step, timing, last_step, nudge);
        let command = Command::AddObjects {
            layer: target,
            index,
            objects,
        };

        let open = session.require_open()?;
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// What the clipboard holds.
#[tauri::command]
pub fn clipboard_state(state: tauri::State<'_, AppState>) -> Result<ClipboardState> {
    with_session(&state, |session| {
        Ok(ClipboardState {
            count: session.clipboard.len(),
        })
    })
}

/// Which of the two things `Cmd`-`V` can paste is held (spec.md 8.5).
///
/// One clipboard, two kinds of content: objects, or a field captured from a
/// region. Copying either drops the other, so at most one is ever held and
/// the key can ask rather than remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "ClipboardKind.ts")]
#[serde(rename_all = "snake_case")]
pub enum ClipboardKind {
    /// Nothing to paste.
    Empty,
    /// Objects, keyframes and all.
    Objects,
    /// A captured field, pasted as a patch.
    Capture,
}

/// What kind of thing a paste would put down.
#[tauri::command]
pub fn clipboard_kind(state: tauri::State<'_, AppState>) -> Result<ClipboardKind> {
    kind_held(&state)
}

/// Implementation of [`clipboard_kind`].
pub fn kind_held(state: &AppState) -> Result<ClipboardKind> {
    with_session(state, |session| {
        Ok(if session.capture.held.is_some() {
            ClipboardKind::Capture
        } else if session.clipboard.is_empty() {
            ClipboardKind::Empty
        } else {
            ClipboardKind::Objects
        })
    })
}

/// The undo history, for the history panel.
#[tauri::command]
pub fn history_view(state: tauri::State<'_, AppState>) -> Result<HistoryView> {
    history_of(&state)
}

/// Implementation of [`history_view`], callable without a Tauri handle.
pub fn history_of(state: &AppState) -> Result<HistoryView> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let cursor = open.history.cursor();
        Ok(HistoryView {
            entries: open
                .history
                .entries()
                .iter()
                .enumerate()
                .map(|(index, entry)| HistoryEntry {
                    index,
                    label: entry.label.clone(),
                    applied: index < cursor,
                })
                .collect(),
            cursor,
        })
    })
}

/// Moves to a point in the history, undoing or redoing as needed.
#[tauri::command]
pub fn jump_to_history(state: tauri::State<'_, AppState>, target: usize) -> Result<ProjectSummary> {
    history_jump(&state, target)
}

/// Implementation of [`jump_to_history`], callable without a Tauri handle.
pub fn history_jump(state: &AppState, target: usize) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let (project, history) = (&mut open.project, &mut open.history);
        history.jump_to(project, target)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}
