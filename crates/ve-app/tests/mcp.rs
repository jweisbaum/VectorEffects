//! The MCP service over real HTTP against a mock application (spec.md 8.8).
#![allow(clippy::expect_used, reason = "test helpers")]

use std::collections::HashMap;

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
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
