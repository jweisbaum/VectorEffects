//! Backdrops: what the map draws *under* everything (spec.md 4.11).
//!
//! Three of them, and one mechanism. An S-57 chart directory, OpenStreetMap
//! raster tiles, and a GIS layer's vector file all become RGBA tiles of the
//! application's own pyramid, served by the same URI scheme the field tiles
//! use and drawn by the map as plain textures beneath the field.
//!
//! **None of it is field data.** A backdrop reaches no scene, no
//! `FlatScene` hash, no render-cache key and no exported file; deleting
//! every one of these files changes nothing about what a project *is*
//! (invariants 1 and 2). The chart and the map tiles are not even layers:
//! they are a view setting, like the graticule, and the layer panel shows
//! nothing for them.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_chart::paint::{Painted, Style, TileFrame, TilePainter};
use ve_chart::s57::{self, Library, Palette};
use ve_chart::{Bounds, Feature, Geometry, Vectors};
use ve_render::tile::{TILE_SIZE, TileId};

use crate::commands::AppState;
use crate::error::{AppError, Result};

/// What a backdrop tile request names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    /// The S-57 chart directory in the settings.
    Chart,
    /// OpenStreetMap raster tiles.
    Osm,
    /// One GIS layer of the open project.
    Gis(u64),
}

/// What the frontend is told about the chart directory (spec.md 4.11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "ChartStatus.ts")]
pub struct ChartStatus {
    /// The directory, as the user chose it. Empty when none is set.
    pub directory: String,
    /// How many cells were found in it.
    pub cells: u32,
    /// What they cover: west, south, east, north. Absent for an empty set.
    pub bounds: Option<Vec<f64>>,
    /// What went wrong, where something did.
    pub error: Option<String>,
    /// An exact JavaScript integer that changes with the directory or palette,
    /// so cached tile addresses follow the application theme.
    pub token: u64,
}

/// The chart directory and the map tiles, held for the life of the process.
///
/// Opened lazily and only when something asks: a person who never turns a
/// backdrop on never pays for one, and — for the map tiles — never reaches
/// the network (invariant 5).
#[derive(Debug, Default)]
pub struct Backdrops {
    charts: Mutex<Option<Arc<Charts>>>,
    osm: Mutex<Option<Arc<ve_osm::Tiles>>>,
    /// A GIS file's features, by the path they were read from.
    vectors: Mutex<Vec<(PathBuf, Arc<Vectors>)>>,
    /// Recent immutable chart URL palettes, including earlier custom edits.
    palettes: Mutex<Vec<(u64, Palette)>>,
}

/// An indexed chart directory, and what it was opened from.
#[derive(Debug)]
pub struct Charts {
    /// Where it was opened from.
    pub directory: PathBuf,
    /// The index.
    pub library: Library,
}

/// How many GIS files' geometry is held at once. A project has a handful of
/// such layers; this is only a guard against a hundred.
const VECTORS_HELD: usize = 16;

/// How many cells one tile may draw, coarse first.
const CELL_BUDGET: usize = 16;

fn token_of(text: &str, palette: &Palette) -> u64 {
    // WebKit caches backdrop URLs across launches. Include the palette so a
    // theme change cannot leave chart tiles in the previous colours.
    let identity = format!("{text}\n{palette:?}");
    let hash = blake3::hash(identity.as_bytes());
    // The frontend carries this through JSON and back in a URL. Keep all bits
    // within Number's exact-integer range so native token matching is lossless.
    (u64::from_le_bytes(
        hash.as_bytes()[..8]
            .try_into()
            .unwrap_or([0, 0, 0, 0, 0, 0, 0, 0]),
    ) & ((1_u64 << 53) - 1))
        | 1
}

impl Backdrops {
    /// Resolve an immutable chart URL without substituting newly edited colours.
    fn palette_for_token(&self, directory: &str, token: u64) -> Option<Palette> {
        self.palettes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(held, palette)| *held == token && token_of(directory, palette) == token)
            .map(|(_, palette)| *palette)
            .or_else(|| {
                crate::theme::themes()
                    .iter()
                    .map(crate::theme::Theme::chart_palette)
                    .find(|palette| token_of(directory, palette) == token)
            })
    }

    /// The chart directory named by the settings, indexed on first use.
    ///
    /// A directory that changes is re-indexed; one that is unset or will not
    /// open leaves nothing to draw, which is not an error until somebody
    /// asks for the status.
    pub fn charts(&self, directory: &str) -> Option<Arc<Charts>> {
        if directory.trim().is_empty() {
            *self.charts.lock().unwrap_or_else(PoisonError::into_inner) = None;
            return None;
        }
        let mut held = self.charts.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(charts) = held.as_ref()
            && charts.directory.as_os_str() == directory
        {
            return Some(charts.clone());
        }
        let path = PathBuf::from(directory);
        let library = Library::open(&path)
            .inspect_err(|err| tracing::warn!(%err, directory, "chart directory"))
            .ok()?;
        let charts = Arc::new(Charts {
            directory: path,
            library,
        });
        *held = Some(charts.clone());
        Some(charts)
    }

    /// What to tell the frontend about the chart directory.
    pub fn chart_status(&self, directory: &str, palette: Palette) -> ChartStatus {
        let token = token_of(directory, &palette);
        {
            let mut palettes = self.palettes.lock().unwrap_or_else(PoisonError::into_inner);
            palettes.retain(|(held, _)| *held != token);
            palettes.push((token, palette));
            if palettes.len() > 32 {
                palettes.remove(0);
            }
        }
        let empty = ChartStatus {
            directory: directory.to_owned(),
            cells: 0,
            bounds: None,
            error: None,
            token,
        };
        if directory.trim().is_empty() {
            return empty;
        }
        // A directory that holds no cell is the likeliest mistake — the
        // exchange set's parent, or an archive unzipped one level up — and
        // indexing it succeeds with nothing in it. So "it worked, and there
        // is nothing here" has to be said rather than shown as an empty map.
        let found = self
            .charts(directory)
            .filter(|charts| !charts.library.entries().is_empty());
        match found {
            Some(charts) => ChartStatus {
                cells: charts.library.entries().len() as u32,
                bounds: charts
                    .library
                    .bounds()
                    .map(|b| vec![b.west, b.south, b.east, b.north]),
                ..empty
            },
            None => ChartStatus {
                error: Some(format!(
                    "no S-57 cells were found in {directory}. An ENC exchange set holds \
                     .000 files, usually in folders beside a CATALOG.031."
                )),
                ..empty
            },
        }
    }

    /// The OpenStreetMap tile source, built on first use.
    ///
    /// **The one place the application reaches the network for a tile**, and
    /// only from here, and only once the user has turned the layer on.
    pub fn osm(&self, cache: &Path, agent: &str) -> Option<Arc<ve_osm::Tiles>> {
        let mut held = self.osm.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(tiles) = held.as_ref() {
            return Some(tiles.clone());
        }
        let tiles = Arc::new(
            ve_osm::Tiles::new(ve_osm::DEFAULT_TILE_URL, cache, agent)
                .inspect_err(|err| tracing::warn!(%err, "OpenStreetMap tiles"))
                .ok()?,
        );
        *held = Some(tiles.clone());
        Some(tiles)
    }

    /// A GIS file's features, read on first use and held.
    pub fn vectors(&self, path: &Path) -> Result<Arc<Vectors>> {
        let mut held = self.vectors.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, vectors)) = held.iter().find(|(known, _)| known == path) {
            return Ok(vectors.clone());
        }
        let read =
            Arc::new(ve_chart::read_gis(path).map_err(|err| AppError::Internal(err.to_string()))?);
        held.push((path.to_path_buf(), read.clone()));
        if held.len() > VECTORS_HELD {
            held.remove(0);
        }
        Ok(read)
    }

    /// Forgets a GIS file, so the next draw reads it again.
    pub fn forget(&self, path: &Path) {
        self.vectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(known, _)| known != path);
    }
}

/// A tile's box, as the chart crate states one.
fn frame_of(id: TileId) -> TileFrame {
    let bounds = id.bounds();
    TileFrame {
        bounds: Bounds {
            west: bounds.west,
            south: bounds.south,
            east: bounds.east,
            north: bounds.north,
        },
        size: TILE_SIZE,
    }
}

/// Draws one backdrop tile, or `None` where there is nothing there.
///
/// Returns straight RGBA; the caller encodes it. Nothing here touches the
/// session: a backdrop depends on the settings and its own files, never on
/// what is being painted, which is what lets it be drawn while an edit is
/// in flight.
pub fn tile(state: &AppState, backdrop: Backdrop, id: TileId) -> Option<Vec<u8>> {
    tile_for_token(state, backdrop, id, None)
}

/// A queued chart request keeps the palette named by its immutable URL even
/// if the application switches themes while it is being drawn.
pub fn tile_for_token(
    state: &AppState,
    backdrop: Backdrop,
    id: TileId,
    token: Option<u64>,
) -> Option<Vec<u8>> {
    let frame = frame_of(id);
    match backdrop {
        Backdrop::Chart => {
            let directory = state.chart_directory();
            let palette = match token {
                Some(token) => state.backdrops.palette_for_token(&directory, token)?,
                None => {
                    let settings = crate::settings::settings_of(state).ok()?;
                    crate::theme::palette_for(&settings.theme, settings.custom_theme.as_ref())
                }
            };
            let charts = state.backdrops.charts(&directory)?;
            s57::draw_tile(&charts.library, frame, &palette, CELL_BUDGET)
        }
        Backdrop::Osm => {
            let tiles = state
                .backdrops
                .osm(&state.paths.osm_cache_dir(), &osm_agent())?;
            let zoom = ve_osm::source_zoom_for(id.z);
            let wanted = ve_osm::source_tiles(
                frame.bounds.west,
                frame.bounds.south,
                frame.bounds.east,
                frame.bounds.north,
                zoom,
            );
            let sources: Vec<_> = wanted
                .into_iter()
                .filter_map(|source| tiles.tile(source, true))
                .collect();
            ve_osm::warp(
                frame.bounds.west,
                frame.bounds.south,
                frame.bounds.east,
                frame.bounds.north,
                frame.size,
                &sources,
            )
        }
        Backdrop::Gis(layer) => {
            let (path, style) = state.gis_layer(layer)?;
            let vectors = state.backdrops.vectors(&path).ok()?;
            if !vectors.bounds.overlaps(&frame.bounds) {
                return None;
            }
            draw_vectors(&vectors.features, frame, style)
        }
    }
}

/// How this application identifies itself to the tile servers.
pub fn osm_agent() -> String {
    ve_osm::user_agent(env!("CARGO_PKG_VERSION"))
}

/// Draws a GIS layer's features into a tile.
fn draw_vectors(features: &[Feature], frame: TileFrame, style: Style) -> Option<Vec<u8>> {
    let mut painter = TilePainter::new(frame)?;
    let mut drawn = false;
    for feature in features {
        let Some(bounds) = feature.bounds() else {
            continue;
        };
        if !bounds.overlaps(&frame.bounds) {
            continue;
        }
        // **Only an area is filled.** The layer's fill opacity is the one
        // control here that means a thing about areas alone, and handing it
        // to the other two geometries drew neither what it is:
        //
        // - A LineString, a route or a track is a path. Filled, a GPX track
        //   came back as a translucent slab between its first and last point.
        //   The painter fills an open path on purpose, because an S-57 area
        //   arrives as its edges (`Style::close`), so the fill is dropped
        //   here — where the geometry is known — and not there.
        // - A point's mark takes the fill when it has one and the line
        //   colour otherwise, so a GPX waypoint under the default fill was a
        //   faint ring rather than the dot a mark is.
        let style = match feature.geometry {
            Geometry::Points(_) => Style {
                point_radius: style.width.max(1.0) * 1.8,
                fill: None,
                ..style
            },
            Geometry::Lines(_) => Style {
                fill: None,
                ..style
            },
            Geometry::Areas(_) => style,
        };
        painter.draw(Painted { feature, style });
        drawn = true;
    }
    (drawn && !painter.is_blank()).then(|| painter.into_rgba())
}

/// Turns `#rrggbb` into bytes, with an alpha. An unreadable colour reads as
/// the default rather than refusing to draw the layer.
pub fn rgba_of(colour: &str, alpha: f64) -> [u8; 4] {
    let hex = colour.trim().trim_start_matches('#');
    let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    let (r, g, b) = match hex.len() {
        6 | 8 => (channel(0), channel(2), channel(4)),
        _ => (None, None, None),
    };
    [
        r.unwrap_or(0xc8),
        g.unwrap_or(0xa0),
        b.unwrap_or(0x50),
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square, as a ring of points, in whichever geometry.
    fn square(as_area: bool) -> Feature {
        let ring = vec![
            [-4.0, -4.0],
            [4.0, -4.0],
            [4.0, 4.0],
            [-4.0, 4.0],
            [-4.0, -4.0],
        ];
        Feature {
            class: String::new(),
            attributes: Vec::new(),
            geometry: if as_area {
                Geometry::Areas(vec![vec![ring]])
            } else {
                Geometry::Lines(vec![ring])
            },
        }
    }

    /// The alpha at the middle of a tile spanning ±10°, which is well inside
    /// the square and far from its stroke.
    fn centre_alpha(feature: Feature, fill: Option<[u8; 4]>) -> u8 {
        let frame = ve_chart::paint::TileFrame {
            bounds: ve_chart::geometry::Bounds {
                west: -10.0,
                south: -10.0,
                east: 10.0,
                north: 10.0,
            },
            size: 64,
        };
        let style = Style {
            stroke: Some([255, 255, 255, 255]),
            fill,
            width: 1.0,
            point_radius: 0.0,
            close: false,
        };
        let rgba = draw_vectors(&[feature], frame, style).expect("something was drawn");
        let middle = (32 * 64 + 32) * 4;
        rgba[middle + 3]
    }

    /// A LineString, a route or a track is a path and not an area, so a
    /// layer's fill does not apply to it. The same ring proves it both ways:
    /// as an area it is filled, as a line it is hollow.
    #[test]
    fn a_line_is_never_filled_and_the_same_ring_as_an_area_is() {
        let fill = Some([200, 100, 50, 255]);
        assert_eq!(
            centre_alpha(square(false), fill),
            0,
            "a line was filled; a GPX track would paint a slab"
        );
        assert!(
            centre_alpha(square(true), fill) > 0,
            "an area with a fill asked for must still be filled"
        );
    }

    /// A mark is a dot in the line colour, not a ring of the area fill: a
    /// waypoint at the default fill opacity would otherwise barely show.
    #[test]
    fn a_point_takes_the_line_colour_whatever_the_fill_is() {
        let frame = ve_chart::paint::TileFrame {
            bounds: ve_chart::geometry::Bounds {
                west: -10.0,
                south: -10.0,
                east: 10.0,
                north: 10.0,
            },
            size: 64,
        };
        let mark = Feature {
            class: String::new(),
            attributes: Vec::new(),
            geometry: Geometry::Points(vec![[0.0, 0.0, f64::NAN]]),
        };
        let style = Style {
            stroke: Some([255, 255, 255, 255]),
            // A fill as faint as the panel's default, which the mark must
            // not take for its own colour.
            fill: Some([200, 100, 50, 46]),
            width: 3.0,
            point_radius: 0.0,
            close: false,
        };
        let rgba = draw_vectors(&[mark], frame, style).expect("the mark is drawn");
        let middle = (32 * 64 + 32) * 4;
        assert_eq!(
            &rgba[middle..middle + 4],
            &[255, 255, 255, 255],
            "the mark took the fill instead of the line colour"
        );
    }

    #[test]
    fn a_colour_reads_as_its_bytes_and_a_bad_one_as_the_default() {
        assert_eq!(rgba_of("#c8a050", 1.0), [0xc8, 0xa0, 0x50, 255]);
        assert_eq!(rgba_of("11223344", 0.5), [0x11, 0x22, 0x33, 128]);
        assert_eq!(rgba_of("not a colour", 1.0), [0xc8, 0xa0, 0x50, 255]);
        assert_eq!(rgba_of("#fff", 1.0), [0xc8, 0xa0, 0x50, 255], "too short");
        assert_eq!(
            rgba_of("#000000", 2.0)[3],
            255,
            "alpha is held to its range"
        );
    }

    #[test]
    fn a_token_follows_the_directory_and_is_never_zero() {
        let palette = Palette::default();
        assert_eq!(token_of("/charts", &palette), token_of("/charts", &palette));
        assert_ne!(token_of("/charts", &palette), token_of("/other", &palette));
        assert_ne!(
            token_of("", &palette),
            0,
            "zero would read as no token at all"
        );
        let tokens: std::collections::BTreeSet<_> = crate::theme::themes()
            .iter()
            .map(|theme| token_of("/charts", &theme.chart_palette()))
            .collect();
        assert_eq!(
            tokens.len(),
            crate::theme::themes().len(),
            "each palette needs its own immutable URL"
        );
        assert!(tokens.iter().all(|token| *token < (1_u64 << 53)));
    }

    #[test]
    fn custom_chart_edits_keep_the_old_url_palette_and_get_a_new_token() {
        let backdrops = Backdrops::default();
        let original = Palette::default();
        let first = backdrops.chart_status("", original);
        let changed = Palette {
            land: [1, 2, 3, 255],
            ..original
        };
        let second = backdrops.chart_status("", changed);
        assert_ne!(first.token, second.token);
        assert!(second.token < (1_u64 << 53));
        assert_eq!(backdrops.palette_for_token("", first.token), Some(original));
        assert_eq!(backdrops.palette_for_token("", second.token), Some(changed));
        assert_eq!(
            backdrops.palette_for_token("another directory", second.token),
            None
        );
    }

    #[test]
    fn the_agent_names_the_application_as_the_tile_policy_asks() {
        let agent = osm_agent();
        assert!(agent.starts_with("VectorEffects/"), "{agent}");
        assert!(
            agent.contains("github.com"),
            "and where to find it: {agent}"
        );
    }
}

/// The colours a new GIS layer is given, in order, so two layers imported
/// one after another are told apart without the user choosing anything.
const GIS_COLOURS: [&str; 6] = [
    "#c8a050", "#7fb3d5", "#a8c686", "#d98880", "#bb8fce", "#f0e68c",
];

/// Imports a GIS vector file as a display-only layer (spec.md 4.11).
///
/// A georeferenced raster is *not* imported here: a GeoTIFF is a picture,
/// and the application already places one as an image layer (spec.md 4.9).
/// The dialog offers both and this routes the vector ones.
#[tauri::command(async)]
pub fn import_gis(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<crate::projects::ProjectSummary> {
    gis_imported(&state, path)
}

/// Implementation of [`import_gis`].
pub fn gis_imported(state: &AppState, path: String) -> Result<crate::projects::ProjectSummary> {
    let path = PathBuf::from(path);
    // Read before the layer is made, so a file that cannot be read is an
    // error the user sees rather than an empty layer they have to delete.
    let vectors = state.backdrops.vectors(&path)?;
    let features = vectors.features.len();

    crate::projects::with_session(state, |session| {
        let open = session.require_open()?;
        let colour = GIS_COLOURS[open.project.layers.len() % GIS_COLOURS.len()];
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let mut layer = ve_core::document::Layer::new(name);
        layer.source = ve_core::document::LayerSource::Gis {
            path: path.clone(),
            colour: colour.to_owned(),
            width_px: 1.4,
            // An outline, not a wash: a survey boundary laid over the field
            // is there to be seen through.
            fill_opacity: 0.18,
        };
        let command = ve_core::command::Command::AddLayer {
            index: open.project.layers.len(),
            layer: Box::new(layer),
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        // The map addresses a GIS layer's tiles by the document revision, so
        // a new one has to reach a webview that caches by URL for a year.
        open.touch();
        tracing::info!(path = %path.display(), features, "imported GIS layer");
        Ok(crate::projects::ProjectSummary::of(session.require_open()?))
    })
}

/// Changes how a GIS layer is drawn.
#[tauri::command(async)]
pub fn set_gis_style(
    state: tauri::State<'_, AppState>,
    layer: u64,
    colour: Option<String>,
    width_px: Option<f64>,
    fill_opacity: Option<f64>,
) -> Result<crate::projects::ProjectSummary> {
    gis_style_set(&state, layer, colour, width_px, fill_opacity)
}

/// Implementation of [`set_gis_style`].
pub fn gis_style_set(
    state: &AppState,
    layer: u64,
    colour: Option<String>,
    width_px: Option<f64>,
    fill_opacity: Option<f64>,
) -> Result<crate::projects::ProjectSummary> {
    crate::projects::with_session(state, |session| {
        let open = session.require_open()?;
        let id = ve_core::Id::from_raw(layer);
        let found = open
            .project
            .layers
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .ok_or(AppError::BadOption {
                field: "layer",
                value: layer.to_string(),
            })?;
        let ve_core::document::LayerSource::Gis {
            colour: current,
            width_px: width,
            fill_opacity: fill,
            ..
        } = &mut found.source
        else {
            return Err(AppError::BadOption {
                field: "layer",
                value: "not a GIS layer".to_owned(),
            });
        };
        if let Some(chosen) = colour {
            *current = chosen;
        }
        if let Some(chosen) = width_px {
            *width = chosen.clamp(0.2, 12.0);
        }
        if let Some(chosen) = fill_opacity {
            *fill = chosen.clamp(0.0, 1.0);
        }
        // Not a history entry: this is how the layer looks, the way an
        // image layer's opacity is, and a slider dragged across its range
        // should not fill the undo stack.
        open.touch();
        Ok(crate::projects::ProjectSummary::of(session.require_open()?))
    })
}
