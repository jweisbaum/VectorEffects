#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The whole tool catalogue, end to end (spec.md 6.2).
//!
//! Every tool is drawn through the same command the map calls and then read
//! back out of the authoritative CPU evaluator, so these cover the path from the
//! wire type to the field rather than checking that a property got written.
//!
//! The cross-tool rules of spec 6.1 are tested *per tool* and parameterised over
//! the catalogue, not asserted once against the brush. That is the whole point
//! of writing them down: a new tool fails these without anyone remembering to
//! add a bespoke test for it.

use ve_app::commands::AppState;
use ve_app::create::{self, Gesture, NewObject, PathPoint, Tool, ToolOption};
use ve_app::document::PropertyValue;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_core::LonLat;
use ve_core::schema::{PropId, ToolKind};

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-tools-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp root");
        Self(dir)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn project(label: &str) -> (TempRoot, AppState) {
    let root = TempRoot::new(label);
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Tools".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 4,
        },
        false,
    )
    .expect("create");
    (root, state)
}

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat).expect("position")
}

/// Speed and azimuth-toward at a position, from the authoritative evaluator.
fn sample(state: &AppState, position: LonLat) -> (f64, f64) {
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let scene = ve_render::scene::flatten(&project, 0);
    let uv = ve_render::cpu::sample_scene(&scene, position);
    let (speed, azimuth) = ve_core::vector::speed_azimuth_from_uv(uv);
    (speed, azimuth.degrees())
}

/// The document as the session holds it.
fn document(state: &AppState) -> ve_core::project::Project {
    let mut session = state.session.lock().expect("lock");
    session.require_open().expect("open").project.clone()
}

fn option(id: PropId, value: PropertyValue) -> ToolOption {
    ToolOption {
        property: format!("{id:?}"),
        value,
    }
}

fn number(id: PropId, value: f64) -> ToolOption {
    option(id, PropertyValue::Number { value })
}

fn pick(id: PropId, index: u8) -> ToolOption {
    option(id, PropertyValue::Choice { index })
}

fn at(id: PropId, lon: f64, lat: f64) -> ToolOption {
    option(id, PropertyValue::Position { lon, lat })
}

fn draw(state: &AppState, tool: Tool, gesture: Gesture, options: Vec<ToolOption>) {
    create::create(
        state,
        NewObject {
            tool,
            gesture,
            options,
            layer: None,
        },
    )
    .unwrap_or_else(|err| panic!("{tool:?} refused its gesture: {err}"));
}

/// A gesture that draws something for each tool, with the options each needs to
/// mean anything. The single place the catalogue is enumerated: the shared-rule
/// tests below all walk this, so a new tool joins them by being added here.
fn catalogue() -> Vec<(Tool, Gesture, Vec<ToolOption>)> {
    vec![
        (
            Tool::Brush,
            Gesture::Stroke {
                points: vec![[0.0, 0.0], [2.0, 0.0]],
            },
            vec![number(PropId::SizeKm, 400.0), number(PropId::Speed, 12.0)],
        ),
        (
            Tool::Circle,
            Gesture::Point { at: [0.0, 0.0] },
            vec![
                number(PropId::DiameterKm, 800.0),
                number(PropId::Speed, 12.0),
            ],
        ),
        (
            Tool::ShapeFill,
            Gesture::Ring {
                points: vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]],
            },
            vec![number(PropId::Speed, 12.0)],
        ),
        (
            Tool::Eraser,
            Gesture::Stroke {
                points: vec![[0.0, 0.0], [2.0, 0.0]],
            },
            vec![number(PropId::SizeKm, 400.0)],
        ),
        (
            Tool::CloneStamp,
            Gesture::Stroke {
                points: vec![[0.0, 0.0], [2.0, 0.0]],
            },
            vec![
                number(PropId::SizeKm, 400.0),
                at(PropId::SourcePoint, 40.0, 0.0),
            ],
        ),
        (
            Tool::Curve,
            Gesture::Path {
                nodes: vec![
                    PathPoint {
                        at: [0.0, 0.0],
                        in_handle: None,
                        out_handle: None,
                    },
                    PathPoint {
                        at: [4.0, 0.0],
                        in_handle: None,
                        out_handle: None,
                    },
                ],
            },
            vec![number(PropId::WidthKm, 300.0), number(PropId::Speed, 12.0)],
        ),
    ]
}

// --- Each tool draws something ----------------------------------------------

/// The baseline claim of the milestone: every tool in the catalogue produces an
/// object that the evaluator actually paints. A tool whose gesture is accepted
/// but whose footprint is empty passes every property test and paints nothing.
#[test]
fn every_tool_paints_a_field() {
    for (tool, gesture, options) in catalogue() {
        let (_root, state) = project("paints");

        // The eraser writes calm, so it needs something under it to erase; and
        // the clone stamp copies what is below, so it needs something to copy.
        if matches!(tool, Tool::Eraser | Tool::CloneStamp) {
            draw(
                &state,
                Tool::Brush,
                Gesture::Stroke {
                    points: vec![[-10.0, 0.0], [50.0, 0.0]],
                },
                vec![
                    number(PropId::SizeKm, 900.0),
                    number(PropId::Speed, 20.0),
                    number(PropId::Feather, 0.0),
                    option(PropId::Direction, PropertyValue::Angle { degrees: 90.0 }),
                ],
            );
        }

        draw(&state, tool, gesture, options);

        let (speed, _) = sample(&state, ll(1.0, 0.0));
        match tool {
            // Calm is what the eraser paints; the assertion is that it changed
            // the 20 m/s underneath it, not that it produced a speed.
            Tool::Eraser => assert!(speed < 1.0, "the eraser left {speed} m/s standing"),
            _ => assert!(speed > 1.0, "{tool:?} painted nothing: {speed} m/s"),
        }
    }
}

/// Spec 6.2: the eraser is a first-class object, not a deletion. Disabling it
/// must bring back exactly what was underneath.
#[test]
fn erasing_is_reversible_because_it_is_an_object() {
    let (_root, state) = project("eraser");
    draw(
        &state,
        Tool::Brush,
        Gesture::Stroke {
            points: vec![[-10.0, 0.0], [10.0, 0.0]],
        },
        vec![
            number(PropId::SizeKm, 900.0),
            number(PropId::Speed, 20.0),
            number(PropId::Feather, 0.0),
        ],
    );
    let (before, azimuth) = sample(&state, ll(0.0, 0.0));
    assert!((before - 20.0).abs() < 0.5, "{before}");

    draw(
        &state,
        Tool::Eraser,
        Gesture::Stroke {
            points: vec![[0.0, 0.0]],
        },
        vec![number(PropId::SizeKm, 400.0), number(PropId::Feather, 0.0)],
    );
    let (erased, _) = sample(&state, ll(0.0, 0.0));
    assert!(erased < 0.5, "the eraser left {erased} m/s");

    // Disable it: the object is still there, contributing nothing.
    let eraser = document(&state).layers[0].objects[1].id.raw();
    ve_app::document::set_property(
        &state,
        eraser,
        "Enabled",
        PropertyValue::Bool { value: false },
    )
    .expect("disable");

    let (restored, restored_azimuth) = sample(&state, ll(0.0, 0.0));
    assert!(
        (restored - before).abs() < 1e-6 && (restored_azimuth - azimuth).abs() < 1e-6,
        "disabling the eraser gave back {restored} m/s at {restored_azimuth}, not {before} at {azimuth}"
    );
}

/// Spec 6.2 and 7.6: a clone stamp paints what the composite below it holds at
/// a fixed offset. The check is that it reproduces the *source*, not that it
/// paints something — a stamp that painted its own default would pass the
/// weaker test.
#[test]
fn a_clone_stamp_reproduces_what_is_under_its_source() {
    let (_root, state) = project("clone");

    // A band of 25 m/s flowing east, far from where the stamp will paint.
    draw(
        &state,
        Tool::Brush,
        Gesture::Stroke {
            points: vec![[38.0, 0.0], [42.0, 0.0]],
        },
        vec![
            number(PropId::SizeKm, 900.0),
            number(PropId::Speed, 25.0),
            number(PropId::Feather, 0.0),
            option(PropId::Direction, PropertyValue::Angle { degrees: 90.0 }),
        ],
    );

    let (source_speed, source_azimuth) = sample(&state, ll(40.0, 0.0));
    assert!((source_speed - 25.0).abs() < 0.5, "{source_speed}");

    // Nothing at the origin yet.
    assert!(sample(&state, ll(0.0, 0.0)).0 < 0.01);

    draw(
        &state,
        Tool::CloneStamp,
        Gesture::Stroke {
            points: vec![[0.0, 0.0]],
        },
        vec![
            number(PropId::SizeKm, 400.0),
            number(PropId::Feather, 0.0),
            at(PropId::SourcePoint, 40.0, 0.0),
        ],
    );

    let (cloned, cloned_azimuth) = sample(&state, ll(0.0, 0.0));
    assert!(
        (cloned - source_speed).abs() < 0.5,
        "cloned {cloned} m/s where the source holds {source_speed}"
    );
    assert!(
        (cloned_azimuth - source_azimuth).abs() < 1.0,
        "cloned {cloned_azimuth}° where the source flows {source_azimuth}°"
    );
}

/// Spec 6.2: `Aligned` moves the source with the brush, so a long stroke copies
/// a correspondingly long band; `Fixed` leaves it where it was put, so every
/// stamp along the stroke reads the same neighbourhood of it.
///
/// The two are told apart by painting over a source region that is *not*
/// uniform: a narrow band of fast flow with calm either side of it. Aligned
/// drags the reading window across that band and picks up the calm; fixed keeps
/// reading the band's middle all the way along. A uniform source would make the
/// two modes indistinguishable, which is why the fixture has an edge in it.
#[test]
fn the_two_clone_offset_modes_read_from_different_places() {
    let mut painted = Vec::new();

    for mode in [0u8, 1] {
        let (_root, state) = project("offset");

        // A short band of fast flow around 40°E, calm to the east of it.
        draw(
            &state,
            Tool::Brush,
            Gesture::Stroke {
                points: vec![[39.0, 0.0], [41.0, 0.0]],
            },
            vec![
                number(PropId::SizeKm, 500.0),
                number(PropId::Speed, 25.0),
                number(PropId::Feather, 0.0),
                option(PropId::Direction, PropertyValue::Angle { degrees: 90.0 }),
            ],
        );

        // A stroke running well east of the band's width, from the origin.
        draw(
            &state,
            Tool::CloneStamp,
            Gesture::Stroke {
                points: vec![[0.0, 0.0], [10.0, 0.0], [20.0, 0.0]],
            },
            vec![
                number(PropId::SizeKm, 400.0),
                number(PropId::Feather, 0.0),
                at(PropId::SourcePoint, 40.0, 0.0),
                pick(PropId::OffsetMode, mode),
            ],
        );

        // Far along the stroke: aligned has dragged its window off the band
        // and reads calm, fixed is still reading the band's middle.
        painted.push(sample(&state, ll(20.0, 0.0)).0);
    }

    assert!(
        painted[0] < 1.0,
        "aligned kept painting {} m/s after its source left the band",
        painted[0]
    );
    assert!(
        painted[1] > 20.0,
        "fixed read {} m/s where its source holds 25",
        painted[1]
    );
}

/// Spec 6.2: `RelativeToPath` at 0° is flow along the curve, and at 90° across
/// it. Checked on a curve running due east, where "along" and "across" have
/// known bearings rather than ones read back off the implementation.
#[test]
fn a_curve_aims_along_its_path_and_across_it() {
    for (offset, expected, what) in [(0.0, 90.0, "along"), (90.0, 180.0, "across")] {
        let (_root, state) = project("curve");
        draw(
            &state,
            Tool::Curve,
            Gesture::Path {
                nodes: vec![
                    PathPoint {
                        at: [-6.0, 0.0],
                        in_handle: None,
                        out_handle: None,
                    },
                    PathPoint {
                        at: [6.0, 0.0],
                        in_handle: None,
                        out_handle: None,
                    },
                ],
            },
            vec![
                number(PropId::WidthKm, 400.0),
                number(PropId::Speed, 15.0),
                number(PropId::Feather, 0.0),
                // Mode 1 is relative to the path tangent.
                pick(PropId::CurveDirectionMode, 1),
                option(PropId::Direction, PropertyValue::Angle { degrees: offset }),
            ],
        );

        let (speed, azimuth) = sample(&state, ll(0.0, 0.0));
        assert!(speed > 1.0, "the curve painted nothing");
        let error = (azimuth - expected + 540.0) % 360.0 - 180.0;
        assert!(
            error.abs() < 2.0,
            "{what}: flowed {azimuth}° where an eastward path wants {expected}°"
        );
    }
}

/// Spec 6.2: a Bézier curve bends. A polyline through the same two end nodes is
/// straight, so a point off the chord is inside one and outside the other —
/// which is the difference the handles are supposed to make.
#[test]
fn a_bezier_bends_where_a_polyline_does_not() {
    let nodes = |bend: bool| {
        vec![
            PathPoint {
                at: [-6.0, 0.0],
                in_handle: None,
                out_handle: bend.then_some([-2.0, 8.0]),
            },
            PathPoint {
                at: [6.0, 0.0],
                in_handle: bend.then_some([2.0, 8.0]),
                out_handle: None,
            },
        ]
    };

    let mut speeds = Vec::new();
    for bend in [false, true] {
        let (_root, state) = project("bezier");
        draw(
            &state,
            Tool::Curve,
            Gesture::Path { nodes: nodes(bend) },
            vec![
                number(PropId::WidthKm, 300.0),
                number(PropId::Speed, 15.0),
                number(PropId::Feather, 0.0),
                pick(PropId::CurveKind, u8::from(bend)),
            ],
        );
        // Well north of the chord, and inside the arc the handles pull out.
        speeds.push(sample(&state, ll(0.0, 5.0)).0);
    }

    assert!(
        speeds[0] < 0.01,
        "a polyline reached {} m/s off its chord",
        speeds[0]
    );
    assert!(
        speeds[1] > 1.0,
        "the bezier did not bend: {} m/s",
        speeds[1]
    );
}

/// Spec 6.2: the circle's gradient fill ramps from the centre outward, so the
/// speed at the middle is `speed_min` and at the rim `speed_max`. A ramp read
/// the other way round is a plausible bug that only a signed check catches.
#[test]
fn a_gradient_circle_ramps_from_its_centre_outward() {
    let (_root, state) = project("gradient");
    draw(
        &state,
        Tool::Circle,
        Gesture::Point { at: [0.0, 0.0] },
        vec![
            // Fill mode 2 is the radial gradient.
            pick(PropId::FillMode, 2),
            number(PropId::DiameterKm, 2000.0),
            number(PropId::SpeedMin, 2.0),
            number(PropId::SpeedMax, 24.0),
            number(PropId::Feather, 0.0),
        ],
    );

    let centre = sample(&state, ll(0.0, 0.0)).0;
    // Just inside the rim: 1000 km is the radius, and one degree of latitude is
    // about 111 km, so 8 degrees north is four fifths of the way out.
    let outer = sample(&state, ll(0.0, 8.0)).0;

    assert!((centre - 2.0).abs() < 1.0, "centre held {centre} m/s");
    assert!(
        outer > centre + 10.0,
        "the rim held {outer} m/s, the centre {centre}"
    );
}

/// Spec 6.2: the perimeter mode paints a ring, so the middle of the circle is
/// untouched. The property that separates a ring from a disc is the hole.
#[test]
fn a_perimeter_circle_leaves_its_middle_alone() {
    let (_root, state) = project("ring");
    draw(
        &state,
        Tool::Circle,
        Gesture::Point { at: [0.0, 0.0] },
        vec![
            pick(PropId::FillMode, 1),
            number(PropId::DiameterKm, 2000.0),
            number(PropId::RingWidthKm, 200.0),
            number(PropId::Speed, 18.0),
            number(PropId::Feather, 0.0),
        ],
    );

    assert!(
        sample(&state, ll(0.0, 0.0)).0 < 0.01,
        "the ring has no hole"
    );
    // The centreline sits at 1000 km, about 9 degrees of latitude.
    assert!(sample(&state, ll(0.0, 9.0)).0 > 1.0, "the ring is missing");
}

/// Spec 6.2: the circle's rotation is a tangential flow about its centre, so
/// clockwise and counter-clockwise are reciprocals at the same point — and
/// north of the centre a clockwise flow runs east.
#[test]
fn a_circle_rotates_about_its_centre_in_the_sense_it_was_given() {
    let mut azimuths = Vec::new();
    for sense in [0u8, 1] {
        let (_root, state) = project("rotation");
        draw(
            &state,
            Tool::Circle,
            Gesture::Point { at: [0.0, 0.0] },
            vec![
                number(PropId::DiameterKm, 2000.0),
                number(PropId::Speed, 15.0),
                number(PropId::Feather, 0.0),
                pick(PropId::RotationSense, sense),
            ],
        );
        azimuths.push(sample(&state, ll(0.0, 5.0)).1);
    }

    let error = (azimuths[0] - 90.0 + 540.0) % 360.0 - 180.0;
    assert!(
        error.abs() < 2.0,
        "clockwise flows {}° north of centre, not east",
        azimuths[0]
    );
    let opposed = (azimuths[0] - azimuths[1] + 540.0) % 360.0 - 180.0;
    assert!(
        (opposed.abs() - 180.0).abs() < 2.0,
        "the two senses are {} apart, not opposed",
        opposed.abs()
    );
}

/// Spec 6.2: the shape fill's gradient ramps along `gradient_axis`, between two
/// speeds and two bearings. Checked along an eastward axis, where the two ends
/// are known positions rather than whatever the ramp happens to produce.
#[test]
fn a_shape_fill_gradient_ramps_along_its_axis() {
    let (_root, state) = project("shape-gradient");
    draw(
        &state,
        Tool::ShapeFill,
        Gesture::Ring {
            points: vec![[-8.0, -4.0], [8.0, -4.0], [8.0, 4.0], [-8.0, 4.0]],
        },
        vec![
            // Vector mode 1 is the gradient.
            pick(PropId::VectorMode, 1),
            number(PropId::SpeedStart, 4.0),
            number(PropId::SpeedEnd, 28.0),
            option(PropId::GradientAxis, PropertyValue::Angle { degrees: 90.0 }),
            number(PropId::Feather, 0.0),
        ],
    );

    let west = sample(&state, ll(-6.0, 0.0)).0;
    let east = sample(&state, ll(6.0, 0.0)).0;
    assert!(
        east > west + 10.0,
        "the ramp runs {west} m/s west to {east} m/s east, which is not a ramp"
    );
}

// --- The cross-tool rules, per tool ------------------------------------------

/// Spec 3.5, 6.1: a shape sized in px is the same number of pixels across *and*
/// tall at every latitude, and the same size in km is a constant ground
/// distance. One assertion parameterised over the catalogue, not one per tool.
///
/// Measured on the footprint itself: the object's ground extent north-south and
/// east-west, taken from the evaluator's own coverage test rather than from
/// anything the tool believes about itself.
#[test]
fn a_projected_shape_is_round_on_the_map_and_a_geodesic_one_on_the_ground() {
    /// Tools whose size is a number they are given, so it can be held fixed
    /// while the latitude moves. The shape fill's presets are dragged out, so
    /// their size comes from the gesture and is covered by the drag instead.
    const SIZED: [(Tool, PropId); 4] = [
        (Tool::Brush, PropId::SizeKm),
        (Tool::Circle, PropId::DiameterKm),
        (Tool::Eraser, PropId::SizeKm),
        (Tool::CloneStamp, PropId::SizeKm),
    ];

    for (tool, size) in SIZED {
        for latitude in [0.0, 45.0, 70.0] {
            for (space, name) in [(0u8, "geodesic"), (1, "projected")] {
                let (_root, state) = project("sizing");
                let gesture = match tool {
                    Tool::Circle => Gesture::Point {
                        at: [0.0, latitude],
                    },
                    _ => Gesture::Stroke {
                        points: vec![[0.0, latitude]],
                    },
                };
                let mut options = vec![
                    number(size, 400.0),
                    number(PropId::Feather, 0.0),
                    pick(PropId::StampSpace, space),
                ];
                if tool == Tool::CloneStamp {
                    options.push(at(PropId::SourcePoint, 40.0, 0.0));
                }
                draw(&state, tool, gesture, options);

                let (north, east) = extents_of(&state, ll(0.0, latitude));

                match space {
                    // On the ground the footprint is 400 km across in every
                    // direction, at every latitude.
                    0 => {
                        assert!(
                            (north - 400.0).abs() < 25.0 && (east - 400.0).abs() < 25.0,
                            "{tool:?} {name} at {latitude}°: {north} km N-S by {east} km E-W"
                        );
                    }
                    // On the map it is a circle on screen, which means its
                    // east-west ground extent shrinks with the cosine — and
                    // its north-south extent does not move.
                    _ => {
                        let wanted = 400.0 * latitude.to_radians().cos();
                        assert!(
                            (north - 400.0).abs() < 25.0,
                            "{tool:?} {name} at {latitude}°: {north} km tall, not 400"
                        );
                        assert!(
                            (east - wanted).abs() < 25.0,
                            "{tool:?} {name} at {latitude}°: {east} km wide, not {wanted}"
                        );
                    }
                }
            }
        }
    }
}

/// The footprint's north-south and east-west ground extents in kilometres,
/// found by walking outward from the centre until the field stops.
fn extents_of(state: &AppState, centre: LonLat) -> (f64, f64) {
    let project = document(state);
    let scene = ve_render::scene::flatten(&project, 0);
    // Coverage rather than speed: the eraser's footprint paints calm, and a
    // speed test would measure it as having no footprint at all.
    let covered = |position: LonLat| {
        scene
            .objects
            .iter()
            .any(|flat| ve_render::scene::covers(flat, position))
    };

    // Step in metres and convert back, so the two axes are measured in the same
    // units rather than one in degrees of latitude and one in degrees of
    // longitude — which differ by the cosine that is the thing under test.
    let reach = |bearing: f64| {
        let mut far = 0.0;
        let mut step = 5_000.0;
        while step < 2_000_000.0 {
            let probe = centre.destination(ve_core::angle::Angle::new(bearing), step);
            if covered(probe) {
                far = step;
            }
            step += 5_000.0;
        }
        far
    };

    (
        (reach(0.0) + reach(180.0)) / 1000.0,
        (reach(90.0) + reach(270.0)) / 1000.0,
    )
}

/// Spec 6.1 and 6.2: `TowardPoint` and `AwayFromPoint` are exact reciprocals at
/// every cell, for every tool that has a target. Tested per tool, because the
/// rule is about the modes and not about the brush that first had them.
#[test]
fn the_two_aimed_modes_are_reciprocals_for_every_tool_that_aims() {
    for (tool, gesture, base) in catalogue() {
        if ve_core::schema::spec_for(tool.kind(), PropId::Target).is_none() {
            continue;
        }

        let mut azimuths = Vec::new();
        for mode in [1u8, 2] {
            let (_root, state) = project("aim");
            let mut options = base.clone();
            options.push(pick(PropId::DirectionMode, mode));
            options.push(at(PropId::Target, 30.0, 20.0));
            options.push(number(PropId::Feather, 0.0));
            draw(&state, tool, gesture.clone(), options);
            azimuths.push(sample(&state, ll(1.0, 0.0)).1);
        }

        let apart = (azimuths[0] - azimuths[1] + 540.0) % 360.0 - 180.0;
        assert!(
            (apart.abs() - 180.0).abs() < 0.01,
            "{tool:?}: toward {} and away {} are {} apart, not opposed",
            azimuths[0],
            azimuths[1],
            apart.abs()
        );
    }
}

/// Spec 6.1: the inspector lists exactly the options that are live and
/// editable — nothing the mode makes inert, nothing creation-only. Walked over
/// every tool, and over every variant of every mode each tool has, so a new
/// dependency that hides the wrong thing is caught wherever it is added.
#[test]
fn the_inspector_lists_exactly_what_is_live_and_editable() {
    for (tool, gesture, options) in catalogue() {
        let (_root, state) = project("inspector");
        draw(&state, tool, gesture, options);

        let object = document(&state).layers[0].objects[0].id.raw();
        let listed = ve_app::document::properties(&state, object, 0).expect("properties");

        let kind = tool.kind();
        for spec in ve_core::schema::all_specs(kind) {
            let shown = listed
                .iter()
                .any(|view| view.id == format!("{:?}", spec.id));
            let live = ve_core::schema::is_live(kind, spec.id, |id| {
                document(&state).layers[0].objects[0]
                    .props
                    .value_at(kind, id, 0)
                    .and_then(ve_core::PropValue::as_enum)
                    .unwrap_or(0)
            });
            let expected = live && !spec.creation_only;
            assert_eq!(
                shown, expected,
                "{tool:?}/{:?}: listed={shown}, but live={live} and creation_only={}",
                spec.id, spec.creation_only
            );
        }
    }
}

/// Spec 6.1: an option that defines what the object *is* is refused at the
/// write path, not merely hidden — a rule the document does not enforce is
/// decorative, and the timeline would walk straight through it.
#[test]
fn every_creation_only_option_is_refused_after_the_fact() {
    for (tool, gesture, options) in catalogue() {
        let (_root, state) = project("frozen");
        draw(&state, tool, gesture, options);
        let object = document(&state).layers[0].objects[0].id.raw();

        for spec in ve_core::schema::all_specs(tool.kind()) {
            if !spec.creation_only {
                continue;
            }
            let refused = ve_app::document::set_property(
                &state,
                object,
                &format!("{:?}", spec.id),
                PropertyValue::Choice { index: 0 },
            );
            assert!(
                refused.is_err(),
                "{tool:?}/{:?} was editable after creation",
                spec.id
            );
        }
    }
}

/// Spec 6.1 acceptance: every tool round-trips through save and load with all
/// options intact. Compared against the whole document rather than a chosen
/// property, so an option nobody thought to check still fails this.
#[test]
fn every_tool_survives_a_save_and_a_load() {
    for (tool, gesture, options) in catalogue() {
        let (root, state) = project("roundtrip");
        draw(&state, tool, gesture, options);

        let before = document(&state);
        let path = root.0.join(format!("{tool:?}.veproj"));
        ve_core::io::save(&before, &path).expect("save");
        let after = ve_core::io::load(&path).expect("load");

        assert_eq!(before, after, "{tool:?} did not survive a round trip");
    }
}

/// Spec 8.2: move, rotate and scale act on the object's frame rather than on
/// its shape, so they behave identically for every geometry. Checked by moving
/// each object and confirming its footprint moved with it — the failure this
/// catches is a geometry whose transform silently does nothing.
#[test]
fn every_geometry_moves_when_its_frame_does() {
    for (tool, gesture, options) in catalogue() {
        let (_root, state) = project("transform");
        draw(&state, tool, gesture, options);

        let object = document(&state).layers[0].objects[0].id.raw();
        let before = document(&state).layers[0].objects[0]
            .props
            .value_at(tool.kind(), PropId::Position, 0)
            .and_then(ve_core::PropValue::as_lonlat)
            .expect("a position");

        ve_app::document::set_property(
            &state,
            object,
            "Position",
            PropertyValue::Position {
                lon: before.lon + 20.0,
                lat: before.lat + 10.0,
            },
        )
        .expect("move");

        let after = document(&state).layers[0].objects[0]
            .props
            .value_at(tool.kind(), PropId::Position, 0)
            .and_then(ve_core::PropValue::as_lonlat)
            .expect("a position");
        assert!(
            (after.lon - before.lon - 20.0).abs() < 1e-6,
            "{tool:?} did not move"
        );

        // The footprint went with it: the object no longer covers where it was,
        // and does cover where it went. The eraser paints calm, so it is asked
        // about coverage rather than about speed.
        let project = document(&state);
        let scene = ve_render::scene::flatten(&project, 0);
        let covers = |position: LonLat| {
            scene
                .objects
                .iter()
                .any(|flat| ve_render::scene::covers(flat, position))
        };
        assert!(!covers(before), "{tool:?} still covers where it was");
        assert!(covers(after), "{tool:?} does not cover where it went");
    }
}

// --- Merging, which is a property of the gesture -----------------------------

/// Spec 6.1: two gestures of the same tool with identical properties and
/// overlapping footprints merge into one object. The eraser draws the same
/// stroke the brush does, so it inherits this rather than being given it.
#[test]
fn a_second_eraser_stroke_is_absorbed_by_the_first() {
    let (_root, state) = project("merge-eraser");
    let stroke = |points: Vec<[f64; 2]>| {
        (
            Gesture::Stroke { points },
            vec![number(PropId::SizeKm, 800.0), number(PropId::Feather, 0.2)],
        )
    };

    let (first, options) = stroke(vec![[0.0, 0.0], [2.0, 0.0]]);
    draw(&state, Tool::Eraser, first, options);
    let (second, options) = stroke(vec![[2.0, 0.0], [4.0, 0.0]]);
    draw(&state, Tool::Eraser, second, options);

    let objects = &document(&state).layers[0].objects;
    assert_eq!(
        objects.len(),
        1,
        "two overlapping erases stayed two objects"
    );
    assert_eq!(
        objects[0].geometry.stroke_chains().len(),
        2,
        "the second stroke is a second chain"
    );
}

/// A clone stamp reads from a displacement off its own anchor, and a merge
/// re-expresses the absorbed stroke in the target's frame — a different anchor,
/// and so a different patch of the field. Merging one would change what it
/// paints, which is the one thing a merge may never do.
#[test]
fn two_clone_strokes_stay_two_objects() {
    let (_root, state) = project("merge-clone");
    for points in [vec![[0.0, 0.0], [2.0, 0.0]], vec![[2.0, 0.0], [4.0, 0.0]]] {
        draw(
            &state,
            Tool::CloneStamp,
            Gesture::Stroke { points },
            vec![
                number(PropId::SizeKm, 800.0),
                at(PropId::SourcePoint, 40.0, 0.0),
            ],
        );
    }

    assert_eq!(
        document(&state).layers[0].objects.len(),
        2,
        "clone strokes merged, which moves where one of them samples from"
    );
}

/// Spec 6.1: two gestures that differ in *any* property never merge — including
/// the stamp's shape or space, whose footprints are different shapes.
#[test]
fn gestures_that_differ_in_a_frozen_option_never_merge() {
    for (id, index) in [(PropId::BrushShape, 1u8), (PropId::StampSpace, 1)] {
        let (_root, state) = project("no-merge");
        draw(
            &state,
            Tool::Eraser,
            Gesture::Stroke {
                points: vec![[0.0, 0.0], [2.0, 0.0]],
            },
            vec![number(PropId::SizeKm, 800.0)],
        );
        draw(
            &state,
            Tool::Eraser,
            Gesture::Stroke {
                points: vec![[2.0, 0.0], [4.0, 0.0]],
            },
            vec![number(PropId::SizeKm, 800.0), pick(id, index)],
        );

        assert_eq!(
            document(&state).layers[0].objects.len(),
            2,
            "strokes differing in {id:?} merged anyway"
        );
    }
}

/// Different tools never merge, however alike their gestures. The eraser and
/// the clone stamp draw exactly the strokes the brush does, which is what makes
/// this worth asserting rather than assuming.
#[test]
fn the_brush_like_tools_do_not_merge_into_each_other() {
    let (_root, state) = project("cross-tool");
    for tool in [Tool::Brush, Tool::Eraser] {
        let mut options = vec![number(PropId::SizeKm, 800.0)];
        if tool == Tool::CloneStamp {
            options.push(at(PropId::SourcePoint, 40.0, 0.0));
        }
        draw(
            &state,
            tool,
            Gesture::Stroke {
                points: vec![[0.0, 0.0], [2.0, 0.0]],
            },
            options,
        );
    }

    let objects = &document(&state).layers[0].objects;
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[0].tool, ToolKind::Brush);
    assert_eq!(objects[1].tool, ToolKind::Eraser);
}
