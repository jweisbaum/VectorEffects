//! GIS vector files as a display-only layer (spec.md 4.11).
//!
//! Five formats, one shape: everything is read into [`Feature`]s in
//! longitude and latitude, and drawn by the same painter a chart is. None of
//! it reaches a scene, a render-cache key or an exported file — a survey
//! traced onto the map is something to paint *against*, not a field.
//!
//! A georeferenced raster is not here: a GeoTIFF is a picture, and the
//! application already places one as an image layer (spec.md 4.9).

pub mod geojson;
pub mod gpx;
pub mod kml;
pub mod shapefile;

use std::path::Path;

use crate::error::{ChartError, Result};
use crate::geometry::{Bounds, Feature};

/// What a file holds, once read.
#[derive(Debug, Clone)]
pub struct Vectors {
    /// Everything in it.
    pub features: Vec<Feature>,
    /// The box around all of it.
    pub bounds: Bounds,
    /// How many of the features are areas.
    ///
    /// Counted here, with the bounds, rather than by whoever asks: the panel
    /// asks on every refresh to decide whether to offer a fill at all, and a
    /// survey of a hundred thousand features should not be walked for it.
    pub areas: u32,
}

/// The formats read here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// GeoJSON, by RFC 7946.
    GeoJson,
    /// An Esri shapefile: `.shp` with its `.dbf` and `.prj`.
    Shapefile,
    /// KML.
    Kml,
    /// KML, zipped.
    Kmz,
    /// GPX: waypoints, routes and tracks.
    Gpx,
}

impl Format {
    /// The format a path names, by extension. By extension and not by
    /// sniffing, because a shapefile *is* its extensions: the geometry, the
    /// attributes and the reference system are three files with one stem.
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
        Some(match extension.as_str() {
            "geojson" | "json" => Self::GeoJson,
            "shp" | "dbf" | "shx" | "prj" => Self::Shapefile,
            "kml" => Self::Kml,
            "kmz" => Self::Kmz,
            "gpx" => Self::Gpx,
            _ => return None,
        })
    }

    /// What the open dialog offers, and what the error says when a file is
    /// none of them.
    pub const EXTENSIONS: &'static [&'static str] =
        &["geojson", "json", "shp", "kml", "kmz", "gpx"];
}

/// Reads any of the supported files.
pub fn read(path: &Path) -> Result<Vectors> {
    let format = Format::of(path).ok_or_else(|| {
        ChartError::Unsupported(format!(
            "{} is not a GIS file this reads; it reads {}",
            path.display(),
            Format::EXTENSIONS.join(", ")
        ))
    })?;
    let features = match format {
        Format::GeoJson => geojson::read(&std::fs::read_to_string(path)?)?,
        Format::Shapefile => shapefile::read(path)?,
        Format::Kml => kml::read(&std::fs::read_to_string(path)?)?,
        Format::Kmz => kml::read_zipped(&std::fs::read(path)?)?,
        Format::Gpx => gpx::read(&std::fs::read_to_string(path)?)?,
    };
    let mut bounds = Bounds::EMPTY;
    let mut areas = 0u32;
    for feature in &features {
        if let Some(own) = feature.bounds() {
            bounds.merge(own);
        }
        if matches!(feature.geometry, crate::geometry::Geometry::Areas(_)) {
            areas += 1;
        }
    }
    if bounds.west > bounds.east {
        return Err(ChartError::malformed(
            "a GIS file",
            "holds no feature with a position",
        ));
    }
    // Degrees, or the file is not in the system every reader here assumes.
    // A projected shapefile is caught by its own `.prj`; this catches one
    // with no `.prj` at all, whose metres would otherwise be drawn as
    // degrees somewhere off the earth.
    if bounds.west < -400.0 || bounds.east > 400.0 || bounds.south < -95.0 || bounds.north > 95.0 {
        return Err(ChartError::Unsupported(format!(
            "the coordinates run to {:.0}, {:.0}, which are not degrees; \
             reproject the file to WGS 84 (EPSG:4326)",
            bounds.east, bounds.north
        )));
    }
    Ok(Vectors {
        features,
        bounds,
        areas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_names_its_format_by_every_extension_a_shapefile_has() {
        assert_eq!(Format::of(Path::new("a.geojson")), Some(Format::GeoJson));
        assert_eq!(Format::of(Path::new("a.KML")), Some(Format::Kml));
        assert_eq!(Format::of(Path::new("a.kmz")), Some(Format::Kmz));
        assert_eq!(Format::of(Path::new("a.gpx")), Some(Format::Gpx));
        assert_eq!(Format::of(Path::new("a.GPX")), Some(Format::Gpx));
        for part in ["a.shp", "a.dbf", "a.shx", "a.prj"] {
            assert_eq!(
                Format::of(Path::new(part)),
                Some(Format::Shapefile),
                "{part}"
            );
        }
        for offered in Format::EXTENSIONS {
            assert!(
                Format::of(Path::new(&format!("a.{offered}"))).is_some(),
                "the dialog offers .{offered} and `read` cannot dispatch it"
            );
        }
        assert_eq!(Format::of(Path::new("a.grib2")), None);
        assert_eq!(Format::of(Path::new("a")), None);
    }

    /// Metres drawn as degrees land off the earth entirely, which is the one
    /// case a reader can catch without being told the reference system.
    #[test]
    fn coordinates_that_are_not_degrees_are_refused_by_what_they_are() {
        let root = tempfile::tempdir().expect("temp");
        let path = root.path().join("utm.geojson");
        std::fs::write(
            &path,
            r#"{"type":"Point","coordinates":[412345.0,4100000.0]}"#,
        )
        .expect("write");
        let err = read(&path).expect_err("refused");
        assert!(err.to_string().contains("not degrees"), "{err}");
        assert!(err.to_string().contains("WGS 84"));
    }

    #[test]
    fn a_file_of_a_kind_this_does_not_read_says_which_kinds_it_does() {
        let err = read(Path::new("survey.dwg")).expect_err("refused");
        for extension in Format::EXTENSIONS {
            assert!(err.to_string().contains(extension), "{err}");
        }
    }
}
