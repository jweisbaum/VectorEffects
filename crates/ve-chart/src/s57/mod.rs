//! Reading an S-57 electronic navigational chart cell (spec.md 4.11).
//!
//! A cell is an ISO 8211 file (`iso8211`) of two kinds of record. *Vector*
//! records hold the geometry: isolated and connected nodes, and edges, each
//! a run of coordinates in ten-millionths of a degree. *Feature* records say
//! what the geometry means — an object class, its attributes, and pointers
//! to the vector records that give it a shape. Nothing in a cell is drawn
//! without following those pointers, which is most of what this module does.
//!
//! What is read is the base edition, the `.000` file. Update files are not
//! applied (spec.md 4.11, and the chart panel says so), so a cell is exactly
//! as its publisher first issued it.

mod catalogue;
pub mod iso8211;
pub mod library;
pub mod style;

use std::collections::HashMap;
use std::path::Path;

use crate::error::{ChartError, Result};
use crate::geometry::{Bounds, Feature, Geometry, Value};
use iso8211::{File, Sub};

pub use catalogue::{attribute_name, object_class};
pub use library::{Entry, Library, draw_tile, finest_band};
pub use style::{Palette, layer_of, shown_at, style_for};

/// A record's name: its kind, then its number within the cell.
type Name = (u8, u32);

/// A pointer from a feature to a vector record: what it points at, which way
/// round, and what the pointed-at edge is to the feature.
type Pointer = (Name, u8, u8);

/// A feature record read but not yet given its shape: a feature may point at
/// a vector record that comes after it in the file, so the geometry is
/// assembled in a second pass.
type Pending = (String, Vec<(String, Value)>, u8, Vec<Pointer>);

/// Vector record kinds (S-57 RCNM).
const ISOLATED_NODE: u8 = 110;
const CONNECTED_NODE: u8 = 120;
const EDGE: u8 = 130;

/// A chart cell, read.
#[derive(Debug, Clone)]
pub struct Cell {
    /// The cell's own name, from its file (`US5MD11M`).
    pub name: String,
    /// Compilation scale: 1:`scale`. What decides when a cell is shown.
    pub scale: u32,
    /// Everything the cell covers.
    pub bounds: Bounds,
    /// Its features, in the order the file lists them.
    pub features: Vec<Feature>,
}

/// One vector record: its coordinates, and the nodes an edge runs between.
#[derive(Debug, Default, Clone)]
struct Vector {
    /// Positions, as longitude, latitude, depth (NaN where there is none).
    points: Vec<[f64; 3]>,
    /// For an edge: the connected node it starts at, and the one it ends at.
    begin: Option<Name>,
    end: Option<Name>,
}

/// Reads a cell from its `.000` file.
pub fn read_cell(path: &Path) -> Result<Cell> {
    let bytes = std::fs::read(path)?;
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    read_bytes(&name, &bytes)
}

/// Reads a cell already in memory, for a test or an archive.
pub fn read_bytes(name: &str, bytes: &[u8]) -> Result<Cell> {
    let mut file = File::open(bytes)?;

    // Ten-millionths of a degree and tenths of a metre, unless the cell says
    // otherwise; every cell does say, in its parameter record.
    let mut coordinate_factor = 10_000_000.0;
    let mut sounding_factor = 10.0;
    let mut scale = 0_u32;

    let mut vectors: HashMap<Name, Vector> = HashMap::new();
    let mut features = Vec::new();
    // Feature records are read in a second pass: a feature may point to a
    // vector record that comes after it in the file.
    let mut pending: Vec<Pending> = Vec::new();

    while let Some(record) = file.next_record()? {
        if let Some(field) = record.field("DSPM") {
            let format = file
                .format("DSPM")
                .ok_or_else(|| ChartError::malformed("an S-57 cell", "DSPM is not described"))?;
            let row = format.read_one(field);
            let at = |label: &str| format.index_of(label).and_then(|i| row.get(i)).cloned();
            if let Some(value) = at("COMF").and_then(|v| v.int()).filter(|n| *n > 0) {
                coordinate_factor = value as f64;
            }
            if let Some(value) = at("SOMF").and_then(|v| v.int()).filter(|n| *n > 0) {
                sounding_factor = value as f64;
            }
            if let Some(value) = at("CSCL").and_then(|v| v.int()).filter(|n| *n > 0) {
                scale = value.min(i64::from(u32::MAX)) as u32;
            }
            continue;
        }

        if let Some(field) = record.field("VRID") {
            let Some(name) = record_name(&file, "VRID", field, "RCNM", "RCID") else {
                continue;
            };
            let mut vector = Vector::default();
            for (tag, factor) in [("SG2D", 0.0), ("SG3D", sounding_factor)] {
                let (Some(data), Some(format)) = (record.field(tag), file.format(tag)) else {
                    continue;
                };
                let y = format.index_of("YCOO");
                let x = format.index_of("XCOO");
                let z = format.index_of("VE3D");
                for row in format.read(data) {
                    let value = |index: Option<usize>| index.and_then(|i| row.get(i)?.int());
                    let (Some(lat), Some(lon)) = (value(y), value(x)) else {
                        continue;
                    };
                    let depth = match (z, factor) {
                        (Some(_), f) if f > 0.0 => value(z).map_or(f64::NAN, |raw| raw as f64 / f),
                        _ => f64::NAN,
                    };
                    vector.points.push([
                        lon as f64 / coordinate_factor,
                        lat as f64 / coordinate_factor,
                        depth,
                    ]);
                }
            }
            // An edge names the connected nodes it runs between. Which end
            // is which is the topology indicator, not the order listed.
            if let (Some(data), Some(format)) = (record.field("VRPT"), file.format("VRPT")) {
                let name_at = format.index_of("NAME");
                let topi = format.index_of("TOPI");
                for row in format.read(data) {
                    let Some(pointed) = name_at
                        .and_then(|i| row.get(i))
                        .and_then(Sub::bytes)
                        .and_then(name_of)
                    else {
                        continue;
                    };
                    match topi.and_then(|i| row.get(i)?.int()) {
                        Some(1) => vector.begin = Some(pointed),
                        Some(2) => vector.end = Some(pointed),
                        _ => {}
                    }
                }
            }
            vectors.insert(name, vector);
            continue;
        }

        if let Some(field) = record.field("FRID") {
            let Some(format) = file.format("FRID") else {
                continue;
            };
            let row = format.read_one(field);
            let at = |label: &str| format.index_of(label).and_then(|i| row.get(i)?.int());
            let class = at("OBJL")
                .map(|code| object_class(code as u16))
                .unwrap_or_default();
            let primitive = at("PRIM").unwrap_or(255) as u8;

            let mut attributes = Vec::new();
            for tag in ["ATTF", "NATF"] {
                let (Some(data), Some(format)) = (record.field(tag), file.format(tag)) else {
                    continue;
                };
                let label = format.index_of("ATTL");
                let value = format.index_of("ATVL");
                for row in format.read(data) {
                    let Some(code) = label.and_then(|i| row.get(i)?.int()) else {
                        continue;
                    };
                    let text = value
                        .and_then(|i| row.get(i))
                        .and_then(Sub::text)
                        .unwrap_or("")
                        .trim()
                        .to_owned();
                    if text.is_empty() {
                        continue;
                    }
                    let name = attribute_name(code as u16);
                    attributes.push((name, parse_value(&text)));
                }
            }

            let mut pointers = Vec::new();
            if let (Some(data), Some(format)) = (record.field("FSPT"), file.format("FSPT")) {
                let name_at = format.index_of("NAME");
                let ornt = format.index_of("ORNT");
                let usag = format.index_of("USAG");
                for row in format.read(data) {
                    let Some(pointed) = name_at
                        .and_then(|i| row.get(i))
                        .and_then(Sub::bytes)
                        .and_then(name_of)
                    else {
                        continue;
                    };
                    pointers.push((
                        pointed,
                        ornt.and_then(|i| row.get(i)?.int()).unwrap_or(255) as u8,
                        usag.and_then(|i| row.get(i)?.int()).unwrap_or(255) as u8,
                    ));
                }
            }
            if !class.is_empty() && !pointers.is_empty() {
                pending.push((class, attributes, primitive, pointers));
            }
        }
    }

    let mut bounds = Bounds::EMPTY;
    for (class, attributes, primitive, pointers) in pending {
        let Some(geometry) = assemble(primitive, &pointers, &vectors) else {
            continue;
        };
        let feature = Feature {
            class,
            attributes,
            geometry,
        };
        if let Some(own) = feature.bounds() {
            bounds.merge(own);
        }
        features.push(feature);
    }
    if bounds.west > bounds.east {
        return Err(ChartError::malformed(
            "an S-57 cell",
            "holds no feature with a position",
        ));
    }

    Ok(Cell {
        name: name.to_owned(),
        scale,
        bounds,
        features,
    })
}

/// Text to a value: a number where it is one, characters otherwise. S-57
/// writes every attribute as text, including its enumerations.
fn parse_value(text: &str) -> Value {
    if let Ok(whole) = text.parse::<i64>() {
        return Value::Int(whole);
    }
    if let Ok(real) = text.parse::<f64>()
        && real.is_finite()
    {
        return Value::Real(real);
    }
    Value::Text(text.to_owned())
}

/// A record's own name, from the two subfields that carry it.
fn record_name(file: &File<'_>, tag: &str, field: &[u8], kind: &str, id: &str) -> Option<Name> {
    let format = file.format(tag)?;
    let row = format.read_one(field);
    let at = |label: &str| format.index_of(label).and_then(|i| row.get(i)?.int());
    Some((at(kind)? as u8, at(id)? as u32))
}

/// A pointer's five bytes: the kind, then the number, little-endian.
fn name_of(bytes: &[u8]) -> Option<Name> {
    let (kind, rest) = bytes.split_first()?;
    if rest.len() < 4 {
        return None;
    }
    Some((
        *kind,
        u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]),
    ))
}

/// The positions along one edge, from the node it begins at to the node it
/// ends at, in the direction asked for.
fn edge_points(name: Name, forward: bool, vectors: &HashMap<Name, Vector>) -> Vec<[f64; 3]> {
    let Some(edge) = vectors.get(&name) else {
        return Vec::new();
    };
    let node = |at: Option<Name>| {
        at.and_then(|name| vectors.get(&name))
            .and_then(|node| node.points.first().copied())
    };
    let mut points = Vec::with_capacity(edge.points.len() + 2);
    points.extend(node(edge.begin));
    points.extend(edge.points.iter().copied());
    points.extend(node(edge.end));
    if !forward {
        points.reverse();
    }
    points
}

/// Builds a feature's shape from the vector records it points to.
///
/// An area is stated as the edges around it, which are given in order but
/// each in its own direction; the orientation subfield says which. A ring is
/// closed when the walk returns to where it started, and what follows is
/// another ring — a hole, or an island's own outline.
fn assemble(
    primitive: u8,
    pointers: &[Pointer],
    vectors: &HashMap<Name, Vector>,
) -> Option<Geometry> {
    match primitive {
        // Point.
        1 => {
            let mut points = Vec::new();
            for (name, _, _) in pointers {
                if name.0 == ISOLATED_NODE || name.0 == CONNECTED_NODE {
                    points.extend(vectors.get(name)?.points.iter().copied());
                }
            }
            (!points.is_empty()).then_some(Geometry::Points(points))
        }
        // Line.
        2 => {
            let mut lines: Vec<Vec<[f64; 2]>> = Vec::new();
            for (name, orientation, _) in pointers {
                if name.0 != EDGE {
                    continue;
                }
                let points = edge_points(*name, *orientation != 2, vectors);
                if points.len() < 2 {
                    continue;
                }
                let flat: Vec<[f64; 2]> = points.iter().map(|p| [p[0], p[1]]).collect();
                // Edges that meet are one line, so a contour is drawn as one
                // stroke rather than as a row of butting segments.
                match lines.last_mut() {
                    Some(last) if joins(last, &flat) => last.extend(flat.into_iter().skip(1)),
                    _ => lines.push(flat),
                }
            }
            (!lines.is_empty()).then_some(Geometry::Lines(lines))
        }
        // Area.
        3 => {
            let mut rings: Vec<Vec<[f64; 2]>> = Vec::new();
            let mut current: Vec<[f64; 2]> = Vec::new();
            for (name, orientation, _) in pointers {
                if name.0 != EDGE {
                    continue;
                }
                let points = edge_points(*name, *orientation != 2, vectors);
                if points.len() < 2 {
                    continue;
                }
                let flat = points.iter().map(|p| [p[0], p[1]]);
                if current.is_empty() {
                    current.extend(flat);
                } else {
                    current.extend(flat.skip(1));
                }
                if closed(&current) {
                    rings.push(std::mem::take(&mut current));
                }
            }
            if current.len() >= 3 {
                rings.push(current);
            }
            rings.retain(|ring| ring.len() >= 3);
            // The first ring is the outside and the rest are holes, which is
            // how S-57 orders them; the painter fills even-odd either way.
            (!rings.is_empty()).then_some(Geometry::Areas(vec![rings]))
        }
        _ => None,
    }
}

/// Whether a ring's ends meet, to a ten-thousandth of a minute of arc.
fn closed(ring: &[[f64; 2]]) -> bool {
    match (ring.first(), ring.last()) {
        (Some(first), Some(last)) => {
            ring.len() >= 3
                && (first[0] - last[0]).abs() < 1e-9
                && (first[1] - last[1]).abs() < 1e-9
        }
        _ => false,
    }
}

/// Whether the next run continues where the last left off.
fn joins(last: &[[f64; 2]], next: &[[f64; 2]]) -> bool {
    match (last.last(), next.first()) {
        (Some(end), Some(start)) => {
            (end[0] - start[0]).abs() < 1e-9 && (end[1] - start[1]).abs() < 1e-9
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pointer_names_a_record_kind_and_number() {
        assert_eq!(name_of(&[130, 0x2a, 0x01, 0, 0]), Some((130, 298)));
        assert_eq!(name_of(&[110, 1]), None, "too short to be a name");
    }

    #[test]
    fn an_attribute_that_reads_as_a_number_is_one() {
        assert_eq!(parse_value("12"), Value::Int(12));
        assert_eq!(parse_value("-3.5"), Value::Real(-3.5));
        assert_eq!(parse_value("Chesapeake"), Value::Text("Chesapeake".into()));
        // An enumeration list stays text: it is not one number.
        assert_eq!(parse_value("2,6"), Value::Text("2,6".into()));
    }

    /// Two edges, each stated in its own direction, making one square. The
    /// walk has to reverse the second and drop the repeated corner.
    #[test]
    fn an_area_is_walked_from_its_edges_whichever_way_each_is_stated() {
        let node = |lon: f64, lat: f64| Vector {
            points: vec![[lon, lat, f64::NAN]],
            ..Vector::default()
        };
        let mut vectors = HashMap::new();
        vectors.insert((CONNECTED_NODE, 1), node(0.0, 0.0));
        vectors.insert((CONNECTED_NODE, 2), node(1.0, 1.0));
        vectors.insert(
            (EDGE, 10),
            Vector {
                points: vec![[1.0, 0.0, f64::NAN]],
                begin: Some((CONNECTED_NODE, 1)),
                end: Some((CONNECTED_NODE, 2)),
            },
        );
        // Stated from the far corner back, so the feature points at it in
        // reverse: without honouring that, the ring crosses itself.
        vectors.insert(
            (EDGE, 11),
            Vector {
                points: vec![[0.0, 1.0, f64::NAN]],
                begin: Some((CONNECTED_NODE, 1)),
                end: Some((CONNECTED_NODE, 2)),
            },
        );
        let geometry =
            assemble(3, &[((EDGE, 10), 1, 1), ((EDGE, 11), 2, 1)], &vectors).expect("an area");
        let Geometry::Areas(areas) = geometry else {
            panic!("not an area");
        };
        assert_eq!(
            areas,
            vec![vec![vec![
                [0.0, 0.0],
                [1.0, 0.0],
                [1.0, 1.0],
                [0.0, 1.0],
                [0.0, 0.0],
            ]]],
            "one closed ring, corners in order and stated once each"
        );
    }

    #[test]
    fn edges_that_meet_make_one_line() {
        let mut vectors = HashMap::new();
        vectors.insert(
            (EDGE, 1),
            Vector {
                points: vec![[0.0, 0.0, f64::NAN], [1.0, 0.0, f64::NAN]],
                ..Vector::default()
            },
        );
        vectors.insert(
            (EDGE, 2),
            Vector {
                points: vec![[1.0, 0.0, f64::NAN], [2.0, 0.0, f64::NAN]],
                ..Vector::default()
            },
        );
        vectors.insert(
            (EDGE, 3),
            Vector {
                points: vec![[9.0, 9.0, f64::NAN], [9.0, 8.0, f64::NAN]],
                ..Vector::default()
            },
        );
        let Some(Geometry::Lines(lines)) = assemble(
            2,
            &[((EDGE, 1), 1, 1), ((EDGE, 2), 1, 1), ((EDGE, 3), 1, 1)],
            &vectors,
        ) else {
            panic!("not lines");
        };
        assert_eq!(lines.len(), 2, "two runs: one joined pair and one apart");
        assert_eq!(lines[0].len(), 3, "the shared point is stated once");
    }

    #[test]
    fn a_feature_pointing_at_nothing_is_dropped_rather_than_drawn_empty() {
        assert!(assemble(3, &[((EDGE, 404), 1, 1)], &HashMap::new()).is_none());
        assert!(assemble(255, &[], &HashMap::new()).is_none());
    }
}
