//! Undo and redo.
//!
//! One shared history per project, so undo order matches the order the user
//! acted (spec.md 8.4). Selection changes are deliberately not recorded: they
//! are not document mutations, and burying real edits under selection noise
//! makes undo unusable.

use crate::command::Command;
use crate::error::Result;
use crate::project::Project;

/// The default number of entries kept.
pub const DEFAULT_LIMIT: usize = 200;

/// One recorded change.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The change itself.
    pub command: Command,
    /// Description for the history panel.
    pub label: String,
    /// Groups consecutive commands into one entry. See [`History::push_coalesced`].
    pub coalesce_key: Option<String>,
}

/// A project's undo stack.
#[derive(Debug)]
pub struct History {
    entries: Vec<Entry>,
    /// Number of entries currently applied; everything at or past this index is
    /// available to redo.
    cursor: usize,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(DEFAULT_LIMIT)
    }
}

impl History {
    /// An empty history keeping at most `limit` entries.
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            limit: limit.max(1),
        }
    }

    /// Applies a command and records it.
    ///
    /// Any redo entries are discarded: the user has taken a new branch.
    pub fn push(&mut self, project: &mut Project, command: Command) -> Result<()> {
        self.push_inner(project, command, None)
    }

    /// Applies a command, merging it into the previous entry when they share a
    /// `key` and the same shape.
    ///
    /// The key is explicit rather than time-based. A drag holds one key for its
    /// duration and drops it on release, so a gesture becomes exactly one
    /// history entry regardless of how long it took or how many events it
    /// produced -- which a time window cannot guarantee.
    pub fn push_coalesced(
        &mut self,
        project: &mut Project,
        command: Command,
        key: impl Into<String>,
    ) -> Result<()> {
        self.push_inner(project, command, Some(key.into()))
    }

    fn push_inner(
        &mut self,
        project: &mut Project,
        mut command: Command,
        key: Option<String>,
    ) -> Result<()> {
        command.apply(project)?;

        // Applying succeeded, so the new state is real. Only now is it safe to
        // discard the redo branch.
        self.entries.truncate(self.cursor);

        if let Some(ref key) = key
            && let Some(last) = self.entries.last_mut()
            && last.coalesce_key.as_ref() == Some(key)
            && last.command.merge(&command)
        {
            return Ok(());
        }

        let label = command.label();
        self.entries.push(Entry {
            command,
            label,
            coalesce_key: key,
        });

        if self.entries.len() > self.limit {
            let excess = self.entries.len() - self.limit;
            self.entries.drain(..excess);
        }
        self.cursor = self.entries.len();
        Ok(())
    }

    /// Reverses the most recent change, returning its label.
    pub fn undo(&mut self, project: &mut Project) -> Result<Option<String>> {
        let Some(index) = self.cursor.checked_sub(1) else {
            return Ok(None);
        };
        let Some(entry) = self.entries.get_mut(index) else {
            return Ok(None);
        };
        entry.command.undo(project)?;
        self.cursor = index;
        Ok(Some(entry.label.clone()))
    }

    /// Reapplies the next undone change, returning its label.
    pub fn redo(&mut self, project: &mut Project) -> Result<Option<String>> {
        let Some(entry) = self.entries.get_mut(self.cursor) else {
            return Ok(None);
        };
        entry.command.apply(project)?;
        self.cursor += 1;
        Ok(Some(entry.label.clone()))
    }

    /// Whether anything can be undone.
    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    /// Whether anything can be redone.
    pub fn can_redo(&self) -> bool {
        self.cursor < self.entries.len()
    }

    /// Every recorded entry, oldest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// How many entries are currently applied.
    ///
    /// The history panel draws entries before this index as done and the rest
    /// as available to redo.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Ends any coalescing group, so the next edit starts a new entry.
    ///
    /// Called on pointer-up, on blur, and whenever a gesture ends.
    pub fn break_coalescing(&mut self) {
        if let Some(last) = self.entries.last_mut() {
            last.coalesce_key = None;
        }
    }

    /// Moves to an arbitrary point in the history, undoing or redoing as needed.
    ///
    /// `target` is a cursor position: 0 is "before everything".
    pub fn jump_to(&mut self, project: &mut Project, target: usize) -> Result<()> {
        let target = target.min(self.entries.len());
        while self.cursor > target {
            if self.undo(project)?.is_none() {
                break;
            }
        }
        while self.cursor < target {
            if self.redo(project)?.is_none() {
                break;
            }
        }
        Ok(())
    }

    /// Discards every entry, keeping the document as it is.
    ///
    /// Used after opening or saving, where there is nothing to undo *into*.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Geometry, Layer, LocalPoint, Object, StepRange};
    use crate::error::CoreError;
    use crate::id::Id;
    use crate::project::{FieldKind, ProjectSettings, Resolution, StepHours};
    use crate::schema::{PropId, ToolKind};
    use crate::value::{Interpolation, PropValue};

    fn project() -> Project {
        Project::new(
            "T",
            ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 24),
        )
    }

    fn with_object() -> (Project, Id) {
        let mut p = project();
        let obj = Object::new(ToolKind::Brush, "b", 24);
        let id = obj.id;
        let layer = p.layers[0].id;
        let mut cmd = Command::AddObject {
            layer,
            index: 0,
            object: Box::new(obj),
        };
        cmd.apply(&mut p).unwrap();
        (p, id)
    }

    fn set_speed(object: Id, before: f32, after: f32) -> Command {
        Command::SetProperty {
            object,
            prop: PropId::Speed,
            before: Box::new(crate::keyframe::Animatable::constant(PropValue::F32(
                before,
            ))),
            after: Box::new(crate::keyframe::Animatable::constant(PropValue::F32(after))),
        }
    }

    fn speed_of(p: &Project, id: Id) -> f32 {
        p.object(id)
            .unwrap()
            .props
            .get(PropId::Speed)
            .unwrap()
            .base()
            .as_f32()
            .unwrap()
    }

    #[test]
    fn push_applies_and_undo_reverses() {
        let (mut p, id) = with_object();
        let mut h = History::default();

        h.push(&mut p, set_speed(id, 10.0, 42.0)).unwrap();
        assert_eq!(speed_of(&p, id), 42.0);
        assert!(h.can_undo() && !h.can_redo());

        assert_eq!(h.undo(&mut p).unwrap().as_deref(), Some("Change Speed"));
        assert_eq!(speed_of(&p, id), 10.0);
        assert!(!h.can_undo() && h.can_redo());

        assert!(h.redo(&mut p).unwrap().is_some());
        assert_eq!(speed_of(&p, id), 42.0);
    }

    #[test]
    fn undo_and_redo_stop_at_the_ends() {
        let mut p = project();
        let mut h = History::default();
        assert!(h.undo(&mut p).unwrap().is_none());
        assert!(h.redo(&mut p).unwrap().is_none());
    }

    #[test]
    fn a_new_edit_discards_the_redo_branch() {
        let (mut p, id) = with_object();
        let mut h = History::default();

        h.push(&mut p, set_speed(id, 10.0, 20.0)).unwrap();
        h.undo(&mut p).unwrap();
        assert!(h.can_redo());

        h.push(&mut p, set_speed(id, 10.0, 30.0)).unwrap();
        assert!(!h.can_redo(), "the old branch must be gone");
        assert_eq!(h.entries().len(), 1);
        assert_eq!(speed_of(&p, id), 30.0);
    }

    /// A drag produces many events but must leave one entry, keeping the
    /// original `before` so a single undo returns to where the drag started.
    #[test]
    fn a_coalesced_gesture_is_one_entry() {
        let (mut p, id) = with_object();
        let mut h = History::default();

        for v in [11.0, 12.0, 13.0, 14.0] {
            h.push_coalesced(&mut p, set_speed(id, 10.0, v), "drag:speed")
                .unwrap();
        }
        assert_eq!(h.entries().len(), 1);
        assert_eq!(speed_of(&p, id), 14.0);

        h.undo(&mut p).unwrap();
        assert_eq!(speed_of(&p, id), 10.0, "one undo must span the whole drag");
    }

    #[test]
    fn ending_a_gesture_starts_a_new_entry() {
        let (mut p, id) = with_object();
        let mut h = History::default();

        h.push_coalesced(&mut p, set_speed(id, 10.0, 11.0), "drag")
            .unwrap();
        h.break_coalescing();
        h.push_coalesced(&mut p, set_speed(id, 11.0, 12.0), "drag")
            .unwrap();

        assert_eq!(h.entries().len(), 2);
        h.undo(&mut p).unwrap();
        assert_eq!(speed_of(&p, id), 11.0);
    }

    #[test]
    fn different_keys_do_not_merge() {
        let (mut p, id) = with_object();
        let mut h = History::default();
        h.push_coalesced(&mut p, set_speed(id, 10.0, 11.0), "a")
            .unwrap();
        h.push_coalesced(&mut p, set_speed(id, 11.0, 12.0), "b")
            .unwrap();
        assert_eq!(h.entries().len(), 2);
    }

    #[test]
    fn the_oldest_entries_are_dropped_at_the_limit() {
        let (mut p, id) = with_object();
        let mut h = History::new(3);
        for v in 1..=6 {
            h.push(&mut p, set_speed(id, v as f32, (v + 1) as f32))
                .unwrap();
        }
        assert_eq!(h.entries().len(), 3);
        assert_eq!(h.cursor(), 3);
    }

    #[test]
    fn jumping_moves_in_both_directions() {
        let (mut p, id) = with_object();
        let mut h = History::default();
        h.push(&mut p, set_speed(id, 10.0, 20.0)).unwrap();
        h.push(&mut p, set_speed(id, 20.0, 30.0)).unwrap();
        h.push(&mut p, set_speed(id, 30.0, 40.0)).unwrap();

        h.jump_to(&mut p, 1).unwrap();
        assert_eq!(speed_of(&p, id), 20.0);
        h.jump_to(&mut p, 3).unwrap();
        assert_eq!(speed_of(&p, id), 40.0);
        h.jump_to(&mut p, 0).unwrap();
        assert_eq!(speed_of(&p, id), 10.0);
    }

    #[test]
    fn layers_add_and_remove_reversibly() {
        let mut p = project();
        let mut h = History::default();
        let layer = Layer::new("New");
        let lid = layer.id;

        h.push(
            &mut p,
            Command::AddLayer {
                index: 1,
                layer: Box::new(layer),
            },
        )
        .unwrap();
        assert_eq!(p.layers.len(), 2);

        let existing = p.layers[1].clone();
        h.push(
            &mut p,
            Command::RemoveLayer {
                index: 1,
                layer: Box::new(existing),
            },
        )
        .unwrap();
        assert_eq!(p.layers.len(), 1);

        h.undo(&mut p).unwrap();
        assert_eq!(p.layers.len(), 2);
        assert_eq!(
            p.layers[1].id, lid,
            "the same layer must come back, in place"
        );

        h.undo(&mut p).unwrap();
        assert_eq!(p.layers.len(), 1);
    }

    #[test]
    fn objects_move_between_layers_reversibly() {
        let (mut p, id) = with_object();
        let mut h = History::default();
        let source = p.layers[0].id;
        let dest = Layer::new("Dest");
        let did = dest.id;
        p.layers.push(dest);

        h.push(
            &mut p,
            Command::MoveObject {
                object: id,
                from: (source, 0),
                to: (did, 0),
            },
        )
        .unwrap();
        assert_eq!(p.locate(id), Some((1, 0)));

        h.undo(&mut p).unwrap();
        assert_eq!(p.locate(id), Some((0, 0)));
    }

    /// A command that fails must leave the document untouched, not half-moved.
    #[test]
    fn a_failed_move_leaves_the_document_intact() {
        let (mut p, id) = with_object();
        let mut h = History::default();
        let source = p.layers[0].id;

        let err = h.push(
            &mut p,
            Command::MoveObject {
                object: id,
                from: (source, 0),
                to: (Id::from_raw(u64::MAX), 0),
            },
        );
        assert!(matches!(err, Err(CoreError::MissingLayer(_))));
        assert_eq!(
            p.locate(id),
            Some((0, 0)),
            "the object must still be where it was"
        );
        assert!(!h.can_undo(), "a failed command must not be recorded");
    }

    #[test]
    fn editing_a_missing_object_is_an_error() {
        let mut p = project();
        let mut h = History::default();
        let err = h.push(&mut p, set_speed(Id::from_raw(u64::MAX), 1.0, 2.0));
        assert!(matches!(err, Err(CoreError::MissingObject(_))));
    }

    #[test]
    fn geometry_edits_reverse() {
        let (mut p, id) = with_object();
        let mut h = History::default();
        let before = p.object(id).unwrap().geometry.clone();
        let after = Geometry::Stroke {
            chains: vec![vec![LocalPoint::new(1.0, 2.0)]],
        };

        h.push(
            &mut p,
            Command::SetGeometry {
                object: id,
                before: Box::new(before.clone()),
                after: Box::new(after.clone()),
            },
        )
        .unwrap();
        assert_eq!(p.object(id).unwrap().geometry, after);

        h.undo(&mut p).unwrap();
        assert_eq!(p.object(id).unwrap().geometry, before);
    }

    /// Shrinking deletes keyframes; undo must bring back every one, plus the
    /// active ranges that were clamped alongside them (decision D13).
    #[test]
    fn shrinking_the_project_is_fully_reversible() {
        let (mut p, id) = with_object();
        let mut h = History::default();
        {
            let obj = p.object_mut(id).unwrap();
            let speed = obj.props.get_mut(PropId::Speed).unwrap();
            speed.set_key(0, PropValue::F32(5.0), Interpolation::Linear);
            speed.set_key(12, PropValue::F32(25.0), Interpolation::Linear);
            speed.set_key(20, PropValue::F32(5.0), Interpolation::Linear);
            obj.active_range = StepRange::new(0, 23);
        }

        let (count, names) = p.keyframes_after(9);
        assert_eq!((count, names.len()), (2, 1));

        h.push(
            &mut p,
            Command::SetStepCount {
                before: 24,
                after: 10,
                restore: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(p.settings.step_count, 10);
        assert_eq!(
            p.object(id)
                .unwrap()
                .props
                .get(PropId::Speed)
                .unwrap()
                .keys()
                .len(),
            1
        );
        assert_eq!(p.object(id).unwrap().active_range.end, 9);

        h.undo(&mut p).unwrap();
        assert_eq!(p.settings.step_count, 24);
        assert_eq!(
            p.object(id)
                .unwrap()
                .props
                .get(PropId::Speed)
                .unwrap()
                .keys()
                .len(),
            3
        );
        assert_eq!(p.object(id).unwrap().active_range.end, 23);
    }

    /// Redo must reuse the snapshot the first apply captured. Re-deriving it
    /// from an already-truncated document would capture nothing and silently
    /// make the change unrecoverable.
    #[test]
    fn shrinking_survives_undo_then_redo_then_undo() {
        let (mut p, id) = with_object();
        let mut h = History::default();
        {
            let speed = p
                .object_mut(id)
                .unwrap()
                .props
                .get_mut(PropId::Speed)
                .unwrap();
            speed.set_key(0, PropValue::F32(5.0), Interpolation::Linear);
            speed.set_key(20, PropValue::F32(9.0), Interpolation::Linear);
        }

        h.push(
            &mut p,
            Command::SetStepCount {
                before: 24,
                after: 10,
                restore: Vec::new(),
            },
        )
        .unwrap();
        h.undo(&mut p).unwrap();
        h.redo(&mut p).unwrap();
        assert_eq!(
            p.object(id)
                .unwrap()
                .props
                .get(PropId::Speed)
                .unwrap()
                .keys()
                .len(),
            1
        );

        h.undo(&mut p).unwrap();
        assert_eq!(
            p.object(id)
                .unwrap()
                .props
                .get(PropId::Speed)
                .unwrap()
                .keys()
                .len(),
            2,
            "the second undo must still restore both keys"
        );
    }

    #[test]
    fn clearing_keeps_the_document_but_drops_the_stack() {
        let (mut p, id) = with_object();
        let mut h = History::default();
        h.push(&mut p, set_speed(id, 10.0, 42.0)).unwrap();
        h.clear();
        assert!(!h.can_undo() && !h.can_redo());
        assert_eq!(speed_of(&p, id), 42.0);
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::document::Object;
    use crate::project::{FieldKind, ProjectSettings, Resolution, StepHours};
    use crate::schema::ToolKind;

    /// A paste is one action to the user, so it must be one undo.
    #[test]
    fn a_batch_insert_is_a_single_history_entry() {
        let mut project = Project::new(
            "T",
            ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 12),
        );
        let layer = project.layers[0].id;
        let mut history = History::default();

        let objects = vec![
            Object::new(ToolKind::Brush, "a", 12),
            Object::new(ToolKind::Brush, "b", 12),
            Object::new(ToolKind::Brush, "c", 12),
        ];
        history
            .push(
                &mut project,
                Command::AddObjects {
                    layer,
                    index: 0,
                    objects,
                },
            )
            .expect("paste");

        assert_eq!(project.object_count(), 3);
        assert_eq!(history.entries().len(), 1, "one action, one entry");
        assert_eq!(history.entries()[0].label, "Paste 3 objects");

        history.undo(&mut project).expect("undo");
        assert_eq!(project.object_count(), 0, "one undo removes all three");

        history.redo(&mut project).expect("redo");
        assert_eq!(project.object_count(), 3);
    }

    #[test]
    fn a_batch_insert_lands_in_order_at_the_index() {
        let mut project = Project::new(
            "T",
            ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 12),
        );
        let layer = project.layers[0].id;
        project.layers[0]
            .objects
            .push(Object::new(ToolKind::Brush, "existing", 12));

        let mut history = History::default();
        history
            .push(
                &mut project,
                Command::AddObjects {
                    layer,
                    index: 0,
                    objects: vec![
                        Object::new(ToolKind::Brush, "first", 12),
                        Object::new(ToolKind::Brush, "second", 12),
                    ],
                },
            )
            .expect("paste");

        let names: Vec<&str> = project.layers[0]
            .objects
            .iter()
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(names, vec!["first", "second", "existing"]);
    }
}
