//! Document mutations, each one reversible.
//!
//! Every change to a project goes through a [`Command`]. Commands carry both
//! the new value and the value they displaced, so undo is exact rather than
//! recomputed.
//!
//! The alternative -- snapshotting the whole document per edit -- was rejected:
//! a 5,000-object project is far too large to clone on every brush stroke and
//! still meet the 16 ms undo budget (spec.md 13).
//!
//! Keyframe edits do not get their own variants. Adding, moving, deleting, or
//! re-typing a key is a [`Command::SetProperty`] carrying the whole
//! [`Animatable`] before and after. One reversible operation covers every
//! keyframe operation, including ones not invented yet.

use serde::{Deserialize, Serialize};

use crate::document::{Geometry, Layer, Object, StepRange};
use crate::error::{CoreError, Result};
use crate::keyframe::Animatable;
use crate::project::Project;
use crate::schema::{PropId, PropertyMap};

/// A captured slice of an object's state, for restoring after a destructive
/// project-wide change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectSnapshot {
    /// Which object.
    pub object: crate::id::Id,
    /// Its properties before the change.
    pub props: Box<PropertyMap>,
    /// Its active range before the change.
    pub active_range: StepRange,
}

/// A reversible change to a project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Inserts a layer at `index`.
    AddLayer {
        /// Position in the stack.
        index: usize,
        /// The layer being added.
        layer: Box<Layer>,
    },
    /// Removes the layer at `index`, keeping it for undo.
    RemoveLayer {
        /// Position in the stack.
        index: usize,
        /// The removed layer, restored on undo.
        layer: Box<Layer>,
    },
    /// Renames a layer.
    RenameLayer {
        /// Target layer.
        layer: crate::id::Id,
        /// Previous name.
        before: String,
        /// New name.
        after: String,
    },
    /// Shows or hides a layer.
    SetLayerVisible {
        /// Target layer.
        layer: crate::id::Id,
        /// Previous state.
        before: bool,
        /// New state.
        after: bool,
    },
    /// Sets which speeds an imported field keeps (spec.md 4.8).
    SetLayerSpeedRange {
        /// Target layer.
        layer: crate::id::Id,
        /// Previous band.
        before: Option<crate::document::SpeedRange>,
        /// New band; `None` keeps every speed.
        after: Option<crate::document::SpeedRange>,
    },
    /// Replaces which of an imported file's messages a layer's steps show
    /// (spec.md 4.8, M20).
    ///
    /// The whole list rather than one entry, because a paste is one action to
    /// the user: pasting a run of four frames is one command with one inverse,
    /// and one undo returns all four.
    SetFrameOverrides {
        /// Target layer.
        layer: crate::id::Id,
        /// Previous overrides.
        before: Vec<crate::document::FrameOverride>,
        /// New overrides.
        after: Vec<crate::document::FrameOverride>,
    },
    /// Locks or unlocks a layer.
    SetLayerLocked {
        /// Target layer.
        layer: crate::id::Id,
        /// Previous state.
        before: bool,
        /// New state.
        after: bool,
    },
    /// Reorders the layer stack.
    MoveLayer {
        /// Index before the move.
        from: usize,
        /// Index after the move.
        to: usize,
    },

    /// Inserts an object into a layer.
    AddObject {
        /// Target layer.
        layer: crate::id::Id,
        /// Position within the layer.
        index: usize,
        /// The object being added.
        object: Box<Object>,
    },
    /// Inserts several objects at once.
    ///
    /// A paste is one action to the user, so it is one history entry. Pasting
    /// as N separate `AddObject` commands would need N undos to reverse.
    AddObjects {
        /// Target layer.
        layer: crate::id::Id,
        /// Position of the first object within the layer.
        index: usize,
        /// The objects being added, in order.
        objects: Vec<Object>,
    },
    /// Removes an object, keeping it for undo.
    RemoveObject {
        /// Source layer.
        layer: crate::id::Id,
        /// Position within the layer.
        index: usize,
        /// The removed object, restored on undo.
        object: Box<Object>,
    },
    /// Renames an object.
    RenameObject {
        /// Target object.
        object: crate::id::Id,
        /// Previous name.
        before: String,
        /// New name.
        after: String,
    },
    /// Moves an object within a layer or between layers.
    MoveObject {
        /// Target object.
        object: crate::id::Id,
        /// Layer and index before the move.
        from: (crate::id::Id, usize),
        /// Layer and index after the move.
        to: (crate::id::Id, usize),
    },
    /// Turns one of an object's own movements into the field it paints
    /// (spec.md 9.3, M13).
    SetMotion {
        /// Target object.
        object: crate::id::Id,
        /// Previous flags.
        before: crate::document::MotionFlags,
        /// New flags.
        after: crate::document::MotionFlags,
    },
    /// Changes an object's coarse lifetime.
    SetActiveRange {
        /// Target object.
        object: crate::id::Id,
        /// Previous range.
        before: StepRange,
        /// New range.
        after: StepRange,
    },
    /// Replaces an object's shape.
    SetGeometry {
        /// Target object.
        object: crate::id::Id,
        /// Previous geometry.
        before: Box<Geometry>,
        /// New geometry.
        after: Box<Geometry>,
    },
    /// Replaces one property, including all of its keyframes.
    SetProperty {
        /// Target object.
        object: crate::id::Id,
        /// Which property.
        prop: PropId,
        /// Previous value and keyframes.
        before: Box<Animatable>,
        /// New value and keyframes.
        after: Box<Animatable>,
    },

    /// Renames the project.
    SetProjectName {
        /// Previous name.
        before: String,
        /// New name.
        after: String,
    },
    /// Sets when step 0 is (spec.md 9.1).
    SetStartTime {
        /// Previous start, seconds since the epoch.
        before: Option<i64>,
        /// New start.
        after: Option<i64>,
    },
    /// Changes the number of time steps.
    ///
    /// Shrinking deletes keyframes past the new end (decision D13). Everything
    /// deleted is captured in `restore` so undo puts it back exactly.
    SetStepCount {
        /// Previous step count.
        before: u32,
        /// New step count.
        after: u32,
        /// State captured at apply time, for undo.
        restore: Vec<ObjectSnapshot>,
    },

    /// Several commands as one reversible step.
    ///
    /// Exists for edits that touch many objects at once and must undo as a
    /// unit: transforming a multi-selection writes a position, a rotation and a
    /// scale per object, and a user who drags six objects and presses undo
    /// expects one press to put all six back.
    ///
    /// Applying is all-or-nothing: a failure part-way undoes what already
    /// succeeded, so a project is never left half-transformed.
    Batch {
        /// What the history panel shows.
        label: String,
        /// Applied in order, undone in reverse.
        commands: Vec<Command>,
    },
}

impl Command {
    /// A short description for the history panel.
    pub fn label(&self) -> String {
        match self {
            Self::Batch { label, .. } => label.clone(),
            Self::AddLayer { .. } => "Add layer".into(),
            Self::RemoveLayer { .. } => "Delete layer".into(),
            Self::RenameLayer { .. } => "Rename layer".into(),
            Self::SetLayerSpeedRange { after, .. } => {
                if after.is_some() {
                    "Filter layer speeds".into()
                } else {
                    "Clear the speed filter".into()
                }
            }
            Self::SetLayerVisible { after, .. } => {
                if *after {
                    "Show layer".into()
                } else {
                    "Hide layer".into()
                }
            }
            Self::SetLayerLocked { after, .. } => {
                if *after {
                    "Lock layer".into()
                } else {
                    "Unlock layer".into()
                }
            }
            Self::SetFrameOverrides { before, after, .. } => {
                let added = after.len().saturating_sub(before.len());
                match (added, after.len() < before.len()) {
                    (0, true) => "Restore the file's frames".into(),
                    (0, false) => "Change pasted frames".into(),
                    (1, _) => "Paste 1 frame".into(),
                    (n, _) => format!("Paste {n} frames"),
                }
            }
            Self::MoveLayer { .. } => "Reorder layers".into(),
            Self::AddObject { object, .. } => format!("Add {}", object.name),
            Self::AddObjects { objects, .. } => match objects.len() {
                1 => "Paste 1 object".into(),
                n => format!("Paste {n} objects"),
            },
            Self::RemoveObject { object, .. } => format!("Delete {}", object.name),
            Self::RenameObject { .. } => "Rename object".into(),
            Self::MoveObject { .. } => "Move object".into(),
            Self::SetMotion { after, before, .. } => {
                if after.any() && !before.any() {
                    "Add motion to the field".into()
                } else if !after.any() {
                    "Take motion out of the field".into()
                } else {
                    "Change which motion reaches the field".into()
                }
            }
            Self::SetActiveRange { .. } => "Change active range".into(),
            Self::SetGeometry { .. } => "Edit shape".into(),
            Self::SetProperty { prop, .. } => format!("Change {prop:?}"),
            Self::SetProjectName { .. } => "Rename project".into(),
            Self::SetStartTime { .. } => "Set start time".into(),
            Self::SetStepCount { .. } => "Change duration".into(),
        }
    }

    /// Applies the change.
    pub fn apply(&mut self, project: &mut Project) -> Result<()> {
        match self {
            Self::Batch { commands, .. } => {
                for (done, command) in commands.iter_mut().enumerate() {
                    if let Err(err) = command.apply(project) {
                        // Roll back what did apply, so a failure leaves the
                        // project where it started rather than part-way.
                        for earlier in commands[..done].iter_mut().rev() {
                            let _ = earlier.undo(project);
                        }
                        return Err(err);
                    }
                }
                Ok(())
            }
            Self::AddLayer { index, layer } => {
                let index = (*index).min(project.layers.len());
                project.layers.insert(index, (**layer).clone());
                Ok(())
            }
            Self::RemoveLayer { index, layer } => {
                let i = index_of_layer(project, layer.id)?;
                *index = i;
                **layer = project.layers.remove(i);
                Ok(())
            }
            Self::RenameLayer { layer, after, .. } => {
                layer_mut(project, *layer)?.name = after.clone();
                Ok(())
            }
            Self::SetLayerVisible { layer, after, .. } => {
                layer_mut(project, *layer)?.visible = *after;
                Ok(())
            }
            Self::SetLayerSpeedRange { layer, after, .. } => {
                layer_mut(project, *layer)?.speed_range = *after;
                Ok(())
            }
            Self::SetLayerLocked { layer, after, .. } => {
                layer_mut(project, *layer)?.locked = *after;
                Ok(())
            }
            Self::SetFrameOverrides { layer, after, .. } => {
                layer_mut(project, *layer)?.set_frame_overrides(after.clone());
                Ok(())
            }
            Self::MoveLayer { from, to } => move_layer(project, *from, *to),

            Self::AddObject {
                layer,
                index,
                object,
            } => {
                let l = layer_mut(project, *layer)?;
                let index = (*index).min(l.objects.len());
                l.objects.insert(index, (**object).clone());
                Ok(())
            }
            Self::AddObjects {
                layer,
                index,
                objects,
            } => {
                let l = layer_mut(project, *layer)?;
                let at = (*index).min(l.objects.len());
                for (offset, object) in objects.iter().enumerate() {
                    l.objects.insert(at + offset, object.clone());
                }
                Ok(())
            }
            Self::RemoveObject {
                layer,
                index,
                object,
            } => {
                let (li, oi) = project
                    .locate(object.id)
                    .ok_or(CoreError::MissingObject(object.id.raw()))?;
                *layer = project.layers[li].id;
                *index = oi;
                **object = project.layers[li].objects.remove(oi);
                Ok(())
            }
            Self::RenameObject { object, after, .. } => {
                object_mut(project, *object)?.name = after.clone();
                Ok(())
            }
            Self::MoveObject { object, to, .. } => move_object(project, *object, to.0, to.1),
            Self::SetActiveRange { object, after, .. } => {
                object_mut(project, *object)?.active_range = *after;
                Ok(())
            }
            Self::SetMotion { object, after, .. } => {
                object_mut(project, *object)?.motion = *after;
                Ok(())
            }
            Self::SetGeometry { object, after, .. } => {
                object_mut(project, *object)?.geometry = (**after).clone();
                Ok(())
            }
            Self::SetProperty {
                object,
                prop,
                after,
                ..
            } => {
                object_mut(project, *object)?
                    .props
                    .insert(*prop, (**after).clone());
                Ok(())
            }

            Self::SetProjectName { after, .. } => {
                project.name = after.clone();
                Ok(())
            }
            Self::SetStartTime { after, .. } => {
                project.settings.start_unix_s = *after;
                Ok(())
            }
            Self::SetStepCount { after, restore, .. } => {
                let last = after.saturating_sub(1);
                // Capture on first apply only; a redo must restore the same
                // snapshot the original apply took, not re-derive it from an
                // already-truncated document.
                if restore.is_empty() {
                    *restore = project
                        .layers
                        .iter()
                        .flat_map(|l| l.objects.iter())
                        .filter(|o| o.props.count_after(last) > 0 || o.active_range.end > last)
                        .map(|o| ObjectSnapshot {
                            object: o.id,
                            props: Box::new(o.props.clone()),
                            active_range: o.active_range,
                        })
                        .collect();
                }
                project.settings.step_count = (*after).clamp(1, crate::project::MAX_STEPS);
                project.truncate_to(project.settings.last_step());
                if project.view.current_step > project.settings.last_step() {
                    project.view.current_step = project.settings.last_step();
                }
                Ok(())
            }
        }
    }

    /// Reverses the change.
    pub fn undo(&mut self, project: &mut Project) -> Result<()> {
        match self {
            Self::Batch { commands, .. } => {
                for command in commands.iter_mut().rev() {
                    command.undo(project)?;
                }
                Ok(())
            }
            Self::AddLayer { layer, .. } => {
                let i = index_of_layer(project, layer.id)?;
                project.layers.remove(i);
                Ok(())
            }
            Self::RemoveLayer { index, layer } => {
                let index = (*index).min(project.layers.len());
                project.layers.insert(index, (**layer).clone());
                Ok(())
            }
            Self::RenameLayer { layer, before, .. } => {
                layer_mut(project, *layer)?.name = before.clone();
                Ok(())
            }
            Self::SetLayerVisible { layer, before, .. } => {
                layer_mut(project, *layer)?.visible = *before;
                Ok(())
            }
            Self::SetLayerSpeedRange { layer, before, .. } => {
                layer_mut(project, *layer)?.speed_range = *before;
                Ok(())
            }
            Self::SetLayerLocked { layer, before, .. } => {
                layer_mut(project, *layer)?.locked = *before;
                Ok(())
            }
            Self::SetFrameOverrides { layer, before, .. } => {
                layer_mut(project, *layer)?.set_frame_overrides(before.clone());
                Ok(())
            }
            Self::MoveLayer { from, to } => move_layer(project, *to, *from),

            Self::AddObjects { objects, .. } => {
                for object in objects.iter() {
                    if let Some((li, oi)) = project.locate(object.id) {
                        project.layers[li].objects.remove(oi);
                    }
                }
                Ok(())
            }
            Self::AddObject { object, .. } => {
                let (li, oi) = project
                    .locate(object.id)
                    .ok_or(CoreError::MissingObject(object.id.raw()))?;
                project.layers[li].objects.remove(oi);
                Ok(())
            }
            Self::RemoveObject {
                layer,
                index,
                object,
            } => {
                let l = layer_mut(project, *layer)?;
                let index = (*index).min(l.objects.len());
                l.objects.insert(index, (**object).clone());
                Ok(())
            }
            Self::RenameObject { object, before, .. } => {
                object_mut(project, *object)?.name = before.clone();
                Ok(())
            }
            Self::MoveObject { object, from, .. } => move_object(project, *object, from.0, from.1),
            Self::SetActiveRange { object, before, .. } => {
                object_mut(project, *object)?.active_range = *before;
                Ok(())
            }
            Self::SetMotion { object, before, .. } => {
                object_mut(project, *object)?.motion = *before;
                Ok(())
            }
            Self::SetGeometry { object, before, .. } => {
                object_mut(project, *object)?.geometry = (**before).clone();
                Ok(())
            }
            Self::SetProperty {
                object,
                prop,
                before,
                ..
            } => {
                object_mut(project, *object)?
                    .props
                    .insert(*prop, (**before).clone());
                Ok(())
            }

            Self::SetProjectName { before, .. } => {
                project.name = before.clone();
                Ok(())
            }
            Self::SetStartTime { before, .. } => {
                project.settings.start_unix_s = *before;
                Ok(())
            }
            Self::SetStepCount {
                before, restore, ..
            } => {
                project.settings.step_count = *before;
                for snap in restore.iter() {
                    if let Some(obj) = project.object_mut(snap.object) {
                        obj.props = (*snap.props).clone();
                        obj.active_range = snap.active_range;
                    }
                }
                Ok(())
            }
        }
    }

    /// Merges a later command of the same shape and target into this one.
    ///
    /// This is what collapses a drag or a slider scrub into a single history
    /// entry: keep the original `before`, adopt the newest `after`.
    pub fn merge(&mut self, next: &Self) -> bool {
        // Batches merge element-wise, which is what makes a multi-selection drag
        // one history entry. Tried on a copy first: a batch that merged only
        // some of its commands would leave the entry describing a transform
        // nobody performed.
        if let (Self::Batch { commands, .. }, Self::Batch { commands: next, .. }) = (&*self, next) {
            if commands.len() != next.len() {
                return false;
            }
            let mut merged = commands.clone();
            for (target, source) in merged.iter_mut().zip(next) {
                if !target.merge(source) {
                    return false;
                }
            }
            if let Self::Batch { commands, .. } = self {
                *commands = merged;
            }
            return true;
        }

        match (self, next) {
            (
                Self::SetProperty {
                    object: a,
                    prop: p,
                    after,
                    ..
                },
                Self::SetProperty {
                    object: b,
                    prop: q,
                    after: next_after,
                    ..
                },
            ) if a == b && p == q => {
                *after = next_after.clone();
                true
            }
            (
                Self::SetActiveRange {
                    object: a, after, ..
                },
                Self::SetActiveRange {
                    object: b,
                    after: next_after,
                    ..
                },
            ) if a == b => {
                *after = *next_after;
                true
            }
            (
                Self::SetGeometry {
                    object: a, after, ..
                },
                Self::SetGeometry {
                    object: b,
                    after: next_after,
                    ..
                },
            ) if a == b => {
                *after = next_after.clone();
                true
            }
            (
                Self::RenameObject {
                    object: a, after, ..
                },
                Self::RenameObject {
                    object: b,
                    after: next_after,
                    ..
                },
            ) if a == b => {
                after.clone_from(next_after);
                true
            }
            (
                Self::RenameLayer {
                    layer: a, after, ..
                },
                Self::RenameLayer {
                    layer: b,
                    after: next_after,
                    ..
                },
            ) if a == b => {
                after.clone_from(next_after);
                true
            }
            (Self::SetProjectName { after, .. }, Self::SetProjectName { after: n, .. }) => {
                after.clone_from(n);
                true
            }
            (Self::SetStartTime { after, .. }, Self::SetStartTime { after: n, .. }) => {
                *after = *n;
                true
            }
            _ => false,
        }
    }
}

fn layer_mut(project: &mut Project, id: crate::id::Id) -> Result<&mut Layer> {
    project
        .layer_mut(id)
        .ok_or(CoreError::MissingLayer(id.raw()))
}

fn object_mut(project: &mut Project, id: crate::id::Id) -> Result<&mut Object> {
    project
        .object_mut(id)
        .ok_or(CoreError::MissingObject(id.raw()))
}

fn index_of_layer(project: &Project, id: crate::id::Id) -> Result<usize> {
    project
        .layer_index(id)
        .ok_or(CoreError::MissingLayer(id.raw()))
}

fn move_layer(project: &mut Project, from: usize, to: usize) -> Result<()> {
    let len = project.layers.len();
    if from >= len || to >= len {
        return Err(CoreError::IndexOutOfBounds {
            index: from.max(to),
            len,
        });
    }
    let layer = project.layers.remove(from);
    project.layers.insert(to, layer);
    Ok(())
}

fn move_object(
    project: &mut Project,
    object: crate::id::Id,
    to_layer: crate::id::Id,
    to_index: usize,
) -> Result<()> {
    let (li, oi) = project
        .locate(object)
        .ok_or(CoreError::MissingObject(object.raw()))?;
    let obj = project.layers[li].objects.remove(oi);
    let dest = match project.layer_mut(to_layer) {
        Some(l) => l,
        None => {
            // Put it back rather than losing it: a failed move must leave the
            // document exactly as it was.
            project.layers[li].objects.insert(oi, obj);
            return Err(CoreError::MissingLayer(to_layer.raw()));
        }
    };
    let index = to_index.min(dest.objects.len());
    dest.objects.insert(index, obj);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{FieldKind, ProjectSettings, Resolution, StepHours};
    use crate::schema::ToolKind;
    use crate::value::PropValue;

    fn project_with_two() -> (Project, crate::id::Id, crate::id::Id) {
        let mut project = Project::new(
            "T",
            ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 12),
        );
        let a = Object::new(ToolKind::Brush, "a", 12);
        let b = Object::new(ToolKind::Brush, "b", 12);
        let (ida, idb) = (a.id, b.id);
        project.layers[0].objects.push(a);
        project.layers[0].objects.push(b);
        (project, ida, idb)
    }

    fn set_speed(object: crate::id::Id, before: f32, after: f32) -> Command {
        Command::SetProperty {
            object,
            prop: PropId::Speed,
            before: Box::new(Animatable::constant(PropValue::F32(before))),
            after: Box::new(Animatable::constant(PropValue::F32(after))),
        }
    }

    fn speed_of(project: &Project, object: crate::id::Id) -> f32 {
        match project
            .object(object)
            .expect("object")
            .props
            .get(PropId::Speed)
            .expect("speed")
            .base()
        {
            PropValue::F32(v) => v,
            other => panic!("speed is {other:?}"),
        }
    }

    #[test]
    fn a_batch_applies_and_undoes_every_member() {
        let (mut project, a, b) = project_with_two();
        let mut batch = Command::Batch {
            label: "Transform".to_owned(),
            commands: vec![set_speed(a, 10.0, 20.0), set_speed(b, 10.0, 30.0)],
        };

        batch.apply(&mut project).expect("apply");
        assert_eq!((speed_of(&project, a), speed_of(&project, b)), (20.0, 30.0));

        batch.undo(&mut project).expect("undo");
        assert_eq!(
            (speed_of(&project, a), speed_of(&project, b)),
            (10.0, 10.0),
            "one undo must put every member back"
        );
    }

    /// Turning a movement into the field is undoable like anything else, and
    /// undo restores the exact flags rather than clearing them (spec.md 9.3).
    #[test]
    fn setting_motion_inverts_exactly() {
        use crate::document::MotionFlags;

        let (mut project, a, _) = project_with_two();
        let before = MotionFlags {
            rotation: true,
            ..Default::default()
        };
        project.object_mut(a).expect("object").motion = before;
        let after = MotionFlags {
            position: true,
            rotation: true,
            scale: false,
        };
        let mut command = Command::SetMotion {
            object: a,
            before,
            after,
        };
        command.apply(&mut project).expect("apply");
        assert_eq!(project.object(a).expect("object").motion, after);
        command.undo(&mut project).expect("undo");
        assert_eq!(
            project.object(a).expect("object").motion,
            before,
            "undo must restore the flags that were there, not clear them"
        );
    }

    /// A half-applied batch would leave a multi-selection transform with some
    /// objects moved and some not, and no single step that fixes it.
    #[test]
    fn a_failing_member_rolls_the_batch_back() {
        let (mut project, a, _) = project_with_two();
        let missing = crate::id::Id::from_raw(9_999);
        let mut batch = Command::Batch {
            label: "Transform".to_owned(),
            commands: vec![set_speed(a, 10.0, 20.0), set_speed(missing, 10.0, 30.0)],
        };

        assert!(batch.apply(&mut project).is_err());
        assert_eq!(
            speed_of(&project, a),
            10.0,
            "the member that succeeded must have been rolled back"
        );
    }

    #[test]
    fn batches_of_the_same_shape_merge() {
        let (_, a, b) = project_with_two();
        let mut first = Command::Batch {
            label: "Transform".to_owned(),
            commands: vec![set_speed(a, 10.0, 11.0), set_speed(b, 10.0, 11.0)],
        };
        let second = Command::Batch {
            label: "Transform".to_owned(),
            commands: vec![set_speed(a, 10.0, 12.0), set_speed(b, 10.0, 12.0)],
        };

        assert!(first.merge(&second));
        let Command::Batch { commands, .. } = &first else {
            panic!("still a batch");
        };
        for command in commands {
            let Command::SetProperty { after, before, .. } = command else {
                panic!("still a property write");
            };
            assert_eq!(after.base(), PropValue::F32(12.0), "the newest value wins");
            assert_eq!(before.base(), PropValue::F32(10.0), "the original is kept");
        }
    }

    #[test]
    fn batches_of_different_shapes_do_not_merge() {
        let (_, a, b) = project_with_two();
        let mut first = Command::Batch {
            label: "Transform".to_owned(),
            commands: vec![set_speed(a, 10.0, 11.0)],
        };
        let second = Command::Batch {
            label: "Transform".to_owned(),
            commands: vec![set_speed(a, 10.0, 12.0), set_speed(b, 10.0, 12.0)],
        };
        assert!(
            !first.merge(&second),
            "a different length is a different gesture"
        );

        // Members addressing different objects must not merge either, or the
        // entry silently retargets.
        let mut first = Command::Batch {
            label: "Transform".to_owned(),
            commands: vec![set_speed(a, 10.0, 11.0)],
        };
        let second = Command::Batch {
            label: "Transform".to_owned(),
            commands: vec![set_speed(b, 10.0, 12.0)],
        };
        assert!(!first.merge(&second));
    }
}
