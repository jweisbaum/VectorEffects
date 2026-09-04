//! Build-time converter: DWD `CLAT`/`CLON` GRIB messages to
//! `assets/icon_grids.bin`.
//!
//! A GRIB message on an unstructured grid names its grid by a UUID and says
//! nothing about where its cells are (template 3.101). DWD publishes the
//! positions as two more GRIB messages on that same grid, one of latitudes
//! and one of longitudes. This turns a pair of them into the compact form the
//! app embeds, so an ICON import needs no files but the forecast itself and
//! reaches no network (invariant 5).
//!
//! ```text
//! cargo run -p icon-grid-builder --release -- \
//!     --out assets/icon_grids.bin \
//!     --grid "ICON global R03B07" clat.grib2 clon.grib2 \
//!     --grid "ICON global EPS R02B06" eps_clat.grib2 eps_clon.grib2
//! ```
//!
//! The coordinates are stored on their own quantisation lattice — the one the
//! source messages were packed on, found by inspection and checked — as
//! varint deltas, deflated. That lattice is the whole reason the asset is
//! small: the values are already only as precise as 16-bit packing made them,
//! and ICON lists its cells in a space-filling order, so a delta is usually a
//! byte.

use std::io::Write;
use std::path::{Path, PathBuf};

use ve_grib::decode::{self, Grid};

fn main() {
    if let Err(message) = run() {
        eprintln!("icon-grid-builder: {message}");
        std::process::exit(1);
    }
}

struct Request {
    name: String,
    clat: PathBuf,
    clon: PathBuf,
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut out = PathBuf::from("assets/icon_grids.bin");
    let mut requests = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                out = PathBuf::from(args.next().ok_or("--out needs a path")?);
            }
            "--grid" => {
                let name = args.next().ok_or("--grid needs a name")?;
                let clat = PathBuf::from(args.next().ok_or("--grid needs a CLAT file")?);
                let clon = PathBuf::from(args.next().ok_or("--grid needs a CLON file")?);
                requests.push(Request { name, clat, clon });
            }
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    if requests.is_empty() {
        return Err("no --grid given".to_owned());
    }

    let mut asset = Vec::new();
    asset.extend_from_slice(ve_grib::icon::MAGIC);
    asset.extend_from_slice(&(requests.len() as u32).to_le_bytes());

    for request in &requests {
        let (uuid, lat) = coordinates(&request.clat)?;
        let (uuid2, lon) = coordinates(&request.clon)?;
        if uuid != uuid2 {
            return Err(format!(
                "{}: CLAT is on grid {} but CLON is on {}",
                request.name,
                hex(&uuid),
                hex(&uuid2)
            ));
        }
        if lat.len() != lon.len() {
            return Err(format!(
                "{}: {} latitudes against {} longitudes",
                request.name,
                lat.len(),
                lon.len()
            ));
        }

        let payload = pack(&lat, &lon)?;
        println!(
            "{}: {} cells, grid {}, {} bytes",
            request.name,
            lat.len(),
            hex(&uuid),
            payload.len()
        );

        asset.extend_from_slice(&uuid);
        asset.extend_from_slice(&(lat.len() as u32).to_le_bytes());
        asset.extend_from_slice(&(request.name.len() as u32).to_le_bytes());
        asset.extend_from_slice(request.name.as_bytes());
        asset.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        asset.extend_from_slice(&payload);
    }

    std::fs::write(&out, &asset).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote {} ({} bytes)", out.display(), asset.len());
    Ok(())
}

/// Reads one coordinate file: its grid UUID and its values.
fn coordinates(path: &Path) -> Result<([u8; 16], Vec<f32>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let decoded = decode::read_all(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let message = decoded
        .messages
        .into_iter()
        .next()
        .ok_or_else(|| format!("{}: no message decoded", path.display()))?;
    let Grid::Unstructured(grid) = message.header.grid else {
        return Err(format!(
            "{}: this is a lat/lon file, not an unstructured grid definition",
            path.display()
        ));
    };
    Ok((grid.uuid, message.values))
}

/// Packs the two coordinate series onto their own lattices.
fn pack(lat: &[f32], lon: &[f32]) -> Result<Vec<u8>, String> {
    let (lat_min, lat_step) = lattice(lat)?;
    let (lon_min, lon_step) = lattice(lon)?;

    let mut raw = Vec::new();
    raw.extend_from_slice(&lat_min.to_le_bytes());
    raw.extend_from_slice(&lat_step.to_le_bytes());
    raw.extend_from_slice(&lon_min.to_le_bytes());
    raw.extend_from_slice(&lon_step.to_le_bytes());

    // The two coordinates go in separate runs rather than interleaved: the
    // deltas of one are drawn from a much narrower distribution than the two
    // together, and deflate reads a homogeneous run better. Worth about 8%,
    // for the cost of one length.
    //
    // What deflate cannot reach is the long-range repetition in ICON's cell
    // ordering: a proper LZMA encoder gets this to 0.37 MB against deflate's
    // 2.4. `lzma-rs`, the only pure-Rust option, has a naive match finder and
    // produced 3.6 MB, so the saving is real but not available without either
    // a C library or a second toolchain in the build.
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    let (mut last_a, mut last_o) = (0i64, 0i64);
    for (&a, &o) in lat.iter().zip(lon) {
        let ia = ((f64::from(a) - lat_min) / lat_step).round() as i64;
        let io = ((f64::from(o) - lon_min) / lon_step).round() as i64;
        put_varint(&mut lats, ia - last_a);
        put_varint(&mut lons, io - last_o);
        last_a = ia;
        last_o = io;
    }
    raw.extend_from_slice(&(lats.len() as u32).to_le_bytes());
    raw.extend_from_slice(&lats);
    raw.extend_from_slice(&lons);

    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(&raw)
        .map_err(|e| format!("deflate failed: {e}"))?;
    encoder.finish().map_err(|e| format!("deflate failed: {e}"))
}

/// Finds the lattice the coordinates were packed on.
///
/// The source messages are 16-bit packed, so every value is `R + X · 2^E` and
/// the values lie on a lattice of `2^E` degrees. Recovering `E` is the whole
/// reason the asset is small: on its own lattice a delta between neighbouring
/// cells is a byte, and on a lattice a million times finer it is four.
///
/// `E` is read off the data rather than guessed: with millions of cells every
/// lattice step occurs somewhere, so the smallest gap between distinct values
/// *is* the step. It is snapped to a power of two because that is what GRIB's
/// binary scale factor can express, and then checked against every value —
/// a lattice that does not reproduce the file is refused rather than
/// quietly rounding it.
fn lattice(values: &[f32]) -> Result<(f64, f64), String> {
    let mut sorted: Vec<f64> = values.iter().map(|&v| f64::from(v)).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = *sorted.first().ok_or("no coordinates")?;

    let mut gap = f64::INFINITY;
    for pair in sorted.windows(2) {
        let d = pair[1] - pair[0];
        if d > 0.0 && d < gap {
            gap = d;
        }
    }
    if !gap.is_finite() {
        // Every cell at the same coordinate: any step reproduces it.
        return Ok((min, 1.0));
    }
    let step = 2f64.powi(gap.log2().round() as i32);

    // Half a step is the most a value can sit off a lattice it is on; more
    // than that means this is not the lattice.
    let worst = values
        .iter()
        .map(|&v| {
            let k = ((f64::from(v) - min) / step).round();
            (f64::from(v) - (min + k * step)).abs()
        })
        .fold(0.0f64, f64::max);
    if worst > step * 0.25 {
        return Err(format!(
            "coordinates are not on a lattice of {step} degrees: worst miss {worst}"
        ));
    }
    Ok((min, step))
}

fn put_varint(out: &mut Vec<u8>, value: i64) {
    let mut zig = ((value << 1) ^ (value >> 63)) as u64;
    loop {
        let byte = (zig & 0x7f) as u8;
        zig >>= 7;
        if zig == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn hex(uuid: &[u8; 16]) -> String {
    uuid.iter().map(|b| format!("{b:02x}")).collect()
}
