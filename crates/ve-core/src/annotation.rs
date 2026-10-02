//! Measurements laid over the map (spec.md 10).
//!
//! An annotation is not an object. It contributes nothing to the field, is
//! never flattened into a scene, and never reaches an exported GRIB — it is
//! something the user drew *on* the map to read a number off it. That is why
//! it lives beside the layers in [`crate::project::Annotations`] rather than
//! in one, and why nothing in `ve-render` or `ve-grib` knows this module
//! exists.
//!
//! It is still document state: a passage measured on Tuesday is there on
//! Wednesday, so it is saved, and it is edited through the same undoable
//! commands everything else is.
//!
//! # What is here and what is not
//!
//! The geodesy is in [`crate::geo`] — the paths, the distances, the bearings.
//! This module holds the *model*: what the user placed, and the geometry that
//! follows from it. The **formatting** is not here either: a distance leaves
//! this crate in metres and a bearing in degrees, and `ve-app` turns them into
//! "1 304 nm" at the IPC boundary like every other unit (spec.md 3).

use serde::{Deserialize, Serialize};

use crate::geo::{
    LonLat, geodesic_ring, great_circle_path, rhumb_bearing, rhumb_distance_m, rhumb_path,
};
use crate::id::Id;

/// The largest number of rings one annotation may draw.
///
/// A cap rather than a warning: the count is a typed number, and a ring is 180
/// geodesic destinations, so a mistyped one would spend a second building a
/// figure nobody asked for.
pub const MAX_RINGS: u32 = 50;

/// What a measurement is.
///
/// Four shapes, matching the four modes of spec.md 10. They are one enum and
/// one list rather than four lists because everything above them treats them
/// alike — placed, dragged, cleared, drawn — and the one place that does not,
/// the per-tool clear, is a filter on the variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Measurement {
    /// A chain of points, measured leg by leg with a running total.
    ///
    /// The navigator's dividers: step along a route and read off how far it
    /// is. Two points is the shortest chain that measures anything.
    Dividers {
        /// The chain, in the order it was placed. Every one is draggable.
        points: Vec<LonLat>,
    },
    /// Both ways of sailing between two points, drawn together.
    ///
    /// A *passage*, because that is what the pair is for: the great circle is
    /// the shorter way and the rhumb line is the one a hand-steered compass
    /// course actually follows, and the point of drawing both is the gap
    /// between them.
    Passage {
        /// Where the passage starts.
        from: LonLat,
        /// And ends.
        to: LonLat,
    },
    /// Geodesic circles about a centre, at a fixed interval.
    ///
    /// Range from somewhere: how far out is a front, or how far can this boat
    /// reach in a day. The circles are geodesic — a true distance at every
    /// bearing — so near a pole they are nothing like circles on the map,
    /// which is the reason to draw them rather than estimate.
    Rings {
        /// What the rings are measured from.
        centre: LonLat,
        /// Spacing between consecutive rings, in metres.
        #[serde(with = "crate::canonical::metres_field")]
        interval_m: f64,
        /// How many rings, capped at [`MAX_RINGS`].
        count: u32,
    },
    /// Where a feature was at one time step and where it was at a later one.
    ///
    /// The one measurement with time in it: a low, a front or an eddy is
    /// marked at two steps, and what is read off is how fast and which way it
    /// travelled between them. The steps are stored rather than the hours
    /// because the step is what the user stood on when they clicked; the
    /// project's step length, which never changes, turns them into time.
    Motion {
        /// Where the feature was at `from_step`.
        from: LonLat,
        /// Where it was at `to_step`.
        to: LonLat,
        /// The step of the first mark.
        from_step: u32,
        /// The step of the second, which is later.
        to_step: u32,
    },
}

/// Which tool made a measurement, for the per-tool clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementKind {
    /// [`Measurement::Dividers`].
    Dividers,
    /// [`Measurement::Passage`].
    Passage,
    /// [`Measurement::Rings`].
    Rings,
    /// [`Measurement::Motion`].
    Motion,
}

/// A measurement with an identity, so it can be edited and cleared on its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    /// Stable identity.
    pub id: Id,
    /// What was measured.
    pub measurement: Measurement,
}

/// Which of a measurement's paths a drawn line is.
///
/// The style is the reader's only way to tell the two paths of a passage
/// apart, so it travels with the geometry rather than being inferred from the
/// order the segments arrive in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    /// The shortest path over the sphere.
    GreatCircle,
    /// The constant-bearing path.
    Rhumb,
    /// One circle of a range-ring set.
    Ring,
}

/// One measured line: where it runs, how long it is, and which way it points.
///
/// In metres and degrees. The bearing is a **geometric** bearing — a course
/// steered, not a wind direction — so it is never converted to the project's
/// direction convention (spec.md 3.3). Showing the reciprocal of a course
/// because a project happens to name winds by where they come from would be a
/// serious error, and this is the sentence that prevents it.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredPath {
    /// Which path this is.
    pub kind: PathKind,
    /// The polyline to draw, dense enough to look like the curve it is.
    pub path: Vec<LonLat>,
    /// Length in metres.
    pub distance_m: f64,
    /// The bearing to label it with, if it has one. A ring does not.
    pub bearing_deg: Option<f64>,
    /// Where the label belongs: the middle of the path.
    pub label_at: LonLat,
}

/// Everything a measurement has to say.
#[derive(Debug, Clone, PartialEq)]
pub struct Measured {
    /// The lines, in draw order.
    pub paths: Vec<MeasuredPath>,
    /// The points the user placed, which are the draggable handles.
    pub handles: Vec<LonLat>,
    /// The chain's total length, for a measurement that has one.
    ///
    /// `None` rather than a sum of the paths: a passage's two lines are two
    /// answers to one question and adding them would be meaningless, and a
    /// ring set's total is its outermost radius, which is a label of its own.
    pub total_m: Option<f64>,
}

impl Measurement {
    /// Which tool this came from.
    pub fn kind(&self) -> MeasurementKind {
        match self {
            Self::Dividers { .. } => MeasurementKind::Dividers,
            Self::Passage { .. } => MeasurementKind::Passage,
            Self::Rings { .. } => MeasurementKind::Rings,
            Self::Motion { .. } => MeasurementKind::Motion,
        }
    }

    /// The points the user placed, in the order a drag addresses them.
    pub fn handles(&self) -> Vec<LonLat> {
        match self {
            Self::Dividers { points } => points.clone(),
            Self::Passage { from, to } => vec![*from, *to],
            Self::Rings { centre, .. } => vec![*centre],
            Self::Motion { from, to, .. } => vec![*from, *to],
        }
    }

    /// Moves one placed point, leaving everything else alone.
    ///
    /// Out-of-range indices are ignored rather than refused: the index comes
    /// from a pointer drag against a list the frontend was told about, and a
    /// measurement cleared underneath one is a dropped drag, not an error the
    /// user can act on.
    pub fn move_handle(&mut self, index: usize, to: LonLat) {
        match self {
            Self::Dividers { points } => {
                if let Some(point) = points.get_mut(index) {
                    *point = to;
                }
            }
            Self::Passage { from, to: end } | Self::Motion { from, to: end, .. } => match index {
                0 => *from = to,
                1 => *end = to,
                _ => {}
            },
            Self::Rings { centre, .. } => {
                if index == 0 {
                    *centre = to;
                }
            }
        }
    }

    /// Whether this measurement says anything yet.
    ///
    /// A chain of one point and a passage being placed have a handle but no
    /// length, so they are drawn while the gesture is live and never stored.
    pub fn is_measurable(&self) -> bool {
        match self {
            Self::Dividers { points } => points.len() >= 2,
            Self::Passage { .. } => true,
            Self::Rings {
                count, interval_m, ..
            } => *count > 0 && *interval_m > 0.0,
            // No time between the marks is no speed: a division by zero, and
            // a feature that has not been given the chance to move.
            Self::Motion {
                from_step, to_step, ..
            } => to_step > from_step,
        }
    }

    /// How many time steps a measurement spans, for the one kind that spans
    /// any.
    pub fn elapsed_steps(&self) -> Option<u32> {
        match self {
            Self::Motion {
                from_step, to_step, ..
            } => to_step.checked_sub(*from_step).filter(|steps| *steps > 0),
            _ => None,
        }
    }

    /// The average speed of a [`Self::Motion`] in metres per second, given
    /// the project's hours per step.
    ///
    /// The great-circle distance over the elapsed time, so it is the speed of
    /// the straight run between the two marks: a feature that wandered on the
    /// way went faster than this, which is what "average" says.
    pub fn average_speed_mps(&self, step_hours: u32) -> Option<f64> {
        let Self::Motion { from, to, .. } = self else {
            return None;
        };
        let seconds = f64::from(self.elapsed_steps()?) * f64::from(step_hours) * 3600.0;
        (seconds > 0.0).then(|| from.distance_m(*to) / seconds)
    }

    /// The geometry and the numbers this measurement produces.
    pub fn measure(&self) -> Measured {
        match self {
            Self::Dividers { points } => dividers(points),
            Self::Passage { from, to } => passage(*from, *to),
            Self::Rings {
                centre,
                interval_m,
                count,
            } => rings(*centre, *interval_m, *count),
            // One leg of a chain, geometrically: the same great circle, the
            // same initial bearing. What makes it a motion is the time, and
            // that is not geometry.
            Self::Motion { from, to, .. } => dividers(&[*from, *to]),
        }
    }
}

/// A chain measured leg by leg, each leg a great circle.
///
/// Great circles and not rhumb lines, because the chain is a measuring
/// instrument: the question a pair of dividers answers is how far apart two
/// places are, and that is the great circle. A passage is where the other
/// answer belongs.
fn dividers(points: &[LonLat]) -> Measured {
    let mut paths = Vec::new();
    let mut total_m = 0.0;
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let path = great_circle_path(from, to);
        let distance_m = from.distance_m(to);
        total_m += distance_m;
        paths.push(MeasuredPath {
            kind: PathKind::GreatCircle,
            label_at: midpoint(&path),
            path,
            distance_m,
            bearing_deg: Some(from.initial_bearing(to).degrees()),
        });
    }
    Measured {
        paths,
        handles: points.to_vec(),
        total_m: (points.len() >= 2).then_some(total_m),
    }
}

/// The two paths between a pair of points, both drawn and both measured.
fn passage(from: LonLat, to: LonLat) -> Measured {
    let great = great_circle_path(from, to);
    let rhumb = rhumb_path(from, to);
    Measured {
        paths: vec![
            MeasuredPath {
                kind: PathKind::GreatCircle,
                label_at: midpoint(&great),
                path: great,
                distance_m: from.distance_m(to),
                // The *initial* bearing, and it is labelled as such by the
                // caller: a great circle's bearing changes along the way, which
                // is exactly what distinguishes it from the line beneath it.
                bearing_deg: Some(from.initial_bearing(to).degrees()),
            },
            MeasuredPath {
                kind: PathKind::Rhumb,
                label_at: midpoint(&rhumb),
                path: rhumb,
                distance_m: rhumb_distance_m(from, to),
                bearing_deg: Some(rhumb_bearing(from, to).degrees()),
            },
        ],
        handles: vec![from, to],
        total_m: None,
    }
}

/// Concentric geodesic circles, the innermost first.
fn rings(centre: LonLat, interval_m: f64, count: u32) -> Measured {
    let count = count.min(MAX_RINGS);
    let paths = (1..=count)
        .map(|i| {
            let radius_m = interval_m * f64::from(i);
            let path = geodesic_ring(centre, radius_m);
            MeasuredPath {
                kind: PathKind::Ring,
                // Due north of the centre, which is where the first vertex is:
                // a predictable place, so a stack of rings labels in a column
                // rather than scattering.
                label_at: path[0],
                path,
                distance_m: radius_m,
                bearing_deg: None,
            }
        })
        .collect();
    Measured {
        paths,
        handles: vec![centre],
        total_m: (count > 0).then(|| interval_m * f64::from(count)),
    }
}

/// The middle vertex of a path, which is where its label goes.
///
/// The middle of the *list*, and the paths are built with their vertices an
/// equal distance apart, so this is the middle of the line rather than the
/// middle of its bounding box — which near a pole are far apart.
fn midpoint(path: &[LonLat]) -> LonLat {
    path.get(path.len() / 2)
        .or_else(|| path.first())
        .copied()
        .unwrap_or(LonLat { lon: 0.0, lat: 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ll(lon: f64, lat: f64) -> LonLat {
        LonLat::new(lon, lat).expect("test coordinate")
    }

    #[test]
    fn a_chain_totals_its_legs() {
        // Three points along the equator: the total is the sum, and each leg
        // is a quarter of the way round measured independently.
        let chain = Measurement::Dividers {
            points: vec![ll(0.0, 0.0), ll(60.0, 0.0), ll(150.0, 0.0)],
        };
        let measured = chain.measure();
        assert_eq!(measured.paths.len(), 2);
        let legs: f64 = measured.paths.iter().map(|p| p.distance_m).sum();
        let total = measured.total_m.expect("a chain has a total");
        assert!((total - legs).abs() < 1e-6, "{total} != {legs}");

        let quarter = crate::geo::EARTH_RADIUS_M * std::f64::consts::PI / 180.0;
        assert!((measured.paths[0].distance_m - quarter * 60.0).abs() < 1e-3);
        assert!((measured.paths[1].distance_m - quarter * 90.0).abs() < 1e-3);
        // Due east along the equator, both legs.
        for path in &measured.paths {
            assert!((path.bearing_deg.expect("a leg has a bearing") - 90.0).abs() < 1e-9);
        }
    }

    #[test]
    fn a_single_point_measures_nothing() {
        let chain = Measurement::Dividers {
            points: vec![ll(10.0, 10.0)],
        };
        assert!(!chain.is_measurable());
        let measured = chain.measure();
        assert!(measured.paths.is_empty());
        assert_eq!(measured.total_m, None);
        // But it still has its handle, so the point the user placed is drawn.
        assert_eq!(measured.handles.len(), 1);
    }

    /// The acceptance case: on a long high-latitude passage the two paths
    /// visibly part company, and the great circle is the shorter one.
    #[test]
    fn a_passage_shows_the_two_paths_diverging() {
        let passage = Measurement::Passage {
            from: ll(-73.78, 40.64),
            to: ll(4.9, 52.3),
        };
        let measured = passage.measure();
        let great = &measured.paths[0];
        let rhumb = &measured.paths[1];
        assert_eq!(great.kind, PathKind::GreatCircle);
        assert_eq!(rhumb.kind, PathKind::Rhumb);
        assert!(
            great.distance_m < rhumb.distance_m,
            "the great circle is the shorter path"
        );

        // They part by more than a hundred kilometres in the middle, which is
        // the whole reason a navigator draws both.
        let apart = great.label_at.distance_m(rhumb.label_at);
        assert!(
            apart > 100_000.0,
            "the paths are {apart} m apart in the middle"
        );

        // And the great circle bulges *poleward* of the rhumb in the north.
        assert!(great.label_at.lat > rhumb.label_at.lat);
    }

    #[test]
    fn rings_are_concentric_and_capped() {
        let set = Measurement::Rings {
            centre: ll(-30.0, 45.0),
            interval_m: 250_000.0,
            count: 4,
        };
        let measured = set.measure();
        assert_eq!(measured.paths.len(), 4);
        for (i, path) in measured.paths.iter().enumerate() {
            let expected = 250_000.0 * (i as f64 + 1.0);
            assert!((path.distance_m - expected).abs() < 1e-9);
            assert_eq!(path.bearing_deg, None, "a ring has no bearing");
            // Every vertex is at that radius, measured back independently.
            for vertex in &path.path {
                let r = ll(-30.0, 45.0).distance_m(*vertex);
                assert!((r - expected).abs() < 1e-3, "{r} != {expected}");
            }
        }
        assert_eq!(measured.total_m, Some(1_000_000.0));

        let absurd = Measurement::Rings {
            centre: ll(0.0, 0.0),
            interval_m: 1000.0,
            count: 10_000,
        };
        assert_eq!(absurd.measure().paths.len(), MAX_RINGS as usize);
    }

    /// Ten degrees of the equator in a day, worked by hand: a degree of arc
    /// on this sphere is 6 371 229 m x pi / 180 = 111 198.9 m, so the run is
    /// 1 111 989 m, and over 86 400 s that is 12.870 m/s — 25 kt, since a
    /// degree is sixty nautical miles and six hundred of them took a day.
    #[test]
    fn a_motion_is_distance_over_elapsed_time() {
        let motion = Measurement::Motion {
            from: ll(0.0, 0.0),
            to: ll(10.0, 0.0),
            from_step: 2,
            to_step: 10,
        };
        assert!(motion.is_measurable());
        assert_eq!(motion.elapsed_steps(), Some(8));
        let speed = motion.average_speed_mps(3).expect("a speed");
        assert!((speed - 12.870).abs() < 1e-3, "{speed}");

        let measured = motion.measure();
        assert_eq!(measured.paths.len(), 1);
        assert!((measured.paths[0].distance_m - 1_111_989.0).abs() < 1.0);
        let course = measured.paths[0].bearing_deg.expect("a course");
        assert!((course - 90.0).abs() < 1e-9, "{course}");
        assert_eq!(measured.handles, vec![ll(0.0, 0.0), ll(10.0, 0.0)]);
    }

    /// A feature that crosses the dateline went the short way, eastward, and
    /// not three hundred and fifty-eight degrees back round the world.
    #[test]
    fn a_motion_across_the_antimeridian_goes_the_short_way() {
        let motion = Measurement::Motion {
            from: ll(179.0, 0.0),
            to: ll(-179.0, 0.0),
            from_step: 0,
            to_step: 1,
        };
        let measured = motion.measure();
        let degree = crate::geo::EARTH_RADIUS_M * std::f64::consts::PI / 180.0;
        assert!((measured.paths[0].distance_m - 2.0 * degree).abs() < 1e-3);
        let course = measured.paths[0].bearing_deg.expect("a course");
        assert!((course - 90.0).abs() < 1e-6, "{course}");
        // Two degrees in an hour.
        let speed = motion.average_speed_mps(1).expect("a speed");
        assert!((speed - 2.0 * degree / 3600.0).abs() < 1e-6, "{speed}");
    }

    /// Over the pole: from 89 N on the Greenwich meridian to 89 N on the
    /// dateline is two degrees of arc straight across, setting out due north,
    /// and nothing like the half-circle of the parallel between them.
    #[test]
    fn a_motion_over_the_pole_crosses_it() {
        let motion = Measurement::Motion {
            from: ll(0.0, 89.0),
            to: ll(-180.0, 89.0),
            from_step: 4,
            to_step: 6,
        };
        let measured = motion.measure();
        let degree = crate::geo::EARTH_RADIUS_M * std::f64::consts::PI / 180.0;
        assert!((measured.paths[0].distance_m - 2.0 * degree).abs() < 1e-3);
        let course = measured.paths[0].bearing_deg.expect("a course");
        assert!(course.min(360.0 - course) < 1e-6, "{course}");
        let speed = motion.average_speed_mps(6).expect("a speed");
        assert!(
            (speed - 2.0 * degree / (12.0 * 3600.0)).abs() < 1e-6,
            "{speed}"
        );
    }

    #[test]
    fn a_motion_with_no_time_in_it_has_no_speed() {
        for (from_step, to_step) in [(5, 5), (5, 3)] {
            let motion = Measurement::Motion {
                from: ll(0.0, 0.0),
                to: ll(1.0, 0.0),
                from_step,
                to_step,
            };
            assert!(!motion.is_measurable());
            assert_eq!(motion.elapsed_steps(), None);
            assert_eq!(motion.average_speed_mps(3), None);
        }
        // And nothing but a motion has a speed at all.
        let chain = Measurement::Dividers {
            points: vec![ll(0.0, 0.0), ll(1.0, 0.0)],
        };
        assert_eq!(chain.average_speed_mps(3), None);
    }

    #[test]
    fn moving_a_handle_moves_that_point_and_no_other() {
        let mut chain = Measurement::Dividers {
            points: vec![ll(0.0, 0.0), ll(10.0, 0.0), ll(20.0, 0.0)],
        };
        chain.move_handle(1, ll(10.0, 30.0));
        assert_eq!(
            chain.handles(),
            vec![ll(0.0, 0.0), ll(10.0, 30.0), ll(20.0, 0.0)]
        );
        // An index the measurement does not have is a dropped drag, not a panic.
        chain.move_handle(9, ll(0.0, 80.0));
        assert_eq!(chain.handles()[0], ll(0.0, 0.0));

        let mut rings = Measurement::Rings {
            centre: ll(5.0, 5.0),
            interval_m: 1000.0,
            count: 2,
        };
        rings.move_handle(0, ll(6.0, 6.0));
        assert_eq!(rings.handles(), vec![ll(6.0, 6.0)]);

        // Dragging a mark moves where the feature was, never when.
        let mut motion = Measurement::Motion {
            from: ll(0.0, 0.0),
            to: ll(1.0, 0.0),
            from_step: 1,
            to_step: 4,
        };
        motion.move_handle(1, ll(2.0, 2.0));
        assert_eq!(motion.handles(), vec![ll(0.0, 0.0), ll(2.0, 2.0)]);
        assert_eq!(motion.elapsed_steps(), Some(3));
    }
}
