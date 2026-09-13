//! Document editing commands.
//!
//! Every mutation goes through the undo stack (`ve-core::history`), so undo is
//! available from the moment the first tool exists rather than being retrofitted
//! once several tools have their own bespoke paths.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_core::schema::PropId;

use crate::commands::AppState;
use crate::create::{Gesture, NewObject, Tool, ToolOption, create};
use crate::document::PropertyValue;
use crate::error::Result;
use crate::projects::{ProjectSummary, with_session};

/// Which stamp the brush sweeps along the stroke (spec.md 6.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "BrushShape.ts")]
pub enum BrushShape {
    /// A round stamp: the footprint is a swept disc.
    #[default]
    Circle,
    /// A square stamp, axis-aligned in the object's own frame — so rotating
    /// the object turns the stamp with it.
    Square,
}

impl BrushShape {
    /// Index into [`ve_core::schema::BRUSH_SHAPES`].
    fn variant(self) -> u8 {
        match self {
            Self::Circle => 0,
            Self::Square => 1,
        }
    }
}

/// How the brush decides each cell's direction (spec.md 6.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "BrushDirectionMode.ts")]
pub enum BrushDirectionMode {
    /// One bearing across the whole stroke.
    #[default]
    Constant,
    /// Every cell takes the initial great-circle bearing to a fixed point, so
    /// the stroke converges on it.
    TowardPoint,
    /// The reciprocal: every cell points directly away from a fixed point, so
    /// the stroke radiates out of it.
    AwayFromPoint,
}

impl BrushDirectionMode {
    /// Index into [`ve_core::schema::DIRECTION_MODES`].
    fn variant(self) -> u8 {
        match self {
            Self::Constant => 0,
            Self::TowardPoint => 1,
            Self::AwayFromPoint => 2,
        }
    }
}

/// Which space a stamp's footprint is a circle in (spec.md 3.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "StampSpace.ts")]
pub enum StampSpace {
    /// A shape on the ground: a disc of `size_km` at any latitude, which the
    /// map draws as an ellipse widening away from the equator.
    #[default]
    Geodesic,
    /// A shape on the map: a circle on screen at any latitude and any zoom,
    /// which covers `cos(lat)` as much ground east-west as it does north-south.
    /// What a size entered in pixels asks for.
    Projected,
    /// A circle in the Mercator projection used when it was drawn.
    Mercator,
    /// A circle in the Miller projection used when it was drawn.
    Miller,
}

impl StampSpace {
    /// Index into [`ve_core::schema::STAMP_SPACES`].
    pub(crate) fn variant(self) -> u8 {
        match self {
            Self::Geodesic => 0,
            Self::Projected => 1,
            Self::Mercator => 2,
            Self::Miller => 3,
        }
    }
}

/// A brush stroke, as painted on the map.
///
/// [`Default`] is derived so a caller — a test, or an example — can name only
/// the options it is about, rather than restating every one of them.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[ts(export, export_to = "BrushStroke.ts")]
pub struct BrushStroke {
    /// Path in geographic coordinates, as `[lon, lat]` pairs.
    pub points: Vec<[f64; 2]>,
    /// Brush size in kilometres: the disc's diameter, or the square's side.
    ///
    /// Kilometres, never pixels. A size entered in pixels is resolved against
    /// the map scale before it gets here, and never revisited, so zooming
    /// afterwards cannot resize an existing stroke (spec.md 3.5).
    pub size_km: f32,
    /// Speed in metres per second.
    pub speed_mps: f32,
    /// Direction the flow points toward, degrees clockwise from north.
    ///
    /// Already converted from the project's display convention by the caller;
    /// nothing below the IPC boundary sees a "from" bearing (spec.md 3.3).
    /// Ignored by the modes that aim at a target.
    pub direction_toward_deg: f64,
    /// Edge falloff, 0 to 1.
    pub feather: f32,
    /// Circular or square stamp.
    #[serde(default)]
    pub shape: BrushShape,
    /// Whether the stamp is a shape on the ground or a shape on the map.
    ///
    /// `size_km` means the same number either way: it is the footprint's
    /// north-south ground extent, which is the one axis the projection leaves
    /// alone (spec.md 3.5).
    #[serde(default)]
    pub space: StampSpace,
    /// A constant bearing, or every vector aimed at or away from `target`.
    #[serde(default)]
    pub direction_mode: BrushDirectionMode,
    /// Where the vectors aim, as `[lon, lat]`. Required by the aimed modes.
    #[serde(default)]
    #[ts(optional)]
    pub target: Option<[f64; 2]>,
    /// Which layer receives it. `None` means the top layer.
    #[ts(optional)]
    pub layer: Option<u64>,
}

/// Adds a painted stroke to the topmost layer.
#[tauri::command]
pub fn add_brush_stroke(
    state: tauri::State<'_, AppState>,
    stroke: BrushStroke,
) -> Result<ProjectSummary> {
    paint(&state, stroke)
}

/// Implementation of [`add_brush_stroke`], callable without a Tauri handle.
///
/// A thin translation onto [`crate::create::create`] rather than a write path
/// of its own. The brush was the first tool and had the only one; keeping it
/// separate is how the next tool ends up with subtly different merging, naming
/// or layer rules, which is the class of bug spec 6.1 exists to prevent.
pub fn paint(state: &AppState, stroke: BrushStroke) -> Result<ProjectSummary> {
    let mut options = vec![
        option(
            PropId::SizeKm,
            PropertyValue::Number {
                value: f64::from(stroke.size_km.max(0.001)),
            },
        ),
        option(
            PropId::Speed,
            PropertyValue::Number {
                value: f64::from(stroke.speed_mps.max(0.0)),
            },
        ),
        option(
            PropId::Direction,
            PropertyValue::Angle {
                degrees: stroke.direction_toward_deg,
            },
        ),
        option(
            PropId::Feather,
            PropertyValue::Number {
                value: f64::from(stroke.feather.clamp(0.0, 1.0)),
            },
        ),
        option(
            PropId::BrushShape,
            PropertyValue::Choice {
                index: stroke.shape.variant(),
            },
        ),
        option(
            PropId::StampSpace,
            PropertyValue::Choice {
                index: stroke.space.variant(),
            },
        ),
        option(
            PropId::DirectionMode,
            PropertyValue::Choice {
                index: stroke.direction_mode.variant(),
            },
        ),
    ];
    // Left at its default in constant mode, so two strokes painted with the
    // same options still compare equal and can merge.
    if let Some([lon, lat]) = stroke.target {
        options.push(option(PropId::Target, PropertyValue::Position { lon, lat }));
    }

    create(
        state,
        NewObject {
            tool: Tool::Brush,
            gesture: Gesture::Stroke {
                points: stroke.points,
            },
            options,
            layer: stroke.layer,
        },
    )
    .map(|created| created.project)
}

/// One option, named the way the schema names it.
fn option(id: PropId, value: PropertyValue) -> ToolOption {
    ToolOption {
        property: format!("{id:?}"),
        value,
    }
}

/// Reverses the most recent change.
#[tauri::command]
pub fn undo(state: tauri::State<'_, AppState>) -> Result<ProjectSummary> {
    step_history(&state, true)
}

/// Reapplies the most recently undone change.
#[tauri::command]
pub fn redo(state: tauri::State<'_, AppState>) -> Result<ProjectSummary> {
    step_history(&state, false)
}

/// Reverses the most recent change. Exposed for tests, which drive the same
/// path the command does.
pub fn undo_for_test(state: &AppState) -> Result<ProjectSummary> {
    step_history(state, true)
}

/// Reapplies the most recently undone change. Exposed for tests, which drive
/// the same path the command does.
pub fn redo_for_test(state: &AppState) -> Result<ProjectSummary> {
    step_history(state, false)
}

fn step_history(state: &AppState, backwards: bool) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let (project, history) = (&mut open.project, &mut open.history);
        let moved = if backwards {
            history.undo(project)?
        } else {
            history.redo(project)?
        };
        if moved.is_some() {
            open.touch();
        }
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::schema::{BRUSH_SHAPES, DIRECTION_MODES};

    /// The wire enums are indices into the schema's variant lists. Written as
    /// literals for legibility, so something has to notice when a list is
    /// reordered.
    #[test]
    fn the_wire_enums_index_the_schema_variants() {
        assert_eq!(
            BRUSH_SHAPES[BrushShape::Circle.variant() as usize],
            "circle"
        );
        assert_eq!(
            BRUSH_SHAPES[BrushShape::Square.variant() as usize],
            "square"
        );
        assert_eq!(
            DIRECTION_MODES[BrushDirectionMode::Constant.variant() as usize],
            "constant"
        );
        assert_eq!(
            DIRECTION_MODES[BrushDirectionMode::TowardPoint.variant() as usize],
            "toward_point"
        );
        assert_eq!(
            DIRECTION_MODES[BrushDirectionMode::AwayFromPoint.variant() as usize],
            "away_from_point"
        );
    }
}
