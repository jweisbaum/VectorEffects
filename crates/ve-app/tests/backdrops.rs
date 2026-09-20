#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The backdrops, end to end through the tile protocol (spec.md 4.11).
//!
//! What matters here is the *seam*: a path the map asks for becomes a
//! painted tile, a GIS file becomes a layer that draws, and none of it
//! reaches the field. The chart reader and the painter have their own tests
//! in `ve-chart`; this is the application's half.

use std::path::{Path, PathBuf};

use ve_app::charts::{self, Backdrop};
use ve_app::commands::AppState;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_render::tile::TileId;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-backdrops-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp root");
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn app(root: &TempRoot) -> AppState {
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Backdrops".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 4,
        },
        false,
    )
    .expect("create");
    state
}

/// The tile holding a place, at a level, in the application's own pyramid.
fn tile_at(z: u32, lon: f64, lat: f64) -> TileId {
    let span = 360.0 / f64::from(2 << z);
    TileId::new(
        z,
        ((lon + 180.0) / span) as u32,
        ((90.0 - lat) / span) as u32,
    )
    .expect("a tile")
}

fn painted(rgba: &[u8]) -> usize {
    rgba.chunks_exact(4).filter(|px| px[3] > 0).count()
}

/// A survey off the Virginia coast: a boundary, a track and two marks.
fn survey(path: &Path) {
    std::fs::write(
        path,
        r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"name":"Survey area"},
             "geometry":{"type":"Polygon","coordinates":[[[-76.2,37.4],[-75.6,37.4],[-75.6,37.9],[-76.2,37.9],[-76.2,37.4]]]}},
            {"type":"Feature","properties":{"name":"Transect"},
             "geometry":{"type":"LineString","coordinates":[[-76.2,37.4],[-75.6,37.9]]}},
            {"type":"Feature","properties":{},
             "geometry":{"type":"MultiPoint","coordinates":[[-76.0,37.6],[-75.8,37.5]]}}]}"#,
    )
    .expect("write survey");
}

#[test]
fn a_gis_file_becomes_a_layer_that_draws_where_it_says_it_is() {
    let root = TempRoot::new("gis");
    let state = app(&root);
    let path = root.path("survey.geojson");
    survey(&path);

    let before = ve_app::document::tree(&state, 0)
        .expect("tree")
        .layers
        .len();
    charts::gis_imported(&state, path.to_string_lossy().into_owned()).expect("import");
    let tree = ve_app::document::tree(&state, 0).expect("tree");
    assert_eq!(tree.layers.len(), before + 1, "one layer added");

    let layer = tree.layers.last().expect("the new layer");
    assert_eq!(layer.source, "gis");
    assert!(layer.grib.is_none(), "a GIS layer contributes no field");
    let gis = layer.gis.as_ref().expect("a GIS view");
    assert!(gis.loaded, "{:?}", gis.error);
    assert_eq!(gis.features, 3);
    assert_eq!(
        gis.bounds.as_ref().map(|b| b.len()),
        Some(4),
        "and says what it covers"
    );

    // Over the survey it draws; a tile away from it draws nothing at all,
    // rather than an empty picture the map would still upload.
    let over = charts::tile(&state, Backdrop::Gis(layer.id), tile_at(7, -75.9, 37.6));
    assert!(
        painted(&over.expect("a tile over the survey")) > 100,
        "the survey should be drawn where it is"
    );
    assert!(
        charts::tile(&state, Backdrop::Gis(layer.id), tile_at(7, 20.0, -30.0)).is_none(),
        "nothing to draw is drawn as nothing"
    );

    // Hidden is hidden: an eye that hides a layer hides all of it (M49).
    ve_app::document::layer_visibility(&state, layer.id, false).expect("hide");
    assert!(
        charts::tile(&state, Backdrop::Gis(layer.id), tile_at(7, -75.9, 37.6)).is_none(),
        "a hidden GIS layer is not drawn"
    );
}

#[test]
fn a_gis_layers_style_is_the_style_it_is_drawn_in() {
    let root = TempRoot::new("style");
    let state = app(&root);
    let path = root.path("survey.geojson");
    survey(&path);
    charts::gis_imported(&state, path.to_string_lossy().into_owned()).expect("import");
    let layer = ve_app::document::tree(&state, 0)
        .expect("tree")
        .layers
        .last()
        .expect("the layer")
        .id;

    let id = tile_at(7, -75.9, 37.6);
    let outlined = charts::tile(&state, Backdrop::Gis(layer), id).expect("drawn");

    // Filling the areas covers more of the tile than outlining them does.
    charts::gis_style_set(
        &state,
        layer,
        Some("#ff0000".to_owned()),
        Some(3.0),
        Some(1.0),
    )
    .expect("style");
    let filled = charts::tile(&state, Backdrop::Gis(layer), id).expect("drawn");
    assert!(
        painted(&filled) > painted(&outlined),
        "a filled area covers more than its outline: {} then {}",
        painted(&outlined),
        painted(&filled)
    );
    // And it is drawn in the colour asked for.
    let reddest = filled
        .chunks_exact(4)
        .filter(|px| px[3] > 200)
        .max_by_key(|px| px[0])
        .expect("an opaque pixel");
    assert!(reddest[0] > 200 && reddest[1] < 60, "{reddest:?}");

    let view = ve_app::document::tree(&state, 0).expect("tree");
    let gis = view
        .layers
        .last()
        .expect("layer")
        .gis
        .as_ref()
        .expect("view");
    assert_eq!(gis.colour, "#ff0000");
    assert_eq!(gis.width_px, 3.0);
    assert_eq!(gis.fill_opacity, 1.0);
    // The survey is a polygon, a line and two marks in one MultiPoint: three
    // features, one of them an area. The count is what the panel offers the
    // fill on, so it has to mean areas and not features.
    assert_eq!(gis.features, 3);
    assert_eq!(gis.areas, 1);
}

/// A GPX passage is all lines and marks, so the panel has no fill to offer:
/// `areas` is what it asks, and a route file must answer zero while still
/// holding features.
#[test]
fn a_track_and_route_file_reports_no_areas_to_fill() {
    let root = TempRoot::new("gpx");
    let state = app(&root);
    let path = root.path("passage.gpx");
    std::fs::write(
        &path,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="test">
  <wpt lat="41.5150" lon="-70.6700"><name>Woods Hole</name></wpt>
  <rte><name>To Nantucket</name>
    <rtept lat="41.5150" lon="-70.6700"/>
    <rtept lat="41.3900" lon="-70.3400"/>
    <rtept lat="41.2850" lon="-70.0970"/>
  </rte>
  <trk><name>Sailed</name><trkseg>
    <trkpt lat="41.5150" lon="-70.6700"/>
    <trkpt lat="41.4400" lon="-70.4900"/>
  </trkseg></trk>
</gpx>"#,
    )
    .expect("write passage");
    charts::gis_imported(&state, path.to_string_lossy().into_owned()).expect("import");

    let view = ve_app::document::tree(&state, 0).expect("tree");
    let gis = view
        .layers
        .last()
        .expect("layer")
        .gis
        .as_ref()
        .expect("view");
    assert!(gis.loaded, "{:?}", gis.error);
    assert_eq!(gis.features, 3, "a mark, a route and a track");
    assert_eq!(gis.areas, 0, "a passage has nothing to fill");
    // And it is drawn where the passage is, off Woods Hole.
    assert!(
        charts::tile(&state, Backdrop::Gis(gis.layer), tile_at(7, -70.4, 41.4)).is_some(),
        "the passage should be drawn where it is"
    );
}

#[test]
fn a_file_that_is_not_gis_data_is_refused_rather_than_added_as_an_empty_layer() {
    let root = TempRoot::new("refused");
    let state = app(&root);
    let before = ve_app::document::tree(&state, 0)
        .expect("tree")
        .layers
        .len();

    let bad = root.path("notes.txt");
    std::fs::write(&bad, "this is not a survey").expect("write");
    assert!(charts::gis_imported(&state, bad.to_string_lossy().into_owned()).is_err());

    // And one that *is* a GeoJSON but in metres: refused by what it is.
    let projected = root.path("utm.geojson");
    std::fs::write(
        &projected,
        r#"{"type":"Point","coordinates":[412345.0,4100000.0]}"#,
    )
    .expect("write");
    let err = charts::gis_imported(&state, projected.to_string_lossy().into_owned())
        .expect_err("refused");
    assert!(err.to_string().contains("degrees"), "{err}");

    assert_eq!(
        ve_app::document::tree(&state, 0)
            .expect("tree")
            .layers
            .len(),
        before,
        "a refused import leaves no layer behind"
    );
}

#[test]
fn the_chart_directory_says_what_is_in_it_and_an_empty_one_says_so() {
    let root = TempRoot::new("charts");
    let state = app(&root);

    let none = ve_app::settings::chart_directory_set(&state, String::new()).expect("clear");
    assert_eq!(none.cells, 0);
    assert!(none.error.is_none(), "no directory is not an error");
    assert!(charts::tile(&state, Backdrop::Chart, tile_at(5, -76.0, 37.0)).is_none());

    // A directory with no cells in it is a mistake worth naming.
    let empty = root.path("not-charts");
    std::fs::create_dir_all(&empty).expect("dir");
    let status =
        ve_app::settings::chart_directory_set(&state, empty.to_string_lossy().into_owned())
            .expect("set");
    assert_eq!(status.cells, 0);
    assert!(
        status
            .error
            .as_deref()
            .is_some_and(|text| text.contains(".000")),
        "{:?} should say what an exchange set looks like",
        status.error
    );

    // The setting survives, so the next launch draws what the last one chose.
    let settings = ve_app::settings::settings_of(&state).expect("settings");
    assert_eq!(settings.chart_directory, empty.to_string_lossy());
}

/// With a real exchange set: the tile a chart covers is painted, and the
/// address the map builds for it is the one the protocol parses.
#[test]
#[ignore = "reads the chart set named by VE_TEST_ENC"]
fn a_real_chart_directory_paints_the_coast_it_covers() {
    let Some(directory) = std::env::var_os("VE_TEST_ENC") else {
        return;
    };
    let root = TempRoot::new("real-charts");
    let state = app(&root);
    let status =
        ve_app::settings::chart_directory_set(&state, directory.to_string_lossy().into_owned())
            .expect("set");
    assert!(status.cells > 0, "{:?}", status.error);
    println!("{} cells, covering {:?}", status.cells, status.bounds);

    let bounds = status.bounds.expect("coverage");
    let (lon, lat) = ((bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0);
    for z in [3, 5, 7, 9] {
        let tile = charts::tile(&state, Backdrop::Chart, tile_at(z, lon, lat));
        let count = tile.as_ref().map_or(0, |rgba| painted(rgba));
        println!("z{z}: {count} of 65536 pixels painted");
        assert!(count > 1000, "z{z}: a tile over the charts should be chart");
    }
    // The Pacific, which these charts do not cover.
    assert!(charts::tile(&state, Backdrop::Chart, tile_at(7, -150.0, 10.0)).is_none());
}
