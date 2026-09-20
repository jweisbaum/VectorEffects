//! How a chart is drawn (spec.md 4.11).
//!
//! Not S-52. The official presentation library is a symbol set, a lookup
//! grammar and a conditional-symbology language of its own; what this is, is
//! a legible nautical chart in the application's own palette — shallow water
//! paler than deep, land behind it, the coastline and the depth contours
//! over that, and a mark for every aid and hazard. It is drawn under the
//! field the user is painting, so it is deliberately quiet: nothing here is
//! as bright as a wind barb.
//!
//! The order features are drawn in is the order of [`layer_of`], not the
//! order of the cell: an area drawn after a contour would bury it.

use crate::geometry::{Feature, Geometry, Value};
use crate::paint::{Rgba, Style};

/// The colours a chart is drawn in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Water too shallow to be safe.
    pub shallow: Rgba,
    /// Water of middling depth.
    pub middle: Rgba,
    /// Water deep enough to ignore.
    pub deep: Rgba,
    /// Dry land.
    pub land: Rgba,
    /// The line between them.
    pub coast: Rgba,
    /// Depth contours.
    pub contour: Rgba,
    /// Aids to navigation.
    pub aid: Rgba,
    /// Wrecks, rocks and obstructions.
    pub hazard: Rgba,
    /// Soundings and other numbers.
    pub text: Rgba,
    /// The outline of a restricted or caution area.
    pub caution: Rgba,
    /// Piers, jetties and the like.
    pub built: Rgba,
}

impl Default for Palette {
    /// Dimmed to sit under a painted field: the map's own sea is a dark
    /// blue-grey, and these sit within a few steps of it.
    ///
    /// **`land` is the basemap's own land, to the byte** (`LAND` in
    /// `renderer.ts`, `0.20, 0.25, 0.23`). A chart covers the stretch of
    /// coast it was published for and nothing else, so a land fill of its
    /// own drew every cell's rectangle across the continent behind it. The
    /// same colour makes the seam invisible and still lets the chart's own
    /// coastline and its dredged areas sit on top.
    fn default() -> Self {
        Self {
            shallow: [58, 86, 116, 255],
            middle: [40, 62, 90, 255],
            deep: [28, 44, 68, 255],
            land: [51, 64, 59, 255],
            coast: [150, 160, 150, 255],
            contour: [92, 116, 140, 200],
            aid: [190, 170, 90, 230],
            hazard: [200, 110, 100, 235],
            text: [170, 185, 200, 220],
            caution: [150, 130, 175, 190],
            built: [96, 96, 88, 255],
        }
    }
}

/// Depths, in metres, at which the water's shade changes.
///
/// Two of them, which is what a chart needs to be read at a glance: the
/// safety contour a small craft cares about, and the one past which depth
/// stops mattering. Not settings — a chart drawn under a wind field is a
/// backdrop, and a backdrop with its own controls is a second map.
const SHALLOW_M: f64 = 5.0;
const DEEP_M: f64 = 20.0;

/// What order a feature is drawn in: lower first, so later covers earlier.
pub fn layer_of(class: &str) -> u8 {
    match class {
        "UNSARE" => 0,
        "DEPARE" | "DRGARE" | "LAKARE" => 1,
        "LNDARE" | "LNDRGN" | "BUAARE" => 2,
        "SEAARE" | "FAIRWY" | "CBLARE" => 3,
        "DEPCNT" => 4,
        "SLCONS" | "BRIDGE" | "BUISGL" | "MORFAC" | "HULKES" => 5,
        "COALNE" => 6,
        "RECTRC" | "CBLOHD" | "CBLSUB" => 7,
        "CTNARE" => 8,
        "WRECKS" | "OBSTRN" | "UWTROC" => 9,
        "SBDARE" => 10,
        "BCNCAR" | "BCNISD" | "BCNLAT" | "BCNSAW" | "BCNSPP" | "BOYCAR" | "BOYINB" | "BOYISD"
        | "BOYLAT" | "BOYSAW" | "BOYSPP" | "LIGHTS" | "LNDMRK" => 11,
        "SOUNDG" => 12,
        _ => 3,
    }
}

/// A number-valued attribute.
fn number(feature: &Feature, name: &str) -> Option<f64> {
    feature.attribute(name).and_then(Value::number)
}

/// The shade of a depth area, from the depth range it states.
fn water(palette: &Palette, feature: &Feature) -> Rgba {
    // DRVAL1 is the shallowest depth in the area, which is what a vessel has
    // to clear. An area with no stated range is drawn as the middle shade
    // rather than guessed at either way.
    match number(feature, "DRVAL1") {
        Some(depth) if depth < SHALLOW_M => palette.shallow,
        Some(depth) if depth < DEEP_M => palette.middle,
        Some(_) => palette.deep,
        None => palette.middle,
    }
}

/// How to draw one feature, or `None` for one this chart does not show.
pub fn style_for(palette: &Palette, feature: &Feature) -> Option<Style> {
    let line = |colour: Rgba, width: f32| Style {
        stroke: Some(colour),
        width,
        ..Style::default()
    };
    let area = |colour: Rgba| Style {
        fill: Some(colour),
        ..Style::default()
    };
    let mark = |colour: Rgba, radius: f32| Style {
        stroke: Some(colour),
        point_radius: radius,
        width: 1.0,
        ..Style::default()
    };

    // A restricted or caution area says so in an attribute, whatever class
    // it is: there are a dozen classes that carry RESTRN and one meaning.
    let restricted = feature.attribute("RESTRN").is_some();

    Some(match feature.class.as_str() {
        "DEPARE" | "DRGARE" => area(water(palette, feature)),
        "LAKARE" => area(palette.middle),
        "UNSARE" => area(palette.deep),
        "LNDARE" | "LNDRGN" | "BUAARE" => match feature.geometry {
            Geometry::Areas(_) => area(palette.land),
            Geometry::Lines(_) => line(palette.coast, 1.0),
            Geometry::Points(_) => return None,
        },
        "COALNE" => line(palette.coast, 1.2),
        "DEPCNT" => line(palette.contour, 0.8),
        "SLCONS" | "BRIDGE" | "MORFAC" | "HULKES" | "BUISGL" => match feature.geometry {
            Geometry::Areas(_) => area(palette.built),
            _ => line(palette.built, 1.4),
        },
        "RECTRC" | "CBLOHD" | "CBLSUB" => line(palette.caution, 0.7),
        "CTNARE" => line(palette.caution, 1.0),
        "WRECKS" | "OBSTRN" | "UWTROC" => match feature.geometry {
            Geometry::Points(_) => mark(palette.hazard, 2.6),
            _ => line(palette.hazard, 1.0),
        },
        "SOUNDG" => Style {
            // A sounding is a number on a paper chart. Here it is a dot,
            // sized by how shallow it is: legible under a field, and honest
            // about not being the number itself.
            stroke: Some(palette.text),
            point_radius: 1.4,
            width: 0.8,
            ..Style::default()
        },
        "BCNCAR" | "BCNISD" | "BCNLAT" | "BCNSAW" | "BCNSPP" | "BOYCAR" | "BOYINB" | "BOYISD"
        | "BOYLAT" | "BOYSAW" | "BOYSPP" => mark(palette.aid, 2.4),
        "LIGHTS" => mark(palette.aid, 3.0),
        "LNDMRK" => mark(palette.built, 2.0),
        "SBDARE" => return None,
        _ if restricted => line(palette.caution, 0.9),
        _ => return None,
    })
}

/// Whether a feature is drawn at all at this scale.
///
/// A cell states, per feature, the smallest scale it is worth showing at
/// (`SCAMIN`): a light's name at 1:2,000,000 is a chart nobody can read.
/// The scale here is the denominator, so a *larger* number is further out.
pub fn shown_at(feature: &Feature, scale_denominator: f64) -> bool {
    match number(feature, "SCAMIN") {
        Some(minimum) if minimum > 0.0 => scale_denominator <= minimum,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth_area(shallowest: Option<f64>) -> Feature {
        Feature {
            class: "DEPARE".into(),
            attributes: shallowest
                .map(|value| vec![("DRVAL1".to_owned(), Value::Real(value))])
                .unwrap_or_default(),
            geometry: Geometry::Areas(vec![vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]]]),
        }
    }

    #[test]
    fn water_is_shaded_by_the_depth_a_vessel_has_to_clear() {
        let palette = Palette::default();
        let fill = |depth| style_for(&palette, &depth_area(depth)).and_then(|style| style.fill);
        assert_eq!(fill(Some(0.0)), Some(palette.shallow));
        assert_eq!(fill(Some(4.9)), Some(palette.shallow));
        assert_eq!(
            fill(Some(5.0)),
            Some(palette.middle),
            "the boundary is not shallow"
        );
        assert_eq!(fill(Some(19.9)), Some(palette.middle));
        assert_eq!(fill(Some(20.0)), Some(palette.deep));
        assert_eq!(fill(None), Some(palette.middle), "unstated is not guessed");
    }

    #[test]
    fn the_things_that_matter_are_drawn_over_the_things_that_do_not() {
        for (under, over) in [
            ("DEPARE", "LNDARE"),
            ("LNDARE", "COALNE"),
            ("COALNE", "WRECKS"),
            ("DEPCNT", "COALNE"),
            ("WRECKS", "BOYLAT"),
            ("BOYLAT", "SOUNDG"),
        ] {
            assert!(
                layer_of(under) < layer_of(over),
                "{over} must be drawn over {under}"
            );
        }
    }

    /// A chart's land has to be the basemap's, or every cell's rectangle
    /// shows as a block of a slightly different colour over the continent.
    /// `LAND` in `ui/src/map/renderer.ts`, rounded to bytes.
    #[test]
    fn chart_land_is_the_basemaps_own_land() {
        let basemap = [0.20_f32, 0.25, 0.23];
        let land = Palette::default().land;
        for (channel, expected) in land[..3].iter().zip(basemap) {
            let got = f32::from(*channel) / 255.0;
            assert!(
                (got - expected).abs() <= 0.004,
                "chart land {land:?} is not the basemap's {basemap:?}"
            );
        }
        assert_eq!(land[3], 255, "and opaque, as the basemap's is");
    }

    #[test]
    fn a_class_the_chart_does_not_show_is_not_drawn() {
        let palette = Palette::default();
        let unknown = Feature {
            class: "OBJL_308".into(),
            attributes: Vec::new(),
            geometry: Geometry::Areas(vec![vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]]]),
        };
        assert!(style_for(&palette, &unknown).is_none());
        // Unless it says it is restricted, whatever class it is.
        let restricted = Feature {
            attributes: vec![("RESTRN".to_owned(), Value::Text("2,6".into()))],
            ..unknown
        };
        assert_eq!(
            style_for(&palette, &restricted).and_then(|s| s.stroke),
            Some(palette.caution)
        );
    }

    #[test]
    fn a_feature_is_hidden_past_the_scale_its_cell_gives_it() {
        let light = |minimum: i64| Feature {
            class: "LIGHTS".into(),
            attributes: vec![("SCAMIN".to_owned(), Value::Int(minimum))],
            geometry: Geometry::Points(vec![[0.0, 0.0, f64::NAN]]),
        };
        // SCAMIN is the smallest scale it is worth showing at, as a
        // denominator: shown when zoomed in past it, hidden further out.
        assert!(shown_at(&light(120_000), 50_000.0));
        assert!(shown_at(&light(120_000), 120_000.0));
        assert!(!shown_at(&light(120_000), 500_000.0));
        // No SCAMIN: always shown.
        let plain = Feature {
            attributes: Vec::new(),
            ..light(0)
        };
        assert!(shown_at(&plain, 10_000_000.0));
    }
}
