//! GPX: waypoints, routes and tracks (spec.md 4.11).
//!
//! The format a chartplotter, a handheld and every routing tool exports, so
//! it is the one a passage most often arrives in. Three kinds of thing, and
//! this reads all three:
//!
//! - `<wpt>` — a mark. One [`Geometry::Points`] feature each, because a
//!   waypoint's name is the point of it and one feature holding every
//!   waypoint could carry only one name.
//! - `<rte>` — a route, the legs someone intends to sail. One feature, its
//!   `<rtept>`s in order.
//! - `<trk>` — a track, where someone actually went. One feature holding a
//!   line per `<trkseg>`: a track that lost its fix has a gap, and joining
//!   across it would draw a leg that was never sailed.
//!
//! GPX states positions as WGS 84 decimal degrees in the schema itself, so
//! there is no reference system to read and none to check.
//!
//! `<ele>` is kept as an attribute and never in the third number of a point,
//! which this crate reserves for a *sounding* — a depth, measured downwards.
//! An elevation written there would read as its own negation.

use quick_xml::Reader;
use quick_xml::events::BytesStart;

use crate::error::{ChartError, Result};
use crate::geometry::{Feature, Geometry, Point, Value};

/// How deep a GPX document may nest before it is refused. Real ones reach
/// four; more than this is a file built to exhaust a reader.
const MAX_DEPTH: usize = 64;

/// The child elements kept as attributes, wherever they appear.
const KEPT: &[&str] = &["name", "desc", "cmt", "type", "time", "ele", "sym"];

/// A feature's attributes, in the order the file listed them.
type Attributes = Vec<(String, Value)>;

/// A run of positions: a route, or one segment of a track.
type Line = Vec<Point>;

/// Reads a `.gpx` document.
pub fn read(text: &str) -> Result<Vec<Feature>> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    reader.config_mut().check_end_names = false;

    let mut state = Gpx::default();
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(start)) => {
                state.depth += 1;
                if state.depth > MAX_DEPTH {
                    return Err(ChartError::malformed("GPX", "nested too deeply"));
                }
                state.open(&start)?;
            }
            // A self-closing element — `<trkpt lat=".." lon=".."/>` is the
            // common shape of a track — opens and closes at once, and never
            // changes the depth.
            Ok(quick_xml::events::Event::Empty(start)) => {
                let tag = local_name(start.name().as_ref());
                state.open(&start)?;
                state.close(&tag);
            }
            Ok(quick_xml::events::Event::Text(chunk)) => {
                if state.kept.is_some()
                    && let Ok(text) = chunk.decode()
                {
                    state.text_of.push_str(&text);
                }
            }
            Ok(quick_xml::events::Event::CData(chunk)) => {
                if state.kept.is_some() {
                    state.text_of.push_str(&String::from_utf8_lossy(&chunk));
                }
            }
            Ok(quick_xml::events::Event::End(end)) => {
                state.depth = state.depth.saturating_sub(1);
                let tag = local_name(end.name().as_ref());
                state.close(&tag);
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(error) => return Err(ChartError::malformed("GPX", error.to_string())),
            _ => {}
        }
    }

    if !state.saw_gpx {
        return Err(ChartError::malformed("GPX", "has no <gpx> element"));
    }
    Ok(state.features)
}

/// What has been read so far.
///
/// A waypoint, a route and a track never nest inside one another, so one
/// slot of each is enough to hold the one being built.
#[derive(Default)]
struct Gpx {
    features: Vec<Feature>,
    depth: usize,
    saw_gpx: bool,
    waypoint: Option<(Point, Attributes)>,
    route: Option<(Line, Attributes)>,
    track: Option<(Vec<Line>, Attributes)>,
    segment: Option<Line>,
    /// The element whose text is being gathered, and that text.
    kept: Option<String>,
    text_of: String,
    /// Whether text belongs to the point being read rather than to the route
    /// or track around it: `<trkpt><ele>` is the point's, `<trk><name>` is
    /// the track's.
    in_point: bool,
}

impl Gpx {
    fn open(&mut self, start: &BytesStart<'_>) -> Result<()> {
        let tag = local_name(start.name().as_ref());
        match tag.as_str() {
            "gpx" => self.saw_gpx = true,
            "wpt" => {
                self.waypoint = position_of(start)?.map(|point| (point, Vec::new()));
                self.in_point = true;
            }
            "rte" => {
                self.route = Some((Vec::new(), Vec::new()));
                self.in_point = false;
            }
            "trk" => {
                self.track = Some((Vec::new(), Vec::new()));
                self.in_point = false;
            }
            "trkseg" => self.segment = Some(Vec::new()),
            "rtept" | "trkpt" => {
                if let Some(point) = position_of(start)? {
                    if tag == "rtept" {
                        if let Some((points, _)) = self.route.as_mut() {
                            points.push(point);
                        }
                    } else if let Some(points) = self.segment.as_mut() {
                        points.push(point);
                    }
                }
                self.in_point = true;
            }
            other if KEPT.contains(&other) => {
                self.kept = Some(other.to_owned());
                self.text_of.clear();
            }
            _ => {}
        }
        Ok(())
    }

    fn close(&mut self, tag: &str) {
        if self.kept.as_deref() == Some(tag) {
            let text = self.text_of.trim().to_owned();
            // A point's own `<name>` stays with the point; a route's or a
            // track's belongs to the whole feature.
            let into = if self.in_point {
                self.waypoint.as_mut().map(|(_, a)| a)
            } else {
                self.route
                    .as_mut()
                    .map(|(_, a)| a)
                    .or_else(|| self.track.as_mut().map(|(_, a)| a))
            };
            if let Some(attributes) = into
                && !text.is_empty()
            {
                attributes.push((tag.to_owned(), Value::Text(text)));
            }
            self.kept = None;
            self.text_of.clear();
        }
        match tag {
            "wpt" => {
                if let Some((point, attributes)) = self.waypoint.take() {
                    self.features.push(Feature {
                        class: String::new(),
                        attributes,
                        geometry: Geometry::Points(vec![[point[0], point[1], f64::NAN]]),
                    });
                }
                self.in_point = false;
            }
            "rtept" | "trkpt" => self.in_point = false,
            "trkseg" => {
                if let Some(points) = self.segment.take() {
                    // One point is a fix, not a leg: it strokes nothing, so
                    // it is dropped rather than kept as a line of length one.
                    if points.len() >= 2
                        && let Some((lines, _)) = self.track.as_mut()
                    {
                        lines.push(points);
                    }
                }
            }
            "rte" => {
                if let Some((points, attributes)) = self.route.take()
                    && points.len() >= 2
                {
                    self.features.push(Feature {
                        class: String::new(),
                        attributes,
                        geometry: Geometry::Lines(vec![points]),
                    });
                }
            }
            "trk" => {
                if let Some((lines, attributes)) = self.track.take()
                    && !lines.is_empty()
                {
                    self.features.push(Feature {
                        class: String::new(),
                        attributes,
                        geometry: Geometry::Lines(lines),
                    });
                }
            }
            _ => {}
        }
    }
}

/// The `lat` and `lon` of a `<wpt>`, `<rtept>` or `<trkpt>`, as
/// longitude-then-latitude.
///
/// A point missing either, or holding something that is not a number or not a
/// position, is skipped rather than failing the file: one bad fix in a track
/// of ten thousand should not cost the passage.
fn position_of(start: &BytesStart<'_>) -> Result<Option<Point>> {
    let mut lat = None;
    let mut lon = None;
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|error| ChartError::malformed("GPX", error.to_string()))?;
        let key = local_name(attribute.key.as_ref());
        let value = String::from_utf8_lossy(&attribute.value).trim().to_owned();
        match key.as_str() {
            "lat" => lat = value.parse::<f64>().ok(),
            "lon" => lon = value.parse::<f64>().ok(),
            _ => {}
        }
    }
    Ok(match (lon, lat) {
        // The schema's own range. Anything outside it is not a position.
        (Some(lon), Some(lat))
            if (-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat) =>
        {
            Some([lon, lat])
        }
        _ => None,
    })
}

/// An element name without its namespace prefix.
fn local_name(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    text.rsplit(':').next().unwrap_or(&text).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A passage in the shape a chartplotter writes one: a named mark, a
    /// planned route, and a recorded track whose fix dropped out in the
    /// middle. The positions are the ones written here, so what is asserted
    /// is the reading and not a second copy of the parser.
    const PASSAGE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="test">
  <wpt lat="41.3500" lon="-70.5000">
    <name>Nobska</name>
    <ele>12.5</ele>
  </wpt>
  <rte>
    <name>To Nantucket</name>
    <rtept lat="41.4000" lon="-70.6000"/>
    <rtept lat="41.3000" lon="-70.2000"/>
    <rtept lat="41.2800" lon="-70.1000"/>
  </rte>
  <trk>
    <name>Sailed</name>
    <trkseg>
      <trkpt lat="41.4000" lon="-70.6000"><ele>0</ele></trkpt>
      <trkpt lat="41.3600" lon="-70.4500"></trkpt>
    </trkseg>
    <trkseg>
      <trkpt lat="41.3000" lon="-70.2000"/>
      <trkpt lat="41.2800" lon="-70.1000"/>
    </trkseg>
  </trk>
</gpx>"#;

    fn named(feature: &Feature) -> Option<&str> {
        match feature.attribute("name") {
            Some(Value::Text(text)) => Some(text.as_str()),
            _ => None,
        }
    }

    #[test]
    fn a_waypoint_is_a_point_where_the_file_says_with_its_name() {
        let features = read(PASSAGE).expect("reads");
        let waypoint = &features[0];
        assert_eq!(named(waypoint), Some("Nobska"));
        let Geometry::Points(points) = &waypoint.geometry else {
            panic!("a waypoint is a point, got {:?}", waypoint.geometry);
        };
        assert_eq!(points.len(), 1);
        // Longitude first, as everything in this crate is.
        assert!((points[0][0] - -70.5).abs() < 1e-9, "{:?}", points[0]);
        assert!((points[0][1] - 41.35).abs() < 1e-9, "{:?}", points[0]);
    }

    /// The third number of a point is a sounding — a depth, downwards — so an
    /// elevation must not be written into it. 12.5 m above the water would
    /// otherwise be read as 12.5 m beneath it.
    #[test]
    fn an_elevation_is_an_attribute_and_never_a_sounding() {
        let features = read(PASSAGE).expect("reads");
        let Geometry::Points(points) = &features[0].geometry else {
            panic!("a waypoint is a point");
        };
        assert!(points[0][2].is_nan(), "got a sounding of {}", points[0][2]);
        assert_eq!(
            features[0].attribute("ele"),
            Some(&Value::Text("12.5".to_owned()))
        );
    }

    #[test]
    fn a_route_is_one_line_through_its_points_in_order() {
        let features = read(PASSAGE).expect("reads");
        let route = features.iter().find(|f| named(f) == Some("To Nantucket"));
        let route = route.expect("the route");
        let Geometry::Lines(lines) = &route.geometry else {
            panic!("a route is a line, got {:?}", route.geometry);
        };
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 3);
        assert!((lines[0][0][0] - -70.6).abs() < 1e-9);
        assert!((lines[0][2][0] - -70.1).abs() < 1e-9);
    }

    /// A track that lost its fix has a gap, and a leg drawn across it is one
    /// that was never sailed. Two segments stay two lines in one feature.
    #[test]
    fn a_tracks_segments_are_not_joined_across_the_gap() {
        let features = read(PASSAGE).expect("reads");
        let track = features
            .iter()
            .find(|f| named(f) == Some("Sailed"))
            .expect("the track");
        let Geometry::Lines(lines) = &track.geometry else {
            panic!("a track is lines, got {:?}", track.geometry);
        };
        assert_eq!(lines.len(), 2, "one line per segment");
        assert_eq!(lines[0].len(), 2);
        assert_eq!(lines[1].len(), 2);
        // The end of the first segment is not the start of the second.
        assert!((lines[0][1][0] - -70.45).abs() < 1e-9);
        assert!((lines[1][0][0] - -70.2).abs() < 1e-9);
    }

    /// A handheld writes `<gpx:trk>`; the prefix is not part of the name.
    ///
    /// The namespace is not declared here on purpose. What is under test is
    /// that the prefix is stripped, and the declaration would put the schema's
    /// URL in this crate's source, which the offline check reads as a fetch
    /// (invariant 5). Nothing here resolves a namespace, so nothing is lost.
    #[test]
    fn a_namespaced_document_reads_the_same() {
        let text = r#"<gpx:gpx>
          <gpx:trk><gpx:trkseg>
            <gpx:trkpt lat="1.0" lon="2.0"/>
            <gpx:trkpt lat="3.0" lon="4.0"/>
          </gpx:trkseg></gpx:trk>
        </gpx:gpx>"#;
        let features = read(text).expect("reads");
        assert_eq!(features.len(), 1);
        let Geometry::Lines(lines) = &features[0].geometry else {
            panic!("a track is lines");
        };
        assert_eq!(lines[0], vec![[2.0, 1.0], [4.0, 3.0]]);
    }

    /// One unreadable fix in a long track should not cost the passage.
    #[test]
    fn a_point_with_no_position_is_skipped_rather_than_fatal() {
        let text = r#"<gpx><trk><trkseg>
            <trkpt lat="1.0" lon="2.0"/>
            <trkpt lat="not a number" lon="4.0"/>
            <trkpt lat="200.0" lon="4.0"/>
            <trkpt lon="6.0"/>
            <trkpt lat="7.0" lon="8.0"/>
          </trkseg></trk></gpx>"#;
        let features = read(text).expect("reads");
        let Geometry::Lines(lines) = &features[0].geometry else {
            panic!("a track is lines");
        };
        assert_eq!(lines[0], vec![[2.0, 1.0], [8.0, 7.0]]);
    }

    /// A segment of one fix strokes nothing, and a route of one point is not
    /// a route. Neither becomes a line that cannot be drawn.
    #[test]
    fn a_single_position_is_not_a_line() {
        let text = r#"<gpx>
            <trk><trkseg><trkpt lat="1.0" lon="2.0"/></trkseg></trk>
            <rte><rtept lat="1.0" lon="2.0"/></rte>
          </gpx>"#;
        assert!(read(text).expect("reads").is_empty());
    }

    #[test]
    fn something_that_is_not_gpx_is_refused() {
        let text = r#"<kml><Placemark><name>no</name></Placemark></kml>"#;
        assert!(read(text).is_err());
    }
}
