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
use ve_render::scene::flatten;

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

/// The state of a running capture.
#[derive(Debug, Clone)]
pub struct ActiveCapture {
    /// The region's shape, fixed when the capture began: it cannot be
    /// redrawn or reshaped while recording (spec.md 8.7).
    pub shape: RegionShape,
    /// Where the region sits at each step, by step index. A step the user
    /// never visited keeps the position it was started at.
    pub positions: std::collections::BTreeMap<u32, [f64; 2]>,
    /// Where it was drawn: the position every step starts from.
    pub origin: [f64; 2],
    /// Whether the recorded movement is kept.
    pub record_movement: bool,
    /// The first step of the run.
    pub first_step: u32,
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
}

fn mode_of(session: &CaptureSession) -> CaptureMode {
    match &session.active {
        Some(active) => CaptureMode {
            active: true,
            first_step: active.first_step,
            placed_steps: active.positions.len() as u32,
            record_movement: active.record_movement,
        },
        None => CaptureMode {
            active: false,
            first_step: 0,
            placed_steps: 0,
            record_movement: false,
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
) -> Result<CaptureMode> {
    capture_start(&state, region, step, record_movement)
}

/// Implementation of [`start_capture`].
pub fn capture_start(
    state: &AppState,
    region: RegionShape,
    step: u32,
    record_movement: bool,
) -> Result<CaptureMode> {
    with_session(state, |session| {
        let origin = region_centre(&region)?;
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
            }),
        };
        Ok(mode_of(&session.capturing))
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
        if let Some(active) = session.capturing.active.as_mut() {
            active.positions.insert(step, [lon, lat]);
        }
        Ok(mode_of(&session.capturing))
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
        if let Ok(open) = session.require_open() {
            open.history.unlock();
        }
        Ok(mode_of(&session.capturing))
    })
}

/// What the capture mode is, for the transport.
#[tauri::command]
pub fn capture_mode(state: tauri::State<'_, AppState>) -> Result<CaptureMode> {
    mode(&state)
}

/// Implementation of [`capture_mode`].
pub fn mode(state: &AppState) -> Result<CaptureMode> {
    with_session(state, |session| Ok(mode_of(&session.capturing)))
}

/// Bakes the capture under a name and writes it to the library.
#[tauri::command]
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

        let last = last_step.max(active.first_step);
        let mut frames = Vec::new();
        let mut held = active.origin;
        for step in active.first_step..=last {
            // A step the user never visited keeps the last position the
            // region was put at, which is what dragging it forward means.
            if let Some(at) = active.positions.get(&step) {
                held = *at;
            }
            let scene = flatten(&open.project, step);
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
            let elapsed =
                f64::from(step - active.first_step) * f64::from(settings.step_hours.hours());
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

        let capture = Capture::new(
            settings.field_kind,
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
        .map_err(AppError::Core)?;
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

/// Inserts a library macro as an object, centred on a position.
#[tauri::command]
pub fn insert_macro(
    state: tauri::State<'_, AppState>,
    id: String,
    lon: f64,
    lat: f64,
) -> Result<ProjectSummary> {
    macro_insert(&state, &id, lon, lat)
}

/// Implementation of [`insert_macro`].
///
/// The project takes **its own copy** of the frames, as a `captures/` entry
/// keyed by content hash (D52) — so two inserts of one macro share one entry,
/// and clearing the library leaves both working.
pub fn macro_insert(state: &AppState, id: &str, lon: f64, lat: f64) -> Result<ProjectSummary> {
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
        let mut object = Object::new(ToolKind::Macro, "Macro", open.project.settings.step_count);
        object.geometry = capture.shape.clone();
        object.capture = Some(capture.hash.clone());
        if let Some(anim) = object.props.get_mut(PropId::Position) {
            anim.set_base(PropValue::LonLat(anchor));
        }
        if let Some(anim) = object.props.get_mut(PropId::StampSpace) {
            anim.set_base(PropValue::Enum(1));
        }
        let layer = open
            .project
            .layers
            .last()
            .map(|layer| layer.id)
            .ok_or(AppError::Core(ve_core::CoreError::MissingLayer(0)))?;
        let index = open
            .project
            .layer(layer)
            .map(|layer| layer.objects.len())
            .unwrap_or(0);
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
