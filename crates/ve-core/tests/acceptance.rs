//! M1 acceptance criteria, as executable checks.
//!
//! Two property-based tests carry most of the weight (see `plan.md` M1):
//! any project survives a save/load/save cycle byte-identically, and any
//! sequence of commands followed by the same number of undos restores the
//! starting document exactly.
//!
//! Both are generated rather than hand-written because the interesting failures
//! are in combinations nobody thinks to write down: a keyframe landing exactly
//! on a range boundary, a layer removed while it holds the object a later
//! command targets, a shrink that clamps a range that was already clamped.

use proptest::prelude::*;
use ve_core::command::Command;
use ve_core::document::{Geometry, Layer, LocalPoint, Object, StepRange};
use ve_core::history::History;
use ve_core::io;
use ve_core::keyframe::Animatable;
use ve_core::project::{FieldKind, Project, ProjectSettings, Resolution, StepHours};
use ve_core::schema::{PropId, ToolKind, all_specs};
use ve_core::value::{Interpolation, PropKind, PropValue};

const TOOLS: [ToolKind; ToolKind::ALL.len()] = ToolKind::ALL;
const INTERPS: [Interpolation; 5] = [
    Interpolation::Step,
    Interpolation::Linear,
    Interpolation::EaseIn,
    Interpolation::EaseOut,
    Interpolation::EaseInOut,
];

// --- Generators --------------------------------------------------------------
//
// Projects are generated from a compact recipe rather than by composing
// strategies over the document types directly. The recipe is far easier to
// shrink, so a failure reports a minimal reproduction instead of a wall of
// generated JSON.

#[derive(Debug, Clone)]
struct KeyRecipe {
    prop: usize,
    step: u32,
    magnitude: f32,
    interp: usize,
}

#[derive(Debug, Clone)]
struct ObjectRecipe {
    tool: usize,
    name: String,
    range: (u32, u32),
    points: Vec<(f64, f64)>,
    keys: Vec<KeyRecipe>,
}

#[derive(Debug, Clone)]
struct ProjectRecipe {
    name: String,
    field_kind: usize,
    resolution: usize,
    step_hours: usize,
    step_count: u32,
    layers: Vec<(String, bool, bool, Vec<ObjectRecipe>)>,
}

fn arb_key() -> impl Strategy<Value = KeyRecipe> {
    (0usize..16, 0u32..64, -50.0f32..150.0, 0usize..INTERPS.len()).prop_map(
        |(prop, step, magnitude, interp)| KeyRecipe {
            prop,
            step,
            magnitude,
            interp,
        },
    )
}

fn arb_object() -> impl Strategy<Value = ObjectRecipe> {
    (
        0usize..TOOLS.len(),
        "[a-z ]{1,12}",
        (0u32..64, 0u32..64),
        prop::collection::vec((-5.0e6..5.0e6f64, -5.0e6..5.0e6f64), 0..4),
        prop::collection::vec(arb_key(), 0..6),
    )
        .prop_map(|(tool, name, range, points, keys)| ObjectRecipe {
            tool,
            name,
            range,
            points,
            keys,
        })
}

fn arb_project_recipe() -> impl Strategy<Value = ProjectRecipe> {
    (
        "[A-Za-z ]{1,16}",
        0usize..2,
        0usize..4,
        0usize..4,
        1u32..40,
        prop::collection::vec(
            (
                "[a-z]{1,8}",
                any::<bool>(),
                any::<bool>(),
                prop::collection::vec(arb_object(), 0..4),
            ),
            1..4,
        ),
    )
        .prop_map(
            |(name, field_kind, resolution, step_hours, step_count, layers)| ProjectRecipe {
                name,
                field_kind,
                resolution,
                step_hours,
                step_count,
                layers,
            },
        )
}

fn build(recipe: &ProjectRecipe) -> Project {
    let settings = ProjectSettings::new(
        [FieldKind::Wind, FieldKind::Current][recipe.field_kind % 2],
        Resolution::ALL[recipe.resolution % 4],
        StepHours::ALL[recipe.step_hours % 4],
        recipe.step_count,
    );
    let mut project = Project::new(recipe.name.clone(), settings);
    project.layers.clear();
    let last = project.last_step();

    for (name, visible, locked, objects) in &recipe.layers {
        let mut layer = Layer::new(name.clone());
        layer.visible = *visible;
        layer.locked = *locked;

        for spec in objects {
            let tool = TOOLS[spec.tool % TOOLS.len()];
            let mut object = Object::new(tool, spec.name.clone(), project.settings.step_count);
            object.active_range = StepRange::new(spec.range.0.min(last), spec.range.1.min(last));

            let pts: Vec<LocalPoint> = spec
                .points
                .iter()
                .map(|(x, y)| LocalPoint::new(*x, *y))
                .collect();
            object.geometry = match object.geometry {
                Geometry::Stroke { .. } => Geometry::Stroke { chains: vec![pts] },
                Geometry::Polygon { .. } => Geometry::Polygon { points: pts },
                other => other,
            };

            // Generate edits a user can make; fixed properties (including a
            // macro's strategy) cannot acquire keys in a valid new document.
            let ids: Vec<PropId> = all_specs(tool)
                .filter(|s| ve_core::schema::animatable(tool, s.id))
                .map(|s| s.id)
                .collect();
            for key in &spec.keys {
                let Some(&id) = ids.get(key.prop % ids.len()) else {
                    continue;
                };
                let Some(anim) = object.props.get_mut(id) else {
                    continue;
                };
                let value = match anim.kind() {
                    PropKind::F32 => PropValue::F32(key.magnitude),
                    PropKind::Angle => PropValue::Angle(f64::from(key.magnitude).into()),
                    PropKind::Bool => PropValue::Bool(key.magnitude > 0.0),
                    PropKind::Enum => PropValue::Enum((key.magnitude.abs() as u8) % 3),
                    PropKind::LonLat => continue,
                };
                anim.set_key(
                    key.step.min(last),
                    value,
                    INTERPS[key.interp % INTERPS.len()],
                );
            }
            layer.objects.push(object);
        }
        project.layers.push(layer);
    }
    project
}

// --- Acceptance: persistence -------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// Any project must survive save -> load -> save with identical bytes.
    #[test]
    fn any_project_round_trips_byte_identically(recipe in arb_project_recipe()) {
        let project = build(&recipe);
        prop_assert!(project.validate().is_ok(), "generated an invalid project");

        let first = io::to_canonical_json(&project).unwrap();
        let reloaded = io::from_json(&first).unwrap();
        let second = io::to_canonical_json(&reloaded).unwrap();

        prop_assert_eq!(first, second, "document json drifted across a round trip");
    }

    /// Sampling a property must never produce a value that cannot be saved.
    #[test]
    fn evaluated_values_stay_finite(recipe in arb_project_recipe()) {
        let project = build(&recipe);
        for layer in &project.layers {
            for object in &layer.objects {
                for step in 0..=project.last_step() {
                    for (_, anim) in object.props.iter() {
                        prop_assert!(
                            anim.value_at(step).is_finite(),
                            "non-finite value at step {}", step
                        );
                    }
                }
            }
        }
    }
}

// --- Acceptance: undo/redo ---------------------------------------------------

#[derive(Debug, Clone)]
enum Action {
    AddObject(usize, usize),
    RemoveObject(usize),
    RenameObject(usize, String),
    SetScalar(usize, f32),
    SetKey(usize, u32, f32),
    SetRange(usize, u32, u32),
    MoveObject(usize, usize),
    AddLayer(String),
    RemoveLayer(usize),
    MoveLayer(usize, usize),
    ToggleVisible(usize),
    RenameProject(String),
    SetStepCount(u32),
}

fn arb_action() -> impl Strategy<Value = Action> {
    prop_oneof![
        (0usize..4, 0usize..TOOLS.len()).prop_map(|(l, t)| Action::AddObject(l, t)),
        (0usize..8).prop_map(Action::RemoveObject),
        (0usize..8, "[a-z]{1,8}").prop_map(|(i, n)| Action::RenameObject(i, n)),
        (0usize..8, 0.0f32..80.0).prop_map(|(i, v)| Action::SetScalar(i, v)),
        (0usize..8, 0u32..30, 0.0f32..80.0).prop_map(|(i, s, v)| Action::SetKey(i, s, v)),
        (0usize..8, 0u32..30, 0u32..30).prop_map(|(i, a, b)| Action::SetRange(i, a, b)),
        (0usize..8, 0usize..4).prop_map(|(o, l)| Action::MoveObject(o, l)),
        "[a-z]{1,8}".prop_map(Action::AddLayer),
        (0usize..4).prop_map(Action::RemoveLayer),
        (0usize..4, 0usize..4).prop_map(|(a, b)| Action::MoveLayer(a, b)),
        (0usize..4).prop_map(Action::ToggleVisible),
        "[a-z]{1,8}".prop_map(Action::RenameProject),
        (1u32..40).prop_map(Action::SetStepCount),
    ]
}

/// Turns an abstract action into a command against the current document.
///
/// Returns `None` when the action does not apply -- an index past the end, or a
/// removal that would empty the layer stack. Indices are taken modulo the live
/// collection so generated actions stay useful as the document changes shape.
fn to_command(project: &Project, action: &Action) -> Option<Command> {
    let object_ids: Vec<_> = project
        .layers
        .iter()
        .flat_map(|l| l.objects.iter().map(|o| o.id))
        .collect();

    match action {
        Action::AddObject(layer, tool) => {
            let layer_id = project.layers.get(*layer % project.layers.len())?.id;
            let tool = TOOLS[*tool % TOOLS.len()];
            Some(Command::AddObject {
                layer: layer_id,
                index: 0,
                object: Box::new(Object::new(tool, "generated", project.settings.step_count)),
            })
        }
        Action::RemoveObject(i) => {
            let id = *object_ids.get(*i % object_ids.len().max(1))?;
            let (li, oi) = project.locate(id)?;
            Some(Command::RemoveObject {
                layer: project.layers[li].id,
                index: oi,
                object: Box::new(project.layers[li].objects[oi].clone()),
            })
        }
        Action::RenameObject(i, name) => {
            let id = *object_ids.get(*i % object_ids.len().max(1))?;
            Some(Command::RenameObject {
                object: id,
                before: project.object(id)?.name.clone(),
                after: name.clone(),
            })
        }
        Action::SetScalar(i, value) => {
            let id = *object_ids.get(*i % object_ids.len().max(1))?;
            let before = project.object(id)?.props.get(PropId::Feather)?.clone();
            let mut after = before.clone();
            after.set_base(PropValue::F32(*value));
            Some(Command::SetProperty {
                object: id,
                prop: PropId::Feather,
                before: Box::new(before),
                after: Box::new(after),
            })
        }
        Action::SetKey(i, step, value) => {
            let id = *object_ids.get(*i % object_ids.len().max(1))?;
            let before: Animatable = project.object(id)?.props.get(PropId::Feather)?.clone();
            let mut after = before.clone();
            after.set_key(
                (*step).min(project.last_step()),
                PropValue::F32(*value),
                Interpolation::Linear,
            );
            Some(Command::SetProperty {
                object: id,
                prop: PropId::Feather,
                before: Box::new(before),
                after: Box::new(after),
            })
        }
        Action::SetRange(i, a, b) => {
            let id = *object_ids.get(*i % object_ids.len().max(1))?;
            let last = project.last_step();
            Some(Command::SetActiveRange {
                object: id,
                before: project.object(id)?.active_range,
                after: StepRange::new((*a).min(last), (*b).min(last)),
            })
        }
        Action::MoveObject(o, l) => {
            let id = *object_ids.get(*o % object_ids.len().max(1))?;
            let (li, oi) = project.locate(id)?;
            let dest = project.layers.get(*l % project.layers.len())?.id;
            Some(Command::MoveObject {
                object: id,
                from: (project.layers[li].id, oi),
                to: (dest, 0),
            })
        }
        Action::AddLayer(name) => Some(Command::AddLayer {
            index: project.layers.len(),
            layer: Box::new(Layer::new(name.clone())),
        }),
        Action::RemoveLayer(i) => {
            // Never remove the last layer: a project always has somewhere to
            // put an object, and the UI does not offer it either.
            if project.layers.len() <= 1 {
                return None;
            }
            let index = *i % project.layers.len();
            Some(Command::RemoveLayer {
                index,
                layer: Box::new(project.layers[index].clone()),
            })
        }
        Action::MoveLayer(a, b) => {
            let len = project.layers.len();
            Some(Command::MoveLayer {
                from: *a % len,
                to: *b % len,
            })
        }
        Action::ToggleVisible(i) => {
            let layer = project.layers.get(*i % project.layers.len())?;
            Some(Command::SetLayerVisible {
                layer: layer.id,
                before: layer.visible,
                after: !layer.visible,
            })
        }
        Action::RenameProject(name) => Some(Command::SetProjectName {
            before: project.name.clone(),
            after: name.clone(),
        }),
        Action::SetStepCount(n) => Some(Command::SetStepCount {
            before: project.settings.step_count,
            after: *n,
            restore: Vec::new(),
        }),
    }
}

fn seed_project() -> Project {
    let mut project = Project::new(
        "Seed",
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 30),
    );
    project.layers.push(Layer::new("Second"));
    for i in 0..3 {
        let object = Object::new(TOOLS[i % TOOLS.len()], format!("seed {i}"), 30);
        project.layers[i % 2].objects.push(object);
    }
    project
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// N commands followed by N undos must restore the exact starting document.
    #[test]
    fn undoing_everything_restores_the_document(
        actions in prop::collection::vec(arb_action(), 1..24)
    ) {
        let mut project = seed_project();
        let original = project.clone();
        let mut history = History::default();

        for action in &actions {
            if let Some(command) = to_command(&project, action) {
                // A command may legitimately fail (a stale index after an
                // earlier removal). What matters is that a failure leaves the
                // document untouched, which the next assertion covers.
                let _ = history.push(&mut project, command);
            }
        }

        while history.can_undo() {
            history.undo(&mut project).unwrap();
        }

        prop_assert_eq!(&project, &original, "undoing everything did not restore the document");
    }

    /// Undo then redo must land back where the edits left off.
    #[test]
    fn redoing_everything_returns_to_the_edited_state(
        actions in prop::collection::vec(arb_action(), 1..16)
    ) {
        let mut project = seed_project();
        let mut history = History::default();

        for action in &actions {
            if let Some(command) = to_command(&project, action) {
                let _ = history.push(&mut project, command);
            }
        }
        let edited = project.clone();

        while history.can_undo() {
            history.undo(&mut project).unwrap();
        }
        while history.can_redo() {
            history.redo(&mut project).unwrap();
        }

        prop_assert_eq!(&project, &edited, "redo did not return to the edited state");
    }

    /// Whatever the edits, the document must stay saveable.
    #[test]
    fn the_document_stays_valid_throughout(
        actions in prop::collection::vec(arb_action(), 1..24)
    ) {
        let mut project = seed_project();
        let mut history = History::default();

        for action in &actions {
            if let Some(command) = to_command(&project, action) {
                let _ = history.push(&mut project, command);
                prop_assert!(project.validate().is_ok(), "edit left the project invalid");
            }
        }
    }
}

// --- Acceptance: scale -------------------------------------------------------

/// A 5,000-object project must round-trip, and in a release build must open
/// within the 500 ms budget (spec.md 13).
///
/// The timing assertion is release-only on purpose: a debug build is several
/// times slower and asserting against it would either fail constantly or force
/// a threshold so loose it measures nothing.
#[test]
fn a_five_thousand_object_project_opens_quickly() {
    let mut project = Project::new(
        "Large",
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg025, StepHours::H3, 48),
    );
    for l in 0..5 {
        let mut layer = Layer::new(format!("Layer {l}"));
        for o in 0..1000 {
            let mut object =
                Object::new(TOOLS[(l + o) % TOOLS.len()], format!("object {l}-{o}"), 48);
            if let Some(anim) = object.props.get_mut(PropId::Feather) {
                anim.set_key(0, PropValue::F32(0.1), Interpolation::Linear);
                anim.set_key(24, PropValue::F32(0.9), Interpolation::EaseInOut);
            }
            layer.objects.push(object);
        }
        project.layers.push(layer);
    }
    assert_eq!(project.object_count(), 5000);

    let json = io::to_canonical_json(&project).unwrap();
    let start = std::time::Instant::now();
    let loaded = io::from_json(&json).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(loaded.object_count(), 5000);
    println!(
        "5,000-object open: {:?} ({} of json, debug_assertions={})",
        elapsed,
        json.len(),
        cfg!(debug_assertions)
    );

    #[cfg(not(debug_assertions))]
    assert!(
        elapsed.as_millis() <= 500,
        "open took {elapsed:?}, budget is 500ms"
    );
}
