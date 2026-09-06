//! The macro library, and the two tools that fill and use it
//! (spec.md 8.7, M16).
//!
//! A macro is M14's capture over a **run of frames**, kept under a name in a
//! directory of `.vemacro` files rather than in any project. It can be dropped
//! into a project that knows nothing about where it came from.
//!
//! **Nothing in the library is project data.** A project that uses a macro
//! carries its own copy of the frames, as its own `captures/` entry (D52), so
//! clearing the library breaks nothing already inserted — which is what makes
//! *Delete all macros* a safe button rather than a destructive one.
//!
//! # Capture mode is a backend lockout
//!
//! While a capture is running, the document must not change: the frames being
//! baked are of a field that has to still be there at the end. That is **one
//! flag in the session that every write path checks**, not a set of disabled
//! controls in the UI — a control that was missed would be a write during a
//! capture, and nothing would say so until the macro came out wrong.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use ve_core::capture::{Capture, CaptureFrame, CaptureLattice, UNDEFINED};
use ve_core::document::Object;
use ve_core::project::FieldKind;
use ve_core::schema::{PropId, ToolKind};
use ve_core::{Command, LonLat, PropValue};
use ve_render::cpu::sample_scene_covered;
use ve_render::scene::flatten_kind;

use crate::capture::RegionShape;
use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::{ProjectSummary, with_session};

/// File extension for a library entry.
const EXTENSION: &str = "vemacro";

/// What a `.vemacro` carries besides its samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MacroHeader {
    name: String,
    created_unix_s: i64,
    /// The field kind of the project it was taken in.
    field_kind: String,
    /// The step size it was taken at, in hours — what makes its frame times
    /// mean something in a project with a different one.
    step_hours: u32,
}

/// One entry of the library, as the panels list it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "MacroEntry.ts")]
pub struct MacroEntry {
    /// The file's stem, which is also how it is addressed.
    pub id: String,
    /// The name the user gave it.
    pub name: String,
    /// `"wind"` or `"current"`, from the project it was taken in.
    pub field_kind: String,
    /// How many time slices it holds.
    pub frames: u32,
    /// Hours from its first frame to its last.
    pub span_hours: f64,
    /// The step size it was captured at, in hours.
    pub step_hours: u32,
    /// Bytes on disk.
    pub size_bytes: u64,
    /// Whether any frame's region moved: a macro that records movement.
    pub moves: bool,
    /// The region's shape about its centre, in degrees of the map, so the
    /// insert tool can show where the macro will land before the click
    /// (M24).
    pub outline: MacroOutline,
    /// Where each frame's region sat relative to the first frame's, as
    /// `[dx, dy]` in degrees — the recorded movement, for the hover to draw
    /// as a track. Every entry is `[0, 0]` for a static macro.
    pub track: Vec<[f64; 2]>,
}

/// A macro's region, about its centre, in degrees of the map.
///
/// The capture stores its shape in the local metres a projected frame
/// measures in (D28, D55); this is that shape read back into the degrees the
/// map draws in, which is what `Region` on the frontend is made of.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "MacroOutline.ts")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MacroOutline {
    /// A rectangle about the centre.
    Rect {
        /// Half-extent east-west.
        half_width_deg: f64,
        /// Half-extent north-south.
        half_height_deg: f64,
    },
    /// A circle, round on the map.
    Disc {
        /// Radius.
        radius_deg: f64,
    },
    /// A freehand ring, as offsets from the centre.
    Polygon {
        /// Vertices in order, `[dx, dy]`.
        points: Vec<[f64; 2]>,
    },
}

impl MacroOutline {
    /// Reads a capture's shape back into map degrees.
    fn of(shape: &ve_core::document::Geometry) -> Self {
        use ve_core::document::Geometry;
        use ve_render::aeqd::M_PER_DEGREE;
        match shape {
            Geometry::Rect {
                half_width_m,
                half_height_m,
            } => Self::Rect {
                half_width_deg: half_width_m / M_PER_DEGREE,
                half_height_deg: half_height_m / M_PER_DEGREE,
            },
            Geometry::Disc { radius_m } => Self::Disc {
                radius_deg: radius_m.unwrap_or(0.0) / M_PER_DEGREE,
            },
            Geometry::Polygon { points } => Self::Polygon {
                points: points
                    .iter()
                    .map(|p| [p.x / M_PER_DEGREE, p.y / M_PER_DEGREE])
                    .collect(),
            },
            // A capture's shape is one of the three above (`region_geometry`);
            // anything else is a file from a build that does not exist, and a
            // point outline is the honest answer.
            Geometry::Stroke { .. } | Geometry::Path { .. } | Geometry::Smear { .. } => {
                Self::Disc { radius_deg: 0.0 }
            }
        }
    }
}

/// The library's contents, for the insert bar and the settings dialog.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "MacroLibrary.ts")]
pub struct MacroLibrary {
    /// Where the files live.
    pub directory: String,
    /// Entries, newest first.
    pub entries: Vec<MacroEntry>,
    /// Total bytes on disk, for the settings dialog.
    pub total_bytes: u64,
}

/// Where the library lives: the setting, or the default beside the app data.
fn directory(state: &AppState, configured: &str) -> PathBuf {
    if configured.trim().is_empty() {
        state.paths.config_dir.join("macros")
    } else {
        PathBuf::from(configured)
    }
}

/// Reads one file's header and lattice without inflating its samples.
fn entry_of(path: &Path) -> Option<MacroEntry> {
    let bytes = std::fs::read(path).ok()?;
    let (header, capture) = decode(&bytes).ok()?;
    Some(MacroEntry {
        id: path.file_stem()?.to_string_lossy().into_owned(),
        name: header.name,
        field_kind: header.field_kind,
        frames: capture.frames.len() as u32,
        span_hours: capture.frames.last().map_or(0.0, |f| f.offset_hours),
        step_hours: header.step_hours,
        size_bytes: bytes.len() as u64,
        moves: capture
            .frames
            .iter()
            .any(|f| f.dx_deg != 0.0 || f.dy_deg != 0.0),
        outline: MacroOutline::of(&capture.shape),
        track: capture
            .frames
            .iter()
            .map(|f| [f.dx_deg, f.dy_deg])
            .collect(),
    })
}

/// A `.vemacro` is its header as JSON, length-prefixed, then a `.vecap`.
fn encode(header: &MacroHeader, capture: &Capture) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(header).map_err(ve_core::CoreError::Json)?;
    let mut out = Vec::with_capacity(json.len() + 8);
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&capture.encode().map_err(AppError::Core)?);
    Ok(out)
}

fn decode(bytes: &[u8]) -> Result<(MacroHeader, Capture)> {
    let bad = || AppError::Core(ve_core::CoreError::Capture("a macro file".to_owned()));
    let len = u32::from_le_bytes(
        bytes
            .get(0..4)
            .ok_or_else(bad)?
            .try_into()
            .map_err(|_| bad())?,
    ) as usize;
    let json = bytes.get(4..4 + len).ok_or_else(bad)?;
    let header: MacroHeader = serde_json::from_slice(json).map_err(ve_core::CoreError::Json)?;
    let capture = Capture::decode(bytes.get(4 + len..).ok_or_else(bad)?).map_err(AppError::Core)?;
    Ok((header, capture))
}

/// Lists the library.
#[tauri::command]
pub fn macro_library(state: tauri::State<'_, AppState>) -> Result<MacroLibrary> {
    library(&state)
}

/// Implementation of [`macro_library`].
///
/// A file that will not decode is **skipped**, not fatal: a directory the user
/// chose may hold anything at all, and one bad file must not empty the list.
pub fn library(state: &AppState) -> Result<MacroLibrary> {
    let configured = with_session(state, |session| {
        Ok(session.settings.macro_directory.clone())
    })?;
    let dir = directory(state, &configured);
    let mut entries: Vec<MacroEntry> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|found| found.path())
        .filter(|path| path.extension().is_some_and(|e| e == EXTENSION))
        .filter_map(|path| entry_of(&path))
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let total_bytes = entries.iter().map(|e| e.size_bytes).sum();
    Ok(MacroLibrary {
        directory: dir.to_string_lossy().into_owned(),
        entries,
        total_bytes,
    })
}

/// Deletes one macro, or every one.
#[tauri::command]
pub fn delete_macros(
    state: tauri::State<'_, AppState>,
    id: Option<String>,
) -> Result<MacroLibrary> {
    macros_delete(&state, id)
}

/// Implementation of [`delete_macros`].
///
/// Safe to do at any time: a project that used one of these carries its own
/// copy of the frames, so nothing already inserted goes blank (D52).
pub fn macros_delete(state: &AppState, id: Option<String>) -> Result<MacroLibrary> {
    let configured = with_session(state, |session| {
        Ok(session.settings.macro_directory.clone())
    })?;
    let dir = directory(state, &configured);
    match id {
        Some(one) => {
            let _ = std::fs::remove_file(dir.join(format!("{one}.{EXTENSION}")));
        }
        None => {
            for found in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = found.path();
                if path.extension().is_some_and(|e| e == EXTENSION) {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
    library(state)
}

/// A capture in progress: the region at each step, and how it was started.
#[derive(Debug, Clone, Default)]
pub struct CaptureSession {
    /// `Some` while capture mode is on.
    pub active: Option<ActiveCapture>,
}

/// Which half of a capture the session is in (spec.md 8.7, M26).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "CapturePhase.ts")]
#[serde(rename_all = "snake_case")]
pub enum CapturePhase {
    /// The region is being placed at steps; the document is locked.
    Recording,
    /// The capture is baked and shown on an empty map, to be kept, edited
    /// or dropped. The document is still locked.
    Previewing,
}

/// The state of a running capture.
#[derive(Debug, Clone)]
pub struct ActiveCapture {
    /// The region's shape, fixed when the capture began: it cannot be
    /// redrawn or reshaped while recording (spec.md 8.7).
    pub shape: RegionShape,
    /// The region's position **keys**, by step (D72). Every step the
    /// playhead visits while recording gets one at the region's position
    /// then; dragging the region at a step sets it; a key can be removed,
    /// and the position between keys interpolates by great circle. The
    /// first step always has one.
    pub positions: std::collections::BTreeMap<u32, [f64; 2]>,
    /// Where it was drawn: the position every step starts from.
    pub origin: [f64; 2],
    /// Whether the recorded movement is kept.
    pub record_movement: bool,
    /// The first step of the run.
    pub first_step: u32,
    /// Recording, or previewing what was recorded.
    pub phase: CapturePhase,
    /// The bake, once the preview has made it. Kept through *edit* → back to
    /// recording? No: editing drops it, since the keys may change.
    pub baked: Option<Arc<Capture>>,
    /// The last step of the run, fixed when the preview was made.
    pub last_step: u32,
    /// Where the preview's stamp sits, `[lon, lat]`; the origin until the
    /// user clicks somewhere.
    pub stamp: [f64; 2],
    /// The kind of field being recorded: the one the map showed when the
    /// capture began (M29).
    pub kind: ve_core::project::FieldKind,
    /// Where the preview has been clicked (M29): a copy of the macro is shown
    /// looping at each, in the preview's own scene and nowhere else — they
    /// are for looking at, and go with the preview. Edit clears them.
    pub stamps: Vec<[f64; 2]>,
}

impl ActiveCapture {
    /// Where the region sits at `step` (D72): its key there, or the great
    /// circle between the keys either side, or the last key held past the
    /// end. Before the first key — which cannot happen, since the first
    /// step always has one — the first key.
    pub fn position_at(&self, step: u32) -> [f64; 2] {
        if let Some(at) = self.positions.get(&step) {
            return *at;
        }
        let before = self.positions.range(..step).next_back();
        let after = self.positions.range(step..).next();
        match (before, after) {
            (Some((s0, p0)), Some((s1, p1))) => {
                let t = f64::from(step - s0) / f64::from(s1 - s0);
                slerp(*p0, *p1, t)
            }
            (Some((_, p0)), None) => *p0,
            (None, Some((_, p1))) => *p1,
            (None, None) => self.origin,
        }
    }
}

/// A point `t` of the way along the great circle from `a` to `b`, in degrees.
///
/// Unit vectors rather than degree arithmetic, so a track across the seam or
/// near a pole is the short way round, the same rule a position keyframe's
/// segment follows (spec.md 4.5).
fn slerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    let to_vec = |p: [f64; 2]| {
        let (lon, lat) = (p[0].to_radians(), p[1].to_radians());
        [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
    };
    let (u, v) = (to_vec(a), to_vec(b));
    let dot = (u[0] * v[0] + u[1] * v[1] + u[2] * v[2]).clamp(-1.0, 1.0);
    let omega = dot.acos();
    if omega < 1e-9 {
        return a;
    }
    let (wa, wb) = (
        ((1.0 - t) * omega).sin() / omega.sin(),
        (t * omega).sin() / omega.sin(),
    );
    let x = wa * u[0] + wb * v[0];
    let y = wa * u[1] + wb * v[1];
    let z = wa * u[2] + wb * v[2];
    [
        y.atan2(x).to_degrees(),
        z.atan2((x * x + y * y).sqrt()).to_degrees(),
    ]
}

/// The macro preview the tile protocol serves (D71): a project holding the
/// baked capture as one object at the stamp, under a revision of its own.
#[derive(Debug, Clone)]
pub struct PreviewScene {
    /// The one-object project.
    pub project: ve_core::project::Project,
    /// The revision its tiles are addressed by. Never a document's.
    pub revision: u64,
}

/// What the transport shows while a capture runs.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "CaptureMode.ts")]
pub struct CaptureMode {
    /// Whether a capture is running. While it is, every document write is
    /// refused.
    pub active: bool,
    /// The step it began at.
    pub first_step: u32,
    /// How many steps have been visited and therefore have a position.
    pub placed_steps: u32,
    /// Whether movement is being recorded.
    pub record_movement: bool,
    /// The steps the region has been placed at, in order.
    ///
    /// The timeline marks them, so the user can see which frames the capture
    /// has visited and which still hold the position the region was drawn at.
    pub visited: Vec<u32>,
    /// Where the region sits at the step that was asked about, as `[lon, lat]`.
    ///
    /// The map draws the region while a capture runs, and each frame holds its
    /// own position — so scrubbing to a step has to show *that step's* place,
    /// not the last one clicked. Null when no capture is running or no step was
    /// named.
    pub position: Option<[f64; 2]>,
    /// Recording, or previewing (M26).
    pub phase: CapturePhase,
    /// The steps that hold a position key (D72). What the timeline's
    /// *selection position* row draws its diamonds at.
    pub keys: Vec<u32>,
    /// The last step of the run, once the preview has fixed it; the first
    /// step until then.
    pub last_step: u32,
    /// The revision the preview's tiles are addressed by, while previewing.
    pub preview_revision: Option<u64>,
    /// Where the preview is stamped, `[lon, lat]`, while previewing.
    pub stamp: Option<[f64; 2]>,
    /// While previewing, each baked frame's displacement from the first, in
    /// degrees east and north — the track the macro's centre follows, one
    /// entry per frame, for the stamp hover to draw (M27). Empty otherwise,
    /// and one entry for a macro that recorded no movement.
    pub track: Vec<[f64; 2]>,
}

fn mode_of(
    session: &CaptureSession,
    preview: Option<&PreviewScene>,
    step: Option<u32>,
) -> CaptureMode {
    match &session.active {
        Some(active) => CaptureMode {
            active: true,
            first_step: active.first_step,
            placed_steps: active.positions.len() as u32,
            record_movement: active.record_movement,
            visited: active.positions.keys().copied().collect(),
            // The same rule the bake follows: the key, or the great circle
            // between keys (D72).
            position: step.map(|step| active.position_at(step)),
            phase: active.phase,
            keys: active.positions.keys().copied().collect(),
            last_step: active.last_step,
            preview_revision: preview.map(|scene| scene.revision),
            stamp: (active.phase == CapturePhase::Previewing).then_some(active.stamp),
            track: match (&active.baked, active.phase) {
                (Some(baked), CapturePhase::Previewing) => baked
                    .frames
                    .iter()
                    .map(|frame| [frame.dx_deg, frame.dy_deg])
                    .collect(),
                _ => Vec::new(),
            },
        },
        None => CaptureMode {
            active: false,
            first_step: 0,
            placed_steps: 0,
            record_movement: false,
            visited: Vec::new(),
            position: None,
            phase: CapturePhase::Recording,
            keys: Vec::new(),
            last_step: 0,
            preview_revision: None,
            stamp: None,
            track: Vec::new(),
        },
    }
}

/// Begins a capture over a region.
#[tauri::command]
pub fn start_capture(
    state: tauri::State<'_, AppState>,
    region: RegionShape,
    step: u32,
    record_movement: bool,
    kind: Option<String>,
) -> Result<CaptureMode> {
    let kind = kind
        .as_deref()
        .map(crate::projects::parse_field_kind)
        .transpose()?;
    capture_start(&state, region, step, record_movement, kind)
}

/// Implementation of [`start_capture`]. `kind` is the field the map is
/// showing, which is the one recorded (M29); the project's own when absent.
pub fn capture_start(
    state: &AppState,
    region: RegionShape,
    step: u32,
    record_movement: bool,
    kind: Option<ve_core::project::FieldKind>,
) -> Result<CaptureMode> {
    with_session(state, |session| {
        let origin = region_centre(&region)?;
        let kind = kind.unwrap_or(session.require_open()?.project.settings.field_kind);
        // The lockout: one flag on the history, which every write path in the
        // application already goes through (spec.md 8.7).
        session.require_open()?.history.lock();
        session.capturing = CaptureSession {
            active: Some(ActiveCapture {
                shape: region,
                positions: std::collections::BTreeMap::from([(step, origin)]),
                origin,
                record_movement,
                first_step: step,
                phase: CapturePhase::Recording,
                baked: None,
                last_step: step,
                stamp: origin,
                kind,
                stamps: Vec::new(),
            }),
        };
        session.preview = None;
        Ok(mode_of(&session.capturing, None, Some(step)))
    })
}

/// Moves the capture's region at one step. Every step holds its own position.
#[tauri::command]
pub fn place_capture(
    state: tauri::State<'_, AppState>,
    step: u32,
    lon: f64,
    lat: f64,
) -> Result<CaptureMode> {
    capture_place(&state, step, lon, lat)
}

/// Implementation of [`place_capture`].
///
/// **Each frame holds its own position**, initialised to where the region was
/// drawn: moving it at frame 3 moves frame 3 and no other. These are capture
/// state — not keyframes, not the document, not history.
pub fn capture_place(state: &AppState, step: u32, lon: f64, lat: f64) -> Result<CaptureMode> {
    with_session(state, |session| {
        if let Some(active) = session.capturing.active.as_mut()
            && active.phase == CapturePhase::Recording
        {
            active.positions.insert(step, [lon, lat]);
        }
        Ok(mode_of(
            &session.capturing,
            session.preview.as_ref(),
            Some(step),
        ))
    })
}

/// Keys the region at a step where it stands (D72): visiting a step while
/// recording records the position there, so the track the timeline shows is
/// the steps the user went through.
#[tauri::command]
pub fn visit_capture(state: tauri::State<'_, AppState>, step: u32) -> Result<CaptureMode> {
    capture_visit(&state, step)
}

/// Implementation of [`visit_capture`]: a key at `step` with the position
/// the region already has there, unless one is there already. Nothing
/// before the first step, which the ruler refuses.
pub fn capture_visit(state: &AppState, step: u32) -> Result<CaptureMode> {
    with_session(state, |session| {
        if let Some(active) = session.capturing.active.as_mut()
            && active.phase == CapturePhase::Recording
            && step >= active.first_step
            && !active.positions.contains_key(&step)
        {
            let at = active.position_at(step);
            active.positions.insert(step, at);
        }
        Ok(mode_of(
            &session.capturing,
            session.preview.as_ref(),
            Some(step),
        ))
    })
}

/// Removes the region's position key at a step (D72); the position there
/// then interpolates between its neighbours. The first step's key stays.
#[tauri::command]
pub fn unplace_capture(state: tauri::State<'_, AppState>, step: u32) -> Result<CaptureMode> {
    capture_unplace(&state, step)
}

/// Implementation of [`unplace_capture`].
pub fn capture_unplace(state: &AppState, step: u32) -> Result<CaptureMode> {
    with_session(state, |session| {
        if let Some(active) = session.capturing.active.as_mut()
            && active.phase == CapturePhase::Recording
            && step != active.first_step
        {
            active.positions.remove(&step);
        }
        Ok(mode_of(
            &session.capturing,
            session.preview.as_ref(),
            Some(step),
        ))
    })
}

/// Bakes the capture into the session and shows it on an empty map (M26,
/// D71). Nothing is written to disk and nothing to the document.
#[tauri::command(async)]
pub fn preview_capture(state: tauri::State<'_, AppState>, last_step: u32) -> Result<CaptureMode> {
    capture_preview(&state, last_step)
}

/// Implementation of [`preview_capture`].
pub fn capture_preview(state: &AppState, last_step: u32) -> Result<CaptureMode> {
    with_session(state, |session| {
        let baked = {
            let active = session
                .capturing
                .active
                .clone()
                .ok_or(AppError::BadOption {
                    field: "capture",
                    value: "no capture is running".to_owned(),
                })?;
            let open = session.require_open()?;
            Arc::new(bake(
                &active,
                &open.project,
                last_step.max(active.first_step),
            )?)
        };
        let settings = session.require_open()?.project.settings;
        let active = session
            .capturing
            .active
            .as_mut()
            .ok_or(AppError::BadOption {
                field: "capture",
                value: "no capture is running".to_owned(),
            })?;
        active.last_step = last_step.max(active.first_step);
        active.phase = CapturePhase::Previewing;
        active.stamp = active.origin;
        active.baked = Some(Arc::clone(&baked));
        session.preview = Some(preview_scene(active, &baked, settings)?);
        Ok(mode_of(&session.capturing, session.preview.as_ref(), None))
    })
}

/// A click in the preview (spec.md 8.7, M29): a copy of the macro is placed
/// there — **in the preview's own scene**, looping with the original, and in
/// no layer of the document. The placements are for looking at: they go
/// when the preview does, whatever ends it, and are never in the panel or
/// the file. Nothing is written and the history lock stands.
#[tauri::command]
pub fn place_preview(state: tauri::State<'_, AppState>, lon: f64, lat: f64) -> Result<CaptureMode> {
    preview_place(&state, lon, lat)
}

/// Implementation of [`place_preview`]. A new revision, so the scene with
/// one more copy in it is a new set of tiles.
pub fn preview_place(state: &AppState, lon: f64, lat: f64) -> Result<CaptureMode> {
    with_session(state, |session| {
        let settings = session.require_open()?.project.settings;
        let active = session
            .capturing
            .active
            .as_mut()
            .ok_or(AppError::BadOption {
                field: "capture",
                value: "no capture is running".to_owned(),
            })?;
        if let Some(baked) = active.baked.clone()
            && active.phase == CapturePhase::Previewing
        {
            active.stamps.push([wrap180(lon), lat.clamp(-90.0, 90.0)]);
            session.preview = Some(preview_scene(active, &baked, settings)?);
        }
        Ok(mode_of(&session.capturing, session.preview.as_ref(), None))
    })
}

/// Moves the preview's stamp: the macro is shown centred there instead. A
/// new revision, so the old stamp's tiles are unreachable rather than stale.
pub fn preview_stamp(state: &AppState, lon: f64, lat: f64) -> Result<CaptureMode> {
    with_session(state, |session| {
        let settings = session.require_open()?.project.settings;
        let active = session
            .capturing
            .active
            .as_mut()
            .ok_or(AppError::BadOption {
                field: "capture",
                value: "no capture is running".to_owned(),
            })?;
        if let Some(baked) = active.baked.clone()
            && active.phase == CapturePhase::Previewing
        {
            active.stamp = [wrap180(lon), lat.clamp(-90.0, 90.0)];
            session.preview = Some(preview_scene(active, &baked, settings)?);
        }
        Ok(mode_of(&session.capturing, session.preview.as_ref(), None))
    })
}

/// Back from the preview to recording, keys intact; the bake is dropped,
/// since the keys may change.
#[tauri::command]
pub fn edit_capture(state: tauri::State<'_, AppState>) -> Result<CaptureMode> {
    capture_edit(&state)
}

/// Implementation of [`edit_capture`].
pub fn capture_edit(state: &AppState) -> Result<CaptureMode> {
    with_session(state, |session| {
        if let Some(active) = session.capturing.active.as_mut() {
            active.phase = CapturePhase::Recording;
            active.baked = None;
            // The copies were of this bake; the next preview starts clean.
            active.stamps.clear();
        }
        session.preview = None;
        Ok(mode_of(&session.capturing, session.preview.as_ref(), None))
    })
}

/// The one-object project the preview is rendered from (D71).
fn preview_scene(
    active: &ActiveCapture,
    baked: &Arc<Capture>,
    settings: ve_core::project::ProjectSettings,
) -> Result<PreviewScene> {
    // The preview's own project is of the capture's kind, so its one layer
    // flattens whichever kind the map is showing (M29).
    let mut settings = settings;
    settings.field_kind = baked.kind;
    let mut project = ve_core::project::Project::new("Macro preview", settings);
    if let Some(layer) = project.layers.first_mut() {
        layer.parameter = baked.kind;
    }
    let step_count = settings.step_count;
    let mut object = Object::new(ToolKind::Macro, "Preview", step_count);
    object.geometry = baked.shape.clone();
    object.capture = Some(baked.hash.clone());
    object.active_range = ve_core::document::StepRange::new(
        active.first_step.min(step_count.saturating_sub(1)),
        active.last_step.min(step_count.saturating_sub(1)),
    );
    if let Some(anim) = object.props.get_mut(PropId::StampSpace) {
        anim.set_base(PropValue::Enum(1));
    }
    project
        .captures
        .insert(baked.hash.clone(), Arc::clone(baked));
    // The original where it was recorded, and a copy at every place the
    // preview has been clicked (M29) — all in the preview's one layer, all
    // looping together, none of them the document's.
    let places = std::iter::once(active.stamp).chain(active.stamps.iter().copied());
    if let Some(layer) = project.layers.first_mut() {
        for place in places {
            let mut copy = object.clone();
            copy.id = ve_core::id::Id::new();
            let anchor = LonLat::new(wrap180(place[0]), place[1].clamp(-90.0, 90.0))?;
            if let Some(anim) = copy.props.get_mut(PropId::Position) {
                anim.set_base(PropValue::LonLat(anchor));
            }
            layer.objects.push(copy);
        }
    }
    Ok(PreviewScene {
        project,
        // A document's revision is seeded from the clock and counts up by
        // one per edit, so a preview seeded the same way in the same
        // millisecond *was* the document's — and the webview caches tiles by
        // revision, immutably. A bit no document revision will reach keeps
        // the two address spaces apart for good.
        revision: crate::session::fresh_revision() | (1 << 62),
    })
}

/// Abandons a capture, writing nothing.
#[tauri::command]
pub fn cancel_capture(state: tauri::State<'_, AppState>) -> Result<CaptureMode> {
    capture_cancel(&state)
}

/// Implementation of [`cancel_capture`].
pub fn capture_cancel(state: &AppState) -> Result<CaptureMode> {
    with_session(state, |session| {
        session.capturing = CaptureSession::default();
        session.preview = None;
        if let Ok(open) = session.require_open() {
            open.history.unlock();
        }
        Ok(mode_of(&session.capturing, None, None))
    })
}

/// What the capture mode is, for the transport.
#[tauri::command]
pub fn capture_mode(state: tauri::State<'_, AppState>, step: Option<u32>) -> Result<CaptureMode> {
    mode(&state, step)
}

/// Implementation of [`capture_mode`].
pub fn mode(state: &AppState, step: Option<u32>) -> Result<CaptureMode> {
    with_session(state, |session| {
        Ok(mode_of(&session.capturing, session.preview.as_ref(), step))
    })
}

/// Bakes the capture under a name and writes it to the library.
#[tauri::command(async)]
pub fn finish_capture(
    state: tauri::State<'_, AppState>,
    name: String,
    last_step: u32,
) -> Result<MacroLibrary> {
    capture_finish(&state, name, last_step)
}

/// Implementation of [`finish_capture`].
///
/// For each step of the run, the visible composite inside *that step's* region
/// is evaluated with `CpuEvaluator` at the project's grid spacing, undefined
/// kept distinct from calm (D58). **Static** stores no displacement: a region
/// dragged to follow a moving system then yields a macro of that system
/// standing still, which is the whole point of the option.
///
/// No object is created, and the capture is cleared either way.
pub fn capture_finish(state: &AppState, name: String, last_step: u32) -> Result<MacroLibrary> {
    let configured = with_session(state, |session| {
        Ok(session.settings.macro_directory.clone())
    })?;
    let dir = directory(state, &configured);
    let (header, capture) = with_session(state, |session| {
        let active = session
            .capturing
            .active
            .clone()
            .ok_or(AppError::BadOption {
                field: "capture",
                value: "no capture is running".to_owned(),
            })?;
        let open = session.require_open()?;
        let settings = open.project.settings;
        // The preview's bake is the macro, when there is one: what was looked
        // at is what is kept. Finishing straight from recording bakes now.
        let capture = match &active.baked {
            Some(baked) if active.phase == CapturePhase::Previewing => Capture::clone(baked),
            _ => bake(&active, &open.project, last_step.max(active.first_step))?,
        };
        let header = MacroHeader {
            name: if name.trim().is_empty() {
                "Macro".to_owned()
            } else {
                name.trim().to_owned()
            },
            created_unix_s: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or_default(),
            field_kind: match settings.field_kind {
                FieldKind::Wind => "wind".to_owned(),
                FieldKind::Current => "current".to_owned(),
            },
            step_hours: settings.step_hours.hours(),
        };
        // Cleared whatever happens next: a capture that baked and a capture
        // that failed both end the lockout.
        session.capturing = CaptureSession::default();
        session.preview = None;
        session.require_open()?.history.unlock();
        Ok((header, capture))
    })?;

    std::fs::create_dir_all(&dir)?;
    let stem = slug(&header.name, &capture.hash);
    std::fs::write(
        dir.join(format!("{stem}.{EXTENSION}")),
        encode(&header, &capture)?,
    )?;
    library(state)
}

/// Bakes a capture: for each step of the run, the visible composite inside
/// the region **where the region is at that step** (spec.md 8.7), evaluated
/// with `CpuEvaluator` at the project's grid spacing, undefined kept distinct
/// from calm. Shared by the preview and the finish, so what is looked at is
/// what is kept.
fn bake(
    active: &ActiveCapture,
    project: &ve_core::project::Project,
    last_step: u32,
) -> Result<Capture> {
    let settings = project.settings;
    let spacing = settings.resolution.degrees();
    let (half_w, half_h) = half_extents(&active.shape);
    if !(half_w > 0.0 && half_h > 0.0) {
        return Err(AppError::BadOption {
            field: "capture",
            value: "the region has no extent".to_owned(),
        });
    }
    let ni = ((half_w * 2.0 / spacing).ceil() as u32 + 1).min(2_048);
    let nj = ((half_h * 2.0 / spacing).ceil() as u32 + 1).min(2_048);
    let x0 = -f64::from(ni - 1) * spacing / 2.0;
    let y0 = f64::from(nj - 1) * spacing / 2.0;

    let mut frames = Vec::new();
    for step in active.first_step..=last_step {
        // The key at this step, or the great circle between keys (D72).
        let held = active.position_at(step);
        let scene = flatten_kind(project, step, active.kind);
        let mut uv = Vec::with_capacity(ni as usize * nj as usize);
        for j in 0..nj {
            let lat = held[1] + y0 - f64::from(j) * spacing;
            for i in 0..ni {
                let lon = held[0] + x0 + f64::from(i) * spacing;
                let Ok(at) = LonLat::new(wrap180(lon), lat.clamp(-90.0, 90.0)) else {
                    uv.push(UNDEFINED);
                    continue;
                };
                uv.push(match sample_scene_covered(&scene, at) {
                    Some(sample) => [sample.u, sample.v],
                    None => UNDEFINED,
                });
            }
        }
        let elapsed = f64::from(step - active.first_step) * f64::from(settings.step_hours.hours());
        // Static stores no displacement at all.
        let (dx, dy) = if active.record_movement {
            (
                wrap180(held[0] - active.origin[0]),
                held[1] - active.origin[1],
            )
        } else {
            (0.0, 0.0)
        };
        frames.push(CaptureFrame {
            offset_hours: elapsed,
            dx_deg: dx,
            dy_deg: dy,
            uv,
        });
    }

    Capture::new(
        active.kind,
        CaptureLattice {
            ni,
            nj,
            spacing_deg: spacing,
            x0_deg: x0,
            y0_deg: y0,
        },
        f64::from(settings.step_hours.hours()) * 3600.0,
        geometry_of(&active.shape, active.origin),
        frames,
    )
    .map_err(AppError::Core)
}

/// Inserts a library macro as an object, centred on a position.
#[tauri::command(async)]
pub fn insert_macro(
    state: tauri::State<'_, AppState>,
    id: String,
    lon: f64,
    lat: f64,
    step: u32,
    layer: Option<u64>,
) -> Result<ProjectSummary> {
    macro_insert(&state, &id, lon, lat, step, layer)
}

/// Implementation of [`insert_macro`].
///
/// The project takes **its own copy** of the frames, as a `captures/` entry
/// keyed by content hash (D52) — so two inserts of one macro share one entry,
/// and clearing the library leaves both working.
///
/// **The macro begins at `step`.** Its frames run from the object's first
/// active step (spec.md 8.7), and a new object's range began at 0 whatever
/// step it was placed at — so a five-frame macro placed at step 12 had
/// already ended, and the click made an object that showed nothing. It joins
/// `layer` under the one creation rule (D66).
pub fn macro_insert(
    state: &AppState,
    id: &str,
    lon: f64,
    lat: f64,
    step: u32,
    layer: Option<u64>,
) -> Result<ProjectSummary> {
    let configured = with_session(state, |session| {
        Ok(session.settings.macro_directory.clone())
    })?;
    let dir = directory(state, &configured);
    let bytes = std::fs::read(dir.join(format!("{id}.{EXTENSION}")))?;
    let (_, capture) = decode(&bytes)?;
    let capture = Arc::new(capture);
    let anchor = LonLat::new(wrap180(lon), lat.clamp(-90.0, 90.0))?;

    with_session(state, |session| {
        let open = session.require_open()?;
        let step_count = open.project.settings.step_count;
        let object = macro_object(&capture, anchor, step, step_count);
        let target = crate::document::creation_layer(&open.project, layer)?;
        let (layer, index) = (target.id, target.objects.len());
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
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// The macro object a capture becomes when it is placed, at `anchor` and
/// beginning at `step` (spec.md 8.7). One builder for the library's insert
/// and the preview's click, so the two place the same object.
fn macro_object(capture: &Arc<Capture>, anchor: LonLat, step: u32, step_count: u32) -> Object {
    let last = step_count.saturating_sub(1);
    let mut object = Object::new(ToolKind::Macro, "Macro", step_count);
    object.geometry = capture.shape.clone();
    object.capture = Some(capture.hash.clone());
    object.active_range = ve_core::document::StepRange::new(step.min(last), last);
    if let Some(anim) = object.props.get_mut(PropId::Position) {
        anim.set_base(PropValue::LonLat(anchor));
    }
    // A region is map space, so the macro is a projected stamp (D28, D55).
    if let Some(anim) = object.props.get_mut(PropId::StampSpace) {
        anim.set_base(PropValue::Enum(1));
    }
    object
}

/// A file stem from a name: readable, unique, and safe on every filesystem.
fn slug(name: &str, hash: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let stem = stem.trim_matches('-').to_lowercase();
    let stem = if stem.is_empty() {
        "macro".to_owned()
    } else {
        stem
    };
    format!("{stem}-{}", &hash[..8.min(hash.len())])
}

fn wrap180(degrees: f64) -> f64 {
    let wrapped = (degrees + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped <= -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

fn region_centre(region: &RegionShape) -> Result<[f64; 2]> {
    let at = crate::capture::region_anchor(region).ok_or(AppError::BadOption {
        field: "region",
        value: "is nowhere".to_owned(),
    })?;
    Ok([at.lon, at.lat])
}

fn half_extents(region: &RegionShape) -> (f64, f64) {
    crate::capture::region_half_extents(region)
}

fn geometry_of(region: &RegionShape, origin: [f64; 2]) -> ve_core::document::Geometry {
    let anchor = LonLat::new(wrap180(origin[0]), origin[1].clamp(-90.0, 90.0))
        .unwrap_or(LonLat { lon: 0.0, lat: 0.0 });
    crate::capture::region_geometry(region, anchor)
}
