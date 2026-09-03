//! The bundled basemap asset.
//!
//! The coastline data is converted from Natural Earth at build time by
//! `tools/basemap-builder` and committed to `assets/basemap.bin`, so the app
//! ships its own basemap and never contacts a tile server (invariant 5).
//!
//! The bytes are handed to the frontend unparsed: the renderer wants typed
//! arrays of vertices and indices, so decoding here only to re-encode for the
//! webview would be wasted work. What this module does provide is validation,
//! which turns a corrupt or truncated asset into a clear error at startup
//! rather than a blank map with no explanation.

use crate::error::{RenderError, Result};

/// The compiled-in basemap.
pub const EMBEDDED: &[u8] = include_bytes!("../../../assets/basemap.bin");

/// File magic.
const MAGIC: &[u8; 4] = b"VEBM";
/// Format version this build understands.
pub const FORMAT_VERSION: u32 = 1;

/// Summary of one level of detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LodInfo {
    /// Natural Earth scale marker: 110 or 50.
    pub marker: u32,
    /// Number of triangle vertices.
    pub tri_vertices: u32,
    /// Number of triangle indices.
    pub tri_indices: u32,
    /// Number of coastline vertices.
    pub line_vertices: u32,
    /// Number of coastline rings.
    pub line_strips: u32,
}

/// What the asset contains, without decoding its geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasemapInfo {
    /// Format version.
    pub version: u32,
    /// One entry per level of detail, coarsest first.
    pub lods: Vec<LodInfo>,
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32> {
    bytes
        .get(at..at + 4)
        .and_then(|s| <[u8; 4]>::try_from(s).ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| RenderError::Asset(format!("basemap truncated at byte {at}")))
}

/// Validates the asset and reports what it holds.
///
/// Every declared section is bounds-checked against the actual length, so a
/// truncated asset is caught here rather than by the frontend reading past the
/// end of a buffer.
pub fn inspect(bytes: &[u8]) -> Result<BasemapInfo> {
    if bytes.get(..4) != Some(MAGIC.as_slice()) {
        return Err(RenderError::Asset("basemap has wrong magic".to_owned()));
    }
    let version = read_u32(bytes, 4)?;
    if version != FORMAT_VERSION {
        return Err(RenderError::Asset(format!(
            "basemap format version {version}, expected {FORMAT_VERSION}"
        )));
    }

    let lod_count = read_u32(bytes, 8)?;
    if lod_count == 0 || lod_count > 8 {
        return Err(RenderError::Asset(format!(
            "implausible lod count {lod_count}"
        )));
    }

    let mut pos = 12usize;
    let mut lods = Vec::with_capacity(lod_count as usize);
    for _ in 0..lod_count {
        let info = LodInfo {
            marker: read_u32(bytes, pos)?,
            tri_vertices: read_u32(bytes, pos + 4)?,
            tri_indices: read_u32(bytes, pos + 8)?,
            line_vertices: read_u32(bytes, pos + 12)?,
            line_strips: read_u32(bytes, pos + 16)?,
        };
        pos += 20;

        // Section sizes, in the order the builder writes them.
        let payload = (info.tri_vertices as usize) * 8
            + (info.tri_indices as usize) * 4
            + (info.line_vertices as usize) * 8
            + (info.line_strips as usize) * 8;
        pos = pos
            .checked_add(payload)
            .ok_or_else(|| RenderError::Asset("basemap section sizes overflow".to_owned()))?;
        if pos > bytes.len() {
            return Err(RenderError::Asset(format!(
                "basemap lod {} claims {payload} bytes past the end of a {}-byte file",
                info.marker,
                bytes.len()
            )));
        }
        lods.push(info);
    }

    if pos != bytes.len() {
        return Err(RenderError::Asset(format!(
            "basemap has {} trailing bytes",
            bytes.len() - pos
        )));
    }
    Ok(BasemapInfo { version, lods })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped asset must be well formed. If the builder or the committed
    /// binary drift apart, this fails at test time rather than as a blank map.
    #[test]
    fn the_embedded_asset_is_valid() {
        let info = inspect(EMBEDDED).expect("embedded basemap must parse");
        assert_eq!(info.version, FORMAT_VERSION);
        assert_eq!(info.lods.len(), 2, "expected a 110m and a 50m level");

        let markers: Vec<u32> = info.lods.iter().map(|l| l.marker).collect();
        assert_eq!(markers, vec![110, 50], "levels must be coarsest first");

        for lod in &info.lods {
            assert!(lod.tri_indices > 0 && lod.tri_indices % 3 == 0, "{lod:?}");
            assert!(lod.tri_vertices > 0 && lod.line_strips > 0, "{lod:?}");
        }
        // The finer level must actually be finer, or the LOD switch is pointless.
        assert!(info.lods[1].tri_indices > info.lods[0].tri_indices * 4);
    }

    #[test]
    fn a_truncated_asset_is_rejected() {
        let half = &EMBEDDED[..EMBEDDED.len() / 2];
        assert!(matches!(inspect(half), Err(RenderError::Asset(_))));
    }

    #[test]
    fn wrong_magic_is_rejected() {
        assert!(matches!(inspect(b"NOPE1234"), Err(RenderError::Asset(_))));
        assert!(matches!(inspect(&[]), Err(RenderError::Asset(_))));
    }

    #[test]
    fn a_future_version_is_rejected() {
        let mut bytes = EMBEDDED[..16].to_vec();
        bytes[4..8].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
        assert!(matches!(inspect(&bytes), Err(RenderError::Asset(_))));
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = EMBEDDED.to_vec();
        bytes.push(0);
        assert!(matches!(inspect(&bytes), Err(RenderError::Asset(_))));
    }
}
