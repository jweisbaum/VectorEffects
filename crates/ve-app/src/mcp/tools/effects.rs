//! What each MCP tool does to the person's work, published as the MCP tool
//! annotations (spec.md 8.8).
//!
//! One table, so the annotations, the Claude Code permission rules and the
//! settings dialog cannot disagree. Annotations are hints a client may act
//! on: Codex does (it runs a read-only tool, and one neither destructive nor
//! open-world, without asking); Claude Code does not, which is why
//! `claude_settings` writes rules from this same table.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::ToolAnnotations;

use crate::settings::McpAsk;

/// What a tool does, coarsely enough to decide whether to ask first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads; changes nothing.
    Read,
    /// Changes the open document, undoably.
    Edit,
    /// Moves what the interface shows; not the document.
    View,
    /// Writes or replaces files on disk.
    Files,
    /// Downloads from the internet.
    Network,
    /// Can discard unsaved work when told to.
    Discard,
    /// `invoke`: any command at all.
    Anything,
}

/// Every tool the service lists, by name, with its effect. The integration
/// test holds this to the tool list both ways.
pub const TABLE: &[(&str, Effect)] = &[
    // guide
    ("vectoreffects_guide", Effect::Read),
    // project
    ("project_status", Effect::Read),
    ("project_new", Effect::Discard),
    ("project_open", Effect::Discard),
    ("project_save", Effect::Files),
    ("project_close", Effect::Discard),
    ("recent_projects", Effect::Read),
    ("tool_catalogue", Effect::Read),
    // structure
    ("layers_list", Effect::Read),
    ("layer_add", Effect::Edit),
    ("layer_set", Effect::Edit),
    ("layer_move", Effect::Edit),
    ("layer_remove", Effect::Edit),
    ("objects_list", Effect::Read),
    ("object_get", Effect::Read),
    ("object_create", Effect::Edit),
    ("object_set", Effect::Edit),
    ("object_move", Effect::Edit),
    ("object_duplicate", Effect::Edit),
    ("object_remove", Effect::Edit),
    ("objects_in_region", Effect::Read),
    // time
    ("keyframe_set", Effect::Edit),
    ("keyframe_remove", Effect::Edit),
    ("keyframe_move", Effect::Edit),
    ("interpolation_set", Effect::Edit),
    ("motion_add", Effect::Edit),
    ("follow_set", Effect::Edit),
    ("timeline_set", Effect::Edit),
    ("object_tracks", Effect::Read),
    // field
    ("field_sample", Effect::Read),
    ("field_capture", Effect::Edit),
    ("field_paste", Effect::Edit),
    ("macro_list", Effect::Read),
    ("macro_insert", Effect::Edit),
    // files: an import reads a local file into an undoable layer; an export
    // or a save writes, and can replace, a file
    ("import_grib", Effect::Edit),
    ("import_zarr", Effect::Edit),
    ("import_image", Effect::Edit),
    ("nrt_products", Effect::Read),
    ("import_nrt", Effect::Network),
    ("history_archives", Effect::Read),
    ("import_history", Effect::Network),
    ("export_grib", Effect::Files),
    ("export_zarr", Effect::Files),
    ("export_cancel", Effect::Edit),
    // view
    ("view_focus", Effect::View),
    ("step_set", Effect::View),
    ("selection_set", Effect::View),
    ("screenshot", Effect::Read),
    // history
    ("undo", Effect::Edit),
    ("redo", Effect::Edit),
    ("history_list", Effect::Read),
    ("history_jump", Effect::Edit),
    // weather
    ("storm_create", Effect::Edit),
    // escape
    ("invoke", Effect::Anything),
];

/// The effect of the tool called `name`, if the table knows it.
pub fn effect_of(name: &str) -> Option<Effect> {
    TABLE.iter().find(|(n, _)| *n == name).map(|&(_, e)| e)
}

/// The annotations a tool of `effect` publishes. Every hint is stated: a
/// missing one is read as `true` by Codex, which asks.
pub fn annotations(effect: Effect) -> ToolAnnotations {
    let (read_only, destructive, idempotent, open_world) = match effect {
        Effect::Read => (true, false, true, false),
        Effect::Edit => (false, false, false, false),
        Effect::View => (false, false, true, false),
        Effect::Files | Effect::Discard => (false, true, false, false),
        Effect::Network => (false, false, false, true),
        Effect::Anything => (false, true, false, true),
    };
    ToolAnnotations::from_raw(
        None,
        Some(read_only),
        Some(destructive),
        Some(idempotent),
        Some(open_world),
    )
}

/// Whether a tool of `effect` may run without asking under the choice `ask`.
pub fn runs_without_asking(effect: Effect, ask: McpAsk) -> bool {
    match ask {
        McpAsk::Everything => false,
        McpAsk::Nothing => true,
        McpAsk::Outside => matches!(effect, Effect::Read | Effect::Edit | Effect::View),
    }
}

/// Sets every routed tool's annotations from the table. A tool the table
/// does not know is left without, which the integration test refuses.
pub fn apply<S>(router: &mut ToolRouter<S>) {
    for (name, route) in router.map.iter_mut() {
        if let Some(effect) = effect_of(name) {
            route.attr.annotations = Some(annotations(effect));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_runs_without_asking_under_each_choice() {
        for e in [Effect::Read, Effect::Edit, Effect::View] {
            assert!(runs_without_asking(e, McpAsk::Outside), "{e:?}");
        }
        for e in [
            Effect::Files,
            Effect::Network,
            Effect::Discard,
            Effect::Anything,
        ] {
            assert!(!runs_without_asking(e, McpAsk::Outside), "{e:?}");
        }
        assert!(
            TABLE
                .iter()
                .all(|&(_, e)| !runs_without_asking(e, McpAsk::Everything))
        );
        assert!(
            TABLE
                .iter()
                .all(|&(_, e)| runs_without_asking(e, McpAsk::Nothing))
        );
    }
}
