//! Drawing a layer of sea-surface temperature (spec.md 4.10, M93).
//!
//! An SST layer is display only: it reaches no scene and no export, and is
//! drawn as coloured tiles under the field through the same backdrop path a
//! GIS layer takes. Each tile samples the day's temperatures — bilinearly,
//! the missing corners left out, as the field's own sampler does — and
//! colours them on one fixed ramp, so a colour means the same temperature on
//! every day and in every project. Where there is no water there is no
//! colour.
//!
//! A tile is addressed by the *day's* token, a hash of its temperatures, so
//! the steps that show one day share its tiles and a tile can be served
//! immutable: a different day is a different address.

use std::sync::Arc;

use serde::Serialize;
use ts_rs::TS;
use ve_core::raster::RasterGrid;

use crate::commands::AppState;
use crate::error::Result;
use crate::projects::with_session;

/// The coldest temperature on the ramp, in degrees Celsius: sea water
/// freezes a little below zero.
pub const COLDEST_C: f32 = -2.0;
/// The warmest: the warmest open ocean.
pub const WARMEST_C: f32 = 32.0;

/// The ramp's stops, cold to warm, as sRGB. A blue-to-red thermal scale
/// whose brightness rises and falls once, so neither end is lost against the
/// dark map.
const STOPS: [[u8; 3]; 7] = [
    [40, 26, 120],
    [33, 102, 172],
    [67, 170, 196],
    [153, 213, 148],
    [254, 224, 139],
    [244, 109, 67],
    [165, 0, 38],
];

/// How strongly the layer is drawn: the field and its glyphs are on top.
const ALPHA: u8 = 235;

/// A temperature's colour on the ramp. Values beyond the ends take the end's
/// colour; a temperature that is not a number has none.
pub fn colour(celsius: f32) -> Option<[u8; 4]> {
    if !celsius.is_finite() {
        return None;
    }
    let t = ((celsius - COLDEST_C) / (WARMEST_C - COLDEST_C)).clamp(0.0, 1.0);
    let at = t * (STOPS.len() - 1) as f32;
    let i = (at.floor() as usize).min(STOPS.len() - 2);
    let f = at - i as f32;
    let mix = |k: usize| {
        let (a, b) = (f32::from(STOPS[i][k]), f32::from(STOPS[i + 1][k]));
        (a + (b - a) * f).round() as u8
    };
    Some([mix(0), mix(1), mix(2), ALPHA])
}

/// A day's address: the first six bytes of its temperatures' hash, which a
/// JavaScript number holds exactly.
pub fn token(grid: &RasterGrid) -> u64 {
    let mut bytes = [0u8; 8];
    bytes[2..].copy_from_slice(&grid.hash[..6]);
    u64::from_be_bytes(bytes)
}

/// Paints one tile of a day's temperatures, or `None` where the tile has no
/// water at all — which is answered as "nothing here", not as a failure.
pub fn paint(
    grid: &RasterGrid,
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    size: u32,
) -> Option<Vec<u8>> {
    let n = size as usize;
    let mut rgba = vec![0u8; n * n * 4];
    let mut any = false;
    let (dx, dy) = (
        (east - west) / f64::from(size),
        (north - south) / f64::from(size),
    );
    for row in 0..n {
        let lat = north - (row as f64 + 0.5) * dy;
        for col in 0..n {
            let lon = west + (col as f64 + 0.5) * dx;
            let Some(c) = grid.sample(lon, lat).and_then(|uv| colour(uv.u)) else {
                continue;
            };
            rgba[(row * n + col) * 4..][..4].copy_from_slice(&c);
            any = true;
        }
    }
    any.then_some(rgba)
}

/// The day an SST layer shows that has this token, if the layer is shown.
pub fn day_of(state: &AppState, layer: u64, wanted: u64) -> Option<Arc<RasterGrid>> {
    let session = state.session.lock().ok()?;
    let open = session.open.as_ref()?;
    // Not held to the layer being shown: the address names a day, and a
    // tile served blank while the layer was hidden would be cached blank for
    // good under that day's immutable address. The map asks only for the
    // layers it shows.
    let found = open
        .project
        .layers
        .iter()
        .find(|candidate| candidate.id.raw() == layer)?;
    found
        .temperature
        .as_ref()?
        .frames
        .iter()
        .find(|frame| token(&frame.grid) == wanted)
        .map(|frame| Arc::clone(&frame.grid))
}

/// The temperature under a point at a step, in degrees Celsius, from the
/// topmost visible SST layer that has a value there.
pub fn temperature_at(state: &AppState, step: u32, lon: f64, lat: f64) -> Result<Option<f32>> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let settings = &open.project.settings;
        Ok(open
            .project
            .layers
            .iter()
            .rev()
            .filter(|layer| layer.visible)
            .filter_map(|layer| layer.temperature_frame(settings, step))
            .find_map(|frame| frame.grid.sample(lon, lat).map(|uv| uv.u)))
    })
}

/// The temperature under the pointer, for the readout (spec.md 4.10, M93).
/// `None` where no shown SST layer has a value — over land, or with no SST
/// layer at all.
#[tauri::command]
pub fn sample_temperature(
    state: tauri::State<'_, AppState>,
    step: u32,
    lon: f64,
    lat: f64,
) -> Result<Option<f32>> {
    temperature_at(&state, step, lon, lat)
}

/// What the frontend needs to draw one SST layer.
#[derive(Debug, Clone, Serialize, TS, schemars::JsonSchema)]
#[ts(export, export_to = "SstLayerView.ts")]
pub struct SstLayerView {
    /// The product, as `ve_zarr::Product::id` spells it.
    pub product: String,
    /// Which day each step shows, as that day's tile token; `None` where the
    /// layer shows nothing.
    pub tokens: Vec<Option<u64>>,
}

/// The view of an SST layer, one token per step.
pub fn view_of(
    layer: &ve_core::document::Layer,
    settings: &ve_core::project::ProjectSettings,
) -> Option<SstLayerView> {
    let ve_core::document::LayerSource::Sst { product, .. } = &layer.source else {
        return None;
    };
    Some(SstLayerView {
        product: product.clone(),
        tokens: (0..settings.step_count)
            .map(|s| layer.temperature_frame(settings, s).map(|f| token(&f.grid)))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ramp's ends are its first and last stops, its middle is the middle
    /// stop, and what is past an end is the end.
    #[test]
    fn the_ramp_runs_cold_to_warm() {
        assert_eq!(colour(COLDEST_C), Some([40, 26, 120, ALPHA]));
        assert_eq!(colour(WARMEST_C), Some([165, 0, 38, ALPHA]));
        assert_eq!(
            colour(15.0),
            Some([153, 213, 148, ALPHA]),
            "the middle stop"
        );
        assert_eq!(colour(-30.0), colour(COLDEST_C));
        assert_eq!(colour(45.0), colour(WARMEST_C));
        assert_eq!(colour(f32::NAN), None);
    }

    /// A pixel next to sea takes the sea's temperature — a missing corner is
    /// left out of the blend, as the field's own sampler leaves it — and a
    /// pixel between two land points is clear; a tile with no sea is nothing.
    #[test]
    fn a_tile_is_coloured_where_there_is_water() {
        use ve_core::raster::MISSING;
        // Three columns 10 degrees apart: 15 C at 0 E, land at 10 E and 20 E.
        let grid = RasterGrid::new(
            3,
            2,
            0.0,
            10.0,
            10.0,
            10.0,
            vec![
                [15.0, 15.0],
                [MISSING, MISSING],
                [MISSING, MISSING],
                [15.0, 15.0],
                [MISSING, MISSING],
                [MISSING, MISSING],
            ],
        )
        .unwrap();
        let coast = paint(&grid, 0.0, 0.0, 10.0, 10.0, 4).expect("some sea");
        assert_eq!(
            &coast[4 * 4..][..4],
            colour(15.0).unwrap(),
            "beside the sea"
        );
        let inland = paint(&grid, 10.0, 0.0, 20.0, 10.0, 4);
        assert!(inland.is_none(), "between two land points, nothing");
        assert!(
            paint(&grid, 40.0, 40.0, 50.0, 50.0, 4).is_none(),
            "off the grid, nothing"
        );
    }

    /// A day's token is stable for its temperatures, differs between days,
    /// and is exact as a JavaScript number.
    #[test]
    fn a_day_s_token_is_its_temperatures() {
        let day = |c: f32| RasterGrid::new(2, 2, 0.0, 1.0, 1.0, 1.0, vec![[c, c]; 4]).unwrap();
        assert_eq!(token(&day(15.0)), token(&day(15.0)));
        assert_ne!(token(&day(15.0)), token(&day(15.5)));
        assert!(token(&day(15.0)) < 1 << 53);
    }
}
