//! The MCP service over real HTTP against a mock application (spec.md 8.8).
#![allow(clippy::expect_used, reason = "test helpers")]

use std::collections::HashMap;

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use tauri::Listener;
use tauri::Manager;
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
use ve_app::commands::AppState;
use ve_app::paths::AppPaths;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-mcp-{}-{label}-{}",
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

fn mock_app(root: &TempRoot) -> tauri::App<MockRuntime> {
    mock_builder()
        .manage(AppState::new(
            AppPaths::in_directory(&root.0).expect("paths"),
        ))
        .manage(ve_app::export::ExportCancel::default())
        .manage(ve_app::mcp::McpService::default())
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

/// Starts the service on a free port with a fresh token.
fn serve(app: &tauri::App<MockRuntime>) -> (u16, String) {
    let token = ve_app::mcp::token::fresh();
    let running = ve_app::mcp::server::start(app.handle().clone(), 0, token.clone()).expect("bind");
    let port = running.port;
    // Held for the test's life by the service, the way the app holds it.
    app.state::<ve_app::mcp::McpService>().adopt(running);
    (port, token)
}

fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

async fn client(port: u16, token: &str) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let mut headers = HashMap::new();
    headers.insert(
        http::HeaderName::from_static("authorization"),
        http::HeaderValue::from_str(&format!("Bearer {token}")).expect("header"),
    );
    let config = StreamableHttpClientTransportConfig::with_uri(url(port)).custom_headers(headers);
    let transport = StreamableHttpClientTransport::with_client(reqwest::Client::new(), config);
    ().serve(transport).await.expect("initialize")
}

#[tokio::test]
async fn a_request_without_the_token_is_refused_before_mcp() {
    let root = TempRoot::new("auth");
    let app = mock_app(&root);
    let (port, _token) = serve(&app);
    let response = reqwest::Client::new()
        .post(url(port))
        .header("content-type", "application/json")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn a_wrong_token_is_refused() {
    let root = TempRoot::new("wrong-token");
    let app = mock_app(&root);
    let (port, _token) = serve(&app);
    let response = reqwest::Client::new()
        .post(url(port))
        .header("content-type", "application/json")
        .header("authorization", "Bearer nope")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn the_right_token_initialises_and_lists_tools() {
    let root = TempRoot::new("init");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    let info = client.peer_info().expect("server info");
    let server_info = info.server_info.as_ref().expect("implementation identity");
    assert_eq!(server_info.name, "VectorEffects");
    let tools = client.list_all_tools().await.expect("tools");
    assert!(
        !tools.is_empty(),
        "the project and structure tools: {tools:?}"
    );
    client.cancel().await.expect("close");
}

#[tokio::test]
async fn off_means_no_socket_and_stop_releases_the_port() {
    let root = TempRoot::new("off");
    let app = mock_app(&root);
    let service = app.state::<ve_app::mcp::McpService>();
    assert_eq!(service.running_port(), None);
    let (port, _token) = serve(&app);
    assert_eq!(service.running_port(), Some(port));
    service.apply(app.handle(), &ve_app::settings::McpSettings::default());
    assert_eq!(service.running_port(), None);
    // The port is free again: a bind of our own succeeds.
    let again = std::net::TcpListener::bind(("127.0.0.1", port));
    assert!(again.is_ok(), "port still held after stop");
}

#[tokio::test]
async fn mcp_set_issues_a_token_on_enable_and_clears_it_on_disable() {
    let root = TempRoot::new("set");
    let app = mock_app(&root);
    let free_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind free port");
        listener.local_addr().expect("addr").port()
    };
    let status = ve_app::settings::mcp_set(
        app.handle().clone(),
        app.state(),
        app.state(),
        true,
        free_port,
    )
    .expect("enable");
    assert_eq!(status.token.len(), 43);
    assert!(matches!(status.bound_port, Some(p) if p != 0));

    let status = ve_app::settings::mcp_set(
        app.handle().clone(),
        app.state(),
        app.state(),
        false,
        free_port,
    )
    .expect("disable");
    assert!(status.token.is_empty());
    assert_eq!(status.bound_port, None);
    assert_eq!(app.state::<ve_app::mcp::McpService>().running_port(), None);
}

use rmcp::model::CallToolResult;
use serde_json::{Value, json};

/// Calls a tool and returns its structured content, or panics with its text.
async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &'static str,
    args: Value,
) -> Value {
    let params = CallToolRequestParams::new(name)
        .with_arguments(args.as_object().cloned().unwrap_or_default());
    let result: CallToolResult = client.call_tool(params).await.expect("call");
    assert_ne!(
        result.is_error,
        Some(true),
        "{name} failed: {:?}",
        result.content
    );
    result.structured_content.unwrap_or(Value::Null)
}

async fn call_err(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &'static str,
    args: Value,
) -> String {
    let params = CallToolRequestParams::new(name)
        .with_arguments(args.as_object().cloned().unwrap_or_default());
    let result: CallToolResult = client.call_tool(params).await.expect("call");
    assert_eq!(result.is_error, Some(true), "{name} should have failed");
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn new_project_args(name: &str) -> Value {
    json!({ "name": name, "field_kind": "wind", "resolution": "1.0", "step_hours": 3, "step_count": 4 })
}

#[tokio::test]
async fn a_client_creates_a_project_and_the_app_sees_it() {
    let root = TempRoot::new("project");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    let summary = call(&client, "project_new", new_project_args("Painted")).await;
    assert_eq!(summary["name"], "Painted");
    assert_eq!(summary["step_count"], 4);
    // The interface's own read agrees.
    let current = ve_app::projects::current(app.state::<AppState>().inner())
        .expect("current")
        .expect("open");
    assert_eq!(current.name, "Painted");
    let status = call(&client, "project_status", json!({})).await;
    assert_eq!(status["project"]["name"], "Painted");
    client.cancel().await.expect("close");
}

#[tokio::test]
async fn unsaved_work_is_refused_without_discard() {
    let root = TempRoot::new("dirty");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("One")).await;
    call(&client, "layer_add", json!({ "name": "Extra" })).await;
    let message = call_err(&client, "project_new", new_project_args("Two")).await;
    // `AppError::UnsavedChanges`'s own wording ("...changes that are not
    // saved...") never uses the word "unsaved" as one token.
    assert!(message.contains("not saved"), "{message}");
    let summary = call(
        &client,
        "project_new",
        json!({ "name": "Two", "field_kind": "wind", "resolution": "1.0", "step_hours": 3, "step_count": 4, "discard_unsaved": true }),
    )
    .await;
    assert_eq!(summary["name"], "Two");
    client.cancel().await.expect("close");
}

#[tokio::test]
async fn layers_and_objects_round_trip_through_the_interface_reads() {
    let root = TempRoot::new("objects");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("Objects")).await;
    let layers = call(&client, "layers_list", json!({ "step": 0 })).await;
    let first = layers["layers"][0]["id"].as_u64().expect("layer id");
    call(
        &client,
        "layer_set",
        json!({ "layer": first, "name": "Surface", "visible": true }),
    )
    .await;
    let created = call(
        &client,
        "object_create",
        json!({
            "tool": "circle",
            "gesture": { "kind": "point", "at": [-40.0, 30.0] },
            "options": [],
            "layer": first
        }),
    )
    .await;
    let object = created["object"].as_u64().expect("object id");
    let tree = ve_app::document::tree(app.state::<AppState>().inner(), 0).expect("tree");
    let names: Vec<String> = tree.layers.iter().map(|l| l.name.clone()).collect();
    assert!(names.contains(&"Surface".to_owned()), "{names:?}");
    assert!(
        tree.layers
            .iter()
            .any(|l| l.objects.iter().any(|o| o.id == object))
    );
    let props = call(
        &client,
        "object_get",
        json!({ "object": object, "step": 0 }),
    )
    .await;
    assert!(
        props["properties"]
            .as_array()
            .map(|p| !p.is_empty())
            .unwrap_or(false)
    );
    let set = call(
        &client,
        "object_set",
        json!({ "object": object, "step": 0, "values": { "Speed": { "kind": "number", "value": 12.0 } } }),
    )
    .await;
    assert_eq!(set["can_undo"], true);
    let found = call(
        &client,
        "objects_in_region",
        json!({ "west": -50.0, "south": 20.0, "east": -30.0, "north": 40.0, "step": 0 }),
    )
    .await;
    assert!(
        found["objects"]
            .as_array()
            .expect("ids")
            .contains(&json!(object))
    );
    call(&client, "object_remove", json!({ "objects": [object] })).await;
    let tree = ve_app::document::tree(app.state::<AppState>().inner(), 0).expect("tree");
    assert!(
        !tree
            .layers
            .iter()
            .any(|l| l.objects.iter().any(|o| o.id == object))
    );
    client.cancel().await.expect("close");
}

#[tokio::test]
async fn a_write_tool_emits_document_changed_once_and_a_read_tool_emits_nothing() {
    let root = TempRoot::new("events");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;

    // `emit` calls a registered listener synchronously (tauri's event bus
    // does not go through the mock runtime's own event loop), but a channel
    // with a bounded recv is used anyway rather than assuming that — it is
    // the deterministic form either way, with no sleep.
    let (changed_tx, changed_rx) = std::sync::mpsc::channel::<String>();
    app.listen(ve_app::mcp::events::CHANGED, move |event| {
        let _ = changed_tx.send(event.payload().to_owned());
    });
    let (activity_tx, activity_rx) = std::sync::mpsc::channel::<String>();
    app.listen(ve_app::mcp::events::ACTIVITY, move |event| {
        let _ = activity_tx.send(event.payload().to_owned());
    });

    // A write tool: exactly one `document://changed`, naming the project,
    // and `mcp://activity` naming the tool that ran.
    call(&client, "project_new", new_project_args("Events")).await;

    let changed_payload = changed_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("document://changed should have fired for a write tool");
    let changed: Value = serde_json::from_str(&changed_payload).expect("json");
    assert_eq!(changed["project"]["name"], "Events");
    assert_eq!(changed["opened"], true);
    assert!(
        matches!(
            changed_rx.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "document://changed fired more than once for one write tool call"
    );

    let activity_payload = activity_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("mcp://activity should have fired");
    let activity: Value = serde_json::from_str(&activity_payload).expect("json");
    assert_eq!(activity["last_tool"], "project_new");
    assert_eq!(activity["sessions"], 1);

    // A read tool: no `document://changed` at all.
    call(&client, "project_status", json!({})).await;
    assert!(
        matches!(
            changed_rx.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "a read tool must not emit document://changed"
    );

    // `McpService::status` reports the live session while the client is
    // connected, the way `off_means_no_socket_and_stop_releases_the_port`
    // already reads `running_port()` straight off the service.
    let status = app
        .state::<ve_app::mcp::McpService>()
        .status(&ve_app::settings::McpSettings::default());
    assert_eq!(status.sessions, 1);
    assert_eq!(status.last_tool.as_deref(), Some("project_status"));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn layer_set_keeps_the_other_speed_bound_and_refuses_a_lone_bound_with_no_band() {
    let root = TempRoot::new("speed-bounds");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("Bounds")).await;
    let layers = call(&client, "layers_list", json!({ "step": 0 })).await;
    let layer = layers["layers"][0]["id"].as_u64().expect("layer id");

    // No band yet: a lone bound is refused, and writes nothing.
    let message = call_err(
        &client,
        "layer_set",
        json!({ "layer": layer, "min_mps": 5.0 }),
    )
    .await;
    assert!(message.contains("min_mps/max_mps"), "{message}");
    let tree = ve_app::document::tree(app.state::<AppState>().inner(), 0).expect("tree");
    let filter = tree.layers[0].speed_filter.as_ref().expect("filter");
    assert_eq!(filter.speed_min_mps, None);
    assert_eq!(filter.speed_max_mps, None);

    // Establish a band with both ends.
    call(
        &client,
        "layer_set",
        json!({ "layer": layer, "min_mps": 2.0, "max_mps": 20.0 }),
    )
    .await;

    // Setting one end alone (ruling, task 3 review, fix round 1, finding 1)
    // keeps the layer's other current bound rather than clearing it.
    call(
        &client,
        "layer_set",
        json!({ "layer": layer, "min_mps": 5.0 }),
    )
    .await;
    let tree = ve_app::document::tree(app.state::<AppState>().inner(), 0).expect("tree");
    let filter = tree.layers[0].speed_filter.as_ref().expect("filter");
    assert_eq!(filter.speed_min_mps, Some(5.0));
    assert_eq!(filter.speed_max_mps, Some(20.0));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn keyframes_and_motion_animate_an_object() {
    let root = TempRoot::new("time");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("Time")).await;
    let created = call(
        &client,
        "object_create",
        json!({ "tool": "circle", "gesture": { "kind": "point", "at": [10.0, 10.0] } }),
    )
    .await;
    let object = created["object"].as_u64().expect("id");
    // `PropId::Position`'s `Debug` form is "Position" (schema.rs), and
    // `PropertyValue` is tagged `{"kind":"position","lon":..,"lat":..}"`
    // (document.rs), not a raw `[lon, lat]` pair.
    //
    // `Animatable::value_at` (keyframe.rs) holds a single key's value at
    // every step, first and last alike, so keying only step 3 would move
    // the object at step 0 too. Step 0 is keyed first, with `value: null`
    // to pin the position it already has (create.rs sets only the base),
    // so the object actually animates from (10, 10) to (20, 10).
    call(
        &client,
        "keyframe_set",
        json!({ "object": object, "property": "Position", "step": 0, "value": null }),
    )
    .await;
    call(
        &client,
        "keyframe_set",
        json!({ "object": object, "property": "Position", "step": 3, "value": { "kind": "position", "lon": 20.0, "lat": 10.0 } }),
    )
    .await;
    let tracks = call(
        &client,
        "object_tracks",
        json!({ "object": object, "step": 0 }),
    )
    .await;
    let position = tracks["tracks"]
        .as_array()
        .expect("tracks")
        .iter()
        .find(|t| t["property"] == "Position")
        .expect("position track");
    assert_eq!(
        position["keys"].as_array().expect("keys").len(),
        2,
        "{position}"
    );
    let summary = call(
        &client,
        "motion_add",
        json!({ "object": object, "step": 0, "direction": 90.0, "speed_mps": 5.0, "overwrite": true }),
    )
    .await;
    assert_eq!(summary["can_undo"], true);
    let sample = call(
        &client,
        "field_sample",
        json!({ "points": [[10.0, 10.0]], "steps": [0] }),
    )
    .await;
    assert_eq!(sample["samples"][0]["defined"], true, "{sample}");
    let undone = call(&client, "undo", json!({})).await;
    assert_eq!(undone["can_redo"], true);
    client.cancel().await.expect("close");
}

#[tokio::test]
async fn view_tools_emit_events_for_the_frontend() {
    use tauri::Listener;
    let root = TempRoot::new("view");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("View")).await;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let focus_tx = tx.clone();
    app.listen("view://focus", move |event| {
        let _ = focus_tx.send(event.payload().to_owned());
    });
    app.listen("view://step", move |event| {
        let _ = tx.send(event.payload().to_owned());
    });
    call(
        &client,
        "view_focus",
        json!({ "lon": -70.5, "lat": 41.0, "px_per_deg": 12.0 }),
    )
    .await;
    call(&client, "step_set", json!({ "step": 2 })).await;
    let focus = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("focus event");
    assert!(focus.contains("-70.5"), "{focus}");
    let step = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("step event");
    assert_eq!(step.trim(), "2");
    let message = call_err(&client, "step_set", json!({ "step": 99 })).await;
    assert!(message.contains("step"), "{message}");
    client.cancel().await.expect("close");
}
