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

use crate::source::{NI, NJ};

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
    assert_eq!(values.len(), src.len(), "field does not match its grid");
    assert!(
        ((src.nlon as f64) * src.dlon - 360.0).abs() < 1e-6,
        "source grid must span the full circle"
    );

    let mut out = Vec::with_capacity((NI * NJ) as usize);
    for j in 0..NJ {
        let lat = 90.0 - (j as f64) * 0.25;
        let (j0, j1, t) = row_neighbours(src, lat);
        for i in 0..NI {
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
