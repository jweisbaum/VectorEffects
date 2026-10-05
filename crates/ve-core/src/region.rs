//! The part of the earth a project covers (spec.md 4.1, 4.2).
//!
//! A region is a box on the same lattice the global project uses: its nodes
//! are a sub-grid of the global one, so an import cropped to it and an export
//! written on it agree with a global project node for node.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::geo::normalize_lon;
use crate::project::Resolution;
use crate::regrid::TargetGrid;

const MICRO: f64 = 1_000_000.0;
const FULL_TURN: i32 = 360_000_000;
const POLE: i32 = 90_000_000;

/// A box of the earth, in integer micro-degrees.
///
/// Integers (decision R2) because the box is a project setting that names
/// fetch files, so it must compare, hash and round-trip exactly; a float edge
/// would drift in its last bit through a save and load. The eastern edge is
/// stored as a **span** from the west, not as a longitude: a box over the
/// antimeridian then needs no special case, since east is simply `west + span`
/// and may pass 180, where "east < west" would be ambiguous with a full turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    /// Western edge, longitude in `[-180e6, 180e6)`.
    pub west_udeg: i32,
    /// Eastward extent from the west edge, in `(0, 360e6]`.
    pub span_udeg: i32,
    /// Northern edge, latitude.
    pub north_udeg: i32,
    /// Southern edge, latitude.
    pub south_udeg: i32,
}

fn invalid(reason: impl Into<String>) -> CoreError {
    CoreError::InvalidRegion {
        reason: reason.into(),
    }
}

/// Degrees to micro-degrees, rounded, so `10.1` is `10_100_000` and not one
/// short through the float.
fn udeg(deg: f64) -> i64 {
    (deg * MICRO).round() as i64
}

impl Region {
    /// A region from a drawn box, snapped **outward** to `res`'s lattice so the
    /// box the person drew is always inside what is fetched.
    ///
    /// `east` is read as the eastward arc from `west`, so `160, -160` is forty
    /// degrees across the antimeridian. `full_circle` ignores `east` and
    /// covers every longitude (a polar cap).
    pub fn snapped(
        west: f64,
        east: f64,
        south: f64,
        north: f64,
        full_circle: bool,
        res: Resolution,
    ) -> Result<Region> {
        if ![west, east, south, north].iter().all(|v| v.is_finite()) {
            return Err(invalid("a coordinate was not finite"));
        }
        if south < -90.0 || north > 90.0 {
            return Err(invalid("a latitude is past the pole"));
        }
        let d = i64::from(res.micro_degrees());
        let floor = |v: i64| v.div_euclid(d) * d;
        let ceil = |v: i64| -((-v).div_euclid(d)) * d;

        let west_u = udeg(normalize_lon(west));
        let mut span = if full_circle {
            i64::from(FULL_TURN)
        } else {
            let s = (udeg(normalize_lon(east)) - west_u).rem_euclid(i64::from(FULL_TURN));
            if s == 0 {
                return Err(invalid("the box has no width"));
            }
            s
        };
        let mut west_s = floor(west_u);
        // Outward on the east edge too: ceil of west + span, not of the span.
        span = (ceil(west_u + span) - west_s).min(i64::from(FULL_TURN));
        // The poles are multiples of every resolution, so snapping outward
        // from a latitude within +-90 never leaves the range.
        let south_s = floor(udeg(south));
        let north_s = ceil(udeg(north));
        // One coverage, one key: any full turn is canonically west -180.
        if full_circle || span >= i64::from(FULL_TURN) {
            west_s = -i64::from(FULL_TURN) / 2;
            span = i64::from(FULL_TURN);
        }
        if south_s >= north_s {
            return Err(invalid("the south edge must be below the north edge"));
        }
        let region = Region {
            west_udeg: west_s as i32,
            span_udeg: span as i32,
            north_udeg: north_s as i32,
            south_udeg: south_s as i32,
        };
        region.validate(res)?;
        Ok(region)
    }

    /// Whether the region is well formed on `res`'s lattice.
    pub fn validate(&self, res: Resolution) -> Result<()> {
        let d = res.micro_degrees() as i32;
        if self.span_udeg <= 0 || self.span_udeg > FULL_TURN {
            return Err(invalid("the span must be above 0 and at most 360 degrees"));
        }
        if !(-FULL_TURN / 2..FULL_TURN / 2).contains(&self.west_udeg) {
            return Err(invalid("the west edge must be in [-180, 180)"));
        }
        if self.south_udeg >= self.north_udeg || self.south_udeg < -POLE || self.north_udeg > POLE {
            return Err(invalid("latitudes must satisfy -90 <= south < north <= 90"));
        }
        if [
            self.west_udeg,
            self.span_udeg,
            self.north_udeg,
            self.south_udeg,
        ]
        .iter()
        .any(|v| v % d != 0)
        {
            return Err(invalid("an edge is not on the resolution's lattice"));
        }
        if self.is_full_circle() && self.west_udeg != -FULL_TURN / 2 {
            return Err(invalid("a full circle starts at -180"));
        }
        if self.is_full_circle() && self.south_udeg == -POLE && self.north_udeg == POLE {
            return Err(invalid("the whole earth is a global project"));
        }
        Ok(())
    }

    /// Whether the region covers every longitude and so wraps like the global
    /// grid: no duplicated column at the seam.
    pub fn is_full_circle(&self) -> bool {
        self.span_udeg == FULL_TURN
    }

    /// The lattice of this region at `res`: the region's nodes, edges
    /// inclusive, north-west first. A full circle drops the repeated column.
    pub fn lattice(&self, res: Resolution) -> TargetGrid {
        let d = res.micro_degrees() as i32;
        let columns = self.span_udeg / d;
        TargetGrid {
            ni: (if self.is_full_circle() {
                columns
            } else {
                columns + 1
            }) as u32,
            nj: ((self.north_udeg - self.south_udeg) / d + 1) as u32,
            lon0: f64::from(self.west_udeg) / MICRO,
            lat0: f64::from(self.north_udeg) / MICRO,
            dlon: res.degrees(),
            dlat: res.degrees(),
        }
    }

    /// Whether a point lies in the region, edges included.
    pub fn contains(&self, lon: f64, lat: f64) -> bool {
        let (west, south, north) = (
            f64::from(self.west_udeg) / MICRO,
            f64::from(self.south_udeg) / MICRO,
            f64::from(self.north_udeg) / MICRO,
        );
        let in_lon = self.is_full_circle()
            || (lon - west).rem_euclid(360.0) <= f64::from(self.span_udeg) / MICRO + 1e-9;
        in_lon && south - 1e-9 <= lat && lat <= north + 1e-9
    }

    /// `(west, east, south, north)` in degrees, east **unwrapped**: it is
    /// `west + span`, so a box over the antimeridian has east above 180.
    pub fn bounds_deg(&self) -> (f64, f64, f64, f64) {
        let west = f64::from(self.west_udeg) / MICRO;
        (
            west,
            west + f64::from(self.span_udeg) / MICRO,
            f64::from(self.south_udeg) / MICRO,
            f64::from(self.north_udeg) / MICRO,
        )
    }

    /// A file-name-safe identity: west, span, north, south in micro-degrees.
    pub fn key(&self) -> String {
        format!(
            "r{}_{}_{}_{}",
            self.west_udeg, self.span_udeg, self.north_udeg, self.south_udeg
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Resolution;

    const Q: Resolution = Resolution::Deg025;

    #[test]
    fn a_box_snaps_outward_to_the_lattice() {
        let r = Region::snapped(10.1, 19.9, 40.1, 49.9, false, Q).unwrap();
        assert_eq!(r.west_udeg, 10_000_000);
        assert_eq!(r.span_udeg, 10_000_000);
        assert_eq!((r.south_udeg, r.north_udeg), (40_000_000, 50_000_000));
        let g = r.lattice(Q);
        assert_eq!((g.ni, g.nj), (41, 41));
        assert_eq!((g.lon0, g.lat0), (10.0, 50.0));
    }

    #[test]
    fn a_box_across_the_antimeridian_runs_east_past_180() {
        // 160°E to 160°W: east is smaller than west, and the arc is 40°.
        let r = Region::snapped(160.0, -160.0, -10.0, 10.0, false, Q).unwrap();
        assert_eq!(r.span_udeg, 40_000_000);
        let g = r.lattice(Q);
        assert_eq!(g.ni, 161);
        assert_eq!(g.lon0, 160.0);
        assert!(r.contains(179.9, 0.0));
        assert!(r.contains(-179.9, 0.0));
        assert!(r.contains(-160.0, 0.0));
        assert!(!r.contains(-159.0, 0.0));
        assert!(!r.contains(0.0, 0.0));
        assert_eq!(r.bounds_deg(), (160.0, 200.0, -10.0, 10.0));
        assert_eq!(r.key(), "r160000000_40000000_10000000_-10000000");
    }

    #[test]
    fn an_arctic_cap_wraps_and_holds_the_pole_row_once() {
        let r = Region::snapped(0.0, 0.0, 60.0, 90.0, true, Q).unwrap();
        assert!(r.is_full_circle());
        assert_eq!(r.west_udeg, -180_000_000);
        let g = r.lattice(Q);
        assert_eq!(g.ni, 1440); // no duplicated column, as the global grid
        assert_eq!(g.nj, 121);
        assert_eq!(g.lat0, 90.0);
        assert!(r.contains(123.0, 89.99));
        assert!(r.contains(-180.0, 60.0));
        assert!(!r.contains(0.0, 59.0));
    }

    #[test]
    fn an_antarctic_cap_reaches_the_south_pole() {
        let r = Region::snapped(0.0, 0.0, -90.0, -60.0, true, Q).unwrap();
        let g = r.lattice(Q);
        assert_eq!(g.nj, 121);
        assert_eq!(g.lat0 - f64::from(g.nj - 1) * g.dlat, -90.0);
    }

    #[test]
    fn a_wedge_that_touches_the_pole_is_an_ordinary_rectangle() {
        let r = Region::snapped(-30.0, 30.0, 70.0, 90.0, false, Q).unwrap();
        assert!(!r.is_full_circle());
        assert_eq!(r.lattice(Q).ni, 241);
        assert!(r.contains(0.0, 90.0));
    }

    #[test]
    fn the_whole_earth_is_global_not_a_region() {
        assert!(Region::snapped(0.0, 0.0, -90.0, 90.0, true, Q).is_err());
    }

    #[test]
    fn degenerate_and_inverted_boxes_are_refused() {
        assert!(Region::snapped(10.0, 10.0, 40.0, 50.0, false, Q).is_err()); // zero span
        assert!(Region::snapped(10.0, 20.0, 50.0, 40.0, false, Q).is_err()); // south > north
        assert!(Region::snapped(10.0, 20.0, 40.0, 95.0, false, Q).is_err()); // past the pole
    }

    #[test]
    fn a_region_off_the_lattice_does_not_validate() {
        let r = Region {
            west_udeg: 10_100_000,
            span_udeg: 10_000_000,
            north_udeg: 50_000_000,
            south_udeg: 40_000_000,
        };
        assert!(r.validate(Q).is_err());
        assert!(r.validate(Resolution::Deg01).is_ok());
    }

    #[test]
    fn malformed_regions_do_not_validate() {
        let ok = Region {
            west_udeg: 10_000_000,
            span_udeg: 10_000_000,
            north_udeg: 50_000_000,
            south_udeg: 40_000_000,
        };
        assert!(ok.validate(Q).is_ok());
        let cases = [
            ("span 0", Region { span_udeg: 0, ..ok }),
            (
                "span < 0",
                Region {
                    span_udeg: -250_000,
                    ..ok
                },
            ),
            (
                "span > 360",
                Region {
                    span_udeg: 360_250_000,
                    ..ok
                },
            ),
            (
                "west = 180",
                Region {
                    west_udeg: 180_000_000,
                    ..ok
                },
            ),
            (
                "west < -180",
                Region {
                    west_udeg: -180_250_000,
                    ..ok
                },
            ),
            (
                "south == north",
                Region {
                    south_udeg: 50_000_000,
                    ..ok
                },
            ),
            (
                "south > north",
                Region {
                    south_udeg: 60_000_000,
                    ..ok
                },
            ),
            (
                "south < -90",
                Region {
                    south_udeg: -90_250_000,
                    ..ok
                },
            ),
            (
                "north > 90",
                Region {
                    north_udeg: 90_250_000,
                    ..ok
                },
            ),
            (
                "whole earth",
                Region {
                    west_udeg: -180_000_000,
                    span_udeg: 360_000_000,
                    north_udeg: 90_000_000,
                    south_udeg: -90_000_000,
                },
            ),
            (
                "full circle not at -180",
                Region {
                    span_udeg: 360_000_000,
                    north_udeg: 90_000_000,
                    south_udeg: 60_000_000,
                    ..ok
                },
            ),
        ];
        for (name, r) in cases {
            assert!(r.validate(Q).is_err(), "{name} must not validate");
        }
    }

    #[test]
    fn a_box_that_snaps_to_a_full_turn_is_canonical() {
        let r = Region::snapped(10.0, 9.9, 60.0, 90.0, false, Q).unwrap();
        assert!(r.is_full_circle());
        assert_eq!(r.west_udeg, -180_000_000);
    }

    #[test]
    fn a_project_with_an_invalid_region_does_not_validate() {
        use crate::project::{FieldKind, Project, ProjectSettings, StepHours};
        let mut p = Project::new(
            "R",
            ProjectSettings::new(FieldKind::Wind, Q, StepHours::H3, 4),
        );
        assert!(p.validate().is_ok());
        p.settings.region = Some(Region {
            west_udeg: 10_000_000,
            span_udeg: 0,
            north_udeg: 50_000_000,
            south_udeg: 40_000_000,
        });
        assert!(p.validate().is_err());
    }
}
