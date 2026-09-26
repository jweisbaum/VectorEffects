//! The open project and the recent-files list.
//!
//! One project is open at a time. The document itself lives in `ve-core`; this
//! module owns only what a *session* adds: where it came from, whether it has
//! unsaved changes, and its undo stack.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ve_core::history::History;
use ve_core::io;
use ve_core::project::Project;

use crate::error::{AppError, Context, Result};

/// How many recent projects to remember.
pub const MAX_RECENT: usize = 10;

/// The highest revision handed out so far in this process.
static LAST_REVISION: AtomicU64 = AtomicU64::new(0);

/// A revision no project has used before, in this run or an earlier one.
///
/// Revisions address tiles, and tiles are served `immutable` with a year of
/// cache lifetime — a URL names one revision of one step and so never changes
/// meaning. That contract breaks the moment two *different* documents both
/// start counting at 1: the webview then answers the new project's tile
/// requests out of the old project's cache entries, and the previous project's
/// strokes appear on the new map.
///
/// Seeding from the wall clock rather than a counter is what makes this hold
/// across a restart, when the process counter would begin again but the
/// webview's cache would not. Nothing depends on a revision being reproducible;
/// it is a cache key, and never reaches a project file or an export.
pub fn fresh_revision() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    // Strictly increasing even for two projects opened in the same millisecond.
    LAST_REVISION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            Some(last.max(now).saturating_add(1))
        })
        .map_or(now, |last| last.max(now).saturating_add(1))
}

/// A project open for editing.
#[derive(Debug)]
pub struct OpenProject {
    /// The document.
    pub project: Project,
    /// Undo stack for this document. Commands attach to it from M5.
    pub history: History,
    /// Where it was loaded from or last saved to; `None` until first saved.
    pub path: Option<PathBuf>,
    /// Whether there are changes not yet written to disk.
    pub dirty: bool,
    /// Bumped on every document change, and unique to this opening.
    ///
    /// Tile URLs carry it, so an edit makes every previously fetched tile
    /// unreachable rather than stale. Coarser than the per-frame content hash
    /// the render cache will use, but never wrong: this invalidates the whole
    /// document where the hash will invalidate only affected frames.
    ///
    /// It starts somewhere no other project has been rather than at 1, so one
    /// document's tiles can never be served for another's. See
    /// [`fresh_revision`].
    pub revision: u64,
    /// What an image layer's picture is addressed by (spec.md 4.9, M37).
    ///
    /// Unique to this opening, like [`Self::revision`], and — unlike it —
    /// **fixed for the life of the opening**. The pixels an image layer
    /// serves depend on its file and on nothing the document does, so
    /// addressing them by the document's revision made every edit a new
    /// address: the webview refetched every picture, the backend decoded it
    /// again, and dragging one meant a full decode per pointer report with
    /// nothing on screen in between. A layer id is fresh per import, so this
    /// and the layer say which picture is meant; the opening is what keeps
    /// two projects, or one reopened after its file changed on disk, from
    /// sharing an immutable address.
    pub image_token: u64,
}

impl OpenProject {
    /// Wraps a freshly created, never-saved project.
    pub fn created(project: Project) -> Self {
        // A new project is dirty from the start: it exists only in memory, so
        // closing without saving really would lose it.
        Self {
            project,
            history: History::default(),
            path: None,
            dirty: true,
            revision: fresh_revision(),
            image_token: fresh_revision(),
        }
    }

    /// Wraps a project loaded from disk.
    pub fn loaded(project: Project, path: PathBuf) -> Self {
        Self {
            project,
            history: History::default(),
            path: Some(path),
            dirty: false,
            revision: fresh_revision(),
            image_token: fresh_revision(),
        }
    }

    /// Records a document change.
    ///
    /// Bumping the revision is what makes previously fetched tiles unreachable
    /// rather than stale, so the map cannot show a field that no longer exists.
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
    }
}

/// Persisted application settings.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Most recently opened projects, newest first.
    pub recent_projects: Vec<PathBuf>,
    /// Shortcuts, display defaults and the macro library (spec.md 8.6, M15).
    ///
    /// The file **grows** rather than being replaced: this field is absent
    /// from one written by an older build, which then takes its defaults.
    #[serde(default)]
    pub app: crate::settings::AppSettings,
}

/// One object's state at the moment a drag began.
///
/// A drag is computed from this rather than from the current state, so the many
/// pointer reports a gesture produces are idempotent: each one asks "where does
/// this object go given the pointer is *here*", never "move it a bit further".
#[derive(Debug, Clone)]
pub struct TransformBaseline {
    /// Which object.
    pub object: ve_core::Id,
    /// Its anchor when the drag began, at the step being edited.
    pub anchor: ve_core::LonLat,
    /// Its rotation when the drag began, degrees, at the step being edited.
    pub rotation_deg: f64,
    /// Its scale when the drag began, percent, at the step being edited.
    pub scale_pct: f64,
    /// The three transform properties **as stored** — base and keys — when
    /// the drag began.
    ///
    /// The numbers above are what the step *shows*; these are what the drag
    /// *writes*. A drag that built its command from the numbers alone replaced
    /// the whole property with a constant and erased every keyframe the object
    /// had — and, having fabricated `before` the same way, undo could not bring
    /// them back.
    pub position: ve_core::keyframe::Animatable,
    /// See [`Self::position`].
    pub rotation: ve_core::keyframe::Animatable,
    /// See [`Self::position`].
    pub scale: ve_core::keyframe::Animatable,
    /// Which space its geometry is defined in.
    ///
    /// Moving an anchor re-expresses the geometry in the new frame, and a
    /// projected object's points are map-space metres: converting them through
    /// a ground frame would bend the shape as the anchor moved.
    pub space: ve_render::aeqd::Space,
    /// The globe aspect frozen with the geometry.
    pub projection_origin: ve_core::LonLat,
    /// How far it reaches from its anchor, in metres.
    pub reach_m: f64,
    /// Its geometry when the drag began, for anchor moves.
    pub geometry: ve_core::document::Geometry,
    /// Animated perimeter, retained when moving the anchor.
    pub shape_animation: Option<ve_core::shape_animation::ShapeAnimation>,
    /// Destructive cuts, reframed with the object on anchor moves.
    pub erased: Vec<ve_core::document::Erasure>,
    /// Its footprint in local units, for the drag preview.
    ///
    /// Captured once, at pointer-down: the document does not change during a
    /// drag, so re-flattening the object on every pointer report would be work
    /// for an answer that cannot have changed.
    pub outline: crate::transform::BaselineOutline,
    /// A Displace selection's stored displacement, which a move rewrites so
    /// the destination stays on the ground: the source and the target are
    /// moved independently. `None` for every other object.
    pub displacement: Option<ve_core::keyframe::Animatable>,
}

/// A transform drag in progress.
#[derive(Debug, Clone)]
pub struct TransformGesture {
    /// Coalescing key: every update in the drag shares it, so the drag is one
    /// history entry.
    pub key: String,
    /// What the drag is doing.
    pub kind: crate::transform::TransformKind,
    /// The step being edited.
    pub step: u32,
    /// Whether an edit keys the step rather than the base (spec.md 9.3).
    ///
    /// The frontend's auto-key switch. An *animated* property is keyed
    /// regardless: with keys present the base shows at no step, so changing it
    /// would be an edit nobody could see.
    pub auto_key: bool,
    /// Where the pointer went down.
    pub pointer: ve_core::LonLat,
    /// What the selection turns and scales about.
    pub pivot: ve_core::LonLat,
    /// Bearing from the pivot to the pointer at pointer-down. Rotation is
    /// measured against it, so grabbing a handle does not snap the selection.
    pub pointer_bearing: f64,
    /// Distance from the pivot to the pointer at pointer-down, for scale.
    pub pointer_distance_m: f64,
    /// The objects being dragged, as they were when it started.
    pub items: Vec<TransformBaseline>,
}

/// Everything the running application holds beyond static configuration.
#[derive(Debug, Default)]
pub struct Session {
    /// The open project, if any.
    pub open: Option<OpenProject>,
    /// Recent files, newest first.
    pub recent: Vec<PathBuf>,
    /// Objects held for pasting. Survives closing and opening a project, the
    /// way a system clipboard does.
    pub clipboard: ve_core::clipboard::Clipboard,
    /// Imported frames held for pasting, with the layer they came from
    /// (spec.md 4.8, M20). Separate from `clipboard` because the two are
    /// different things to paste and the timeline decides which gesture the
    /// key press was.
    pub frames: crate::frames::FrameClipboard,
    /// A field captured from a region, held for pasting (spec.md 8.5, M14).
    pub capture: crate::capture::CaptureClipboard,
    /// The application's own preferences (spec.md 8.6, M15).
    pub settings: crate::settings::AppSettings,
    /// A macro capture in progress (spec.md 8.7, M16).
    ///
    /// While this holds one, **every document write is refused**: the frames
    /// being baked are of a field that has to still be there at the end.
    pub capturing: crate::macros::CaptureSession,
    /// A macro being previewed before it is kept (spec.md 8.7, M26, D71).
    ///
    /// A one-object project — the baked capture at the stamped anchor, on
    /// an empty layer — served by the tile protocol under its own revision,
    /// so the document is never written to show it and the history lock
    /// stands.
    pub preview: Option<crate::macros::PreviewScene>,
    /// The transform drag in progress, if any.
    pub transform: Option<TransformGesture>,
    /// Frozen image geometry for an absolute resize drag.
    pub image_resize: Option<crate::image::ResizeBaseline>,
    /// Counter behind [`Session::next_gesture_id`].
    gesture_counter: u64,
}

impl Session {
    /// Loads the recent list from disk, dropping anything unreadable.
    ///
    /// A malformed settings file is ignored rather than fatal: losing the
    /// recent list is a far better outcome than refusing to start.
    pub fn load(settings_file: &Path) -> Self {
        let stored = std::fs::read_to_string(settings_file)
            .ok()
            .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
            .unwrap_or_default();
        Self {
            open: None,
            recent: stored.recent_projects,
            // Normalised rather than trusted: a hand-edited file loses the
            // lines it got wrong and keeps the rest (spec.md 8.6).
            settings: stored.app.normalised(),
            ..Default::default()
        }
    }

    /// Writes the recent list back to disk.
    pub fn save_settings(&self, settings_file: &Path) -> Result<()> {
        let settings = Settings {
            recent_projects: self.recent.clone(),
            app: self.settings.clone(),
        };
        let json = serde_json::to_string_pretty(&settings)?;
        if let Some(parent) = settings_file.parent() {
            std::fs::create_dir_all(parent)
                .doing("make the settings folder at", parent.display())?;
        }
        std::fs::write(settings_file, json)
            .doing("write the settings to", settings_file.display())?;
        // The file holds the MCP bearer token in plain text (design spec
        // §5): owner-only is what makes "the file the person already owns"
        // true in practice. `fs::write` keeps an existing file's mode, so
        // this runs after the write, every time.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(settings_file, std::fs::Permissions::from_mode(0o600))
                .doing("restrict the settings file at", settings_file.display())?;
        }
        Ok(())
    }

    /// Drops every entry from the recent list, leaving the other settings.
    ///
    /// The caller persists: the list is only really cleared once the settings
    /// file no longer holds it.
    pub fn forget_recent(&mut self) {
        self.recent.clear();
    }

    /// Moves `path` to the front of the recent list.
    pub fn remember(&mut self, path: &Path) {
        self.recent.retain(|existing| existing != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }

    /// A number no earlier gesture in this session has used.
    ///
    /// Coalescing keys must differ between drags or two separate drags collapse
    /// into one undo entry. A counter is used rather than a clock because two
    /// drags can begin inside the same millisecond.
    pub fn next_gesture_id(&mut self) -> u64 {
        self.gesture_counter += 1;
        self.gesture_counter
    }

    /// The open project, or an error naming what the caller should do first.
    pub fn require_open(&mut self) -> Result<&mut OpenProject> {
        self.open.as_mut().ok_or(AppError::NoProjectOpen)
    }

    /// Saves the open project to `path`, remembering it as recent.
    pub fn save_to(&mut self, path: PathBuf) -> Result<()> {
        let open = self.require_open()?;
        io::save(&open.project, &path).doing("save the project to", path.display())?;
        open.path = Some(path.clone());
        open.dirty = false;
        self.remember(&path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::project::{FieldKind, ProjectSettings, Resolution, StepHours};

    fn project() -> Project {
        Project::new(
            "T",
            ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 24),
        )
    }

    #[test]
    fn a_new_project_starts_dirty_and_unsaved() {
        let open = OpenProject::created(project());
        assert!(open.dirty, "an unsaved new project must count as dirty");
        assert!(open.path.is_none());
    }

    #[test]
    fn editing_bumps_the_revision_and_marks_it_dirty() {
        let mut open = OpenProject::loaded(project(), PathBuf::from("/tmp/x.veproj"));
        let before = open.revision;
        open.touch();
        assert!(
            open.revision > before,
            "the revision must change so tiles are refetched"
        );
        assert!(open.dirty);
    }

    /// The bug this prevents: both projects starting at revision 1, so the
    /// webview answers the new project's tile requests out of the old
    /// project's cache and paints the previous document's strokes.
    #[test]
    fn two_projects_never_share_a_revision() {
        let a = OpenProject::created(project());
        let b = OpenProject::created(project());
        let c = OpenProject::loaded(project(), PathBuf::from("/tmp/x.veproj"));

        assert!(a.revision < b.revision, "{} < {}", a.revision, b.revision);
        assert!(b.revision < c.revision, "{} < {}", b.revision, c.revision);
    }

    /// A revision must also outrun anything a *previous* run handed out, or a
    /// restart re-enters the same cached tile addresses.
    #[test]
    fn a_revision_is_seeded_from_the_clock_not_from_zero() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64);
        let open = OpenProject::created(project());
        assert!(
            open.revision > now - 1000,
            "revision {} is not clock-seeded (now {now})",
            open.revision
        );
    }

    #[test]
    fn a_loaded_project_starts_clean() {
        let open = OpenProject::loaded(project(), PathBuf::from("/tmp/x.veproj"));
        assert!(!open.dirty);
        assert_eq!(open.path.as_deref(), Some(Path::new("/tmp/x.veproj")));
    }

    #[test]
    fn recent_entries_move_to_the_front_without_duplicating() {
        let mut session = Session::default();
        session.remember(Path::new("/a"));
        session.remember(Path::new("/b"));
        session.remember(Path::new("/a"));

        assert_eq!(
            session.recent,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn the_recent_list_is_capped() {
        let mut session = Session::default();
        for i in 0..(MAX_RECENT + 5) {
            session.remember(Path::new(&format!("/p{i}")));
        }
        assert_eq!(session.recent.len(), MAX_RECENT);
        assert_eq!(
            session.recent[0],
            PathBuf::from(&format!("/p{}", MAX_RECENT + 4))
        );
    }

    #[test]
    fn operations_needing_a_project_say_so() {
        let mut session = Session::default();
        assert!(matches!(
            session.require_open(),
            Err(AppError::NoProjectOpen)
        ));
    }

    /// A corrupt settings file must not stop the application starting.
    #[test]
    fn a_broken_settings_file_is_ignored() {
        let dir = std::env::temp_dir().join(format!("ve-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("settings.json");
        std::fs::write(&file, "{ not json").expect("write");

        let session = Session::load(&file);
        assert!(session.recent.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_settings_file_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("settings.json");
        // A pre-existing world-readable file must be tightened too, not only
        // a fresh one: `fs::write` keeps an existing file's mode.
        std::fs::write(&file, "{}").expect("seed");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let session = Session::default();
        session.save_settings(&file).expect("save");
        let mode = std::fs::metadata(&file).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "settings file mode was {mode:o}");
    }

    #[test]
    fn settings_round_trip_through_disk() {
        let dir = std::env::temp_dir().join(format!("ve-session-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("settings.json");

        let mut session = Session::default();
        session.remember(Path::new("/one.veproj"));
        session.remember(Path::new("/two.veproj"));
        session.save_settings(&file).expect("save");

        let loaded = Session::load(&file);
        assert_eq!(loaded.recent, session.recent);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Clearing has to reach the disk. A cleared list that was only cleared in
    /// memory comes back at the next launch, which looks like the button did
    /// nothing.
    #[test]
    fn a_cleared_recent_list_stays_cleared_across_a_reload() {
        let dir = std::env::temp_dir().join(format!("ve-session-clear-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("settings.json");

        let mut session = Session::default();
        session.remember(Path::new("/one.veproj"));
        session.remember(Path::new("/two.veproj"));
        session.save_settings(&file).expect("save");

        session.forget_recent();
        session.save_settings(&file).expect("save cleared");

        assert!(session.recent.is_empty());
        assert!(Session::load(&file).recent.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The other preferences share the settings file, so clearing the recent
    /// list must not take them with it.
    #[test]
    fn clearing_the_recent_list_keeps_the_other_settings() {
        let mut session = Session::default();
        session.remember(Path::new("/one.veproj"));
        let settings = session.settings.clone();

        session.forget_recent();

        assert!(session.recent.is_empty());
        assert_eq!(session.settings, settings);
    }
}
