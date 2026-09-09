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

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use ts_rs::TS;
use ve_core::project::{FieldKind, Project};
use ve_core::vector::Uv;
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, write_message};
use ve_render::cpu::CpuEvaluator;
use ve_render::evaluator::FieldEvaluator;
use ve_render::scene::flatten_kind;

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
    /// Originating centre code. 255 means missing, which is the honest default.
    pub centre: u16,
    /// Bits per packed value: 8, 12, 16 or 24 (spec.md 12.3, M19).
    ///
    /// Sixteen unless chosen otherwise, which is what every export before this
    /// option existed wrote; an old caller that sends nothing gets the file it
    /// always got.
    #[serde(default = "default_bits")]
    pub bits: u8,
}

fn default_bits() -> u8 {
    ve_grib::packing::BITS_PER_VALUE
}

/// What an export produced.
#[derive(Debug, Clone, Serialize, TS)]
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

/// Progress, emitted as the `export://progress` event.
#[derive(Debug, Clone, Serialize, TS)]
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
/// An upper bound. A time step whose field is entirely constant — every step of
/// a project with nothing painted in it — needs no data section at all and
/// collapses to a few hundred bytes. Any step with content packs every point,
/// so the estimate is accurate as soon as a project has anything in it.
pub fn estimate(project: &Project, bits: u8) -> ExportEstimate {
    let points = project.settings.resolution.point_count();
    let messages = project.settings.step_count * 2;
    // `bits` per packed value, rounded up to whole octets per message, plus a
    // little for section headers.
    let data = (points * u64::from(bits)).div_ceil(8);
    let bytes = data * u64::from(messages) + u64::from(messages) * 200;
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
pub fn export_estimate(
    state: tauri::State<'_, AppState>,
    bits: Option<u8>,
) -> Result<ExportEstimate> {
    let mut session = state
        .session
        .lock()
        .map_err(|_| AppError::Internal("session lock was poisoned".to_owned()))?;
    Ok(estimate(
        &session.require_open()?.project,
        bits.unwrap_or(ve_grib::packing::BITS_PER_VALUE),
    ))
}

/// Asks a running export to stop.
#[tauri::command]
pub fn cancel_export(cancel: tauri::State<'_, ExportCancel>) {
    cancel.0.store(true, Ordering::Relaxed);
}

/// Writes the open project to a GRIB2 file.
#[tauri::command(async)]
pub fn export_grib(
    app: tauri::AppHandle,
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
    // Refused here, before a file is opened, rather than by the packer on the
    // first message: the widths are the four the dialog offers, and anything
    // else is a caller's bug and not a preference (spec.md 12.3, M19).
    if !ve_grib::packing::BIT_WIDTHS.contains(&request.bits) {
        return Err(AppError::BadOption {
            field: "bits",
            value: format!(
                "{} bits per value; the export offers {:?}",
                request.bits,
                ve_grib::packing::BIT_WIDTHS
            ),
        });
    }
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
                let samples: Vec<Uv> = CpuEvaluator.evaluate(&scene, &points)?;
                for (index, sample) in samples.iter().enumerate() {
                    u[index] = sample.u;
                    v[index] = sample.v;
                }
                let (u_parameter, v_parameter) = parameters(kind);
                for (parameter, values) in [(u_parameter, &u), (v_parameter, &v)] {
                    let spec = MessageSpec {
                        parameter,
                        grid,
                        reference_time,
                        forecast_hour: hour,
                        centre: request.centre,
                        bits: request.bits,
                    };
                    bytes += write_message(&mut out, &spec, values)? as u64;
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
