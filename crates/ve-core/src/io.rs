//! Reading and writing `.veproj` project files.
//!
//! The container is a ZIP holding canonical JSON (spec.md 4.7). JSON was chosen
//! over a binary format for diffability and recoverability: a project is a few
//! megabytes even at five thousand objects, and a corrupt file a user can open
//! in a text editor is far better than one they cannot.
//!
//! **No rasters, ever** (invariant 1). Not even a preview thumbnail. The
//! archive holds the document and the embedded polars, and nothing else.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{CoreError, Result};
use crate::project::{Project, SCHEMA_VERSION};

/// The project document inside the archive.
pub const PROJECT_ENTRY: &str = "project.json";
/// A plain-text schema version, readable without parsing the document.
pub const VERSION_ENTRY: &str = "META-INF/version";
/// The file extension, without a dot.
pub const EXTENSION: &str = "veproj";

/// A function that upgrades a document from one schema version to the next.
pub type Migration = fn(&mut Value) -> Result<()>;

/// The migration chain, ordered by the version each step upgrades *from*.
const MIGRATIONS: &[(u32, Migration)] = &[
    (1, stroke_points_to_chains),
    (2, brush_fill_mode_becomes_brush_shape),
    (3, brush_loses_divergence_and_curl),
    (4, circle_space_becomes_stamp_space),
];

/// Version 1 stored a stroke as one polyline: `{"stroke": {"points": [...]}}`.
///
/// Version 2 stores a list of them, because overlapping strokes with identical
/// properties merge into one object and cannot share a single polyline — the
/// join would sweep the brush across the gap between them.
fn stroke_points_to_chains(value: &mut Value) -> Result<()> {
    fn convert(node: &mut Value) {
        match node {
            Value::Object(map) => {
                if let Some(Value::Object(stroke)) = map.get_mut("stroke")
                    && let Some(points) = stroke.remove("points")
                {
                    stroke.insert("chains".to_owned(), Value::Array(vec![points]));
                }
                for child in map.values_mut() {
                    convert(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    convert(item);
                }
            }
            _ => {}
        }
    }
    convert(value);
    Ok(())
}

/// Version 2 held the brush's circle-or-square choice in `fill_mode`, the same
/// property id the circle stamp uses for filled/perimeter/gradient.
///
/// Version 3 gives it `brush_shape` of its own. The old key is dropped rather
/// than renamed: it was never settable, so every version-2 brush carries the
/// default, and `PropertyMap::backfill` inserts `brush_shape` at that same
/// default on load. Leaving it in place would strand a property the brush
/// schema no longer declares — inert to the evaluator, but enough to stop two
/// otherwise identical strokes merging, since merging compares property counts.
fn brush_fill_mode_becomes_brush_shape(value: &mut Value) -> Result<()> {
    let Some(layers) = value.get_mut("layers").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for layer in layers {
        let Some(objects) = layer.get_mut("objects").and_then(Value::as_array_mut) else {
            continue;
        };
        for object in objects {
            // Only the brush: the circle stamp's `fill_mode` is untouched, and
            // stripping it would turn every ring into a filled disc.
            if object.get("tool").and_then(Value::as_str) != Some("brush") {
                continue;
            }
            if let Some(Value::Object(props)) = object.get_mut("props") {
                props.remove("fill_mode");
            }
        }
    }
    Ok(())
}

/// Serialises a project to canonical JSON.
///
/// Canonical means byte-identical for equal documents: struct fields serialise
/// in declaration order and `PropertyMap` is a `BTreeMap` keyed by an ordered
/// enum, so no sorting pass is needed here.
pub fn to_canonical_json(project: &Project) -> Result<String> {
    Ok(serde_json::to_string_pretty(project)?)
}

/// Parses, migrates, normalises, and validates a project document.
pub fn from_json(json: &str) -> Result<Project> {
    let mut value: Value = serde_json::from_str(json)?;
    migrate(&mut value)?;

    let mut project: Project = serde_json::from_value(value)?;
    project.schema_version = SCHEMA_VERSION;
    project.normalize();
    project.validate()?;
    Ok(project)
}

/// The brush no longer has `divergence` or `curl` (spec.md 6.2).
///
/// A migration and not just a table change: [`PropertyMap::value_at`] returns
/// whatever the map holds, and only falls back to the schema when the key is
/// absent. A leftover entry would therefore go on being read and go on bending
/// the flow, on an object whose panel no longer shows it and whose tool no
/// longer defines it — invisible, unreachable and still in the export.
///
/// Only the brush: the circle stamp and the shape fill still have both, and
/// stripping theirs would flatten every rotating circle.
fn brush_loses_divergence_and_curl(value: &mut Value) -> Result<()> {
    let Some(layers) = value.get_mut("layers").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for layer in layers {
        let Some(objects) = layer.get_mut("objects").and_then(Value::as_array_mut) else {
            continue;
        };
        for object in objects {
            if object.get("tool").and_then(Value::as_str) != Some("brush") {
                continue;
            }
            if let Some(Value::Object(props)) = object.get_mut("props") {
                props.remove("divergence");
                props.remove("curl");
            }
        }
    }
    Ok(())
}

/// The circle stamp's `circle_space` is the brush's `stamp_space` (spec.md 6.1).
///
/// One question — is this shape defined on the ground or on the map — had two
/// property names and two vocabularies, `screen_circular`/`geodesic_circular`
/// against `projected`/`geodesic`. They are renamed onto one before the circle
/// tool ships, so "px" cannot come to mean two different things.
///
/// The variants are in the opposite order, so the index is remapped rather than
/// carried: `screen_circular` was 0 and is `projected`, which is 1.
fn circle_space_becomes_stamp_space(value: &mut Value) -> Result<()> {
    let Some(layers) = value.get_mut("layers").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for layer in layers {
        let Some(objects) = layer.get_mut("objects").and_then(Value::as_array_mut) else {
            continue;
        };
        for object in objects {
            let Some(Value::Object(props)) = object.get_mut("props") else {
                continue;
            };
            let Some(old) = props.remove("circle_space") else {
                continue;
            };
            let screen_circular = old
                .get("base")
                .and_then(|base| base.get("enum"))
                .and_then(Value::as_u64)
                == Some(0);
            props.insert(
                "stamp_space".to_owned(),
                serde_json::json!({
                    "base": { "enum": u64::from(screen_circular) }
                }),
            );
        }
    }
    Ok(())
}

/// Upgrades a raw document in place to [`SCHEMA_VERSION`].
fn migrate(value: &mut Value) -> Result<()> {
    run_migrations(value, MIGRATIONS).map(|_| ())
}

/// Runs a migration chain. Split out so tests can drive a synthetic chain.
fn run_migrations(value: &mut Value, migrations: &[(u32, Migration)]) -> Result<u32> {
    let mut version = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| CoreError::Migration {
            from: 0,
            reason: "document has no schema_version".to_owned(),
        })?;

    if version > SCHEMA_VERSION {
        return Err(CoreError::SchemaTooNew {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }

    while version < SCHEMA_VERSION {
        let Some((_, step)) = migrations.iter().find(|(from, _)| *from == version) else {
            return Err(CoreError::Migration {
                from: version,
                reason: format!("no migration from version {version}"),
            });
        };
        step(value)?;
        version += 1;
        value["schema_version"] = Value::from(version);
    }
    Ok(version)
}

/// Writes a project to `path`, atomically.
///
/// The document is written to a sibling temporary file and renamed into place,
/// so an interrupted save cannot leave a truncated file where a valid project
/// used to be.
pub fn save(project: &Project, path: &Path) -> Result<()> {
    project.validate()?;
    let json = to_canonical_json(project)?;

    let temp = temp_path_for(path);
    write_archive(&temp, &json)?;

    // Rename is atomic within a filesystem; if it fails, the original file is
    // still intact and only the temporary is left behind.
    if let Err(err) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(err.into());
    }
    Ok(())
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

fn write_archive(path: &Path, json: &str) -> Result<()> {
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);

    // A fixed timestamp keeps the archive byte-reproducible. With the current
    // clock baked in, two saves of an unchanged project would differ, which
    // would make "did this actually change?" unanswerable by comparing files.
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());

    zip.start_file(VERSION_ENTRY, options).map_err(zip_err)?;
    zip.write_all(SCHEMA_VERSION.to_string().as_bytes())?;

    zip.start_file(PROJECT_ENTRY, options).map_err(zip_err)?;
    zip.write_all(json.as_bytes())?;

    zip.finish().map_err(zip_err)?;
    Ok(())
}

/// Reads a project from `path`.
pub fn load(path: &Path) -> Result<Project> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_err)?;

    let mut json = String::new();
    archive
        .by_name(PROJECT_ENTRY)
        .map_err(|_| CoreError::Archive(format!("archive has no {PROJECT_ENTRY}")))?
        .read_to_string(&mut json)?;

    from_json(&json)
}

/// Reads only the schema version, without parsing the document.
///
/// Lets the open dialog reject a too-new file immediately rather than after
/// deserialising a large project that was never going to load.
pub fn peek_version(path: &Path) -> Result<u32> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_err)?;
    let mut text = String::new();
    archive
        .by_name(VERSION_ENTRY)
        .map_err(|_| CoreError::Archive(format!("archive has no {VERSION_ENTRY}")))?
        .read_to_string(&mut text)?;
    text.trim()
        .parse()
        .map_err(|_| CoreError::Archive(format!("bad version marker {text:?}")))
}

fn zip_err(err: zip::result::ZipError) -> CoreError {
    CoreError::Archive(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Object;
    use crate::keyframe::Animatable;
    use crate::project::{FieldKind, ProjectSettings, Resolution, StepHours};
    use crate::schema::{PropId, ToolKind};
    use crate::value::{Interpolation, PropValue};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "ve-io-{}-{}",
                std::process::id(),
                crate::id::Id::new().raw()
            ));
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sample() -> Project {
        let mut p = Project::new(
            "Sample",
            ProjectSettings::new(FieldKind::Wind, Resolution::Deg025, StepHours::H3, 24),
        );
        let mut obj = Object::new(ToolKind::Brush, "gale", 24);
        let speed = obj.props.get_mut(PropId::Speed).expect("present");
        speed.set_key(0, PropValue::F32(5.0), Interpolation::EaseInOut);
        speed.set_key(12, PropValue::F32(30.0), Interpolation::Linear);
        p.layers[0].objects.push(obj);
        p
    }

    #[test]
    fn projects_round_trip_through_a_file() {
        let dir = TempDir::new();
        let path = dir.path("test.veproj");
        let project = sample();

        save(&project, &path).unwrap();
        let loaded = load(&path).unwrap();

        assert_eq!(loaded.name, project.name);
        assert_eq!(loaded.object_count(), project.object_count());
        assert_eq!(
            loaded.layers[0].objects[0]
                .props
                .get(PropId::Speed)
                .unwrap()
                .keys()
                .len(),
            2
        );
    }

    /// The M1 acceptance criterion: save, load, save again, and the document
    /// bytes must be identical.
    #[test]
    fn save_load_save_is_byte_identical() {
        let dir = TempDir::new();
        let (a, b) = (dir.path("a.veproj"), dir.path("b.veproj"));

        let project = sample();
        save(&project, &a).unwrap();
        let loaded = load(&a).unwrap();
        save(&loaded, &b).unwrap();

        let json_a = to_canonical_json(&load(&a).unwrap()).unwrap();
        let json_b = to_canonical_json(&loaded).unwrap();
        assert_eq!(
            json_a, json_b,
            "document json drifted across a save/load cycle"
        );

        assert_eq!(
            std::fs::read(&a).unwrap(),
            std::fs::read(&b).unwrap(),
            "archive bytes drifted; the container is not reproducible"
        );
    }

    #[test]
    fn canonical_json_is_stable_across_calls() {
        let project = sample();
        assert_eq!(
            to_canonical_json(&project).unwrap(),
            to_canonical_json(&project).unwrap()
        );
    }

    /// Invariant 1 has to be enforced, not just documented. If someone adds a
    /// preview thumbnail to the archive, this fails.
    #[test]
    fn the_archive_holds_no_unexpected_entries() {
        let dir = TempDir::new();
        let path = dir.path("t.veproj");
        save(&sample(), &path).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_owned()))
            .collect();

        for name in &names {
            let allowed =
                name == PROJECT_ENTRY || name == VERSION_ENTRY || name.starts_with("polars/");
            assert!(
                allowed,
                "unexpected archive entry {name:?} -- rasters are never stored"
            );
        }
        assert!(names.contains(&PROJECT_ENTRY.to_owned()));
    }

    #[test]
    fn the_version_marker_can_be_read_without_parsing() {
        let dir = TempDir::new();
        let path = dir.path("t.veproj");
        save(&sample(), &path).unwrap();
        assert_eq!(peek_version(&path).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(SCHEMA_VERSION + 5);
        let json = serde_json::to_string(&value).unwrap();

        assert!(matches!(
            from_json(&json),
            Err(CoreError::SchemaTooNew { .. })
        ));
    }

    #[test]
    fn a_document_without_a_version_is_refused() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value.as_object_mut().unwrap().remove("schema_version");
        let json = serde_json::to_string(&value).unwrap();
        assert!(matches!(from_json(&json), Err(CoreError::Migration { .. })));
    }

    /// Driven with a synthetic chain so it tests the runner rather than any one
    /// migration: every version below the current one gets a step that counts
    /// itself, and all of them must run, in order.
    #[test]
    fn the_migration_runner_walks_the_chain() {
        fn step(value: &mut Value) -> Result<()> {
            let steps = value["steps"].as_u64().unwrap_or(0);
            value["steps"] = Value::from(steps + 1);
            Ok(())
        }

        let mut value = serde_json::json!({ "schema_version": 0 });
        let chain: Vec<(u32, Migration)> = (0..SCHEMA_VERSION)
            .map(|from| (from, step as Migration))
            .collect();

        let reached = run_migrations(&mut value, &chain).unwrap();
        assert_eq!(reached, SCHEMA_VERSION);
        assert_eq!(value["steps"], Value::from(u64::from(SCHEMA_VERSION)));
        assert_eq!(value["schema_version"], Value::from(SCHEMA_VERSION));
    }

    /// A version 1 document stored one polyline per stroke. Opening one must
    /// produce a single chain, not a stroke with no geometry at all.
    #[test]
    fn a_version_1_stroke_opens_as_one_chain() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(1);
        let object = &mut value["layers"][0]["objects"][0];
        object["geometry"] = serde_json::json!({
            "stroke": { "points": [{ "x": 1000.0, "y": -2000.0 }, { "x": 3000.0, "y": 500.0 }] }
        });

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        let chains = project.layers[0].objects[0].geometry.stroke_chains();
        assert_eq!(chains.len(), 1, "one polyline becomes one chain");
        assert_eq!(chains[0].len(), 2);
        assert_eq!(chains[0][1].x, 3000.0);
    }

    /// A version-2 brush must lose its orphaned `fill_mode` and gain
    /// `brush_shape` at the same value it always rendered as: a round brush.
    #[test]
    fn a_version_2_brush_trades_fill_mode_for_brush_shape() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(2);
        let object = &mut value["layers"][0]["objects"][0];
        assert_eq!(object["tool"], "brush");
        object["props"]["fill_mode"] = serde_json::json!({ "base": { "enum": 0 } });
        object["props"]
            .as_object_mut()
            .unwrap()
            .remove("brush_shape");

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        let props = &project.layers[0].objects[0].props;
        assert!(props.get(PropId::FillMode).is_none(), "the orphan is gone");
        assert_eq!(
            props.get(PropId::BrushShape).map(Animatable::base),
            Some(PropValue::Enum(0)),
            "a migrated brush still paints a round footprint"
        );
    }

    /// A version-3 brush loses `divergence` and `curl`. Removing them from the
    /// table is not enough on its own: `value_at` reads whatever the map holds,
    /// so a leftover entry would go on bending a flow that nothing shows and
    /// nothing can reach.
    #[test]
    fn a_version_3_brush_loses_divergence_and_curl() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(3);
        let object = &mut value["layers"][0]["objects"][0];
        assert_eq!(object["tool"], "brush");
        object["props"]["divergence"] = serde_json::json!({ "base": { "f32": 0.75 } });
        object["props"]["curl"] = serde_json::json!({ "base": { "f32": -1.0 } });

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        let props = &project.layers[0].objects[0].props;
        assert!(
            props.get(PropId::Divergence).is_none(),
            "the orphan is gone"
        );
        assert!(props.get(PropId::Curl).is_none(), "the orphan is gone");
        // And nothing reads them back into the flow.
        assert_eq!(
            props.value_at(ToolKind::Brush, PropId::Curl, 0),
            None,
            "a brush has no curl to resolve at all"
        );
    }

    /// A version-4 circle's `circle_space` becomes `stamp_space`, and the
    /// meaning survives the rename: its variants were in the opposite order, so
    /// carrying the index across would have turned every screen circle into a
    /// geodesic cap.
    #[test]
    fn a_version_4_circle_space_becomes_a_stamp_space() {
        for (was, becomes) in [(0u64, 1u64), (1, 0)] {
            let mut value: Value = serde_json::to_value(sample()).unwrap();
            value["schema_version"] = Value::from(4);
            let object = &mut value["layers"][0]["objects"][0];
            object["tool"] = Value::from("circle");
            object["geometry"] = serde_json::json!("disc");
            object["props"]["circle_space"] = serde_json::json!({ "base": { "enum": was } });

            let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
            let props = &project.layers[0].objects[0].props;
            assert_eq!(
                props.get(PropId::StampSpace).map(Animatable::base),
                Some(PropValue::Enum(becomes as u8)),
                "screen_circular is projected, and geodesic_circular is geodesic"
            );
        }
    }

    /// The circle stamp keeps its own `divergence` and `curl`: it has a centre
    /// to define them about, and stripping them would flatten every rotating
    /// circle into a straight flow.
    #[test]
    fn the_circle_stamp_keeps_its_divergence_and_curl() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(3);
        let object = &mut value["layers"][0]["objects"][0];
        object["tool"] = Value::from("circle");
        object["geometry"] = serde_json::json!("disc");
        object["props"]["curl"] = serde_json::json!({ "base": { "f32": 1.0 } });

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        assert_eq!(
            project.layers[0].objects[0]
                .props
                .get(PropId::Curl)
                .map(Animatable::base),
            Some(PropValue::F32(1.0)),
            "a circle's curl is its own"
        );
    }

    /// The circle stamp keeps its own `fill_mode`: stripping it would turn
    /// every ring and gradient into a plain filled disc.
    #[test]
    fn the_migration_leaves_a_circle_stamps_fill_mode_alone() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(2);
        let mut circle = serde_json::to_value(Object::new(ToolKind::Circle, "eye", 4)).unwrap();
        circle["props"]["fill_mode"] = serde_json::json!({ "base": { "enum": 1 } });
        value["layers"][0]["objects"]
            .as_array_mut()
            .unwrap()
            .push(circle);

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        let circle = &project.layers[0].objects[1];
        assert_eq!(circle.tool, ToolKind::Circle);
        assert_eq!(
            circle.props.get(PropId::FillMode).map(Animatable::base),
            Some(PropValue::Enum(1))
        );
    }

    #[test]
    fn a_gap_in_the_chain_is_an_error() {
        let mut value = serde_json::json!({ "schema_version": 0 });
        let err = run_migrations(&mut value, &[]);
        assert!(matches!(err, Err(CoreError::Migration { from: 0, .. })));
    }

    #[test]
    fn an_archive_without_a_document_is_rejected() {
        let dir = TempDir::new();
        let path = dir.path("empty.veproj");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("readme.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"not a project").unwrap();
            zip.finish().unwrap();
        }
        assert!(matches!(load(&path), Err(CoreError::Archive(_))));
    }

    #[test]
    fn a_file_that_is_not_an_archive_is_rejected() {
        let dir = TempDir::new();
        let path = dir.path("junk.veproj");
        std::fs::write(&path, b"definitely not a zip").unwrap();
        assert!(matches!(load(&path), Err(CoreError::Archive(_))));
    }

    /// An atomic save must not leave its temporary behind on success.
    #[test]
    fn saving_leaves_no_temporary_file() {
        let dir = TempDir::new();
        let path = dir.path("t.veproj");
        save(&sample(), &path).unwrap();

        let leftovers: Vec<String> = std::fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "left temporaries behind: {leftovers:?}"
        );
    }

    /// Loading must repair ids so a later edit cannot collide with the document.
    #[test]
    fn loading_reserves_ids_from_the_file() {
        let dir = TempDir::new();
        let path = dir.path("t.veproj");
        let mut project = sample();
        project.layers[0].objects[0].id = crate::id::Id::from_raw(8_000_000);
        save(&project, &path).unwrap();

        load(&path).unwrap();
        assert!(crate::id::Id::new().raw() > 8_000_000);
    }

    /// Regression guard for the serde_json parser defect documented in
    /// `canonical.rs`. Every `f64` that reaches a project file is set to a
    /// value known to shift by one ULP without canonical rounding. If someone
    /// adds an `f64` field and forgets the rounding helper, extend this test
    /// with it and it will fail until the helper is added.
    #[test]
    fn hostile_floats_survive_a_round_trip() {
        use crate::angle::Angle;
        use crate::document::{Geometry, LocalPoint};
        use crate::geo::LonLat;

        // Values measured to fail a raw serde_json round trip.
        const HOSTILE: [f64; 3] = [
            380_812.358_415_356_84,
            260_785.120_217_839_26,
            -107.943_262_213_740_03,
        ];

        let mut project = sample();
        project.view.zoom = 12.508_890_379_193_765;
        project.view.center = LonLat {
            lon: HOSTILE[2],
            lat: -12.508_890_379_193_765,
        };

        let object = &mut project.layers[0].objects[0];
        object.geometry = Geometry::Stroke {
            chains: vec![vec![
                LocalPoint {
                    x: HOSTILE[0],
                    y: HOSTILE[1],
                },
                LocalPoint {
                    x: HOSTILE[1],
                    y: HOSTILE[0],
                },
            ]],
        };
        object
            .props
            .get_mut(PropId::Direction)
            .expect("present")
            .set_base(PropValue::Angle(Angle::new(HOSTILE[2].abs())));
        object
            .props
            .get_mut(PropId::Target)
            .expect("present")
            .set_base(PropValue::LonLat(LonLat {
                lon: HOSTILE[2],
                lat: 45.123_456_789_012_34,
            }));

        let mut rect = Object::new(ToolKind::ShapeFill, "rect", 24);
        rect.geometry = Geometry::Rect {
            half_width_m: HOSTILE[0],
            half_height_m: HOSTILE[1],
        };
        project.layers[0].objects.push(rect);

        // One cycle canonicalises; every later cycle must be a fixed point.
        let first = to_canonical_json(&project).unwrap();
        let once = from_json(&first).unwrap();
        let second = to_canonical_json(&once).unwrap();
        let twice = from_json(&second).unwrap();
        let third = to_canonical_json(&twice).unwrap();

        assert_eq!(second, third, "json drifted on a second round trip");
        assert_eq!(once, twice, "document drifted on a second round trip");
    }

    #[test]
    fn an_invalid_project_is_not_written() {
        let dir = TempDir::new();
        let path = dir.path("bad.veproj");
        let mut project = sample();
        project.settings.step_count = 0;

        assert!(save(&project, &path).is_err());
        assert!(!path.exists(), "an invalid project must not reach disk");
    }
}
