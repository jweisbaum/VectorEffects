#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! M18's acceptance: a georeferenced image lands where the file says, a
//! hand-placed one survives a save, and neither reaches an export (spec.md 4.9).
//!
//! The fixtures are written here rather than committed. A PNG with a world file
//! and a GeoTIFF with the two model tags are a few dozen lines to produce and
//! knowing exactly what went in is the whole point — a committed fixture whose
//! provenance is "it decoded" asserts nothing about the tags being read right.

use std::io::Write;

use ve_app::commands::AppState;
use ve_app::image::{self, MAX_SERVED_EDGE};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-image-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp root");
        Self(dir)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn app(root: &TempRoot) -> AppState {
    AppState::new(AppPaths::in_directory(&root.0).expect("paths"))
}

fn open(state: &AppState) {
    projects::create(
        state,
        NewProjectRequest {
            name: "Image".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 2,
        },
        true,
    )
    .expect("create");
}

/// A PNG of a known size, with a recognisable corner.
///
/// The top-left pixel is red and every other one is blue, so a downsample or a
/// flip shows up as a colour in the wrong corner rather than as a size that
/// happens to match.
fn write_png(path: &std::path::Path, width: u32, height: u32) {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for (i, pixel) in rgba.chunks_exact_mut(4).enumerate() {
        pixel.copy_from_slice(if i == 0 {
            &[255, 0, 0, 255]
        } else {
            &[0, 0, 255, 255]
        });
    }
    let file = std::fs::File::create(path).expect("create png");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("header");
    writer.write_image_data(&rgba).expect("pixels");
    writer.finish().expect("finish");
}

/// A world file, in its six-line order: a, d, b, e, and the *centre* of the
/// top-left pixel.
fn write_world(path: &std::path::Path, a: f64, d: f64, b: f64, e: f64, x: f64, y: f64) {
    let mut file = std::fs::File::create(path).expect("create world file");
    for value in [a, d, b, e, x, y] {
        writeln!(file, "{value}").expect("write");
    }
}

/// A minimal GeoTIFF: a grey strip with `ModelPixelScale` and `ModelTiepoint`.
///
/// Written by hand — the `tiff` crate encodes pixels but not the geo tags — so
/// the bytes the decoder reads are the bytes this test chose. Little-endian,
/// one strip, 8-bit greyscale.
fn write_geotiff(
    path: &std::path::Path,
    width: u32,
    height: u32,
    scale: [f64; 3],
    tiepoint: [f64; 6],
    projected: bool,
) {
    let pixels = (width * height) as usize;
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"II\x2a\x00"); // little-endian, magic 42
    out.extend_from_slice(&8u32.to_le_bytes()); // first IFD at offset 8

    // The IFD's own size has to be known before the data it points at can be
    // placed: 2 bytes of count, 12 per entry, 4 for the next-IFD offset.
    let entries = if projected { 12 } else { 11 };
    let ifd_bytes = 2 + entries * 12 + 4;
    let mut data_at = 8 + ifd_bytes as u32;

    // Out-of-line values, in the order they are appended below.
    let scale_at = data_at;
    data_at += 24;
    let tiepoint_at = data_at;
    data_at += 48;
    let keys_at = data_at;
    data_at += if projected { 8 * 2 } else { 4 * 2 };
    let strip_at = data_at;

    let mut ifd: Vec<u8> = Vec::new();
    ifd.extend_from_slice(&(entries as u16).to_le_bytes());
    let mut entry = |tag: u16, kind: u16, count: u32, value: u32| {
        ifd.extend_from_slice(&tag.to_le_bytes());
        ifd.extend_from_slice(&kind.to_le_bytes());
        ifd.extend_from_slice(&count.to_le_bytes());
        ifd.extend_from_slice(&value.to_le_bytes());
    };
    entry(256, 3, 1, width); // ImageWidth
    entry(257, 3, 1, height); // ImageLength
    entry(258, 3, 1, 8); // BitsPerSample
    entry(259, 3, 1, 1); // Compression: none
    entry(262, 3, 1, 1); // PhotometricInterpretation: black is zero
    entry(273, 4, 1, strip_at); // StripOffsets
    entry(277, 3, 1, 1); // SamplesPerPixel
    entry(278, 3, 1, height); // RowsPerStrip
    entry(279, 4, 1, pixels as u32); // StripByteCounts
    entry(33550, 12, 3, scale_at); // ModelPixelScale
    entry(33922, 12, 6, tiepoint_at); // ModelTiepoint
    if projected {
        entry(34735, 3, 8, keys_at); // GeoKeyDirectory
    }
    ifd.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    out.extend_from_slice(&ifd);

    for value in scale {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for value in tiepoint {
        out.extend_from_slice(&value.to_le_bytes());
    }
    if projected {
        // Header (1, 1, 0, 1 key) then one key: 1024 = 1, projected.
        for value in [1u16, 1, 0, 1, 1024, 0, 1, 1] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    } else {
        for value in [1u16, 1, 0, 0] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out.extend(std::iter::repeat_n(128u8, pixels));
    std::fs::write(path, out).expect("write geotiff");
}

/// The acceptance case: a GeoTIFF of known extent lands with its corners at the
/// right lon/lat.
#[test]
fn a_geotiff_lands_where_its_tags_say() {
    let root = TempRoot::new("geotiff");
    let path = root.0.join("chart.tif");
    // 40 by 20 pixels at half a degree: 20 degrees across, 10 down, with the
    // top-left corner at 10°W 50°N.
    write_geotiff(
        &path,
        40,
        20,
        [0.5, 0.5, 0.0],
        [0.0, 0.0, 0.0, -10.0, 50.0, 0.0],
        false,
    );

    let probed = image::probe(&path).expect("probe");
    assert_eq!((probed.width, probed.height), (40, 20));
    let placement = probed.placement.expect("a GeoTIFF carries its own place");

    assert_eq!(placement.place(0.0, 0.0), (-10.0, 50.0), "top left");
    assert_eq!(placement.place(40.0, 0.0), (10.0, 50.0), "top right");
    assert_eq!(placement.place(40.0, 20.0), (10.0, 40.0), "bottom right");
    assert_eq!(placement.place(0.0, 20.0), (-10.0, 40.0), "bottom left");
}

/// A tiepoint that is not the corner still resolves to the corner.
#[test]
fn a_geotiff_tiepoint_away_from_the_corner_is_walked_back() {
    let root = TempRoot::new("tiepoint");
    let path = root.0.join("offset.tif");
    // The tiepoint names raster (10, 4) rather than (0, 0): the top-left corner
    // is therefore five degrees west and two north of the stated position.
    write_geotiff(
        &path,
        40,
        20,
        [0.5, 0.5, 0.0],
        [10.0, 4.0, 0.0, 0.0, 0.0, 0.0],
        false,
    );

    let placement = image::probe(&path)
        .expect("probe")
        .placement
        .expect("placed");
    assert_eq!(placement.place(0.0, 0.0), (-5.0, 2.0));
    assert_eq!(
        placement.place(10.0, 4.0),
        (0.0, 0.0),
        "the tiepoint itself"
    );
}

/// The acceptance case's other half: an image spanning the antimeridian.
#[test]
fn an_image_across_the_antimeridian_keeps_its_longitudes_unwrapped() {
    let root = TempRoot::new("dateline");
    let path = root.0.join("pacific.tif");
    // 60 degrees wide starting at 150°E, so the right edge is at 210° — which
    // is 150°W. Normalising it would put the right edge to the *left* of the
    // left one and fold the picture in half.
    write_geotiff(
        &path,
        60,
        20,
        [1.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 150.0, 20.0, 0.0],
        false,
    );

    let placement = image::probe(&path)
        .expect("probe")
        .placement
        .expect("placed");
    let (west, _) = placement.place(0.0, 0.0);
    let (east, _) = placement.place(60.0, 0.0);
    assert_eq!(west, 150.0);
    assert_eq!(east, 210.0, "the right edge stays to the right of the left");

    let corners = placement.corners(60, 20);
    assert!(corners[1].0 > corners[0].0, "the image does not fold");
}

/// A projected GeoTIFF is refused by name, not placed as if its metres were
/// degrees.
#[test]
fn a_projected_geotiff_is_refused_with_a_reason() {
    let root = TempRoot::new("projected");
    let path = root.0.join("utm.tif");
    write_geotiff(
        &path,
        10,
        10,
        [1000.0, 1000.0, 0.0],
        [0.0, 0.0, 0.0, 500_000.0, 4_649_776.0, 0.0],
        true,
    );

    let err = image::probe(&path).expect_err("a projected CRS must be refused");
    let message = format!("{err}");
    assert!(message.contains("projected"), "{message}");
    assert!(
        message.contains("utm.tif"),
        "the message names the file: {message}"
    );
    assert!(
        message.contains("EPSG:4326"),
        "and says what to do: {message}"
    );
}

/// A world file places a PNG, and the half-pixel is taken off.
#[test]
fn a_world_file_places_a_png_by_its_pixel_corner() {
    let root = TempRoot::new("world");
    let path = root.0.join("scan.png");
    write_png(&path, 100, 50);
    // One degree per pixel, north-up, with the *centre* of the top-left pixel
    // at 0.5°E 0.5°N — so the corner is exactly (0, 1).
    write_world(&root.0.join("scan.pgw"), 1.0, 0.0, 0.0, -1.0, 0.5, 0.5);

    let probed = image::probe(&path).expect("probe");
    assert_eq!((probed.width, probed.height), (100, 50));
    let placement = probed.placement.expect("a world file places it");
    assert_eq!(
        placement.place(0.0, 0.0),
        (0.0, 1.0),
        "the corner, not the centre"
    );
    assert_eq!(placement.place(100.0, 50.0), (100.0, -49.0));
}

/// An image with nothing beside it has no placement of its own, and is not an
/// error.
#[test]
fn a_plain_png_has_no_georeference_and_is_still_readable() {
    let root = TempRoot::new("plain");
    let path = root.0.join("plain.png");
    write_png(&path, 32, 16);

    let probed = image::probe(&path).expect("probe");
    assert_eq!((probed.width, probed.height), (32, 16));
    assert_eq!(probed.placement, None);

    let (png, width, height) = image::render(&path, 4096).expect("render");
    assert_eq!((width, height), (32, 16), "nothing to downsample");
    assert_eq!(&png[1..4], b"PNG");
}

/// A large image is downsampled on the way out, and the file is left alone.
#[test]
fn a_large_image_is_downsampled_and_the_file_is_untouched() {
    let root = TempRoot::new("large");
    let path = root.0.join("big.png");
    write_png(&path, 600, 300);
    let before = std::fs::read(&path).expect("read");

    let (_, width, height) = image::render(&path, 200).expect("render");
    assert!(
        width <= 200 && height <= 200,
        "{width}x{height} is over the cap"
    );
    // An integer factor, so the aspect ratio survives: 600x300 at a cap of 200
    // goes to 200x100 and not to something square.
    assert_eq!((width, height), (200, 100));
    assert_eq!(
        std::fs::read(&path).expect("read"),
        before,
        "the file changed"
    );

    // And the cap itself is capped: a hostile URL cannot ask for a gigapixel.
    let (_, huge, _) = image::render(&path, u32::MAX).expect("render");
    assert!(huge <= MAX_SERVED_EDGE);
}

/// An imported image becomes a layer, and the project keeps no pixels.
#[test]
fn an_image_layer_keeps_the_path_and_never_the_pixels() {
    let root = TempRoot::new("layer");
    let state = app(&root);
    open(&state);

    let path = root.0.join("chart.tif");
    write_geotiff(
        &path,
        40,
        20,
        [0.5, 0.5, 0.0],
        [0.0, 0.0, 0.0, -10.0, 50.0, 0.0],
        false,
    );
    let before = projects::current(&state)
        .expect("summary")
        .expect("open")
        .layer_count;
    image::image_imported(&state, path.to_string_lossy().into_owned(), None).expect("import");
    let after = projects::current(&state).expect("summary").expect("open");
    assert_eq!(after.layer_count, before + 1);

    // Saved, the file holds the path and the six placement numbers — and no
    // pixel data at all (invariant 2).
    let saved = root.0.join("with-image.veproj");
    projects::save_as(&state, saved.to_string_lossy().into_owned()).expect("save");
    let bytes = std::fs::read(&saved).expect("read");
    // A 40x20 grey TIFF's pixels are 800 bytes of 128; if any of them reached
    // the project the run would be in there.
    assert!(
        !bytes.windows(64).any(|w| w.iter().all(|b| *b == 128)),
        "the project file carries image pixels"
    );
}

/// Three control points place an image exactly, and the drag is one undo.
#[test]
fn control_points_place_an_image_and_a_drag_is_one_undo() {
    let root = TempRoot::new("corners");
    let state = app(&root);
    open(&state);

    let path = root.0.join("hand.png");
    write_png(&path, 100, 50);
    image::image_imported(
        &state,
        path.to_string_lossy().into_owned(),
        Some([-20.0, 40.0, 20.0, 0.0]),
    )
    .expect("import");

    let layer = image_layer(&state);
    // A drag of the top-right corner, four reports of it.
    for lon in [30.0, 31.0, 32.0, 33.0] {
        image::corners_set(
            &state,
            layer,
            [0.0, 10.0],
            [lon, 10.0],
            [0.0, 0.0],
            Some("image:corner".to_owned()),
        )
        .expect("place");
    }
    let placed = corners_of(&state);
    assert_eq!(placed[0], [0.0, 10.0], "top left");
    assert_eq!(placed[1], [33.0, 10.0], "top right");
    assert_eq!(placed[3], [0.0, 0.0], "bottom left");
    // The fourth corner follows from the other three, which is what makes it
    // an affine rather than four independent points.
    assert_eq!(placed[2], [33.0, 0.0], "bottom right");

    ve_app::edit::undo_for_test(&state).expect("undo");
    let back = corners_of(&state);
    assert_ne!(back[1], [33.0, 10.0], "one undo returns the whole drag");
    assert_ne!(back[1], [32.0, 10.0], "and not one report of it");

    // Three points on a line have no area, and are refused rather than stored.
    assert!(image::corners_set(&state, layer, [0.0, 0.0], [10.0, 0.0], [20.0, 0.0], None).is_err());
}

/// A picture is addressed by the *opening*, not by the document's revision
/// (M37).
///
/// The address is served immutably, so the revision in it meant every edit
/// re-addressed every image: the webview refetched the picture and the
/// backend decoded the whole chart again. Dragging one was a decode per
/// pointer report with nothing on screen in between, which is what "it only
/// appears when the mouse is released" was.
#[test]
fn an_edit_does_not_re_address_a_picture() {
    let root = TempRoot::new("token");
    let state = app(&root);
    open(&state);

    let path = root.0.join("chart.png");
    write_png(&path, 100, 50);
    let after_import =
        image::image_imported(&state, path.to_string_lossy().into_owned(), None).expect("import");
    let token = after_import.image_token;

    // Moving the picture is an edit like any other: the revision moves, the
    // address does not.
    let layer = image_layer(&state);
    let moved = image::corners_set(
        &state,
        layer,
        [1.0, 11.0],
        [21.0, 11.0],
        [1.0, 1.0],
        Some("image:move".to_owned()),
    )
    .expect("move");
    assert_ne!(moved.revision, after_import.revision, "the document moved");
    assert_eq!(moved.image_token, token, "and the picture is where it was");

    // A second opening is a second address, so no picture is ever served
    // from another project's cache entry.
    let other = TempRoot::new("token-two");
    let elsewhere = app(&other);
    open(&elsewhere);
    let fresh = ve_app::document::tree(&elsewhere, 0).expect("tree");
    let _ = fresh;
    let summary = image::image_imported(
        &elsewhere,
        {
            let path = other.0.join("chart.png");
            write_png(&path, 100, 50);
            path.to_string_lossy().into_owned()
        },
        None,
    )
    .expect("import");
    assert_ne!(
        summary.image_token, token,
        "a second opening is a second address"
    );
}

/// Opacity is clamped, editable and undoable.
#[test]
fn opacity_is_clamped_to_what_it_can_mean() {
    let root = TempRoot::new("opacity");
    let state = app(&root);
    open(&state);
    let path = root.0.join("faint.png");
    write_png(&path, 20, 20);
    image::image_imported(&state, path.to_string_lossy().into_owned(), None).expect("import");
    let layer = image_layer(&state);

    image::opacity_set(&state, layer, 0.4).expect("set");
    assert!((view_of(&state).opacity - 0.4).abs() < 1e-9);
    image::opacity_set(&state, layer, 4.0).expect("set");
    assert!((view_of(&state).opacity - 1.0).abs() < 1e-9);
    image::opacity_set(&state, layer, f64::NAN).expect("set");
    assert!((view_of(&state).opacity - 1.0).abs() < 1e-9);
}

/// A hand-placed image survives a save and a reopen, corners and all.
#[test]
fn a_hand_placed_image_survives_a_round_trip() {
    let root = TempRoot::new("roundtrip");
    let state = app(&root);
    open(&state);

    let path = root.0.join("hand.png");
    write_png(&path, 80, 40);
    image::image_imported(&state, path.to_string_lossy().into_owned(), None).expect("import");
    let layer = image_layer(&state);
    image::corners_set(
        &state,
        layer,
        [-33.5, 47.25],
        [11.125, 44.5],
        [-30.0, 20.75],
        None,
    )
    .expect("place");
    image::opacity_set(&state, layer, 0.65).expect("opacity");
    let before = view_of(&state);

    let file = root.0.join("placed.veproj").to_string_lossy().into_owned();
    projects::save_as(&state, file.clone()).expect("save");
    projects::close_open(&state, true).expect("close");
    projects::open(&state, file, true).expect("reopen");

    let after = view_of(&state);
    assert_eq!(after.path, before.path);
    assert!((after.opacity - before.opacity).abs() < 1e-9);
    for (a, b) in after.corners.iter().zip(&before.corners) {
        // To the file's nine decimal places, as everywhere else.
        assert!(
            (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9,
            "{a:?} != {b:?}"
        );
    }
    assert!(!after.georeferenced, "a hand-placed image says it is one");
}

/// An image layer never reaches an export.
#[test]
fn an_image_layer_exports_to_the_same_bytes_as_no_image() {
    use std::sync::atomic::AtomicBool;
    use ve_app::edit::BrushStroke;
    use ve_app::export::{self, ExportRequest};

    let root = TempRoot::new("export");
    let state = app(&root);
    open(&state);

    ve_app::edit::paint(
        &state,
        BrushStroke {
            points: vec![[-10.0, 0.0], [10.0, 0.0]],
            size_km: 800.0,
            speed_mps: 15.0,
            direction_toward_deg: 45.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");

    let export_now = |name: &str| {
        let out = root.0.join(name);
        let project = {
            let mut session = state.session.lock().expect("lock");
            session.require_open().expect("open").project.clone()
        };
        export::run(
            &project,
            &ExportRequest {
                path: out.to_string_lossy().into_owned(),
                year: 2026,
                month: 9,
                day: 4,
                hour: 0,
                centre: 255,
                bits: 16,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .expect("export");
        std::fs::read(&out).expect("read")
    };

    let clean = export_now("clean.grib2");

    let path = root.0.join("over.tif");
    // Right over the painted stroke, so an image that did reach the field
    // would reach the values being compared.
    write_geotiff(
        &path,
        40,
        20,
        [1.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, -20.0, 10.0, 0.0],
        false,
    );
    image::image_imported(&state, path.to_string_lossy().into_owned(), None).expect("import");
    assert_eq!(view_of(&state).width, 40, "the image really is there");

    assert_eq!(
        clean,
        export_now("with-image.grib2"),
        "an image layer changed the exported bytes, which it must never do"
    );
}

/// The id of the one image layer.
fn image_layer(state: &AppState) -> u64 {
    view_of(state).layer
}

/// The one image layer's view.
fn view_of(state: &AppState) -> ve_app::image::ImageLayerView {
    ve_app::document::tree(state, 0)
        .expect("tree")
        .layers
        .into_iter()
        .find_map(|layer| layer.image)
        .expect("an image layer")
}

/// The one image layer's corners.
fn corners_of(state: &AppState) -> Vec<[f64; 2]> {
    view_of(state).corners
}
