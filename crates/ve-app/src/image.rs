//! Georeferenced image layers (spec.md 4.9, M18).
//!
//! A chart scan, a satellite picture or a synoptic chart, laid under the field
//! to trace or to compare against. **Display only**: never composited, never
//! evaluated, never exported, no vectors of its own.
//!
//! # No pixels anywhere but on the way to the screen
//!
//! The project keeps the file's path and six numbers saying where it sits, and
//! nothing else (invariant 2) — the same bargain a GRIB layer makes. The
//! session keeps no pixels either: an image is decoded *on demand* by the URI
//! scheme, downsampled to what the caller's GPU can hold, encoded as a PNG and
//! cached in that form. A ten-thousand-pixel TIFF is 400 MB as RGBA and a few
//! as a PNG, so what is held is the small one.
//!
//! # Decoders
//!
//! `png`, `jpeg-decoder` and `tiff`, all pure Rust and none of them pulling a
//! `-sys` crate — invariant 5 and the three-platform build both forbid a C
//! library, which is the same rule the GRIB codecs live under. Check
//! `cargo tree` after any version bump.
//!
//! # Georeferencing
//!
//! Read from the file where the file says: a GeoTIFF's `ModelTiepoint` and
//! `ModelPixelScale`, or a world file beside a PNG or a JPEG. Both reduce to
//! exactly the six numbers [`Placement`] holds, so there is no conversion —
//! only a half-pixel, because a world file names the *centre* of the top-left
//! pixel and a placement names its corner.
//!
//! A GeoTIFF in a projected CRS is **refused by name**. Reprojecting a raster
//! is a different piece of work from placing one, and quietly treating metres
//! as degrees would put an image somewhere plausible and wrong.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Serialize;
use ts_rs::TS;
use ve_core::document::{LayerSource, Placement};
use ve_core::id::Id;

use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::with_session;

/// The largest image edge served, in pixels.
///
/// Every WebGL2 implementation guarantees at least 2048 and real hardware
/// offers 8192 or more; the caller sends its own maximum and this is the
/// ceiling on what that may ask for, so a mistyped or hostile URL cannot ask
/// the app to encode a gigapixel PNG.
pub const MAX_SERVED_EDGE: u32 = 8192;

/// What the frontend gets when it asks about an image layer.
#[derive(Debug, Clone, Serialize, JsonSchema, TS)]
#[ts(export, export_to = "ImageLayerView.ts")]
pub struct ImageLayerView {
    /// Which layer this is.
    pub layer: u64,
    /// The file, for the panel to show and for a missing-file message.
    pub path: String,
    /// Whether the file could be read at all.
    pub loaded: bool,
    /// The image's size in its own pixels, before any downsampling.
    pub width: u32,
    /// And its height.
    pub height: u32,
    /// Where it sits: `[a, b, c, d, e, f]` (see `Placement`).
    pub placement: [f64; 6],
    /// How strongly it shows, 0 to 1.
    pub opacity: f64,
    /// The four corners as `[lon, lat]`, top-left first, clockwise.
    ///
    /// Computed here rather than in the frontend so the placement has one
    /// implementation: the map draws these and the control points drag them.
    pub corners: Vec<[f64; 2]>,
    /// Whether the file carried its own georeference.
    ///
    /// A hand-placed image says so, because "the corners are where the file
    /// said" and "the corners are where you dragged them" are very different
    /// claims about an overlay someone is about to trace.
    pub georeferenced: bool,
}

/// An image's size and, if the file carries one, where it sits.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The file's own georeference, if it has one.
    pub placement: Option<Placement>,
}

/// Which decoder a file needs, by extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Png,
    Jpeg,
    Tiff,
}

impl Format {
    /// The format a path names, by extension.
    ///
    /// By extension and not by sniffing the magic bytes, because the extension
    /// is also what names the world file beside it — a `.png` takes a `.pgw`.
    /// A file whose contents disagree with its name fails in the decoder with
    /// a message naming the decoder, which is the more useful error anyway.
    fn of(path: &Path) -> Option<Self> {
        match path
            .extension()
            .and_then(|e| e.to_str())?
            .to_ascii_lowercase()
            .as_str()
        {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "tif" | "tiff" => Some(Self::Tiff),
            _ => None,
        }
    }

    /// The world-file extensions this format's images use.
    ///
    /// The three-letter form is the first and last letters of the image
    /// extension plus `w`, which is the convention every GIS writes; `.wld` is
    /// the catch-all, accepted by everything and written by some.
    fn world_extensions(self) -> &'static [&'static str] {
        match self {
            Self::Png => &["pgw", "pngw", "wld"],
            Self::Jpeg => &["jgw", "jpgw", "jpegw", "wld"],
            Self::Tiff => &["tfw", "tifw", "tiffw", "wld"],
        }
    }
}

/// Reads an image's size and georeference without decoding its pixels.
///
/// What an import needs: how big it is, and where — if anywhere — the file says
/// it goes. Decoding a hundred-megapixel TIFF to answer that would be the
/// slowest possible way to find out.
pub fn probe(path: &Path) -> Result<Probe> {
    let format = Format::of(path).ok_or_else(|| unsupported(path))?;
    let mut probed = match format {
        Format::Png => probe_png(path)?,
        Format::Jpeg => probe_jpeg(path)?,
        Format::Tiff => probe_tiff(path)?,
    };
    // A world file beside the image wins over nothing, and loses to a
    // georeference the file itself carries: a GeoTIFF that also has a `.tfw`
    // is a GeoTIFF, and the two are meant to agree.
    if probed.placement.is_none() {
        probed.placement = world_file(path, format);
    }
    Ok(probed)
}

/// Decodes an image and returns it as a PNG, no edge longer than `max_edge`.
///
/// PNG rather than raw pixels because the webview decodes it for free and the
/// bytes on the wire are a fraction of the size; lossless because an overlay
/// someone is tracing must not gain artefacts on the way to the screen.
///
/// **The file on disk is never modified.** Downsampling happens on the way out.
pub fn render(path: &Path, max_edge: u32) -> Result<(Vec<u8>, u32, u32)> {
    let format = Format::of(path).ok_or_else(|| unsupported(path))?;
    let (rgba, width, height) = match format {
        Format::Png => decode_png(path)?,
        Format::Jpeg => decode_jpeg(path)?,
        Format::Tiff => decode_tiff(path)?,
    };
    let cap = max_edge.clamp(64, MAX_SERVED_EDGE);
    let (rgba, width, height) = downsample(rgba, width, height, cap);
    Ok((encode_png(&rgba, width, height)?, width, height))
}

/// Everything the frontend needs to draw one image layer.
pub fn view(layer: Id, source: &LayerSource) -> Option<ImageLayerView> {
    let LayerSource::Image {
        path,
        placement,
        opacity,
    } = source
    else {
        return None;
    };
    // A probe on every read would open the file each time the layer panel
    // refreshes; it is a header read, and the alternative is caching a size
    // that the file could have changed underneath.
    let probed = probe(path).ok();
    let (width, height) = probed.as_ref().map_or((0, 0), |p| (p.width, p.height));
    Some(ImageLayerView {
        layer: layer.raw(),
        path: path.to_string_lossy().into_owned(),
        loaded: probed.is_some(),
        width,
        height,
        placement: [
            placement.a,
            placement.b,
            placement.c,
            placement.d,
            placement.e,
            placement.f,
        ],
        opacity: *opacity,
        corners: placement
            .corners(width, height)
            .iter()
            .map(|(lon, lat)| [*lon, *lat])
            .collect(),
        georeferenced: probed.as_ref().is_some_and(|p| p.placement.is_some()),
    })
}

// --- PNG --------------------------------------------------------------------

fn probe_png(path: &Path) -> Result<Probe> {
    let file = std::fs::File::open(path).map_err(|e| unreadable(path, &e))?;
    let reader = png::Decoder::new(BufReader::new(file))
        .read_info()
        .map_err(|e| undecodable(path, &e))?;
    let info = reader.info();
    Ok(Probe {
        width: info.width,
        height: info.height,
        placement: None,
    })
}

fn decode_png(path: &Path) -> Result<(Vec<u8>, u32, u32)> {
    let file = std::fs::File::open(path).map_err(|e| unreadable(path, &e))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    // Whatever the file is — palette, grey, 16-bit — comes out as 8-bit RGBA,
    // which is the one layout the rest of this module knows.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| undecodable(path, &e))?;
    let mut buffer = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|e| undecodable(path, &e))?;
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return Err(undecodable(path, &"palette after expansion")),
    };
    buffer.truncate(info.buffer_size());
    Ok((
        to_rgba(&buffer, channels, info.width, info.height),
        info.width,
        info.height,
    ))
}

pub(crate) fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| AppError::Internal(format!("encoding the image failed: {e}")))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| AppError::Internal(format!("encoding the image failed: {e}")))?;
    writer
        .finish()
        .map_err(|e| AppError::Internal(format!("encoding the image failed: {e}")))?;
    Ok(out)
}

// --- JPEG -------------------------------------------------------------------

fn probe_jpeg(path: &Path) -> Result<Probe> {
    let file = std::fs::File::open(path).map_err(|e| unreadable(path, &e))?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));
    decoder.read_info().map_err(|e| undecodable(path, &e))?;
    let info = decoder
        .info()
        .ok_or_else(|| undecodable(path, &"no frame header"))?;
    Ok(Probe {
        width: u32::from(info.width),
        height: u32::from(info.height),
        placement: None,
    })
}

fn decode_jpeg(path: &Path) -> Result<(Vec<u8>, u32, u32)> {
    let file = std::fs::File::open(path).map_err(|e| unreadable(path, &e))?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));
    let pixels = decoder.decode().map_err(|e| undecodable(path, &e))?;
    let info = decoder
        .info()
        .ok_or_else(|| undecodable(path, &"no frame header"))?;
    let channels = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => 3,
        jpeg_decoder::PixelFormat::L8 => 1,
        jpeg_decoder::PixelFormat::L16 => {
            return Err(undecodable(path, &"16-bit greyscale JPEG"));
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            // A print-workflow JPEG. Converting CMYK to RGB needs a colour
            // profile to be anything but a guess, and a guessed conversion on
            // a chart someone is tracing is worse than a refusal.
            return Err(undecodable(path, &"CMYK JPEG"));
        }
    };
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    Ok((to_rgba(&pixels, channels, width, height), width, height))
}

// --- TIFF and its georeference ----------------------------------------------

/// GeoTIFF tag 33550: the size of one pixel in model units.
const MODEL_PIXEL_SCALE: u16 = 33550;
/// GeoTIFF tag 33922: raster point to model point, in groups of six.
const MODEL_TIEPOINT: u16 = 33922;
/// GeoTIFF tag 34735: the geo keys, whose first entry says which CRS family.
const GEO_KEY_DIRECTORY: u16 = 34735;
/// Geo key 1024: 1 projected, 2 geographic, 3 geocentric.
const GT_MODEL_TYPE: u16 = 1024;
/// The value of geo key 1024 that means longitude and latitude.
const MODEL_TYPE_GEOGRAPHIC: u16 = 2;

fn open_tiff(path: &Path) -> Result<tiff::decoder::Decoder<BufReader<std::fs::File>>> {
    let file = std::fs::File::open(path).map_err(|e| unreadable(path, &e))?;
    tiff::decoder::Decoder::new(BufReader::new(file)).map_err(|e| undecodable(path, &e))
}

fn probe_tiff(path: &Path) -> Result<Probe> {
    let mut decoder = open_tiff(path)?;
    let (width, height) = decoder.dimensions().map_err(|e| undecodable(path, &e))?;
    Ok(Probe {
        width,
        height,
        placement: geotiff_placement(path, &mut decoder)?,
    })
}

/// The georeference a GeoTIFF carries, if it carries one this app can use.
///
/// `ModelTiepoint` gives one raster point and where it lands; `ModelPixelScale`
/// gives the size of a pixel. Together they are a north-up affine, which is
/// what the overwhelming majority of GeoTIFFs are. A rotated one carries
/// `ModelTransformation` instead and is treated as ungeoreferenced — it is
/// placed by hand, which is a worse answer than reading the matrix and a much
/// better one than reading the matrix wrongly.
fn geotiff_placement(
    path: &Path,
    decoder: &mut tiff::decoder::Decoder<BufReader<std::fs::File>>,
) -> Result<Option<Placement>> {
    let keys = decoder
        .get_tag_u32_vec(tiff::tags::Tag::Unknown(GEO_KEY_DIRECTORY))
        .ok();
    if let Some(model) = model_type(keys.as_deref())
        && model != u32::from(MODEL_TYPE_GEOGRAPHIC)
    {
        // Refused by name, not silently mis-placed: reprojecting a raster
        // is a different piece of work from placing one, and metres read as
        // degrees put an image somewhere plausible and wrong.
        return Err(AppError::BadOption {
            field: "image",
            value: format!(
                "{} is a GeoTIFF in a projected coordinate system; \
                 only longitude/latitude images can be placed automatically. \
                     Reproject it to EPSG:4326, or import it and place it by hand.",
                name_of(path)
            ),
        });
    }

    let (Ok(scale), Ok(tie)) = (
        decoder.get_tag_f64_vec(tiff::tags::Tag::Unknown(MODEL_PIXEL_SCALE)),
        decoder.get_tag_f64_vec(tiff::tags::Tag::Unknown(MODEL_TIEPOINT)),
    ) else {
        return Ok(None);
    };
    let (Some(&[sx, sy, ..]), Some(&[i, j, _k, x, y, _z, ..])) = (scale.get(..2), tie.get(..6))
    else {
        return Ok(None);
    };
    if sx <= 0.0 || sy <= 0.0 || ![i, j, x, y].iter().all(|v| v.is_finite()) {
        return Ok(None);
    }

    // The tiepoint says raster (i, j) is model (x, y). The placement names the
    // top-left corner, so walk back from the tiepoint by its own raster
    // position. `sy` is positive in the file and the image runs *down*, so the
    // latitude coefficient is its negation.
    Ok(Some(Placement {
        a: sx,
        b: 0.0,
        c: x - i * sx,
        d: 0.0,
        e: -sy,
        f: y + j * sy,
    }))
}

/// The `GTModelTypeGeoKey` value in a geo key directory, if it is there.
///
/// The directory is a flat list of `u16` quadruples after a four-entry header:
/// key, location, count, value. A key stored inline has location 0 and its
/// value in the fourth slot, which is where the model type always is.
fn model_type(keys: Option<&[u32]>) -> Option<u32> {
    let keys = keys?;
    if keys.len() < 4 {
        return None;
    }
    keys[4..]
        .chunks_exact(4)
        .find(|entry| entry[0] == u32::from(GT_MODEL_TYPE) && entry[1] == 0)
        .map(|entry| entry[3])
}

fn decode_tiff(path: &Path) -> Result<(Vec<u8>, u32, u32)> {
    let mut decoder = open_tiff(path)?;
    let (width, height) = decoder.dimensions().map_err(|e| undecodable(path, &e))?;
    let colour = decoder.colortype().map_err(|e| undecodable(path, &e))?;
    let image = decoder.read_image().map_err(|e| undecodable(path, &e))?;

    let (bytes, channels) = match (image, colour) {
        (tiff::decoder::DecodingResult::U8(data), tiff::ColorType::RGBA(8)) => (data, 4),
        (tiff::decoder::DecodingResult::U8(data), tiff::ColorType::RGB(8)) => (data, 3),
        (tiff::decoder::DecodingResult::U8(data), tiff::ColorType::GrayA(8)) => (data, 2),
        (tiff::decoder::DecodingResult::U8(data), tiff::ColorType::Gray(8)) => (data, 1),
        (_, other) => {
            return Err(undecodable(
                path,
                &format!("{other:?} TIFF; 8-bit grey, RGB and RGBA are supported"),
            ));
        }
    };
    Ok((to_rgba(&bytes, channels, width, height), width, height))
}

// --- World files ------------------------------------------------------------

/// The placement a world file beside `path` describes, if there is one.
///
/// Six lines: the two pixel-size terms, the two skews, and the model position
/// of the **centre** of the top-left pixel. A placement names that pixel's
/// *corner*, which is the half-pixel this function exists to get right — half a
/// pixel on a coarse image is tens of kilometres.
fn world_file(path: &Path, format: Format) -> Option<Placement> {
    for extension in format.world_extensions() {
        let Ok(text) = std::fs::read_to_string(beside(path, extension)) else {
            continue;
        };
        let numbers: Vec<f64> = text
            .lines()
            .filter_map(|line| line.trim().parse::<f64>().ok())
            .take(6)
            .collect();
        let Ok([a, d, b, e, x_centre, y_centre]) = <[f64; 6]>::try_from(numbers) else {
            continue;
        };
        if !(a * e - b * d).is_finite() || (a * e - b * d).abs() < 1e-15 {
            continue;
        }
        return Some(Placement {
            a,
            b,
            c: x_centre - (a + b) / 2.0,
            d,
            e,
            f: y_centre - (d + e) / 2.0,
        });
    }
    None
}

// --- Pixels -----------------------------------------------------------------

/// Widens any channel count to RGBA, so one layout reaches the encoder.
fn to_rgba(bytes: &[u8], channels: usize, width: u32, height: u32) -> Vec<u8> {
    let pixels = (width as usize) * (height as usize);
    let mut out = vec![255; pixels * 4];
    for i in 0..pixels {
        let from = i * channels;
        let to = i * 4;
        match channels {
            4 => out[to..to + 4].copy_from_slice(bytes.get(from..from + 4).unwrap_or(&[0; 4])),
            3 => {
                out[to..to + 3].copy_from_slice(bytes.get(from..from + 3).unwrap_or(&[0; 3]));
            }
            2 => {
                let grey = bytes.get(from).copied().unwrap_or(0);
                out[to] = grey;
                out[to + 1] = grey;
                out[to + 2] = grey;
                out[to + 3] = bytes.get(from + 1).copied().unwrap_or(255);
            }
            _ => {
                let grey = bytes.get(from).copied().unwrap_or(0);
                out[to] = grey;
                out[to + 1] = grey;
                out[to + 2] = grey;
            }
        }
    }
    out
}

/// Shrinks an image by an integer factor until it fits `cap`.
///
/// A box filter over whole blocks, not a nearest-neighbour pick: an image
/// halved by dropping pixels loses thin coastlines and gains aliasing on
/// exactly the kind of ruled chart this feature is for. An integer factor
/// keeps every output pixel the mean of the same number of inputs, so there is
/// no seam where the block size changes.
fn downsample(rgba: Vec<u8>, width: u32, height: u32, cap: u32) -> (Vec<u8>, u32, u32) {
    let longest = width.max(height);
    if longest <= cap || cap == 0 {
        return (rgba, width, height);
    }
    let factor = longest.div_ceil(cap).max(2) as usize;
    let (w, h) = (width as usize, height as usize);
    let out_w = w.div_ceil(factor);
    let out_h = h.div_ceil(factor);
    let mut out = vec![0u8; out_w * out_h * 4];

    for oy in 0..out_h {
        for ox in 0..out_w {
            let mut sums = [0u32; 4];
            let mut count = 0u32;
            for y in oy * factor..((oy + 1) * factor).min(h) {
                for x in ox * factor..((ox + 1) * factor).min(w) {
                    let at = (y * w + x) * 4;
                    for (c, sum) in sums.iter_mut().enumerate() {
                        *sum += u32::from(rgba.get(at + c).copied().unwrap_or(0));
                    }
                    count += 1;
                }
            }
            let at = (oy * out_w + ox) * 4;
            for (c, sum) in sums.iter().enumerate() {
                out[at + c] = sum.checked_div(count).unwrap_or(0) as u8;
            }
        }
    }
    (out, out_w as u32, out_h as u32)
}

// --- Errors -----------------------------------------------------------------

/// The file's name, for a message a user can act on.
fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn unsupported(path: &Path) -> AppError {
    AppError::BadOption {
        field: "image",
        value: format!(
            "{} is not an image this app can read; PNG, JPEG and TIFF are supported",
            name_of(path)
        ),
    }
}

fn unreadable(path: &Path, why: &dyn std::fmt::Display) -> AppError {
    AppError::BadOption {
        field: "image",
        value: format!("{} could not be opened: {why}", name_of(path)),
    }
}

fn undecodable(path: &Path, why: &dyn std::fmt::Display) -> AppError {
    AppError::BadOption {
        field: "image",
        value: format!("{} could not be decoded: {why}", name_of(path)),
    }
}

/// A sibling path with a different extension, matching the original's case.
///
/// A GIS that writes `MAP.TIF` writes `MAP.TFW`, and one that writes `map.tif`
/// writes `map.tfw`; looking for only one of the two finds the world file on
/// one machine and not on another.
///
/// A free function rather than an extension trait, because `Path` already has
/// an inherent `with_extension` and it would win every call — silently, and
/// with the case never matched.
fn beside(path: &Path, extension: &str) -> PathBuf {
    let shouting = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| !e.is_empty() && e.chars().all(|c| !c.is_lowercase()));
    let mut out = path.to_path_buf();
    out.set_extension(if shouting {
        extension.to_ascii_uppercase()
    } else {
        extension.to_owned()
    });
    out
}

// --- Commands ---------------------------------------------------------------

/// Adds an image layer, georeferenced by the file or by the view.
///
/// The layer goes on top of the stack like any other import. It carries no
/// objects and never will: an image layer is a picture, and painting on it
/// would make it a painted layer with a picture stuck to it.
#[tauri::command(async)]
pub fn import_image(
    state: tauri::State<'_, AppState>,
    path: String,
    view: Option<[f64; 4]>,
) -> Result<crate::projects::ProjectSummary> {
    image_imported(&state, path, view)
}

/// Implementation of [`import_image`].
///
/// `view` is the visible map as `[west, north, east, south]`, used only when
/// the file says nothing about where it goes: the image lands filling the view,
/// north-up and keeping its aspect ratio, which puts it where the user is
/// looking and at a size they can grab. Without it a hand-placed image would
/// land on the whole globe and its control points would be off screen.
pub fn image_imported(
    state: &AppState,
    path: String,
    view: Option<[f64; 4]>,
) -> Result<crate::projects::ProjectSummary> {
    let path = PathBuf::from(path);
    let probed = probe(&path)?;
    let placement = probed
        .placement
        .unwrap_or_else(|| centred(probed.width, probed.height, view));

    with_session(state, |session| {
        let open = session.require_open()?;
        let mut layer = ve_core::document::Layer::new(name_of(&path));
        layer.source = LayerSource::Image {
            path: path.clone(),
            placement,
            opacity: 1.0,
        };
        let command = ve_core::command::Command::AddLayer {
            index: open.project.layers.len(),
            layer: Box::new(layer),
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        // The tile revision *is* bumped, unlike a measurement: an image layer
        // is drawn under the field and the map addresses it by revision, so a
        // new one has to reach a webview that caches by URL for a year.
        open.touch();
        tracing::info!(
            path = %path.display(),
            width = probed.width,
            height = probed.height,
            georeferenced = probed.placement.is_some(),
            "imported image layer"
        );
        Ok(crate::projects::ProjectSummary::of(session.require_open()?))
    })
}

/// Where an image with no georeference of its own lands.
///
/// Filling the visible map, north-up, with its aspect ratio kept — so a chart
/// scan arrives where the user is looking, the right shape, and big enough to
/// grab by the corners. Keeping the aspect matters: an image stretched to the
/// viewport would have to be un-stretched by hand before it could be placed.
fn centred(width: u32, height: u32, view: Option<[f64; 4]>) -> Placement {
    let [west, north, east, south] = view.unwrap_or([-180.0, 85.0, 180.0, -85.0]);
    let (centre_lon, centre_lat) = ((west + east) / 2.0, (north + south) / 2.0);
    let (span_lon, span_lat) = (
        (east - west).abs().max(1e-6),
        (north - south).abs().max(1e-6),
    );

    let aspect = f64::from(width.max(1)) / f64::from(height.max(1));
    // Fit inside the view: whichever axis runs out first sets the size.
    let (mut w, mut h) = (span_lon, span_lon / aspect);
    if h > span_lat {
        h = span_lat;
        w = span_lat * aspect;
    }
    // Three quarters of the view, so the corners are on screen and grabbable
    // rather than exactly under the edge.
    let (w, h) = (w * 0.75, h * 0.75);
    Placement::spanning(
        centre_lon - w / 2.0,
        centre_lat + h / 2.0,
        centre_lon + w / 2.0,
        centre_lat - h / 2.0,
        width,
        height,
    )
}

/// Moves an image by its three control points.
///
/// Top-left, top-right and bottom-left in the image's own pixels, each given a
/// place on the map. Three points determine an affine exactly, which is why
/// there are three (spec.md 4.9).
#[tauri::command]
pub fn set_image_corners(
    state: tauri::State<'_, AppState>,
    layer: u64,
    top_left: [f64; 2],
    top_right: [f64; 2],
    bottom_left: [f64; 2],
    gesture: Option<String>,
) -> Result<crate::projects::ProjectSummary> {
    corners_set(&state, layer, top_left, top_right, bottom_left, gesture)
}

/// Implementation of [`set_image_corners`].
pub fn corners_set(
    state: &AppState,
    layer: u64,
    top_left: [f64; 2],
    top_right: [f64; 2],
    bottom_left: [f64; 2],
    gesture: Option<String>,
) -> Result<crate::projects::ProjectSummary> {
    write(state, layer, gesture, |source, probed| {
        let LayerSource::Image { placement, .. } = source else {
            return Err(not_an_image());
        };
        *placement = Placement::from_corners(
            probed.width,
            probed.height,
            (top_left[0], top_left[1]),
            (top_right[0], top_right[1]),
            (bottom_left[0], bottom_left[1]),
        )
        .ok_or_else(|| AppError::BadOption {
            field: "image",
            value: "those three corners are on a line, which is an image with no area".to_owned(),
        })?;
        Ok(())
    })
}

/// Sets how strongly an image shows.
#[tauri::command]
pub fn set_image_opacity(
    state: tauri::State<'_, AppState>,
    layer: u64,
    opacity: f64,
) -> Result<crate::projects::ProjectSummary> {
    opacity_set(&state, layer, opacity)
}

/// Implementation of [`set_image_opacity`].
pub fn opacity_set(
    state: &AppState,
    layer: u64,
    opacity: f64,
) -> Result<crate::projects::ProjectSummary> {
    let wanted = if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    write(
        state,
        layer,
        Some(format!("image:{layer}:opacity")),
        |source, _| {
            let LayerSource::Image { opacity, .. } = source else {
                return Err(not_an_image());
            };
            *opacity = wanted;
            Ok(())
        },
    )
}

/// Puts an image back where its file says it goes.
///
/// Only for a file that says: a hand-placed image has nothing to go back to,
/// and the command says so rather than silently doing nothing.
#[tauri::command]
pub fn reset_image_placement(
    state: tauri::State<'_, AppState>,
    layer: u64,
) -> Result<crate::projects::ProjectSummary> {
    placement_reset(&state, layer)
}

/// Implementation of [`reset_image_placement`].
pub fn placement_reset(state: &AppState, layer: u64) -> Result<crate::projects::ProjectSummary> {
    write(state, layer, None, |source, probed| {
        let LayerSource::Image { placement, .. } = source else {
            return Err(not_an_image());
        };
        *placement = probed.placement.ok_or_else(|| AppError::BadOption {
            field: "image",
            value: "this image carries no georeference of its own to go back to".to_owned(),
        })?;
        Ok(())
    })
}

/// Edits one image layer's source, as one undoable step.
fn write(
    state: &AppState,
    layer: u64,
    gesture: Option<String>,
    edit: impl FnOnce(&mut LayerSource, &Probe) -> Result<()>,
) -> Result<crate::projects::ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let id = Id::from_raw(layer);
        let before = open
            .project
            .layer(id)
            .ok_or(ve_core::CoreError::MissingLayer(layer))?
            .source
            .clone();
        let path = before.path().ok_or_else(not_an_image)?.to_path_buf();
        let probed = probe(&path)?;

        let mut after = before.clone();
        edit(&mut after, &probed)?;
        if after == before {
            return Ok(crate::projects::ProjectSummary::of(open));
        }

        let command = ve_core::command::Command::SetLayerSource {
            layer: id,
            before: Box::new(before),
            after: Box::new(after),
        };
        let (project, history) = (&mut open.project, &mut open.history);
        match gesture {
            Some(key) => history.push_coalesced(project, command, key)?,
            None => history.push(project, command)?,
        }
        open.touch();
        Ok(crate::projects::ProjectSummary::of(session.require_open()?))
    })
}

fn not_an_image() -> AppError {
    AppError::BadOption {
        field: "image",
        value: "that layer is not an image layer".to_owned(),
    }
}
