//! Copying an imported layer's frames between steps (spec.md 4.8, M20).
//!
//! A forecast file rarely lines up with a timeline. A 6-hourly file in a
//! 3-hourly project has a message on every other step and nothing between;
//! a file ends before the timeline does; a message is sometimes simply bad.
//! §4.8's rule that a step the file says nothing about shows nothing is right
//! — holding a measurement forward draws a forecast for a time it was never
//! made for — but it leaves the user no way to say otherwise.
//!
//! This is that way, and it is deliberately the shape of a keyframe rather
//! than of a message: *this* step shows *that* one, as an instruction the user
//! gave. **What is stored is a step number, never a sample** (invariants 1 and
//! 2), so the frame that reaches the scene is one the file already holds and
//! the render cache, the readiness probe and both kernels never learn that a
//! choice was made.

use serde::Serialize;
use ts_rs::TS;

use ve_core::Command;
use ve_core::document::FrameOverride;

use crate::commands::AppState;
use crate::document::object_id;
use crate::error::{AppError, Result};
use crate::projects::{ProjectSummary, with_session};

/// Frames held for pasting.
///
/// **The layer travels with them.** A GRIB frame pasted into another layer
/// would be a copy of samples by another route — the target layer's file
/// holds different messages, or none — so the paste goes back to the layer
/// the copy came from, whatever layer happens to be active (D59).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameClipboard {
    /// The layer the steps were copied from.
    pub layer: Option<ve_core::Id>,
    /// The steps copied, ascending.
    pub steps: Vec<u32>,
}

impl FrameClipboard {
    /// The steps, re-based so the first lands on `at` and the rest keep their
    /// spacing: 0, 1, 3 pasted at 6 lands on 6, 7, 9.
    fn placed(&self, at: u32) -> Vec<(u32, u32)> {
        let Some(&first) = self.steps.first() else {
            return Vec::new();
        };
        self.steps
            .iter()
            .map(|&source| (at + (source - first), source))
            .collect()
    }
}

/// What the timeline shows about the frame clipboard.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "FrameClipboardState.ts")]
pub struct FrameClipboardState {
    /// Frames available to paste.
    pub count: usize,
    /// The layer they will paste into, whatever layer is active.
    pub layer: Option<u64>,
}

/// Copies a run of an imported layer's steps.
///
/// Only steps that show something are worth copying — a copied gap would
/// paste a gap, which `Delete` already does — so empty ones are dropped
/// rather than refused: shift-selecting across a sparse file should not have
/// to avoid the holes.
#[tauri::command]
pub fn copy_grib_frames(
    state: tauri::State<'_, AppState>,
    layer: u64,
    steps: Vec<u32>,
) -> Result<FrameClipboardState> {
    frames_copy(&state, layer, steps)
}

/// Implementation of [`copy_grib_frames`].
pub fn frames_copy(state: &AppState, layer: u64, steps: Vec<u32>) -> Result<FrameClipboardState> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let id = object_id(layer);
        let found = open
            .project
            .layer(id)
            .ok_or(AppError::Core(ve_core::CoreError::MissingLayer(layer)))?;
        let settings = open.project.settings;
        let mut steps: Vec<u32> = steps
            .into_iter()
            .filter(|&s| s < settings.step_count && found.imported_frame(&settings, s).is_some())
            .collect();
        steps.sort_unstable();
        steps.dedup();
        session.frames = FrameClipboard {
            layer: (!steps.is_empty()).then_some(id),
            steps,
        };
        Ok(clipboard_state_of(&session.frames))
    })
}

/// Pastes the copied frames, the first landing on `at`.
///
/// A pasted step past the end of the timeline is **dropped, not clamped**:
/// clamping would pile the tail of a run onto the last step, each overwriting
/// the one before, and leave the user with one frame where they asked for
/// four.
#[tauri::command]
pub fn paste_grib_frames(state: tauri::State<'_, AppState>, at: u32) -> Result<ProjectSummary> {
    frames_paste(&state, at)
}

/// Implementation of [`paste_grib_frames`].
pub fn frames_paste(state: &AppState, at: u32) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let clipboard = session.frames.clone();
        let open = session.require_open()?;
        let Some(layer_id) = clipboard.layer else {
            return Ok(ProjectSummary::of(open));
        };
        let last = open.project.settings.last_step();
        let found = open.project.layer(layer_id).ok_or(AppError::Core(
            ve_core::CoreError::MissingLayer(layer_id.raw()),
        ))?;
        let mut after = found.frame_overrides.clone();
        let mut pasted = 0;
        for (step, source) in clipboard.placed(at) {
            if step > last {
                continue;
            }
            after.retain(|o| o.step != step);
            after.push(FrameOverride {
                step,
                source: Some(source),
            });
            pasted += 1;
        }
        if pasted == 0 {
            return Ok(ProjectSummary::of(open));
        }
        let command = Command::SetFrameOverrides {
            layer: layer_id,
            before: found.frame_overrides.clone(),
            after,
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// `Delete` on a run of an imported layer's steps.
///
/// Two meanings, and which one applies is a property of the step rather than
/// of the key: on a pasted frame it removes the override and the file's own
/// message comes back, and on the file's own message it hides that message —
/// the same override with no source, which is what a bad message needs and
/// costs nothing extra.
#[tauri::command]
pub fn delete_grib_frames(
    state: tauri::State<'_, AppState>,
    layer: u64,
    steps: Vec<u32>,
) -> Result<ProjectSummary> {
    frames_delete(&state, layer, steps)
}

/// Implementation of [`delete_grib_frames`].
pub fn frames_delete(state: &AppState, layer: u64, steps: Vec<u32>) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let id = object_id(layer);
        let last = open.project.settings.last_step();
        let found = open
            .project
            .layer(id)
            .ok_or(AppError::Core(ve_core::CoreError::MissingLayer(layer)))?;
        let mut after = found.frame_overrides.clone();
        let mut changed = false;
        for step in steps.into_iter().filter(|&s| s <= last) {
            match found.frame_override(step) {
                // A pasted frame, or one already hidden: the file's own
                // message comes back.
                Some(_) => {
                    after.retain(|o| o.step != step);
                    changed = true;
                }
                // The file's own message: hide it here.
                None => {
                    after.push(FrameOverride { step, source: None });
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(ProjectSummary::of(open));
        }
        let command = Command::SetFrameOverrides {
            layer: id,
            before: found.frame_overrides.clone(),
            after,
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// What the frame clipboard holds.
#[tauri::command]
pub fn frame_clipboard_state(state: tauri::State<'_, AppState>) -> Result<FrameClipboardState> {
    frames_clipboard(&state)
}

/// Implementation of [`frame_clipboard_state`].
pub fn frames_clipboard(state: &AppState) -> Result<FrameClipboardState> {
    with_session(state, |session| Ok(clipboard_state_of(&session.frames)))
}

fn clipboard_state_of(clipboard: &FrameClipboard) -> FrameClipboardState {
    FrameClipboardState {
        count: clipboard.steps.len(),
        layer: clipboard.layer.map(|id| id.raw()),
    }
}
