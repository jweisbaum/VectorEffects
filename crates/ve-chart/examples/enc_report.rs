//! What a directory of S-57 cells holds, read by this crate.
//!
//! The tool the object catalogue was built with, and the one that checks it
//! against a real chart set after a change:
//!
//! ```bash
//! cargo run -p ve-chart --release --example enc_report -- ~/Downloads/ENC_ROOT
//! ```
//!
//! It prints each class by name with its geometry and the attributes it
//! carries, so a class listed in `catalogue.rs` under the wrong number shows
//! up as an area class full of depths called something that is not DEPARE.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "ENC_ROOT".to_owned()),
    );
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--tile") {
        let number = |offset: usize, fallback: f64| {
            args.get(at + offset)
                .and_then(|a| a.parse().ok())
                .unwrap_or(fallback)
        };
        write_tile(
            &root,
            number(1, -76.4),
            number(2, 38.9),
            number(3, 0.5),
            700,
        );
        return;
    }
    let mut cells = Vec::new();
    collect(&root, &mut cells);
    cells.sort();
    if cells.is_empty() {
        eprintln!("no .000 cells under {}", root.display());
        return;
    }

    /// Per class: how many, how many of each geometry, and one example of
    /// every attribute it was seen carrying.
    type Seen = (usize, [usize; 3], BTreeMap<String, String>);
    let mut classes: BTreeMap<String, Seen> = BTreeMap::new();
    let mut read = 0;
    let mut failed = 0;
    let mut bounds = ve_chart::Bounds::EMPTY;
    let started = std::time::Instant::now();
    for path in &cells {
        match ve_chart::read_cell(path) {
            Ok(cell) => {
                read += 1;
                bounds.merge(cell.bounds);
                for feature in &cell.features {
                    let entry = classes.entry(feature.class.clone()).or_default();
                    entry.0 += 1;
                    let kind = match feature.geometry {
                        ve_chart::Geometry::Points(_) => 0,
                        ve_chart::Geometry::Lines(_) => 1,
                        ve_chart::Geometry::Areas(_) => 2,
                    };
                    entry.1[kind] += 1;
                    for (name, value) in &feature.attributes {
                        entry
                            .2
                            .entry(name.clone())
                            .or_insert_with(|| format!("{value:?}"));
                    }
                }
            }
            Err(err) => {
                failed += 1;
                eprintln!("{}: {err}", path.display());
            }
        }
    }
    println!(
        "{read} cells read, {failed} refused, in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    println!(
        "covering {:.3}..{:.3} east, {:.3}..{:.3} north",
        bounds.west, bounds.east, bounds.south, bounds.north
    );
    println!(
        "{:<10} {:>8}  {:<18} ATTRIBUTES",
        "CLASS", "COUNT", "POINT/LINE/AREA"
    );
    let mut rows: Vec<_> = classes.into_iter().collect();
    rows.sort_by_key(|(_, (count, _, _))| std::cmp::Reverse(*count));
    for (class, (count, kinds, attributes)) in rows.iter().take(60) {
        let names: Vec<&str> = attributes.keys().map(String::as_str).take(8).collect();
        println!(
            "{class:<10} {count:>8}  {:>5}/{:>5}/{:>6}  {}",
            kinds[0],
            kinds[1],
            kinds[2],
            names.join(" ")
        );
    }
}

fn collect(at: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "000") {
            out.push(path);
        }
    }
}

/// `--tile <west> <south> <degrees>` also writes that tile as a BMP, so the
/// chart can be looked at rather than counted.
fn write_tile(root: &std::path::Path, west: f64, south: f64, degrees: f64, size: u32) {
    use ve_chart::paint::TileFrame;
    use ve_chart::s57::{Library, Palette, draw_tile};
    let library = match Library::open(root) {
        Ok(library) => library,
        Err(err) => return eprintln!("{err}"),
    };
    let frame = TileFrame {
        bounds: ve_chart::Bounds {
            west,
            south,
            east: west + degrees,
            north: south + degrees,
        },
        size,
    };
    let Some(rgba) = draw_tile(&library, frame, &Palette::default(), 24) else {
        return eprintln!("no chart at {west},{south}");
    };
    let row = (size as usize * 3).next_multiple_of(4);
    let mut bmp = vec![0_u8; 54 + row * size as usize];
    bmp[..2].copy_from_slice(b"BM");
    let put = |bmp: &mut [u8], at: usize, value: u32| {
        bmp[at..at + 4].copy_from_slice(&value.to_le_bytes());
    };
    let total = bmp.len() as u32;
    put(&mut bmp, 2, total);
    put(&mut bmp, 10, 54);
    put(&mut bmp, 14, 40);
    put(&mut bmp, 18, size);
    put(&mut bmp, 22, (-(size as i32)) as u32);
    bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
    for y in 0..size as usize {
        for x in 0..size as usize {
            let from = (y * size as usize + x) * 4;
            let (r, g, b, a) = (
                f32::from(rgba[from]),
                f32::from(rgba[from + 1]),
                f32::from(rgba[from + 2]),
                f32::from(rgba[from + 3]) / 255.0,
            );
            // Over the map's own sea, so the chart is seen as it is drawn.
            let sea = [7.0, 12.0, 24.0];
            let at = 54 + y * row + x * 3;
            for (channel, (value, under)) in [b, g, r]
                .into_iter()
                .zip([sea[2], sea[1], sea[0]])
                .enumerate()
            {
                bmp[at + channel] = (value * a + under * (1.0 - a)) as u8;
            }
        }
    }
    let out = std::env::temp_dir().join("ve-chart-tile.bmp");
    if let Err(err) = std::fs::write(&out, bmp) {
        eprintln!("{err}");
    } else {
        println!("wrote {}", out.display());
    }
}
