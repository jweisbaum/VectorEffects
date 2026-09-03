//! Converts Natural Earth land polygons into the compact basemap binary.
//!
//! This is a build tool, not part of the shipped application. It runs once; its
//! output is committed to `assets/basemap.bin` so the app needs no network and
//! no GeoJSON parsing at runtime (invariant 5).
//!
//! Source data (public domain):
//!   https://github.com/nvkelso/natural-earth-vector/tree/master/geojson
//!   ne_110m_land.geojson, ne_50m_land.geojson
//!
//! Unlike the library crates, this tool panics on bad input rather than
//! threading errors upward: it is a developer-run converter, and a clear panic
//! naming the offending file is the right failure mode for it.
//!
//! Usage:
//!   cargo run -p basemap-builder -- <110m.geojson> <50m.geojson> <out.bin>
//!
//! # Antimeridian handling
//!
//! Longitudes are *unwrapped* per ring: when consecutive points jump by more
//! than 180 degrees the ring is continued past ±180 rather than snapping back.
//! A landmass straddling the dateline therefore stays one continuous polygon
//! (Chukotka runs 170..190 rather than splitting into a 170..180 piece and a
//! -180..-170 piece). The renderer draws the world three times, at longitude
//! offsets -360, 0 and +360, so the part past the edge appears where it should.
//! Splitting rings at the dateline instead would need exact edge intersections
//! and produce seams; unwrapping needs neither.
//!
//! **Circumnavigating rings are the exception.** Antarctica runs along the coast
//! to longitude 180, drops to latitude -90, crosses the pole edge to -180, and
//! returns. That 360-degree step is a real part of the shape, not a dateline
//! artefact, and unwrapping it leaves the ring 360 degrees from where it began
//! so it can no longer close -- which draws a spurious line across the whole
//! map and wrecks the triangulation. Such a ring is detected by the fact that
//! unwrapping stops it closing, and its raw coordinates are used instead. Raw
//! is already correct there: the pole-edge step renders as a straight line
//! along the bottom of an equirectangular map, which is exactly right.

#![allow(
    clippy::expect_used,
    reason = "build tool: a panic naming the bad input is the right failure"
)]

use std::io::Write;

/// File magic.
const MAGIC: &[u8; 4] = b"VEBM";
/// Format version.
const VERSION: u32 = 1;

/// One level of detail.
struct Lod {
    /// Natural Earth scale marker: 110 or 50.
    marker: u32,
    /// Triangle vertices as (lon, lat) pairs.
    tri_vertices: Vec<[f32; 2]>,
    /// Triangle indices into `tri_vertices`.
    tri_indices: Vec<u32>,
    /// Coastline vertices as (lon, lat) pairs.
    line_vertices: Vec<[f32; 2]>,
    /// `(offset, length)` into `line_vertices`, one per closed ring.
    line_strips: Vec<(u32, u32)>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [a, b, out] = args.as_slice() else {
        eprintln!("usage: basemap-builder <110m.geojson> <50m.geojson> <out.bin>");
        std::process::exit(2);
    };

    let lods = vec![build_lod(a, 110), build_lod(b, 50)];
    for lod in &lods {
        println!(
            "  {}m: {} triangles, {} vertices, {} coastline rings",
            lod.marker,
            lod.tri_indices.len() / 3,
            lod.tri_vertices.len(),
            lod.line_strips.len()
        );
    }

    match write_binary(&lods, out) {
        Ok(bytes) => println!("wrote {out} ({bytes} bytes)"),
        Err(err) => {
            eprintln!("failed to write {out}: {err}");
            std::process::exit(1);
        }
    }
}

fn build_lod(path: &str, marker: u32) -> Lod {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let json: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path} is not json: {e}"));

    let mut lod = Lod {
        marker,
        tri_vertices: Vec::new(),
        tri_indices: Vec::new(),
        line_vertices: Vec::new(),
        line_strips: Vec::new(),
    };

    let features = json["features"].as_array().expect("no features array");
    for feature in features {
        let geometry = &feature["geometry"];
        match geometry["type"].as_str().unwrap_or_default() {
            "Polygon" => add_polygon(&mut lod, &geometry["coordinates"]),
            "MultiPolygon" => {
                for polygon in geometry["coordinates"].as_array().into_iter().flatten() {
                    add_polygon(&mut lod, polygon);
                }
            }
            other => eprintln!("skipping unsupported geometry {other:?}"),
        }
    }
    lod
}

/// Reads one ring, unwrapping longitude so it stays continuous across ±180.
///
/// Falls back to raw coordinates for rings that circumnavigate the globe; see
/// the antimeridian note in the module docs.
fn read_ring(value: &serde_json::Value) -> Vec<[f64; 2]> {
    let mut raw: Vec<[f64; 2]> = Vec::new();
    for point in value.as_array().into_iter().flatten() {
        let coords = point.as_array().map(Vec::as_slice).unwrap_or_default();
        let (Some(lon), Some(lat)) = (
            coords.first().and_then(|v| v.as_f64()),
            coords.get(1).and_then(|v| v.as_f64()),
        ) else {
            continue;
        };
        raw.push([lon, lat]);
    }
    if raw.is_empty() {
        return raw;
    }

    let mut unwrapped: Vec<[f64; 2]> = Vec::with_capacity(raw.len());
    let mut previous: Option<f64> = None;
    for point in &raw {
        let lon = match previous {
            None => point[0],
            Some(prev) => {
                // A jump larger than half the globe is usually the ring
                // crossing the dateline, not a genuine leap.
                let mut candidate = point[0];
                while candidate - prev > 180.0 {
                    candidate -= 360.0;
                }
                while prev - candidate > 180.0 {
                    candidate += 360.0;
                }
                candidate
            }
        };
        previous = Some(lon);
        unwrapped.push([lon, point[1]]);
    }

    // A GeoJSON ring repeats its first point last. If unwrapping has moved that
    // repeat away from where it started, the ring goes all the way round the
    // globe and the "jump" it removed was real geometry. Keep the raw form.
    let closes = match (unwrapped.first(), unwrapped.last()) {
        (Some(first), Some(last)) => (last[0] - first[0]).abs() < 1e-6,
        _ => true,
    };
    let mut out = if closes { unwrapped } else { raw };

    // Triangulation does not want the repeated closing point.
    if out.len() > 1 && out[0] == out[out.len() - 1] {
        out.pop();
    }
    out
}

fn add_polygon(lod: &mut Lod, polygon: &serde_json::Value) {
    let rings: Vec<Vec<[f64; 2]>> = polygon
        .as_array()
        .into_iter()
        .flatten()
        .map(read_ring)
        .collect();
    let Some(outer) = rings.first() else { return };
    if outer.len() < 3 {
        return;
    }

    // Coastlines are drawn from every ring, holes included: the shore of an
    // inland sea is a coastline too.
    for ring in &rings {
        if ring.len() < 2 {
            continue;
        }
        let offset = u32::try_from(lod.line_vertices.len()).expect("vertex count fits u32");
        for point in ring {
            lod.line_vertices.push([point[0] as f32, point[1] as f32]);
        }
        let len = u32::try_from(ring.len()).expect("ring length fits u32");
        lod.line_strips.push((offset, len));
    }

    // Flatten for earcut: outer ring first, then each hole, with hole start
    // indices given separately.
    let mut flat: Vec<f64> = Vec::new();
    let mut holes: Vec<usize> = Vec::new();
    for (i, ring) in rings.iter().enumerate() {
        if i > 0 {
            if ring.len() < 3 {
                continue;
            }
            holes.push(flat.len() / 2);
        }
        for point in ring {
            flat.push(point[0]);
            flat.push(point[1]);
        }
    }

    let Ok(indices) = earcutr::earcut(&flat, &holes, 2) else {
        eprintln!("triangulation failed for a ring of {} points", outer.len());
        return;
    };

    let base = u32::try_from(lod.tri_vertices.len()).expect("vertex count fits u32");
    for pair in flat.chunks_exact(2) {
        lod.tri_vertices.push([pair[0] as f32, pair[1] as f32]);
    }
    for index in indices {
        lod.tri_indices
            .push(base + u32::try_from(index).expect("index fits u32"));
    }
}

fn write_binary(lods: &[Lod], path: &str) -> std::io::Result<usize> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&u32::try_from(lods.len()).unwrap_or(0).to_le_bytes());

    for lod in lods {
        out.extend_from_slice(&lod.marker.to_le_bytes());
        out.extend_from_slice(&(lod.tri_vertices.len() as u32).to_le_bytes());
        out.extend_from_slice(&(lod.tri_indices.len() as u32).to_le_bytes());
        out.extend_from_slice(&(lod.line_vertices.len() as u32).to_le_bytes());
        out.extend_from_slice(&(lod.line_strips.len() as u32).to_le_bytes());

        for v in &lod.tri_vertices {
            out.extend_from_slice(&v[0].to_le_bytes());
            out.extend_from_slice(&v[1].to_le_bytes());
        }
        for i in &lod.tri_indices {
            out.extend_from_slice(&i.to_le_bytes());
        }
        for v in &lod.line_vertices {
            out.extend_from_slice(&v[0].to_le_bytes());
            out.extend_from_slice(&v[1].to_le_bytes());
        }
        for (offset, len) in &lod.line_strips {
            out.extend_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
        }
    }

    std::fs::File::create(path)?.write_all(&out)?;
    Ok(out.len())
}
