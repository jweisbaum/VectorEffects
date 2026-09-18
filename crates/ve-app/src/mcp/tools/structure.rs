//! The structure group (spec.md 8.8): layers and the objects on them.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;

use super::events;
use super::{ObjectStepParams, StepParams, ToolError, VectorEffects};
use crate::commands::AppState;
use crate::error::AppError;
use crate::projects::ProjectSummary;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NameParams {
    pub name: String,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LayerParams {
    pub layer: u64,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectParams {
    pub object: u64,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectsParams {
    pub objects: Vec<u64>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveParams {
    pub from: usize,
    pub to: usize,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LayerSetParams {
    pub layer: u64,
    pub name: Option<String>,
    pub visible: Option<bool>,
    pub locked: Option<bool>,
    pub parameter: Option<String>,
    pub min_mps: Option<f32>,
    pub max_mps: Option<f32>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectsListParams {
    pub step: u32,
    pub layer: Option<u64>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectCreateParams {
    /// A tool name from `tool_catalogue`: "circle", "brush", "curve", …
    pub tool: String,
    /// Where it is drawn, in the form the catalogue names for the tool.
    /// Coordinates are `[lon, lat]` in degrees, longitude in -180..180.
    #[schemars(with = "crate::create::Gesture")]
    pub gesture: Value,
    /// The tool's options; any left out take the catalogue's defaults.
    #[serde(default)]
    #[schemars(with = "Vec<crate::create::ToolOption>")]
    pub options: Value,
    /// A layer id from `layers_list`; null or absent is the top layer.
    pub layer: Option<u64>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectSetParams {
    pub object: u64,
    /// The step the change is made at, from 0 to step_count - 1.
    pub step: u32,
    /// Property id (`object_get`'s `id`) to tagged value.
    #[schemars(with = "std::collections::BTreeMap<String, crate::document::PropertyValue>")]
    pub values: Value,
    #[serde(default)]
    pub auto_key: bool,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectMoveParams {
    pub object: u64,
    pub layer: u64,
    pub index: usize,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegionParams {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
    pub step: u32,
    pub layer: Option<u64>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct ObjectsList {
    pub objects: Vec<crate::document::ObjectNode>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct ObjectProperties {
    pub properties: Vec<crate::document::PropertyView>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct ObjectIds {
    pub objects: Vec<u64>,
}

#[tool_router(router = tool_router_structure, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    #[tool(
        description = "Layers and their objects at a step: ids, names, kinds, visibility, locks. Ids are what every other tool takes."
    )]
    async fn layers_list(
        &self,
        Parameters(p): Parameters<StepParams>,
    ) -> std::result::Result<Json<crate::document::DocumentTree>, ToolError> {
        self.run("layers_list", move |app| {
            crate::document::document_tree(app.state(), p.step)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Adds an empty painted layer on top and returns the summary.")]
    async fn layer_add(
        &self,
        Parameters(p): Parameters<NameParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("layer_add", false, move |app| {
            crate::document::add_layer(app.state(), p.name)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Sets any of a layer's name, visibility, lock, parameter (\"wind\"/\"current\") or speed range in m/s; each field set is its own undo step. Giving only one of min_mps/max_mps keeps the layer's other current bound; refused (writing nothing) if the layer has no existing band to fill it from. Omitted fields are left alone."
    )]
    async fn layer_set(
        &self,
        Parameters(p): Parameters<LayerSetParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("layer_set", false, move |app| {
            let state = app.state::<AppState>();

            // Resolved and validated before any sub-write runs (ruling,
            // task 3 review, fix round 1, findings 1 and 2c): a lone bound
            // fills its partner from the layer's *current* band rather than
            // clearing it, since `set_layer_speed_range` treats anything but
            // (Some, Some) as "no filter". A lone bound with no existing band
            // to fill from is refused before `name`/`visible`/`locked`/
            // `parameter` ever write, so a bad call leaves nothing committed.
            let bounds = if p.min_mps.is_some() || p.max_mps.is_some() {
                let tree = crate::document::tree(state.inner(), 0)?;
                let layer = tree
                    .layers
                    .iter()
                    .find(|l| l.id == p.layer)
                    .ok_or(AppError::Core(ve_core::CoreError::MissingLayer(p.layer)))?;
                let current = layer.speed_filter.as_ref();
                let min = p
                    .min_mps
                    .or_else(|| current.and_then(|f| f.speed_min_mps));
                let max = p
                    .max_mps
                    .or_else(|| current.and_then(|f| f.speed_max_mps));
                match (min, max) {
                    (Some(min), Some(max)) => Some((min, max)),
                    _ => {
                        return Err(AppError::BadOption {
                            field: "min_mps/max_mps",
                            value: "one bound was given but the layer has no band to fill the other end from".to_owned(),
                        });
                    }
                }
            } else {
                None
            };

            let mut last = None;
            if let Some(name) = p.name {
                last = Some(crate::document::rename_layer(state.clone(), p.layer, name)?);
            }
            if let Some(visible) = p.visible {
                last = Some(crate::document::set_layer_visible(
                    state.clone(),
                    p.layer,
                    visible,
                )?);
            }
            if let Some(locked) = p.locked {
                last = Some(crate::document::set_layer_locked(
                    state.clone(),
                    p.layer,
                    locked,
                )?);
            }
            if let Some(parameter) = p.parameter {
                last = Some(crate::document::set_layer_parameter(
                    state.clone(),
                    p.layer,
                    parameter,
                )?);
            }
            if let Some((min, max)) = bounds {
                last = Some(crate::document::set_layer_speed_range(
                    state.clone(),
                    p.layer,
                    Some(min),
                    Some(max),
                    None,
                )?);
            }
            match last {
                Some(summary) => Ok(summary),
                None => crate::projects::current_project(state)?.ok_or(AppError::NoProjectOpen),
            }
        })
        .await
        .map(Json)
    }

    #[tool(description = "Moves a layer from one index to another. Index 0 is the bottom.")]
    async fn layer_move(
        &self,
        Parameters(p): Parameters<MoveParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("layer_move", false, move |app| {
            crate::document::move_layer(app.state(), p.from, p.to)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Removes a layer and everything on it. Undoable.")]
    async fn layer_remove(
        &self,
        Parameters(p): Parameters<LayerParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("layer_remove", false, move |app| {
            crate::document::remove_layer(app.state(), p.layer)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Objects on one layer, or on every layer, at a step.")]
    async fn objects_list(
        &self,
        Parameters(p): Parameters<ObjectsListParams>,
    ) -> std::result::Result<Json<ObjectsList>, ToolError> {
        let layer = p.layer;
        let tree = self
            .run("objects_list", move |app| {
                crate::document::document_tree(app.state(), p.step)
            })
            .await?;
        let objects = tree
            .layers
            .into_iter()
            .filter(|l| layer.is_none_or(|id| id == l.id))
            .flat_map(|l| l.objects)
            .collect();
        Ok(Json(ObjectsList { objects }))
    }

    #[tool(
        description = "An object's properties at a step, with kinds, units, ranges and whether each is keyed."
    )]
    async fn object_get(
        &self,
        Parameters(p): Parameters<ObjectStepParams>,
    ) -> std::result::Result<Json<ObjectProperties>, ToolError> {
        let properties = self
            .run("object_get", move |app| {
                crate::document::object_properties(app.state(), p.object, p.step)
            })
            .await?;
        Ok(Json(ObjectProperties { properties }))
    }

    #[tool(
        description = "Draws an object on the VectorEffects map with a tool — a front, a jet, a wind shift, a current, a calm patch; a cyclone that moves is storm_create, and weather that really happened is import_history. `tool` is a name from tool_catalogue; `gesture` is that tool's gesture ({\"kind\":\"point\",\"at\":[lon,lat]}, {\"kind\":\"stroke\",\"points\":[[lon,lat],...]}, or the catalogue's drag form); `options` is a list of {\"property\",\"value\"} pairs (value tagged as object_set's are), omitted ones take defaults; `layer` null means the top layer. Returns the summary and the new object's id, which becomes the selection."
    )]
    async fn object_create(
        &self,
        Parameters(p): Parameters<ObjectCreateParams>,
    ) -> std::result::Result<Json<crate::create::Created>, ToolError> {
        let options = match p.options {
            Value::Null => Vec::new(),
            raw => super::typed("options", raw)?,
        };
        let object = crate::create::NewObject {
            tool: super::typed("tool", Value::String(p.tool))?,
            gesture: super::typed("gesture", p.gesture)?,
            options,
            layer: p.layer,
        };
        let created = self
            .write("object_create", false, move |app| {
                crate::create::create_object(app.state(), object)
            })
            .await?;
        let _ = tauri::Emitter::emit(&self.app, events::SELECTION, vec![created.object]);
        Ok(Json(created))
    }

    #[tool(
        description = "Sets one or more properties of an object at a step; each is applied as its own command and its own undo step. `values` maps property name (from object_get's `id`) to a tagged value: {\"kind\":\"number\",\"value\":n}, {\"kind\":\"bool\",\"value\":b}, {\"kind\":\"angle\",\"degrees\":d}, {\"kind\":\"position\",\"lon\":x,\"lat\":y} or {\"kind\":\"choice\",\"index\":i}. Every value is parsed and validated before any is applied, so a malformed one writes nothing. With auto_key true a change on an animated property adds a keyframe at that step."
    )]
    async fn object_set(
        &self,
        Parameters(p): Parameters<ObjectSetParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let ObjectSetParams {
            object,
            step,
            values,
            auto_key,
        } = p;
        // Parsed before any sub-write runs (ruling, task 3 review, fix
        // round 1, finding 2b): a malformed value used to be caught only
        // when its own turn in the loop came around, after any values ahead
        // of it in the map had already written.
        let values: serde_json::Map<String, Value> = super::typed("values", values)?;
        let parsed = values
            .into_iter()
            .map(|(property, raw)| {
                let value: crate::document::PropertyValue =
                    super::typed(&format!("values.{property}"), raw)?;
                Ok((property, value))
            })
            .collect::<std::result::Result<Vec<_>, ToolError>>()?;
        if parsed.is_empty() {
            return Err(ToolError::Refused("values is empty".to_owned()));
        }
        self.write("object_set", false, move |app| {
            let state = app.state::<AppState>();
            let mut last = None;
            for (property, value) in parsed {
                last = Some(crate::document::set_object_property(
                    state.clone(),
                    object,
                    property,
                    value,
                    step,
                    auto_key,
                    None,
                )?);
            }
            crate::document::end_gesture(state.clone())?;
            last.ok_or(AppError::BadOption {
                field: "values",
                value: "empty".to_owned(),
            })
        })
        .await
        .map(Json)
    }

    #[tool(description = "Moves an object to a layer at an index (0 = bottom of that layer).")]
    async fn object_move(
        &self,
        Parameters(p): Parameters<ObjectMoveParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("object_move", false, move |app| {
            crate::document::move_object(app.state(), p.object, p.layer, p.index)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Duplicates an object in place.")]
    async fn object_duplicate(
        &self,
        Parameters(p): Parameters<ObjectParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("object_duplicate", false, move |app| {
            crate::document::duplicate_object(app.state(), p.object)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Removes objects. One undo.")]
    async fn object_remove(
        &self,
        Parameters(p): Parameters<ObjectsParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("object_remove", false, move |app| {
            crate::document::remove_objects(app.state(), p.objects)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Ids of the objects whose footprint touches a lon/lat box at a step, optionally on one layer."
    )]
    async fn objects_in_region(
        &self,
        Parameters(p): Parameters<RegionParams>,
    ) -> std::result::Result<Json<ObjectIds>, ToolError> {
        let objects = self
            .run("objects_in_region", move |app| {
                crate::transform::objects_in_region(
                    app.state(),
                    p.west,
                    p.south,
                    p.east,
                    p.north,
                    p.step,
                    p.layer,
                )
            })
            .await?;
        Ok(Json(ObjectIds { objects }))
    }
}
