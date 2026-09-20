//! KML, and the zipped KMZ it usually travels as.
//!
//! What is read is the placemarks: their names, their descriptions and their
//! geometry. Styles, overlays, network links and the rest of KML are not —
//! a network link would fetch, which nothing outside `ve-osm` and `ve-zarr`
//! may do (invariant 5).
//!
//! KML states its coordinates as longitude, latitude and an optional
//! altitude, on WGS 84, and says so in the specification; there is no
//! reference system to check.

use std::io::Read;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::{ChartError, Result};
use crate::geometry::{Feature, Geometry, Value};

/// How deep a KML document may nest before it is refused. A folder tree of
/// more than this is a file built to exhaust a reader, not a chart.
const MAX_DEPTH: usize = 64;

/// Reads a `.kml` document.
pub fn read(text: &str) -> Result<Vec<Feature>> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    reader.config_mut().check_end_names = false;

    let mut features = Vec::new();
    // Where we are: the element path, the placemark being built, and the
    // text of the element being read.
    let mut path: Vec<String> = Vec::new();
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut rings: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut inner = false;
    let mut made: Vec<Geometry> = Vec::new();
    let mut text_of = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                let tag = local_name(start.name().as_ref());
                if path.len() >= MAX_DEPTH {
                    return Err(ChartError::malformed("KML", "nested too deeply"));
                }
                match tag.as_str() {
                    "Placemark" => {
                        name = None;
                        description = None;
                        made.clear();
                        rings.clear();
                    }
                    "innerBoundaryIs" => inner = true,
                    "outerBoundaryIs" => inner = false,
                    "Polygon" => rings.clear(),
                    _ => {}
                }
                path.push(tag);
                text_of.clear();
            }
            Ok(Event::Text(chunk)) => {
                if let Ok(text) = chunk.decode() {
                    text_of.push_str(&text);
                }
            }
            Ok(Event::CData(chunk)) => {
                text_of.push_str(&String::from_utf8_lossy(&chunk));
            }
            Ok(Event::End(end)) => {
                let tag = local_name(end.name().as_ref());
                let within = |what: &str| path.iter().any(|open| open == what);
                match tag.as_str() {
                    "name" if within("Placemark") && name.is_none() => {
                        name = Some(text_of.trim().to_owned());
                    }
                    "description" if within("Placemark") && description.is_none() => {
                        description = Some(text_of.trim().to_owned());
                    }
                    "coordinates" => {
                        let points = coordinates(&text_of);
                        // Which element the coordinates were in says what
                        // they are: KML has no other marker.
                        if within("Polygon") {
                            if points.len() >= 3 {
                                if inner {
                                    rings.push(points);
                                } else {
                                    // The outer ring comes first, whatever
                                    // order the file lists the two in.
                                    rings.insert(0, points);
                                }
                            }
                        } else if within("LineString") || within("LinearRing") {
                            if points.len() >= 2 {
                                made.push(Geometry::Lines(vec![points]));
                            }
                        } else if !points.is_empty() {
                            made.push(Geometry::Points(
                                points.iter().map(|p| [p[0], p[1], f64::NAN]).collect(),
                            ));
                        }
                    }
                    "Polygon" => {
                        if !rings.is_empty() {
                            made.push(Geometry::Areas(vec![std::mem::take(&mut rings)]));
                        }
                    }
                    "Placemark" => {
                        let mut attributes = Vec::new();
                        if let Some(name) = name.take().filter(|text| !text.is_empty()) {
                            attributes.push(("name".to_owned(), Value::Text(name)));
                        }
                        if let Some(text) = description.take().filter(|text| !text.is_empty()) {
                            attributes.push(("description".to_owned(), Value::Text(text)));
                        }
                        for geometry in made.drain(..) {
                            features.push(Feature {
                                class: String::new(),
                                attributes: attributes.clone(),
                                geometry,
                            });
                        }
                    }
                    _ => {}
                }
                // Close to the matching open, so an unbalanced document does
                // not leave the path growing for ever.
                if let Some(at) = path.iter().rposition(|open| *open == tag) {
                    path.truncate(at);
                }
                text_of.clear();
            }
            Ok(Event::Eof) => break,
            Err(err) => return Err(ChartError::malformed("KML", err.to_string())),
            _ => {}
        }
    }

    if features.is_empty() {
        return Err(ChartError::malformed("KML", "holds no placemark geometry"));
    }
    Ok(features)
}

/// Reads a `.kmz`: a zip whose first `.kml` is the document.
pub fn read_zipped(bytes: &[u8]) -> Result<Vec<Feature>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|err| ChartError::malformed("KMZ", err.to_string()))?;
    // `doc.kml` by convention, but the standard only says a `.kml` is in
    // there; the first one in the archive's order is the document.
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    let wanted = names
        .iter()
        .find(|name| name.eq_ignore_ascii_case("doc.kml"))
        .or_else(|| {
            names
                .iter()
                .find(|name| name.to_ascii_lowercase().ends_with(".kml"))
        })
        .ok_or_else(|| ChartError::malformed("KMZ", "holds no .kml"))?
        .clone();
    let mut text = String::new();
    archive
        .by_name(&wanted)
        .map_err(|err| ChartError::malformed("KMZ", err.to_string()))?
        .read_to_string(&mut text)?;
    read(&text)
}

/// An element's name without its namespace prefix.
fn local_name(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    text.rsplit(':').next().unwrap_or(&text).to_owned()
}

/// KML coordinates: `lon,lat[,alt]` tuples separated by whitespace.
fn coordinates(text: &str) -> Vec<[f64; 2]> {
    text.split_whitespace()
        .filter_map(|tuple| {
            let mut parts = tuple.split(',');
            let lon: f64 = parts.next()?.trim().parse().ok()?;
            let lat: f64 = parts.next()?.trim().parse().ok()?;
            (lon.is_finite() && lat.is_finite() && (-90.0..=90.0).contains(&lat))
                .then_some([lon, lat])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2"><Document>
  <Placemark>
    <name>Thimble Shoal Light</name>
    <description><![CDATA[Fl W 10s]]></description>
    <Point><coordinates>-76.2436,36.9964,0</coordinates></Point>
  </Placemark>
  <Folder><name>Routes</name>
    <Placemark><name>Approach</name>
      <LineString><coordinates>
        -76.0,37.0,0
        -76.1,37.1,0
        -76.2,37.2,0
      </coordinates></LineString>
    </Placemark>
  </Folder>
  <Placemark><name>Anchorage</name>
    <Polygon>
      <outerBoundaryIs><LinearRing><coordinates>
        0,0 1,0 1,1 0,1 0,0
      </coordinates></LinearRing></outerBoundaryIs>
      <innerBoundaryIs><LinearRing><coordinates>
        0.2,0.2 0.8,0.2 0.8,0.8 0.2,0.2
      </coordinates></LinearRing></innerBoundaryIs>
    </Polygon>
  </Placemark>
</Document></kml>"#;

    #[test]
    fn placemarks_read_with_their_names_and_the_right_kind_of_geometry() {
        let features = read(DOCUMENT).expect("reads");
        assert_eq!(features.len(), 3);

        let Geometry::Points(points) = &features[0].geometry else {
            panic!("not a point");
        };
        assert_eq!([points[0][0], points[0][1]], [-76.2436, 36.9964]);
        assert!(points[0][2].is_nan(), "the altitude is not a depth to draw");
        assert_eq!(
            features[0].attribute("name"),
            Some(&Value::Text("Thimble Shoal Light".into()))
        );
        assert_eq!(
            features[0].attribute("description"),
            Some(&Value::Text("Fl W 10s".into())),
            "a CDATA description is read"
        );

        // A folder's own name is not the placemark's.
        assert_eq!(
            features[1].attribute("name"),
            Some(&Value::Text("Approach".into()))
        );
        let Geometry::Lines(lines) = &features[1].geometry else {
            panic!("not a line");
        };
        assert_eq!(lines[0].len(), 3);

        let Geometry::Areas(areas) = &features[2].geometry else {
            panic!("not an area");
        };
        assert_eq!(areas[0].len(), 2, "the outside and its hole");
        assert_eq!(areas[0][0][0], [0.0, 0.0], "the outside comes first");
    }

    /// A LinearRing inside a Polygon is that polygon's boundary; one on its
    /// own is a closed line. Confusing the two fills the map with a shape
    /// nobody drew.
    #[test]
    fn a_ring_outside_a_polygon_is_a_line_not_an_area() {
        let features = read(
            r#"<kml><Placemark><LinearRing><coordinates>0,0 1,0 1,1 0,0</coordinates></LinearRing></Placemark></kml>"#,
        )
        .expect("reads");
        assert!(matches!(features[0].geometry, Geometry::Lines(_)));
    }

    #[test]
    fn namespaced_elements_read_the_same() {
        let features = read(
            r#"<kml:kml xmlns:kml="http://www.opengis.net/kml/2.2"><kml:Placemark><kml:name>N</kml:name>
               <kml:Point><kml:coordinates>1,2</kml:coordinates></kml:Point></kml:Placemark></kml:kml>"#,
        )
        .expect("reads");
        assert_eq!(
            features[0].attribute("name"),
            Some(&Value::Text("N".into()))
        );
    }

    #[test]
    fn a_kmz_reads_the_document_inside_it() {
        let mut zipped = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut zipped));
            writer
                .start_file("doc.kml", zip::write::SimpleFileOptions::default())
                .expect("entry");
            std::io::Write::write_all(&mut writer, DOCUMENT.as_bytes()).expect("write");
            writer.finish().expect("finish");
        }
        let features = read_zipped(&zipped).expect("reads");
        assert_eq!(features.len(), 3);
    }

    #[test]
    fn a_document_with_no_placemarks_says_so() {
        assert!(read("<kml><Document><name>Empty</name></Document></kml>").is_err());
        assert!(read("not xml at all").is_err());
        // A coordinate outside the earth is dropped rather than drawn.
        assert!(read(r#"<kml><Placemark><Point><coordinates>1,999</coordinates></Point></Placemark></kml>"#).is_err());
    }
}
