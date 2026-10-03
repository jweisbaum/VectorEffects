//! Fetching the last few days of observed wind and current (spec.md 4.10).
//!
//! The history import's sibling. That one asks for a range of past hours
//! and reads the archives; this one asks for a number of days, counts back
//! from *now*, and reads the near-real-time products. Everything after the
//! question is the same machinery: each product's fields are packed into a
//! GRIB2 file in the data directory and the layer reads that file, so a
//! project opens the same way away from a connection (`crate::history`).
//!
//! What differs is what a near-real-time product is:
//!
//! - **It trails the present.** Every product is about a day behind, so the
//!   end of the period is usually empty. That is not a failure; the layer
//!   holds what there was.
//! - **It has a period.** A daily analysis is the field for its day, so the
//!   layer holds each time for the product's own period (D73) rather than
//!   showing it on one step in twenty-four.
//! - **It is anchored to the period, not to itself.** Several products are
//!   fetched together and each may be missing different times; all of them
//!   count from the period's start, so they agree step for step.
//! - **One failing does not stop the rest.** The products are independent
//!   servers' worth of data; what arrived is imported and what did not is
//!   named.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_core::Command;
use ve_zarr::{FieldSource, Product};

use crate::commands::AppState;
use crate::error::{AppError, Context, Result};
use crate::history::{
    HistoryProgress, MAX_FETCHED_STEPS, Origin, fetch_source_to_file, fetched_layer,
    temperature_layer,
};
use crate::projects::{ProjectSummary, with_session};

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

/// What the dialog asks for.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "NrtRequest.ts")]
pub struct NrtRequest {
    /// Product identifiers, as [`Product::id`] spells them.
    pub products: Vec<String>,
    /// How many days back from today the period starts.
    pub days: u32,
    /// Whether to set the timeline's start to the period's.
    pub set_start_time: bool,
    /// Whether to lengthen the timeline when the period needs more steps
    /// than it has. Never shortens it.
    pub extend_timeline: bool,
}

/// A product that was asked for and did not arrive.
#[derive(Debug, Clone, Serialize, TS, schemars::JsonSchema)]
#[ts(export, export_to = "NrtSkipped.ts")]
pub struct NrtSkipped {
    /// The product, as its layer would have been called.
    pub product: String,
    /// Why, as the reader that failed said it.
    pub why: String,
}

/// What an import did.
#[derive(Debug, Clone, Serialize, TS, schemars::JsonSchema)]
#[ts(export, export_to = "NrtOutcome.ts")]
pub struct NrtOutcome {
    /// The project, with the layers that arrived.
    pub project: ProjectSummary,
    /// The products that did not.
    pub skipped: Vec<NrtSkipped>,
}

/// What a client is told about the products, without reaching the network.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct NrtProducts {
    /// The present moment, ISO 8601 UTC. "The last N days" counts back from
    /// here, and a model's own sense of the date is its training's.
    pub now: String,
    /// What can be fetched.
    pub products: Vec<NrtProduct>,
}

/// One product of [`NrtProducts`].
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct NrtProduct {
    /// What `import_nrt` calls it.
    pub id: String,
    /// What its layer is called.
    pub label: String,
    /// "wind", "current" or "sst". An SST layer is display only: it is drawn
    /// and never exported.
    pub field: String,
    /// How long each of its times stands for, in hours.
    pub period_hours: u32,
    /// The credit its publisher asks to be shown.
    pub credit: String,
    /// Whether it needs the person's NASA Earthdata token (CMC), which is
    /// set in Settings and nowhere else.
    pub needs_earthdata_token: bool,
}

/// The products, as [`NrtProducts`].
pub fn products() -> NrtProducts {
    NrtProducts {
        now: chrono::Utc::now().format("%Y-%m-%dT%H:%MZ").to_string(),
        products: Product::ALL
            .into_iter()
            .map(|product| NrtProduct {
                id: product.id().to_owned(),
                label: product.label().to_owned(),
                field: match product.variable() {
                    ve_zarr::Variable::Wind10m => "wind",
                    ve_zarr::Variable::SurfaceCurrent => "current",
                    ve_zarr::Variable::SeaSurfaceTemperature => "sst",
                }
                .to_owned(),
                period_hours: product.period_hours(),
                credit: product.credit().to_owned(),
                needs_earthdata_token: product.needs_earthdata(),
            })
            .collect(),
    }
}

/// The span an import covers and what the timeline needs to show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    /// 00:00 UTC, the asked number of days before the day of "now".
    pub start_unix_s: i64,
    /// The hour "now" is in.
    pub end_unix_s: i64,
    /// Steps from the start to the last one not past the end.
    pub steps: u32,
}

/// The period `days` back from `now_unix_s`, on a timeline of `step_hours`.
///
/// It starts on a midnight so a daily product has whole days and the
/// timeline's labels start on one; it ends on the current hour because
/// nothing later exists. A period needing more steps than a project can
/// hold is refused here, with the count, before anything is fetched.
pub fn period(now_unix_s: i64, days: u32, step_hours: u32) -> Result<Period> {
    if days == 0 {
        return Err(bad("ask for at least one day"));
    }
    let today = now_unix_s.div_euclid(DAY) * DAY;
    let start_unix_s = today - i64::from(days) * DAY;
    let end_unix_s = now_unix_s.div_euclid(HOUR) * HOUR;
    let stride = i64::from(step_hours.max(1)) * HOUR;
    let steps = (end_unix_s - start_unix_s) / stride + 1;
    if steps > MAX_FETCHED_STEPS as i64 {
        return Err(bad(&format!(
            "{days} days is {steps} steps of {step_hours} h, and a timeline holds \
             {MAX_FETCHED_STEPS}; ask for fewer days"
        )));
    }
    Ok(Period {
        start_unix_s,
        end_unix_s,
        steps: steps as u32,
    })
}

/// The times of a product worth fetching: those a step of the timeline can
/// show, each once.
///
/// A product is walked at the coarser of its own period and the timeline's
/// step — a daily product at its midnights however fine the timeline, an
/// hourly one at every third hour on a three-hourly timeline — from the
/// period's start, to its end, and no further than the timeline's last step.
/// The periods and the steps are all of 1, 3, 6 and 24 hours, each dividing
/// the next, so the coarser of two is always a multiple of the other.
pub fn wanted_times(
    period: &Period,
    step_hours: u32,
    step_count: u32,
    product_period_hours: u32,
) -> Vec<i64> {
    let step = i64::from(step_hours.max(1)) * HOUR;
    let stride = step.max(i64::from(product_period_hours) * HOUR);
    let timeline_end = period.start_unix_s + i64::from(step_count) * step;
    (0..)
        .map(|k| period.start_unix_s + k * stride)
        .take_while(|time| *time <= period.end_unix_s && *time < timeline_end)
        .collect()
}

/// Imports the last few days of the products asked for.
///
/// Runs its work on a plain thread and returns the project, for the reasons
/// [`crate::history::import_history`] gives: the blocking client will not
/// run on the async runtime, and the frontend's spinner, error and refresh
/// all hang off the call.
#[tauri::command(async)]
pub fn import_nrt<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    request: NrtRequest,
) -> Result<NrtOutcome> {
    let worker = app.clone();
    let outcome = std::thread::spawn(move || {
        use tauri::{Emitter, Manager};
        let state = worker.state::<AppState>();
        let now = chrono::Utc::now().timestamp();
        // Read once, here; it reaches PO.DAAC's archive and nothing else.
        let token = crate::earthdata::read(&state.paths);
        let open = |product: Product| product.open_with(token.as_deref());
        nrt_import(&state, &request, now, open, |progress| {
            let _ = worker.emit("nrt://progress", progress);
        })
    })
    .join()
    .map_err(|_| {
        AppError::Internal(
            "The near-real-time import stopped unexpectedly. Nothing was added to the \
             project; the log has the details."
                .to_owned(),
        )
    })?;
    if let Err(err) = &outcome {
        tracing::error!(%err, "near-real-time import failed");
    }
    outcome
}

/// Implementation of [`import_nrt`]: the clock and the way a product is
/// opened are passed in, so neither the time of day nor a network decides
/// what a test sees.
pub fn nrt_import(
    state: &AppState,
    request: &NrtRequest,
    now_unix_s: i64,
    open: impl Fn(Product) -> ve_zarr::Result<Box<dyn FieldSource>>,
    mut on_progress: impl FnMut(HistoryProgress),
) -> Result<NrtOutcome> {
    let products = products_of(request)?;
    let (step_hours, step_count) = with_session(state, |session| {
        let settings = &session.require_open()?.project.settings;
        Ok((settings.step_hours.hours(), settings.step_count))
    })?;
    let period = period(now_unix_s, request.days, step_hours)?;
    // Lengthened, never shortened: a timeline longer than the period keeps
    // whatever is on its later steps.
    let steps = if request.extend_timeline {
        step_count.max(period.steps)
    } else {
        step_count
    };
    let range = (period.start_unix_s, period.end_unix_s);
    let began = std::time::Instant::now();
    tracing::info!(
        products = ?request.products,
        days = request.days,
        steps,
        step_hours,
        "near-real-time import starting"
    );

    let directory = state.paths.history_dir.clone();
    std::fs::create_dir_all(&directory).doing(
        "make the folder fetched data is written to at",
        directory.display(),
    )?;

    let wanted: Vec<(Product, Vec<i64>)> = products
        .iter()
        .map(|product| {
            (
                *product,
                wanted_times(&period, step_hours, steps, product.period_hours()),
            )
        })
        .collect();
    let total: u32 = wanted.iter().map(|(_, times)| times.len() as u32).sum();

    // Fetched and written first, outside the session lock: this is minutes.
    let mut done = 0u32;
    let mut layers = Vec::new();
    let mut skipped = Vec::new();
    for (product, times) in &wanted {
        let origin = Origin {
            id: product.id(),
            label: product.label(),
        };
        on_progress(HistoryProgress {
            archive: origin.label.to_owned(),
            done,
            total,
        });
        let fetched = open(*product)
            .doing("reach", format!("\"{}\"", origin.label))
            .and_then(|source| {
                let source = Arc::<dyn FieldSource>::from(source);
                fetch_source_to_file(
                    &origin,
                    &source,
                    range,
                    times,
                    true,
                    &directory,
                    |arrived| {
                        on_progress(HistoryProgress {
                            archive: origin.label.to_owned(),
                            done: done + arrived,
                            total,
                        });
                    },
                )
            })
            .and_then(|path| {
                if product.variable().is_scalar() {
                    temperature_layer(&origin, &path, range, product.period_hours())
                } else {
                    fetched_layer(&origin, &path, range, product.period_hours())
                }
            });
        done += times.len() as u32;
        match fetched {
            Ok(layer) => layers.push(layer),
            Err(err) => {
                tracing::warn!(product = product.id(), %err, "near-real-time product skipped");
                skipped.push(NrtSkipped {
                    product: origin.label.to_owned(),
                    why: err.to_string(),
                });
            }
        }
    }
    if layers.is_empty() {
        let reasons: Vec<String> = skipped
            .iter()
            .map(|s| format!("{}: {}", s.product, s.why))
            .collect();
        return Err(AppError::Doing {
            doing: "fetch",
            what: "any of the products asked for".to_owned(),
            why: reasons.join("; "),
        });
    }

    let project = with_session(state, |session| {
        let open = session.require_open()?;
        let layers = std::mem::take(&mut layers);
        let settings = &open.project.settings;
        let mut commands = Vec::with_capacity(layers.len() + 2);
        // The period's start, not the first time fetched: the layers count
        // from it, so it is the hour step 0 shows whether or not any product
        // had data there.
        if request.set_start_time && settings.start_unix_s != Some(period.start_unix_s) {
            commands.push(Command::SetStartTime {
                before: settings.start_unix_s,
                after: Some(period.start_unix_s),
            });
        }
        // Decided here, against the timeline as it is *now*, not as it was
        // when the fetch began minutes ago: a timeline that grew in the
        // meantime keeps its length, and this never shortens one. The times
        // fetched were a lower bound on what the steps can show, so a longer
        // timeline loses nothing by it.
        let after = if request.extend_timeline {
            settings.step_count.max(period.steps)
        } else {
            settings.step_count
        };
        if after > settings.step_count {
            commands.push(Command::SetStepCount {
                before: settings.step_count,
                after,
                restore: Vec::new(),
            });
        }
        let base = open.project.layers.len();
        for (at, layer) in layers.into_iter().enumerate() {
            commands.push(Command::AddLayer {
                index: base + at,
                layer: Box::new(layer),
            });
        }
        let command = if commands.len() == 1 {
            commands.remove(0)
        } else {
            Command::Batch {
                label: "Import near-real-time data".to_owned(),
                commands,
            }
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })?;

    tracing::info!(
        layers = project.layer_count,
        skipped = skipped.len(),
        elapsed_s = format!("{:.1}", began.elapsed().as_secs_f64()),
        "near-real-time import finished"
    );
    Ok(NrtOutcome { project, skipped })
}

/// The products a request names, in the order they are read.
fn products_of(request: &NrtRequest) -> Result<Vec<Product>> {
    if request.products.is_empty() {
        return Err(bad("no product was asked for"));
    }
    request
        .products
        .iter()
        .map(|id| {
            Product::parse(id).ok_or_else(|| bad(&format!("{id} is not a product this reads")))
        })
        .collect()
}

fn bad(why: &str) -> AppError {
    AppError::BadOption {
        field: "near-real-time import",
        value: why.to_owned(),
    }
}
