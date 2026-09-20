//! Web Mercator tiles onto the application's lat/lon grid.
//!
//! OpenStreetMap serves `z/x/y` in Web Mercator; the application's own tiles
//! are a lat/lon lattice (`ve_render::tile`). Resampling here rather than in
//! the webview is what lets the map draw them as ordinary textures in every
//! projection it has, through the paths that already exist.

use crate::{OsmError, Result};

/// The latitude Web Mercator stops at, where the projection's square closes.
pub const MERCATOR_LIMIT: f64 = 85.051_128_779_806_59;

/// One source tile's address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceTile {
    /// Zoom, 0 to 19.
    pub z: u32,
    /// Column, west to east.
    pub x: u32,
    /// Row, north to south.
    pub y: u32,
}

/// Web Mercator's y for a latitude, 0 at the north edge and 1 at the south.
fn mercator_y(lat: f64) -> f64 {
    let clamped = lat.clamp(-MERCATOR_LIMIT, MERCATOR_LIMIT).to_radians();
    (1.0 - (clamped.tan() + 1.0 / clamped.cos()).ln() / std::f64::consts::PI) / 2.0
}

/// The source zoom whose tiles are about as fine as the tile being drawn.
///
/// An application tile of level `z` spans `360 / (2 << z)` degrees over its
/// own pixels, and a Mercator tile of level `Z` spans `360 / 2^Z` over the
/// same count, so the two match at `Z = z + 1`. Held to what the tile server
/// publishes.
pub fn source_zoom_for(app_level: u32) -> u32 {
    (app_level + 1).min(crate::MAX_SOURCE_ZOOM)
}

/// Every source tile a lat/lon box touches, at that zoom.
///
/// The box's longitudes may run past the antimeridian, which is one turn of
/// the source grid: a column is taken modulo the grid's width.
pub fn source_tiles(west: f64, south: f64, east: f64, north: f64, zoom: u32) -> Vec<SourceTile> {
    let side = f64::from(1_u32 << zoom);
    let column = |lon: f64| ((lon + 180.0) / 360.0 * side).floor();
    let row = |lat: f64| (mercator_y(lat) * side).floor().clamp(0.0, side - 1.0);
    let (first_col, last_col) = (column(west), column(east));
    let (first_row, last_row) = (row(north), row(south));
    let mut out = Vec::new();
    let mut y = first_row;
    while y <= last_row {
        let mut x = first_col;
        while x <= last_col {
            // A view wider than the world would ask for the same column
            // twice; the tile is the same tile.
            let wrapped = x.rem_euclid(side);
            let tile = SourceTile {
                z: zoom,
                x: wrapped as u32,
                y: y as u32,
            };
            if !out.contains(&tile) {
                out.push(tile);
            }
            x += 1.0;
            if out.len() > 256 {
                return out;
            }
        }
        y += 1.0;
    }
    out
}

/// One decoded source tile: straight RGBA, `size` square.
#[derive(Debug, Clone)]
pub struct Decoded {
    /// Which tile it is.
    pub tile: SourceTile,
    /// Its edge in pixels.
    pub size: u32,
    /// Its pixels, straight RGBA.
    pub rgba: Vec<u8>,
}

/// Decodes a PNG tile.
pub fn decode(tile: SourceTile, bytes: &[u8]) -> Result<Decoded> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // The tile servers serve 8-bit palette PNGs, and a sixteen-bit one is
    // allowed by the format. Both are normalised to eight-bit colour here,
    // so what comes out is grey, RGB or RGBA and nothing else.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|err| OsmError::Decode(err.to_string()))?;
    let mut buffer = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|err| OsmError::Decode(err.to_string()))?;
    if info.width != info.height {
        return Err(OsmError::Decode(format!(
            "a {}x{} tile is not square",
            info.width, info.height
        )));
    }
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        // Normalised away above; a palette that survived it is one this
        // cannot read rather than one to guess at.
        png::ColorType::Indexed => {
            return Err(OsmError::Decode(
                "a palette this decoder cannot expand".to_owned(),
            ));
        }
    };
    if info.bit_depth != png::BitDepth::Eight {
        return Err(OsmError::Decode(format!(
            "{:?} bits per sample",
            info.bit_depth
        )));
    }
    let pixels = (info.width * info.height) as usize;
    let mut rgba = vec![255_u8; pixels * 4];
    for (index, chunk) in buffer.chunks_exact(channels).take(pixels).enumerate() {
        let out = &mut rgba[index * 4..index * 4 + 4];
        match channels {
            4 => out.copy_from_slice(chunk),
            3 => out[..3].copy_from_slice(chunk),
            2 => {
                out[..3].fill(chunk[0]);
                out[3] = chunk[1];
            }
            _ => out[..3].fill(chunk[0]),
        }
    }
    Ok(Decoded {
        tile,
        size: info.width,
        rgba,
    })
}

/// Resamples source tiles into one lat/lon tile.
///
/// Nearest neighbour: a source tile is chosen to be about as fine as the
/// tile being drawn (`source_zoom_for`), so this is at worst a half-pixel
/// shift, and a map label blurred by a bilinear blend is harder to read than
/// one shifted by half a pixel.
pub fn warp(
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    size: u32,
    sources: &[Decoded],
) -> Option<Vec<u8>> {
    if sources.is_empty() {
        return None;
    }
    let mut out = vec![0_u8; (size as usize) * (size as usize) * 4];
    let mut painted = false;
    for py in 0..size {
        // The pixel's own centre, which is what its colour stands for.
        let lat = north - (f64::from(py) + 0.5) / f64::from(size) * (north - south);
        let y = mercator_y(lat);
        for px in 0..size {
            let lon = west + (f64::from(px) + 0.5) / f64::from(size) * (east - west);
            let Some(source) = sources.iter().find_map(|decoded| {
                let side = f64::from(1_u32 << decoded.tile.z);
                // The world copy this pixel falls in: a map showing two
                // worlds reads the same tiles twice.
                let column = (lon + 180.0) / 360.0 * side;
                let column = column - (column / side).floor() * side;
                let sx = (column - f64::from(decoded.tile.x)) * f64::from(decoded.size);
                let sy = (y * side - f64::from(decoded.tile.y)) * f64::from(decoded.size);
                let (sx, sy) = (sx.floor(), sy.floor());
                if sx < 0.0
                    || sy < 0.0
                    || sx >= f64::from(decoded.size)
                    || sy >= f64::from(decoded.size)
                {
                    return None;
                }
                let at = ((sy as usize) * decoded.size as usize + sx as usize) * 4;
                decoded.rgba.get(at..at + 4)
            }) else {
                continue;
            };
            let at = ((py as usize) * size as usize + px as usize) * 4;
            out[at..at + 4].copy_from_slice(source);
            painted = true;
        }
    }
    painted.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Web Mercator's square: the equator halfway down, and the latitude the
    /// projection stops at exactly at its edge.
    #[test]
    fn the_projection_is_the_one_the_tiles_are_cut_on() {
        assert!((mercator_y(0.0) - 0.5).abs() < 1e-12);
        assert!(mercator_y(MERCATOR_LIMIT).abs() < 1e-9);
        assert!((mercator_y(-MERCATOR_LIMIT) - 1.0).abs() < 1e-9);
        // Past the limit is held to it rather than run to infinity.
        assert!((mercator_y(89.9) - mercator_y(MERCATOR_LIMIT)).abs() < 1e-12);
        // A known value: the 45th parallel, which is not halfway up.
        assert!((mercator_y(45.0) - 0.359_725_1).abs() < 1e-6);
    }

    #[test]
    fn a_source_tile_is_about_as_fine_as_the_tile_it_fills() {
        // An application tile of level z spans 360/(2<<z) degrees; a source
        // tile of level z+1 spans 360/2^(z+1), which is the same.
        for level in 0..12 {
            let app = 360.0 / f64::from(2_u32 << level);
            let source = 360.0 / f64::from(1_u32 << source_zoom_for(level));
            assert!((app - source).abs() < 1e-9, "level {level}");
        }
        assert_eq!(
            source_zoom_for(30),
            crate::MAX_SOURCE_ZOOM,
            "held to what is served"
        );
    }

    #[test]
    fn a_box_asks_for_the_tiles_it_touches() {
        // The whole world at zoom 1 is four tiles.
        let all = source_tiles(-180.0, -85.0, 180.0, 85.0, 1);
        assert_eq!(all.len(), 4);
        // One tile's own box asks for that tile.
        let one = source_tiles(-180.0, 0.1, -179.9, 0.2, 1);
        assert_eq!(one, vec![SourceTile { z: 1, x: 0, y: 0 }]);
        // Across the antimeridian, stated past 180: the far column wraps to
        // the near one rather than falling off the grid.
        let across = source_tiles(179.0, 0.0, 181.0, 1.0, 2);
        assert!(across.iter().all(|tile| tile.x < 4), "{across:?}");
        assert!(across.iter().any(|tile| tile.x == 0), "wraps: {across:?}");
    }

    fn solid(tile: SourceTile, colour: [u8; 4]) -> Decoded {
        Decoded {
            tile,
            size: 4,
            rgba: colour.repeat(16),
        }
    }

    /// The tile servers serve palette PNGs, which is what this failed on
    /// first: a fetched tile decoded as nothing and the map stayed empty.
    #[test]
    fn a_palette_tile_decodes_as_colour() {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 2);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_palette(vec![170, 200, 230, 40, 80, 40]);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(&[0, 1, 1, 0]).expect("image data");
        }
        let decoded = decode(SourceTile { z: 0, x: 0, y: 0 }, &png).expect("decodes");
        assert_eq!(decoded.size, 2);
        assert_eq!(&decoded.rgba[..4], &[170, 200, 230, 255]);
        assert_eq!(&decoded.rgba[4..8], &[40, 80, 40, 255]);
    }

    #[test]
    fn a_tile_takes_its_pixels_from_the_source_under_each_one() {
        // Zoom 1: the north-west quarter of the world is tile (0,0).
        let sources = vec![
            solid(SourceTile { z: 1, x: 0, y: 0 }, [10, 20, 30, 255]),
            solid(SourceTile { z: 1, x: 1, y: 0 }, [200, 100, 50, 255]),
        ];
        let out = warp(-180.0, 10.0, 0.0, 20.0, 2, &sources).expect("painted");
        assert_eq!(&out[..4], &[10, 20, 30, 255], "west of the meridian");
        let east = warp(0.0, 10.0, 180.0, 20.0, 2, &sources).expect("painted");
        assert_eq!(&east[..4], &[200, 100, 50, 255], "east of it");
    }

    #[test]
    fn a_tile_with_no_source_under_it_is_not_drawn() {
        let sources = vec![solid(SourceTile { z: 1, x: 0, y: 0 }, [1, 2, 3, 255])];
        // The southern hemisphere, which tile (0,0) does not cover.
        assert!(warp(-180.0, -80.0, -90.0, -70.0, 2, &sources).is_none());
        assert!(warp(-180.0, 10.0, -90.0, 20.0, 2, &[]).is_none());
    }
}
