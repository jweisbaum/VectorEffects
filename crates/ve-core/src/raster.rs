//! Gridded vector fields imported from a GRIB file.
//!
//! An imported field is a regular lat/lon lattice of `u`/`v` samples at one or
//! more forecast times. It is **never project data**: the project file keeps
//! the path it came from and the decoded lattice lives in memory only, beside
//! the layer that owns it (invariants 1 and 2). Deleting the render cache, or
//! the project file's own contents, can therefore never lose or contain a
//! raster.
//!
//! The sampling rule below is the authority: the CPU evaluator calls it and the
//! WGSL kernel mirrors it. A change here without the matching change in the
//! shader is exactly the kind of drift the fidelity suite exists to catch.

use std::fmt;
use std::sync::Arc;

use crate::project::FieldKind;
use crate::vector::Uv;

/// Sentinel for a grid node with no value, as read from a GRIB bitmap.
///
/// A sentinel rather than `NaN` because the GPU kernel has to make the same
/// decision, and NaN comparisons are the one thing shader compilers are free
/// to optimise away.
pub const MISSING: f32 = -1.0e30;

/// A block of nodes, inclusive at both ends (M33).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// First column.
    pub i0: u32,
    /// Last column.
    pub i1: u32,
    /// First row.
    pub j0: u32,
    /// Last row.
    pub j1: u32,
}

/// Whether a stored component marks a missing node.
pub fn is_missing(value: f32) -> bool {
    value <= -1.0e29
}

/// Tolerance, in grid cells, for a sample that lands just past the last node.
///
/// Floating-point index arithmetic puts a point that is exactly on the last
/// node a few ULPs beyond it; without this the outermost row of a regional
/// grid would flicker in and out of coverage.
const EDGE_SLACK: f64 = 1e-6;

/// One time slice of an imported field: a regular lat/lon lattice of `u`/`v`.
///
/// Stored in a canonical orientation whatever the file's scanning mode was:
/// row-major, columns running eastward from `lon0`, rows running southward
/// from `lat0`. Every sampler can then assume one layout.
#[derive(Clone)]
pub struct RasterGrid {
    /// Columns.
    pub ni: u32,
    /// Rows.
    pub nj: u32,
    /// Longitude of column 0, in `[-180, 180)`.
    pub lon0: f64,
    /// Latitude of row 0, the northernmost.
    pub lat0: f64,
    /// Degrees of longitude between columns. Positive: eastward.
    pub dlon: f64,
    /// Degrees of latitude between rows. Positive: the stored direction is
    /// southward.
    pub dlat: f64,
    /// Whether the last column's eastern neighbour is column 0.
    ///
    /// True for a global grid, where a sample between the last column and the
    /// prime meridian interpolates across the seam rather than falling off it.
    pub wraps: bool,
    /// Samples, `[u, v]` in m/s, at index `j * ni + i`. [`MISSING`] where the
    /// file had no value.
    pub uv: Vec<[f32; 2]>,
    /// BLAKE3 of everything above: the render cache key for this slice.
    pub hash: [u8; 32],
}

impl fmt::Debug for RasterGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RasterGrid")
            .field("ni", &self.ni)
            .field("nj", &self.nj)
            .field("lon0", &self.lon0)
            .field("lat0", &self.lat0)
            .field("dlon", &self.dlon)
            .field("dlat", &self.dlat)
            .field("wraps", &self.wraps)
            .field("hash", &hex(&self.hash))
            .finish()
    }
}

impl PartialEq for RasterGrid {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl RasterGrid {
    /// Builds a grid, checking that the samples fill it.
    ///
    /// `lon0` is normalised into `[-180, 180)`. `dlon` and `dlat` must be
    /// positive: callers reorder the file's samples into the canonical
    /// orientation before getting here.
    pub fn new(
        ni: u32,
        nj: u32,
        lon0: f64,
        lat0: f64,
        dlon: f64,
        dlat: f64,
        uv: Vec<[f32; 2]>,
    ) -> Result<Self, String> {
        if ni == 0 || nj == 0 {
            return Err("a grid needs at least one row and one column".to_owned());
        }
        if uv.len() != (ni as usize) * (nj as usize) {
            return Err(format!(
                "grid is {ni} x {nj} = {} points but {} samples were given",
                (ni as usize) * (nj as usize),
                uv.len()
            ));
        }
        if !(dlon > 0.0 && dlat > 0.0) || !dlon.is_finite() || !dlat.is_finite() {
            return Err(format!(
                "grid increments must be positive, got {dlon} x {dlat}"
            ));
        }
        if !(-90.0..=90.0).contains(&lat0) || !lon0.is_finite() {
            return Err(format!("grid origin ({lon0}, {lat0}) is off the earth"));
        }
        let lon0 = normalize_lon(lon0);
        // Wrapping is a property of the extent: `ni` columns cover `ni * dlon`
        // degrees, and once that reaches a full turn the column after the last
        // is the first. A little slack absorbs increments stored in
        // micro-degrees that do not divide 360 exactly.
        let wraps = f64::from(ni) * dlon >= 360.0 - dlon * 0.5;

        let mut hasher = blake3::Hasher::new();
        hasher.update(&ni.to_le_bytes());
        hasher.update(&nj.to_le_bytes());
        hasher.update(&lon0.to_le_bytes());
        hasher.update(&lat0.to_le_bytes());
        hasher.update(&dlon.to_le_bytes());
        hasher.update(&dlat.to_le_bytes());
        for sample in &uv {
            hasher.update(&sample[0].to_le_bytes());
            hasher.update(&sample[1].to_le_bytes());
        }
        let hash = *hasher.finalize().as_bytes();

        Ok(Self {
            ni,
            nj,
            lon0,
            lat0,
            dlon,
            dlat,
            wraps,
            uv,
            hash,
        })
    }

    /// Total nodes.
    pub fn len(&self) -> usize {
        self.uv.len()
    }

    /// Whether the grid has no nodes. It never does; this exists for clippy.
    pub fn is_empty(&self) -> bool {
        self.uv.is_empty()
    }

    /// The sample at column `i`, row `j`, or `None` if the node is missing.
    fn node(&self, i: u32, j: u32) -> Option<[f32; 2]> {
        let sample = self.uv.get(j as usize * self.ni as usize + i as usize)?;
        if is_missing(sample[0]) || is_missing(sample[1]) {
            None
        } else {
            Some(*sample)
        }
    }

    /// The block of nodes a lat/lon box can read, one cell wider each way
    /// so the bilinear blend's corners are inside it (M33).
    ///
    /// `None` when the box reads no node at all: a regional grid and a box
    /// somewhere else entirely. Conservative wherever the arithmetic is
    /// awkward — a box that straddles the gap behind a regional grid takes
    /// the whole row — because saying "maybe" costs a little work and saying
    /// "no" wrongly would drop a field the map should be drawing.
    pub fn window(&self, west: f64, east: f64, north: f64, south: f64) -> Option<Window> {
        let last_i = f64::from(self.ni - 1);
        let last_j = f64::from(self.nj - 1);

        // Rows run north to south from `lat0`.
        let top = (self.lat0 - north) / self.dlat;
        let bottom = (self.lat0 - south) / self.dlat;
        if bottom < -1.0 || top > last_j + 1.0 {
            return None;
        }
        let j0 = (top - 1.0).max(0.0) as u32;
        let j1 = (bottom + 1.0).clamp(0.0, last_j) as u32;

        let span = (east - west).max(0.0);
        if self.wraps || span >= 360.0 - 1e-9 {
            return Some(Window {
                i0: 0,
                i1: self.ni - 1,
                j0,
                j1,
            });
        }
        // Columns are measured eastward from column 0 around the full turn,
        // the same way `sample` measures them.
        let full = 360.0 / self.dlon;
        let west_at = (west - self.lon0).rem_euclid(360.0) / self.dlon;
        let east_at = west_at + span / self.dlon;
        if west_at > last_i + 1.0 {
            // The box begins in the gap behind the grid. Either it reaches
            // round to column 0 — take the row, which is conservative — or it
            // never reaches the grid at all.
            return (east_at >= full).then_some(Window {
                i0: 0,
                i1: self.ni - 1,
                j0,
                j1,
            });
        }
        Some(Window {
            i0: (west_at - 1.0).max(0.0) as u32,
            i1: (east_at + 1.0).clamp(0.0, last_i) as u32,
            j0,
            j1,
        })
    }

    /// The greatest speed among the nodes a lat/lon box can read, in m/s.
    ///
    /// `None` where the box reads no node, or none that has a value. What a
    /// speed filter's upper end is compared against per tile (spec.md 7.10,
    /// M33): a band whose top is above everything the tile holds keeps the
    /// same nodes as any wider band, so the tile's key does not move when the
    /// slider does. The lower end has no such bound — a blend of two vectors
    /// can be slower than either — so only the top is answered here.
    pub fn max_speed_in(&self, west: f64, east: f64, north: f64, south: f64) -> Option<f32> {
        let window = self.window(west, east, north, south)?;
        let mut most: Option<f32> = None;
        for j in window.j0..=window.j1 {
            for i in window.i0..=window.i1 {
                let Some([u, v]) = self.node(i, j) else {
                    continue;
                };
                let speed = u.hypot(v);
                most = Some(most.map_or(speed, |best: f32| best.max(speed)));
            }
        }
        most
    }

    /// Bilinear sample at a position, or `None` where the grid has no value.
    ///
    /// Outside the grid's extent there is nothing; a missing corner is left
    /// out of the blend and the remaining weights renormalised, so a coastline
    /// in a current field does not bleed calm into the water beside it. All
    /// four corners missing is no coverage.
    ///
    /// The WGSL `sample_raster` is a port of this function, decision for
    /// decision.
    pub fn sample(&self, lon: f64, lat: f64) -> Option<Uv> {
        let last_i = f64::from(self.ni - 1);
        let last_j = f64::from(self.nj - 1);

        let fj = (self.lat0 - lat) / self.dlat;
        if !(-EDGE_SLACK..=last_j + EDGE_SLACK).contains(&fj) {
            return None;
        }
        // Longitude is measured eastward from column 0 around the full turn,
        // so a grid that starts at 0 and a sample at -10 meet at 350: the
        // right answer for a global grid and, for a regional one, a value
        // past the last column that the extent check below rejects.
        let fi = (lon - self.lon0).rem_euclid(360.0) / self.dlon;
        if !self.wraps && fi > last_i + EDGE_SLACK {
            return None;
        }

        let fi = fi.clamp(
            0.0,
            if self.wraps {
                f64::from(self.ni)
            } else {
                last_i
            },
        );
        let fj = fj.clamp(0.0, last_j);

        let i0 = (fi.floor() as u32).min(self.ni - 1);
        let j0 = (fj.floor() as u32).min(self.nj - 1);
        let tx = (fi - f64::from(i0)) as f32;
        let ty = (fj - f64::from(j0)) as f32;
        let i1 = if i0 + 1 < self.ni {
            i0 + 1
        } else if self.wraps {
            0
        } else {
            i0
        };
        let j1 = (j0 + 1).min(self.nj - 1);

        let corners = [
            (self.node(i0, j0), (1.0 - tx) * (1.0 - ty)),
            (self.node(i1, j0), tx * (1.0 - ty)),
            (self.node(i0, j1), (1.0 - tx) * ty),
            (self.node(i1, j1), tx * ty),
        ];
        let mut total = 0.0f32;
        let mut u = 0.0f32;
        let mut v = 0.0f32;
        for (node, weight) in corners {
            if let Some(sample) = node {
                total += weight;
                u += sample[0] * weight;
                v += sample[1] * weight;
            }
        }
        if total <= 1e-6 {
            return None;
        }
        Some(Uv {
            u: u / total,
            v: v / total,
        })
    }
}

fn normalize_lon(lon: f64) -> f64 {
    let wrapped = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped >= 180.0 {
        wrapped - 360.0
    } else {
        wrapped
    }
}

/// How far a message's valid time may sit from a step's for the two to be the
/// same time, in hours.
///
/// Both sides are built from whole seconds — a step's forecast hour from its
/// index and the project's step size, a frame's offset from two GRIB valid
/// times — so this absorbs the division and nothing else. Thirty-six
/// milliseconds: far below any time a file or a timeline can express, and far
/// above the noise of `seconds / 3600.0`.
const MATCH_TOLERANCE_HOURS: f64 = 1e-5;

/// One forecast time of an imported field.
#[derive(Debug, Clone, PartialEq)]
pub struct RasterFrame {
    /// Hours after the sequence's first frame.
    ///
    /// Relative rather than absolute because the first frame is aligned with
    /// the project's first step whatever the file's reference time was.
    pub offset_hours: f64,
    /// When the frame is valid, seconds since the Unix epoch. Informational.
    pub valid_unix_s: i64,
    /// The lattice.
    pub grid: Arc<RasterGrid>,
}

/// An imported field: every time slice of one kind found in one file.
#[derive(Debug, Clone)]
pub struct RasterSequence {
    /// Wind or current, from the messages' discipline.
    pub kind: FieldKind,
    /// Frames in time order, the first at offset 0.
    pub frames: Vec<RasterFrame>,
    /// BLAKE3 over the frames' hashes and offsets.
    pub hash: [u8; 32],
}

impl PartialEq for RasterSequence {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl RasterSequence {
    /// Builds a sequence from frames, which must be non-empty and in time
    /// order with the first at offset 0.
    pub fn new(kind: FieldKind, frames: Vec<RasterFrame>) -> Result<Self, String> {
        if frames.is_empty() {
            return Err("a field needs at least one time step".to_owned());
        }
        if frames[0].offset_hours != 0.0 {
            return Err("the first frame must be at offset 0".to_owned());
        }
        if frames
            .windows(2)
            .any(|w| w[1].offset_hours <= w[0].offset_hours)
        {
            return Err("frames must be in strictly increasing time order".to_owned());
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[match kind {
            FieldKind::Wind => 0,
            FieldKind::Current => 1,
        }]);
        for frame in &frames {
            hasher.update(&frame.offset_hours.to_le_bytes());
            hasher.update(&frame.grid.hash);
        }
        Ok(Self {
            kind,
            frames,
            hash: *hasher.finalize().as_bytes(),
        })
    }

    /// The frame valid at a forecast hour, or `None` if the file has none.
    ///
    /// An imported field is data, and a step the file says nothing about is a
    /// step where there is nothing to show (spec.md 4.8). Not the last frame at
    /// or before the hour: holding one forward draws a forecast for a time it
    /// was never made for, and does it most misleadingly past the end of the
    /// file, where the last message would stand for the rest of the timeline
    /// however long that is.
    ///
    /// So an hourly file in a 3-hourly project is read at 0, 3, 6 and its
    /// other messages are never shown; a 3-hourly file in an hourly project
    /// shows nothing at hours 1 and 2; and past the file's last message there
    /// is nothing at all.
    pub fn frame_at(&self, hour: f64) -> Option<&RasterFrame> {
        let index = self
            .frames
            .partition_point(|frame| frame.offset_hours < hour - MATCH_TOLERANCE_HOURS);
        self.frames
            .get(index)
            .filter(|frame| (frame.offset_hours - hour).abs() <= MATCH_TOLERANCE_HOURS)
    }

    /// The fastest speed anywhere in the sequence, in m/s.
    ///
    /// What a speed filter's scale runs to (spec.md 4.8): a filter is set by
    /// looking at the field, and a scale ending at a number the file never
    /// reaches spends most of its travel on nothing. Computed over every node
    /// of every message, which is a pass over the data — the caller is the
    /// layer panel, not the evaluator.
    pub fn fastest_mps(&self) -> f32 {
        self.frames
            .iter()
            .flat_map(|frame| frame.grid.uv.iter())
            .filter(|uv| !is_missing(uv[0]) && !is_missing(uv[1]))
            .map(|uv| uv[0].hypot(uv[1]))
            .fold(0.0, f32::max)
    }

    /// Hours spanned, first message to last.
    pub fn span_hours(&self) -> f64 {
        self.frames.last().map_or(0.0, |f| f.offset_hours)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(
        ni: u32,
        nj: u32,
        lon0: f64,
        lat0: f64,
        d: f64,
        f: impl Fn(u32, u32) -> [f32; 2],
    ) -> RasterGrid {
        let mut uv = Vec::new();
        for j in 0..nj {
            for i in 0..ni {
                uv.push(f(i, j));
            }
        }
        RasterGrid::new(ni, nj, lon0, lat0, d, d, uv).expect("valid grid")
    }

    #[test]
    fn a_global_grid_wraps_and_a_regional_one_does_not() {
        let global = grid(360, 181, 0.0, 90.0, 1.0, |_, _| [0.0, 0.0]);
        assert!(global.wraps);
        let regional = grid(20, 10, -10.0, 60.0, 1.0, |_, _| [0.0, 0.0]);
        assert!(!regional.wraps);
    }

    #[test]
    fn a_node_is_returned_exactly() {
        let g = grid(36, 19, 0.0, 90.0, 10.0, |i, j| [i as f32, j as f32]);
        let s = g.sample(30.0, 70.0).expect("covered");
        assert_eq!((s.u, s.v), (3.0, 2.0));
        // West of the prime meridian reads the eastern end of the row.
        let s = g.sample(-10.0, 90.0).expect("covered");
        assert_eq!((s.u, s.v), (35.0, 0.0));
    }

    /// Hand-computed bilinear: quarter of the way east and half way south.
    #[test]
    fn samples_between_nodes_blend_linearly() {
        let g = grid(4, 3, 0.0, 10.0, 10.0, |i, j| {
            [(i * 10) as f32, (j * 100) as f32]
        });
        let s = g.sample(12.5, 5.0).expect("covered");
        assert!((s.u - 12.5).abs() < 1e-5, "{}", s.u);
        assert!((s.v - 50.0).abs() < 1e-5, "{}", s.v);
    }

    #[test]
    fn a_global_grid_interpolates_across_the_seam() {
        // u equals the column index, so the seam sits between 35 and 0.
        let g = grid(36, 19, 0.0, 90.0, 10.0, |i, _| [i as f32, 0.0]);
        let s = g.sample(-5.0, 0.0).expect("covered");
        assert!((s.u - 17.5).abs() < 1e-5, "{}", s.u);
    }

    #[test]
    fn a_regional_grid_covers_only_its_extent() {
        let g = grid(11, 11, -10.0, 60.0, 1.0, |_, _| [1.0, 1.0]);
        assert!(g.sample(0.0, 50.0).is_some(), "the far corner is a node");
        assert!(g.sample(-10.0, 60.0).is_some(), "the near corner is a node");
        assert!(g.sample(0.5, 55.0).is_none(), "east of the last column");
        assert!(g.sample(-5.0, 49.0).is_none(), "south of the last row");
        assert!(g.sample(-5.0, 61.0).is_none(), "north of the first row");
        assert!(g.sample(170.0, 55.0).is_none(), "the far side of the world");
    }

    #[test]
    fn a_grid_across_the_antimeridian_is_continuous() {
        // 170 E to 170 W (190) in 41 columns of half a degree.
        let g = grid(41, 5, 170.0, 2.0, 0.5, |i, _| [i as f32, 0.0]);
        let s = g.sample(-179.75, 0.0).expect("covered");
        assert!((s.u - 20.5).abs() < 1e-5, "{}", s.u);
        assert!(g.sample(-169.0, 0.0).is_none());
    }

    #[test]
    fn missing_corners_are_left_out_of_the_blend() {
        let g = grid(2, 2, 0.0, 1.0, 1.0, |i, j| {
            if i == 1 && j == 1 {
                [MISSING, MISSING]
            } else {
                [4.0, 0.0]
            }
        });
        let s = g.sample(0.5, 0.5).expect("three corners remain");
        assert!((s.u - 4.0).abs() < 1e-5);
        let all = grid(2, 2, 0.0, 1.0, 1.0, |_, _| [MISSING, MISSING]);
        assert!(all.sample(0.5, 0.5).is_none());
    }

    #[test]
    fn the_hash_follows_the_content() {
        let a = grid(4, 4, 0.0, 10.0, 1.0, |i, _| [i as f32, 0.0]);
        let b = grid(4, 4, 0.0, 10.0, 1.0, |i, _| [i as f32, 0.0]);
        let c = grid(4, 4, 0.0, 10.0, 1.0, |i, _| [i as f32, 0.5]);
        let moved = grid(4, 4, 1.0, 10.0, 1.0, |i, _| [i as f32, 0.0]);
        assert_eq!(a.hash, b.hash);
        assert_ne!(a.hash, c.hash);
        assert_ne!(a.hash, moved.hash);
    }

    fn sequence(offsets: &[f64]) -> RasterSequence {
        let frames = offsets
            .iter()
            .map(|&h| RasterFrame {
                offset_hours: h,
                valid_unix_s: (h * 3600.0) as i64,
                grid: Arc::new(grid(2, 2, 0.0, 1.0, 1.0, |_, _| [h as f32, 0.0])),
            })
            .collect();
        RasterSequence::new(FieldKind::Wind, frames).expect("valid")
    }

    #[test]
    fn a_frame_is_shown_only_at_the_hour_it_is_valid_for() {
        let s = sequence(&[0.0, 3.0, 6.0]);
        let at = |hour: f64| s.frame_at(hour).map(|frame| frame.offset_hours);
        assert_eq!(at(0.0), Some(0.0));
        assert_eq!(at(3.0), Some(3.0));
        assert_eq!(at(6.0), Some(6.0));

        // Between two messages the file says nothing, and neither does the map.
        assert_eq!(at(1.0), None);
        assert_eq!(at(2.0), None);
        assert_eq!(at(5.9), None);
        // And past the end it says nothing for the rest of the timeline, which
        // is where holding the last message was most misleading: a ten-day
        // project showed a six-hour forecast for nine and a half days.
        assert_eq!(at(6.1), None);
        assert_eq!(at(240.0), None);
    }

    /// Both sides are built from whole seconds, so the match has to survive the
    /// division that turns them into hours — and nothing wider than that.
    #[test]
    fn a_frame_matches_through_the_rounding_of_its_own_arithmetic() {
        let s = sequence(&[0.0, 1.5, 3.0]);
        assert!(s.frame_at(1.5 + 1e-9).is_some());
        assert!(s.frame_at(1.5 - 1e-9).is_some());
        // A second either way is a different time, not rounding.
        assert!(s.frame_at(1.5 + 1.0 / 3600.0).is_none());
    }

    #[test]
    fn a_sequence_must_start_at_zero_and_increase() {
        assert!(RasterSequence::new(FieldKind::Wind, vec![]).is_err());
        let late = sequence(&[0.0, 3.0]);
        let mut frames = late.frames.clone();
        frames[0].offset_hours = 1.0;
        assert!(RasterSequence::new(FieldKind::Wind, frames.clone()).is_err());
        frames[0].offset_hours = 0.0;
        frames[1].offset_hours = 0.0;
        assert!(RasterSequence::new(FieldKind::Wind, frames).is_err());
    }
}
