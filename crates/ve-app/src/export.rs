//! GRIB2 export.
//!
//! The exported file is the product, so this path is the authoritative one: it
//! re-evaluates every object at the project's full grid resolution using the
//! CPU evaluator. **No preview pixel is involved** — the view is a proxy, never
//! a source (spec.md, invariant 3).
//!
//! Export may be slow; interaction may not. So this runs off the UI thread,
//! streams one message at a time rather than assembling gigabytes in memory,
//! and can be cancelled.

use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use ts_rs::TS;
use ve_core::project::{FieldKind, Project};
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, write_message_masked};
use ve_render::cpu::CpuEvaluator;
use ve_render::evaluator::FieldEvaluator;
use ve_render::scene::flatten_kind;
use ve_zarr::export::{Layout as ZarrLayout, Writer as ZarrWriter, f16};

use crate::commands::AppState;
use crate::error::{AppError, Context, Result};

/// Set to ask a running export to stop.
#[derive(Debug, Default)]
pub struct ExportCancel(pub AtomicBool);

/// What to export and where.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "ExportRequest.ts")]
pub struct ExportRequest {
    /// Destination path.
    pub path: String,
    /// Forecast reference year.
    pub year: u16,
    /// Month, 1-12.
    pub month: u8,
    /// Day, 1-31.
    pub day: u8,
    /// Hour, 0-23.
    pub hour: u8,
}

/// What an export produced.
#[derive(Debug, Clone, Serialize, JsonSchema, TS)]
#[ts(export, export_to = "ExportResult.ts")]
pub struct ExportResult {
    /// Where it was written.
    pub path: String,
    /// Total bytes.
    pub bytes: u64,
    /// Messages written: two per time step.
    pub messages: u32,
    /// How long it took, in milliseconds.
    pub elapsed_ms: u64,
}

/// What to export and where for a Zarr V3 export.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "ExportZarrRequest.ts")]
pub struct ExportZarrRequest {
    /// Destination directory (normally ending in `.zarr`).
    pub path: String,
    /// Forecast reference year.
    pub year: u16,
    /// Month, 1-12.
    pub month: u8,
    /// Day, 1-31.
    pub day: u8,
    /// Hour, 0-23.
    pub hour: u8,
}

/// What a Zarr V3 export produced.
#[derive(Debug, Clone, Serialize, JsonSchema, TS)]
#[ts(export, export_to = "ExportZarrResult.ts")]
pub struct ExportZarrResult {
    /// Where it was written.
    pub path: String,
    /// Total bytes in the Zarr directory.
    pub bytes: u64,
    /// Number of data chunks written.
    pub chunks: u64,
    /// How long it took, in milliseconds.
    pub elapsed_ms: u64,
}

/// Progress, emitted as the `export://progress` event.
///
/// `Deserialize` as well as `Serialize`: `mcp::tools`'s progress relay reads
/// the event back off the bus to forward it to an MCP client.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export, export_to = "ExportProgress.ts")]
pub struct ExportProgress {
    /// Steps completed.
    pub step: u32,
    /// Steps in total.
    pub total: u32,
    /// Bytes written so far.
    pub bytes: u64,
}

/// An estimate shown before the user commits to an export.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ExportEstimate.ts")]
pub struct ExportEstimate {
    /// Approximate output size in bytes.
    pub bytes: u64,
    /// Messages that will be written.
    pub messages: u32,
    /// Grid points per message.
    pub points_per_message: u64,
    /// The width the estimate was made at.
    pub bits: u8,
    /// The step one level is worth at that width, in knots, over a nominal
    /// ±60 m/s field — the resolution the width implies before the field's own
    /// range is known.
    pub step_knots: f64,
}

/// Estimates the output size without doing any work.
///
/// At 0.1 degrees with 240 steps this is about 12 GB, which the user should see
/// before starting rather than discover afterwards (spec.md 12.4).
///
/// An upper bound including a bitmap. Missing cells have no packed value;
/// constant fields need no packed data, so their files can be much smaller.
pub fn estimate(project: &Project, bits: u8) -> ExportEstimate {
    let points = project.settings.resolution.point_count();
    let messages = project.settings.step_count * 2 * project.kinds_present().len() as u32;
    // `bits` per packed value, rounded up to whole octets per message, plus a
    // little for section headers.
    let data = (points * u64::from(bits)).div_ceil(8);
    let bytes = (data + points.div_ceil(8) + 200 + ve_grib::writer::PROVENANCE.len() as u64 + 5)
        * u64::from(messages);
    ExportEstimate {
        bytes,
        messages,
        points_per_message: points,
        bits,
        step_knots: ve_core::units::knots_from_mps(ve_grib::packing::nominal_step_mps(bits)),
    }
}

/// The parameters this project's field kind exports as.
fn parameters(kind: FieldKind) -> (Parameter, Parameter) {
    match kind {
        FieldKind::Wind => (Parameter::WindU, Parameter::WindV),
        FieldKind::Current => (Parameter::CurrentU, Parameter::CurrentV),
    }
}

/// Estimates the size of the export for the open project.
#[tauri::command]
pub fn export_estimate(state: tauri::State<'_, AppState>) -> Result<ExportEstimate> {
    let mut session = state
        .session
        .lock()
        .map_err(|_| AppError::Internal("session lock was poisoned".to_owned()))?;
    Ok(estimate(
        &session.require_open()?.project,
        ve_grib::packing::BITS_PER_VALUE,
    ))
}

/// Asks a running export to stop.
#[tauri::command]
pub fn cancel_export(cancel: tauri::State<'_, ExportCancel>) {
    cancel.0.store(true, Ordering::Relaxed);
}

/// Writes the open project to a Zarr V3 directory.
///
/// Generic over the Tauri runtime so the MCP service, which is generic for
/// the mock application its tests drive, can call the same command the
/// interface does.
#[tauri::command(async)]
pub fn export_zarr<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    cancel: tauri::State<'_, ExportCancel>,
    request: ExportZarrRequest,
) -> Result<ExportZarrResult> {
    cancel.0.store(false, Ordering::Relaxed);

    let project = {
        let mut session = state
            .session
            .lock()
            .map_err(|_| AppError::Internal("session lock was poisoned".to_owned()))?;
        session.require_open()?.project.clone()
    };

    run_zarr(&project, &request, &cancel.0, |progress| {
        let _ = app.emit("export://progress", progress);
    })
}

/// Writes the open project to a GRIB2 file.
///
/// Generic over the Tauri runtime, for the reason [`export_zarr`] gives.
#[tauri::command(async)]
pub fn export_grib<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    cancel: tauri::State<'_, ExportCancel>,
    request: ExportRequest,
) -> Result<ExportResult> {
    cancel.0.store(false, Ordering::Relaxed);

    // Copy the document out from under the lock: export takes minutes at the
    // finest grid, and holding the session lock for that would freeze the UI.
    let project = {
        let mut session = state
            .session
            .lock()
            .map_err(|_| AppError::Internal("session lock was poisoned".to_owned()))?;
        session.require_open()?.project.clone()
    };

    run(&project, &request, &cancel.0, |progress| {
        let _ = app.emit("export://progress", progress);
    })
}

/// Runs an export. Separated from the command so it can be tested directly.
pub fn run(
    project: &Project,
    request: &ExportRequest,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(ExportProgress),
) -> Result<ExportResult> {
    let started = std::time::Instant::now();
    let settings = &project.settings;
    let grid = GridSpec {
        ni: settings.resolution.ni(),
        nj: settings.resolution.nj(),
        micro_degrees: settings.resolution.micro_degrees(),
    };

    let reference_time = ReferenceTime {
        year: request.year,
        month: request.month,
        day: request.day,
        hour: request.hour,
        minute: 0,
        second: 0,
    };
    reference_time.validate()?;

    // Grid positions are the same for every step, so build them once.
    let points: Vec<ve_core::LonLat> = grid
        .points()
        .map(|(lon, lat)| ve_core::LonLat { lon, lat })
        .collect();

    let path = PathBuf::from(&request.path);
    // Write to a temporary and rename on success, so an interrupted export
    // cannot leave a plausible-looking truncated file behind (spec.md 12.4).
    let temporary = path.with_extension("grib2.partial");
    // One message pair per step for each kind of field the visible layers
    // hold (M29): all the wind layers baked together, then all the current
    // layers, so a file carries at most one u/v pair of each per step.
    let kinds = project.kinds_present();

    let mut bytes = 0u64;
    let mut messages = 0u32;

    let outcome = (|| -> Result<()> {
        let file = std::fs::File::create(&temporary)
            .doing("create the export file", temporary.display())?;
        let mut out = BufWriter::new(file);

        let mut u = vec![0f32; points.len()];
        let mut v = vec![0f32; points.len()];

        for step in 0..settings.step_count {
            if cancel.load(Ordering::Relaxed) {
                return Err(AppError::ExportCancelled);
            }

            let hour = settings.forecast_hour(step);
            for &kind in &kinds {
                let scene = flatten_kind(project, step, kind);
                let samples = CpuEvaluator.evaluate_samples(&scene, &points)?;
                for (index, sample) in samples.iter().enumerate() {
                    u[index] = if sample.coverage > 0.0 {
                        sample.uv.u
                    } else {
                        f32::NAN
                    };
                    v[index] = if sample.coverage > 0.0 {
                        sample.uv.v
                    } else {
                        f32::NAN
                    };
                }
                let (u_parameter, v_parameter) = parameters(kind);
                for (parameter, values) in [(u_parameter, &u), (v_parameter, &v)] {
                    let spec = MessageSpec {
                        parameter,
                        grid,
                        reference_time,
                        forecast_hour: hour,
                        centre: u16::MAX,
                        bits: ve_grib::packing::BITS_PER_VALUE,
                    };
                    bytes += write_message_masked(&mut out, &spec, values)? as u64;
                    messages += 1;
                }
            }

            on_progress(ExportProgress {
                step: step + 1,
                total: settings.step_count,
                bytes,
            });
        }

        out.flush().doing("finish writing", path.display())?;
        Ok(())
    })();

    if let Err(err) = outcome {
        // Leave nothing behind on failure or cancellation.
        let _ = std::fs::remove_file(&temporary);
        return Err(err);
    }

    std::fs::rename(&temporary, &path)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
        .doing("put the finished file in place at", path.display())?;

    Ok(ExportResult {
        path: path.to_string_lossy().into_owned(),
        bytes,
        messages,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Writes the project to a Zarr V3 store (spec §12.3).
///
/// A chunk spans 72 hours, so it cannot be written until every step in it
/// has been evaluated, and a full time chunk of a fine grid does not fit in
/// memory. So each time chunk goes in two phases: every step is evaluated
/// **once** — the evaluation is the whole cost of an export — and its four
/// planes are spooled to a file beside the store as Float16; then the chunks
/// are assembled a band of ten degrees of latitude at a time by seeking the
/// spool, in groups bounded to `ASSEMBLY_BYTES`, and written. The spool is
/// removed with the temporary store on any failure.
pub fn run_zarr(
    project: &Project,
    request: &ExportZarrRequest,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(ExportProgress),
) -> Result<ExportZarrResult> {
    use std::io::{Read, Seek, SeekFrom, Write};

    let started = std::time::Instant::now();
    let settings = &project.settings;
    let grid = GridSpec {
        ni: settings.resolution.ni(),
        nj: settings.resolution.nj(),
        micro_degrees: settings.resolution.micro_degrees(),
    };
    let layout = ZarrLayout::new(
        settings.resolution.degrees(),
        settings.step_count,
        settings.step_hours.hours(),
        grid.ni,
        grid.nj,
    );
    let path = PathBuf::from(&request.path);
    let temporary = PathBuf::from(format!("{}.partial", path.display()));
    let spool_path = PathBuf::from(format!("{}.spool", path.display()));
    if path.exists() {
        return Err(AppError::Doing {
            doing: "write",
            what: path.display().to_string(),
            why: "the destination already exists; choose a new .zarr directory".into(),
        });
    }
    let _ = std::fs::remove_dir_all(&temporary);
    let _ = std::fs::remove_file(&spool_path);

    let reference_time = ReferenceTime {
        year: request.year,
        month: request.month,
        day: request.day,
        hour: request.hour,
        minute: 0,
        second: 0,
    };
    reference_time.validate()?;

    let outcome = (|| -> Result<(u64, u64)> {
        let iso = format!(
            "{:04}-{:02}-{:02}T{:02}:00:00Z",
            reference_time.year, reference_time.month, reference_time.day, reference_time.hour
        );
        let writer = ZarrWriter::create(&temporary, layout, &iso, settings.start_unix_s)
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let points: Vec<ve_core::LonLat> = grid
            .points()
            .map(|(lon, lat)| ve_core::LonLat { lon, lat })
            .collect();
        let kinds = project.kinds_present();
        let ni = grid.ni as usize;
        let nj = grid.nj as usize;
        let plane = ni * nj;
        let spatial = layout.spatial_chunk_points as usize;
        let time_chunk = layout.time_chunk_steps as usize;
        let spatial_x = ni.div_ceil(spatial);
        let spatial_y = nj.div_ceil(spatial);
        let step_count = settings.step_count as usize;
        let time_chunks = step_count.div_ceil(time_chunk);
        // A chunk is always the regular shape, even at the array's edge: the
        // cells past the last row, column or step are padding and hold the
        // fill value. Only the real cells are copied in; the rest stay NaN.
        let chunk_len = time_chunk * 4 * spatial * spatial;
        // How many chunks of one band are assembled together. At 0.1° hourly a
        // chunk is 5.8 MB, so eleven at a time; a coarse project takes a band
        // in one group.
        const ASSEMBLY_BYTES: usize = 64 << 20;
        let group = (ASSEMBLY_BYTES / (chunk_len * 2)).clamp(1, spatial_x);
        let mut chunks_written = 0u64;
        let mut frame = vec![f16::NAN; 4 * plane];
        let mut bytes = vec![0u8; 2 * 4 * plane];
        let mut band = vec![f16::NAN; spatial * ni];
        let mut band_bytes = vec![0u8; 2 * spatial * ni];

        for time_index in 0..time_chunks {
            let time_start = time_index * time_chunk;
            let time_len = (step_count - time_start).min(time_chunk);

            // Phase one: evaluate each step once and spool its four planes.
            let mut spool = std::io::BufWriter::new(
                std::fs::File::create(&spool_path).doing("write", spool_path.display())?,
            );
            for local_time in 0..time_len {
                if cancel.load(Ordering::Relaxed) {
                    return Err(AppError::ExportCancelled);
                }
                let step = (time_start + local_time) as u32;
                frame.fill(f16::NAN);
                for &kind in &kinds {
                    let scene = flatten_kind(project, step, kind);
                    let samples = CpuEvaluator.evaluate_samples(&scene, &points)?;
                    let (u_plane, v_plane) = match kind {
                        FieldKind::Wind => (0, plane),
                        FieldKind::Current => (2 * plane, 3 * plane),
                    };
                    for (index, sample) in samples.iter().enumerate() {
                        if sample.coverage > 0.0 {
                            frame[u_plane + index] = f16::from_f32(sample.uv.u);
                            frame[v_plane + index] = f16::from_f32(sample.uv.v);
                        }
                    }
                }
                for (out, value) in bytes.chunks_exact_mut(2).zip(&frame) {
                    out.copy_from_slice(&value.to_le_bytes());
                }
                spool
                    .write_all(&bytes)
                    .doing("write", spool_path.display())?;
                on_progress(ExportProgress {
                    step: step + 1,
                    total: settings.step_count,
                    bytes: 0,
                });
            }
            spool.flush().doing("write", spool_path.display())?;
            drop(spool);
            let mut spool = std::fs::File::open(&spool_path).doing("read", spool_path.display())?;

            // Phase two: assemble the chunks, a band of ten degrees at a time.
            for y in 0..spatial_y {
                let y0 = y * spatial;
                let rows = (nj - y0).min(spatial);
                for group_start in (0..spatial_x).step_by(group) {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(AppError::ExportCancelled);
                    }
                    let group_end = (group_start + group).min(spatial_x);
                    let mut buffers: Vec<Vec<f16>> = (group_start..group_end)
                        .map(|_| vec![f16::NAN; chunk_len])
                        .collect();
                    for local_time in 0..time_len {
                        for parameter in 0..4 {
                            let offset = ((local_time * 4 + parameter) * nj + y0) * ni * 2;
                            spool
                                .seek(SeekFrom::Start(offset as u64))
                                .doing("read", spool_path.display())?;
                            spool
                                .read_exact(&mut band_bytes[..rows * ni * 2])
                                .doing("read", spool_path.display())?;
                            for (value, raw) in band.iter_mut().zip(band_bytes.chunks_exact(2)) {
                                *value = f16::from_le_bytes([raw[0], raw[1]]);
                            }
                            for row in 0..rows {
                                let source = &band[row * ni..(row + 1) * ni];
                                let destination = (local_time * 4 + parameter) * spatial * spatial
                                    + row * spatial;
                                for (buffer, x) in buffers.iter_mut().zip(group_start..) {
                                    let x0 = x * spatial;
                                    let cols = (ni - x0).min(spatial);
                                    buffer[destination..destination + cols]
                                        .copy_from_slice(&source[x0..x0 + cols]);
                                }
                            }
                        }
                    }
                    for (buffer, x) in buffers.iter().zip(group_start..) {
                        writer
                            .write_chunk(&[time_index as u64, 0, y as u64, x as u64], buffer)
                            .map_err(|error| AppError::Internal(error.to_string()))?;
                        chunks_written += 1;
                    }
                }
            }
        }
        std::fs::remove_file(&spool_path).doing("remove", spool_path.display())?;
        Ok((chunks_written, directory_size(&temporary)?))
    })();

    let (chunks, bytes) = match outcome {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&temporary);
            let _ = std::fs::remove_file(&spool_path);
            return Err(error);
        }
    };
    std::fs::rename(&temporary, &path).map_err(|error| AppError::Doing {
        doing: "put",
        what: path.display().to_string(),
        why: error.to_string(),
    })?;
    Ok(ExportZarrResult {
        path: path.to_string_lossy().into_owned(),
        bytes,
        chunks,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn directory_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total += directory_size(&entry.path())?;
        } else {
            total += metadata.len();
        }
    }
    Ok(total)
}
