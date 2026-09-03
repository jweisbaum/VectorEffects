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
    fn of(value: PropValue) -> Self {
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
    fn into_prop(self, expected: PropKind) -> Result<PropValue> {
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
}

fn tool_name(tool: ToolKind) -> &'static str {
    match tool {
        ToolKind::Brush => "brush",
        ToolKind::Circle => "circle",
        ToolKind::ShapeFill => "shape_fill",
        ToolKind::Eraser => "eraser",
        ToolKind::CloneStamp => "clone_stamp",
        ToolKind::Curve => "curve",
    }
}

fn unit_name(unit: ve_core::schema::Unit) -> &'static str {
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
    gesture: Option<String>,
) -> Result<ProjectSummary> {
    set_property_with(&state, object, &property, value, gesture)
}

/// Implementation of [`set_object_property`] without a gesture.
pub fn set_property(
    state: &AppState,
    object: u64,
    property: &str,
    value: PropertyValue,
) -> Result<ProjectSummary> {
    set_property_with(state, object, property, value, None)
}

/// Implementation of [`set_object_property`], callable without a Tauri handle.
pub fn set_property_with(
    state: &AppState,
    object: u64,
    property: &str,
    value: PropertyValue,
    gesture: Option<String>,
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

        let mut after = before.clone();
        after.set_base(value.into_prop(before.kind())?);

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
    with_session(state, |session| {
        let command = build(&session.require_open()?.project)?;
        let open = session.require_open()?;
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
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
            layer: Box::new(Layer::new(name)),
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
pub fn object_remove(state: &AppState, object: u64) -> Result<ProjectSummary> {
    apply(state, |project| {
        let id = object_id(object);
        let (layer_index, index) = project.locate(id).ok_or_else(|| missing_object(object))?;
        Ok(Command::RemoveObject {
            layer: project.layers[layer_index].id,
            index,
            object: Box::new(project.layers[layer_index].objects[index].clone()),
        })
    })
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
        let destination = project
            .layer(object_id(layer))
            .ok_or_else(|| missing_layer(layer))?;
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

        let mut copy = project.layers[layer_index].objects[index].clone();
        // A fresh identity, or the two would be the same object to every
        // command that follows.
        copy.id = ve_core::Id::new();
        copy.name = format!("{} copy", copy.name);

        Ok(Command::AddObject {
            layer: project.layers[layer_index].id,
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

        for layer in project.layers.iter().rev() {
            if !layer.visible || layer.locked {
                continue;
            }
            for object in layer.objects.iter().rev() {
                let Some(flat) = ve_render::scene::flatten_object(object, step) else {
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
) -> Result<ProjectSummary> {
    clipboard_paste(&state, layer, step, absolute_timing)
}

/// Implementation of [`paste_objects`], callable without a Tauri handle.
pub fn clipboard_paste(
    state: &AppState,
    layer: Option<u64>,
    step: u32,
    absolute_timing: bool,
) -> Result<ProjectSummary> {
    with_session(state, |session| {
        if session.clipboard.is_empty() {
            return Err(AppError::Internal("the clipboard is empty".to_owned()));
        }

        let timing = if absolute_timing {
            PasteTiming::Absolute
        } else {
            PasteTiming::Relative
        };

        // Read the clipboard before borrowing the project mutably.
        let nudge = session.clipboard.source_step() == step;

        let (target, index, last_step) = {
            let project = &session.require_open()?.project;
            let target = match layer {
                Some(raw) => project
                    .layer(object_id(raw))
                    .ok_or_else(|| missing_layer(raw))?,
                // No layer named: the top one, which is where a new object goes.
                None => project
                    .layers
                    .last()
                    .ok_or_else(|| AppError::Internal("project has no layers".to_owned()))?,
            };
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
