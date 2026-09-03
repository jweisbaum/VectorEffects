//! The preview tile pyramid.
//!
//! Tiles are addressed in an equirectangular pyramid: level 0 is two tiles side
//! by side covering the globe, and each level doubles in both directions. The
//! grid is global lat/lon, so an equirectangular pyramid keeps tiles aligned
//! with the data rather than resampling through a second projection.
//!
//! Tiles carry *speed and direction*, never `u`/`v` (spec.md 5.3). The webview
//! never sees components, so it cannot accidentally show them.

use ve_core::LonLat;
use ve_core::vector::{Uv, speed_azimuth_from_uv};

use crate::error::{RenderError, Result};

/// Tile edge length in pixels.
pub const TILE_SIZE: u32 = 256;

/// Deepest level served. Level 12 is about 0.04° per pixel, finer than the
/// finest grid, so there is nothing to gain past it.
pub const MAX_LEVEL: u32 = 12;

/// Full-scale speed for the encoding, in m/s.
///
/// Speed is stored as a `u16` fraction of this, giving about 0.0015 m/s of
/// resolution -- far finer than the 16-bit GRIB packing the export uses, so the
/// preview is never the limiting factor.
pub const SPEED_SCALE_MPS: f32 = 100.0;

/// Bytes in one encoded tile: RGBA8 at [`TILE_SIZE`] squared.
pub const TILE_BYTES: usize = (TILE_SIZE as usize) * (TILE_SIZE as usize) * 4;

/// A tile address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId {
    /// Zoom level; 0 is the whole globe in two tiles.
    pub z: u32,
    /// Column, west to east.
    pub x: u32,
    /// Row, north to south.
    pub y: u32,
}

/// The geographic extent of a tile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileBounds {
    /// Western edge, degrees.
    pub west: f64,
    /// Eastern edge, degrees.
    pub east: f64,
    /// Northern edge, degrees.
    pub north: f64,
    /// Southern edge, degrees.
    pub south: f64,
}

impl TileId {
    /// Creates an address, rejecting levels and indices outside the pyramid.
    pub fn new(z: u32, x: u32, y: u32) -> Result<Self> {
        let id = Self { z, x, y };
        if z > MAX_LEVEL || x >= Self::columns(z) || y >= Self::rows(z) {
            return Err(RenderError::BadTile { z, x, y });
        }
        Ok(id)
    }

    /// Columns at `z`. Twice the rows, because the globe is twice as wide as
    /// it is tall in equirectangular.
    pub fn columns(z: u32) -> u32 {
        2u32 << z.min(MAX_LEVEL)
    }

    /// Rows at `z`.
    pub fn rows(z: u32) -> u32 {
        1u32 << z.min(MAX_LEVEL)
    }

    /// The tile's geographic extent.
    pub fn bounds(self) -> TileBounds {
        let span_x = 360.0 / f64::from(Self::columns(self.z));
        let span_y = 180.0 / f64::from(Self::rows(self.z));
        let west = -180.0 + f64::from(self.x) * span_x;
        let north = 90.0 - f64::from(self.y) * span_y;
        TileBounds {
            west,
            east: west + span_x,
            north,
            south: north - span_y,
        }
    }

    /// Degrees per pixel along each axis.
    pub fn degrees_per_pixel(self) -> (f64, f64) {
        let b = self.bounds();
        (
            (b.east - b.west) / f64::from(TILE_SIZE),
            (b.north - b.south) / f64::from(TILE_SIZE),
        )
    }

    /// The geographic position of a pixel centre.
    ///
    /// Latitude is clamped to the poles: the top row of the top tile sits half
    /// a pixel above 90°, which is not a real place.
    pub fn pixel_position(self, px: u32, py: u32) -> LonLat {
        let b = self.bounds();
        let (dx, dy) = self.degrees_per_pixel();
        let lon = b.west + (f64::from(px) + 0.5) * dx;
        let lat = (b.north - (f64::from(py) + 0.5) * dy).clamp(-90.0, 90.0);
        LonLat {
            lon: ve_core::geo::normalize_lon(lon),
            lat,
        }
    }

    /// Every pixel-centre position, row-major from the north-west corner.
    pub fn sample_positions(self) -> Vec<LonLat> {
        let mut out = Vec::with_capacity((TILE_SIZE * TILE_SIZE) as usize);
        for py in 0..TILE_SIZE {
            for px in 0..TILE_SIZE {
                out.push(self.pixel_position(px, py));
            }
        }
        out
    }
}

/// Encodes samples as RGBA8: speed in R+G, azimuth-toward in B+A.
///
/// Both are 16-bit little-endian pairs. Packing direction as an angle rather
/// than as components means the glyph shader can read a bearing directly, and
/// means `u`/`v` never cross into the webview at all.
pub fn encode(samples: &[Uv]) -> Vec<u8> {
    let mut out = vec![0u8; TILE_BYTES];
    for (i, uv) in samples.iter().take(TILE_BYTES / 4).enumerate() {
        let (speed, azimuth) = speed_azimuth_from_uv(*uv);
        let speed_frac = (speed / f64::from(SPEED_SCALE_MPS)).clamp(0.0, 1.0);
        let speed_u16 = (speed_frac * f64::from(u16::MAX)).round() as u16;
        // 360 maps back onto 0, so the wrap is exact rather than one step short.
        let az_u16 =
            ((azimuth.degrees() / 360.0).rem_euclid(1.0) * f64::from(u16::MAX)).round() as u16;

        let o = i * 4;
        out[o] = (speed_u16 & 0xff) as u8;
        out[o + 1] = (speed_u16 >> 8) as u8;
        out[o + 2] = (az_u16 & 0xff) as u8;
        out[o + 3] = (az_u16 >> 8) as u8;
    }
    out
}

/// Decodes one pixel back to `(speed m/s, azimuth-toward degrees)`.
///
/// The frontend does this in a shader; this exists so tests can assert the
/// encoding round-trips rather than trusting it.
pub fn decode_pixel(tile: &[u8], index: usize) -> Option<(f64, f64)> {
    let o = index * 4;
    let bytes = tile.get(o..o + 4)?;
    let speed_u16 = u16::from(bytes[0]) | (u16::from(bytes[1]) << 8);
    let az_u16 = u16::from(bytes[2]) | (u16::from(bytes[3]) << 8);
    Some((
        f64::from(speed_u16) / f64::from(u16::MAX) * f64::from(SPEED_SCALE_MPS),
        f64::from(az_u16) / f64::from(u16::MAX) * 360.0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::angle::Angle;
    use ve_core::vector::uv_from_speed_azimuth;

    #[test]
    fn level_zero_is_two_tiles_covering_the_globe() {
        assert_eq!(TileId::columns(0), 2);
        assert_eq!(TileId::rows(0), 1);

        let west = TileId::new(0, 0, 0).unwrap().bounds();
        assert_eq!(
            (west.west, west.east, west.north, west.south),
            (-180.0, 0.0, 90.0, -90.0)
        );
        let east = TileId::new(0, 1, 0).unwrap().bounds();
        assert_eq!((east.west, east.east), (0.0, 180.0));
    }

    #[test]
    fn each_level_doubles() {
        for z in 0..=4 {
            assert_eq!(TileId::columns(z), 2 * TileId::rows(z));
            assert_eq!(TileId::rows(z), 1 << z);
        }
    }

    /// The tiles at any level must tile the globe exactly: no gaps, no overlap.
    #[test]
    fn tiles_cover_the_globe_without_gaps() {
        for z in 0..=3 {
            let (cols, rows) = (TileId::columns(z), TileId::rows(z));
            for y in 0..rows {
                let mut cursor = -180.0;
                for x in 0..cols {
                    let b = TileId::new(z, x, y).unwrap().bounds();
                    assert!((b.west - cursor).abs() < 1e-9, "gap at z{z} x{x}");
                    cursor = b.east;
                }
                assert!(
                    (cursor - 180.0).abs() < 1e-9,
                    "row {y} at z{z} does not reach 180"
                );
            }
            let top = TileId::new(z, 0, 0).unwrap().bounds();
            let bottom = TileId::new(z, 0, rows - 1).unwrap().bounds();
            assert!((top.north - 90.0).abs() < 1e-9);
            assert!((bottom.south + 90.0).abs() < 1e-9);
        }
    }

    #[test]
    fn out_of_range_addresses_are_rejected() {
        assert!(TileId::new(0, 2, 0).is_err(), "only two columns at level 0");
        assert!(TileId::new(0, 0, 1).is_err(), "only one row at level 0");
        assert!(TileId::new(MAX_LEVEL + 1, 0, 0).is_err());
        assert!(TileId::new(3, 15, 7).is_ok());
    }

    #[test]
    fn pixel_positions_stay_inside_the_tile() {
        let id = TileId::new(2, 3, 1).unwrap();
        let b = id.bounds();
        for (px, py) in [(0, 0), (255, 255), (128, 128)] {
            let p = id.pixel_position(px, py);
            assert!(p.lon >= b.west && p.lon <= b.east, "{p:?} outside {b:?}");
            assert!(p.lat <= b.north && p.lat >= b.south, "{p:?} outside {b:?}");
        }
        assert_eq!(
            id.sample_positions().len(),
            (TILE_SIZE * TILE_SIZE) as usize
        );
    }

    /// The top and bottom rows must not sample past the poles.
    #[test]
    fn latitudes_are_clamped_at_the_poles() {
        let top = TileId::new(0, 0, 0).unwrap();
        for py in [0u32, 255] {
            let p = top.pixel_position(0, py);
            assert!((-90.0..=90.0).contains(&p.lat), "{p:?}");
        }
    }

    #[test]
    fn encoding_round_trips_within_quantisation() {
        let cases = [
            (0.0, 0.0),
            (12.5, 90.0),
            (33.3, 187.5),
            (99.0, 359.9),
            (0.0, 180.0),
        ];
        let samples: Vec<Uv> = cases
            .iter()
            .map(|(s, a)| uv_from_speed_azimuth(*s, Angle::new(*a)))
            .collect();

        let tile = encode(&samples);
        assert_eq!(tile.len(), TILE_BYTES);

        for (i, (speed, azimuth)) in cases.iter().enumerate() {
            let (got_speed, got_az) = decode_pixel(&tile, i).expect("in range");
            assert!(
                (got_speed - speed).abs() < 0.01,
                "speed {got_speed} != {speed}"
            );
            if *speed > 0.0 {
                let delta = (got_az - azimuth)
                    .abs()
                    .min(360.0 - (got_az - azimuth).abs());
                assert!(delta < 0.02, "azimuth {got_az} != {azimuth}");
            }
        }
    }

    #[test]
    fn speeds_beyond_full_scale_clamp_rather_than_wrap() {
        let samples = vec![uv_from_speed_azimuth(500.0, Angle::new(45.0))];
        let tile = encode(&samples);
        let (speed, _) = decode_pixel(&tile, 0).expect("in range");
        assert!(
            (speed - f64::from(SPEED_SCALE_MPS)).abs() < 0.01,
            "expected clamp to full scale, got {speed}"
        );
    }

    #[test]
    fn unwritten_pixels_are_calm() {
        let tile = encode(&[]);
        assert_eq!(tile.len(), TILE_BYTES);
        assert_eq!(decode_pixel(&tile, 1000), Some((0.0, 0.0)));
    }
}
