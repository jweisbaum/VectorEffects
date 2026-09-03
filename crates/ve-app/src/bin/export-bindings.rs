//! Regenerates the frontend's TypeScript bindings from the Rust types.
//!
//! `cargo run -p ve-app --bin export-bindings`
//!
//! Every type that crosses IPC must be listed here. CI runs this and fails if
//! the working tree changes, so bindings cannot drift from the Rust types.

use std::path::PathBuf;

use ts_rs::{Config, TS};
use ve_app::animation::{InterpolationView, KeyframeView, ObjectTracks, ShrinkImpact, TrackView};
use ve_app::commands::{AppInfo, EvaluatorSelection, FieldSample};
use ve_app::create::{Gesture, NewObject, PathPoint, Tool, ToolOption};
use ve_app::document::{
    ClipboardState, DocumentTree, HistoryEntry, HistoryView, LayerNode, ObjectNode, PropertyValue,
    PropertyView,
};
use ve_app::edit::{BrushDirectionMode, BrushShape, BrushStroke};
use ve_app::error::AppErrorPayload;
use ve_app::export::{ExportEstimate, ExportProgress, ExportRequest, ExportResult};
use ve_app::palette::{
    GestureSelector, OptionDependency, PreviewKind, Sizing, ToolOptionSpec, ToolSchema,
};
use ve_app::projects::{NewProjectRequest, ProjectSummary, RecentProject};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Resolved against the manifest rather than the cwd, so the command works
    // from anywhere in the workspace.
    let out_dir: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/generated");
    std::fs::create_dir_all(&out_dir)?;

    // Ids, revisions and byte counts are all u64 but never approach 2^53, and
    // bigint is painful to work with in React — comparisons, JSON, template
    // strings all need care. Plain numbers keep the frontend simple.
    let cfg = Config::new()
        .with_out_dir(&out_dir)
        .with_large_int("number");
    AppInfo::export_all(&cfg)?;
    EvaluatorSelection::export_all(&cfg)?;
    FieldSample::export_all(&cfg)?;
    AppErrorPayload::export_all(&cfg)?;
    ProjectSummary::export_all(&cfg)?;
    RecentProject::export_all(&cfg)?;
    NewProjectRequest::export_all(&cfg)?;
    BrushStroke::export_all(&cfg)?;
    BrushShape::export_all(&cfg)?;
    BrushDirectionMode::export_all(&cfg)?;
    Tool::export_all(&cfg)?;
    Gesture::export_all(&cfg)?;
    PathPoint::export_all(&cfg)?;
    ToolOption::export_all(&cfg)?;
    NewObject::export_all(&cfg)?;
    ToolSchema::export_all(&cfg)?;
    ToolOptionSpec::export_all(&cfg)?;
    OptionDependency::export_all(&cfg)?;
    GestureSelector::export_all(&cfg)?;
    Sizing::export_all(&cfg)?;
    PreviewKind::export_all(&cfg)?;
    ObjectTracks::export_all(&cfg)?;
    TrackView::export_all(&cfg)?;
    KeyframeView::export_all(&cfg)?;
    InterpolationView::export_all(&cfg)?;
    ShrinkImpact::export_all(&cfg)?;
    DocumentTree::export_all(&cfg)?;
    ve_app::transform::SelectionTransform::export_all(&cfg)?;
    ve_app::transform::TransformKind::export_all(&cfg)?;
    ve_app::transform::TransformPreview::export_all(&cfg)?;
    ve_app::transform::ObjectOutline::export_all(&cfg)?;
    ClipboardState::export_all(&cfg)?;
    HistoryView::export_all(&cfg)?;
    HistoryEntry::export_all(&cfg)?;
    LayerNode::export_all(&cfg)?;
    ObjectNode::export_all(&cfg)?;
    PropertyView::export_all(&cfg)?;
    PropertyValue::export_all(&cfg)?;
    ExportRequest::export_all(&cfg)?;
    ExportResult::export_all(&cfg)?;
    ExportProgress::export_all(&cfg)?;
    ExportEstimate::export_all(&cfg)?;

    println!("bindings written to {}", out_dir.display());
    Ok(())
}
