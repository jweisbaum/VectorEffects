//! The bundled unstructured-grid definitions.
//!
//! A GRIB message on an unstructured grid (template 3.101) names its grid by
//! a UUID and says nothing about where any of its cells is. DWD ships the
//! positions separately, as `CLAT` and `CLON` messages on that same grid, and
//! without them an ICON file cannot be placed on the earth at all.
//!
//! Rather than make the user find two more files — or fetch them, which
//! invariant 5 forbids outright — the cell centres of the grids this app
//! knows are converted at build time by `tools/icon-grid-builder` and
//! committed to `assets/icon_grids.bin`, exactly as the basemap is. An ICON
//! import then needs nothing but the forecast file.
//!
//! **The asset is the grid's geometry, not a field.** It is time-invariant,
//! it is the same for every ICON file ever issued on that grid, and it is
//! what makes a list of numbers into positions on the earth — so it is the
//! kind of thing invariants 1 and 2 say a project may hold, not the kind they
//! forbid.
//!
//! Storage is a delta of each coordinate's own quantisation lattice, varint
//! encoded, then deflated. ICON lists its cells in a space-filling order, so
//! consecutive cells are neighbours and the deltas are tiny: the global
//! R03B07 mesh is 11.8 MB of raw coordinates and 2.4 MB here.

use std::io::Read;
use std::sync::Arc;

use ve_core::regrid::CellCentres;

use crate::error::{GribError, Result};

/// The compiled-in grid definitions.
pub const EMBEDDED: &[u8] = include_bytes!("../../../assets/icon_grids.bin");

/// File magic.
pub const MAGIC: &[u8; 8] = b"VEICONG1";

/// What the asset says about one grid, before its coordinates are unpacked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridEntry {
    /// The grid's UUID, as template 3.101 carries it.
    pub uuid: [u8; 16],
    /// Number of cells.
    pub cells: u32,
    /// A human name for logs and errors.
    pub name: String,
    /// Offset of the deflated payload within the asset.
    offset: usize,
    /// Length of the deflated payload.
    length: usize,
}

impl GridEntry {
    /// The UUID as the lower-case hex DWD and ecCodes print.
    pub fn uuid_hex(&self) -> String {
        self.uuid.iter().map(|b| format!("{b:02x}")).collect()
    }
}

fn malformed(what: impl Into<String>) -> GribError {
    GribError::Malformed(what.into())
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    bytes
        .get(at..at + 4)
        .and_then(|s| <[u8; 4]>::try_from(s).ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| malformed(format!("grid asset truncated at byte {at}")))
}

/// Reads the asset's table of contents.
pub fn entries(asset: &[u8]) -> Result<Vec<GridEntry>> {
    if asset.len() < 12 || &asset[..8] != MAGIC {
        return Err(malformed("grid asset has wrong magic"));
    }
    let count = u32_at(asset, 8)? as usize;
    let mut out = Vec::with_capacity(count);
    let mut at = 12;
    for _ in 0..count {
        let uuid: [u8; 16] = asset
            .get(at..at + 16)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| malformed("grid asset truncated in a UUID"))?;
        at += 16;
        let cells = u32_at(asset, at)?;
        at += 4;
        let name_len = u32_at(asset, at)? as usize;
        at += 4;
        let name = asset
            .get(at..at + name_len)
            .ok_or_else(|| malformed("grid asset truncated in a name"))
            .and_then(|s| {
                std::str::from_utf8(s)
                    .map(str::to_owned)
                    .map_err(|_| malformed("a grid name that is not UTF-8"))
            })?;
        at += name_len;
        let length = u32_at(asset, at)? as usize;
        at += 4;
        if at + length > asset.len() {
            return Err(malformed("grid asset truncated in a payload"));
        }
        out.push(GridEntry {
            uuid,
            cells,
            name,
            offset: at,
            length,
        });
        at += length;
    }
    Ok(out)
}

/// Unpacks one grid's cell centres.
pub fn centres(asset: &[u8], entry: &GridEntry) -> Result<CellCentres> {
    let payload = asset
        .get(entry.offset..entry.offset + entry.length)
        .ok_or_else(|| malformed("grid asset truncated in a payload"))?;
    let mut raw = Vec::new();
    flate2::read::DeflateDecoder::new(payload)
        .read_to_end(&mut raw)
        .map_err(|e| malformed(format!("grid payload does not inflate: {e}")))?;

    let mut at = 0usize;
    let lat_min = read_f64(&raw, &mut at)?;
    let lat_step = read_f64(&raw, &mut at)?;
    let lon_min = read_f64(&raw, &mut at)?;
    let lon_step = read_f64(&raw, &mut at)?;

    // The latitudes run first, then the longitudes; the split is recorded
    // because varints are not fixed width.
    let lat_bytes = u32_at(&raw, at)? as usize;
    at += 4;
    let lat_end = at
        .checked_add(lat_bytes)
        .filter(|end| *end <= raw.len())
        .ok_or_else(|| malformed("grid payload truncated at its split"))?;
    let mut lon_at = lat_end;

    let cells = entry.cells as usize;
    let mut lat = Vec::with_capacity(cells);
    let mut lon = Vec::with_capacity(cells);
    let (mut a, mut o) = (0i64, 0i64);
    for _ in 0..cells {
        a += take_varint(&raw, &mut at)?;
        o += take_varint(&raw, &mut lon_at)?;
        lat.push((lat_min + a as f64 * lat_step) as f32);
        lon.push((lon_min + o as f64 * lon_step) as f32);
    }
    // Both runs must land exactly on their ends: a payload that decodes into
    // the right number of cells but leaves bytes over is not this grid.
    if at != lat_end {
        return Err(malformed(format!(
            "{} bytes left in the latitudes after {cells} cells",
            lat_end.saturating_sub(at)
        )));
    }
    if lon_at != raw.len() {
        return Err(malformed(format!(
            "{} bytes left in the longitudes after {cells} cells",
            raw.len() - lon_at
        )));
    }
    CellCentres::new(lat, lon).map_err(malformed)
}

/// The bundled grid matching a UUID, with its cell centres unpacked.
///
/// `None` when nothing bundled matches: the caller reports which grid the
/// file wanted rather than guessing at another.
pub fn bundled(uuid: &[u8; 16]) -> Result<Option<(GridEntry, Arc<CellCentres>)>> {
    let table = entries(EMBEDDED)?;
    let Some(entry) = table.into_iter().find(|e| &e.uuid == uuid) else {
        return Ok(None);
    };
    let centres = centres(EMBEDDED, &entry)?;
    Ok(Some((entry, Arc::new(centres))))
}

fn read_f64(bytes: &[u8], at: &mut usize) -> Result<f64> {
    let value = bytes
        .get(*at..*at + 8)
        .and_then(|s| <[u8; 8]>::try_from(s).ok())
        .map(f64::from_le_bytes)
        .ok_or_else(|| malformed("grid payload truncated in a header"))?;
    *at += 8;
    Ok(value)
}

fn take_varint(bytes: &[u8], at: &mut usize) -> Result<i64> {
    let mut zig = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *bytes
            .get(*at)
            .ok_or_else(|| malformed("grid payload ends mid-value"))?;
        *at += 1;
        zig |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return Err(malformed("a coordinate wider than 64 bits"));
        }
    }
    Ok(((zig >> 1) as i64) ^ -((zig & 1) as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_asset_lists_its_grids() {
        let table = entries(EMBEDDED).expect("the asset parses");
        assert!(!table.is_empty(), "the asset should hold at least one grid");
        for entry in &table {
            assert!(entry.cells > 0, "{} has no cells", entry.name);
            assert!(!entry.name.is_empty());
        }
    }

    /// The grid this app is built around: ICON global R03B07.
    #[test]
    fn icon_global_is_bundled_and_covers_the_earth() {
        let uuid = [
            0xa2, 0x7b, 0x8d, 0xe6, 0x18, 0xc4, 0x11, 0xe4, 0x82, 0x0a, 0xb5, 0xb0, 0x98, 0xc6,
            0xa5, 0xc0,
        ];
        let (entry, centres) = bundled(&uuid).expect("the asset parses").expect("R03B07");
        assert_eq!(entry.cells, 2_949_120);
        assert_eq!(centres.len(), 2_949_120);

        // A global mesh reaches both poles and both sides of the seam.
        let north = centres.lat.iter().cloned().fold(f32::MIN, f32::max);
        let south = centres.lat.iter().cloned().fold(f32::MAX, f32::min);
        assert!(north > 89.5, "northernmost cell at {north}");
        assert!(south < -89.5, "southernmost cell at {south}");
        assert!(centres.lon.iter().any(|&o| o < -179.0));
        assert!(centres.lon.iter().any(|&o| o > 179.0));
    }

    #[test]
    fn an_unknown_grid_is_not_guessed_at() {
        assert!(bundled(&[0u8; 16]).expect("the asset parses").is_none());
    }
}
