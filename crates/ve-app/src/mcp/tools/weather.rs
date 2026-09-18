//! The weather group (spec.md 8.8): drawing a whole weather system in one
//! call.
//!
//! Everything here could be done with `object_create` and `object_set`, and
//! a test agent asked for "a tropical storm from Miami to Nova Scotia" did
//! exactly that: one circle, then twenty-one position keys, and a storm that
//! crossed an ocean at the strength it was born with. Told in the server's
//! instructions that a storm develops as it travels, it keyed the position
//! again and offered intensity as an afterthought. What a storm *is* — one
//! rotating disc, turning the way its hemisphere turns, strongest at the
//! centre, moving along a track and changing as it goes — is knowledge the
//! application has, so it is a tool: the questions a caller cannot skip are
//! its required parameters.
//!
//! It composes the commands the interface calls (`create_object`,
//! `set_object_property`) and adds no way of writing the document.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::Manager;

use super::events;
use super::{ToolError, VectorEffects};
use crate::commands::AppState;
use crate::create::{Gesture, NewObject, Tool, ToolOption};
use crate::document::PropertyValue;
use crate::error::AppError;
use crate::projects::ProjectSummary;

/// Which way a storm turns, seen from above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    /// Counter-clockwise: a cyclone in the northern hemisphere.
    Ccw,
    /// Clockwise: a cyclone in the southern hemisphere.
    Cw,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StormParams {
    /// The storm centre's track as `[lon, lat]` waypoints, two or more: the
    /// first is where it is at `start_step` and the last where it is at
    /// `end_step`. Add waypoints to bend it — an Atlantic storm leaving
    /// Florida runs north along the coast, then recurves north-east.
    pub track: Vec<[f64; 2]>,
    /// Peak wind at the first waypoint, m/s (1 kt = 0.514 m/s). Tropical
    /// depression under 17, tropical storm 17 to 32, hurricane 33 and over.
    pub peak_wind_start_mps: f64,
    /// Peak wind at the last waypoint, m/s. HIGHER than the start for a
    /// storm that intensifies, which is what a storm does unless the
    /// request says it weakens or holds.
    pub peak_wind_end_mps: f64,
    /// Diameter of the wind field at the first waypoint, km. A young
    /// tropical storm is about 300.
    pub diameter_start_km: f64,
    /// Diameter at the last waypoint, km. A storm grows as it matures and
    /// moves poleward: 500 to 800.
    pub diameter_end_km: f64,
    /// The light wind at the storm's rim, m/s. Default 8.
    pub outer_wind_mps: Option<f64>,
    /// The step the storm is at the first waypoint. Default 0.
    pub start_step: Option<u32>,
    /// The step it reaches the last waypoint. Default the project's last.
    pub end_step: Option<u32>,
    /// Default: counter-clockwise for a track that starts in the northern
    /// hemisphere and clockwise in the southern, as real cyclones turn.
    pub rotation: Option<Rotation>,
    /// A layer id from `layers_list`; null or absent is the top layer.
    pub layer: Option<u64>,
}

/// Where the storm is, and how strong, at one keyed step.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct StormKey {
    pub step: u32,
    pub lon: f64,
    pub lat: f64,
    pub peak_wind_mps: f64,
    pub diameter_km: f64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StormCreated {
    pub project: ProjectSummary,
    /// The `circle` object that is the storm; `object_set`, `keyframe_set`
    /// and `object_tracks` take it from here.
    pub object: u64,
    pub rotation: Rotation,
    /// What was keyed, in step order.
    pub keys: Vec<StormKey>,
}

/// The way a cyclone turns at a latitude.
pub(crate) fn rotation_at(lat: f64) -> Rotation {
    if lat < 0.0 {
        Rotation::Cw
    } else {
        Rotation::Ccw
    }
}

/// The keys of a storm: each waypoint at the step its distance along the
/// track earns it, so the storm moves at an even pace, with wind and size
/// running linearly from the start values to the end values over those
/// steps.
///
/// Two waypoints that land on one step keep the later, except that the first
/// waypoint always keeps `start`.
pub(crate) fn storm_keys(
    track: &[[f64; 2]],
    start: u32,
    end: u32,
    wind: (f64, f64),
    diameter: (f64, f64),
) -> Vec<StormKey> {
    let points: Vec<ve_core::LonLat> = track
        .iter()
        .map(|&[lon, lat]| ve_core::LonLat { lon, lat })
        .collect();
    let mut along = vec![0.0];
    for pair in points.windows(2) {
        along.push(along[along.len() - 1] + pair[0].distance_m(pair[1]));
    }
    let total = along[along.len() - 1];
    let span = f64::from(end - start);
    let mut keys: Vec<StormKey> = Vec::new();
    for (point, travelled) in track.iter().zip(&along) {
        let fraction = if total > 0.0 { travelled / total } else { 0.0 };
        let step = start + (fraction * span).round() as u32;
        // By time, not by distance: the step was rounded, and the values
        // must be the ones a linear run from start to end has at that step.
        let t = if span > 0.0 {
            f64::from(step - start) / span
        } else {
            0.0
        };
        let key = StormKey {
            step,
            lon: point[0],
            lat: point[1],
            peak_wind_mps: wind.0 + (wind.1 - wind.0) * t,
            diameter_km: diameter.0 + (diameter.1 - diameter.0) * t,
        };
        let only_the_first = keys.len() == 1;
        match keys.last_mut() {
            // The first waypoint keeps its step; any other yields to a later one.
            Some(last) if last.step == step => {
                if !only_the_first {
                    *last = key;
                }
            }
            _ => keys.push(key),
        }
    }
    keys
}

fn refused(field: &'static str, value: String) -> AppError {
    AppError::BadOption { field, value }
}

/// The index of a choice option's variant, read from the catalogue rather
/// than written down twice.
fn variant(option: &str, name: &str) -> crate::error::Result<PropertyValue> {
    let palette = crate::palette::tool_palette()?;
    palette
        .iter()
        .filter(|tool| matches!(tool.tool, Tool::Circle))
        .flat_map(|tool| &tool.options)
        .find(|spec| spec.property == option)
        .and_then(|spec| spec.variants.iter().position(|v| v == name))
        .map(|index| PropertyValue::Choice { index: index as u8 })
        .ok_or_else(|| AppError::Internal(format!("the circle tool has no {option} {name}")))
}

#[tool_router(router = tool_router_weather, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    #[tool(
        description = "Draws an invented travelling cyclone in VectorEffects — a tropical storm, hurricane, typhoon or low; a storm that really happened is downloaded with import_history instead — with the circle tool: ONE `circle` object with a gradient fill, strongest at its centre, turning counter-clockwise in the northern hemisphere and clockwise in the southern, keyed along `track` from start_step to end_step with its peak wind and diameter running from the start values to the end values. Use this rather than object_create for any storm that moves. A storm intensifies unless the request says otherwise: give peak_wind_end_mps HIGHER than peak_wind_start_mps. Needs an open project with enough steps (a storm crossing an ocean basin takes 4 to 6 days: step_hours 6, step_count 17 to 25). Returns the object's id and what was keyed."
    )]
    async fn storm_create(
        &self,
        Parameters(p): Parameters<StormParams>,
    ) -> std::result::Result<Json<StormCreated>, ToolError> {
        let created = self
            .write("storm_create", false, move |app| {
                let state = app.state::<AppState>();
                let open = crate::projects::current_project(state.clone())?
                    .ok_or(AppError::NoProjectOpen)?;
                let last = open.step_count.saturating_sub(1);
                let start = p.start_step.unwrap_or(0);
                let end = p.end_step.unwrap_or(last);
                if p.track.len() < 2 {
                    return Err(refused(
                        "track",
                        "a storm that travels needs at least two [lon, lat] waypoints".to_owned(),
                    ));
                }
                if let Some(bad) = p
                    .track
                    .iter()
                    .find(|[lon, lat]| !(-180.0..=180.0).contains(lon) || !(-90.0..=90.0).contains(lat))
                {
                    return Err(refused(
                        "track",
                        format!("{bad:?} is not [lon, lat] in degrees, longitude -180 to 180"),
                    ));
                }
                if end > last || start >= end {
                    return Err(refused(
                        "start_step/end_step",
                        format!(
                            "{start} to {end} does not fit this project's steps 0 to {last}; a track needs at least two steps (timeline_set changes step_count)"
                        ),
                    ));
                }
                for (field, value) in [
                    ("peak_wind_start_mps", p.peak_wind_start_mps),
                    ("peak_wind_end_mps", p.peak_wind_end_mps),
                ] {
                    if !(0.0..=120.0).contains(&value) {
                        return Err(refused(field, format!("{value} m/s is outside 0 to 120")));
                    }
                }
                let outer = p.outer_wind_mps.unwrap_or(8.0);
                let rotation = p.rotation.unwrap_or_else(|| rotation_at(p.track[0][1]));
                let keys = storm_keys(
                    &p.track,
                    start,
                    end,
                    (p.peak_wind_start_mps, p.peak_wind_end_mps),
                    (p.diameter_start_km, p.diameter_end_km),
                );
                let first = keys.first().ok_or_else(|| {
                    AppError::Internal("a storm track produced no keys".to_owned())
                })?;

                let number = |value: f64| PropertyValue::Number { value };
                let option = |property: &str, value: PropertyValue| ToolOption {
                    property: property.to_owned(),
                    value,
                };
                let sense = match rotation {
                    Rotation::Ccw => "ccw",
                    Rotation::Cw => "cw",
                };
                let created = crate::create::create_object(
                    state.clone(),
                    NewObject {
                        tool: Tool::Circle,
                        gesture: Gesture::Point {
                            at: [first.lon, first.lat],
                        },
                        options: vec![
                            option("FillMode", variant("FillMode", "filled_gradient")?),
                            option("RotationSense", variant("RotationSense", sense)?),
                            option("DiameterKm", number(first.diameter_km)),
                            // On a gradient circle SpeedMin is the speed at
                            // the centre and SpeedMax the speed at the rim.
                            option("SpeedMin", number(first.peak_wind_mps)),
                            option("SpeedMax", number(outer)),
                            option("Feather", number(0.3)),
                        ],
                        layer: p.layer,
                    },
                )?;
                for key in &keys {
                    for (property, value) in [
                        (
                            "Position",
                            PropertyValue::Position {
                                lon: key.lon,
                                lat: key.lat,
                            },
                        ),
                        ("SpeedMin", number(key.peak_wind_mps)),
                        ("DiameterKm", number(key.diameter_km)),
                    ] {
                        crate::document::set_object_property(
                            state.clone(),
                            created.object,
                            property.to_owned(),
                            value,
                            key.step,
                            true,
                            None,
                        )?;
                    }
                }
                crate::document::end_gesture(state.clone())?;
                let project = crate::projects::current_project(state.clone())?
                    .ok_or(AppError::NoProjectOpen)?;
                Ok(StormCreated {
                    project,
                    object: created.object,
                    rotation,
                    keys,
                })
            })
            .await?;
        let _ = tauri::Emitter::emit(&self.app, events::SELECTION, vec![created.object]);
        Ok(Json(created))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cyclone_turns_the_way_its_hemisphere_does() {
        // Miami, and Darwin: the Coriolis force changes sign at the equator.
        assert_eq!(rotation_at(25.8), Rotation::Ccw);
        assert_eq!(rotation_at(-12.5), Rotation::Cw);
        assert_eq!(rotation_at(0.0), Rotation::Ccw);
    }

    /// A track due north along a meridian, where distance is latitude: the
    /// waypoint a quarter of the way up is a quarter of the way through the
    /// steps, and its wind a quarter of the way from the start to the end.
    #[test]
    fn waypoints_are_keyed_where_their_distance_along_the_track_puts_them() {
        let track = [[-70.0, 20.0], [-70.0, 25.0], [-70.0, 40.0]];
        let keys = storm_keys(&track, 0, 20, (18.0, 30.0), (300.0, 600.0));
        assert_eq!(
            keys.iter().map(|k| k.step).collect::<Vec<_>>(),
            [0, 5, 20],
            "5 of 20 degrees is step 5 of 20"
        );
        assert_eq!(keys[0].peak_wind_mps, 18.0);
        assert_eq!(
            keys[1].peak_wind_mps, 21.0,
            "a quarter of the way from 18 to 30"
        );
        assert_eq!(keys[2].peak_wind_mps, 30.0);
        assert_eq!(keys[1].diameter_km, 375.0);
        assert_eq!((keys[2].lon, keys[2].lat), (-70.0, 40.0));
    }

    /// More waypoints than steps: every key has its own step, the first
    /// waypoint keeps the first step and the last waypoint the last.
    #[test]
    fn crowded_waypoints_never_share_a_step() {
        let track = [[0.0, 0.0], [0.0, 0.1], [0.0, 0.2], [0.0, 10.0]];
        let keys = storm_keys(&track, 4, 6, (10.0, 20.0), (100.0, 200.0));
        let steps: Vec<u32> = keys.iter().map(|k| k.step).collect();
        assert_eq!(steps, [4, 6]);
        assert_eq!((keys[0].lat, keys[1].lat), (0.0, 10.0));
        assert_eq!(keys[1].peak_wind_mps, 20.0);
    }

    /// Across the antimeridian the short way, not three hundred and fifty
    /// degrees the long way: a typhoon track from 175E to 175W is even.
    #[test]
    fn a_track_across_the_antimeridian_is_measured_the_short_way() {
        let track = [[175.0, 15.0], [180.0, 15.0], [-175.0, 15.0]];
        let keys = storm_keys(&track, 0, 10, (20.0, 40.0), (300.0, 500.0));
        assert_eq!(keys.iter().map(|k| k.step).collect::<Vec<_>>(), [0, 5, 10]);
    }
}
