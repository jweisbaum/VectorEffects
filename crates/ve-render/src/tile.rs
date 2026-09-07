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
use ve_core::project::FieldKind;
use ve_core::vector::speed_azimuth_from_uv;

use crate::evaluator::Sample;

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

/// Bits of a texel's 32-bit word given to the speed (M31).
pub const SPEED_BITS: u32 = 14;
/// Bits given to the azimuth.
pub const AZIMUTH_BITS: u32 = 12;
/// Bits given to the coverage.
pub const COVERAGE_BITS: u32 = 5;
const SPEED_MAX: u32 = (1 << SPEED_BITS) - 1;
const AZIMUTH_STEPS: u32 = 1 << AZIMUTH_BITS;
const COVERAGE_MAX: u32 = (1 << COVERAGE_BITS) - 1;

/// Encodes samples as RGBA8, one little-endian 32-bit word per texel
/// (spec.md 7.7, M31): the speed in the low 14 bits as a fraction of full
/// scale, the azimuth-toward in the next 12 as a fraction of a turn, the
/// coverage in the next 5, and the kind in the top bit — 1 for wind.
///
/// Packing direction as an angle rather than as components means the glyph
/// shader can read a bearing directly, and means `u`/`v` never cross into
/// the webview at all. The coverage is what makes an unwritten cell
/// transparent and a written calm one opaque (D58): zero and undefined are
/// different things, and the tile carries the difference.
pub fn encode(samples: &[Sample]) -> Vec<u8> {
    let mut out = vec![0u8; TILE_BYTES];
    for (i, sample) in samples.iter().take(TILE_BYTES / 4).enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&encode_texel(sample).to_le_bytes());
    }
    out
}

/// One texel's word.
pub fn encode_texel(sample: &Sample) -> u32 {
    let (speed, azimuth) = speed_azimuth_from_uv(sample.uv);
    let speed_frac = (speed / f64::from(SPEED_SCALE_MPS)).clamp(0.0, 1.0);
    let speed_bits = (speed_frac * f64::from(SPEED_MAX)).round() as u32;
    // 360 maps back onto 0, so the wrap is exact rather than one step short.
    let az_bits = ((azimuth.degrees() / 360.0).rem_euclid(1.0) * f64::from(AZIMUTH_STEPS)).round()
        as u32
        % AZIMUTH_STEPS;
    // A cell that is written at all is written: the faint outer edge of a
    // feathered stroke rounds up to the lowest step rather than to nothing.
    let coverage = f64::from(sample.coverage.clamp(0.0, 1.0));
    let coverage_bits = if coverage > 0.0 {
        ((coverage * f64::from(COVERAGE_MAX)).round() as u32).max(1)
    } else {
        0
    };
    let kind_bit = u32::from(sample.kind == FieldKind::Wind);
    speed_bits
        | (az_bits << SPEED_BITS)
        | (coverage_bits << (SPEED_BITS + AZIMUTH_BITS))
        | (kind_bit << (SPEED_BITS + AZIMUTH_BITS + COVERAGE_BITS))
}

/// One texel decoded: speed in m/s, azimuth-toward in degrees, coverage 0
/// to 1, and the kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Texel {
    pub speed_mps: f64,
    pub azimuth_deg: f64,
    pub coverage: f64,
    pub kind: FieldKind,
}

/// Decodes one pixel.
///
/// The frontend does this in a shader; this exists so tests can assert the
/// encoding round-trips rather than trusting it.
pub fn decode_pixel(tile: &[u8], index: usize) -> Option<Texel> {
    let o = index * 4;
    let bytes = tile.get(o..o + 4)?;
    let word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    Some(Texel {
        speed_mps: f64::from(word & SPEED_MAX) / f64::from(SPEED_MAX) * f64::from(SPEED_SCALE_MPS),
        azimuth_deg: f64::from((word >> SPEED_BITS) & (AZIMUTH_STEPS - 1))
            / f64::from(AZIMUTH_STEPS)
            * 360.0,
        coverage: f64::from((word >> (SPEED_BITS + AZIMUTH_BITS)) & COVERAGE_MAX)
            / f64::from(COVERAGE_MAX),
        kind: if word >> (SPEED_BITS + AZIMUTH_BITS + COVERAGE_BITS) == 1 {
            FieldKind::Wind
        } else {
            FieldKind::Current
        },
    })
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

    fn sample(speed: f64, azimuth: f64, coverage: f32, kind: FieldKind) -> Sample {
        Sample {
            uv: uv_from_speed_azimuth(speed, Angle::new(azimuth)),
            coverage,
            kind,
        }
    }

    /// Speed within the 14-bit step, azimuth within the 12-bit one — both
    /// far inside the preview tolerance of 0.25 m/s and 2 degrees (spec.md
    /// 7.9) — and the coverage and the kind exactly, for both kinds.
    #[test]
    fn encoding_round_trips_within_quantisation() {
        let cases = [
            (0.0, 0.0, 1.0, FieldKind::Wind),
            (12.5, 90.0, 1.0, FieldKind::Current),
            (33.3, 187.5, 0.5, FieldKind::Wind),
            (99.0, 359.9, 0.03, FieldKind::Current),
            (0.0, 180.0, 1.0, FieldKind::Current),
        ];
        let samples: Vec<Sample> = cases
            .iter()
            .map(|(s, a, c, k)| sample(*s, *a, *c, *k))
            .collect();

        let tile = encode(&samples);
        assert_eq!(tile.len(), TILE_BYTES);

        let speed_step = f64::from(SPEED_SCALE_MPS) / f64::from(SPEED_MAX);
        let azimuth_step = 360.0 / f64::from(AZIMUTH_STEPS);
        for (i, (speed, azimuth, coverage, kind)) in cases.iter().enumerate() {
            let got = decode_pixel(&tile, i).expect("in range");
            assert!(
                (got.speed_mps - speed).abs() <= speed_step,
                "speed {} != {speed}",
                got.speed_mps
            );
            if *speed > 0.0 {
                let delta = (got.azimuth_deg - azimuth)
                    .abs()
                    .min(360.0 - (got.azimuth_deg - azimuth).abs());
                assert!(
                    delta <= azimuth_step,
                    "azimuth {} != {azimuth}",
                    got.azimuth_deg
                );
            }
            assert!(
                (got.coverage - f64::from(*coverage)).abs() <= 1.0 / f64::from(COVERAGE_MAX),
                "coverage {} != {coverage}",
                got.coverage
            );
            assert_eq!(got.kind, *kind);
        }
    }

    #[test]
    fn speeds_beyond_full_scale_clamp_rather_than_wrap() {
        let samples = vec![sample(500.0, 45.0, 1.0, FieldKind::Wind)];
        let tile = encode(&samples);
        let got = decode_pixel(&tile, 0).expect("in range");
        assert!(
            (got.speed_mps - f64::from(SPEED_SCALE_MPS)).abs() < 0.01,
            "expected clamp to full scale, got {}",
            got.speed_mps
        );
    }

    /// Zero and undefined are different things (D58): a written calm is
    /// covered and an unwritten cell is not, and the faintest written edge
    /// still reads as written rather than rounding away.
    #[test]
    fn coverage_tells_a_written_calm_from_nothing() {
        let tile = encode(&[
            sample(0.0, 0.0, 1.0, FieldKind::Wind),
            Sample::default(),
            sample(3.0, 10.0, 0.001, FieldKind::Current),
        ]);
        assert_eq!(decode_pixel(&tile, 0).expect("in range").coverage, 1.0);
        assert_eq!(decode_pixel(&tile, 1).expect("in range").coverage, 0.0);
        assert!(decode_pixel(&tile, 2).expect("in range").coverage > 0.0);
    }

    #[test]
    fn unwritten_pixels_are_calm_and_uncovered() {
        let tile = encode(&[]);
        assert_eq!(tile.len(), TILE_BYTES);
        let got = decode_pixel(&tile, 1000).expect("in range");
        assert_eq!(
            (got.speed_mps, got.azimuth_deg, got.coverage),
            (0.0, 0.0, 0.0)
        );
    }
}
