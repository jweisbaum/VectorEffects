//! The one shape every source is read into.
//!
//! Longitude then latitude, in degrees, as the file gave them: a chart is
//! drawn where it says it is. Longitudes are not normalised here, because a
//! feature that crosses the antimeridian is whole only while its longitudes
//! run past 180; the painter brings each feature to the tile it is drawing.

/// A position: longitude, latitude, in degrees.
pub type Point = [f64; 2];

/// An attribute's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A whole number, or an enumerated code.
    Int(i64),
    /// A measurement.
    Real(f64),
    /// Anything else, as written.
    Text(String),
}

impl Value {
    /// The value as a number, if it is one or reads as one.
    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Int(n) => Some(*n as f64),
            Self::Real(x) => Some(*x),
            Self::Text(text) => text.trim().parse().ok(),
        }
    }
}

/// What a feature is, spatially.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// Isolated positions. The third number is a depth in metres where the
    /// source gave one (a sounding), and NaN where it did not.
    Points(Vec<[f64; 3]>),
    /// Open or closed paths.
    Lines(Vec<Vec<Point>>),
    /// Polygons, each a list of rings: the first the outside, the rest holes.
    Areas(Vec<Vec<Vec<Point>>>),
}

/// One thing on the map.
#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    /// What kind of thing: an S-57 object class acronym (`DEPARE`), or empty
    /// for a GIS feature, whose file says only what shape it is.
    pub class: String,
    /// Its attributes, in the order the source listed them.
    pub attributes: Vec<(String, Value)>,
    /// Where it is.
    pub geometry: Geometry,
}

impl Feature {
    /// An attribute by name.
    pub fn attribute(&self, name: &str) -> Option<&Value> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// The box around the feature, or `None` for one with no positions.
    pub fn bounds(&self) -> Option<Bounds> {
        let mut bounds = Bounds::EMPTY;
        match &self.geometry {
            Geometry::Points(points) => points.iter().for_each(|p| bounds.include([p[0], p[1]])),
            Geometry::Lines(lines) => lines.iter().flatten().for_each(|p| bounds.include(*p)),
            Geometry::Areas(areas) => areas
                .iter()
                .flatten()
                .flatten()
                .for_each(|p| bounds.include(*p)),
        }
        (bounds.west <= bounds.east).then_some(bounds)
    }
}

/// A box in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Western edge.
    pub west: f64,
    /// Southern edge.
    pub south: f64,
    /// Eastern edge. Past 180 for a box that crosses the antimeridian.
    pub east: f64,
    /// Northern edge.
    pub north: f64,
}

impl Bounds {
    /// A box holding nothing, which `include` grows.
    pub const EMPTY: Self = Self {
        west: f64::INFINITY,
        south: f64::INFINITY,
        east: f64::NEG_INFINITY,
        north: f64::NEG_INFINITY,
    };

    /// Grows the box to hold a position.
    pub fn include(&mut self, point: Point) {
        self.west = self.west.min(point[0]);
        self.east = self.east.max(point[0]);
        self.south = self.south.min(point[1]);
        self.north = self.north.max(point[1]);
    }

    /// Grows the box to hold another.
    pub fn merge(&mut self, other: Self) {
        self.include([other.west, other.south]);
        self.include([other.east, other.north]);
    }

    /// Whether two boxes share any ground, in any world copy: a box stated
    /// from 170 to 190 overlaps one stated from -180 to -175.
    pub fn overlaps(&self, other: &Self) -> bool {
        if self.north < other.south || other.north < self.south {
            return false;
        }
        [-360.0, 0.0, 360.0]
            .iter()
            .any(|turn| self.west + turn <= other.east && other.west <= self.east + turn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_across_the_antimeridian_overlaps_both_sides_of_it() {
        let across = Bounds {
            west: 170.0,
            south: 50.0,
            east: 190.0,
            north: 60.0,
        };
        let aleutians = Bounds {
            west: -180.0,
            south: 51.0,
            east: -175.0,
            north: 53.0,
        };
        let kamchatka = Bounds {
            west: 160.0,
            south: 50.0,
            east: 172.0,
            north: 58.0,
        };
        let hawaii = Bounds {
            west: -160.0,
            south: 18.0,
            east: -154.0,
            north: 23.0,
        };
        assert!(across.overlaps(&aleutians) && aleutians.overlaps(&across));
        assert!(across.overlaps(&kamchatka));
        assert!(!across.overlaps(&hawaii), "too far south, and too far east");
        let north_pole = Bounds {
            west: -180.0,
            south: 85.0,
            east: 180.0,
            north: 90.0,
        };
        assert!(!across.overlaps(&north_pole));
    }

    #[test]
    fn a_feature_with_no_positions_has_no_box() {
        let empty = Feature {
            class: String::new(),
            attributes: Vec::new(),
            geometry: Geometry::Lines(Vec::new()),
        };
        assert_eq!(empty.bounds(), None);
    }
}
