//! The time group (spec.md 8.8): keyframes, interpolation, motion, follow
//! and the timeline.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tauri::Manager;

use super::{ObjectStepParams, ToolError, VectorEffects};
use crate::error::AppError;
use crate::projects::ProjectSummary;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyframeSetParams {
    pub object: u64,
    /// A property id (`object_get`'s `id`, e.g. `"Position"`, `"Speed"`).
    pub property: String,
    pub step: u32,
    /// A tagged value (object_set's shape), or null to key the value the
    /// property has at this step right now.
    #[schemars(with = "Option<crate::document::PropertyValue>")]
    pub value: Option<Value>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyframeParams {
    pub object: u64,
    pub property: String,
    pub step: u32,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyframeMoveParams {
    pub object: u64,
    pub property: String,
    pub from: u32,
    pub to: u32,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct InterpolationParams {
    pub object: u64,
    pub property: String,
    pub step: u32,
    /// A tagged `InterpolationView`: `{"kind":"step"}`, `{"kind":"linear"}`,
    /// `{"kind":"ease_in"}`, `{"kind":"ease_out"}`, `{"kind":"ease_in_out"}`,
    /// or `{"kind":"bezier","x1":..,"y1":..,"x2":..,"y2":..}` — the names
    /// `object_tracks` returns in a track's `interpolations`.
    pub interpolation: Value,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MotionParams {
    pub object: u64,
    pub step: u32,
    /// True bearing toward, degrees clockwise from north.
    pub direction: f64,
    pub speed_mps: f64,
    #[serde(default)]
    pub overwrite: bool,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FollowParams {
    pub object: u64,
    pub property: String,
    /// The object to follow, or null to stop following.
    pub primary: Option<u64>,
    pub step: u32,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TimelineParams {
    pub step_count: Option<u32>,
    /// Unix seconds for step 0. `null` clears the project's start time;
    /// leaving the field out entirely leaves it alone.
    //
    // Plain `Option<Option<i64>>` cannot tell an explicit `null` from an
    // absent field apart: `serde_json`'s `Option` deserialisation collapses
    // both to the outer `None` (verified: `{}"` and `{"x":null}` both
    // deserialise `x: Option<Option<i64>>` to `None`), which would make
    // clearing the start time unreachable through this tool.
    // `deserialize_present` runs only when the key is in the request, so a
    // present `null` reaches it and becomes `Some(None)`.
    #[serde(default, deserialize_with = "deserialize_present")]
    pub start_unix_s: Option<Option<i64>>,
}

/// Deserialises a field only when its key is present, distinguishing an
/// explicit `null` (`Some(None)`) from an absent key (the `default` this is
/// paired with). See [`TimelineParams::start_unix_s`].
fn deserialize_present<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[tool_router(router = tool_router_time, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    #[tool(
        description = "Sets a keyframe on an animated property at a step. value null keys the value the property has there now."
    )]
    async fn keyframe_set(
        &self,
        Parameters(p): Parameters<KeyframeSetParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let value = p
            .value
            .map(|raw| super::typed::<crate::document::PropertyValue>("value", raw))
            .transpose()?;
        self.write("keyframe_set", false, move |app| {
            crate::animation::set_keyframe(app.state(), p.object, p.property, p.step, value)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Removes the keyframe at a step.")]
    async fn keyframe_remove(
        &self,
        Parameters(p): Parameters<KeyframeParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("keyframe_remove", false, move |app| {
            crate::animation::remove_keyframe(app.state(), p.object, p.property, p.step)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Moves a keyframe from one step to another.")]
    async fn keyframe_move(
        &self,
        Parameters(p): Parameters<KeyframeMoveParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("keyframe_move", false, move |app| {
            let summary = crate::animation::move_keyframe(
                app.state(),
                p.object,
                p.property,
                p.from,
                p.to,
                None,
            )?;
            crate::document::end_gesture(app.state())?;
            Ok(summary)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Sets how a property interpolates out of the keyframe at a step.")]
    async fn interpolation_set(
        &self,
        Parameters(p): Parameters<InterpolationParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let interp: crate::animation::InterpolationView =
            serde_json::from_value(p.interpolation)
                .map_err(|e| ToolError::Refused(format!("interpolation: {e}")))?;
        self.write("interpolation_set", false, move |app| {
            crate::animation::set_interpolation(app.state(), p.object, p.property, p.step, interp)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Constant motion from a step to the next position key: a rhumb line at a bearing and speed. Existing keys in between need overwrite true."
    )]
    async fn motion_add(
        &self,
        Parameters(p): Parameters<MotionParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("motion_add", false, move |app| {
            crate::animation::add_constant_motion(
                app.state(),
                p.object,
                p.step,
                p.direction,
                p.speed_mps,
                p.overwrite,
            )
        })
        .await
        .map(Json)
    }

    #[tool(description = "Links a property to another object's, or unlinks it with primary null.")]
    async fn follow_set(
        &self,
        Parameters(p): Parameters<FollowParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("follow_set", false, move |app| {
            crate::animation::set_follow(app.state(), p.object, p.property, p.primary, p.step)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Changes the step count and/or the start time (unix seconds). Shrinking drops keys beyond the end; read step_count_impact through invoke first if that matters."
    )]
    async fn timeline_set(
        &self,
        Parameters(p): Parameters<TimelineParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("timeline_set", false, move |app| {
            let mut last = None;
            if let Some(count) = p.step_count {
                last = Some(crate::animation::set_step_count(app.state(), count)?);
            }
            if let Some(start) = p.start_unix_s {
                last = Some(crate::animation::set_start_time(app.state(), start)?);
            }
            last.ok_or_else(|| AppError::BadOption {
                field: "step_count/start_unix_s",
                value: "nothing to set".to_owned(),
            })
        })
        .await
        .map(Json)
    }

    #[tool(description = "An object's animated properties with their keyframes and interpolation.")]
    async fn object_tracks(
        &self,
        Parameters(p): Parameters<ObjectStepParams>,
    ) -> std::result::Result<Json<crate::animation::ObjectTracks>, ToolError> {
        self.run("object_tracks", move |app| {
            crate::animation::object_tracks(app.state(), p.object, p.step)
        })
        .await
        .map(Json)
    }
}
