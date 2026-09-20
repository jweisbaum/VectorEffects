//! From decoded messages to the rasters a layer carries.
//!
//! A forecast file is a bag of messages: several parameters, several levels,
//! several times, in whatever order the producer wrote them. This module picks
//! out the wind and current components, pairs each `u` with its `v`, puts the
//! pairs in time order and reorients every grid into the canonical layout
//! `ve_core::raster` expects (spec.md 4.8).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use ve_core::project::FieldKind;
use ve_core::raster::{MISSING, RasterFrame, RasterGrid, RasterSequence};
use ve_core::regrid::{CellCentres, Neighbours, TargetGrid};

use crate::decode::{self, Grid, Header, LatLonGrid, Message};
use crate::error::{GribError, Result};
use crate::resample;

/// Which component of a vector field a message carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    /// Eastward.
    U,
    /// Northward.
    V,
}

/// Classifies a message as a wind or current component, if it is one.
///
/// Wind is discipline 0, category 2 (momentum), numbers 2 and 3; current is
/// discipline 10, category 1, numbers 2 and 3 — the same numbers the writer
/// emits (spec.md 12.1).
pub fn classify(header: &Header) -> Option<(FieldKind, Component)> {
    let component = match header.number {
        2 => Component::U,
        3 => Component::V,
        _ => return None,
    };
    match (header.discipline, header.category) {
        (0, 2) => Some((FieldKind::Wind, component)),
        (10, 1) => Some((FieldKind::Current, component)),
        _ => None,
    }
}

/// Whether a message is a wind or current component at all.
pub fn is_vector_component(header: &Header) -> bool {
    classify(header).is_some()
}

/// How well a message's level matches the field the app models.
///
/// Lower is better. A file often carries wind at several heights; the app's
/// wind is at 10 m (surface type 103, value 10) and its current at the
/// surface (type 160, depth 0). Anything else is taken only when nothing
/// closer exists, so a file with only 100 m wind still imports.
fn level_rank(kind: FieldKind, header: &Header) -> u8 {
    let near = |value: f64, target: f64| (value - target).abs() < 1e-6;
    match kind {
        FieldKind::Wind => match header.surface_type {
            103 if near(header.surface_value, 10.0) => 0,
            103 => 1,
            // Surface (1) and the lowest sigma/hybrid levels are the next
            // best stand-ins.
            1 | 104 | 105 => 2,
            _ => 3,
        },
        FieldKind::Current => match header.surface_type {
            160 if near(header.surface_value, 0.0) => 0,
            160 | 1 => 1,
            _ => 2,
        },
    }
}

/// Grid geometry, normalised for comparing `u` against `v`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Canonical {
    ni: u32,
    nj: u32,
    lon0: f64,
    lat0: f64,
    dlon: f64,
    dlat: f64,
}

impl Canonical {
    fn of(grid: &LatLonGrid) -> Self {
        // The canonical origin is the north-west corner: the file's first
        // point, walked back along whichever axes the scan ran the other way.
        let lon0 = if grid.i_eastward() {
            grid.lo1
        } else {
            grid.lo1 - f64::from(grid.ni - 1) * grid.di
        };
        let lat0 = if grid.j_northward() {
            grid.la1 + f64::from(grid.nj - 1) * grid.dj
        } else {
            grid.la1
        };
        Self {
            ni: grid.ni,
            nj: grid.nj,
            lon0,
            lat0: lat0.clamp(-90.0, 90.0),
            dlon: grid.di,
            dlat: grid.dj,
        }
    }
}

/// Reorders a message's values into the canonical layout.
///
/// Canonical is row-major, column 0 at the west edge, row 0 at the north
/// edge. The file may run its rows east to west, its columns south to north,
/// step down columns rather than along rows, or alternate direction every
/// row; each of those is a permutation of the same nodes, and this walks the
/// canonical grid asking the file where each node sits.
pub fn canonical_order(grid: &LatLonGrid, values: &[f32]) -> Vec<f32> {
    let (ni, nj) = (grid.ni as usize, grid.nj as usize);
    let scan = grid.scan();
    let mut out = Vec::with_capacity(ni * nj);
    for j in 0..nj {
        for i in 0..ni {
            let at = scan.file_index(i, j, ni, nj);
            out.push(values.get(at).copied().unwrap_or(MISSING));
        }
    }
    out
}

/// What an unstructured file is resampled onto, and what that cost.
///
/// The neighbour search is the expensive half and does not depend on the
/// values, so it is done once per (mesh, target) pair and kept here. A caller
/// that persists the map hands it back on the next open and pays nothing;
/// `produced` says which entries are new and therefore worth writing out.
#[derive(Debug)]
pub struct Resampling<'a> {
    /// The lattice to resample onto: the project's own grid.
    pub target: TargetGrid,
    /// Neighbour sets by cache key, read and written.
    pub neighbours: &'a mut BTreeMap<String, Arc<Neighbours>>,
    /// Keys added during this run.
    pub produced: Vec<String>,
}

impl<'a> Resampling<'a> {
    /// Starts a resampling onto `target`, reusing whatever is already known.
    pub fn new(target: TargetGrid, neighbours: &'a mut BTreeMap<String, Arc<Neighbours>>) -> Self {
        Self {
            target,
            neighbours,
            produced: Vec::new(),
        }
    }

    /// The neighbour set for one mesh, computing it if this is the first ask.
    fn for_mesh(&mut self, uuid: &[u8; 16], centres: &CellCentres) -> Arc<Neighbours> {
        let key = Neighbours::cache_key(uuid, &self.target);
        if let Some(found) = self.neighbours.get(&key) {
            return Arc::clone(found);
        }
        let built = Arc::new(Neighbours::build(centres, &self.target));
        self.neighbours.insert(key.clone(), Arc::clone(&built));
        self.produced.push(key);
        built
    }
}

/// The mean spacing of a file's grid in degrees, for choosing a project
/// resolution (spec §4.8).
///
/// A lat/lon file states it. A projected one states its spacing in metres,
/// which is a spacing in degrees of latitude wherever the grid sits. An
/// unstructured one does not state anything at all, so it comes from the
/// cell count: a mesh of `n` roughly equal cells
/// covering the sphere has cells about `sqrt(4π/n)` radians across, which for
/// ICON global's 2,949,120 cells is 0.118° — near enough 13 km, which is what
/// DWD publishes.
pub fn nominal_spacing(messages: &[Message]) -> Option<f64> {
    messages.iter().find_map(|m| match &m.header.grid {
        Grid::LatLon(grid) => Some(grid.di.min(grid.dj)),
        Grid::Projected(grid) => Some(grid.nominal_spacing_deg()),
        Grid::Unstructured(grid) => {
            let cells = f64::from(grid.count);
            (cells > 0.0).then(|| (4.0 * std::f64::consts::PI / cells).sqrt().to_degrees())
        }
    })
}

/// The best message for one component at one time.
struct Candidate {
    rank: u8,
    message: Message,
}

/// Turns decoded messages into one sequence per field kind found.
///
/// Kinds come back in a fixed order, wind before current, so the layers an
/// import creates are in a predictable order. A time with only one
/// component, or whose `u` and `v` grids disagree, is dropped: half a vector
/// is not a field.
pub fn sequences(
    messages: Vec<Message>,
    resampling: Option<&mut Resampling<'_>>,
) -> Result<Vec<RasterSequence>> {
    sequences_reporting(messages, resampling, &|_, _| {})
}

/// [`sequences`], saying how far along it is: `progress(done, total)` in
/// frames, from whichever thread built one.
pub fn sequences_reporting(
    messages: Vec<Message>,
    mut resampling: Option<&mut Resampling<'_>>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<RasterSequence>> {
    // Keyed by kind, then valid time, then component: a `BTreeMap` so the
    // frames come out in time order without a second sort, and so nothing
    // here iterates a `HashMap`.
    let mut slots: BTreeMap<u8, BTreeMap<i64, [Option<Candidate>; 2]>> = BTreeMap::new();
    for message in messages {
        let Some((kind, component)) = classify(&message.header) else {
            continue;
        };
        let kind_key = match kind {
            FieldKind::Wind => 0,
            FieldKind::Current => 1,
        };
        let rank = level_rank(kind, &message.header);
        let time = message.header.valid_unix_s();
        let slot =
            &mut slots.entry(kind_key).or_default().entry(time).or_default()[component as usize];
        let better = slot.as_ref().is_none_or(|current| rank < current.rank);
        if better {
            *slot = Some(Candidate { rank, message });
        }
    }

    let total = slots.values().map(BTreeMap::len).sum();
    let done = AtomicUsize::new(0);
    let tick = || progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
    progress(0, total);

    let mut out = Vec::new();
    let mut dropped = Vec::new();
    for (kind_key, times) in slots {
        let kind = if kind_key == 0 {
            FieldKind::Wind
        } else {
            FieldKind::Current
        };
        let mut pairs = Vec::with_capacity(times.len());
        for (time, [u, v]) in times {
            match (u, v) {
                (Some(u), Some(v)) => pairs.push((time, u.message, v.message)),
                _ => {
                    dropped.push(format!("{kind:?} at {time}: only one component"));
                    tick();
                }
            }
        }
        // A lat/lon frame is a reorder, a pairing and a hash of its own
        // samples and nothing else's, so a file of them is built across the
        // pool. A resampled one shares the neighbour sets being gathered, and
        // its own work is already spread across the pool from the inside.
        let lat_lon = pairs.iter().all(|(_, u, v)| {
            matches!(
                (&u.header.grid, &v.header.grid),
                (Grid::LatLon(_), Grid::LatLon(_))
            )
        });
        // Each pair is taken by value, so its two messages are let go of as
        // its frame is finished: what is held falls as the frames grow, and a
        // large file never has all of both in memory.
        let grids: Vec<_> = if lat_lon {
            pairs
                .into_par_iter()
                .map(|(time, u, v)| {
                    let grid = raster_of(&u, &v, None);
                    tick();
                    (time, grid)
                })
                .collect()
        } else {
            pairs
                .into_iter()
                .map(|(time, u, v)| {
                    let grid = raster_of(&u, &v, resampling.as_deref_mut());
                    tick();
                    (time, grid)
                })
                .collect()
        };
        let mut frames = Vec::new();
        let mut first_time = None;
        for (time, grid) in grids {
            let grid = match grid {
                Ok(grid) => grid,
                Err(why) => {
                    dropped.push(format!("{kind:?} at {time}: {why}"));
                    continue;
                }
            };
            let first = *first_time.get_or_insert(time);
            frames.push(RasterFrame {
                offset_hours: (time - first) as f64 / 3600.0,
                valid_unix_s: time,
                grid: Arc::new(grid),
            });
        }
        if !frames.is_empty() {
            out.push(RasterSequence::new(kind, frames).map_err(GribError::Malformed)?);
        }
    }

    if out.is_empty() {
        let why = if dropped.is_empty() {
            "the file holds no u/v wind (discipline 0, category 2) or current \
             (discipline 10, category 1) messages"
                .to_owned()
        } else {
            dropped.join("; ")
        };
        return Err(GribError::NoVectorField(why));
    }
    Ok(out)
}

/// Builds one frame's raster from a `u`/`v` pair.
///
/// The three paths are the three kinds of grid. A lat/lon file is reordered
/// into the canonical north-west-first layout it already implies, and keeps
/// its own lattice: nothing is interpolated and nothing is lost. A projected
/// one and an unstructured one have no lat/lon lattice to keep, so both are
/// resampled onto the project's grid (spec §4.8). Either way what comes out
/// is an ordinary [`RasterGrid`], which is what keeps the rest of the app —
/// both kernels, the cache, the exporter — unaware that any other kind of
/// grid exists at all.
fn raster_of(
    u: &Message,
    v: &Message,
    resampling: Option<&mut Resampling<'_>>,
) -> std::result::Result<RasterGrid, String> {
    match (&u.header.grid, &v.header.grid) {
        (Grid::LatLon(gu_grid), Grid::LatLon(gv_grid)) => {
            let (gu, gv) = (Canonical::of(gu_grid), Canonical::of(gv_grid));
            if gu != gv {
                return Err("u and v are on different grids".to_owned());
            }
            let us = canonical_order(gu_grid, &u.values);
            let vs = canonical_order(gv_grid, &v.values);
            RasterGrid::new(
                gu.ni,
                gu.nj,
                gu.lon0,
                gu.lat0,
                gu.dlon,
                gu.dlat,
                paired(&us, &vs),
            )
        }
        (Grid::Projected(pu), Grid::Projected(pv)) => {
            if pu != pv {
                return Err("u and v are on different projected grids".to_owned());
            }
            let Some(resampling) = resampling else {
                return Err(
                    "a projected grid needs a target resolution to resample onto".to_owned(),
                );
            };
            resample::resample(pu, &u.values, &v.values, &resampling.target)
        }
        (Grid::Unstructured(mu), Grid::Unstructured(mv)) => {
            if mu != mv {
                return Err("u and v are on different unstructured grids".to_owned());
            }
            let Some(resampling) = resampling else {
                return Err(
                    "an unstructured grid needs a target resolution to resample onto".to_owned(),
                );
            };
            let (_, centres) = crate::icon::bundled(&mu.uuid)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| {
                    format!(
                        "no bundled definition for grid {}, so its {} cells cannot be placed",
                        mu.uuid_hex(),
                        mu.count
                    )
                })?;
            if centres.len() != mu.count as usize {
                return Err(format!(
                    "grid {} has {} cells but its definition holds {}",
                    mu.uuid_hex(),
                    mu.count,
                    centres.len()
                ));
            }
            let neighbours = resampling.for_mesh(&mu.uuid, &centres);
            let uv = neighbours.resample(&centres, &u.values, &v.values);
            let target = resampling.target;
            RasterGrid::new(
                target.ni,
                target.nj,
                target.lon0,
                target.lat0,
                target.dlon,
                target.dlat,
                uv,
            )
        }
        _ => Err("u and v are on different kinds of grid".to_owned()),
    }
}

/// Pairs two components, missing in either making both missing: there is no
/// vector to draw from half of one.
fn paired(us: &[f32], vs: &[f32]) -> Vec<[f32; 2]> {
    us.iter()
        .zip(vs)
        .map(|(&a, &b)| {
            if ve_core::raster::is_missing(a)
                || ve_core::raster::is_missing(b)
                || !a.is_finite()
                || !b.is_finite()
            {
                [MISSING, MISSING]
            } else {
                [a, b]
            }
        })
        .collect()
}

/// What an import found in a file.
#[derive(Debug, Clone)]
pub struct Imported {
    /// One sequence per field kind, wind first.
    pub sequences: Vec<RasterSequence>,
    /// Messages the decoder could not use, for the log or a warning.
    pub skipped: Vec<decode::Skipped>,
}

/// Reads a GRIB2 file into rasters.
///
/// Only the vector components are unpacked; other parameters in the file
/// cost a header parse each and nothing more.
pub fn read_file(path: &Path, resampling: Option<&mut Resampling<'_>>) -> Result<Imported> {
    read_file_reporting(path, resampling, &|_, _| {})
}

/// [`read_file`], saying how far along it is.
///
/// One scale for the two halves of the work: a message unpacked is one unit,
/// and a frame built is two, since it uses up a `u` and a `v`. A file with
/// messages that pair with nothing finishes short of its total, so the last
/// report is made here rather than left to the count.
pub fn read_file_reporting(
    path: &Path,
    resampling: Option<&mut Resampling<'_>>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Imported> {
    let (messages, skipped) =
        read_messages_reporting(path, &|done, total| progress(done, total * 2))?;
    let unpacked = messages.len();
    let sequences = sequences_reporting(messages, resampling, &|frames, _| {
        progress((unpacked + frames * 2).min(unpacked * 2), unpacked * 2);
    })?;
    progress(unpacked * 2, unpacked * 2);
    Ok(Imported { sequences, skipped })
}

/// Decodes a file's vector messages without assembling them.
///
/// The two steps are separate because choosing what to resample *onto* needs
/// the messages first: an unstructured file states no spacing, so the grid a
/// new project gets is derived from the mesh (see [`nominal_spacing`]) and
/// only then can the file be turned into rasters.
pub fn read_messages(path: &Path) -> Result<(Vec<Message>, Vec<decode::Skipped>)> {
    read_messages_reporting(path, &|_, _| {})
}

/// [`read_messages`], with `progress(done, total)` in messages unpacked.
pub fn read_messages_reporting(
    path: &Path,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<(Vec<Message>, Vec<decode::Skipped>)> {
    let bytes = std::fs::read(path)?;
    let decoded = decode::read_reporting(&bytes, is_vector_component, progress)?;
    if decoded.messages.is_empty() {
        let reasons: Vec<String> = decoded
            .skipped
            .iter()
            .take(3)
            .map(|s| format!("message {}: {}", s.index + 1, s.reason))
            .collect();
        return Err(GribError::NoVectorField(if reasons.is_empty() {
            "the file holds no u/v wind or current messages".to_owned()
        } else {
            reasons.join("; ")
        }));
    }
    Ok((decoded.messages, decoded.skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(scan: u8, ni: u32, nj: u32) -> LatLonGrid {
        LatLonGrid {
            ni,
            nj,
            la1: 0.0,
            lo1: 0.0,
            la2: 0.0,
            lo2: 0.0,
            di: 1.0,
            dj: 1.0,
            scan,
        }
    }

    /// A 3 x 2 grid whose canonical layout is
    /// ```text
    ///   0 1 2
    ///   3 4 5
    /// ```
    /// written in each scanning mode, must read back as that.
    #[test]
    fn every_scanning_mode_reads_back_to_the_canonical_layout() {
        let canonical = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let cases: Vec<(u8, Vec<f32>)> = vec![
            (0x00, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]),
            // East to west.
            (0x80, vec![2.0, 1.0, 0.0, 5.0, 4.0, 3.0]),
            // South to north.
            (0x40, vec![3.0, 4.0, 5.0, 0.0, 1.0, 2.0]),
            // Both.
            (0xC0, vec![5.0, 4.0, 3.0, 2.0, 1.0, 0.0]),
            // Columns consecutive, north to south within each.
            (0x20, vec![0.0, 3.0, 1.0, 4.0, 2.0, 5.0]),
            // Columns consecutive, south to north.
            (0x60, vec![3.0, 0.0, 4.0, 1.0, 5.0, 2.0]),
            // Boustrophedon, first row eastward.
            (0x10, vec![0.0, 1.0, 2.0, 5.0, 4.0, 3.0]),
            // Boustrophedon, first row westward.
            (0x90, vec![2.0, 1.0, 0.0, 3.0, 4.0, 5.0]),
        ];
        for (scan, file_order) in cases {
            assert_eq!(
                canonical_order(&grid(scan, 3, 2), &file_order),
                canonical,
                "scan mode {scan:#04x}"
            );
        }
    }

    #[test]
    fn the_canonical_origin_is_the_north_west_corner() {
        let mut g = grid(0x00, 4, 3);
        g.la1 = 50.0;
        g.lo1 = 10.0;
        assert_eq!(
            (Canonical::of(&g).lon0, Canonical::of(&g).lat0),
            (10.0, 50.0)
        );
        // Scanning south to north from 48: the north edge is 48 + 2.
        g.scan = 0x40;
        g.la1 = 48.0;
        assert_eq!(Canonical::of(&g).lat0, 50.0);
        // Scanning east to west from 13: the west edge is 13 - 3.
        g.scan = 0x80;
        g.lo1 = 13.0;
        assert_eq!(Canonical::of(&g).lon0, 10.0);
    }

    fn header(discipline: u8, category: u8, number: u8, surface: (u8, f64)) -> Header {
        Header {
            index: 0,
            discipline,
            centre: 0,
            reference_time: crate::writer::ReferenceTime {
                year: 2026,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
            },
            product_template: 0,
            category,
            number,
            forecast_hours: 0.0,
            surface_type: surface.0,
            surface_value: surface.1,
            grid: Grid::LatLon(grid(0, 2, 2)),
            packing_template: 0,
        }
    }

    #[test]
    fn wind_and_current_components_are_recognised_and_nothing_else_is() {
        assert_eq!(
            classify(&header(0, 2, 2, (103, 10.0))),
            Some((FieldKind::Wind, Component::U))
        );
        assert_eq!(
            classify(&header(10, 1, 3, (160, 0.0))),
            Some((FieldKind::Current, Component::V))
        );
        assert_eq!(classify(&header(0, 0, 0, (103, 2.0))), None, "temperature");
        assert_eq!(classify(&header(0, 2, 1, (103, 10.0))), None, "wind speed");
    }

    #[test]
    fn ten_metre_wind_outranks_other_heights() {
        let ten = header(0, 2, 2, (103, 10.0));
        let hundred = header(0, 2, 2, (103, 100.0));
        let isobaric = header(0, 2, 2, (100, 85_000.0));
        assert!(level_rank(FieldKind::Wind, &ten) < level_rank(FieldKind::Wind, &hundred));
        assert!(level_rank(FieldKind::Wind, &hundred) < level_rank(FieldKind::Wind, &isobaric));
    }

    fn message(h: Header, values: Vec<f32>) -> Message {
        Message { header: h, values }
    }

    #[test]
    fn components_pair_by_time_and_lone_ones_are_dropped() {
        let mut u0 = header(0, 2, 2, (103, 10.0));
        u0.forecast_hours = 0.0;
        let mut v0 = u0.clone();
        v0.number = 3;
        let mut u3 = u0.clone();
        u3.forecast_hours = 3.0;
        let mut v3 = v0.clone();
        v3.forecast_hours = 3.0;
        // A lone u at 6 h, and a 100 m level at 0 h that must lose to 10 m.
        let mut u6 = u0.clone();
        u6.forecast_hours = 6.0;
        let mut u0_high = u0.clone();
        u0_high.surface_value = 100.0;

        let messages = vec![
            message(u3, vec![3.0; 4]),
            message(u0_high, vec![99.0; 4]),
            message(u0, vec![1.0; 4]),
            message(v3, vec![-3.0; 4]),
            message(u6, vec![6.0; 4]),
            message(v0, vec![-1.0; 4]),
        ];
        let out = sequences(messages, None).unwrap();
        assert_eq!(out.len(), 1);
        let wind = &out[0];
        assert_eq!(wind.kind, FieldKind::Wind);
        assert_eq!(wind.frames.len(), 2, "the lone 6 h u is dropped");
        assert_eq!(wind.frames[0].offset_hours, 0.0);
        assert_eq!(wind.frames[1].offset_hours, 3.0);
        assert_eq!(
            wind.frames[0].grid.uv[0],
            [1.0, -1.0],
            "10 m wins over 100 m"
        );
        assert_eq!(wind.frames[1].grid.uv[0], [3.0, -3.0]);
    }

    #[test]
    fn a_file_with_both_kinds_yields_wind_then_current() {
        let wu = header(0, 2, 2, (103, 10.0));
        let mut wv = wu.clone();
        wv.number = 3;
        let cu = header(10, 1, 2, (160, 0.0));
        let mut cv = cu.clone();
        cv.number = 3;
        let out = sequences(
            vec![
                message(cu, vec![0.5; 4]),
                message(cv, vec![0.0; 4]),
                message(wu, vec![10.0; 4]),
                message(wv, vec![0.0; 4]),
            ],
            None,
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, FieldKind::Wind);
        assert_eq!(out[1].kind, FieldKind::Current);
    }

    #[test]
    fn a_missing_node_in_either_component_is_missing_in_both() {
        let u = header(0, 2, 2, (103, 10.0));
        let mut v = u.clone();
        v.number = 3;
        let out = sequences(
            vec![
                message(u, vec![1.0, MISSING, 1.0, 1.0]),
                message(v, vec![1.0, 1.0, MISSING, f32::NAN]),
            ],
            None,
        )
        .unwrap();
        let uv = &out[0].frames[0].grid.uv;
        assert_eq!(uv[0], [1.0, 1.0]);
        assert!(ve_core::raster::is_missing(uv[1][0]) && ve_core::raster::is_missing(uv[1][1]));
        assert!(ve_core::raster::is_missing(uv[2][0]));
        assert!(
            ve_core::raster::is_missing(uv[3][1]),
            "NaN counts as missing"
        );
    }

    #[test]
    fn a_file_of_only_temperature_is_refused_with_a_reason() {
        let err = sequences(
            vec![message(header(0, 0, 0, (103, 2.0)), vec![280.0; 4])],
            None,
        )
        .unwrap_err();
        assert!(matches!(err, GribError::NoVectorField(_)), "{err}");
    }
}
