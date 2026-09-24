#![allow(clippy::expect_used, clippy::unwrap_used, reason = "test assertions")]
//! Shape editing, timeline operations, and destructive erasure through the IPC implementations.

use ve_app::commands::AppState;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_app::{animation, document, edit, shape_animation as shape};
use ve_core::{Animatable, Geometry, Object, PropId, PropValue, ToolKind};

struct Fixture {
    state: AppState,
    root: std::path::PathBuf,
    object: u64,
    layer: u64,
}
impl Fixture {
    fn new(tool: ToolKind) -> Self {
        let root = std::env::temp_dir().join(format!(
            "ve-shape-{}-{}",
            std::process::id(),
            ve_core::Id::new().raw()
        ));
        let state = AppState::new(AppPaths::in_directory(&root).unwrap());
        projects::create(
            &state,
            NewProjectRequest {
                name: "Shape".into(),
                field_kind: "wind".into(),
                resolution: "1.0".into(),
                step_hours: 1,
                step_count: 11,
            },
            false,
        )
        .unwrap();
        let mut o = Object::new(tool, "Animated object", 11);
        o.geometry = Geometry::Rect {
            half_width_m: 100_000.0,
            half_height_m: 100_000.0,
        };
        o.props.insert(
            PropId::Position,
            Animatable::constant(PropValue::LonLat(ve_core::LonLat::new(0.0, 0.0).unwrap())),
        );
        o.props
            .insert(PropId::Speed, Animatable::constant(PropValue::F32(10.0)));
        o.props
            .insert(PropId::Feather, Animatable::constant(PropValue::F32(0.0)));
        let object = o.id.raw();
        let layer;
        {
            let mut session = state.session.lock().unwrap();
            let open = session.require_open().unwrap();
            layer = open.project.layers[0].id.raw();
            open.project.layers[0].objects.push(o);
        }
        Self {
            state,
            root,
            object,
            layer,
        }
    }
    fn doc(&self) -> ve_core::Project {
        self.state
            .session
            .lock()
            .unwrap()
            .require_open()
            .unwrap()
            .project
            .clone()
    }
    fn controls(&self, step: u32) -> shape::ShapeControls {
        shape::controls_of(&self.state, self.object, step).unwrap()
    }
    fn local_move(&self, step: u32, point: usize, to: [f64; 2]) {
        let controls = self.controls(step);
        let doc = self.doc();
        let flat = ve_render::scene::flatten_object(
            doc.object(ve_core::Id::from_raw(self.object)).unwrap(),
            step,
        )
        .unwrap();
        let at = flat.frame.to_global(to);
        shape::move_point(
            &self.state,
            self.object,
            step,
            0,
            point,
            [at.lon, at.lat],
            controls.revision,
        )
        .unwrap();
    }
    fn erase(&self, radius: f64, step: Option<u32>) {
        document::stroke_erase(
            &self.state,
            document::EraseStroke {
                projection_origin: None,
                points: vec![[0.0, 0.0]],
                radius_km: radius,
                square: false,
                space: edit::StampSpace::Geodesic,
                feather: 0.0,
                step,
                at_step: step.unwrap_or(0),
                layer: Some(self.layer),
            },
        )
        .unwrap();
    }
    fn covered(&self, step: u32, position: [f64; 2]) -> bool {
        let scene = ve_render::scene::flatten(&self.doc(), step);
        ve_render::cpu::sample_scene_covered(
            &scene,
            ve_core::LonLat::new(position[0], position[1]).unwrap(),
        )
        .is_some()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn controls_are_read_only_and_each_point_keys_independently_for_create_and_edit_objects() {
    for tool in [
        ToolKind::ShapeFill,
        ToolKind::Intensity,
        ToolKind::Turn,
        ToolKind::Mask,
    ] {
        let f = Fixture::new(tool);
        let before = f.doc();
        let controls = f.controls(10);
        assert!(controls.rings[0].len() >= 4);
        assert_eq!(
            f.doc(),
            before,
            "entering and leaving mode must not change the document"
        );
        f.local_move(10, 0, [-300_000.0, -100_000.0]);
        let doc = f.doc();
        let o = doc.object(ve_core::Id::from_raw(f.object)).unwrap();
        let a = o.shape_animation.as_ref().unwrap();
        assert_eq!(a.rings[0][0].keys.len(), 2);
        assert!(a.rings[0][1..].iter().all(|p| p.keys.is_empty()));
        let at5 = a.rings[0][0].at(5);
        assert!((at5.x + 200_000.0).abs() < 1e-5);
        assert!((at5.y + 100_000.0).abs() < 1e-5);
        let flat = ve_render::scene::flatten_object(o, 5).unwrap();
        let ve_render::sdf::Shape::Contours { rings, .. } = &flat.shape else {
            panic!("animated footprint");
        };
        assert!((rings[0][0][0] - at5.x).abs() < 1e-6);
        edit::undo_for_test(&f.state).unwrap();
        assert_eq!(f.doc(), before, "one drag is one exact undo");
        edit::redo_for_test(&f.state).unwrap();
        assert_eq!(f.doc(), doc);
    }
}

#[test]
fn shape_markers_pin_move_ease_delete_and_restore_after_timeline_shrink() {
    let f = Fixture::new(ToolKind::ShapeFill);
    f.local_move(10, 0, [-300_000.0, -100_000.0]);
    animation::key_at(&f.state, f.object, "shape", 5, None).unwrap();
    let doc = f.doc();
    let a = doc
        .object(ve_core::Id::from_raw(f.object))
        .unwrap()
        .shape_animation
        .as_ref()
        .unwrap();
    assert!(a.rings.iter().flatten().all(|p| p.keys.contains_key(&5)));
    animation::move_key(&f.state, f.object, "shape", 5, 6, Some("shape-drag".into())).unwrap();
    animation::ease_from(
        &f.state,
        f.object,
        "shape",
        6,
        animation::InterpolationView::Step,
    )
    .unwrap();
    let track = animation::tracks_of(&f.state, f.object, 6)
        .unwrap()
        .tracks
        .into_iter()
        .find(|t| t.property == "shape")
        .unwrap();
    assert!(track.keyed_here);
    assert_eq!(
        track.keys.iter().map(|k| k.step).collect::<Vec<_>>(),
        vec![0, 6, 10]
    );
    animation::unkey_at(&f.state, f.object, "shape", 6).unwrap();
    let before = f.doc();
    assert_eq!(before.shrink_impact(6).keyframes, 1);
    animation::resize_steps(&f.state, 6).unwrap();
    assert_eq!(f.doc().keyframes_after(5).0, 0);
    edit::undo_for_test(&f.state).unwrap();
    assert_eq!(f.doc(), before);
}

#[test]
fn erasures_stay_removed_after_shape_keys_undo_and_save_load() {
    let f = Fixture::new(ToolKind::ShapeFill);
    f.local_move(10, 0, [-300_000.0, -100_000.0]);
    assert!(f.covered(5, [0.0, 0.0]));
    f.erase(35.0, None);
    for step in 0..11 {
        assert!(!f.covered(step, [0.0, 0.0]));
    }
    assert!(f.covered(5, [0.7, 0.0]));
    f.local_move(5, 1, [-120_000.0, -150_000.0]);
    for step in 0..11 {
        assert!(
            !f.covered(step, [0.0, 0.0]),
            "keying cannot resurrect an erasure"
        );
    }
    let doc = f.doc();
    let json = ve_core::io::to_canonical_json(&doc).unwrap();
    let reopened = ve_core::io::from_json(&json).unwrap();
    assert_eq!(ve_core::io::to_canonical_json(&reopened).unwrap(), json);
    let scene = ve_render::scene::flatten(&reopened, 5);
    assert!(
        ve_render::cpu::sample_scene_covered(&scene, ve_core::LonLat::new(0.0, 0.0).unwrap())
            .is_none()
    );
    assert!(!ve_render::scene::covers(
        &scene.objects[0],
        ve_core::LonLat::new(0.0, 0.0).unwrap()
    ));
    edit::undo_for_test(&f.state).unwrap(); // point drag
    edit::undo_for_test(&f.state).unwrap(); // erase
    assert!(f.covered(5, [0.0, 0.0]));
    edit::redo_for_test(&f.state).unwrap();
    assert!(!f.covered(5, [0.0, 0.0]));
}

#[test]
fn erasing_current_frame_completely_keeps_geometry_visible_in_other_frames() {
    let f = Fixture::new(ToolKind::ShapeFill);
    f.local_move(10, 0, [-700_000.0, -100_000.0]);
    f.erase(180.0, None); // contains all of step zero, but not the later tip
    assert!(f.doc().object(ve_core::Id::from_raw(f.object)).is_some());
    assert!(!f.covered(0, [0.0, 0.0]));
    assert!(f.covered(10, [-5.0, -0.85]));
    f.erase(1000.0, None);
    assert!(f.doc().object(ve_core::Id::from_raw(f.object)).is_none());
}

#[test]
fn a_single_frame_erasure_does_not_remove_neighbouring_frames() {
    let f = Fixture::new(ToolKind::ShapeFill);
    f.local_move(10, 0, [-300_000.0, -100_000.0]);
    f.erase(180.0, Some(5));
    assert!(!f.covered(5, [0.0, 0.0]));
    assert!(f.covered(4, [0.0, 0.0]));
    assert!(f.covered(6, [0.0, 0.0]));
}

#[test]
fn stale_drag_and_locked_layer_cannot_write_geometry() {
    let f = Fixture::new(ToolKind::ShapeFill);
    let controls = f.controls(0);
    animation::key_at(&f.state, f.object, "shape", 5, None).unwrap();
    assert!(shape::move_point(&f.state, f.object, 0, 0, 0, [1.0, 1.0], controls.revision).is_err());
    {
        let mut session = f.state.session.lock().unwrap();
        session.require_open().unwrap().project.layers[0].locked = true;
    }
    assert!(shape::controls_of(&f.state, f.object, 0).is_err());
    assert!(shape::key_at(&f.state, f.object, 0).is_err());
}

#[test]
fn moving_an_anchor_retains_animated_perimeter_on_the_ground() {
    let f = Fixture::new(ToolKind::ShapeFill);
    f.local_move(10, 0, [-300_000.0, -100_000.0]);
    f.erase(35.0, None);
    let original = f.doc();
    let before = f.controls(5);
    ve_app::transform::start_transform(
        &f.state,
        &[f.object],
        5,
        ve_app::transform::TransformKind::Anchor,
        0.0,
        0.0,
        false,
    )
    .unwrap();
    ve_app::transform::update_transform(&f.state, 0.1, 0.1).unwrap();
    ve_app::transform::update_transform(&f.state, 0.2, 0.1).unwrap();
    document::finish_gesture(&f.state).unwrap();
    let after = f.controls(5);
    for (a, b) in before
        .rings
        .iter()
        .flatten()
        .zip(after.rings.iter().flatten())
    {
        let distance = ve_core::LonLat::new(a[0], a[1])
            .unwrap()
            .distance_m(ve_core::LonLat::new(b[0], b[1]).unwrap());
        assert!(
            distance < 5.0,
            "anchor move shifted a point by {distance} m"
        );
    }
    assert!(
        !f.covered(5, [0.0, 0.0]),
        "the erasure stays on the ground too"
    );
    edit::undo_for_test(&f.state).unwrap();
    assert_eq!(f.doc(), original, "an anchor drag with cuts is one undo");
}

#[test]
fn starting_shape_animation_does_not_restore_previously_erased_geometry() {
    let f = Fixture::new(ToolKind::ShapeFill);
    f.erase(35.0, None);
    assert!(!f.covered(0, [0.0, 0.0]));
    let controls = f.controls(10);
    assert!(!controls.rings.is_empty());
    f.local_move(10, 0, [-300_000.0, -100_000.0]);
    for step in 0..11 {
        assert!(!f.covered(step, [0.0, 0.0]));
    }
}

#[test]
fn a_mercator_pixel_eraser_cuts_a_ground_object_in_screen_space_and_survives_save() {
    let fixture = Fixture::new(ToolKind::ShapeFill);
    let centre = ve_core::LonLat::new(0.0, 75.0).unwrap();
    {
        let mut session = fixture.state.session.lock().unwrap();
        let object = session
            .require_open()
            .unwrap()
            .project
            .object_mut(ve_core::Id::from_raw(fixture.object))
            .unwrap();
        object.props.insert(
            PropId::Position,
            Animatable::constant(PropValue::LonLat(centre)),
        );
        object.geometry = Geometry::Rect {
            half_width_m: 500_000.0,
            half_height_m: 500_000.0,
        };
    }
    let space = ve_render::aeqd::Space::Mercator;
    let y = space.y_of(75.0);
    let inside = [0.0, space.lat_of(y + 0.8)];
    let outside = [0.0, space.lat_of(y + 1.2)];
    assert!(fixture.covered(0, outside));
    document::stroke_erase(
        &fixture.state,
        document::EraseStroke {
            projection_origin: None,
            points: vec![[0.0, 75.0]],
            radius_km: ve_render::aeqd::M_PER_DEGREE / 1000.0,
            square: false,
            space: edit::StampSpace::Mercator,
            feather: 0.0,
            step: None,
            at_step: 0,
            layer: Some(fixture.layer),
        },
    )
    .unwrap();
    assert!(!fixture.covered(0, inside));
    assert!(
        fixture.covered(0, outside),
        "the pixel radius is one projected degree, not one latitude degree"
    );
    let path = fixture.root.join("pixel-cut.veproj");
    projects::save_as(&fixture.state, path.to_string_lossy().into_owned()).unwrap();
    projects::open(&fixture.state, path.to_string_lossy().into_owned(), true).unwrap();
    assert!(!fixture.covered(0, inside));
    assert!(fixture.covered(0, outside));
}

#[test]
fn captured_objects_have_no_shape_controls_or_key_commands() {
    for tool in [ToolKind::Macro, ToolKind::Patch, ToolKind::Liquify] {
        let f = Fixture::new(tool);
        assert!(
            animation::tracks_of(&f.state, f.object, 0)
                .unwrap()
                .tracks
                .iter()
                .all(|t| t.property != "shape")
        );
        assert!(shape::controls_of(&f.state, f.object, 0).is_err());
        assert!(animation::key_at(&f.state, f.object, "shape", 2, None).is_err());
        assert!(
            f.doc()
                .object(ve_core::Id::from_raw(f.object))
                .unwrap()
                .shape_animation
                .is_none()
        );
    }
}
