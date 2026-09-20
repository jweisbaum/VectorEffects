//! A directory of S-57 cells, indexed and drawn (spec.md 4.11).
//!
//! An ENC exchange set is a tree of cells with a `CATALOG.031` naming every
//! one and giving its box, which is what makes an index cheap: the cells
//! themselves are opened only when something is drawn from them. Where there
//! is no catalogue — a directory somebody assembled by hand — the cells are
//! found by their `.000` extension and each is opened once for its box.
//!
//! A cell's name says how detailed it is. `US5MD11M` is a US cell of usage
//! band 5, and the band runs 1 (overview) to 6 (berthing); a chart is drawn
//! from the bands that suit the zoom, coarse first, so a harbour cell paints
//! over the coastal one it sits inside.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::error::Result;
use crate::geometry::Bounds;
use crate::s57::iso8211::{File, Sub};
use crate::s57::{Cell, read_cell};

/// One cell, as the index knows it: without its geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Where the cell is.
    pub path: PathBuf,
    /// Its name, which is its file's stem (`US5MD11M`).
    pub name: String,
    /// What the publisher calls it, where the catalogue says.
    pub title: String,
    /// Usage band, 1 (overview) to 6 (berthing).
    pub band: u8,
    /// What it covers.
    pub bounds: Bounds,
}

/// The band a cell's name states: the third character of a standard S-57
/// name. A name that does not follow the convention is treated as coastal,
/// the middle of the range, rather than dropped.
fn band_of(name: &str) -> u8 {
    name.as_bytes()
        .get(2)
        .and_then(|c| (*c as char).to_digit(10))
        .filter(|band| (1..=6).contains(band))
        .map_or(3, |band| band as u8)
}

/// The finest band worth drawing for a tile this many degrees across.
///
/// One band per doubling of scale, from an overview at eight degrees a tile
/// to a berthing plan under a hundredth of one. A chart finer than the view
/// is a chart whose every line falls inside one pixel.
pub fn finest_band(degrees: f64) -> u8 {
    match degrees {
        d if d >= 8.0 => 1,
        d if d >= 2.0 => 2,
        d if d >= 0.5 => 3,
        d if d >= 0.125 => 4,
        d if d >= 0.03 => 5,
        _ => 6,
    }
}

/// An indexed directory of cells, with the ones drawn from held in memory.
#[derive(Debug)]
pub struct Library {
    /// Every cell found, coarsest band first.
    entries: Vec<Entry>,
    root: PathBuf,
    loaded: Mutex<HashMap<PathBuf, Option<Arc<Cell>>>>,
}

impl Library {
    /// Indexes a directory. Reads the catalogue where there is one, and
    /// falls back to opening every cell for its box where there is not.
    pub fn open(root: &Path) -> Result<Self> {
        let mut entries = match catalogue_entries(root) {
            Ok(entries) if !entries.is_empty() => entries,
            _ => scanned_entries(root),
        };
        // Coarse first, so a finer cell is painted over its neighbour; then
        // by name, so a chart is drawn the same way twice.
        entries.sort_by(|a, b| a.band.cmp(&b.band).then_with(|| a.name.cmp(&b.name)));
        Ok(Self {
            entries,
            root: root.to_path_buf(),
            loaded: Mutex::new(HashMap::new()),
        })
    }

    /// Where the library was opened from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every cell in the index.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Everything the library covers, or `None` for an empty one.
    pub fn bounds(&self) -> Option<Bounds> {
        let mut bounds = Bounds::EMPTY;
        for entry in &self.entries {
            bounds.merge(entry.bounds);
        }
        (bounds.west <= bounds.east).then_some(bounds)
    }

    /// The cells to draw for a box this many degrees across, coarse first
    /// and no more than `limit` of them.
    ///
    /// The finest band comes last so it paints over what it is inside. The
    /// limit is taken from the *finest* end, because that is the detail the
    /// zoom asked for; dropping it to keep an overview would draw a chart
    /// coarser than the one the user is looking at.
    ///
    /// **A set with nothing that coarse still draws.** Not every exchange
    /// set has an overview: a harbour pack begins at band 4, and zoomed out
    /// it should show its harbour charts rather than an empty sea. So when
    /// no band is coarse enough, the coarsest there is stands in.
    pub fn cells_for(&self, bounds: &Bounds, degrees: f64, limit: usize) -> Vec<&Entry> {
        let here: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.bounds.overlaps(bounds))
            .collect();
        let finest = finest_band(degrees);
        let coarsest = here.iter().map(|entry| entry.band).min().unwrap_or(finest);
        let keep = finest.max(coarsest);
        let mut wanted: Vec<&Entry> = here
            .into_iter()
            .filter(|entry| entry.band <= keep)
            .collect();
        if wanted.len() > limit {
            wanted.drain(..wanted.len() - limit);
        }
        wanted
    }

    /// A cell's contents, read on first use and held.
    ///
    /// A cell that will not read is remembered as unreadable rather than
    /// retried at every tile: a damaged file in a chart directory would
    /// otherwise cost a parse per tile for the life of the process.
    pub fn cell(&self, entry: &Entry) -> Option<Arc<Cell>> {
        let mut loaded = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(known) = loaded.get(&entry.path) {
            return known.clone();
        }
        let read = match read_cell(&entry.path) {
            Ok(cell) => Some(Arc::new(cell)),
            Err(err) => {
                tracing_warn(&entry.path, &err);
                None
            }
        };
        loaded.insert(entry.path.clone(), read.clone());
        read
    }

    /// How many cells are held in memory.
    pub fn held(&self) -> usize {
        self.loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Drops every cell held in memory, keeping the index.
    pub fn release(&self) {
        self.loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

/// `tracing` is the application's, not this crate's; a bad cell is reported
/// through the error that reaches the caller and noted here for a log.
fn tracing_warn(path: &Path, err: &crate::ChartError) {
    eprintln!("chart cell {} could not be read: {err}", path.display());
}

/// Reads `CATALOG.031`, which names every cell and gives its box.
fn catalogue_entries(root: &Path) -> Result<Vec<Entry>> {
    let path = ["CATALOG.031", "catalog.031"]
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| crate::ChartError::Unsupported("no catalogue".to_owned()))?;
    let bytes = std::fs::read(&path)?;
    let mut file = File::open(&bytes)?;
    let Some(format) = file.format("CATD").cloned() else {
        return Ok(Vec::new());
    };
    let index = |label: &str| format.index_of(label);
    let (file_at, title_at) = (index("FILE"), index("LFIL"));
    let (south, west, north, east) = (index("SLAT"), index("WLON"), index("NLAT"), index("ELON"));
    let mut entries = Vec::new();
    while let Some(record) = file.next_record()? {
        let Some(field) = record.field("CATD") else {
            continue;
        };
        let row = format.read_one(field);
        let text = |at: Option<usize>| at.and_then(|i| row.get(i)).and_then(Sub::text);
        let number = |at: Option<usize>| match at.and_then(|i| row.get(i)) {
            Some(Sub::Real(value)) => Some(*value),
            Some(Sub::Int(value)) => Some(*value as f64),
            Some(Sub::Text(text)) => text.trim().parse().ok(),
            _ => None,
        };
        let Some(relative) = text(file_at) else {
            continue;
        };
        if !relative.to_ascii_lowercase().ends_with(".000") {
            continue;
        }
        // The catalogue writes its paths the way the media was made, which
        // for an exchange set is with backslashes whatever the reader runs on.
        let mut path = root.to_path_buf();
        for part in relative.split(['\\', '/']).filter(|part| !part.is_empty()) {
            path.push(part);
        }
        let (Some(south), Some(west), Some(north), Some(east)) =
            (number(south), number(west), number(north), number(east))
        else {
            continue;
        };
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        entries.push(Entry {
            band: band_of(&name),
            title: text(title_at).unwrap_or(&name).to_owned(),
            name,
            path,
            bounds: Bounds {
                west,
                south,
                // A cell that crosses the antimeridian states an eastern
                // edge west of its western one; carried past 180 it is one
                // box again, which is what `Bounds::overlaps` expects.
                east: if east < west { east + 360.0 } else { east },
                north,
            },
        });
    }
    Ok(entries)
}

/// Finds cells by extension and opens each for its box.
fn scanned_entries(root: &Path) -> Vec<Entry> {
    let mut paths = Vec::new();
    collect(root, &mut paths, 0);
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let cell = read_cell(&path).ok()?;
            Some(Entry {
                band: band_of(&cell.name),
                title: cell.name.clone(),
                name: cell.name,
                path,
                bounds: cell.bounds,
            })
        })
        .collect()
}

fn collect(at: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out, depth + 1);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("000"))
        {
            out.push(path);
        }
    }
}

/// Draws one tile of chart, or `None` where no cell reaches it.
///
/// Cells are drawn coarse first so the finer paints over it, and within a
/// cell the features are ordered by [`style::layer_of`] rather than by the
/// order the file lists them — an area drawn after a contour would bury it.
pub fn draw_tile(
    library: &Library,
    frame: crate::paint::TileFrame,
    palette: &crate::s57::style::Palette,
    budget: usize,
) -> Option<Vec<u8>> {
    use crate::paint::{Painted, TilePainter};
    use crate::s57::style::{layer_of, shown_at, style_for};

    let degrees = frame.bounds.east - frame.bounds.west;
    let cells = library.cells_for(&frame.bounds, degrees, budget);
    if cells.is_empty() {
        return None;
    }
    // A tile's own scale, as a chart states one: its width in degrees over
    // its width on screen, taken as a denominator against a 0.28 mm pixel.
    let scale_denominator = degrees * 60.0 * 1852.0 / (f64::from(frame.size) * 0.000_28);

    let mut painter = TilePainter::new(frame)?;
    let mut drawn = false;
    for entry in cells {
        let Some(cell) = library.cell(entry) else {
            continue;
        };
        let mut order: Vec<(u8, usize)> = cell
            .features
            .iter()
            .enumerate()
            .map(|(index, feature)| (layer_of(&feature.class), index))
            .collect();
        order.sort_by_key(|(layer, index)| (*layer, *index));
        for (_, index) in order {
            let feature = &cell.features[index];
            if !shown_at(feature, scale_denominator) {
                continue;
            }
            let Some(style) = style_for(palette, feature) else {
                continue;
            };
            let Some(bounds) = feature.bounds() else {
                continue;
            };
            if !bounds.overlaps(&frame.bounds) {
                continue;
            }
            painter.draw(Painted { feature, style });
            drawn = true;
        }
    }
    (drawn && !painter.is_blank()).then(|| painter.into_rgba())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cells_name_says_how_detailed_it_is() {
        assert_eq!(band_of("US2ATLOC"), 2, "a general chart");
        assert_eq!(band_of("US5MD11M"), 5, "a harbour chart");
        assert_eq!(band_of("NO"), 3, "too short to say: the middle");
        assert_eq!(band_of("USXATLOC"), 3, "not a band: the middle");
    }

    /// One band per doubling, and the boundaries fall where they are stated.
    #[test]
    fn the_band_follows_the_zoom() {
        assert_eq!(finest_band(30.0), 1);
        assert_eq!(finest_band(8.0), 1);
        assert_eq!(finest_band(7.99), 2);
        assert_eq!(finest_band(2.0), 2);
        assert_eq!(finest_band(0.5), 3);
        assert_eq!(finest_band(0.125), 4);
        assert_eq!(finest_band(0.03), 5);
        assert_eq!(finest_band(0.0001), 6);
    }

    fn entry(name: &str, band: u8, west: f64, east: f64) -> Entry {
        Entry {
            path: PathBuf::from(format!("{name}.000")),
            name: name.to_owned(),
            title: name.to_owned(),
            band,
            bounds: Bounds {
                west,
                south: 30.0,
                east,
                north: 40.0,
            },
        }
    }

    fn library(entries: Vec<Entry>) -> Library {
        let mut entries = entries;
        entries.sort_by(|a, b| a.band.cmp(&b.band).then_with(|| a.name.cmp(&b.name)));
        Library {
            entries,
            root: PathBuf::new(),
            loaded: Mutex::new(HashMap::new()),
        }
    }

    /// The finest band the zoom allows is what is kept when there are more
    /// cells than the budget: an overview under a harbour plan is invisible,
    /// and dropping the harbour plan instead is dropping what was asked for.
    #[test]
    fn the_budget_is_spent_on_the_detail_the_zoom_asked_for() {
        let library = library(vec![
            entry("US1OVER", 1, -80.0, -60.0),
            entry("US3COAS", 3, -76.0, -74.0),
            entry("US5HARB", 5, -75.6, -75.4),
            entry("US5HARC", 5, -75.5, -75.3),
        ]);
        let here = Bounds {
            west: -75.55,
            south: 33.0,
            east: -75.45,
            north: 34.0,
        };
        let all = library.cells_for(&here, 0.01, 10);
        assert_eq!(
            all.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["US1OVER", "US3COAS", "US5HARB", "US5HARC"],
            "coarse first, so the finer paints over it"
        );
        let two = library.cells_for(&here, 0.01, 2);
        assert_eq!(
            two.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["US5HARB", "US5HARC"],
            "the budget keeps the detail, not the overview"
        );
        // Zoomed out, the harbour plans are not drawn at all.
        let wide = library.cells_for(&here, 3.0, 10);
        assert_eq!(
            wide.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["US1OVER"],
        );
    }

    /// A harbour pack has no overview cell. Zoomed out it has to show the
    /// charts it does have, or a chart directory of nothing but harbour
    /// plans would draw an empty sea until the user found one.
    #[test]
    fn a_set_with_nothing_coarse_enough_still_draws_its_coarsest() {
        let library = library(vec![
            entry("US4APPR", 4, -76.0, -75.0),
            entry("US5HARB", 5, -75.6, -75.4),
        ]);
        let here = Bounds {
            west: -75.55,
            south: 33.0,
            east: -75.45,
            north: 34.0,
        };
        let wide = library.cells_for(&here, 30.0, 10);
        assert_eq!(
            wide.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["US4APPR"],
            "the coarsest there is, and not the harbour plan under it"
        );
    }

    #[test]
    fn a_cell_that_does_not_reach_the_view_is_not_drawn() {
        let library = library(vec![entry("US3HERE", 3, -76.0, -74.0)]);
        let elsewhere = Bounds {
            west: 10.0,
            south: 33.0,
            east: 11.0,
            north: 34.0,
        };
        assert!(library.cells_for(&elsewhere, 0.5, 10).is_empty());
    }
}
