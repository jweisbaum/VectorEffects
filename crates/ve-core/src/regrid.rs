//! Resampling an unstructured grid onto a regular lat/lon lattice.
//!
//! Some forecast models do not run on a lat/lon grid at all. ICON's mesh is
//! icosahedral: 2,949,120 cells of roughly equal area, listed in the file as a
//! bare run of values whose positions live in a separate grid definition
//! (spec §4.8). Nothing in this app can consume that directly — the render
//! cache, both evaluation kernels and the exporter all speak
//! [`RasterGrid`](crate::raster::RasterGrid) — so an unstructured field is
//! resampled onto the project's own grid once, at import, and is an ordinary
//! raster from then on. That keeps the WGSL kernel, which would otherwise have
//! to search a point cloud, untouched.
//!
//! **The value at a target node is a point sample, not a cell average.** That
//! is the convention the rest of the app already works in: the evaluator
//! computes each cell's vector at the cell, and the exporter writes what
//! `GridSpec::points` evaluates at cell centres. Averaging here would make an
//! imported field mean something different from a painted one at the same
//! resolution.
//!
//! Interpolation is inverse-distance over the three nearest cells, which on a
//! quasi-uniform mesh is the natural stand-in for the barycentric weights the
//! file gives no triangles for. `u` and `v` are interpolated **separately**,
//! as components: averaging speeds and bearings instead would turn two
//! opposing vectors into a fast one pointing nowhere, and would break at the
//! 360° seam.

use rayon::prelude::*;

use crate::raster::{MISSING, is_missing};

/// Where the cells of an unstructured grid are.
///
/// Degrees, one entry per cell, in the file's own order — which is the order
/// the values arrive in, so the index is the only thing tying them together.
#[derive(Debug, Clone, PartialEq)]
pub struct CellCentres {
    /// Latitude of each cell, degrees in `[-90, 90]`.
    pub lat: Vec<f32>,
    /// Longitude of each cell, degrees in `[-180, 180)`.
    pub lon: Vec<f32>,
}

impl CellCentres {
    /// Builds the centres, checking they pair up and lie on the earth.
    pub fn new(lat: Vec<f32>, lon: Vec<f32>) -> Result<Self, String> {
        if lat.len() != lon.len() {
            return Err(format!(
                "{} latitudes against {} longitudes",
                lat.len(),
                lon.len()
            ));
        }
        if lat.is_empty() {
            return Err("a grid of no cells".to_owned());
        }
        for (k, (&a, &o)) in lat.iter().zip(&lon).enumerate() {
            if !a.is_finite() || !o.is_finite() || !(-90.0..=90.0).contains(&a) {
                return Err(format!("cell {k} is at ({o}, {a})"));
            }
        }
        Ok(Self { lat, lon })
    }

    /// Number of cells.
    pub fn len(&self) -> usize {
        self.lat.len()
    }

    /// Whether there are no cells. Never true of a constructed value.
    pub fn is_empty(&self) -> bool {
        self.lat.is_empty()
    }

    /// Unit vectors on the sphere, which is what the search compares.
    ///
    /// Chord length is monotone in great-circle distance, so the nearest cell
    /// by one is the nearest by the other, and a chord is three subtractions
    /// where a great circle is two trigonometric calls.
    fn unit_vectors(&self) -> Vec<[f32; 3]> {
        self.lat
            .par_iter()
            .zip(&self.lon)
            .map(|(&a, &o)| unit(f64::from(a), f64::from(o)))
            .collect()
    }
}

/// A regular lat/lon lattice to resample onto: the project's own grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetGrid {
    /// Columns.
    pub ni: u32,
    /// Rows.
    pub nj: u32,
    /// Longitude of the first column, degrees.
    pub lon0: f64,
    /// Latitude of the first (northernmost) row, degrees.
    pub lat0: f64,
    /// Longitude step, positive.
    pub dlon: f64,
    /// Latitude step, positive; rows run north to south.
    pub dlat: f64,
}

impl TargetGrid {
    /// Total nodes.
    pub fn len(&self) -> usize {
        self.ni as usize * self.nj as usize
    }

    /// Whether the grid has no nodes.
    pub fn is_empty(&self) -> bool {
        self.ni == 0 || self.nj == 0
    }

    /// The position of node `(i, j)`.
    fn node(&self, i: u32, j: u32) -> (f64, f64) {
        (
            self.lon0 + f64::from(i) * self.dlon,
            self.lat0 - f64::from(j) * self.dlat,
        )
    }

    /// Bytes that identify this grid, for the cache key.
    fn key(&self) -> [u8; 40] {
        let mut out = [0u8; 40];
        out[0..4].copy_from_slice(&self.ni.to_le_bytes());
        out[4..8].copy_from_slice(&self.nj.to_le_bytes());
        out[8..16].copy_from_slice(&self.lon0.to_le_bytes());
        out[16..24].copy_from_slice(&self.lat0.to_le_bytes());
        out[24..32].copy_from_slice(&self.dlon.to_le_bytes());
        out[32..40].copy_from_slice(&self.dlat.to_le_bytes());
        out
    }
}

fn unit(lat_deg: f64, lon_deg: f64) -> [f32; 3] {
    let (a, o) = (lat_deg.to_radians(), lon_deg.to_radians());
    let c = a.cos();
    [(c * o.cos()) as f32, (c * o.sin()) as f32, a.sin() as f32]
}

/// A bucket index over the cells, roughly square on the ground at every
/// latitude.
///
/// A plain lat/lon bucket grid degenerates near the poles, where a degree of
/// longitude is metres and a search window measured in buckets covers no
/// ground at all. Giving each latitude band its own longitude count, scaled by
/// `cos(lat)`, keeps every bucket about `band` degrees across on the ground —
/// so a ring of `r` buckets is a ring of about `r · band` degrees anywhere,
/// including at the pole, where the band is a single bucket holding the cap.
struct Buckets {
    band: f64,
    rows: usize,
    lon_count: Vec<u32>,
    row_start: Vec<u32>,
    offsets: Vec<u32>,
    items: Vec<u32>,
}

impl Buckets {
    fn new(centres: &CellCentres, band: f64) -> Self {
        let rows = ((180.0 / band).ceil() as usize).max(1);
        let mut lon_count = Vec::with_capacity(rows);
        let mut row_start = Vec::with_capacity(rows + 1);
        let mut total: u32 = 0;
        for j in 0..rows {
            let centre = 90.0 - (j as f64 + 0.5) * band;
            let n = ((360.0 * centre.to_radians().cos() / band).round() as i64).clamp(1, 100_000)
                as u32;
            row_start.push(total);
            lon_count.push(n);
            total = total.saturating_add(n);
        }
        row_start.push(total);

        let mut counts = vec![0u32; total as usize];
        let mut of = Vec::with_capacity(centres.len());
        for k in 0..centres.len() {
            let b = bucket_of(
                &lon_count,
                &row_start,
                rows,
                band,
                f64::from(centres.lat[k]),
                f64::from(centres.lon[k]),
            );
            counts[b] += 1;
            of.push(b as u32);
        }
        let mut offsets = Vec::with_capacity(total as usize + 1);
        let mut running = 0u32;
        for c in &counts {
            offsets.push(running);
            running += c;
        }
        offsets.push(running);

        let mut cursor = offsets.clone();
        let mut items = vec![0u32; centres.len()];
        for (k, &b) in of.iter().enumerate() {
            let slot = &mut cursor[b as usize];
            items[*slot as usize] = k as u32;
            *slot += 1;
        }
        Self {
            band,
            rows,
            lon_count,
            row_start,
            offsets,
            items,
        }
    }
}

fn bucket_of(
    lon_count: &[u32],
    row_start: &[u32],
    rows: usize,
    band: f64,
    lat: f64,
    lon: f64,
) -> usize {
    let j = (((90.0 - lat) / band) as usize).min(rows - 1);
    let n = lon_count[j];
    let x = ((lon + 180.0) / 360.0).rem_euclid(1.0);
    let i = ((x * f64::from(n)) as u32).min(n - 1);
    (row_start[j] + i) as usize
}

/// The three cells nearest every node of a target grid.
///
/// This is the expensive half of a resample and the half that does not depend
/// on the values, so it is computed once per (grid, target) pair and kept:
/// see [`Neighbours::encode`]. The weights themselves are *not* stored — three
/// distances recomputed from the cell centres cost a few tens of milliseconds
/// against the half-second the search costs, and storing them would triple
/// the size for nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct Neighbours {
    target: TargetGrid,
    /// How many cells the mesh held, so a decoded set can be checked.
    cells: usize,
    /// Three source cells per target node, nearest first.
    idx: Vec<[u32; 3]>,
}

impl Neighbours {
    /// The grid these were built for.
    pub fn target(&self) -> TargetGrid {
        self.target
    }

    /// Number of target nodes covered.
    pub fn len(&self) -> usize {
        self.idx.len()
    }

    /// Size of the mesh this was built against, which is what
    /// [`Neighbours::decode`] has to be told to check its indices.
    pub fn cells(&self) -> usize {
        self.cells
    }

    /// Whether nothing is covered.
    pub fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }

    /// Finds the three nearest cells for every node of `target`.
    ///
    /// The search widens a ring of buckets until the third-best distance is
    /// inside the ground the ring is *known* to cover, so the answer does not
    /// depend on the bucket size being lucky. Ties go to the lower cell index,
    /// which is what keeps the result identical on every machine — the export
    /// is built from this and has to be byte-reproducible (invariant 4).
    pub fn build(centres: &CellCentres, target: &TargetGrid) -> Self {
        // A bucket about the mean cell spacing across holds a handful of
        // cells, so the first ring is usually the last.
        let band = (41_252.0f64 / centres.len() as f64).sqrt().clamp(0.05, 5.0);
        let buckets = Buckets::new(centres, band);
        let xyz = centres.unit_vectors();

        let idx = (0..target.nj)
            .into_par_iter()
            .flat_map_iter(|j| {
                let mut row = Vec::with_capacity(target.ni as usize);
                for i in 0..target.ni {
                    let (lon, lat) = target.node(i, j);
                    row.push(nearest_three(&buckets, &xyz, lat, lon));
                }
                row.into_iter()
            })
            .collect();

        Self {
            target: *target,
            cells: centres.len(),
            idx,
        }
    }

    /// Interpolates one component onto the target grid.
    ///
    /// A node whose nearest cell is missing takes the nearest *present* one of
    /// the three; with none present it is missing itself, so a bitmap's gaps
    /// survive the resample instead of being filled with a neighbour's wind.
    fn interpolate(&self, centres: &CellCentres, values: &[f32], out: &mut [f32]) {
        let target = self.target;
        out.par_iter_mut().enumerate().for_each(|(k, slot)| {
            let i = (k % target.ni as usize) as u32;
            let j = (k / target.ni as usize) as u32;
            let (lon, lat) = target.node(i, j);
            let here = unit(lat, lon);

            let mut total = 0.0f64;
            let mut sum = 0.0f64;
            for &cell in &self.idx[k] {
                let Some(&value) = values.get(cell as usize) else {
                    continue;
                };
                if is_missing(value) || !value.is_finite() {
                    continue;
                }
                let there = unit(
                    f64::from(centres.lat[cell as usize]),
                    f64::from(centres.lon[cell as usize]),
                );
                let d2 = f64::from(
                    (here[0] - there[0]) * (here[0] - there[0])
                        + (here[1] - there[1]) * (here[1] - there[1])
                        + (here[2] - there[2]) * (here[2] - there[2]),
                );
                // A node that lands on a cell centre takes that cell exactly,
                // rather than dividing by a distance of zero.
                if d2 <= f64::EPSILON {
                    sum = f64::from(value);
                    total = 1.0;
                    break;
                }
                let weight = 1.0 / d2.sqrt();
                sum += weight * f64::from(value);
                total += weight;
            }
            *slot = if total > 0.0 {
                (sum / total) as f32
            } else {
                MISSING
            };
        });
    }

    /// Interpolates a `u`/`v` pair onto the target grid.
    ///
    /// Missing is contagious across the pair, as it is everywhere else: half a
    /// vector is not a field.
    pub fn resample(&self, centres: &CellCentres, u: &[f32], v: &[f32]) -> Vec<[f32; 2]> {
        let n = self.idx.len();
        let mut us = vec![MISSING; n];
        let mut vs = vec![MISSING; n];
        self.interpolate(centres, u, &mut us);
        self.interpolate(centres, v, &mut vs);
        us.into_iter()
            .zip(vs)
            .map(|(a, b)| {
                if is_missing(a) || is_missing(b) {
                    [MISSING, MISSING]
                } else {
                    [a, b]
                }
            })
            .collect()
    }

    /// Packs the neighbour set for storage.
    ///
    /// The three cells of a node are near each other and near the previous
    /// node's, both because a target row walks the earth in order and because
    /// ICON lists its cells in a space-filling order of its own. So the first
    /// cell is stored as a delta from the previous node's first, the other two
    /// as offsets from it, and all three zig-zag varint: a global 0.1° set
    /// falls from 78 MB to 21, and to under 2 once the project's own ZIP
    /// deflates it.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.idx.len() * 3);
        let mut previous = 0i64;
        for triple in &self.idx {
            let first = i64::from(triple[0]);
            put_varint(&mut out, first - previous);
            put_varint(&mut out, i64::from(triple[1]) - first);
            put_varint(&mut out, i64::from(triple[2]) - first);
            previous = first;
        }
        out
    }

    /// Reads back what [`Neighbours::encode`] wrote.
    ///
    /// `target` and `cells` are the context the bytes do not carry: a set
    /// built for another grid or another mesh would decode into plausible
    /// nonsense, so both are checked rather than assumed.
    pub fn decode(bytes: &[u8], target: &TargetGrid, cells: usize) -> Result<Self, String> {
        let expected = target.len();
        let mut idx = Vec::with_capacity(expected);
        let mut at = 0usize;
        let mut previous = 0i64;
        for _ in 0..expected {
            let first = previous + take_varint(bytes, &mut at)?;
            let second = first + take_varint(bytes, &mut at)?;
            let third = first + take_varint(bytes, &mut at)?;
            for value in [first, second, third] {
                if value < 0 || value as usize >= cells {
                    return Err(format!("cell {value} is outside a mesh of {cells}"));
                }
            }
            idx.push([first as u32, second as u32, third as u32]);
            previous = first;
        }
        if at != bytes.len() {
            return Err(format!(
                "{} bytes left after {expected} nodes",
                bytes.len() - at
            ));
        }
        Ok(Self {
            target: *target,
            cells,
            idx,
        })
    }

    /// A stable name for this set: the mesh it is for, and the grid it maps
    /// onto.
    pub fn cache_key(mesh: &[u8], target: &TargetGrid) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(mesh);
        hasher.update(&target.key());
        hasher.finalize().to_hex()[..32].to_string()
    }
}

/// The three nearest cells to `(lat, lon)`, nearest first.
fn nearest_three(buckets: &Buckets, xyz: &[[f32; 3]], lat: f64, lon: f64) -> [u32; 3] {
    let here = unit(lat, lon);
    let home = (((90.0 - lat) / buckets.band) as usize).min(buckets.rows - 1);
    // (chord squared, cell). Ties break on the cell index, so the result does
    // not depend on which thread or bucket order got there first.
    let mut best = [(f64::INFINITY, u32::MAX); 3];

    let mut ring = 1i64;
    loop {
        for db in -ring..=ring {
            let jj = home as i64 + db;
            if jj < 0 || jj >= buckets.rows as i64 {
                continue;
            }
            let jj = jj as usize;
            let n = buckets.lon_count[jj] as i64;
            let centre = (((lon + 180.0) / 360.0).rem_euclid(1.0) * n as f64) as i64;
            // A band near a pole holds fewer longitude buckets than the ring
            // is wide, and stepping `-ring..=ring` through it would land on
            // the same bucket several times and count its cells twice. Once
            // the ring reaches round the band, walk the band instead.
            let (first, last) = if 2 * ring + 1 >= n {
                (0, n - 1)
            } else {
                (-ring, ring)
            };
            for di in first..=last {
                let ii = (centre + di).rem_euclid(n);
                let b = (buckets.row_start[jj] as i64 + ii) as usize;
                let (from, to) = (buckets.offsets[b] as usize, buckets.offsets[b + 1] as usize);
                for &cell in &buckets.items[from..to] {
                    // A widening ring rescans the buckets the last one
                    // covered, so a cell can be offered more than once and
                    // would otherwise fill two of the three slots with
                    // itself.
                    if best.iter().any(|&(_, seen)| seen == cell) {
                        continue;
                    }
                    let there = xyz[cell as usize];
                    let d = f64::from(
                        (here[0] - there[0]) * (here[0] - there[0])
                            + (here[1] - there[1]) * (here[1] - there[1])
                            + (here[2] - there[2]) * (here[2] - there[2]),
                    );
                    if (d, cell) < (best[2].0, best[2].1) {
                        best[2] = (d, cell);
                        if (best[2].0, best[2].1) < (best[1].0, best[1].1) {
                            best.swap(1, 2);
                        }
                        if (best[1].0, best[1].1) < (best[0].0, best[0].1) {
                            best.swap(0, 1);
                        }
                    }
                }
            }
        }

        // A ring of `ring` buckets is `ring · band` degrees of ground in every
        // direction, so anything closer than that chord cannot be beaten by a
        // cell outside it. Stop only when the third best is inside that, which
        // makes the answer independent of the bucket size.
        let covered = (f64::from(ring as f32) * buckets.band).to_radians();
        let chord = 2.0 * (covered / 2.0).sin();
        if best[2].1 != u32::MAX && best[2].0 <= chord * chord {
            break;
        }
        if ring as usize > buckets.rows {
            break;
        }
        ring += 1;
    }

    // A mesh of one or two cells leaves slots unfilled; repeat the nearest so
    // the interpolation has something to weight rather than a sentinel.
    let fallback = best[0].1;
    [
        best[0].1,
        if best[1].1 == u32::MAX {
            fallback
        } else {
            best[1].1
        },
        if best[2].1 == u32::MAX {
            fallback
        } else {
            best[2].1
        },
    ]
}

fn put_varint(out: &mut Vec<u8>, value: i64) {
    let mut zig = ((value << 1) ^ (value >> 63)) as u64;
    loop {
        let byte = (zig & 0x7f) as u8;
        zig >>= 7;
        if zig == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn take_varint(bytes: &[u8], at: &mut usize) -> Result<i64, String> {
    let mut zig = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *bytes
            .get(*at)
            .ok_or_else(|| "the neighbour set ends mid-value".to_owned())?;
        *at += 1;
        zig |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return Err("a neighbour index wider than 64 bits".to_owned());
        }
    }
    Ok(((zig >> 1) as i64) ^ -((zig & 1) as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A coarse global target, small enough to reason about by hand.
    fn target(deg: f64) -> TargetGrid {
        TargetGrid {
            ni: (360.0 / deg) as u32,
            nj: (180.0 / deg) as u32 + 1,
            lon0: -180.0,
            lat0: 90.0,
            dlon: deg,
            dlat: deg,
        }
    }

    /// Cells on a regular lat/lon lattice, which makes the right answer
    /// something we can state independently of the search.
    fn lattice(step: f64) -> CellCentres {
        let mut lat = Vec::new();
        let mut lon = Vec::new();
        let rows = (180.0 / step) as i32 + 1;
        let cols = (360.0 / step) as i32;
        for j in 0..rows {
            for i in 0..cols {
                lat.push((90.0 - f64::from(j) * step) as f32);
                lon.push((-180.0 + f64::from(i) * step) as f32);
            }
        }
        CellCentres::new(lat, lon).expect("a lattice")
    }

    #[test]
    fn a_node_on_a_cell_centre_takes_that_cell_exactly() {
        // Source and target on the same 10 degree lattice: every target node
        // sits on a cell, so interpolation must return that cell untouched.
        let centres = lattice(10.0);
        let grid = target(10.0);
        let neighbours = Neighbours::build(&centres, &grid);

        // A value that differs at every cell, so picking a neighbour instead
        // of the exact hit shows up.
        let u: Vec<f32> = (0..centres.len()).map(|k| k as f32).collect();
        let v: Vec<f32> = u.iter().map(|x| -x).collect();
        let out = neighbours.resample(&centres, &u, &v);

        assert_eq!(out.len(), grid.len());
        for (k, sample) in out.iter().enumerate() {
            assert_eq!(sample[0], u[k], "node {k}");
            assert_eq!(sample[1], v[k], "node {k}");
        }
    }

    #[test]
    fn a_constant_field_survives_interpolation_everywhere() {
        // Whatever the weights are, they sum to one, so a constant field is
        // the check that they do — including at the poles and the seam, where
        // the bucket geometry is at its most awkward.
        let centres = lattice(5.0);
        let grid = target(3.0);
        let neighbours = Neighbours::build(&centres, &grid);
        let u = vec![7.5f32; centres.len()];
        let v = vec![-2.25f32; centres.len()];
        for sample in neighbours.resample(&centres, &u, &v) {
            assert!((sample[0] - 7.5).abs() < 1e-4, "{sample:?}");
            assert!((sample[1] + 2.25).abs() < 1e-4, "{sample:?}");
        }
    }

    #[test]
    fn the_three_nearest_really_are_the_nearest() {
        // Against a brute-force search, at nodes chosen to sit where the
        // bucket layout is least helpful: both poles and either side of the
        // antimeridian.
        let centres = lattice(7.0);
        let grid = TargetGrid {
            ni: 5,
            nj: 5,
            lon0: -179.5,
            lat0: 89.5,
            dlon: 89.9,
            dlat: 44.9,
        };
        let neighbours = Neighbours::build(&centres, &grid);
        let xyz = centres.unit_vectors();

        for k in 0..grid.len() {
            let i = (k % grid.ni as usize) as u32;
            let j = (k / grid.ni as usize) as u32;
            let (lon, lat) = grid.node(i, j);
            let here = unit(lat, lon);
            let mut all: Vec<(f64, u32)> = (0..centres.len())
                .map(|c| {
                    let t = xyz[c];
                    let d = f64::from(
                        (here[0] - t[0]) * (here[0] - t[0])
                            + (here[1] - t[1]) * (here[1] - t[1])
                            + (here[2] - t[2]) * (here[2] - t[2]),
                    );
                    (d, c as u32)
                })
                .collect();
            all.sort_by(|a, b| a.partial_cmp(b).expect("finite distances"));
            let want: Vec<u32> = all.iter().take(3).map(|(_, c)| *c).collect();
            assert_eq!(
                neighbours.idx[k].to_vec(),
                want,
                "node {k} at ({lon}, {lat})"
            );
        }
    }

    #[test]
    fn a_missing_cell_does_not_spread_its_absence() {
        let centres = lattice(10.0);
        let grid = target(10.0);
        let neighbours = Neighbours::build(&centres, &grid);
        let mut u = vec![3.0f32; centres.len()];
        let v = vec![0.0f32; centres.len()];
        u[100] = MISSING;
        let out = neighbours.resample(&centres, &u, &v);
        // The node on the missing cell falls back to its other neighbours.
        assert!(!is_missing(out[100][0]), "{:?}", out[100]);
        assert!((out[100][0] - 3.0).abs() < 1e-4);
    }

    #[test]
    fn a_node_with_nothing_present_is_missing() {
        let centres = lattice(30.0);
        let grid = target(30.0);
        let neighbours = Neighbours::build(&centres, &grid);
        let u = vec![MISSING; centres.len()];
        let v = vec![MISSING; centres.len()];
        for sample in neighbours.resample(&centres, &u, &v) {
            assert!(is_missing(sample[0]) && is_missing(sample[1]));
        }
    }

    #[test]
    fn a_neighbour_set_round_trips_through_its_encoding() {
        let centres = lattice(6.0);
        let grid = target(9.0);
        let neighbours = Neighbours::build(&centres, &grid);
        let bytes = neighbours.encode();
        let back = Neighbours::decode(&bytes, &grid, centres.len()).expect("decodes");
        assert_eq!(back, neighbours);
    }

    #[test]
    fn a_neighbour_set_from_another_mesh_is_refused() {
        let centres = lattice(6.0);
        let grid = target(9.0);
        let bytes = Neighbours::build(&centres, &grid).encode();
        // The same bytes read against a mesh too small to hold the indices.
        assert!(Neighbours::decode(&bytes, &grid, 4).is_err());
        // And against a target of a different size, which runs out of bytes
        // or leaves some over.
        assert!(Neighbours::decode(&bytes, &target(4.0), centres.len()).is_err());
    }

    #[test]
    fn the_cache_key_follows_the_mesh_and_the_target() {
        let a = Neighbours::cache_key(b"mesh-one", &target(1.0));
        assert_eq!(a, Neighbours::cache_key(b"mesh-one", &target(1.0)));
        assert_ne!(a, Neighbours::cache_key(b"mesh-two", &target(1.0)));
        assert_ne!(a, Neighbours::cache_key(b"mesh-one", &target(0.5)));
    }

    #[test]
    fn varints_round_trip_across_the_sign() {
        let mut bytes = Vec::new();
        let values = [0i64, 1, -1, 127, -128, 300_000, -300_000, i32::MAX as i64];
        for v in values {
            put_varint(&mut bytes, v);
        }
        let mut at = 0;
        for v in values {
            assert_eq!(take_varint(&bytes, &mut at).expect("a value"), v);
        }
        assert_eq!(at, bytes.len());
    }
}
