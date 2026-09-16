//! The MCP service over real HTTP against a mock application (spec.md 8.8).
#![allow(clippy::expect_used, reason = "test helpers")]

use std::collections::HashMap;

use rmcp::ServiceExt;
#[allow(unused_imports)]
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
    assert!(tools.is_empty(), "no tools yet in this task: {tools:?}");
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
