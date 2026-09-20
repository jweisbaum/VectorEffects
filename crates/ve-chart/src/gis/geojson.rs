//! GeoJSON (RFC 7946).
//!
//! The coordinate reference system is longitude and latitude on WGS 84 and
//! the format says so; a file that carries the withdrawn `crs` member
//! naming anything else is refused rather than drawn in the wrong place.

use serde_json::Value as Json;

use crate::error::{ChartError, Result};
use crate::geometry::{Feature, Geometry, Value};

/// Reads a GeoJSON document.
pub fn read(text: &str) -> Result<Vec<Feature>> {
    let json: Json = serde_json::from_str(text)
        .map_err(|err| ChartError::malformed("GeoJSON", err.to_string()))?;
    check_crs(&json)?;
    let mut out = Vec::new();
    push(&json, &mut out, 0)?;
    if out.is_empty() {
        return Err(ChartError::malformed("GeoJSON", "holds no geometry"));
    }
    Ok(out)
}

/// RFC 7946 fixed the coordinates as WGS 84 longitude and latitude. The old
/// `crs` member is still written by some tools; anything but the default is
/// refused by name, because metres read as degrees land somewhere plausible.
fn check_crs(json: &Json) -> Result<()> {
    let Some(name) = json
        .get("crs")
        .and_then(|crs| crs.get("properties"))
        .and_then(|properties| properties.get("name"))
        .and_then(Json::as_str)
    else {
        return Ok(());
    };
    let upper = name.to_ascii_uppercase();
    let allowed = ["CRS84", "EPSG::4326", "EPSG:4326", "WGS84"];
    if allowed.iter().any(|ok| upper.contains(ok)) {
        return Ok(());
    }
    Err(ChartError::Unsupported(format!(
        "{name} is not longitude and latitude; reproject the file to WGS 84 (EPSG:4326)"
    )))
}

fn push(json: &Json, out: &mut Vec<Feature>, depth: usize) -> Result<()> {
    if depth > 8 {
        return Err(ChartError::malformed("GeoJSON", "nested too deeply"));
    }
    match json.get("type").and_then(Json::as_str) {
        Some("FeatureCollection") => {
            let features = json
                .get("features")
                .and_then(Json::as_array)
                .ok_or_else(|| ChartError::malformed("GeoJSON", "a collection with no features"))?;
            for feature in features {
                push(feature, out, depth + 1)?;
            }
        }
        Some("Feature") => {
            let attributes = properties(json.get("properties"));
            if let Some(geometry) = json.get("geometry") {
                let mut made = Vec::new();
                push_geometry(geometry, &mut made, depth + 1)?;
                for mut feature in made {
                    feature.attributes = attributes.clone();
                    out.push(feature);
                }
            }
        }
        Some(_) => push_geometry(json, out, depth)?,
        None => return Err(ChartError::malformed("GeoJSON", "an object with no type")),
    }
    Ok(())
}

fn properties(json: Option<&Json>) -> Vec<(String, Value)> {
    let Some(Json::Object(map)) = json else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(key, value)| {
            let value = match value {
                Json::String(text) => Value::Text(text.clone()),
                Json::Number(number) => number
                    .as_i64()
                    .map(Value::Int)
                    .or_else(|| number.as_f64().map(Value::Real))?,
                Json::Bool(yes) => Value::Int(i64::from(*yes)),
                _ => return None,
            };
            Some((key.clone(), value))
        })
        .collect()
}

/// One position: longitude, latitude. Any further numbers are elevation and
/// whatever else the writer added, which nothing here draws.
fn position(json: &Json) -> Option<[f64; 2]> {
    let array = json.as_array()?;
    let lon = array.first()?.as_f64()?;
    let lat = array.get(1)?.as_f64()?;
    (lon.is_finite() && lat.is_finite()).then_some([lon, lat])
}

fn line(json: &Json) -> Option<Vec<[f64; 2]>> {
    let points: Vec<[f64; 2]> = json.as_array()?.iter().filter_map(position).collect();
    (points.len() >= 2).then_some(points)
}

fn rings(json: &Json) -> Option<Vec<Vec<[f64; 2]>>> {
    let rings: Vec<Vec<[f64; 2]>> = json
        .as_array()?
        .iter()
        .filter_map(|ring| {
            let points: Vec<[f64; 2]> = ring.as_array()?.iter().filter_map(position).collect();
            (points.len() >= 3).then_some(points)
        })
        .collect();
    (!rings.is_empty()).then_some(rings)
}

fn push_geometry(json: &Json, out: &mut Vec<Feature>, depth: usize) -> Result<()> {
    if depth > 8 {
        return Err(ChartError::malformed("GeoJSON", "nested too deeply"));
    }
    let kind = json
        .get("type")
        .and_then(Json::as_str)
        .ok_or_else(|| ChartError::malformed("GeoJSON", "a geometry with no type"))?;
    if kind == "GeometryCollection" {
        for geometry in json
            .get("geometries")
            .and_then(Json::as_array)
            .unwrap_or(&Vec::new())
        {
            push_geometry(geometry, out, depth + 1)?;
        }
        return Ok(());
    }
    let coordinates = json.get("coordinates").unwrap_or(&Json::Null);
    let geometry = match kind {
        "Point" => position(coordinates)
            .map(|point| Geometry::Points(vec![[point[0], point[1], f64::NAN]])),
        "MultiPoint" => coordinates.as_array().map(|array| {
            Geometry::Points(
                array
                    .iter()
                    .filter_map(position)
                    .map(|p| [p[0], p[1], f64::NAN])
                    .collect(),
            )
        }),
        "LineString" => line(coordinates).map(|points| Geometry::Lines(vec![points])),
        "MultiLineString" => coordinates
            .as_array()
            .map(|array| Geometry::Lines(array.iter().filter_map(line).collect())),
        "Polygon" => rings(coordinates).map(|rings| Geometry::Areas(vec![rings])),
        "MultiPolygon" => coordinates
            .as_array()
            .map(|array| Geometry::Areas(array.iter().filter_map(rings).collect())),
        other => {
            return Err(ChartError::Unsupported(format!(
                "a GeoJSON {other} geometry"
            )));
        }
    };
    // A geometry whose coordinates were all unusable draws nothing, which is
    // not an error in a file of thousands.
    if let Some(geometry) = geometry.filter(|geometry| match geometry {
        Geometry::Points(points) => !points.is_empty(),
        Geometry::Lines(lines) => !lines.is_empty(),
        Geometry::Areas(areas) => !areas.is_empty(),
    }) {
        out.push(Feature {
            class: String::new(),
            attributes: Vec::new(),
            geometry,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collection_reads_its_features_with_their_properties() {
        let features = read(
            r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","properties":{"name":"Thimble Shoal","depth":12,"lit":true},
                 "geometry":{"type":"Point","coordinates":[-76.1,37.0,3.5]}},
                {"type":"Feature","properties":{},
                 "geometry":{"type":"Polygon","coordinates":[
                    [[0,0],[1,0],[1,1],[0,1],[0,0]],
                    [[0.2,0.2],[0.8,0.2],[0.8,0.8],[0.2,0.8],[0.2,0.2]]]}}]}"#,
        )
        .expect("reads");
        assert_eq!(features.len(), 2);
        let Geometry::Points(points) = &features[0].geometry else {
            panic!("not a point");
        };
        assert_eq!([points[0][0], points[0][1]], [-76.1, 37.0]);
        assert!(
            points[0][2].is_nan(),
            "a third coordinate is elevation, not a depth to draw"
        );
        assert_eq!(
            features[0].attribute("name"),
            Some(&Value::Text("Thimble Shoal".into()))
        );
        assert_eq!(features[0].attribute("depth"), Some(&Value::Int(12)));
        let Geometry::Areas(areas) = &features[1].geometry else {
            panic!("not an area");
        };
        assert_eq!(areas[0].len(), 2, "the outside and its hole");
    }

    #[test]
    fn a_bare_geometry_and_a_collection_of_them_both_read() {
        let point = read(r#"{"type":"Point","coordinates":[1,2]}"#).expect("a bare geometry");
        assert_eq!(point.len(), 1);
        let many = read(
            r#"{"type":"GeometryCollection","geometries":[
                {"type":"Point","coordinates":[1,2]},
                {"type":"LineString","coordinates":[[0,0],[1,1]]}]}"#,
        )
        .expect("a collection");
        assert_eq!(many.len(), 2);
    }

    /// Metres read as degrees land somewhere plausible, which is the worst
    /// way to be wrong: refused by name instead.
    #[test]
    fn a_file_in_another_reference_system_is_refused_by_name() {
        let err = read(
            r#"{"type":"FeatureCollection","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::3857"}},
                "features":[{"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[-8000000,4000000]}}]}"#,
        )
        .expect_err("refused");
        assert!(err.to_string().contains("EPSG::3857"), "{err}");
        assert!(err.to_string().contains("WGS 84"), "and says what to do");
        // The default is fine, written either way.
        for name in ["urn:ogc:def:crs:OGC:1.3:CRS84", "EPSG:4326"] {
            let text = format!(
                r#"{{"type":"FeatureCollection","crs":{{"type":"name","properties":{{"name":"{name}"}}}},
                   "features":[{{"type":"Feature","properties":{{}},"geometry":{{"type":"Point","coordinates":[1,2]}}}}]}}"#
            );
            assert!(read(&text).is_ok(), "{name}");
        }
    }

    #[test]
    fn a_file_with_nothing_in_it_says_so() {
        assert!(read("not json").is_err());
        assert!(read(r#"{"type":"FeatureCollection","features":[]}"#).is_err());
        assert!(read(r#"{"type":"Topology"}"#).is_err(), "not GeoJSON");
    }
}
