//! Moves a field from a cell-centred 0.25 degree grid onto the ERA5 grid.
//!
//! GlobCurrent is 0.25 degree like ERA5, but its values sit at cell centres
//! (-89.875 ... 89.875, -179.875 ... 179.875) where ERA5's sit at cell
//! corners (90 ... -90, 0 ... 359.75). Every ERA5 node is therefore the
//! meeting point of four GlobCurrent cells, exactly half a cell away from
//! each. Bilinear interpolation over those four is the natural estimate, and
//! it puts the current on the identical grid definition as the wind, so the
//! two can share a file.
//!
//! Missing points (NaN) are left out of the average and the weights of the
//! remaining neighbours renormalised, which is what keeps a coastline from
//! bleeding a ring of NaN one cell outwards. A node with no present
//! neighbour at all stays missing.

use std::ops::Range;

use crate::source::Window;

/// A regular grid whose values sit at cell centres, rows in ascending or
/// descending latitude as `dlat` says, columns spanning the full circle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellGrid {
    /// Latitude of the first row.
    pub lat0: f64,
    /// Row spacing, signed.
    pub dlat: f64,
    /// Rows.
    pub nlat: usize,
    /// Longitude of the first column.
    pub lon0: f64,
    /// Column spacing, positive.
    pub dlon: f64,
    /// Columns. `nlon * dlon` must be 360.
    pub nlon: usize,
}

impl CellGrid {
    /// The GlobCurrent grid as published: 720 x 1440, south to north, from
    /// -179.875 east.
    pub const GLOBCURRENT: Self = Self {
        lat0: -89.875,
        dlat: 0.25,
        nlat: 720,
        lon0: -179.875,
        dlon: 0.25,
        nlon: 1440,
    };

    /// Values in one field.
    pub fn len(self) -> usize {
        self.nlat * self.nlon
    }

    /// Whether the grid holds no points.
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// Interpolates `values` (row-major on `src`) onto the ERA5 grid, returning
/// values in GRIB scanning order: 90 degrees north first, prime meridian
/// first, 1440 x 721.
///
/// # Panics
/// Panics if `values.len()` is not `src.len()`, or if the grid does not span
/// the full circle. Both are programming errors, not data errors.
pub fn to_era5_grid(src: CellGrid, values: &[f32]) -> Vec<f32> {
    to_era5_window(src, values, &Window::global())
}

/// [`to_era5_grid`] at only the nodes `window` covers, in the window's own
/// order: what the whole grid would be, cropped, without computing the rest.
/// Only the source cells [`native_window`] names are read, so a reader that
/// fetched just those may leave every other cell NaN.
///
/// # Panics
/// As [`to_era5_grid`].
pub fn to_era5_window(src: CellGrid, values: &[f32], window: &Window) -> Vec<f32> {
    assert_eq!(values.len(), src.len(), "field does not match its grid");
    assert!(
        ((src.nlon as f64) * src.dlon - 360.0).abs() < 1e-6,
        "source grid must span the full circle"
    );

    let mut out = Vec::with_capacity(window.len());
    for j in window.rows() {
        let lat = 90.0 - (j as f64) * 0.25;
        let (j0, j1, t) = row_neighbours(src, lat);
        for i in window.columns() {
            let lon = (i as f64) * 0.25;
            let (i0, i1, s) = column_neighbours(src, lon);
            let candidates = [
                (values[j0 * src.nlon + i0], (1.0 - t) * (1.0 - s)),
                (values[j0 * src.nlon + i1], (1.0 - t) * s),
                (values[j1 * src.nlon + i0], t * (1.0 - s)),
                (values[j1 * src.nlon + i1], t * s),
            ];
            let mut sum = 0.0f64;
            let mut weight = 0.0f64;
            for (v, w) in candidates {
                if v.is_finite() && w > 0.0 {
                    sum += f64::from(v) * w;
                    weight += w;
                }
            }
            out.push(if weight > 0.0 {
                (sum / weight) as f32
            } else {
                f32::NAN
            });
        }
    }
    out
}

/// The source cells a window's nodes are interpolated from: one block of
/// rows, and the columns as one block or, where they cross the source's own
/// first column, two. Each is a cell wider every way than the nodes need,
/// so a node that lands exactly on a cell boundary is never a cell short.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeWindow {
    /// Source rows, in the source's own order.
    pub rows: Range<usize>,
    /// Source columns: one range, or two when the window crosses the
    /// source's seam — the east part first, from the column it starts at to
    /// the end of the row, then from column 0.
    pub columns: Vec<Range<usize>>,
}

impl NativeWindow {
    /// Whether this is every cell of `src`.
    #[allow(
        clippy::single_range_in_vec_init,
        reason = "a list of column ranges, which has one range unless it crosses the seam"
    )]
    pub fn is_whole(&self, src: CellGrid) -> bool {
        self.rows == (0..src.nlat) && self.columns == [0..src.nlon]
    }
}

/// The cells of `src` that the nodes of `window` read.
#[allow(
    clippy::single_range_in_vec_init,
    reason = "a list of column ranges, which has one range unless it crosses the seam"
)]
pub fn native_window(src: CellGrid, window: &Window) -> NativeWindow {
    if window.is_global() {
        return NativeWindow {
            rows: 0..src.nlat,
            columns: vec![0..src.nlon],
        };
    }
    // Rows: the fractional row of the window's northern and southern
    // nodes, whichever way the source runs, and the pair bracketing each.
    let row_of = |j: u32| (90.0 - f64::from(j) * 0.25 - src.lat0) / src.dlat;
    let (a, b) = (row_of(window.j0), row_of(window.j0 + window.nj - 1));
    let (lo, hi) = (a.min(b).floor() - 1.0, a.max(b).floor() + 2.0);
    let last = (src.nlat - 1) as f64;
    let rows = (lo.clamp(0.0, last) as usize)..(hi.clamp(0.0, last) as usize + 1);

    // Columns: from the cell west of the first node to the one east of the
    // last, as a start and a count that may run past the row's end.
    let first = (f64::from(window.i0) * 0.25 - src.lon0).rem_euclid(360.0) / src.dlon;
    let width = f64::from(window.ni - 1) * 0.25 / src.dlon;
    let start = first.floor() as i64 - 1;
    let count = ((first + width).floor() as i64 + 2 - start + 1) as usize;
    let n = src.nlon;
    let columns = if count >= n {
        vec![0..n]
    } else {
        let start = start.rem_euclid(n as i64) as usize;
        if start + count <= n {
            vec![start..start + count]
        } else {
            vec![start..n, 0..start + count - n]
        }
    };
    NativeWindow { rows, columns }
}

/// Puts a block read off the source — `rows` by `columns`, row-major in the
/// source's order — where it belongs in a whole-grid buffer.
///
/// # Panics
/// Panics if the block is not `rows.len() * columns.len()` values.
pub fn place<T: Copy>(
    src: CellGrid,
    buffer: &mut [T],
    rows: Range<usize>,
    columns: Range<usize>,
    block: &[T],
) {
    let width = columns.len();
    assert_eq!(
        block.len(),
        rows.len() * width,
        "block does not match its window"
    );
    for (k, row) in rows.enumerate() {
        let at = row * src.nlon + columns.start;
        buffer[at..at + width].copy_from_slice(&block[k * width..(k + 1) * width]);
    }
}

/// The two source rows bracketing `lat`, and the fraction of the way from the
/// first to the second. Latitudes beyond the outermost row centres -- the
/// poles themselves -- clamp to the nearest row.
fn row_neighbours(src: CellGrid, lat: f64) -> (usize, usize, f64) {
    let f = (lat - src.lat0) / src.dlat;
    let last = (src.nlat - 1) as f64;
    if f <= 0.0 {
        return (0, 0, 0.0);
    }
    if f >= last {
        return (src.nlat - 1, src.nlat - 1, 0.0);
    }
    let j0 = f.floor() as usize;
    (j0, j0 + 1, f - f.floor())
}

/// The two source columns bracketing `lon`, wrapping around the antimeridian.
fn column_neighbours(src: CellGrid, lon: f64) -> (usize, usize, f64) {
    let f = ((lon - src.lon0).rem_euclid(360.0)) / src.dlon;
    let i0 = (f.floor() as usize) % src.nlon;
    let i1 = (i0 + 1) % src.nlon;
    (i0, i1, f - f.floor())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::NI;

    const G: CellGrid = CellGrid::GLOBCURRENT;

    fn field(f: impl Fn(f64, f64) -> f32) -> Vec<f32> {
        let mut v = Vec::with_capacity(G.len());
        for j in 0..G.nlat {
            let lat = G.lat0 + j as f64 * G.dlat;
            for i in 0..G.nlon {
                let lon = G.lon0 + i as f64 * G.dlon;
                v.push(f(lat, lon));
            }
        }
        v
    }

    fn at(out: &[f32], lat: f64, lon: f64) -> f32 {
        let j = ((90.0 - lat) / 0.25).round() as usize;
        let i = (lon.rem_euclid(360.0) / 0.25).round() as usize;
        out[j * NI as usize + i]
    }

    #[test]
    fn the_output_is_the_era5_grid() {
        let out = to_era5_grid(G, &field(|_, _| 1.0));
        assert_eq!(out.len(), 1440 * 721);
    }

    #[test]
    fn a_constant_field_stays_constant() {
        let out = to_era5_grid(G, &field(|_, _| 0.75));
        assert!(out.iter().all(|&v| (v - 0.75).abs() < 1e-6));
    }

    /// Bilinear interpolation reproduces a plane exactly, so a field linear in
    /// latitude must come out linear in latitude at the new nodes.
    #[test]
    fn a_linear_field_is_reproduced_at_the_new_nodes() {
        let out = to_era5_grid(G, &field(|lat, _| lat as f32));
        for lat in [45.0, 0.0, -30.25, 89.75, -89.75] {
            let v = at(&out, lat, 10.0);
            assert!((f64::from(v) - lat).abs() < 1e-3, "lat {lat}: got {v}");
        }
    }

    /// Every ERA5 node sits exactly between four GlobCurrent cells, so the
    /// weights are all a quarter and the value is their plain mean.
    #[test]
    fn a_node_is_the_mean_of_its_four_neighbours() {
        let mut v = field(|_, _| 0.0);
        // The four cells around (lat 0, lon 0): rows for -0.125 and 0.125,
        // columns for -0.125 and 0.125.
        let j0 = ((-0.125 - G.lat0) / G.dlat).round() as usize;
        let i0 = ((-0.125 - G.lon0) / G.dlon).round() as usize;
        v[j0 * G.nlon + i0] = 1.0;
        v[j0 * G.nlon + i0 + 1] = 2.0;
        v[(j0 + 1) * G.nlon + i0] = 3.0;
        v[(j0 + 1) * G.nlon + i0 + 1] = 4.0;
        let out = to_era5_grid(G, &v);
        assert!((at(&out, 0.0, 0.0) - 2.5).abs() < 1e-6);
    }

    /// A coastline must not grow a ring of missing values: a node with three
    /// present neighbours takes their mean, and only a node with none stays
    /// missing.
    #[test]
    fn missing_neighbours_are_left_out_rather_than_poisoning_the_node() {
        let mut v = field(|_, _| 1.0);
        let j0 = ((-0.125 - G.lat0) / G.dlat).round() as usize;
        let i0 = ((-0.125 - G.lon0) / G.dlon).round() as usize;
        v[j0 * G.nlon + i0] = f32::NAN;
        v[j0 * G.nlon + i0 + 1] = 4.0;
        v[(j0 + 1) * G.nlon + i0] = 4.0;
        v[(j0 + 1) * G.nlon + i0 + 1] = 4.0;
        let out = to_era5_grid(G, &v);
        assert!(
            (at(&out, 0.0, 0.0) - 4.0).abs() < 1e-6,
            "mean of the three present"
        );

        let all_nan = field(|lat, lon| {
            if lat.abs() < 1.0 && lon.abs() < 1.0 {
                f32::NAN
            } else {
                1.0
            }
        });
        let out = to_era5_grid(G, &all_nan);
        assert!(
            at(&out, 0.0, 0.0).is_nan(),
            "no present neighbour stays missing"
        );
        assert!(at(&out, 45.0, 45.0).is_finite());
    }

    /// The grids meet at the antimeridian: ERA5's 180 east is between
    /// GlobCurrent's 179.875 and -179.875.
    #[test]
    fn the_antimeridian_wraps() {
        let v = field(|_, lon| {
            if lon > 179.0 {
                10.0
            } else if lon < -179.0 {
                20.0
            } else {
                0.0
            }
        });
        let out = to_era5_grid(G, &v);
        assert!(
            (at(&out, 0.0, 180.0) - 15.0).abs() < 1e-6,
            "mean across the seam"
        );
        // And the prime meridian is between -0.125 and 0.125, not a seam.
        let v = field(|_, lon| lon as f32);
        let out = to_era5_grid(G, &v);
        assert!(at(&out, 0.0, 0.0).abs() < 1e-3);
        assert!(
            (at(&out, 0.0, 359.75) - -0.25).abs() < 1e-3,
            "359.75 is -0.25"
        );
    }

    /// The cells [`native_window`] names are all a window's nodes read: a
    /// field with every other cell NaN regrids, on the window, exactly as the
    /// whole field does cropped. Across the antimeridian (the source's own
    /// seam), across the prime meridian, at a pole, and on a finer grid whose
    /// latitudes run north to south.
    #[test]
    fn a_windowed_regrid_reads_only_its_native_window() {
        let windows = [
            Window {
                i0: 640,
                ni: 161,
                j0: 320,
                nj: 81,
            },
            Window {
                i0: 1360,
                ni: 161,
                j0: 120,
                nj: 81,
            },
            Window {
                i0: 720,
                ni: 1440,
                j0: 0,
                nj: 81,
            },
            Window {
                i0: 720,
                ni: 1440,
                j0: 600,
                nj: 121,
            },
            Window {
                i0: 3,
                ni: 1,
                j0: 719,
                nj: 2,
            },
        ];
        let eighth = CellGrid {
            lat0: 89.9375,
            dlat: -0.125,
            nlat: 1440,
            lon0: 0.0625,
            dlon: 0.125,
            nlon: 2880,
        };
        for src in [G, eighth] {
            let full: Vec<f32> = (0..src.len())
                .map(|k| ((k * 7919) % 1000) as f32 / 10.0)
                .collect();
            let whole = to_era5_grid(src, &full);
            for w in windows {
                let native = native_window(src, &w);
                let mut sparse = vec![f32::NAN; src.len()];
                for cols in &native.columns {
                    for r in native.rows.clone() {
                        for c in cols.clone() {
                            sparse[r * src.nlon + c] = full[r * src.nlon + c];
                        }
                    }
                }
                let windowed = to_era5_window(src, &sparse, &w);
                let ni = NI as usize;
                let whole = &whole;
                let cropped: Vec<f32> = w
                    .rows()
                    .flat_map(|j| w.columns().map(move |i| whole[j * ni + i]))
                    .collect();
                assert_eq!(windowed, cropped, "{w:?} on {src:?}");
                assert!(!native.is_whole(src), "{w:?} read the whole grid");
            }
        }
    }

    /// Across the source's own first column the cells are two blocks, the
    /// east part first; elsewhere one.
    #[test]
    fn a_native_window_splits_only_at_the_sources_seam() {
        // GlobCurrent runs from -179.875: 160 E to 160 W crosses its seam.
        let across = native_window(
            G,
            &Window {
                i0: 640,
                ni: 161,
                j0: 320,
                nj: 81,
            },
        );
        assert_eq!(across.columns.len(), 2);
        assert_eq!(across.columns[0].end, G.nlon);
        assert_eq!(across.columns[1].start, 0);
        // 20 W to 20 E is in the middle of its row.
        let middle = native_window(
            G,
            &Window {
                i0: 1360,
                ni: 161,
                j0: 120,
                nj: 81,
            },
        );
        assert_eq!(middle.columns.len(), 1);
        // The global window is every cell, in one piece.
        assert!(native_window(G, &Window::global()).is_whole(G));
    }

    /// The poles lie past the outermost cell centres; they take the nearest
    /// row rather than extrapolating.
    #[test]
    fn the_poles_clamp_to_the_nearest_row() {
        let v = field(|lat, _| lat as f32);
        let out = to_era5_grid(G, &v);
        assert!((at(&out, 90.0, 0.0) - 89.875).abs() < 1e-3);
        assert!((at(&out, -90.0, 0.0) - -89.875).abs() < 1e-3);
    }

    /// The first output row is the north pole and the first column is the
    /// prime meridian: GRIB scanning order, whatever order the source used.
    #[test]
    fn the_output_is_in_grib_scanning_order() {
        let v = field(|lat, lon| (lat * 1000.0 + lon) as f32);
        let out = to_era5_grid(G, &v);
        assert!(
            out[0] > out[NI as usize],
            "row 0 is further north than row 1"
        );
        assert!(out[1] > out[0], "column 1 is further east than column 0");
    }
}
