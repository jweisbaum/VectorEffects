//! Painting features into one tile of the application's own pyramid.
//!
//! A tile is a lat/lon box (`ve_render::tile`), so longitude and latitude go
//! linearly into its pixels and the result is an ordinary texture the map
//! draws in whatever projection it is in. Nothing here knows what a
//! projection is.
//!
//! The painter is given the features that touch the tile and a [`Style`] per
//! feature; deciding *which* features and *what* style is the caller's, and
//! differs between a chart (`s57`) and a GIS layer (`gis`).

use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Shader, Stroke,
    Transform,
};

use crate::geometry::{Bounds, Feature, Geometry};

/// A colour, as straight (not premultiplied) RGBA bytes.
pub type Rgba = [u8; 4];

/// How one feature is drawn. A feature with neither a fill nor a stroke is
/// still drawn if it has a mark: a sounding has no area and no line.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Style {
    /// Area fill, if any.
    pub fill: Option<Rgba>,
    /// Line colour, if any.
    pub stroke: Option<Rgba>,
    /// Line width in tile pixels.
    pub width: f32,
    /// A dot of this radius at every point of a point feature.
    pub point_radius: f32,
    /// Whether an unclosed line is closed before filling. An S-57 area
    /// arrives as its edges and may not close to the last decimal.
    pub close: bool,
}

/// One feature and how to draw it. Drawn in the order given, so the caller
/// decides what is on top.
#[derive(Debug, Clone, Copy)]
pub struct Painted<'a> {
    /// What to draw.
    pub feature: &'a Feature,
    /// How.
    pub style: Style,
}

/// Where a tile is and how big its picture is.
#[derive(Debug, Clone, Copy)]
pub struct TileFrame {
    /// The tile's box in degrees.
    pub bounds: Bounds,
    /// Its edge in pixels.
    pub size: u32,
}

impl TileFrame {
    /// Pixels per degree, east and south.
    fn scale(&self) -> (f64, f64) {
        (
            f64::from(self.size) / (self.bounds.east - self.bounds.west),
            f64::from(self.size) / (self.bounds.north - self.bounds.south),
        )
    }

    /// The turn of longitude that brings a feature onto this tile, in whole
    /// circles. A chart stated from 170 to 190 has to reach a tile at -175.
    fn turn_for(&self, feature: &Bounds) -> f64 {
        let middle = (self.bounds.west + self.bounds.east) / 2.0;
        let their_middle = (feature.west + feature.east) / 2.0;
        ((middle - their_middle) / 360.0).round() * 360.0
    }
}

/// A canvas the size of a tile, painted onto in order.
#[derive(Debug)]
pub struct TilePainter {
    frame: TileFrame,
    pixmap: Pixmap,
}

/// Straight bytes to a `tiny_skia` colour, which wants premultiplied.
fn colour(rgba: Rgba) -> Shader<'static> {
    let [r, g, b, a] = rgba;
    Shader::SolidColor(tiny_skia::Color::from_rgba8(r, g, b, a))
}

impl TilePainter {
    /// A transparent tile.
    pub fn new(frame: TileFrame) -> Option<Self> {
        Some(Self {
            frame,
            pixmap: Pixmap::new(frame.size, frame.size)?,
        })
    }

    /// Fills the whole tile, for a chart's own background.
    pub fn clear(&mut self, rgba: Rgba) {
        let [r, g, b, a] = rgba;
        self.pixmap.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
    }

    /// Paints one feature.
    pub fn draw(&mut self, painted: Painted<'_>) {
        let Some(bounds) = painted.feature.bounds() else {
            return;
        };
        // Taken out of `self` before anything borrows the pixmap.
        let frame = self.frame;
        let turn = frame.turn_for(&bounds);
        let (sx, sy) = frame.scale();
        let at = |p: [f64; 2]| {
            (
                ((p[0] + turn - frame.bounds.west) * sx) as f32,
                ((frame.bounds.north - p[1]) * sy) as f32,
            )
        };

        match &painted.feature.geometry {
            Geometry::Areas(areas) => {
                for rings in areas {
                    let mut builder = PathBuilder::new();
                    for ring in rings {
                        push_ring(&mut builder, ring, &at, true);
                    }
                    // Non-zero would fill a hole whose ring happens to wind
                    // the same way as its outside; even-odd needs no winding
                    // convention from the source at all.
                    if let Some(path) = builder.finish() {
                        self.fill(&path, painted.style, FillRule::EvenOdd);
                        self.stroke(&path, painted.style);
                    }
                }
            }
            Geometry::Lines(lines) => {
                let mut builder = PathBuilder::new();
                for line in lines {
                    push_ring(&mut builder, line, &at, painted.style.close);
                }
                if let Some(path) = builder.finish() {
                    self.fill(&path, painted.style, FillRule::EvenOdd);
                    self.stroke(&path, painted.style);
                }
            }
            Geometry::Points(points) => {
                if painted.style.point_radius <= 0.0 {
                    return;
                }
                let mut builder = PathBuilder::new();
                for point in points {
                    let (x, y) = at([point[0], point[1]]);
                    if x.is_finite() && y.is_finite() {
                        builder.push_circle(x, y, painted.style.point_radius);
                    }
                }
                let Some(path) = builder.finish() else {
                    return;
                };
                // A mark with no fill of its own is drawn in its line colour:
                // a buoy is a dot, not a ring.
                let filled = Style {
                    fill: painted.style.fill.or(painted.style.stroke),
                    ..painted.style
                };
                self.fill(&path, filled, FillRule::Winding);
                if painted.style.fill.is_some() {
                    self.stroke(&path, painted.style);
                }
            }
        }
    }

    /// Draws another tile's picture over this one, same size and place.
    pub fn over(&mut self, other: &TilePainter) {
        self.pixmap.draw_pixmap(
            0,
            0,
            other.pixmap.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    /// Whether anything at all was painted.
    pub fn is_blank(&self) -> bool {
        self.pixmap.data().chunks_exact(4).all(|px| px[3] == 0)
    }

    /// The picture, as straight (not premultiplied) RGBA.
    pub fn into_rgba(self) -> Vec<u8> {
        let mut out = self.pixmap.take();
        for px in out.chunks_exact_mut(4) {
            let a = u32::from(px[3]);
            if a == 0 || a == 255 {
                continue;
            }
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
        out
    }

    fn fill(&mut self, path: &Path, style: Style, rule: FillRule) {
        let Some(rgba) = style.fill else {
            return;
        };
        self.pixmap.fill_path(
            path,
            &Paint {
                shader: colour(rgba),
                anti_alias: true,
                ..Paint::default()
            },
            rule,
            Transform::identity(),
            None,
        );
    }

    fn stroke(&mut self, path: &Path, style: Style) {
        let Some(rgba) = style.stroke else {
            return;
        };
        if style.width <= 0.0 {
            return;
        }
        self.pixmap.stroke_path(
            path,
            &Paint {
                shader: colour(rgba),
                anti_alias: true,
                ..Paint::default()
            },
            &Stroke {
                width: style.width,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            },
            Transform::identity(),
            None,
        );
    }
}

/// Adds one run of positions to a path, skipping the repeats a source may
/// hold: two identical points in a row make a zero-length segment, which a
/// round cap draws as a blob.
fn push_ring(
    builder: &mut PathBuilder,
    points: &[[f64; 2]],
    at: &impl Fn([f64; 2]) -> (f32, f32),
    close: bool,
) {
    let mut started = false;
    let mut last = (f32::NAN, f32::NAN);
    for point in points {
        let (x, y) = at(*point);
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        if !started {
            builder.move_to(x, y);
            started = true;
        } else if (x - last.0).abs() > 0.01 || (y - last.1).abs() > 0.01 {
            builder.line_to(x, y);
        }
        last = (x, y);
    }
    if started && close {
        builder.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Value;

    fn frame() -> TileFrame {
        TileFrame {
            bounds: Bounds {
                west: -10.0,
                south: 40.0,
                east: 0.0,
                north: 50.0,
            },
            size: 64,
        }
    }

    fn area(points: &[[f64; 2]]) -> Feature {
        Feature {
            class: "TEST".into(),
            attributes: vec![("X".into(), Value::Int(1))],
            geometry: Geometry::Areas(vec![vec![points.to_vec()]]),
        }
    }

    fn pixel(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * size + x) * 4) as usize;
        [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
    }

    /// The west half of the tile, filled: the painted half is the colour
    /// asked for and the other half is untouched.
    #[test]
    fn a_polygon_lands_where_its_degrees_say() {
        let mut painter = TilePainter::new(frame()).expect("pixmap");
        painter.draw(Painted {
            feature: &area(&[[-10.0, 40.0], [-5.0, 40.0], [-5.0, 50.0], [-10.0, 50.0]]),
            style: Style {
                fill: Some([20, 40, 60, 255]),
                ..Style::default()
            },
        });
        let rgba = painter.into_rgba();
        assert_eq!(pixel(&rgba, 64, 8, 32), [20, 40, 60, 255], "inside");
        assert_eq!(pixel(&rgba, 64, 56, 32), [0, 0, 0, 0], "outside");
    }

    /// A chart of the Aleutians states longitudes past 180; the tile it has
    /// to reach is at -175. Whole turns of longitude are the painter's.
    #[test]
    fn a_feature_stated_past_the_antimeridian_reaches_the_tile_it_covers() {
        let east = TileFrame {
            bounds: Bounds {
                west: -180.0,
                south: 50.0,
                east: -170.0,
                north: 60.0,
            },
            size: 32,
        };
        let mut painter = TilePainter::new(east).expect("pixmap");
        painter.draw(Painted {
            feature: &area(&[[180.0, 50.0], [190.0, 50.0], [190.0, 60.0], [180.0, 60.0]]),
            style: Style {
                fill: Some([255, 255, 255, 255]),
                ..Style::default()
            },
        });
        assert!(!painter.is_blank(), "the same ground, stated the other way");
    }

    /// Half-transparent ink must come back as the colour it was, not as the
    /// premultiplied bytes the rasteriser keeps.
    #[test]
    fn the_picture_comes_back_unpremultiplied() {
        let mut painter = TilePainter::new(frame()).expect("pixmap");
        painter.draw(Painted {
            feature: &area(&[[-10.0, 40.0], [0.0, 40.0], [0.0, 50.0], [-10.0, 50.0]]),
            style: Style {
                fill: Some([200, 100, 50, 128]),
                ..Style::default()
            },
        });
        let rgba = painter.into_rgba();
        let px = pixel(&rgba, 64, 32, 32);
        assert_eq!(px[3], 128);
        for (got, want) in px[..3].iter().zip([200, 100, 50]) {
            assert!((i32::from(*got) - want).abs() <= 1, "{px:?}");
        }
    }

    #[test]
    fn a_hole_is_not_filled() {
        let ring = |inset: f64| {
            vec![
                [-10.0 + inset, 40.0 + inset],
                [0.0 - inset, 40.0 + inset],
                [0.0 - inset, 50.0 - inset],
                [-10.0 + inset, 50.0 - inset],
            ]
        };
        let doughnut = Feature {
            class: "TEST".into(),
            attributes: Vec::new(),
            geometry: Geometry::Areas(vec![vec![ring(0.0), ring(3.0)]]),
        };
        let mut painter = TilePainter::new(frame()).expect("pixmap");
        painter.draw(Painted {
            feature: &doughnut,
            style: Style {
                fill: Some([255, 255, 255, 255]),
                ..Style::default()
            },
        });
        let rgba = painter.into_rgba();
        assert_eq!(pixel(&rgba, 64, 32, 32)[3], 0, "the hole");
        assert_eq!(pixel(&rgba, 64, 4, 32)[3], 255, "the ring around it");
    }

    #[test]
    fn a_blank_tile_says_so() {
        let mut painter = TilePainter::new(frame()).expect("pixmap");
        assert!(painter.is_blank());
        painter.draw(Painted {
            feature: &area(&[[-100.0, 10.0], [-90.0, 10.0], [-90.0, 20.0]]),
            style: Style {
                fill: Some([1, 2, 3, 255]),
                ..Style::default()
            },
        });
        assert!(painter.is_blank(), "nowhere near this tile");
    }
}
