//! The measurement tools (spec.md 10, M8).
//!
//! Dividers, a passage's two paths, and range rings. They are overlays: they
//! contribute nothing to the field and never reach an exported GRIB. What they
//! do reach is the project file, so a passage measured today is there tomorrow.
//!
//! # Where the work is split
//!
//! `ve_core::annotation` holds the model and `ve_core::geo` the geodesy — the
//! polylines, the distances, the bearings, all in metres and degrees. This
//! module is the IPC boundary, so it does the two things a boundary does:
//! it turns a metre into "1 304 nm · 2 415 km", and it turns a gesture into an
//! undoable command. The frontend receives finished geometry and finished text
//! and does no measuring of its own — the same rule the readout follows
//! (spec.md 3).
//!
//! # A bearing here is a course
//!
//! Every bearing this module prints is a **geometric** bearing: the way you
//! would steer, not the way a wind blows. It is therefore never converted to
//! the project's direction convention (spec.md 3.3). A project that names
//! winds by where they come from must not show the reciprocal of a course.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use ve_core::annotation::{self, Annotation, MAX_RINGS, Measurement};
use ve_core::geo::LonLat;
use ve_core::id::Id;
use ve_core::project::Annotations;

use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::with_session;

/// Metres in a nautical mile. Exact, by definition.
const M_PER_NM: f64 = 1852.0;

/// Which tool made a measurement (spec.md 10).
///
/// A mirror of [`annotation::MeasurementKind`], for the same reason
/// [`crate::edit::StampSpace`] mirrors its schema constant: `ve-core` depends
/// on nothing, `ts-rs` included, so a type that crosses IPC is declared on this
/// side of the boundary and converted here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "MeasurementKind.ts")]
pub enum MeasurementKind {
    /// A chain of points, measured leg by leg.
    Dividers,
    /// Both paths between two points.
    Passage,
    /// Geodesic circles about a centre.
    Rings,
}

impl MeasurementKind {
    /// The core's own kind.
    fn core(self) -> annotation::MeasurementKind {
        match self {
            Self::Dividers => annotation::MeasurementKind::Dividers,
            Self::Passage => annotation::MeasurementKind::Passage,
            Self::Rings => annotation::MeasurementKind::Rings,
        }
    }

    /// And back.
    fn of(kind: annotation::MeasurementKind) -> Self {
        match kind {
            annotation::MeasurementKind::Dividers => Self::Dividers,
            annotation::MeasurementKind::Passage => Self::Passage,
            annotation::MeasurementKind::Rings => Self::Rings,
        }
    }
}

/// Which line of a measurement a drawn path is, and therefore how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "PathKind.ts")]
pub enum PathKind {
    /// The shortest path over the sphere.
    GreatCircle,
    /// The constant-bearing path.
    Rhumb,
    /// One circle of a range-ring set.
    Ring,
}

impl PathKind {
    /// The core's own kind.
    fn of(kind: annotation::PathKind) -> Self {
        match kind {
            annotation::PathKind::GreatCircle => Self::GreatCircle,
            annotation::PathKind::Rhumb => Self::Rhumb,
            annotation::PathKind::Ring => Self::Ring,
        }
    }
}

/// One drawn line of a measurement, ready for the overlay.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "MeasuredPathView.ts")]
pub struct MeasuredPathView {
    /// Which path this is, and therefore how it is drawn.
    pub kind: PathKind,
    /// The polyline, as `[lon, lat]`.
    pub points: Vec<[f64; 2]>,
    /// What to write beside it, already in the units the UI shows.
    pub label: String,
    /// Where to write it.
    pub label_at: [f64; 2],
}

/// A measurement, as the map draws it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "MeasurementView.ts")]
pub struct MeasurementView {
    /// Which measurement this is, for dragging and clearing.
    pub id: u64,
    /// Which tool made it.
    pub kind: MeasurementKind,
    /// The points the user placed, as `[lon, lat]`. These are the handles.
    pub handles: Vec<[f64; 2]>,
    /// The lines to draw, in order.
    pub paths: Vec<MeasuredPathView>,
    /// The chain's total, for a measurement that has one.
    pub total: Option<String>,
    /// Where the total belongs: at the last point placed.
    pub total_at: Option<[f64; 2]>,
}

/// What a new measurement is made of.
///
/// One request for all three tools, because the frontend places them with one
/// gesture handler and the fields a tool does not use are the fields it does
/// not send. The alternative — three commands — would have three arms of the
/// same match on the other side of the wire.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "NewMeasurement.ts")]
pub struct NewMeasurement {
    /// Which tool is placing it.
    pub kind: MeasurementKind,
    /// The points placed, as `[lon, lat]`.
    pub points: Vec<[f64; 2]>,
    /// Ring spacing in kilometres, for a ring set. The UI's unit, not the
    /// document's: the conversion happens here, at the boundary.
    #[serde(default)]
    pub interval_km: f64,
    /// How many rings.
    #[serde(default)]
    pub count: u32,
}

/// Formats a distance the way the measurement tools show one: both units.
///
/// Nautical miles first, because these are navigator's tools and a passage is
/// quoted in miles; kilometres beside them because the rest of the app is
/// metric and the map's scale is. Spec.md 10 asks for both together and this
/// is what "together" means.
///
/// The precision follows the magnitude rather than being fixed: 0.4 nm and
/// 3 109 nm are both wanted to about four figures, and a fixed decimal gives
/// one of them noise and the other nothing.
fn distance(metres: f64) -> String {
    let miles = figure(metres / M_PER_NM);
    // Under a kilometre the metric side is metres: "0.40 km" is a worse way of
    // writing 400 m, and a range ring at 400 m is a perfectly ordinary thing
    // to draw round a mark.
    if metres.abs() < 1000.0 {
        format!("{miles} nm · {} m", figure(metres))
    } else {
        format!("{miles} nm · {} km", figure(metres / 1000.0))
    }
}

/// One number, at a precision that suits its size.
///
/// About four significant figures throughout, which is what both a short
/// range ring and an ocean passage want: a fixed decimal gives one of them
/// noise and the other nothing.
fn figure(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude < 10.0 {
        format!("{value:.2}")
    } else if magnitude < 100.0 {
        format!("{value:.1}")
    } else {
        // Thin spaces between thousands: a five-figure distance is unreadable
        // as a run of digits, and a comma is a decimal point to half the world.
        group(value.round() as i64)
    }
}

/// Groups an integer's digits in threes, separated by a thin space.
fn group(value: i64) -> String {
    let digits = value.abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(ch);
    }
    if value < 0 { format!("-{out}") } else { out }
}

/// A bearing as three digits, the way a course is written.
fn bearing(degrees: f64) -> String {
    let rounded = degrees.rem_euclid(360.0).round() as i64 % 360;
    format!("{rounded:03}°")
}

/// The label for one path.
///
/// The two paths of a passage are named — "GC" and "RL" — because a reader
/// looking at two curves needs to know which is which, and the styles alone
/// cannot say it in text. A divider's leg needs no name: there is only one
/// kind of leg.
fn label(kind: PathKind, distance_m: f64, bearing_deg: Option<f64>, named: bool) -> String {
    let prefix = match (named, kind) {
        (true, PathKind::GreatCircle) => "GC ",
        (true, PathKind::Rhumb) => "RL ",
        _ => "",
    };
    match bearing_deg {
        Some(degrees) => format!("{prefix}{} · {}", distance(distance_m), bearing(degrees)),
        None => format!("{prefix}{}", distance(distance_m)),
    }
}

/// Everything the map needs to draw one measurement.
fn view(annotation: &Annotation) -> MeasurementView {
    let measured = annotation.measurement.measure();
    let kind = MeasurementKind::of(annotation.measurement.kind());
    // Only a passage names its paths: it is the one measurement that draws two
    // answers to the same question.
    let named = kind == MeasurementKind::Passage;
    MeasurementView {
        id: annotation.id.raw(),
        kind,
        handles: measured.handles.iter().map(point).collect(),
        paths: measured
            .paths
            .iter()
            .map(|path| MeasuredPathView {
                kind: PathKind::of(path.kind),
                points: path.path.iter().map(point).collect(),
                label: label(
                    PathKind::of(path.kind),
                    path.distance_m,
                    path.bearing_deg,
                    named,
                ),
                label_at: point(&path.label_at),
            })
            .collect(),
        total: measured.total_m.map(|m| match kind {
            // A ring set's "total" is how far the outermost ring reaches, which
            // is a different sentence from a chain's running sum.
            MeasurementKind::Rings => format!("outer {}", distance(m)),
            _ => format!("total {}", distance(m)),
        }),
        total_at: measured.handles.last().map(point),
    }
}

/// A position in the `[lon, lat]` pairs the frontend speaks.
fn point(at: &LonLat) -> [f64; 2] {
    [at.lon, at.lat]
}

/// Reads a `[lon, lat]` pair, refusing one that is not on the earth.
fn position(pair: [f64; 2]) -> Result<LonLat> {
    LonLat::new(pair[0], pair[1]).map_err(Into::into)
}

/// The measurements currently on the map.
#[tauri::command]
pub fn measurements(state: tauri::State<'_, AppState>) -> Result<Vec<MeasurementView>> {
    measurements_of(&state)
}

/// Implementation of [`measurements`].
pub fn measurements_of(state: &AppState) -> Result<Vec<MeasurementView>> {
    with_session(state, |session| {
        let open = session.require_open()?;
        Ok(open
            .project
            .annotations
            .measurements
            .iter()
            .map(view)
            .collect())
    })
}

/// Writes a new set of measurements, as one undoable step.
///
/// Every edit goes through here, which is why there is one command and one
/// inverse. `gesture` is the coalescing key: a drag sends the same one on
/// every pointer report and undoes as a single entry.
fn write(
    state: &AppState,
    gesture: Option<String>,
    edit: impl FnOnce(&mut Vec<Annotation>) -> Result<()>,
) -> Result<Vec<MeasurementView>> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let before = open.project.annotations.clone();
        let mut measurements = before.measurements.clone();
        edit(&mut measurements)?;
        let after = Annotations::of(measurements);
        if after == before {
            return Ok(before.measurements.iter().map(view).collect());
        }

        let command = ve_core::command::Command::SetAnnotations { before, after };
        let (project, history) = (&mut open.project, &mut open.history);
        match gesture {
            Some(key) => history.push_coalesced(project, command, key)?,
            None => history.push(project, command)?,
        }
        // Deliberately *not* `touch()`: the revision is the tile cache's, and
        // a measurement changes no pixel of the field. Bumping it would throw
        // away every rendered tile because someone dropped a pair of dividers
        // on the map.
        let open = session.require_open()?;
        Ok(open
            .project
            .annotations
            .measurements
            .iter()
            .map(view)
            .collect())
    })
}

/// Places a measurement.
#[tauri::command]
pub fn add_measurement(
    state: tauri::State<'_, AppState>,
    measurement: NewMeasurement,
) -> Result<Vec<MeasurementView>> {
    measurement_added(&state, measurement)
}

/// Implementation of [`add_measurement`].
pub fn measurement_added(
    state: &AppState,
    measurement: NewMeasurement,
) -> Result<Vec<MeasurementView>> {
    let placed = placed(measurement)?;
    write(state, None, |measurements| {
        measurements.push(Annotation {
            id: Id::new(),
            measurement: placed,
        });
        Ok(())
    })
}

/// The measurement a leg being drawn would be, read out as the placed one
/// will read (spec.md 10, M29): the dividers' distance and bearing follow
/// the pointer from the first click to the second. Nothing is stored and no
/// lock is consulted; it is the same `view` the committed measurement gets,
/// so what the pointer shows is exactly what the click will keep.
#[tauri::command]
pub fn preview_measurement(measurement: NewMeasurement) -> Result<MeasurementView> {
    measurement_preview(measurement)
}

/// Implementation of [`preview_measurement`].
pub fn measurement_preview(measurement: NewMeasurement) -> Result<MeasurementView> {
    Ok(view(&Annotation {
        id: Id::from_raw(0),
        measurement: placed(measurement)?,
    }))
}

/// A measurement from what the frontend sent, checked.
fn placed(measurement: NewMeasurement) -> Result<Measurement> {
    let points: Vec<LonLat> = measurement
        .points
        .iter()
        .map(|pair| position(*pair))
        .collect::<Result<_>>()?;

    let placed = match measurement.kind {
        MeasurementKind::Dividers => {
            if points.len() < 2 {
                return Err(bad("a chain needs two points"));
            }
            Measurement::Dividers { points }
        }
        MeasurementKind::Passage => {
            let [from, to] = points.as_slice() else {
                return Err(bad("a passage is two points"));
            };
            Measurement::Passage {
                from: *from,
                to: *to,
            }
        }
        MeasurementKind::Rings => {
            let [centre] = points.as_slice() else {
                return Err(bad("a ring set is one centre"));
            };
            Measurement::Rings {
                centre: *centre,
                interval_m: interval(measurement.interval_km)?,
                count: measurement.count.clamp(1, MAX_RINGS),
            }
        }
    };
    Ok(placed)
}

/// Moves one placed point. The drag path, so it coalesces.
#[tauri::command]
pub fn move_measurement_handle(
    state: tauri::State<'_, AppState>,
    id: u64,
    index: usize,
    at: [f64; 2],
) -> Result<Vec<MeasurementView>> {
    handle_moved(&state, id, index, at)
}

/// Implementation of [`move_measurement_handle`].
pub fn handle_moved(
    state: &AppState,
    id: u64,
    index: usize,
    at: [f64; 2],
) -> Result<Vec<MeasurementView>> {
    let to = position(at)?;
    write(
        state,
        Some(format!("measure:{id}:{index}")),
        |measurements| {
            find(measurements, id)?.measurement.move_handle(index, to);
            Ok(())
        },
    )
}

/// Adds a point to the end of a dividers chain.
#[tauri::command]
pub fn extend_measurement(
    state: tauri::State<'_, AppState>,
    id: u64,
    at: [f64; 2],
) -> Result<Vec<MeasurementView>> {
    measurement_extended(&state, id, at)
}

/// Implementation of [`extend_measurement`].
pub fn measurement_extended(
    state: &AppState,
    id: u64,
    at: [f64; 2],
) -> Result<Vec<MeasurementView>> {
    let point = position(at)?;
    write(state, None, |measurements| {
        match &mut find(measurements, id)?.measurement {
            Measurement::Dividers { points } => {
                points.push(point);
                Ok(())
            }
            // Refused rather than ignored: a passage has two ends and a ring
            // set has a centre, so a caller extending one has confused itself
            // about which measurement it is holding.
            _ => Err(bad("only a chain can be extended")),
        }
    })
}

/// Sets a ring set's spacing and count.
#[tauri::command]
pub fn set_measurement_rings(
    state: tauri::State<'_, AppState>,
    id: u64,
    interval_km: f64,
    count: u32,
) -> Result<Vec<MeasurementView>> {
    rings_set(&state, id, interval_km, count)
}

/// Implementation of [`set_measurement_rings`].
pub fn rings_set(
    state: &AppState,
    id: u64,
    interval_km: f64,
    count: u32,
) -> Result<Vec<MeasurementView>> {
    let interval_m = interval(interval_km)?;
    let wanted = count.clamp(1, MAX_RINGS);
    write(
        state,
        Some(format!("measure:{id}:rings")),
        |measurements| match &mut find(measurements, id)?.measurement {
            Measurement::Rings {
                interval_m: spacing,
                count,
                ..
            } => {
                *spacing = interval_m;
                *count = wanted;
                Ok(())
            }
            _ => Err(bad("only a ring set has an interval")),
        },
    )
}

/// Removes one measurement.
#[tauri::command]
pub fn remove_measurement(
    state: tauri::State<'_, AppState>,
    id: u64,
) -> Result<Vec<MeasurementView>> {
    measurement_removed(&state, id)
}

/// Implementation of [`remove_measurement`].
pub fn measurement_removed(state: &AppState, id: u64) -> Result<Vec<MeasurementView>> {
    write(state, None, |measurements| {
        measurements.retain(|a| a.id.raw() != id);
        Ok(())
    })
}

/// Clears one tool's measurements, or every one of them.
///
/// `kind` absent is the global "clear all measurements" of spec.md 10; a kind
/// is that tool's own clear action.
#[tauri::command]
pub fn clear_measurements(
    state: tauri::State<'_, AppState>,
    kind: Option<MeasurementKind>,
) -> Result<Vec<MeasurementView>> {
    measurements_cleared(&state, kind)
}

/// Implementation of [`clear_measurements`].
pub fn measurements_cleared(
    state: &AppState,
    kind: Option<MeasurementKind>,
) -> Result<Vec<MeasurementView>> {
    write(state, None, |measurements| {
        match kind {
            Some(kind) => {
                let kind = kind.core();
                measurements.retain(|a| a.measurement.kind() != kind);
            }
            None => measurements.clear(),
        }
        Ok(())
    })
}

/// The measurement with an id, or an error naming what was missing.
fn find(measurements: &mut [Annotation], id: u64) -> Result<&mut Annotation> {
    measurements
        .iter_mut()
        .find(|a| a.id.raw() == id)
        .ok_or_else(|| bad("no such measurement"))
}

/// A ring interval in metres, refusing one that would draw nothing.
fn interval(km: f64) -> Result<f64> {
    if !km.is_finite() || km <= 0.0 {
        return Err(bad("a ring interval must be a positive distance"));
    }
    Ok(km * 1000.0)
}

/// A refused request, with the reason.
fn bad(why: &str) -> AppError {
    AppError::BadOption {
        field: "measurement",
        value: why.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_distance_carries_both_units() {
        // The passage the acceptance case measures: about 3 100 nm.
        let text = distance(5_759_000.0);
        assert!(text.contains("nm"), "{text}");
        assert!(text.contains("km"), "{text}");
        assert!(text.starts_with("3\u{2009}110"), "{text}");
    }

    #[test]
    fn a_short_distance_keeps_its_figures() {
        // Four figures either way: a fixed decimal gives a long passage noise
        // and a short one nothing.
        assert_eq!(distance(1852.0), "1.00 nm · 1.85 km");
        // Under a kilometre the metric side is metres, not a fraction of one.
        assert_eq!(distance(400.0), "0.22 nm · 400 m");
    }

    #[test]
    fn a_bearing_is_three_digits_and_wraps() {
        assert_eq!(bearing(7.4), "007°");
        assert_eq!(bearing(359.6), "000°");
        assert_eq!(bearing(-1.0), "359°");
        assert_eq!(bearing(360.0), "000°");
    }

    #[test]
    fn only_a_passage_names_its_paths() {
        // The reader needs to know which curve is which, and only a passage
        // draws two.
        assert!(label(PathKind::GreatCircle, 1000.0, Some(90.0), true).starts_with("GC "));
        assert!(label(PathKind::Rhumb, 1000.0, Some(90.0), true).starts_with("RL "));
        assert!(!label(PathKind::GreatCircle, 1000.0, Some(90.0), false).starts_with("GC "));
        // A ring has no bearing, so its label has no course on the end.
        assert!(!label(PathKind::Ring, 1000.0, None, false).contains('°'));
    }

    #[test]
    fn thousands_are_grouped_readably() {
        assert_eq!(group(999), "999");
        assert_eq!(group(1_000), "1\u{2009}000");
        assert_eq!(group(12_345), "12\u{2009}345");
        assert_eq!(group(-1_234_567), "-1\u{2009}234\u{2009}567");
    }
}
