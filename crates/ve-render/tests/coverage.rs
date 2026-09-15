#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test setup; clippy's allow-in-tests does not reach helpers in tests/"
)]
//! Zero and undefined are different things (spec.md 8.5, D58).
//!
//! The field itself cannot tell them apart — an unwritten cell composites as
//! calm and always has. A **capture** must, because a patch pasted from one
//! writes nothing where its source was undefined and overwrites where its
//! source was a real zero. These check the bit that says which.

use ve_core::angle::Angle;
use ve_core::document::{Geometry, Layer, LocalPoint, Object};
use ve_core::project::{FieldKind, Project, ProjectSettings, Resolution, StepHours};
use ve_core::schema::{PropId, ToolKind};
use ve_core::{LonLat, PropValue};
use ve_render::cpu::{sample_scene, sample_scene_covered};
use ve_render::scene::flatten;

fn project(objects: Vec<Object>) -> Project {
    let mut project = Project::new(
        "Coverage",
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 4),
    );
    let mut layer = Layer::new("Paint");
    layer.objects = objects;
    project.layers.push(layer);
    project
}

fn set(object: &mut Object, id: PropId, value: PropValue) {
    object
        .props
        .get_mut(id)
        .expect("the property")
        .set_base(value);
}

/// A single hard-edged stamp of one tool, `size_km` across.
fn stamp(tool: ToolKind, at: LonLat, size_km: f64, speed: f64) -> Object {
    let mut object = Object::new(tool, "stamp", 4);
    object.geometry = Geometry::Stroke {
        chains: vec![vec![LocalPoint::new(0.0, 0.0)]],
    };
    set(&mut object, PropId::Position, PropValue::LonLat(at));
    set(&mut object, PropId::SizeKm, PropValue::F32(size_km as f32));
    set(&mut object, PropId::Feather, PropValue::F32(0.0));
    if object.props.get(PropId::Speed).is_some() {
        set(&mut object, PropId::Speed, PropValue::F32(speed as f32));
        set(
            &mut object,
            PropId::Direction,
            PropValue::Angle(Angle::new(90.0)),
        );
    }
    object
}

fn at(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat).unwrap()
}

/// Open water is undefined; a stroke is covered. Both read as the same field.
#[test]
fn an_unwritten_cell_is_undefined_and_a_written_one_is_not() {
    let project = project(vec![stamp(ToolKind::Brush, at(0.0, 0.0), 600.0, 20.0)]);
    let scene = flatten(&project, 0);

    let inside = at(0.0, 0.0);
    let outside = at(40.0, 0.0);
    assert!(sample_scene_covered(&scene, inside).is_some(), "the stroke");
    assert!(
        sample_scene_covered(&scene, outside).is_none(),
        "open water was never written"
    );
    // The *field* says calm in both places, which is exactly why the field
    // cannot answer this question and a capture needs its own bit.
    assert!(sample_scene(&scene, outside).u.abs() < 1e-6);
}

/// A brush painting nothing but calm is still a written cell: the user said
/// "calm here", and a patch taken over it should paint calm.
#[test]
fn a_deliberate_zero_is_covered() {
    let project = project(vec![stamp(ToolKind::Brush, at(0.0, 0.0), 600.0, 0.0)]);
    let scene = flatten(&project, 0);
    let sample = sample_scene_covered(&scene, at(0.0, 0.0)).expect("a written calm");
    assert!(sample.u.abs() < 1e-6 && sample.v.abs() < 1e-6);
}

#[test]
fn an_empty_layer_does_not_obscure_painted_objects() {
    let mut project = project(vec![stamp(ToolKind::Brush, at(0.0, 0.0), 600.0, 20.0)]);
    let expected = sample_scene_covered(&flatten(&project, 0), at(0.0, 0.0));
    assert!(expected.is_some());
    let mut empty = Layer::new("Empty on top");
    empty.parameter = FieldKind::Current;
    empty.speed_range = Some(ve_core::document::SpeedRange {
        min_mps: 0.0,
        max_mps: 0.0,
    });
    project.layers.push(empty);
    assert_eq!(
        sample_scene_covered(&flatten(&project, 0), at(0.0, 0.0)),
        expected
    );
    assert!(sample_scene_covered(&flatten(&project, 0), at(40.0, 0.0)).is_none());
}

/// A mask writes calm to the field and **removes** coverage, so a capture
/// taken over one is transparent there rather than a hole of dead air (D58).
#[test]
fn a_mask_removes_coverage_rather_than_writing_a_zero() {
    let project = project(vec![
        stamp(ToolKind::Brush, at(0.0, 0.0), 1200.0, 20.0),
        stamp(ToolKind::Mask, at(0.0, 0.0), 400.0, 0.0),
    ]);
    let scene = flatten(&project, 0);

    // Under the mask: the field is calm, as it has always been...
    let masked = at(0.0, 0.0);
    assert!(sample_scene(&scene, masked).u.abs() < 1e-6);
    // ...and the capture has nothing there at all.
    assert!(
        sample_scene_covered(&scene, masked).is_none(),
        "a masked cell is undefined, not calm"
    );

    // Outside the mask but inside the stroke, the stroke still covers.
    let painted = at(4.0, 0.0);
    let sample = sample_scene_covered(&scene, painted).expect("the stroke shows");
    assert!(
        (sample.u - 20.0).abs() < 0.5,
        "eastward came out {}",
        sample.u
    );
}

/// A modifier writes what it read: it cannot conjure a field where there is
/// none, so it must not conjure coverage either.
#[test]
fn a_modifier_over_nothing_covers_nothing() {
    let mut turn = stamp(ToolKind::Turn, at(0.0, 0.0), 600.0, 0.0);
    set(&mut turn, PropId::TurnAmountDeg, PropValue::F32(45.0));
    let project = project(vec![turn]);
    let scene = flatten(&project, 0);
    assert!(
        sample_scene_covered(&scene, at(0.0, 0.0)).is_none(),
        "a rotate-flow over open water has nothing to rotate and nothing to capture"
    );
}

/// Copying a larger footprint must leave source holes transparent in both
/// modes, whether they fall on empty map or another painted vector.
#[test]
fn clone_copies_only_defined_source_data_including_real_calm() {
    for offset in [0, 1] {
        for edge in [0, 1] {
            for speed in [0.0, 20.0] {
                let source = stamp(ToolKind::Brush, at(0.0, 0.0), 200.0, speed);
                let destination = stamp(ToolKind::Brush, at(20.0, 0.0), 1200.0, 8.0);
                let mut clone = stamp(ToolKind::CloneStamp, at(20.0, 0.0), 1000.0, 0.0);
                set(
                    &mut clone,
                    PropId::SourcePoint,
                    PropValue::LonLat(at(0.0, 0.0)),
                );
                set(&mut clone, PropId::OffsetMode, PropValue::Enum(offset));
                set(&mut clone, PropId::EdgeMode, PropValue::Enum(edge));
                let scene = flatten(&project(vec![source.clone(), clone.clone()]), 0);
                assert!(
                    sample_scene_covered(&scene, at(23.0, 0.0)).is_none(),
                    "source hole must stay undefined"
                );
                let copied =
                    sample_scene_covered(&scene, at(20.0, 0.0)).expect("even calm is defined");
                assert!((copied.u - speed as f32).abs() < 0.001);
                let scene = flatten(&project(vec![source, destination, clone]), 0);
                let retained = sample_scene_covered(&scene, at(23.0, 0.0))
                    .expect("destination survives a source hole");
                assert!((retained.u - 8.0).abs() < 0.001);
                let copied = sample_scene_covered(&scene, at(20.0, 0.0)).unwrap();
                assert!((copied.u - speed as f32).abs() < 0.001);
            }
        }
    }
}

#[test]
fn cloning_a_soft_source_edge_preserves_its_vector_and_coverage() {
    let mut source = stamp(ToolKind::Brush, at(0.0, 0.0), 800.0, 20.0);
    set(&mut source, PropId::Feather, PropValue::F32(0.5));
    let mut clone = stamp(ToolKind::CloneStamp, at(20.0, 0.0), 1200.0, 0.0);
    set(
        &mut clone,
        PropId::SourcePoint,
        PropValue::LonLat(at(0.0, 0.0)),
    );
    let scene = flatten(&project(vec![source, clone]), 0);
    let original = ve_render::cpu::composite(&scene, at(2.7, 0.0));
    let copied = ve_render::cpu::composite(&scene, at(22.7, 0.0));
    assert!(original.coverage > 0.1 && original.coverage < 0.9);
    assert!((original.coverage - copied.coverage).abs() < 0.001);
    assert!(
        (original.uv.u - copied.uv.u).abs() < 0.001,
        "do not feather the source twice"
    );
}
