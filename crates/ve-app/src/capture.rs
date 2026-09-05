//! Capturing a region of the field, and pasting it back as a patch
//! (spec.md 8.5, M14).
//!
//! `Cmd`-`C` over a region takes the **visible composite** inside it — what
//! the map is showing, evaluated the way an export is — onto the project's own
//! lattice. `Cmd`-`V` puts it down as a patch: an object like any other, whose
//! field is those samples instead of a formula, and which moves, turns,
//! scales, keys and feathers accordingly.
//!
//! Two things make this more than a screenshot.
//!
//! **Zero and undefined are different.** A cell no object and no raster wrote,
//! or one a mask removed, is undefined; it writes nothing when the patch is
//! composited, so what is beneath shows through exactly where the source was
//! transparent (D58). A real zero is calm and overwrites.
//!
//! **The evaluation is the CPU's.** `CpuEvaluator` is the authority for what a
//! field *is* (invariant 3), and a capture is a value the user keeps rather
//! than a frame they look at — so it is taken the way an export is taken, not
//! the way a tile is.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use ve_core::capture::{Capture, CaptureFrame, CaptureLattice, UNDEFINED};
use ve_core::document::{Geometry, LocalPoint, Object, StepRange};
use ve_core::schema::{PropId, ToolKind};
use ve_core::{Command, LonLat, PropValue};
use ve_render::aeqd::M_PER_DEGREE;
use ve_render::cache::scene_hash;
use ve_render::cpu::sample_scene_covered;
use ve_render::scene::flatten;

use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::{ProjectSummary, with_session};

/// The region to capture, as the map describes it.
///
/// Degrees throughout, because a region is map space: it is drawn on the map,
/// and what is made from one is a projected stamp (D28, D55).
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "RegionShape.ts")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegionShape {
    /// A rectangle about its centre.
    Rect {
        /// Centre, `[lon, lat]`.
        centre: [f64; 2],
        /// Half-extent east-west, in degrees.
        half_width_deg: f64,
        /// Half-extent north-south, in degrees.
        half_height_deg: f64,
    },
    /// A circle about its centre, round **on the map**.
    Disc {
        /// Centre, `[lon, lat]`.
        centre: [f64; 2],
        /// Radius in degrees.
        radius_deg: f64,
    },
    /// A freehand ring.
    Polygon {
        /// Vertices in order, `[lon, lat]`. Closed implicitly.
        points: Vec<[f64; 2]>,
    },
}

impl RegionShape {
    /// The anchor a patch made from this region sits on.
    pub(crate) fn anchor(&self) -> Option<LonLat> {
        let (lon, lat) = match self {
            Self::Rect { centre, .. } | Self::Disc { centre, .. } => (centre[0], centre[1]),
            Self::Polygon { points } => {
                if points.is_empty() {
                    return None;
                }
                // The centre of the bounding box, with longitudes carried the
                // short way from the first point so a ring across the seam
                // keeps its shape.
                let first = points[0];
                let mut running = first[0];
                let (mut west, mut east) = (first[0], first[0]);
                let (mut south, mut north) = (first[1], first[1]);
                for point in points {
                    running += wrap180(point[0] - running);
                    west = west.min(running);
                    east = east.max(running);
                    south = south.min(point[1]);
                    north = north.max(point[1]);
                }
                ((west + east) / 2.0, (south + north) / 2.0)
            }
        };
        LonLat::new(wrap180(lon), lat.clamp(-90.0, 90.0)).ok()
    }

    /// Half-extents of the region's bounding box, in degrees.
    pub(crate) fn half_extents(&self) -> (f64, f64) {
        match self {
            Self::Rect {
                half_width_deg,
                half_height_deg,
                ..
            } => (half_width_deg.abs(), half_height_deg.abs()),
            Self::Disc { radius_deg, .. } => (radius_deg.abs(), radius_deg.abs()),
            Self::Polygon { points } => {
                let Some(first) = points.first() else {
                    return (0.0, 0.0);
                };
                let mut running = first[0];
                let (mut west, mut east) = (first[0], first[0]);
                let (mut south, mut north) = (first[1], first[1]);
                for point in points {
                    running += wrap180(point[0] - running);
                    west = west.min(running);
                    east = east.max(running);
                    south = south.min(point[1]);
                    north = north.max(point[1]);
                }
                ((east - west) / 2.0, (north - south) / 2.0)
            }
        }
    }

    /// The object geometry a patch of this region carries, in the local metres
    /// a projected frame measures in.
    fn geometry(&self, anchor: LonLat) -> Geometry {
        match self {
            Self::Rect {
                half_width_deg,
                half_height_deg,
                ..
            } => Geometry::Rect {
                half_width_m: half_width_deg.abs() * M_PER_DEGREE,
                half_height_m: half_height_deg.abs() * M_PER_DEGREE,
            },
            Self::Disc { radius_deg, .. } => Geometry::Disc {
                radius_m: Some(radius_deg.abs() * M_PER_DEGREE),
            },
            Self::Polygon { points } => Geometry::Polygon {
                points: points
                    .iter()
                    .map(|p| {
                        LocalPoint::new(
                            wrap180(p[0] - anchor.lon) * M_PER_DEGREE,
                            (p[1] - anchor.lat) * M_PER_DEGREE,
                        )
                    })
                    .collect(),
            },
        }
    }
}

fn wrap180(degrees: f64) -> f64 {
    let wrapped = (degrees + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped <= -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

/// What the clipboard holds after a capture.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "CaptureState.ts")]
pub struct CaptureState {
    /// Whether a captured field is available to paste.
    pub has_capture: bool,
    /// Its lattice size, for the status line.
    pub ni: u32,
    /// And its height.
    pub nj: u32,
    /// How many time slices it holds.
    pub frames: u32,
}

/// A captured field held for pasting, with the shape it was taken over.
#[derive(Debug, Clone, Default)]
pub struct CaptureClipboard {
    /// The capture itself, and the region it came from.
    pub held: Option<(Arc<Capture>, RegionShape)>,
}

/// Captures the visible composite inside a region (spec.md 8.5).
#[tauri::command(async)]
pub fn capture_region(
    state: tauri::State<'_, AppState>,
    region: RegionShape,
    step: u32,
) -> Result<CaptureState> {
    region_capture(&state, region, step)
}

/// Implementation of [`capture_region`].
pub fn region_capture(state: &AppState, region: RegionShape, step: u32) -> Result<CaptureState> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let project = &open.project;
        let bad = |why: &str| AppError::BadOption {
            field: "region",
            value: why.to_owned(),
        };
        let anchor = region.anchor().ok_or_else(|| bad("is nowhere"))?;
        let (half_w, half_h) = region.half_extents();
        // The project's own lattice, so a captured field lines up with what
        // the project exports and a patch pasted back where it came from
        // reads its own samples node for node.
        let spacing = project.settings.resolution.degrees();
        if !(half_w > 0.0 && half_h > 0.0) {
            return Err(bad("has no extent"));
        }
        let ni = ((half_w * 2.0 / spacing).ceil() as u32 + 1).min(MAX_SIDE);
        let nj = ((half_h * 2.0 / spacing).ceil() as u32 + 1).min(MAX_SIDE);
        let x0 = -f64::from(ni - 1) * spacing / 2.0;
        let y0 = f64::from(nj - 1) * spacing / 2.0;

        // Evaluated through the CPU, like an export: this is a value the user
        // keeps, not a frame they are looking at (invariant 3).
        //
        // **A run of frames, from the copy step to the end** (D65), so a
        // copied animation is an animation when pasted rather than a still
        // of the step it was copied at. Frames are kept only where the scene
        // *changed*: two steps that flatten to the same hash draw the same
        // field, so a still scene bakes one frame and holds it, and a scene
        // that stops moving at step 8 stops baking there. The frame offsets
        // carry the gaps, and the patch holds across them.
        let hours_per_step = f64::from(project.settings.step_hours.hours());
        let last_step = project.last_step();
        let mut frames = Vec::new();
        let mut previous_hash = None;
        for at_step in step..=last_step {
            let scene = flatten(project, at_step);
            let hash = scene_hash(&scene);
            if previous_hash == Some(hash) {
                continue;
            }
            previous_hash = Some(hash);
            let mut uv = Vec::with_capacity(ni as usize * nj as usize);
            for j in 0..nj {
                let lat = anchor.lat + y0 - f64::from(j) * spacing;
                for i in 0..ni {
                    let lon = anchor.lon + x0 + f64::from(i) * spacing;
                    let Ok(at) = LonLat::new(wrap180(lon), lat.clamp(-90.0, 90.0)) else {
                        uv.push(UNDEFINED);
                        continue;
                    };
                    // Undefined where nothing wrote, which is what makes a
                    // paste transparent exactly where its source was (D58).
                    uv.push(match sample_scene_covered(&scene, at) {
                        Some(sample) => [sample.u, sample.v],
                        None => UNDEFINED,
                    });
                }
            }
            frames.push(CaptureFrame::still(
                f64::from(at_step - step) * hours_per_step,
                uv,
            ));
        }
        let seconds_per_frame = if frames.len() > 1 {
            hours_per_step * 3600.0
        } else {
            0.0
        };

        let capture = Capture::new(
            project.settings.field_kind,
            CaptureLattice {
                ni,
                nj,
                spacing_deg: spacing,
                x0_deg: x0,
                y0_deg: y0,
            },
            seconds_per_frame,
            region.geometry(anchor),
            frames,
        )
        .map_err(AppError::Core)?;

        let state = CaptureState {
            has_capture: true,
            ni: capture.ni,
            nj: capture.nj,
            frames: capture.frames.len() as u32,
        };
        session.capture = CaptureClipboard {
            held: Some((Arc::new(capture), region)),
        };
        // One clipboard (spec.md 8.5): a capture supersedes copied objects
        // exactly as copying objects supersedes a capture.
        session.clipboard = ve_core::clipboard::Clipboard::default();
        Ok(state)
    })
}

/// The largest lattice a capture may have on a side.
///
/// A whole-map region at 0.1° would be 3,601 by 1,801, which is 52 MB of
/// samples for one patch. The cap is generous enough for any region anyone
/// draws by hand and small enough that a stray `Cmd`-`Shift`-`A` cannot make
/// the project unopenable.
const MAX_SIDE: u32 = 2_048;

/// Pastes the captured field as a patch, centred at `lon`/`lat`.
#[tauri::command(async)]
pub fn paste_capture(
    state: tauri::State<'_, AppState>,
    lon: Option<f64>,
    lat: Option<f64>,
    step: u32,
    layer: Option<u64>,
) -> Result<ProjectSummary> {
    capture_paste(&state, lon, lat, step, layer)
}

/// Implementation of [`paste_capture`].
///
/// With a position it lands there — the pointer is over the map, and that is
/// where the user is pointing. Without one it lands where it was taken, nudged
/// like any pasted object so the copy is not hidden under its original
/// (spec.md 8.5). It joins `layer` under the one creation rule (D66), and
/// **its run of frames begins at `step`**: the patch's first active step is
/// where `capture_of` measures the frames from, so what was copied at step 5
/// and pasted at 12 shows at 12 what the source showed at 5 (D65).
pub fn capture_paste(
    state: &AppState,
    lon: Option<f64>,
    lat: Option<f64>,
    step: u32,
    layer: Option<u64>,
) -> Result<ProjectSummary> {
    with_session(state, |state_session| {
        let held = state_session.capture.held.clone();
        let open = state_session.require_open()?;
        let Some((capture, region)) = held else {
            return Ok(ProjectSummary::of(open));
        };
        let taken_at = region.anchor().ok_or(AppError::BadOption {
            field: "region",
            value: "is nowhere".to_owned(),
        })?;
        let anchor = match (lon, lat) {
            (Some(lon), Some(lat)) => LonLat::new(wrap180(lon), lat.clamp(-90.0, 90.0))?,
            // Nudged east, the same distance a pasted object is, so the copy
            // is visible rather than exactly under what it came from.
            _ => LonLat::new(wrap180(taken_at.lon + PASTE_NUDGE_DEG), taken_at.lat)?,
        };

        let step_count = open.project.settings.step_count;
        let mut object = Object::new(ToolKind::Patch, "Patch", step_count);
        object.geometry = capture.shape.clone();
        object.capture = Some(capture.hash.clone());
        if let Some(anim) = object.props.get_mut(PropId::Position) {
            anim.set_base(PropValue::LonLat(anchor));
        }
        // A region is map space, so the patch is a projected stamp: variant 1
        // of `STAMP_SPACES` (D28, D55).
        if let Some(anim) = object.props.get_mut(PropId::StampSpace) {
            anim.set_base(PropValue::Enum(1));
        }
        // The frames run from here (D65). A still capture is one frame at
        // every step and its range is left whole, so a still patch pasted at
        // step 12 is not silently absent from the eleven steps before it.
        if capture.frames.len() > 1 {
            object.active_range = StepRange::new(
                step.min(step_count.saturating_sub(1)),
                step_count.saturating_sub(1),
            );
        }

        let target = crate::document::creation_layer(&open.project, layer)?;
        let (layer, index) = (target.id, target.objects.len());

        // The samples go into the project *before* the command, so the object
        // the command adds already has a field to draw. They are keyed by
        // content hash, so pasting the same capture twice is one entry.
        open.project
            .captures
            .entry(capture.hash.clone())
            .or_insert_with(|| Arc::clone(&capture));

        let command = Command::AddObject {
            layer,
            index,
            object: Box::new(object),
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(state_session.require_open()?))
    })
}

/// Degrees a patch pasted in place is nudged, so it is not hidden under its
/// original. The same distance `ve_core::clipboard` nudges an object.
const PASTE_NUDGE_DEG: f64 = 2.0;

/// What the capture clipboard holds.
#[tauri::command]
pub fn capture_state(state: tauri::State<'_, AppState>) -> Result<CaptureState> {
    capture_held(&state)
}

/// Implementation of [`capture_state`].
pub fn capture_held(state: &AppState) -> Result<CaptureState> {
    with_session(state, |session| {
        Ok(match &session.capture.held {
            Some((capture, _)) => CaptureState {
                has_capture: true,
                ni: capture.ni,
                nj: capture.nj,
                frames: capture.frames.len() as u32,
            },
            None => CaptureState {
                has_capture: false,
                ni: 0,
                nj: 0,
                frames: 0,
            },
        })
    })
}

/// The anchor a region's object sits on, for the macro module.
pub(crate) fn region_anchor(region: &RegionShape) -> Option<LonLat> {
    region.anchor()
}

/// Its bounding half-extents, in degrees.
pub(crate) fn region_half_extents(region: &RegionShape) -> (f64, f64) {
    region.half_extents()
}

/// Its object geometry, in the local metres a projected frame measures in.
pub(crate) fn region_geometry(region: &RegionShape, anchor: LonLat) -> Geometry {
    region.geometry(anchor)
}
