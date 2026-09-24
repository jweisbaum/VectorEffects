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

use crate::angle::Angle;
use crate::error::{CoreError, Result};
use crate::geo::LonLat;
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
    (5, disc_carries_an_optional_radius),
    (6, no_tool_has_divergence_or_curl),
    (7, the_eraser_is_the_mask),
    (8, modifiers_are_painted_rather_than_stamped),
    (9, a_warp_pushes_to_a_place),
    (10, layers_carry_the_parameter),
    (11, a_turn_has_a_sense_and_an_amount),
    (12, liquify_displacement_is_one_position),
];

/// Join the former two scalar tracks. Matching key times/easing retain their
/// sparse keys; independently edited legacy axes are sampled at each project
/// step, preserving every evaluated frame even when their easing differs.
fn liquify_displacement_is_one_position(value: &mut Value) -> Result<()> {
    use crate::{
        keyframe::Animatable,
        value::{Interpolation, PropValue},
    };
    let Some(layers) = value.get_mut("layers").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for layer in layers {
        let Some(objects) = layer.get_mut("objects").and_then(Value::as_array_mut) else {
            continue;
        };
        for object in objects {
            if object.get("tool").and_then(Value::as_str) != Some("liquify") {
                continue;
            }
            let Some(props) = object.get_mut("props").and_then(Value::as_object_mut) else {
                continue;
            };
            if props.contains_key("displacement_position") {
                continue;
            }
            let axis = |v: Option<Value>| -> Result<Animatable> {
                Ok(match v {
                    Some(v) => serde_json::from_value(v)?,
                    None => Animatable::constant(PropValue::F32(0.0)),
                })
            };
            let x = axis(props.remove("displacement_x_km"))?;
            let y = axis(props.remove("displacement_y_km"))?;
            let pair = |a: PropValue, b: PropValue| {
                PropValue::Offset([a.as_f32().unwrap_or(0.0), b.as_f32().unwrap_or(0.0)])
            };
            let mut joined = Animatable::constant(pair(x.base(), y.base()));
            let same = x.keys().len() == y.keys().len()
                && x.keys()
                    .iter()
                    .zip(y.keys())
                    .all(|(a, b)| a.step == b.step && a.interp == b.interp);
            if same {
                for (a, b) in x.keys().iter().zip(y.keys()) {
                    joined.set_key(a.step, pair(a.value, b.value), a.interp);
                }
            } else {
                let first = x
                    .keys()
                    .iter()
                    .chain(y.keys())
                    .map(|k| k.step)
                    .min()
                    .unwrap_or(0);
                let last = x
                    .keys()
                    .iter()
                    .chain(y.keys())
                    .map(|k| k.step)
                    .max()
                    .unwrap_or(0);
                if last >= crate::project::MAX_STEPS {
                    return Err(CoreError::Migration {
                        from: 12,
                        reason: "Liquify displacement key exceeds the supported timeline".into(),
                    });
                }
                for step in first..=last {
                    joined.set_key(
                        step,
                        pair(x.value_at(step), y.value_at(step)),
                        Interpolation::Linear,
                    );
                }
            }
            props.insert(
                "displacement_position".into(),
                serde_json::to_value(joined)?,
            );
        }
    }
    Ok(())
}

/// Version 11 stored a turn as one signed number of degrees, `turn_deg`.
///
/// Version 12 stores the sense and the amount apart (M29): `turn_sense`, 0
/// clockwise and 1 counter-clockwise, and `turn_amount_deg`, never negative.
/// Every key travels: the amount keeps its interpolation on its magnitude,
/// and the sense takes a stepped key wherever the sign was.
fn a_turn_has_a_sense_and_an_amount(value: &mut Value) -> Result<()> {
    fn split(signed: &Value) -> (Value, Value) {
        let degrees = signed.get("f32").and_then(Value::as_f64).unwrap_or(0.0);
        let sense = u8::from(degrees < 0.0);
        (
            serde_json::json!({ "enum": sense }),
            serde_json::json!({ "f32": degrees.abs() }),
        )
    }
    let Some(layers) = value.get_mut("layers").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for layer in layers {
        let Some(objects) = layer.get_mut("objects").and_then(Value::as_array_mut) else {
            continue;
        };
        for object in objects {
            if object.get("tool").and_then(Value::as_str) != Some("turn") {
                continue;
            }
            let Some(props) = object.get_mut("props").and_then(Value::as_object_mut) else {
                continue;
            };
            let Some(turn) = props.remove("turn_deg") else {
                continue;
            };
            let base = turn.get("base").cloned().unwrap_or(Value::Null);
            let (sense_base, amount_base) = split(&base);
            let keys = turn
                .get("keys")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut sense_keys = Vec::new();
            let mut amount_keys = Vec::new();
            for key in keys {
                let (sense, amount) = split(key.get("value").unwrap_or(&Value::Null));
                let step = key.get("step").cloned().unwrap_or(Value::from(0));
                let interp = key
                    .get("interp")
                    .cloned()
                    .unwrap_or_else(|| Value::String("linear".to_owned()));
                sense_keys
                    .push(serde_json::json!({ "step": step, "value": sense, "interp": "step" }));
                amount_keys
                    .push(serde_json::json!({ "step": step, "value": amount, "interp": interp }));
            }
            let mut sense = serde_json::Map::new();
            sense.insert("base".to_owned(), sense_base);
            if !sense_keys.is_empty() {
                sense.insert("keys".to_owned(), Value::Array(sense_keys));
            }
            let mut amount = serde_json::Map::new();
            amount.insert("base".to_owned(), amount_base);
            if !amount_keys.is_empty() {
                amount.insert("keys".to_owned(), Value::Array(amount_keys));
            }
            if let Some(follow) = turn.get("follow").cloned() {
                amount.insert("follow".to_owned(), follow);
            }
            props.insert("turn_sense".to_owned(), Value::Object(sense));
            props.insert("turn_amount_deg".to_owned(), Value::Object(amount));
        }
    }
    Ok(())
}

/// Version 10 kept the kind of field on the project and one colour scale.
///
/// Version 11 puts the kind on each layer (M29) — a project may hold wind
/// and current layers together, exported as two message sets — so every
/// layer takes the project's kind as its own, and the one scale becomes the
/// pair, the project's kind keeping the number it had and the other kind
/// taking the default it always would have.
fn layers_carry_the_parameter(value: &mut Value) -> Result<()> {
    let kind = value
        .get("settings")
        .and_then(|settings| settings.get("field_kind"))
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "wind".to_owned());
    let is_current = kind == "current";
    if let Some(Value::Array(layers)) = value.get_mut("layers") {
        for layer in layers {
            if let Value::Object(map) = layer {
                map.entry("parameter")
                    .or_insert_with(|| Value::String(kind.clone()));
            }
        }
    }
    if let Some(Value::Object(settings)) = value.get_mut("settings")
        && let Some(Value::Object(scale)) = settings.get_mut("colour_scale")
        && let Some(max) = scale.remove("max_knots")
    {
        let (wind, current) = if is_current {
            (Value::from(60.0), max)
        } else {
            (max, Value::from(6.0))
        };
        scale.insert("wind_knots".to_owned(), wind);
        scale.insert("current_knots".to_owned(), current);
    }
    Ok(())
}

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

/// A warp pushes the field *to a place* rather than along a bearing
/// (spec.md 6.3).
///
/// `distance_km` and `push_bearing` become a `target` position, so that a warp
/// can be set by dragging the field where it should go and so that both ends of
/// the push are animatable positions. The place is where the old pair pointed:
/// the object's own anchor, moved along the bearing by the distance.
fn a_warp_pushes_to_a_place(value: &mut Value) -> Result<()> {
    let Some(layers) = value.get_mut("layers").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for layer in layers {
        let Some(objects) = layer.get_mut("objects").and_then(Value::as_array_mut) else {
            continue;
        };
        for object in objects {
            if object.get("tool").and_then(Value::as_str) != Some("warp") {
                continue;
            }
            let anchor = object
                .pointer("/props/position/base/lon_lat")
                .and_then(|at| {
                    Some(LonLat::new(
                        at.get("lon")?.as_f64()?,
                        at.get("lat")?.as_f64()?,
                    ))
                })
                .and_then(std::result::Result::ok)
                .unwrap_or(LonLat { lon: 0.0, lat: 0.0 });
            let km = object
                .pointer("/props/distance_km/base/f32")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let degrees = object
                .pointer("/props/push_bearing/base/angle")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let target = anchor.destination(Angle::new(degrees), km * 1000.0);

            if let Some(Value::Object(props)) = object.get_mut("props") {
                props.remove("distance_km");
                props.remove("push_bearing");
                props.insert(
                    "push_to".to_owned(),
                    serde_json::json!({
                        "base": { "lon_lat": { "lon": target.lon, "lat": target.lat } }
                    }),
                );
            }
        }
    }
    Ok(())
}

/// The modifiers are painted along a polyline rather than placed by a click
/// (spec.md 6.3).
///
/// They were disc stamps with a typed `diameter_km`; they are now swept stamps
/// with a `size_km`, like the brush and the mask, so that a swathe can be
/// treated in one gesture and two strokes of the same settings merge into one
/// object. A stamped one becomes the stroke it would have been: a chain of a
/// single point at the object's own anchor, which sweeps to exactly the disc it
/// already was.
fn modifiers_are_painted_rather_than_stamped(value: &mut Value) -> Result<()> {
    const MODIFIERS: [&str; 4] = ["intensity", "divergence", "turn", "warp"];
    let Some(layers) = value.get_mut("layers").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for layer in layers {
        let Some(objects) = layer.get_mut("objects").and_then(Value::as_array_mut) else {
            continue;
        };
        for object in objects {
            let is_modifier = object
                .get("tool")
                .and_then(Value::as_str)
                .is_some_and(|tool| MODIFIERS.contains(&tool));
            if !is_modifier {
                continue;
            }
            object["geometry"] = serde_json::json!({
                "stroke": { "chains": [[{ "x": 0.0, "y": 0.0 }]] }
            });
            if let Some(Value::Object(props)) = object.get_mut("props")
                && let Some(diameter) = props.remove("diameter_km")
            {
                props.insert("size_km".to_owned(), diameter);
            }
        }
    }
    Ok(())
}

/// The eraser is called the mask (spec.md 6.2).
///
/// A rename of the tool itself, which is stored on every object it drew, so it
/// is a migration and not a table change: `ToolKind` deserialises from the
/// string, and an object still saying `eraser` would fail to load rather than
/// load as something else. The name is the only thing that changes — the
/// object's properties, geometry and keyframes are the mask's already.
///
/// Object *names* are left alone. "Erase 3" was typed, or accepted, by whoever
/// drew it; renaming a user's own labels is a bigger surprise than an old name
/// in a list.
fn the_eraser_is_the_mask(value: &mut Value) -> Result<()> {
    let Some(layers) = value.get_mut("layers").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for layer in layers {
        let Some(objects) = layer.get_mut("objects").and_then(Value::as_array_mut) else {
            continue;
        };
        for object in objects {
            if object.get("tool").and_then(Value::as_str) == Some("eraser") {
                object["tool"] = Value::String("mask".to_owned());
            }
        }
    }
    Ok(())
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

/// `Geometry::Disc` gained a field, so it is an object rather than a string.
///
/// Version 5 wrote a disc as the bare tag `"disc"`, its size always coming from
/// the `diameter_km` property. The shape fill's circle preset is dragged out
/// instead of typed, so its radius is part of the geometry — which makes the
/// variant a struct, and `"disc"` no longer parses as one.
///
/// Every existing disc is a circle stamp, and a stamp's radius still comes from
/// its property, so the field is simply absent.
fn disc_carries_an_optional_radius(value: &mut Value) -> Result<()> {
    fn convert(node: &mut Value) {
        match node {
            Value::Object(map) => {
                if map.get("geometry").and_then(Value::as_str) == Some("disc") {
                    map.insert("geometry".to_owned(), serde_json::json!({ "disc": {} }));
                }
                for child in map.values_mut() {
                    convert(child);
                }
            }
            Value::Array(items) => items.iter_mut().for_each(convert),
            _ => {}
        }
    }
    convert(value);
    Ok(())
}

/// No tool has `divergence` or `curl` any more (spec.md 6.2, 7.5).
///
/// The brush lost both at version 3 for want of a centre to measure them from;
/// the circle stamp and the shape fill kept theirs, and now nothing has them.
///
/// A migration and not just a table change, for the reason version 3's was:
/// [`crate::schema::PropertyMap::value_at`] returns whatever the map holds and
/// only falls back to the schema when the key is absent. A leftover entry would
/// go on bending the flow of an object whose panel no longer shows it and whose
/// tool no longer defines it — invisible, unreachable, and still in the export.
///
/// Applied to every object rather than to the two tools that had them, because
/// the property is gone from the model entirely: after this, an object holding
/// one would fail to deserialise at all.
fn no_tool_has_divergence_or_curl(value: &mut Value) -> Result<()> {
    let Some(layers) = value.get_mut("layers").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for layer in layers {
        let Some(objects) = layer.get_mut("objects").and_then(Value::as_array_mut) else {
            continue;
        };
        for object in objects {
            if let Some(Value::Object(props)) = object.get_mut("props") {
                props.remove("divergence");
                props.remove("curl");
            }
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
    write_archive(&temp, &json, project)?;

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

/// Directory of neighbour-set entries inside the archive.
const REGRID_PREFIX: &str = "regrid/";

/// Directory of captured-field entries inside the archive (spec.md 8.5).
const CAPTURE_PREFIX: &str = "captures/";

fn write_archive(path: &Path, json: &str, project: &Project) -> Result<()> {
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

    write_regrid(&mut zip, project)?;
    write_captures(&mut zip, project)?;

    zip.finish().map_err(zip_err)?;
    Ok(())
}

/// Writes the derived neighbour sets as their own entries.
///
/// Binary, and far too large for the JSON: a global 0.1° set is 19 million
/// indices. Each entry carries the mesh size it was built against so the
/// reader can check it rather than trust the file name.
fn write_regrid(zip: &mut zip::ZipWriter<std::fs::File>, project: &Project) -> Result<()> {
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for (key, set) in &project.regrid {
        zip.start_file(format!("{REGRID_PREFIX}{key}.bin"), options)
            .map_err(zip_err)?;
        zip.write_all(&(set.cells() as u32).to_le_bytes())?;
        zip.write_all(&set.encode())?;
    }
    Ok(())
}

/// Writes the captured fields the document still refers to.
///
/// **Only the ones still referenced.** A capture is user content, but a
/// capture nothing points at is a patch that was deleted, and carrying it
/// would make a project grow forever with fields nobody can see. Undo holds
/// the object, not the file, so an undone delete finds its samples again from
/// the map in memory — and a project saved between the two is the one case
/// where the entry is gone, which is the same bargain a GRIB layer's path
/// already makes.
///
/// Already compressed, so the entry is **stored** rather than deflated: lz4
/// over deflate is two passes for a percent (spec.md 8.5).
fn write_captures(zip: &mut zip::ZipWriter<std::fs::File>, project: &Project) -> Result<()> {
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    for hash in referenced_captures(project) {
        let Some(capture) = project.captures.get(&hash) else {
            continue;
        };
        zip.start_file(format!("{CAPTURE_PREFIX}{hash}.vecap"), options)
            .map_err(zip_err)?;
        capture.write(&mut *zip)?;
    }
    Ok(())
}

/// Every capture hash some object still refers to, sorted and unique.
///
/// Sorted so the archive's entry order is the document's and not a hash map's:
/// two saves of one project must produce the same bytes (invariant 4).
fn referenced_captures(project: &Project) -> Vec<String> {
    let mut hashes: Vec<String> = project
        .layers
        .iter()
        .flat_map(|layer| layer.objects.iter())
        .filter_map(|object| object.capture.clone())
        .collect();
    hashes.sort();
    hashes.dedup();
    hashes
}

/// Reads the captured fields back.
///
/// A capture that will not decode is **skipped**, not fatal: the patch that
/// referred to it draws nothing and is marked, and everything else the user
/// authored still opens. Refusing the project over one bad entry would lose
/// the work around it to save the work in it.
fn read_captures(
    archive: &mut zip::ZipArchive<std::fs::File>,
    project: &mut Project,
) -> Result<()> {
    let names: Vec<String> = archive
        .file_names()
        .filter(|n| n.starts_with(CAPTURE_PREFIX) && n.ends_with(".vecap"))
        .map(str::to_owned)
        .collect();
    for name in names {
        let Ok(entry) = archive.by_name(&name) else {
            continue;
        };
        let Ok(capture) = crate::capture::Capture::read(entry) else {
            continue;
        };
        // Keyed by the capture's *own* hash rather than by the file name, so a
        // renamed entry cannot make a patch draw somebody else's field.
        project
            .captures
            .insert(capture.hash.clone(), std::sync::Arc::new(capture));
    }
    Ok(())
}

/// Reads the neighbour sets back, dropping any that do not match the project.
///
/// A set is derived state and a stale one is only a wasted rebuild, so a bad
/// entry is skipped rather than failing the open: the project still holds
/// everything the user authored.
fn read_regrid(archive: &mut zip::ZipArchive<std::fs::File>, project: &mut Project) -> Result<()> {
    let target = project.settings.resolution.target_grid();
    let names: Vec<String> = archive
        .file_names()
        .filter(|n| n.starts_with(REGRID_PREFIX) && n.ends_with(".bin"))
        .map(str::to_owned)
        .collect();
    for name in names {
        let mut bytes = Vec::new();
        {
            let mut entry = match archive.by_name(&name) {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            entry.read_to_end(&mut bytes)?;
        }
        let Some((head, rest)) = bytes.split_at_checked(4) else {
            continue;
        };
        let cells = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let Ok(set) = crate::regrid::Neighbours::decode(rest, &target, cells) else {
            continue;
        };
        let key = name
            .trim_start_matches(REGRID_PREFIX)
            .trim_end_matches(".bin")
            .to_owned();
        project.regrid.insert(key, std::sync::Arc::new(set));
    }
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

    let mut project = from_json(&json)?;
    read_regrid(&mut archive, &mut project)?;
    read_captures(&mut archive, &mut project)?;
    Ok(project)
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
    use crate::document::{Layer, Object};
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

    /// The motion flags are a document field like any other, and an object
    /// that was never told to move must save nothing at all (spec.md 9.3).
    #[test]
    fn motion_flags_round_trip_and_a_still_object_writes_none() {
        use crate::document::MotionFlags;

        let dir = TempDir::new();
        let path = dir.path("motion.veproj");
        let mut project = sample();
        project.layers[0].objects[0].motion = MotionFlags {
            position: true,
            scale: true,
            ..Default::default()
        };
        save(&project, &path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded.layers[0].objects[0].motion,
            MotionFlags {
                position: true,
                rotation: false,
                scale: true,
            }
        );

        // An object that never moves writes no `motion` key, so every project
        // made before the field existed reads back byte-identically.
        let still = dir.path("still.veproj");
        let mut project = sample();
        project.layers[0].objects[0].motion = MotionFlags::default();
        save(&project, &still).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&still).unwrap()).unwrap();
        let mut json = String::new();
        {
            use std::io::Read;
            archive
                .by_name(PROJECT_ENTRY)
                .unwrap()
                .read_to_string(&mut json)
                .unwrap();
        }
        assert!(
            !json.contains("\"motion\""),
            "a still object wrote a motion field"
        );
        assert!(!load(&still).unwrap().layers[0].objects[0].motion.any());
    }

    /// A neighbour set survives a save and reopen, and a stale one is dropped.
    ///
    /// The set is derived state, not the user's work, so a project whose
    /// resolution no longer matches must open cleanly and rebuild rather than
    /// refuse — but it must not open with a set that maps onto some other
    /// grid, which would put the imported field in the wrong places.
    #[test]
    fn a_neighbour_set_travels_with_the_project_unless_it_no_longer_fits() {
        use crate::regrid::{CellCentres, Neighbours};

        let dir = TempDir::new();
        let path = dir.path("regrid.veproj");
        let mut project = sample();
        let target = project.settings.resolution.target_grid();

        // A tiny mesh: three cells is enough to have three neighbours.
        let centres = CellCentres::new(
            vec![10.0, -10.0, 40.0, -40.0],
            vec![0.0, 20.0, -60.0, 100.0],
        )
        .unwrap();
        let set = Neighbours::build(&centres, &target);
        let key = Neighbours::cache_key(b"a-mesh", &target);
        project
            .regrid
            .insert(key.clone(), std::sync::Arc::new(set.clone()));

        save(&project, &path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.regrid.len(), 1);
        assert_eq!(**loaded.regrid.get(&key).unwrap(), set);

        // The same archive opened as a project of a different resolution: the
        // set is for the wrong lattice and is left behind.
        let mut other = load(&path).unwrap();
        other.settings.resolution = if other.settings.resolution == Resolution::Deg1 {
            Resolution::Deg05
        } else {
            Resolution::Deg1
        };
        let moved = dir.path("moved.veproj");
        save(&other, &moved).unwrap();
        let reopened = load(&moved).unwrap();
        assert!(
            reopened.regrid.is_empty(),
            "a set built for another grid must not be reused"
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

    /// A GRIB layer is saved as a reference to its file and nothing else.
    /// The decoded field must not reach the archive (invariants 1 and 2), and
    /// the reference must survive so the app can read the file again on open.
    #[test]
    fn a_grib_layer_keeps_its_reference_and_drops_its_samples() {
        use crate::document::LayerSource;
        use crate::raster::{RasterFrame, RasterGrid, RasterSequence};
        use std::sync::Arc;

        let grid = RasterGrid::new(2, 2, 0.0, 1.0, 1.0, 1.0, vec![[1.0, 2.0]; 4]).unwrap();
        let sequence = RasterSequence::new(
            FieldKind::Current,
            vec![RasterFrame {
                offset_hours: 0.0,
                valid_unix_s: 0,
                grid: Arc::new(grid),
            }],
        )
        .unwrap();
        let mut project = sample();
        project.layers.push(Layer::from_grib(
            "Currents",
            PathBuf::from("/data/rtofs.grib2"),
            Arc::new(sequence),
            false,
        ));

        let json = to_canonical_json(&project).unwrap();
        assert!(json.contains("rtofs.grib2"), "the path is the reference");
        assert!(
            !json.contains("raster"),
            "no samples in the document: {json}"
        );

        let dir = TempDir::new();
        let path = dir.path("grib.veproj");
        save(&project, &path).unwrap();
        let loaded = load(&path).unwrap();
        let layer = &loaded.layers[1];
        assert_eq!(
            layer.source,
            LayerSource::Grib {
                path: PathBuf::from("/data/rtofs.grib2"),
                field: FieldKind::Current,
            }
        );
        assert!(
            layer.raster.is_none(),
            "the field is re-read by the app, never stored"
        );
        assert!(!layer.visible);
        assert!(loaded.layers[0].source.is_painted());
    }

    /// A history layer is a GRIB layer plus where it came from (spec.md
    /// 4.10, M38), and both halves have to survive the archive: the path,
    /// because the field is re-read from it on open, and the provenance,
    /// because a layer that forgot which hours it holds cannot say what it
    /// is. Its samples stay out, exactly as a GRIB layer's do.
    #[test]
    fn a_history_layer_keeps_its_archive_and_its_hours() {
        use crate::document::LayerSource;
        use crate::raster::{RasterFrame, RasterGrid, RasterSequence};
        use std::sync::Arc;

        let grid = RasterGrid::new(2, 2, 0.0, 1.0, 1.0, 1.0, vec![[3.0, 4.0]; 4]).unwrap();
        let sequence = RasterSequence::new(
            FieldKind::Wind,
            vec![RasterFrame {
                offset_hours: 0.0,
                valid_unix_s: 1_600_000_000,
                grid: Arc::new(grid),
            }],
        )
        .unwrap();
        let mut project = sample();
        project.layers.push(Layer::from_history(
            "ERA5 10 m wind",
            PathBuf::from("/history/era5-wind-1600000000-1600086400.grib2"),
            Arc::new(sequence),
            "era5-wind",
            1_600_000_000,
            1_600_086_400,
        ));

        let json = to_canonical_json(&project).unwrap();
        assert!(json.contains("era5-wind"), "the archive is the provenance");

        let dir = TempDir::new();
        let path = dir.path("history.veproj");
        save(&project, &path).unwrap();
        let loaded = load(&path).unwrap();
        let layer = &loaded.layers[1];
        assert_eq!(
            layer.source,
            LayerSource::Zarr {
                path: PathBuf::from("/history/era5-wind-1600000000-1600086400.grib2"),
                field: FieldKind::Wind,
                archive: "era5-wind".to_owned(),
                start_unix_s: 1_600_000_000,
                end_unix_s: 1_600_086_400,
            }
        );
        assert!(
            layer.raster.is_none(),
            "the field is re-read from the file, never stored (invariants 1 and 2)"
        );
        // Everything a GRIB layer offers, it offers: the same field kind, the
        // same file to read, and the same answer to "is this imported".
        assert_eq!(layer.parameter(), FieldKind::Wind);
        assert!(layer.is_grib());
        assert!(layer.source.raster_file().is_some());
        assert!(layer.visible, "a fetched layer is shown");
    }

    /// The gradient a project is drawn with travels with the project, and a
    /// name this build does not know travels too: a file written by a later
    /// version must go back to that version saying what it said. Drawing it
    /// with the default is the right answer to "I do not know this colour";
    /// rewriting the file to say so is not.
    #[test]
    fn a_gradient_survives_a_round_trip_even_when_it_is_unknown() {
        use crate::colour::{ColourGradients, gradient_or_default};

        let mut project = sample();
        project.settings.colour_gradients = Some(ColourGradients {
            wind: "viridis".to_owned(),
            current: "from-a-later-version".to_owned(),
        });

        let dir = TempDir::new();
        let path = dir.path("gradients.veproj");
        save(&project, &path).unwrap();
        let loaded = load(&path).unwrap();
        let chosen = loaded.settings.gradients();
        assert_eq!(chosen.wind, "viridis");
        assert_eq!(
            chosen.current, "from-a-later-version",
            "an unfamiliar name is kept, not replaced"
        );
        // And it is drawn with something rather than not at all.
        assert_eq!(gradient_or_default(&chosen.current).id, "vector");
        assert_eq!(gradient_or_default(&chosen.wind).id, "viridis");
    }

    /// A project that names none is drawn exactly as it was before the
    /// setting existed, so no file changes appearance by being opened.
    #[test]
    fn a_project_without_a_gradient_takes_the_defaults() {
        let dir = TempDir::new();
        let path = dir.path("no-gradient.veproj");
        let project = sample();
        assert!(project.settings.colour_gradients.is_none());
        let json = to_canonical_json(&project).unwrap();
        assert!(
            !json.contains("colour_gradients"),
            "and writes nothing for it: {json}"
        );
        save(&project, &path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded.settings.gradients(),
            crate::colour::ColourGradients::default()
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
    fn old_liquify_axes_become_one_track_without_changing_any_frame() {
        for matching in [true, false] {
            let mut project = sample();
            let mut object = Object::new(ToolKind::Liquify, "Legacy relocation", 24);
            object.props.remove(PropId::DisplacementPosition);
            let mut x = Animatable::constant(PropValue::F32(50.0));
            let mut y = Animatable::constant(PropValue::F32(-20.0));
            x.set_key(2, PropValue::F32(100.0), Interpolation::EaseInOut);
            x.set_key(12, PropValue::F32(300.0), Interpolation::Linear);
            y.set_key(
                if matching { 2 } else { 4 },
                PropValue::F32(-50.0),
                if matching {
                    Interpolation::EaseInOut
                } else {
                    Interpolation::Step
                },
            );
            y.set_key(12, PropValue::F32(120.0), Interpolation::Linear);
            object.props.insert(PropId::DisplacementXKm, x.clone());
            object.props.insert(PropId::DisplacementYKm, y.clone());
            let id = object.id;
            project.layers[0].objects.push(object);
            let mut old = serde_json::to_value(&project).unwrap();
            old["schema_version"] = Value::from(12);
            let loaded = from_json(&old.to_string()).unwrap();
            let props = &loaded.object(id).unwrap().props;
            let joined = props.get(PropId::DisplacementPosition).unwrap();
            assert_eq!(joined.base(), PropValue::Offset([50.0, -20.0]));
            for step in 0..24 {
                assert_eq!(
                    joined.value_at(step).as_offset().unwrap(),
                    [
                        x.value_at(step).as_f32().unwrap(),
                        y.value_at(step).as_f32().unwrap()
                    ]
                );
            }
            if matching {
                assert_eq!(joined.keys().len(), 2);
            }
            assert!(props.get(PropId::DisplacementXKm).is_none());
            assert!(props.get(PropId::DisplacementYKm).is_none());
            assert_eq!(
                from_json(&to_canonical_json(&loaded).unwrap()).unwrap(),
                loaded
            );
        }
    }

    #[test]
    fn liquify_migration_refuses_unbounded_legacy_key_ranges() {
        let mut value = serde_json::json!({"layers":[{"objects":[{
            "tool":"liquify", "props":{"displacement_x_km":{
                "base":{"f32":0.0}, "keys":[{"step":u32::MAX,
                "value":{"f32":1.0}, "interp":"linear"}]
            }}
        }]}]});
        assert!(liquify_displacement_is_one_position(&mut value).is_err());
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
    /// A version-11 turn of −70° opens as a counter-clockwise turn of 70°,
    /// keys and all (M29).
    #[test]
    fn a_version_11_turn_splits_into_a_sense_and_an_amount() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(11);
        let object = &mut value["layers"][0]["objects"][0];
        object["tool"] = Value::String("turn".to_owned());
        object["props"]["turn_deg"] = serde_json::json!({
            "base": { "f32": -70.0 },
            "keys": [
                { "step": 0, "value": { "f32": -70.0 }, "interp": "linear" },
                { "step": 5, "value": { "f32": 20.0 }, "interp": "linear" }
            ]
        });

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        let object = &project.layers[0].objects[0];
        assert_eq!(object.tool, ToolKind::Turn);
        let sense = object.props.get(PropId::TurnSense).expect("a sense");
        let amount = object.props.get(PropId::TurnAmountDeg).expect("an amount");
        assert_eq!(
            sense.value_at(0),
            crate::PropValue::Enum(1),
            "counter-clockwise"
        );
        assert_eq!(
            sense.value_at(5),
            crate::PropValue::Enum(0),
            "clockwise by step 5"
        );
        assert_eq!(amount.value_at(0), crate::PropValue::F32(70.0));
        assert_eq!(amount.value_at(5), crate::PropValue::F32(20.0));
    }

    /// A version-10 current project opens with every layer a current layer
    /// and its one scale become the pair's current end (M29).
    #[test]
    fn a_version_10_project_gives_its_kind_to_its_layers() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(10);
        value["settings"]["field_kind"] = Value::String("current".to_owned());
        value["settings"]["colour_scale"] = serde_json::json!({ "max_knots": 8.0 });
        if let Value::Array(layers) = &mut value["layers"] {
            for layer in layers {
                layer.as_object_mut().unwrap().remove("parameter");
            }
        }

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        assert!(
            project
                .layers
                .iter()
                .all(|layer| layer.parameter() == crate::project::FieldKind::Current),
            "every layer took the project's kind"
        );
        let scale = project.settings.scale();
        assert_eq!(
            scale.current_knots, 8.0,
            "the one scale was the current one"
        );
        assert_eq!(scale.wind_knots, 60.0, "and wind takes the default");
    }

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

    /// A warp saved before the liquify existed opens as the warp it was, with
    /// its mode and its push intact: the liquify is a *new* kind, so the
    /// schema version did not move and nothing about a warp is reinterpreted
    /// (spec.md 6.3, M17).
    #[test]
    fn a_warp_from_before_the_liquify_opens_unchanged() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        let object = &mut value["layers"][0]["objects"][0];
        object["tool"] = Value::String("warp".to_owned());
        object["name"] = Value::String("Warp 1".to_owned());

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("opens");
        let object = &project.layers[0].objects[0];
        assert_eq!(object.tool, ToolKind::Warp);
        assert_eq!(object.name, "Warp 1");
        assert!(
            object.props.get(PropId::WarpMode).is_some(),
            "the warp keeps its mode"
        );
        assert!(
            object.props.get(PropId::Strength).is_none(),
            "and does not acquire the liquify's strength"
        );
    }

    /// A version-7 eraser opens as a mask, with everything it drew intact. The
    /// tool's name is stored on every object, so without the migration the file
    /// would not deserialise at all.
    #[test]
    fn a_version_7_eraser_opens_as_a_mask() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(7);
        let object = &mut value["layers"][0]["objects"][0];
        object["tool"] = Value::String("eraser".to_owned());
        object["name"] = Value::String("Erase 1".to_owned());

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        let object = &project.layers[0].objects[0];
        assert_eq!(object.tool, ToolKind::Mask);
        assert_eq!(
            object.name, "Erase 1",
            "a typed name is the user's, not ours"
        );
        assert_eq!(
            object.props.get(PropId::Invert).map(Animatable::base),
            Some(PropValue::Bool(false)),
            "an old mask covers what it was drawn over, as it always did"
        );
    }

    /// A version-9 warp keeps pushing where it pushed: the distance and the
    /// bearing become the place they pointed at.
    #[test]
    fn a_version_9_warp_pushes_to_where_it_used_to_point() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(9);
        let object = &mut value["layers"][0]["objects"][0];
        object["tool"] = Value::String("warp".to_owned());
        object["geometry"] =
            serde_json::json!({ "stroke": { "chains": [[{ "x": 0.0, "y": 0.0 }]] } });
        object["props"]["position"] = serde_json::json!({
            "base": { "lon_lat": { "lon": 0.0, "lat": 0.0 } }
        });
        object["props"]["distance_km"] = serde_json::json!({ "base": { "f32": 111.19492 } });
        object["props"]["push_bearing"] = serde_json::json!({ "base": { "angle": 90.0 } });

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        let props = &project.layers[0].objects[0].props;
        let PropValue::LonLat(target) = props
            .get(PropId::PushTo)
            .map(Animatable::base)
            .expect("a target")
        else {
            panic!("a target is a position");
        };
        // A degree of longitude at the equator, due east of the anchor.
        assert!(
            (target.lon - 1.0).abs() < 1e-3 && target.lat.abs() < 1e-6,
            "pushed to {target:?}"
        );
    }

    /// A version-8 modifier stamp becomes the stroke it would have been: one
    /// point, at its own anchor, swept to the same disc — and its typed
    /// diameter becomes the size that sweeps it, keeping the object the size it
    /// was drawn.
    #[test]
    fn a_version_8_modifier_stamp_opens_as_a_one_point_stroke() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(8);
        let object = &mut value["layers"][0]["objects"][0];
        object["tool"] = Value::String("intensity".to_owned());
        object["geometry"] = serde_json::json!({ "disc": {} });
        object["props"]["diameter_km"] = serde_json::json!({ "base": { "f32": 1500.0 } });

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        let object = &project.layers[0].objects[0];
        let chains = object.geometry.stroke_chains();
        assert_eq!(chains.len(), 1, "one stamp is one chain");
        assert_eq!(chains[0].len(), 1, "of one point");
        assert_eq!(
            object.props.get(PropId::SizeKm).map(Animatable::base),
            Some(PropValue::F32(1500.0)),
            "the diameter it was drawn at is the size that sweeps it"
        );
        assert!(object.props.get(PropId::DiameterKm).is_none());
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
        // Named rather than looked up by id: the ids are gone from the model,
        // which is what version 7 finished. What has to survive is that a
        // document carrying them still opens, and opens without them.
        let props = &project.layers[0].objects[0].props;
        for name in ["Divergence", "Curl"] {
            assert!(
                props.iter().all(|(id, _)| format!("{id:?}") != name),
                "the orphan {name} is gone"
            );
        }
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

    /// A version-5 disc was the bare tag `"disc"`, and every one of them is a
    /// circle stamp whose radius comes from `diameter_km`. After the migration
    /// it must still be a disc, and must still take its size from there — a
    /// radius appearing in the geometry would override the property and freeze
    /// the diameter an animation was driving.
    #[test]
    fn a_version_5_disc_gains_no_radius_of_its_own() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(5);
        let object = &mut value["layers"][0]["objects"][0];
        object["tool"] = Value::from("circle");
        object["geometry"] = serde_json::json!("disc");
        object["props"]["diameter_km"] = serde_json::json!({ "base": { "f32": 800.0 } });

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        assert_eq!(
            project.layers[0].objects[0].geometry,
            crate::document::Geometry::Disc { radius_m: None },
            "a stamp's size stays in its property"
        );
    }

    /// Version 6 was the last with `divergence` and `curl`, on the circle stamp
    /// and the shape fill. A leftover entry is not inert: `value_at` returns
    /// whatever the map holds, so it would go on bending a flow that no panel
    /// shows and no tool defines — and after the property left the model, it
    /// would not even deserialise.
    #[test]
    fn a_version_6_circle_loses_its_divergence_and_curl() {
        let mut value: Value = serde_json::to_value(sample()).unwrap();
        value["schema_version"] = Value::from(6);
        let object = &mut value["layers"][0]["objects"][0];
        object["tool"] = Value::from("circle");
        object["geometry"] = serde_json::json!({ "disc": {} });
        object["props"]["curl"] = serde_json::json!({ "base": { "f32": 1.0 } });
        object["props"]["divergence"] = serde_json::json!({ "base": { "f32": -0.5 } });

        let project = from_json(&serde_json::to_string(&value).unwrap()).expect("migrates");
        let props = &project.layers[0].objects[0].props;
        for name in ["Divergence", "Curl"] {
            assert!(
                props.iter().all(|(id, _)| format!("{id:?}") != name),
                "{name} survived the migration"
            );
        }
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
        // A follow offset is two more f64s that reach the file (spec.md 9.3).
        object
            .props
            .get_mut(PropId::Position)
            .expect("present")
            .set_follow(Some(crate::keyframe::Follow {
                primary: crate::id::Id::from_raw(7),
                offset: crate::keyframe::FollowOffset::Position {
                    distance_m: HOSTILE[0],
                    bearing_deg: HOSTILE[2].abs(),
                },
            }));

        let mut animated = crate::shape_animation::ShapeAnimation::new(
            vec![vec![
                LocalPoint::new(HOSTILE[0], HOSTILE[1]),
                LocalPoint::new(-HOSTILE[1], HOSTILE[0]),
                LocalPoint::new(HOSTILE[1], -HOSTILE[0]),
            ]],
            HOSTILE[0],
        );
        animated.rings[0][0].set(3, LocalPoint::new(HOSTILE[1], HOSTILE[2]));
        object.shape_animation = Some(animated);

        // An erasure's radius and its centreline reach the file (M29).
        object.erased.push(crate::document::Erasure {
            contour: Vec::new(),
            chains: vec![vec![LocalPoint {
                x: HOSTILE[1],
                y: HOSTILE[0],
            }]],
            radius_m: HOSTILE[0],
            square: true,
            feather: 0.25,
            step: Some(3),
        });

        // A measurement's points and its ring interval reach the file too
        // (spec.md 10, M8).
        project.annotations.measurements = vec![
            crate::annotation::Annotation {
                id: crate::id::Id::from_raw(31),
                measurement: crate::annotation::Measurement::Dividers {
                    points: vec![
                        LonLat {
                            lon: HOSTILE[2],
                            lat: 45.123_456_789_012_34,
                        },
                        LonLat {
                            lon: -12.508_890_379_193_765,
                            lat: HOSTILE[2].abs() / 2.0,
                        },
                    ],
                },
            },
            crate::annotation::Annotation {
                id: crate::id::Id::from_raw(32),
                measurement: crate::annotation::Measurement::Rings {
                    centre: LonLat {
                        lon: HOSTILE[2],
                        lat: -12.508_890_379_193_765,
                    },
                    interval_m: HOSTILE[0],
                    count: 3,
                },
            },
        ];

        let mut rect = Object::new(ToolKind::ShapeFill, "rect", 24);
        rect.geometry = Geometry::Rect {
            half_width_m: HOSTILE[0],
            half_height_m: HOSTILE[1],
        };
        project.layers[0].objects.push(rect);

        // A dragged-out circle's radius is geometry, so it needs the same
        // quantisation the rectangle's extents get.
        let mut circle = Object::new(ToolKind::ShapeFill, "circle", 24);
        circle.geometry = Geometry::Disc {
            radius_m: Some(HOSTILE[1]),
        };
        project.layers[0].objects.push(circle);

        // A layer's own erasures reach the file as well: an imported field's
        // are the only ones with no object to hang on (M29), and the space
        // one is cut in is part of what it is (M67).
        project.layers[0]
            .erased
            .push(crate::document::RasterErasure {
                projection_origin: None,
                projection: 0,
                chains: vec![vec![LonLat {
                    lon: HOSTILE[2],
                    lat: 45.123_456_789_012_34,
                }]],
                radius_m: HOSTILE[1],
                square: false,
                projected: true,
                feather: 0.75,
                step: None,
            });

        // A GIS layer's line width and fill opacity reach the file
        // (spec.md 4.11): two more f64s that need the same quantisation.
        project.layers.push({
            let mut layer = crate::document::Layer::new("Survey");
            layer.source = crate::document::LayerSource::Gis {
                path: std::path::PathBuf::from("/tmp/survey.geojson"),
                colour: "#c8a050".to_owned(),
                width_px: HOSTILE[2].abs() / 100.0,
                fill_opacity: 0.250_000_000_000_000_04,
            };
            layer
        });

        // An image layer's control points reach the file too: two more pairs
        // of f64s (spec.md 4.9), chosen with the same hostile digit counts as
        // `control_points_survive_a_round_trip`.
        project.layers.push({
            let mut layer = crate::document::Layer::new("Chart");
            layer.source = crate::document::LayerSource::Image {
                path: std::path::PathBuf::from("/tmp/chart.png"),
                placement: crate::document::Placement::spanning(-71.0, 42.0, -70.0, 41.0, 800, 600),
                opacity: 0.75,
                control_points: vec![
                    crate::document::ControlPoint {
                        u: 1234.5678912,
                        v: 98.7654321,
                        lon: -70.1234567891,
                        lat: 41.9876543211,
                    },
                    crate::document::ControlPoint {
                        u: 0.0000012,
                        v: 65535.9999992,
                        lon: 179.9999999991,
                        lat: -89.9999999991,
                    },
                ],
            };
            layer
        });

        // One cycle canonicalises; every later cycle must be a fixed point.
        let first = to_canonical_json(&project).unwrap();
        let once = from_json(&first).unwrap();
        let second = to_canonical_json(&once).unwrap();
        let twice = from_json(&second).unwrap();
        let third = to_canonical_json(&twice).unwrap();

        assert_eq!(second, third, "json drifted on a second round trip");
        assert_eq!(once, twice, "document drifted on a second round trip");
        // Named rather than left to the comparison above: a field that
        // defaulted back to false would still round-trip as a fixed point.
        assert!(
            once.layers[0].erased[0].projected,
            "the erasure's space did not survive the file"
        );
        assert!(
            matches!(
                once.layers
                    .get(once.layers.len() - 2)
                    .map(|layer| &layer.source),
                Some(crate::document::LayerSource::Gis { .. })
            ),
            "the GIS layer did not survive the file"
        );
        assert!(
            matches!(
                once.layers.last().map(|layer| &layer.source),
                Some(crate::document::LayerSource::Image { control_points, .. })
                    if control_points.len() == 2
            ),
            "the image layer's control points did not survive the file"
        );
    }

    /// Control points are document state and must survive a save and a load
    /// exactly. Every literal here has one more decimal digit than its
    /// field's canonical precision keeps — `RATIO_PLACES` (6) for `u`/`v`,
    /// `DEGREE_PLACES` (9) for `lon`/`lat` — and a nonzero trailing digit, so
    /// canonical rounding is guaranteed to change every one of the eight
    /// values, not just one of them. That is what makes each field's helper
    /// individually load-bearing: dropping any single `#[serde(with = ...)]`
    /// falls back to a raw `f64`, which round-trips a value this exact
    /// (`serde_json`'s writer is correct, and the workspace's
    /// `float_roundtrip` feature makes its parser correct too) losslessly —
    /// so only the deliberate quantisation below can be what changes it.
    ///
    /// The expected values are rounded through `canonical::ratio`/`degrees`
    /// directly rather than compared against the raw input: canonical
    /// rounding is deliberately lossy (it quantises so `serde_json`'s parser
    /// stays correct without the feature), so `v: 98.7654321` legitimately
    /// becomes `98.765432` once it has a helper. Comparing to the un-rounded
    /// input would fail even with a correct implementation; comparing to the
    /// rounded value fails if the helper is missing, since then the field
    /// would still hold the raw, un-rounded input.
    #[test]
    fn control_points_survive_a_round_trip() {
        use crate::canonical::{degrees, ratio};
        use crate::document::ControlPoint;
        let points = vec![
            ControlPoint {
                u: 1234.5678912,
                v: 98.7654321,
                lon: -70.1234567891,
                lat: 41.9876543211,
            },
            ControlPoint {
                u: 0.0000012,
                v: 65535.9999992,
                lon: 179.9999999991,
                lat: -89.9999999991,
            },
        ];
        let source = crate::document::LayerSource::Image {
            path: std::path::PathBuf::from("/tmp/chart.png"),
            placement: crate::document::Placement::spanning(-71.0, 42.0, -70.0, 41.0, 800, 600),
            opacity: 0.75,
            control_points: points.clone(),
        };
        let json = serde_json::to_string(&source).expect("serialise");
        let back: crate::document::LayerSource = serde_json::from_str(&json).expect("deserialise");
        let crate::document::LayerSource::Image {
            control_points: back,
            ..
        } = back
        else {
            panic!("not an image");
        };
        let expected: Vec<ControlPoint> = points
            .iter()
            .map(|p| ControlPoint {
                u: ratio(p.u),
                v: ratio(p.v),
                lon: degrees(p.lon),
                lat: degrees(p.lat),
            })
            .collect();
        assert_eq!(
            back, expected,
            "a control point did not round-trip to canonical precision"
        );
    }

    /// An older project has no control points and must open exactly as before.
    #[test]
    fn a_project_without_control_points_opens_with_none() {
        let json = r#"{"kind":"image","path":"/tmp/chart.png",
            "placement":{"a":1.0,"b":0.0,"c":-71.0,"d":0.0,"e":-1.0,"f":42.0},
            "opacity":1.0}"#;
        let back: crate::document::LayerSource = serde_json::from_str(json).expect("deserialise");
        let crate::document::LayerSource::Image { control_points, .. } = back else {
            panic!("not an image");
        };
        assert!(control_points.is_empty());
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
