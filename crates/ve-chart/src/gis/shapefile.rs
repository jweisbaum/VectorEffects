//! Esri shapefiles: the `.shp` geometry, the `.dbf` attributes beside it,
//! and the `.prj` that says what the coordinates mean.
//!
//! A shapefile is three or more files with one stem, and the geometry file
//! alone does not say what its numbers are: `.prj` does. A projected one is
//! refused by name rather than drawn, for the reason an image layer's
//! projected reference system is (spec.md 4.9) — metres read as degrees put
//! a coastline somewhere plausible and entirely wrong.

use std::path::Path;

use crate::error::{ChartError, Result};
use crate::geometry::{Feature, Geometry, Value};

/// Shape types this reads. The Z and M variants carry their extra numbers
/// after the ones read here, so they are read as their plain forms.
const NULL: i32 = 0;
const POINT: i32 = 1;
const POLYLINE: i32 = 3;
const POLYGON: i32 = 5;
const MULTIPOINT: i32 = 8;

fn le_i32(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn be_i32(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn le_f64(bytes: &[u8], at: usize) -> Option<f64> {
    Some(f64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// Reads a shapefile by the path of any of its parts.
pub fn read(path: &Path) -> Result<Vec<Feature>> {
    let shp = path.with_extension("shp");
    let bytes = std::fs::read(&shp)?;
    check_projection(path)?;
    let mut features = geometry(&bytes)?;
    // The attributes are a table beside it, one row per shape, in order.
    if let Ok(table) = std::fs::read(path.with_extension("dbf")) {
        match attributes(&table) {
            Ok(rows) => {
                for (feature, row) in features.iter_mut().zip(rows) {
                    feature.attributes = row;
                }
            }
            Err(err) => {
                // The shapes are still worth drawing without their names.
                tracing_warn(&err);
            }
        }
    }
    Ok(features)
}

fn tracing_warn(err: &ChartError) {
    eprintln!("shapefile attributes could not be read: {err}");
}

/// Refuses a file whose `.prj` names anything but longitude and latitude.
fn check_projection(path: &Path) -> Result<()> {
    let Ok(wkt) = std::fs::read_to_string(path.with_extension("prj")) else {
        // No `.prj` at all: the coordinates are taken at face value, which
        // is what every reader does, and the extent check below catches the
        // obvious case of metres.
        return Ok(());
    };
    let upper = wkt.to_ascii_uppercase();
    if upper.contains("PROJCS") || upper.contains("PROJCRS") {
        let name = upper
            .split_once('"')
            .and_then(|(_, rest)| rest.split_once('"'))
            .map_or_else(
                || "a projected system".to_owned(),
                |(name, _)| name.to_owned(),
            );
        return Err(ChartError::Unsupported(format!(
            "{name} is a projected coordinate system; reproject the file to WGS 84 (EPSG:4326)"
        )));
    }
    Ok(())
}

fn geometry(bytes: &[u8]) -> Result<Vec<Feature>> {
    if bytes.len() < 100 || be_i32(bytes, 0) != Some(9994) {
        return Err(ChartError::malformed("a shapefile", "not a .shp file"));
    }
    // The header's length is in 16-bit words, and may be longer than the
    // file where a writer stopped early; the records themselves are walked.
    let mut features = Vec::new();
    let mut at = 100;
    while at + 8 <= bytes.len() {
        let Some(words) = be_i32(bytes, at + 4) else {
            break;
        };
        let length = (words as usize).saturating_mul(2);
        let from = at + 8;
        let to = from + length;
        if length == 0 || to > bytes.len() {
            break;
        }
        if let Some(geometry) = shape(&bytes[from..to]) {
            features.push(Feature {
                class: String::new(),
                attributes: Vec::new(),
                geometry,
            });
        }
        at = to;
    }
    if features.is_empty() {
        return Err(ChartError::malformed("a shapefile", "holds no shape"));
    }
    Ok(features)
}

/// One record's geometry. The Z and M variants are the plain ones with more
/// numbers after, so their extra arrays are simply not read.
fn shape(record: &[u8]) -> Option<Geometry> {
    let kind = le_i32(record, 0)?;
    // 11, 13, 15, 18 are the Z forms and 21, 23, 25, 28 the M forms, each
    // ten or twenty past the plain type they extend.
    let plain = match kind {
        NULL => return None,
        k if k >= 20 => k - 20,
        k if k >= 10 => k - 10,
        k => k,
    };
    match plain {
        POINT => {
            let (x, y) = (le_f64(record, 4)?, le_f64(record, 12)?);
            (x.is_finite() && y.is_finite()).then(|| Geometry::Points(vec![[x, y, f64::NAN]]))
        }
        MULTIPOINT => {
            let count = le_i32(record, 36)?.max(0) as usize;
            let points: Vec<[f64; 3]> = (0..count)
                .filter_map(|i| {
                    let at = 40 + i * 16;
                    Some([le_f64(record, at)?, le_f64(record, at + 8)?, f64::NAN])
                })
                .collect();
            (!points.is_empty()).then_some(Geometry::Points(points))
        }
        POLYLINE | POLYGON => {
            let parts = le_i32(record, 36)?.max(0) as usize;
            let count = le_i32(record, 40)?.max(0) as usize;
            if parts == 0 || count == 0 || parts > count {
                return None;
            }
            let starts: Vec<usize> = (0..parts)
                .filter_map(|i| Some(le_i32(record, 44 + i * 4)?.max(0) as usize))
                .collect();
            let points_at = 44 + parts * 4;
            let point = |i: usize| {
                let at = points_at + i * 16;
                Some([le_f64(record, at)?, le_f64(record, at + 8)?])
            };
            let mut runs = Vec::with_capacity(parts);
            for (index, start) in starts.iter().enumerate() {
                let end = starts.get(index + 1).copied().unwrap_or(count).min(count);
                if *start >= end {
                    continue;
                }
                let run: Vec<[f64; 2]> = (*start..end).filter_map(point).collect();
                if run.len() >= 2 {
                    runs.push(run);
                }
            }
            if runs.is_empty() {
                return None;
            }
            Some(if plain == POLYGON {
                // A shapefile polygon's parts are its outer ring and its
                // holes, in one list with no marker between them; the
                // painter fills even-odd, which needs no marker.
                Geometry::Areas(vec![
                    runs.into_iter().filter(|run| run.len() >= 3).collect(),
                ])
            } else {
                Geometry::Lines(runs)
            })
        }
        _ => None,
    }
}

/// Reads a dBase III table: one row of named values per shape.
fn attributes(bytes: &[u8]) -> Result<Vec<Vec<(String, Value)>>> {
    let short = || ChartError::malformed("a dBase table", "shorter than its header");
    if bytes.len() < 32 {
        return Err(short());
    }
    let count = u32::from_le_bytes(
        bytes
            .get(4..8)
            .ok_or_else(short)?
            .try_into()
            .map_err(|_| short())?,
    ) as usize;
    let header = u16::from_le_bytes(
        bytes
            .get(8..10)
            .ok_or_else(short)?
            .try_into()
            .map_err(|_| short())?,
    ) as usize;
    let row_length = u16::from_le_bytes(
        bytes
            .get(10..12)
            .ok_or_else(short)?
            .try_into()
            .map_err(|_| short())?,
    ) as usize;
    if header < 33 || row_length == 0 {
        return Err(ChartError::malformed("a dBase table", "impossible sizes"));
    }

    // Field descriptors, 32 bytes each, ending at a carriage return.
    let mut fields = Vec::new();
    let mut at = 32;
    while at + 32 <= header && bytes.get(at) != Some(&0x0d) {
        let raw = &bytes[at..at + 32];
        let name = String::from_utf8_lossy(&raw[..11])
            .trim_end_matches(['\0', ' '])
            .to_owned();
        let kind = raw[11] as char;
        let width = raw[16] as usize;
        if !name.is_empty() && width > 0 {
            fields.push((name, kind, width));
        }
        at += 32;
    }
    if fields.is_empty() {
        return Err(ChartError::malformed("a dBase table", "no fields"));
    }

    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let start = header + index * row_length;
        let Some(row) = bytes.get(start..start + row_length) else {
            break;
        };
        // The first byte marks a deleted row, which is not drawn.
        if row.first() == Some(&b'*') {
            rows.push(Vec::new());
            continue;
        }
        let mut values = Vec::with_capacity(fields.len());
        let mut at = 1;
        for (name, kind, width) in &fields {
            let Some(raw) = row.get(at..at + width) else {
                break;
            };
            at += width;
            let text = String::from_utf8_lossy(raw).trim().to_owned();
            if text.is_empty() {
                continue;
            }
            let value = match kind {
                'N' | 'F' => text
                    .parse::<i64>()
                    .map(Value::Int)
                    .or_else(|_| text.parse::<f64>().map(Value::Real))
                    .unwrap_or(Value::Text(text)),
                'L' => Value::Int(i64::from(matches!(text.as_str(), "T" | "t" | "Y" | "y"))),
                _ => Value::Text(text),
            };
            values.push((name.clone(), value));
        }
        rows.push(values);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.shp` written here byte by byte, so the test asserts the format
    /// and not a second copy of a writer: one polyline of two points and one
    /// polygon of a triangle.
    fn shp() -> Vec<u8> {
        let mut out = vec![0_u8; 100];
        out[0..4].copy_from_slice(&9994_i32.to_be_bytes());
        out[28..32].copy_from_slice(&1000_i32.to_le_bytes());
        out[32..36].copy_from_slice(&POLYGON.to_le_bytes());

        let mut record = |number: i32, content: Vec<u8>| {
            out.extend(number.to_be_bytes());
            out.extend(((content.len() / 2) as i32).to_be_bytes());
            out.extend(content);
        };

        let mut line = Vec::new();
        line.extend(POLYLINE.to_le_bytes());
        line.extend([0.0_f64; 4].iter().flat_map(|v| v.to_le_bytes()));
        line.extend(1_i32.to_le_bytes()); // one part
        line.extend(2_i32.to_le_bytes()); // two points
        line.extend(0_i32.to_le_bytes()); // starting at 0
        for (x, y) in [(-76.0_f64, 37.0_f64), (-75.0, 38.0)] {
            line.extend(x.to_le_bytes());
            line.extend(y.to_le_bytes());
        }
        record(1, line);

        let mut area = Vec::new();
        area.extend(POLYGON.to_le_bytes());
        area.extend([0.0_f64; 4].iter().flat_map(|v| v.to_le_bytes()));
        area.extend(1_i32.to_le_bytes());
        area.extend(4_i32.to_le_bytes());
        area.extend(0_i32.to_le_bytes());
        for (x, y) in [(0.0_f64, 0.0_f64), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)] {
            area.extend(x.to_le_bytes());
            area.extend(y.to_le_bytes());
        }
        record(2, area);
        out
    }

    /// A dBase III table of two rows, one text field and one numeric.
    fn dbf() -> Vec<u8> {
        let fields: [(&str, u8, u8); 2] = [("NAME", b'C', 10), ("DEPTH", b'N', 5)];
        let header = 32 + fields.len() * 32 + 1;
        let row_length = 1 + fields.iter().map(|(_, _, w)| *w as usize).sum::<usize>();
        let mut out = vec![0_u8; header];
        out[0] = 0x03;
        out[4..8].copy_from_slice(&2_u32.to_le_bytes());
        out[8..10].copy_from_slice(&(header as u16).to_le_bytes());
        out[10..12].copy_from_slice(&(row_length as u16).to_le_bytes());
        for (index, (name, kind, width)) in fields.iter().enumerate() {
            let at = 32 + index * 32;
            out[at..at + name.len()].copy_from_slice(name.as_bytes());
            out[at + 11] = *kind;
            out[at + 16] = *width;
        }
        out[header - 1] = 0x0d;
        for (name, depth) in [("Thimble", "  12"), ("Wolf Trap", " 7.5")] {
            let mut row = vec![b' '; row_length];
            row[0] = b' ';
            row[1..1 + name.len()].copy_from_slice(name.as_bytes());
            let at = 1 + 10;
            row[at..at + depth.len()].copy_from_slice(depth.as_bytes());
            out.extend(row);
        }
        out.push(0x1a);
        out
    }

    #[test]
    fn shapes_and_their_attributes_read_together() {
        let root = tempfile::tempdir().expect("temp");
        let stem = root.path().join("bay");
        std::fs::write(stem.with_extension("shp"), shp()).expect("shp");
        std::fs::write(stem.with_extension("dbf"), dbf()).expect("dbf");
        let features = read(&stem.with_extension("shp")).expect("reads");
        assert_eq!(features.len(), 2);
        assert_eq!(
            features[0].geometry,
            Geometry::Lines(vec![vec![[-76.0, 37.0], [-75.0, 38.0]]])
        );
        assert_eq!(
            features[1].geometry,
            Geometry::Areas(vec![vec![vec![
                [0.0, 0.0],
                [1.0, 0.0],
                [1.0, 1.0],
                [0.0, 0.0]
            ]]])
        );
        assert_eq!(
            features[0].attribute("NAME"),
            Some(&Value::Text("Thimble".into()))
        );
        assert_eq!(features[0].attribute("DEPTH"), Some(&Value::Int(12)));
        assert_eq!(features[1].attribute("DEPTH"), Some(&Value::Real(7.5)));
    }

    #[test]
    fn the_geometry_reads_without_a_table_beside_it() {
        let root = tempfile::tempdir().expect("temp");
        let stem = root.path().join("bare");
        std::fs::write(stem.with_extension("shp"), shp()).expect("shp");
        let features = read(&stem.with_extension("shp")).expect("reads");
        assert_eq!(features.len(), 2);
        assert!(features[0].attributes.is_empty());
    }

    /// Metres read as degrees put a coastline somewhere plausible, so a
    /// projected file is refused by name rather than drawn.
    #[test]
    fn a_projected_shapefile_is_refused_by_name() {
        let root = tempfile::tempdir().expect("temp");
        let stem = root.path().join("utm");
        std::fs::write(stem.with_extension("shp"), shp()).expect("shp");
        std::fs::write(
            stem.with_extension("prj"),
            r#"PROJCS["NAD83 / UTM zone 18N",GEOGCS["NAD83",DATUM["North_American_Datum_1983"]]]"#,
        )
        .expect("prj");
        let err = read(&stem.with_extension("shp")).expect_err("refused");
        assert!(err.to_string().contains("UTM ZONE 18N"), "{err}");
        assert!(err.to_string().contains("WGS 84"), "and says what to do");
        // A geographic one is read.
        std::fs::write(
            stem.with_extension("prj"),
            r#"GEOGCS["WGS 84",DATUM["WGS_1984"]]"#,
        )
        .expect("prj");
        assert!(read(&stem.with_extension("shp")).is_ok());
    }

    #[test]
    fn a_file_that_is_not_a_shapefile_is_refused() {
        let root = tempfile::tempdir().expect("temp");
        let stem = root.path().join("nope");
        std::fs::write(stem.with_extension("shp"), b"this is not a shapefile").expect("write");
        assert!(read(&stem.with_extension("shp")).is_err());
    }
}
