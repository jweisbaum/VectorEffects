#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Liquify relocates a selected region and preserves saved legacy smears.

use ve_app::commands::AppState;
use ve_app::create::{self, Gesture, NewObject, Tool, ToolOption};
use ve_app::document::{self, PropertyValue};
use ve_app::edit::{self, BrushStroke};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_core::LonLat;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-liquify-{}-{label}-{}",
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

/// A project with a northward field whose speed steps up at the prime
/// meridian: 5 m/s to the west of it and 15 to the east, both hard-edged.
fn stepped(label: &str) -> (TempRoot, AppState) {
    let root = TempRoot::new(label);
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Liquify".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 2,
        },
        false,
    )
    .expect("create");
    for (points, speed) in [
        ([[-20.0, 0.0], [-0.5, 0.0]], 5.0),
        ([[0.5, 0.0], [20.0, 0.0]], 15.0),
    ] {
        edit::paint(
            &state,
            BrushStroke {
                points: points.to_vec(),
                size_km: 1_400.0,
                speed_mps: speed,
                direction_toward_deg: 0.0,
                feather: 0.0,
                layer: None,
                ..Default::default()
            },
        )
        .expect("paint");
    }
    (root, state)
}

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat).expect("position")
}

fn speed(state: &AppState, at: LonLat) -> f64 {
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let scene = ve_render::scene::flatten(&project, 0);
    let uv = ve_render::cpu::sample_scene(&scene, at);
    ve_core::vector::speed_azimuth_from_uv(uv).0
}

fn number(property: &str, value: f64) -> ToolOption {
    ToolOption {
        property: property.to_owned(),
        value: PropertyValue::Number { value },
    }
}

/// A selected western field lands intact east of the step.
#[test]
fn a_selected_region_moves_intact_over_the_step() {
    let (_root, state) = stepped("east");
    assert!(
        (speed(&state, ll(3.0, 0.0)) - 15.0).abs() < 0.5,
        "east of the step is fast"
    );

    // A single painted disc moved thirteen degrees east.
    create::create(
        &state,
        NewObject {
            tool: Tool::Liquify,
            gesture: Gesture::Relocate {
                points: vec![[-10.0, 0.0]],
                from: [-10.0, 0.0],
                to: [3.0, 0.0],
            },
            options: vec![
                number("SizeKm", 500.0),
                number("InterpolationDistanceKm", 300.0),
            ],
            layer: None,
        },
    )
    .expect("liquify");

    assert!(
        (speed(&state, ll(3.0, 0.0)) - 5.0).abs() < 0.5,
        "the slow field was not dragged east: {} m/s",
        speed(&state, ll(3.0, 0.0))
    );
    // Well outside the stroke nothing moved.
    assert!((speed(&state, ll(15.0, 0.0)) - 15.0).abs() < 0.5);
    assert!((speed(&state, ll(-15.0, 0.0)) - 5.0).abs() < 0.5);
}

/// Cells beyond the destination and its interpolation band stay unchanged.
#[test]
fn relative_displacement_does_not_overwrite_distant_field() {
    let (_root, state) = stepped("strength");
    create::create(
        &state,
        NewObject {
            tool: Tool::Liquify,
            gesture: Gesture::Relocate {
                points: vec![[-10.0, 0.0]],
                from: [-10.0, 0.0],
                to: [-2.0, 0.0],
            },
            options: vec![
                number("SizeKm", 500.0),
                number("InterpolationDistanceKm", 300.0),
            ],
            layer: None,
        },
    )
    .expect("liquify");
    assert!(
        (speed(&state, ll(4.5, 0.0)) - 15.0).abs() < 0.5,
        "the move should not reach four and a half degrees: {} m/s",
        speed(&state, ll(4.5, 0.0))
    );
}

/// Two source selections retain their own independent displacements.
#[test]
fn a_liquify_never_merges() {
    let (_root, state) = stepped("merge");
    for points in [
        vec![[-4.0, 0.0], [-1.0, 0.0]],
        vec![[-2.0, 0.0], [1.0, 0.0]],
    ] {
        create::create(
            &state,
            NewObject {
                tool: Tool::Liquify,
                gesture: Gesture::Stroke { points },
                options: vec![number("SizeKm", 1_200.0)],
                layer: None,
            },
        )
        .expect("liquify");
    }
    let objects = document::tree(&state, 0).expect("tree").layers[0]
        .objects
        .len();
    assert_eq!(
        objects, 4,
        "two brush strokes and two liquifies, none absorbed"
    );
}

#[test]
fn saved_legacy_smears_keep_their_recorded_strength() {
    use ve_core::{
        PropValue,
        document::{Geometry, Object, SmearPoint},
        schema::{PropId, ToolKind},
    };
    for (strength, probe, expected) in [(1.0, 3.0, 5.0), (0.2, 4.5, 15.0)] {
        let (root, state) = stepped("legacy");
        let mut object = Object::new(ToolKind::Liquify, "Legacy smear", 2);
        object
            .props
            .get_mut(PropId::Position)
            .unwrap()
            .set_base(PropValue::LonLat(ll(-5.0, 0.0)));
        object
            .props
            .get_mut(PropId::SizeKm)
            .unwrap()
            .set_base(PropValue::F32(1200.0));
        let frame = ve_render::aeqd::Frame::in_space(
            ll(-5.0, 0.0),
            0.0,
            100.0,
            ve_render::aeqd::Space::Geodesic,
        );
        let points = [ll(-5.0, 0.0), ll(0.0, 0.0), ll(5.0, 0.0)].map(|p| frame.to_local(p));
        object.geometry = Geometry::Smear {
            chains: vec![
                points
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let before = points[i.saturating_sub(1)];
                        SmearPoint::new(
                            p[0],
                            p[1],
                            (p[0] - before[0]) * strength,
                            (p[1] - before[1]) * strength,
                        )
                    })
                    .collect(),
            ],
        };
        let mut session = state.session.lock().unwrap();
        let open = session.require_open().unwrap();
        open.project.layers[0].objects.push(object);
        let path = root.0.join("legacy.veproj");
        ve_core::io::save(&open.project, &path).unwrap();
        open.project = ve_core::io::load(&path).unwrap();
        drop(session);
        assert!((speed(&state, ll(probe, 0.0)) - expected).abs() < 0.5);
    }
}

#[test]
fn selection_and_destination_have_surface_outlines_and_one_displacement_track() {
    use ve_core::schema::PropId;
    for at in [[179.0, 72.0], [-150.0, 0.0]] {
        let (_root, state) = stepped("outlines");
        let made = create::create(
            &state,
            NewObject {
                tool: Tool::Liquify,
                gesture: Gesture::Relocate {
                    points: vec![at, [at[0], at[1] + 1.0]],
                    from: at,
                    to: [at[0] + 4.0, at[1]],
                },
                options: vec![number("SizeKm", 200.0), number("Feather", 0.6)],
                layer: None,
            },
        )
        .unwrap();
        let outlines =
            ve_app::transform::outlines_at(&state, 0, Some(Tool::Liquify), &[], None, false)
                .unwrap();
        let outline = outlines.iter().find(|o| o.object == made.object).unwrap();
        let moved = outline.relocation.as_ref().unwrap();
        let session = state.session.lock().unwrap();
        let object = session
            .open
            .as_ref()
            .unwrap()
            .project
            .object(ve_core::Id::from_raw(made.object))
            .unwrap();
        let flat = ve_render::scene::flatten_object(object, 0).unwrap();
        let ve_render::scene::Modifier::Relocate { displacement, .. } = flat.modifier.unwrap()
        else {
            panic!("relocate")
        };
        let local: Vec<_> = moved
            .connection
            .iter()
            .map(|p| flat.frame.to_local(ll(p[0], p[1])))
            .collect();
        for i in 0..2 {
            assert!((local.last().unwrap()[i] - local[0][i] - displacement[i]).abs() < 0.05);
        }
        // Every point stays in the plane between the two centres, then is
        // lifted back onto the globe instead of a chord through the sphere.
        for (index, point) in local.iter().enumerate() {
            for axis in 0..2 {
                assert!(
                    (point[axis] - local[0][axis] - displacement[axis] * index as f64 / 64.0).abs()
                        < 0.05
                );
            }
        }
        assert!(matches!(
            &moved.destination,
            ve_app::transform::ObjectOutline::Contours { .. }
        ));
        assert_eq!(
            object.props.get(PropId::Feather).unwrap().base().as_f32(),
            Some(0.6)
        );
        drop(session);
        let tracks = ve_app::animation::tracks_of(&state, made.object, 0).unwrap();
        let positions: Vec<_> = tracks
            .tracks
            .iter()
            .filter(|t| t.property.starts_with("Displacement"))
            .collect();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].label, "Displacement position");
        assert!(matches!(positions[0].base, PropertyValue::Offset { .. }));
        let samples =
            ve_app::animation::samples_of(&state, made.object, "DisplacementPosition").unwrap();
        assert_eq!(samples.series.len(), 2);
        assert!(samples.series.iter().all(|s| s.unit == "kilometres"));
    }
}
