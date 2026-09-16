# MCP Service Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An MCP server inside the shipped application, on loopback, switched on and off in Settings, through which a client can drive every feature the interface has and watch the interface follow.

**Architecture:** `rmcp` serves Streamable HTTP over `hyper` on `127.0.0.1`, started by a setting. Tools call the existing `#[tauri::command]` functions with a `tauri::State` obtained from the `AppHandle`, so nothing is duplicated; a Tauri **mock app** (`tauri::test`) gives the integration tests that same handle without a window. After each write the service emits `document://changed`, and the frontend treats it like the return value of its own call.

**Tech Stack:** Rust: `rmcp` 3.4 (`server`, `macros`, `transport-streamable-http-server`), `hyper` 1 (`server`, `http1`), `hyper-util` (`tokio`), `http-body-util`, `tokio` 1, `tokio-util` 0.7, `schemars` 1, `getrandom` 0.3, `base64` 0.22. Tests: `rmcp` (`client`, `transport-streamable-http-client-reqwest`), `tauri` feature `test`. Frontend: React, `@tauri-apps/api/event`, vitest with happy-dom.

**Spec:** `docs/superpowers/specs/2026-09-16-mcp-service-design.md`

## Global Constraints

- Bind `127.0.0.1` only; default port `47391`; endpoint path `/mcp`.
- Every request carries `Authorization: Bearer <token>` or gets `401` before any MCP handling.
- Token: 32 random bytes, base64url without padding, regenerated on enable and on rotate; cleared on disable.
- With `mcp.enabled == false` no socket, thread or task exists.
- No new `-sys` crate: `cargo tree -p ve-app -e normal | grep -- '-sys'` must list nothing that `git show HEAD:Cargo.lock` did not.
- The `webdriver` feature and `tests/webdriver_optional.rs` are untouched.
- Every tool that writes emits `document://changed`; commands invoked by the interface emit nothing.
- Every `f64` that reaches the settings file needs no helper here: `McpSettings` holds a `bool`, a `u16` and a `String`.
- Run before each commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `VE_FORCE_CPU=1 cargo test -p ve-app`, and for frontend tasks `npm run ui:typecheck && npm run ui:test`.
- **One deviation from the spec, recorded here and in Task 12:** spec §3 says tools call `*_impl` functions. They call the command functions themselves, with `app.state::<AppState>()`, which is a `tauri::State` like the one Tauri passes. That is the stronger form of "no tool duplicates command logic"; no split is made.

## File structure

| Path | Responsibility |
|---|---|
| `crates/ve-app/src/settings.rs` | `McpSettings` on `AppSettings`; `mcp_status`, `mcp_set`, `mcp_rotate_token` commands |
| `crates/ve-app/src/mcp/mod.rs` | `McpService` managed state: the running listener, pending captures, activity |
| `crates/ve-app/src/mcp/token.rs` | token generation and comparison |
| `crates/ve-app/src/mcp/server.rs` | bind, accept loop, bearer check, `rmcp` service wiring, stop |
| `crates/ve-app/src/mcp/tools.rs` | the `VectorEffects` handler: every curated tool |
| `crates/ve-app/src/mcp/invoke.rs` | the `invoke` table and its coverage test |
| `crates/ve-app/src/mcp/events.rs` | `document://changed`, `view://*`, `mcp://activity` payloads and emit helpers |
| `crates/ve-app/src/mcp/capture.rs` | `deliver_capture` command and the screenshot rendezvous |
| `crates/ve-app/tests/mcp.rs` | integration tests over HTTP against a mock app |
| `ui/src/mcp/follow.ts` | pure reducer for `document://changed` and the view events |
| `ui/src/settings/McpSection.tsx` | the Settings section |
| `ui/src/App.tsx`, `ui/src/map/MapView.tsx`, `ui/src/hint.ts`, `ui/src/ipc.ts` | wiring |
| `spec.md`, `plan.md`, `CLAUDE.md`, `docs/USER-GUIDE.md`, `ui/src/help/topics.ts` | rules and documentation |

---

### Task 1: Settings and token

**Files:**
- Modify: `crates/ve-app/Cargo.toml`
- Modify: `crates/ve-app/src/settings.rs` (struct at line 470, `Default` at line 515, commands after line 965)
- Create: `crates/ve-app/src/mcp/mod.rs`, `crates/ve-app/src/mcp/token.rs`
- Modify: `crates/ve-app/src/lib.rs` (`pub mod mcp;`, handler list, `.manage`)
- Modify: `crates/ve-app/examples/export_bindings.rs`
- Test: `crates/ve-app/src/mcp/token.rs` (unit), `crates/ve-app/src/settings.rs` (unit)

**Interfaces:**
- Produces: `settings::McpSettings { enabled: bool, port: u16, token: String }`, `AppSettings.mcp`, `mcp::token::fresh() -> String`, `mcp::token::matches(header: Option<&str>, token: &str) -> bool`, `mcp::McpService` (fields filled in Task 2), commands `mcp_status`, `mcp_set`, `mcp_rotate_token`, `settings::McpStatus`.

- [ ] **Step 1: Add the dependencies**

In `crates/ve-app/Cargo.toml` `[dependencies]`, after `tiff`:

```toml
# The MCP service (spec.md 8.8). Pure Rust; the listener exists only while
# the setting is on. See invariant 5's second exception.
rmcp = { version = "3.4", default-features = false, features = ["server", "macros", "transport-streamable-http-server"] }
schemars = "1"
hyper = { version = "1", features = ["server", "http1"] }
hyper-util = { version = "0.1", features = ["tokio"] }
http-body-util = "0.1"
http = "1"
bytes = "1"
tokio = { version = "1", features = ["net", "sync", "time", "macros", "rt"] }
tokio-util = "0.7"
getrandom = "0.3"
base64 = "0.22"
```

In `[dev-dependencies]`:

```toml
# A mock application gives the MCP tests a real AppHandle without a window.
tauri = { version = "2.11.5", features = ["test"] }
rmcp = { version = "3.4", default-features = false, features = ["client", "transport-streamable-http-client-reqwest"] }
reqwest = { version = "0.13", default-features = false }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

Run `cargo build -p ve-app` and then:

```bash
cargo tree -p ve-app -e normal | grep -oE '[a-z0-9_-]+-sys v[0-9.]+' | sort -u > /tmp/sys-after.txt
git stash -q; cargo tree -p ve-app -e normal | grep -oE '[a-z0-9_-]+-sys v[0-9.]+' | sort -u > /tmp/sys-before.txt; git stash pop -q
diff /tmp/sys-before.txt /tmp/sys-after.txt
```

Expected: no output. If `rmcp` pulled a `-sys` crate, find the feature that did and drop it.

- [ ] **Step 2: Write the failing token tests**

Create `crates/ve-app/src/mcp/token.rs`:

```rust
//! The bearer token a client must present (spec.md 8.8).

use base64::Engine;

/// Thirty-two random bytes, base64url without padding: 43 characters.
pub fn fresh() -> String {
    let mut bytes = [0u8; 32];
    // The OS random source failing is a broken machine; an empty token would
    // then match nothing, which is the safe failure.
    if getrandom::fill(&mut bytes).is_err() {
        return String::new();
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Whether an `Authorization` header value is `Bearer <token>` for this token.
///
/// An empty token matches nothing: the service is never open by accident.
pub fn matches(header: Option<&str>, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let Some(value) = header else { return false };
    let Some(presented) = value.strip_prefix("Bearer ") else { return false };
    constant_time_eq(presented.trim().as_bytes(), token.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_token_is_43_url_safe_characters_and_unique() {
        let a = fresh();
        let b = fresh();
        assert_eq!(a.len(), 43);
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
        assert_ne!(a, b);
    }

    #[test]
    fn only_the_exact_bearer_matches() {
        let token = fresh();
        assert!(matches(Some(&format!("Bearer {token}")), &token));
        assert!(!matches(Some(&token), &token));
        assert!(!matches(Some("Bearer nope"), &token));
        assert!(!matches(None, &token));
        assert!(!matches(Some("Bearer "), ""));
    }
}
```

Create `crates/ve-app/src/mcp/mod.rs`:

```rust
//! The MCP service (spec.md 8.8): a loopback endpoint through which a client
//! drives the application, on only while the setting says so.

pub mod token;
```

Add `pub mod mcp;` to `crates/ve-app/src/lib.rs` beside the other modules.

- [ ] **Step 3: Run the tests**

Run: `cargo test -p ve-app --lib mcp::token`
Expected: 2 passed.

- [ ] **Step 4: Add `McpSettings` to `AppSettings`**

In `crates/ve-app/src/settings.rs`, before `pub struct AppSettings`:

```rust
/// The MCP service's switch, port and token (spec.md 8.8).
///
/// The token is a plain string in the file the person already owns: it
/// grants a local process what sitting at the keyboard grants, nothing more.
/// Empty means no token, and `token::matches` refuses everything then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "McpSettings.ts")]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
}

/// The port a fresh install listens on when the service is first enabled.
pub const DEFAULT_MCP_PORT: u16 = 47391;

impl Default for McpSettings {
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_MCP_PORT, token: String::new() }
    }
}
```

Add to `AppSettings`, after `auto_scale`:

```rust
    /// The MCP service (spec.md 8.8). Absent from older files: off.
    #[serde(default)]
    pub mcp: McpSettings,
```

and `mcp: McpSettings::default(),` to `impl Default for AppSettings`.

Add a unit test in `settings.rs`'s `mod tests` (create the module if there is none):

```rust
    #[test]
    fn a_settings_file_without_mcp_loads_with_it_off() {
        let json = r#"{"glyphs":{},"distance_unit":"km","speed_unit":"kt","autosave":"recovery","shortcuts":[],"default_wind_scale_knots":60.0,"default_current_scale_knots":6.0,"macro_directory":"","projection":"equirectangular","auto_scale":false}"#;
        let settings: AppSettings = serde_json::from_str(json).expect("older settings load");
        assert_eq!(settings.mcp, McpSettings::default());
        assert!(!settings.mcp.enabled);
        assert_eq!(settings.mcp.port, 47391);
    }
```

If `glyphs` or `projection` refuse those literals, copy the exact JSON `serde_json::to_string(&AppSettings::default())` produces, delete the `"mcp"` key from it, and use that string.

Run: `cargo test -p ve-app --lib settings::tests::a_settings_file_without_mcp_loads_with_it_off`
Expected: PASS.

- [ ] **Step 5: Add the status type and the three commands**

Append to `crates/ve-app/src/settings.rs`:

```rust
/// What the Settings dialog shows about the service (spec.md 8.8).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "McpStatus.ts")]
pub struct McpStatus {
    pub enabled: bool,
    pub port: u16,
    /// The token, so the dialog can build a client configuration.
    pub token: String,
    /// The port actually bound, or null while off or if binding failed.
    pub bound_port: Option<u16>,
    /// Why the listener is not up although the setting is on.
    pub bind_error: Option<String>,
    /// Open client sessions.
    pub sessions: u32,
    /// The last tool a client called, if any.
    pub last_tool: Option<String>,
}

/// Reads the service's settings and live state.
#[tauri::command]
pub fn mcp_status(
    state: tauri::State<'_, AppState>,
    service: tauri::State<'_, crate::mcp::McpService>,
) -> Result<McpStatus> {
    let mcp = with_session(&state, |session| Ok(session.settings.mcp.clone()))?;
    Ok(service.status(&mcp))
}

/// Turns the service on or off and sets its port. Enabling issues a fresh
/// token; disabling clears it and drops the listener.
#[tauri::command]
pub fn mcp_set(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    service: tauri::State<'_, crate::mcp::McpService>,
    enabled: bool,
    port: u16,
) -> Result<McpStatus> {
    if port == 0 {
        return Err(AppError::Invalid("the MCP port must be between 1 and 65535".to_owned()));
    }
    let file = state.paths.settings_file();
    let mcp = with_session(&state, |session| {
        let mcp = &mut session.settings.mcp;
        let turning_on = enabled && !mcp.enabled;
        mcp.enabled = enabled;
        mcp.port = port;
        if turning_on || (enabled && mcp.token.is_empty()) {
            mcp.token = crate::mcp::token::fresh();
        }
        if !enabled {
            mcp.token.clear();
        }
        session.save_settings(&file)?;
        Ok(mcp.clone())
    })?;
    service.apply(&app, &mcp);
    Ok(service.status(&mcp))
}

/// Issues a new token and restarts the listener with it.
#[tauri::command]
pub fn mcp_rotate_token(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    service: tauri::State<'_, crate::mcp::McpService>,
) -> Result<McpStatus> {
    let file = state.paths.settings_file();
    let mcp = with_session(&state, |session| {
        if !session.settings.mcp.enabled {
            return Err(AppError::Invalid("turn the MCP service on before rotating its token".to_owned()));
        }
        session.settings.mcp.token = crate::mcp::token::fresh();
        session.save_settings(&file)?;
        Ok(session.settings.mcp.clone())
    })?;
    service.apply(&app, &mcp);
    Ok(service.status(&mcp))
}
```

Check the name of the "bad input" variant of `AppError` in `crates/ve-app/src/error.rs` and use it in place of `AppError::Invalid` if it differs.

Give `McpService` the two methods those commands call, in `crates/ve-app/src/mcp/mod.rs` (the real bodies arrive in Task 2; these compile now):

```rust
use std::sync::Mutex;

use crate::settings::{McpSettings, McpStatus};

/// The service's live state, managed by Tauri beside `AppState`.
#[derive(Default)]
pub struct McpService {
    /// Why the last bind failed, shown in Settings.
    pub(crate) bind_error: Mutex<Option<String>>,
}

impl McpService {
    /// Reconciles the listener with the settings: start, restart or stop.
    pub fn apply(&self, _app: &tauri::AppHandle, _mcp: &McpSettings) {}

    /// What Settings shows.
    pub fn status(&self, mcp: &McpSettings) -> McpStatus {
        McpStatus {
            enabled: mcp.enabled,
            port: mcp.port,
            token: mcp.token.clone(),
            bound_port: None,
            bind_error: self.bind_error.lock().ok().and_then(|e| e.clone()),
            sessions: 0,
            last_tool: None,
        }
    }
}
```

In `lib.rs`: `.manage(mcp::McpService::default())` after the `RenderPool` manage, and `settings::mcp_status, settings::mcp_set, settings::mcp_rotate_token,` in `generate_handler!` after `settings::set_macro_directory`.

In `examples/export_bindings.rs`, beside `AppSettings::export_all`: `ve_app::settings::McpSettings::export_all(&cfg)?; ve_app::settings::McpStatus::export_all(&cfg)?;`.

- [ ] **Step 6: Build, regenerate bindings, run the crate's tests**

```bash
cargo clippy -p ve-app --all-targets -- -D warnings
npm run bindings
git status --short ui/src/generated   # expect AppSettings.ts modified, McpSettings.ts and McpStatus.ts new
VE_FORCE_CPU=1 cargo test -p ve-app
```

Expected: clean, and all tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/ve-app ui/src/generated Cargo.lock
git commit -m "Add MCP settings, token and status commands"
```

---

### Task 2: The listener

**Files:**
- Create: `crates/ve-app/src/mcp/server.rs`
- Modify: `crates/ve-app/src/mcp/mod.rs`
- Modify: `crates/ve-app/src/lib.rs` (start at launch in `.setup`)
- Test: `crates/ve-app/tests/mcp.rs`

**Interfaces:**
- Consumes: `McpSettings`, `token::matches`, `McpService.bind_error`.
- Produces: `mcp::server::start(app: AppHandle, port: u16, token: String) -> Result<Running, String>`, `Running { port: u16, cancel: CancellationToken }` with `Running::stop(self)`, `McpService::apply` real body, `McpService::running_port() -> Option<u16>`, `mcp::server::Handler` (an empty `rmcp` `ServerHandler` that Task 3 fills), `tests/mcp.rs` helpers `mock_app(root)`, `serve(app) -> (port, token)`, `client(port, token) -> RunningService`.

- [ ] **Step 1: Write the failing integration test**

Create `crates/ve-app/tests/mcp.rs`:

```rust
//! The MCP service over real HTTP against a mock application (spec.md 8.8).
#![allow(clippy::expect_used, reason = "test helpers")]

use std::collections::HashMap;

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParam;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
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
        .manage(AppState::new(AppPaths::in_directory(&root.0).expect("paths")))
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
async fn the_right_token_initialises_and_lists_tools() {
    let root = TempRoot::new("init");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    let info = client.peer_info().expect("server info");
    assert_eq!(info.server_info.name, "VectorEffects");
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
```

The unused `CallToolRequestParam` import is used from Task 3 on; leave it with `#[allow(unused_imports)]` on that line until then.

- [ ] **Step 2: Run it to see it fail to compile**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp`
Expected: errors naming `ve_app::mcp::server`, `adopt`, `running_port`.

- [ ] **Step 3: Write the server**

Create `crates/ve-app/src/mcp/server.rs`:

```rust
//! Bind, accept, check the bearer, hand the request to `rmcp`.
//!
//! **This is the one inbound socket in the application** (invariant 5,
//! second exception). It exists only between `start` and `Running::stop`,
//! and `McpService::apply` is the only caller of both.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

use super::tools::VectorEffects;

/// The endpoint's path. Anything else is 404.
pub const PATH: &str = "/mcp";

/// A bound listener and the token that stops it.
pub struct Running {
    pub port: u16,
    pub cancel: CancellationToken,
}

impl Running {
    /// Drops the listener. Open connections end as their tasks see the token.
    pub fn stop(self) {
        self.cancel.cancel();
    }
}

type Body = BoxBody<Bytes, std::convert::Infallible>;

fn plain(status: StatusCode, text: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::from_static(text.as_bytes())).boxed())
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new()).boxed()))
}

/// Binds `127.0.0.1:port` (0 for any) and serves until `Running::stop`.
///
/// The bind is synchronous so a taken port is reported to the caller, not
/// logged from a task nobody watches.
pub fn start(app: tauri::AppHandle, port: u16, token: String) -> Result<Running, String> {
    if token.is_empty() {
        return Err("the MCP service has no token".to_owned());
    }
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .map_err(|e| format!("could not listen on 127.0.0.1:{port}: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("could not configure the listener: {e}"))?;
    let bound = listener.local_addr().map_err(|e| e.to_string())?.port();

    let cancel = CancellationToken::new();
    let hosts = vec![format!("127.0.0.1:{bound}"), format!("localhost:{bound}")];
    let config = StreamableHttpServerConfig {
        sse_keep_alive: Some(Duration::from_secs(15)),
        allowed_hosts: hosts.clone(),
        allowed_origins: hosts.iter().map(|h| format!("http://{h}")).collect(),
        cancellation_token: cancel.child_token(),
        ..Default::default()
    };
    let handler_app = app.clone();
    let service = Arc::new(StreamableHttpService::new(
        move || Ok(VectorEffects::new(handler_app.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    ));
    let token: Arc<str> = token.into();
    let loop_cancel = cancel.clone();

    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(error = %e, "mcp listener could not join the runtime");
                return;
            }
        };
        loop {
            let (stream, _) = tokio::select! {
                _ = loop_cancel.cancelled() => break,
                accepted = listener.accept() => match accepted {
                    Ok(a) => a,
                    Err(e) => { tracing::warn!(error = %e, "mcp accept failed"); continue; }
                },
            };
            let service = service.clone();
            let token = token.clone();
            let conn_cancel = loop_cancel.clone();
            tauri::async_runtime::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let service = service.clone();
                    let token = token.clone();
                    async move {
                        let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok());
                        if !super::token::matches(auth, &token) {
                            return Ok::<_, std::convert::Infallible>(plain(StatusCode::UNAUTHORIZED, "unauthorized"));
                        }
                        if req.uri().path() != PATH {
                            return Ok(plain(StatusCode::NOT_FOUND, "not found"));
                        }
                        Ok(service.handle(req).await)
                    }
                });
                let conn = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .with_upgrades();
                tokio::select! {
                    _ = conn_cancel.cancelled() => {}
                    r = conn => if let Err(e) = r { tracing::debug!(error = %e, "mcp connection ended"); },
                }
            });
        }
        tracing::info!(port = bound, "mcp listener stopped");
    });

    tracing::info!(port = bound, "mcp listener started");
    Ok(Running { port: bound, cancel })
}
```

If the compiler places `LocalSessionManager` or `StreamableHttpServerConfig` elsewhere in `rmcp` 3.4, follow its `docs.rs` path; the names are those. If `Response<Body>` and `service.handle`'s response type disagree, map the handled response's body with `.map(|b| b.boxed())`.

Create the minimal handler in `crates/ve-app/src/mcp/tools.rs` (Task 3 grows it):

```rust
//! The tools a client sees (spec.md 8.8).

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::{tool_handler, tool_router, ServerHandler};

/// One handler per client session, holding the application it drives.
#[derive(Clone)]
pub struct VectorEffects {
    pub(crate) app: tauri::AppHandle,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl VectorEffects {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app, tool_router: Self::tool_router() }
    }
}

#[tool_handler]
impl ServerHandler for VectorEffects {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation { name: "VectorEffects".into(), version: env!("CARGO_PKG_VERSION").into(), ..Default::default() },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some("Paints global wind and current fields. Open or create a project first; every edit is undoable and shows on the map.".into()),
            ..Default::default()
        }
    }
}
```

Replace `crates/ve-app/src/mcp/mod.rs` with:

```rust
//! The MCP service (spec.md 8.8): a loopback endpoint through which a client
//! drives the application, on only while the setting says so.

pub mod server;
pub mod token;
pub mod tools;

use std::sync::Mutex;

use crate::settings::{McpSettings, McpStatus};

/// The service's live state, managed by Tauri beside `AppState`.
#[derive(Default)]
pub struct McpService {
    running: Mutex<Option<server::Running>>,
    /// Why the last bind failed, shown in Settings.
    pub(crate) bind_error: Mutex<Option<String>>,
}

impl McpService {
    /// Reconciles the listener with the settings: start, restart or stop.
    ///
    /// Always restarts when on, because the token or the port may have
    /// changed and both are baked into the running listener.
    pub fn apply(&self, app: &tauri::AppHandle, mcp: &McpSettings) {
        if let Ok(mut slot) = self.running.lock()
            && let Some(running) = slot.take()
        {
            running.stop();
        }
        if !mcp.enabled {
            self.set_bind_error(None);
            return;
        }
        match server::start(app.clone(), mcp.port, mcp.token.clone()) {
            Ok(running) => {
                self.set_bind_error(None);
                self.adopt(running);
            }
            Err(message) => {
                tracing::warn!(%message, "mcp listener did not start");
                self.set_bind_error(Some(message));
            }
        }
    }

    /// Holds a listener started elsewhere (the tests start one on port 0).
    pub fn adopt(&self, running: server::Running) {
        if let Ok(mut slot) = self.running.lock() {
            if let Some(old) = slot.replace(running) {
                old.stop();
            }
        }
    }

    /// The bound port while the listener is up.
    pub fn running_port(&self) -> Option<u16> {
        self.running.lock().ok().and_then(|r| r.as_ref().map(|r| r.port))
    }

    fn set_bind_error(&self, error: Option<String>) {
        if let Ok(mut slot) = self.bind_error.lock() {
            *slot = error;
        }
    }

    /// What Settings shows.
    pub fn status(&self, mcp: &McpSettings) -> McpStatus {
        McpStatus {
            enabled: mcp.enabled,
            port: mcp.port,
            token: mcp.token.clone(),
            bound_port: self.running_port(),
            bind_error: self.bind_error.lock().ok().and_then(|e| e.clone()),
            sessions: 0,
            last_tool: None,
        }
    }
}
```

Start at launch, in `lib.rs`'s `.setup` after `autosave::start(...)`:

```rust
            // The MCP service, only if the person left it on (spec.md 8.8).
            let mcp = app.state::<AppState>().session.lock().map(|s| s.settings.mcp.clone());
            if let Ok(mcp) = mcp {
                app.state::<mcp::McpService>().apply(app.handle(), &mcp);
            }
```

(`mcp` the module and `mcp` the local collide: name the local `mcp_settings`.)

- [ ] **Step 4: Run the tests**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp`
Expected: 3 passed. If `the_right_token_initialises_and_lists_tools` fails on the `Origin`/`Host` check, print the response body: `rmcp` names the header it refused. The client sends `Host: 127.0.0.1:<port>`, which is allowed.

- [ ] **Step 5: Clippy, format, commit**

```bash
cargo fmt --all
cargo clippy -p ve-app --all-targets -- -D warnings
git add crates/ve-app
git commit -m "Add the MCP listener with bearer auth and lifecycle"
```

---

### Task 3: Events, the tool scaffolding, and the project and structure tools

**Files:**
- Create: `crates/ve-app/src/mcp/events.rs`
- Modify: `crates/ve-app/src/mcp/tools.rs`, `crates/ve-app/src/mcp/mod.rs`
- Modify: `crates/ve-app/examples/export_bindings.rs`
- Test: `crates/ve-app/tests/mcp.rs`

**Interfaces:**
- Consumes: `VectorEffects { app }`, `projects::{new_project, open_project, save_project, save_project_as, close_project, current_project, recent_projects}`, `document::{document_tree, object_properties, set_object_property, add_layer, remove_layer, rename_layer, set_layer_visible, set_layer_locked, set_layer_parameter, set_layer_speed_range, move_layer, rename_object, remove_objects, move_object, duplicate_object, end_gesture}`, `create::create_object`, `transform::objects_in_region`, `palette::tool_palette`.
- Produces: `events::DocumentChanged { project: Option<ProjectSummary>, opened: bool }`, `events::changed(app, opened) -> Result<Option<ProjectSummary>>`, `events::CHANGED = "document://changed"`, `McpService::note_tool(name)`, `McpService::sessions` counter, `VectorEffects::run(f)` and `VectorEffects::write(f, opened)` helpers, tools `project_status`, `project_new`, `project_open`, `project_save`, `project_close`, `recent_projects`, `tool_catalogue`, `layers_list`, `layer_add`, `layer_set`, `layer_move`, `layer_remove`, `objects_list`, `object_get`, `object_create`, `object_set`, `object_move`, `object_duplicate`, `object_remove`, `objects_in_region`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/ve-app/tests/mcp.rs`:

```rust
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

/// Calls a tool and returns its structured content, or panics with its text.
async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &'static str,
    args: Value,
) -> Value {
    let result: CallToolResult = client
        .call_tool(CallToolRequestParam { name: name.into(), arguments: args.as_object().cloned() })
        .await
        .expect("call");
    assert_ne!(result.is_error, Some(true), "{name} failed: {:?}", result.content);
    result.structured_content.unwrap_or(Value::Null)
}

async fn call_err(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &'static str,
    args: Value,
) -> String {
    let result: CallToolResult = client
        .call_tool(CallToolRequestParam { name: name.into(), arguments: args.as_object().cloned() })
        .await
        .expect("call");
    assert_eq!(result.is_error, Some(true), "{name} should have failed");
    result.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect::<Vec<_>>().join(" ")
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
    let current = ve_app::projects::current(app.state::<AppState>().inner()).expect("current").expect("open");
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
    assert!(message.contains("unsaved"), "{message}");
    let summary = call(&client, "project_new", json!({ "name": "Two", "field_kind": "wind", "resolution": "1.0", "step_hours": 3, "step_count": 4, "discard_unsaved": true })).await;
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
    call(&client, "layer_set", json!({ "layer": first, "name": "Surface", "visible": true })).await;
    let created = call(&client, "object_create", json!({
        "tool": "circle",
        "gesture": { "kind": "point", "at": [-40.0, 30.0] },
        "options": [],
        "layer": first
    })).await;
    let object = created["object"].as_u64().expect("object id");
    let tree = ve_app::document::tree(app.state::<AppState>().inner(), 0).expect("tree");
    let names: Vec<String> = tree.layers.iter().map(|l| l.name.clone()).collect();
    assert!(names.contains(&"Surface".to_owned()), "{names:?}");
    assert!(tree.layers.iter().any(|l| l.objects.iter().any(|o| o.id == object)));
    let props = call(&client, "object_get", json!({ "object": object, "step": 0 })).await;
    assert!(props["properties"].as_array().map(|p| !p.is_empty()).unwrap_or(false));
    let set = call(&client, "object_set", json!({ "object": object, "step": 0, "values": { "speed": 12.0 } })).await;
    assert_eq!(set["can_undo"], true);
    let found = call(&client, "objects_in_region", json!({ "west": -50.0, "south": 20.0, "east": -30.0, "north": 40.0, "step": 0 })).await;
    assert!(found["objects"].as_array().expect("ids").contains(&json!(object)));
    call(&client, "object_remove", json!({ "objects": [object] })).await;
    let tree = ve_app::document::tree(app.state::<AppState>().inner(), 0).expect("tree");
    assert!(!tree.layers.iter().any(|l| l.objects.iter().any(|o| o.id == object)));
    client.cancel().await.expect("close");
}
```

Check `DocumentTree`'s field names in `crates/ve-app/src/document.rs` (`layers`, each with `id`, `name`, `objects`, each object with `id`) and the property name of a circle's speed in the palette (`speed` or `speed_mps`; `cargo run -p ve-app --example export_bindings` is not needed, `grep -n '"speed' crates/ve-core/src/schema.rs` is) and adjust the test literals to what the code names them.

- [ ] **Step 2: Run to see them fail**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp`
Expected: the new tests fail with "tool not found" text from `call`.

- [ ] **Step 3: Events**

Create `crates/ve-app/src/mcp/events.rs`:

```rust
//! What the service tells the frontend, so the interface follows the client
//! (spec.md 8.8, M76).
//!
//! **Emitted by the service only.** A command the interface invoked returns
//! its summary to the caller; emitting here as well would refresh every
//! panel twice per edit.

use serde::Serialize;
use tauri::{Emitter, Manager};
use ts_rs::TS;

use crate::commands::AppState;
use crate::error::Result;
use crate::projects::ProjectSummary;

pub const CHANGED: &str = "document://changed";
pub const FOCUS: &str = "view://focus";
pub const STEP: &str = "view://step";
pub const SELECTION: &str = "view://selection";
pub const ACTIVITY: &str = "mcp://activity";

/// The document after a tool wrote to it. `project` null means closed.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "DocumentChanged.ts")]
pub struct DocumentChanged {
    pub project: Option<ProjectSummary>,
    /// A different project than before: the frontend resets step, selection
    /// and the active layer, as its own open path does.
    pub opened: bool,
}

/// Where the map should look.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ViewFocus.ts")]
pub struct ViewFocus {
    pub lon: f64,
    pub lat: f64,
    /// Screen pixels per degree; null keeps the current zoom.
    pub px_per_deg: Option<f64>,
}

/// What a client is doing, for the status bar.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "McpActivity.ts")]
pub struct McpActivity {
    pub sessions: u32,
    pub last_tool: Option<String>,
}

/// Reads the summary and tells the frontend.
pub fn changed(app: &tauri::AppHandle, opened: bool) -> Result<Option<ProjectSummary>> {
    let project = crate::projects::current(app.state::<AppState>().inner())?;
    let _ = app.emit(CHANGED, DocumentChanged { project: project.clone(), opened });
    Ok(project)
}
```

Register `DocumentChanged`, `ViewFocus` and `McpActivity` in `examples/export_bindings.rs` with `export_all`.

- [ ] **Step 4: Activity on the service**

In `crates/ve-app/src/mcp/mod.rs` add fields and methods to `McpService`:

```rust
    /// Open client sessions and the last tool called (status bar, Settings).
    activity: Mutex<events::McpActivity>,
```

(with `pub mod events;` and `#[derive(Default)]` needing `impl Default for McpActivity` — derive `Default` on `McpActivity`.)

```rust
    /// Records a tool call and tells the status bar.
    pub fn note_tool(&self, app: &tauri::AppHandle, name: &str) {
        use tauri::Emitter;
        if let Ok(mut activity) = self.activity.lock() {
            activity.last_tool = Some(name.to_owned());
            let _ = app.emit(events::ACTIVITY, activity.clone());
        }
    }

    /// A client session opened (+1) or closed (-1).
    pub fn session_delta(&self, app: &tauri::AppHandle, delta: i32) {
        use tauri::Emitter;
        if let Ok(mut activity) = self.activity.lock() {
            activity.sessions = activity.sessions.saturating_add_signed(delta);
            if activity.sessions == 0 {
                activity.last_tool = None;
            }
            let _ = app.emit(events::ACTIVITY, activity.clone());
        }
    }
```

and in `status()` replace `sessions: 0, last_tool: None` with the values from `self.activity.lock()`.

- [ ] **Step 5: The tool scaffolding and the first two groups**

Replace `crates/ve-app/src/mcp/tools.rs` with:

```rust
//! The tools a client sees (spec.md 8.8).
//!
//! Every tool calls the command the interface calls, with the `State` the
//! handle gives, so there is one implementation of each feature. A tool that
//! writes ends with `write`, which emits `document://changed`.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorData as McpError, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;

use super::events;
use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::ProjectSummary;

/// One handler per client session, holding the application it drives.
#[derive(Clone)]
pub struct VectorEffects {
    pub(crate) app: tauri::AppHandle,
    tool_router: ToolRouter<Self>,
}

fn mcp_err(err: AppError) -> McpError {
    McpError::invalid_params(err.to_string(), None)
}

impl VectorEffects {
    /// Runs a command off the async thread, since commands lock the session.
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        name: &'static str,
        f: impl FnOnce(&tauri::AppHandle) -> Result<T> + Send + 'static,
    ) -> std::result::Result<T, McpError> {
        let app = self.app.clone();
        app.state::<super::McpService>().note_tool(&app, name);
        tokio::task::spawn_blocking(move || f(&app))
            .await
            .map_err(|e| McpError::internal_error(format!("tool panicked: {e}"), None))?
            .map_err(mcp_err)
    }

    /// `run`, then tell the frontend the document changed.
    pub(crate) async fn write<T: Send + 'static>(
        &self,
        name: &'static str,
        opened: bool,
        f: impl FnOnce(&tauri::AppHandle) -> Result<T> + Send + 'static,
    ) -> std::result::Result<T, McpError> {
        let out = self.run(name, f).await?;
        events::changed(&self.app, opened).map_err(mcp_err)?;
        Ok(out)
    }
}

// ---------------------------------------------------------------- project

#[derive(Deserialize, JsonSchema)]
pub struct ProjectNewParams {
    /// Project name.
    pub name: String,
    /// "wind" or "current".
    pub field_kind: String,
    /// Grid resolution in degrees as a string: "1.0", "0.5", "0.25", "0.1".
    pub resolution: String,
    /// Hours between steps: 1, 3 or 6.
    pub step_hours: u32,
    /// Number of steps.
    pub step_count: u32,
    /// Discard unsaved changes in the open project. Refused without it.
    #[serde(default)]
    pub discard_unsaved: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct ProjectOpenParams {
    /// Path to a .veproj file.
    pub path: String,
    #[serde(default)]
    pub discard_unsaved: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct ProjectSaveParams {
    /// Save here instead of the project's own path (a Save As).
    pub path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DiscardParams {
    #[serde(default)]
    pub discard_unsaved: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct ProjectStatus {
    /// The open project, or null.
    pub project: Option<ProjectSummary>,
}

#[derive(Serialize, JsonSchema)]
pub struct Closed {
    pub closed: bool,
}

#[tool_router]
impl VectorEffects {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app, tool_router: Self::tool_router() }
    }

    #[tool(description = "The open project's summary (name, path, dirty, grid, steps, revision, undo state), or null when none is open.")]
    async fn project_status(&self) -> std::result::Result<Json<ProjectStatus>, McpError> {
        let project = self.run("project_status", |app| crate::projects::current_project(app.state())).await?;
        Ok(Json(ProjectStatus { project }))
    }

    #[tool(description = "Creates a new project and opens it in the interface. Refused while the open project has unsaved changes unless discard_unsaved is true.")]
    async fn project_new(&self, Parameters(p): Parameters<ProjectNewParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        let request = crate::projects::NewProjectRequest { name: p.name, field_kind: p.field_kind, resolution: p.resolution, step_hours: p.step_hours, step_count: p.step_count };
        let discard = p.discard_unsaved;
        self.write("project_new", true, move |app| crate::projects::new_project(app.state(), request, discard)).await.map(Json)
    }

    #[tool(description = "Opens a .veproj file in the interface.")]
    async fn project_open(&self, Parameters(p): Parameters<ProjectOpenParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("project_open", true, move |app| crate::projects::open_project(app.state(), p.path, p.discard_unsaved)).await.map(Json)
    }

    #[tool(description = "Saves the project to its own path, or to `path` as a Save As. An existing file at `path` is overwritten only if overwrite is true.")]
    async fn project_save(&self, Parameters(p): Parameters<ProjectSaveParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("project_save", false, move |app| match p.path {
            Some(path) => crate::projects::save_project_as(app.state(), path),
            None => crate::projects::save_project(app.state()),
        }).await.map(Json)
    }

    #[tool(description = "Closes the project and returns the interface to the start screen. Refused with unsaved changes unless discard_unsaved is true.")]
    async fn project_close(&self, Parameters(p): Parameters<DiscardParams>) -> std::result::Result<Json<Closed>, McpError> {
        self.write("project_close", true, move |app| crate::projects::close_project(app.state(), p.discard_unsaved)).await?;
        Ok(Json(Closed { closed: true }))
    }

    #[tool(description = "Recently opened projects, newest first.")]
    async fn recent_projects(&self) -> std::result::Result<Json<Vec<crate::projects::RecentProject>>, McpError> {
        self.run("recent_projects", |app| crate::projects::recent_projects(app.state())).await.map(Json)
    }

    #[tool(description = "Every drawing tool with its options, defaults, ranges and gesture shape. Read this before object_create.")]
    async fn tool_catalogue(&self) -> std::result::Result<Json<Vec<crate::palette::ToolSchema>>, McpError> {
        self.run("tool_catalogue", |_| crate::palette::tool_palette()).await.map(Json)
    }
}
```

`ProjectSummary`, `RecentProject` and `ToolSchema` need `#[derive(schemars::JsonSchema)]` for `Json<T>` to produce an output schema. Add `JsonSchema` to their derive lists (`schemars` is a dependency now). If a nested type in `ToolSchema` resists the derive, return `Json<Value>` with `serde_json::to_value` instead and say so in the description; the payload is the same.

- [ ] **Step 6: The structure group**

Append a second `#[tool_router(router = structure_router)]` block is not needed: add these tools to the same `#[tool_router] impl VectorEffects` block.

```rust
    // ------------------------------------------------------------- structure

    #[tool(description = "Layers and their objects at a step: ids, names, kinds, visibility, locks. Ids are what every other tool takes.")]
    async fn layers_list(&self, Parameters(p): Parameters<StepParams>) -> std::result::Result<Json<crate::document::DocumentTree>, McpError> {
        self.run("layers_list", move |app| crate::document::document_tree(app.state(), p.step)).await.map(Json)
    }

    #[tool(description = "Adds an empty painted layer on top and returns the summary.")]
    async fn layer_add(&self, Parameters(p): Parameters<NameParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("layer_add", false, move |app| crate::document::add_layer(app.state(), p.name)).await.map(Json)
    }

    #[tool(description = "Sets any of a layer's name, visibility, lock, parameter (\"wind\"/\"current\") or speed range in m/s. Omitted fields are left alone.")]
    async fn layer_set(&self, Parameters(p): Parameters<LayerSetParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("layer_set", false, move |app| {
            let state = app.state::<AppState>();
            let mut last = None;
            if let Some(name) = p.name { last = Some(crate::document::rename_layer(state.clone(), p.layer, name)?); }
            if let Some(visible) = p.visible { last = Some(crate::document::set_layer_visible(state.clone(), p.layer, visible)?); }
            if let Some(locked) = p.locked { last = Some(crate::document::set_layer_locked(state.clone(), p.layer, locked)?); }
            if let Some(parameter) = p.parameter { last = Some(crate::document::set_layer_parameter(state.clone(), p.layer, parameter)?); }
            if p.min_mps.is_some() || p.max_mps.is_some() {
                last = Some(crate::document::set_layer_speed_range(state.clone(), p.layer, p.min_mps, p.max_mps, None)?);
            }
            match last {
                Some(summary) => Ok(summary),
                None => crate::projects::current_project(state).and_then(|s| s.ok_or_else(|| AppError::Invalid("no project is open".to_owned()))),
            }
        }).await.map(Json)
    }

    #[tool(description = "Moves a layer from one index to another. Index 0 is the bottom.")]
    async fn layer_move(&self, Parameters(p): Parameters<MoveParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("layer_move", false, move |app| crate::document::move_layer(app.state(), p.from, p.to)).await.map(Json)
    }

    #[tool(description = "Removes a layer and everything on it. Undoable.")]
    async fn layer_remove(&self, Parameters(p): Parameters<LayerParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("layer_remove", false, move |app| crate::document::remove_layer(app.state(), p.layer)).await.map(Json)
    }

    #[tool(description = "Objects on one layer, or on every layer, at a step.")]
    async fn objects_list(&self, Parameters(p): Parameters<ObjectsListParams>) -> std::result::Result<Json<ObjectsList>, McpError> {
        let layer = p.layer;
        let tree = self.run("objects_list", move |app| crate::document::document_tree(app.state(), p.step)).await?;
        let objects = tree.layers.into_iter().filter(|l| layer.is_none_or(|id| id == l.id)).flat_map(|l| l.objects).collect();
        Ok(Json(ObjectsList { objects }))
    }

    #[tool(description = "An object's properties at a step, with kinds, units, ranges and whether each is keyed.")]
    async fn object_get(&self, Parameters(p): Parameters<ObjectStepParams>) -> std::result::Result<Json<ObjectProperties>, McpError> {
        let properties = self.run("object_get", move |app| crate::document::object_properties(app.state(), p.object, p.step)).await?;
        Ok(Json(ObjectProperties { properties }))
    }

    #[tool(description = "Draws an object with a tool. `tool` is a name from tool_catalogue; `gesture` is that tool's gesture ({\"kind\":\"point\",\"at\":[lon,lat]}, {\"kind\":\"stroke\",\"points\":[[lon,lat],...]}, or the catalogue's drag form); `options` is a list of {\"name\",\"value\"} pairs, omitted ones take defaults; `layer` null means the top layer. Returns the summary and the new object's id, which becomes the selection.")]
    async fn object_create(&self, Parameters(p): Parameters<ObjectCreateParams>) -> std::result::Result<Json<crate::create::Created>, McpError> {
        let object: crate::create::NewObject = serde_json::from_value(serde_json::json!({ "tool": p.tool, "gesture": p.gesture, "options": p.options, "layer": p.layer }))
            .map_err(|e| McpError::invalid_params(format!("object_create: {e}"), None))?;
        let created = self.write("object_create", false, move |app| crate::create::create_object(app.state(), object)).await?;
        let _ = tauri::Emitter::emit(&self.app, events::SELECTION, vec![created.object]);
        Ok(Json(created))
    }

    #[tool(description = "Sets one or more properties of an object at a step. `values` maps property name to value (number, string, boolean, [lon,lat], or the catalogue's enum names). With auto_key true a change on an animated property adds a keyframe at that step.")]
    async fn object_set(&self, Parameters(p): Parameters<ObjectSetParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("object_set", false, move |app| {
            let state = app.state::<AppState>();
            let mut last = None;
            for (property, raw) in p.values {
                let value: crate::document::PropertyValue = serde_json::from_value(raw).map_err(|e| AppError::Invalid(format!("{property}: {e}")))?;
                last = Some(crate::document::set_object_property(state.clone(), p.object, property, value, p.step, p.auto_key, None)?);
            }
            crate::document::end_gesture(state.clone())?;
            last.ok_or_else(|| AppError::Invalid("values is empty".to_owned()))
        }).await.map(Json)
    }

    #[tool(description = "Moves an object to a layer at an index (0 = bottom of that layer).")]
    async fn object_move(&self, Parameters(p): Parameters<ObjectMoveParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("object_move", false, move |app| crate::document::move_object(app.state(), p.object, p.layer, p.index)).await.map(Json)
    }

    #[tool(description = "Duplicates an object in place.")]
    async fn object_duplicate(&self, Parameters(p): Parameters<ObjectParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("object_duplicate", false, move |app| crate::document::duplicate_object(app.state(), p.object)).await.map(Json)
    }

    #[tool(description = "Removes objects. One undo.")]
    async fn object_remove(&self, Parameters(p): Parameters<ObjectsParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("object_remove", false, move |app| crate::document::remove_objects(app.state(), p.objects)).await.map(Json)
    }

    #[tool(description = "Ids of the objects whose footprint touches a lon/lat box at a step, optionally on one layer.")]
    async fn objects_in_region(&self, Parameters(p): Parameters<RegionParams>) -> std::result::Result<Json<ObjectIds>, McpError> {
        let objects = self.run("objects_in_region", move |app| crate::transform::objects_in_region(app.state(), p.west, p.south, p.east, p.north, p.step, p.layer)).await?;
        Ok(Json(ObjectIds { objects }))
    }
```

with these parameter and output types above the `impl`:

```rust
#[derive(Deserialize, JsonSchema)] pub struct StepParams { pub step: u32 }
#[derive(Deserialize, JsonSchema)] pub struct NameParams { pub name: String }
#[derive(Deserialize, JsonSchema)] pub struct LayerParams { pub layer: u64 }
#[derive(Deserialize, JsonSchema)] pub struct ObjectParams { pub object: u64 }
#[derive(Deserialize, JsonSchema)] pub struct ObjectsParams { pub objects: Vec<u64> }
#[derive(Deserialize, JsonSchema)] pub struct ObjectStepParams { pub object: u64, pub step: u32 }
#[derive(Deserialize, JsonSchema)] pub struct MoveParams { pub from: usize, pub to: usize }
#[derive(Deserialize, JsonSchema)]
pub struct LayerSetParams { pub layer: u64, pub name: Option<String>, pub visible: Option<bool>, pub locked: Option<bool>, pub parameter: Option<String>, pub min_mps: Option<f32>, pub max_mps: Option<f32> }
#[derive(Deserialize, JsonSchema)] pub struct ObjectsListParams { pub step: u32, pub layer: Option<u64> }
#[derive(Deserialize, JsonSchema)]
pub struct ObjectCreateParams { pub tool: String, pub gesture: Value, #[serde(default)] pub options: Vec<Value>, pub layer: Option<u64> }
#[derive(Deserialize, JsonSchema)]
pub struct ObjectSetParams { pub object: u64, pub step: u32, pub values: serde_json::Map<String, Value>, #[serde(default)] pub auto_key: bool }
#[derive(Deserialize, JsonSchema)] pub struct ObjectMoveParams { pub object: u64, pub layer: u64, pub index: usize }
#[derive(Deserialize, JsonSchema)]
pub struct RegionParams { pub west: f64, pub south: f64, pub east: f64, pub north: f64, pub step: u32, pub layer: Option<u64> }
#[derive(Serialize, JsonSchema)] pub struct ObjectsList { pub objects: Vec<crate::document::ObjectNode> }
#[derive(Serialize, JsonSchema)] pub struct ObjectProperties { pub properties: Vec<crate::document::PropertyView> }
#[derive(Serialize, JsonSchema)] pub struct ObjectIds { pub objects: Vec<u64> }
```

`DocumentTree`, `ObjectNode`, `PropertyView`, `Created` need `JsonSchema` derives too; if one contains a type that cannot derive it, wrap it as `Json<Value>` via `serde_json::to_value`. The `Option::is_none_or` call needs Rust 1.82+; the toolchain is 1.97.

- [ ] **Step 7: Session counting**

In `tools.rs`, implement `ServerHandler::on_initialized` if `rmcp` 3.4 exposes it (it does, as `fn on_initialized(&self, context: NotificationContext<RoleServer>) -> impl Future<Output = ()>`): call `self.app.state::<super::McpService>().session_delta(&self.app, 1)`. Decrement in `impl Drop for VectorEffects` — one handler per session, dropped when the session ends. Because `#[derive(Clone)]` would make the drop count wrong, remove `Clone` from `VectorEffects` (the router does not require it).

- [ ] **Step 8: Run the tests, clippy, commit**

```bash
VE_FORCE_CPU=1 cargo test -p ve-app --test mcp
cargo clippy -p ve-app --all-targets -- -D warnings
npm run bindings && git status --short ui/src/generated
cargo fmt --all
git add crates/ve-app ui/src/generated
git commit -m "Add MCP project and structure tools with document change events"
```

Expected: 6 tests pass (the earlier `the_right_token_initialises_and_lists_tools` now needs `assert!(!tools.is_empty())`; change it).

---

### Task 4: Time, field, history and view tools

**Files:**
- Modify: `crates/ve-app/src/mcp/tools.rs`
- Test: `crates/ve-app/tests/mcp.rs`

**Interfaces:**
- Consumes: `animation::{set_keyframe, remove_keyframe, move_keyframe, set_interpolation, add_constant_motion, set_follow, set_step_count, set_start_time, object_tracks}`, `commands::sample_field`, `capture::{capture_region, paste_capture}`, `macros::{macro_library, insert_macro}`, `edit::{undo, redo}`, `document::{history_view, jump_to_history}`, `events::{FOCUS, STEP, SELECTION}`.
- Produces: tools `keyframe_set`, `keyframe_remove`, `keyframe_move`, `interpolation_set`, `motion_add`, `follow_set`, `timeline_set`, `object_tracks`, `field_sample`, `field_capture`, `field_paste`, `macro_list`, `macro_insert`, `undo`, `redo`, `history_list`, `history_jump`, `view_focus`, `step_set`, `selection_set`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/ve-app/tests/mcp.rs`:

```rust
#[tokio::test]
async fn keyframes_and_motion_animate_an_object() {
    let root = TempRoot::new("time");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("Time")).await;
    let created = call(&client, "object_create", json!({ "tool": "circle", "gesture": { "kind": "point", "at": [10.0, 10.0] } })).await;
    let object = created["object"].as_u64().expect("id");
    call(&client, "keyframe_set", json!({ "object": object, "property": "position", "step": 3, "value": [20.0, 10.0] })).await;
    let tracks = call(&client, "object_tracks", json!({ "object": object, "step": 0 })).await;
    let position = tracks["tracks"].as_array().expect("tracks").iter().find(|t| t["property"] == "position").expect("position track");
    assert_eq!(position["keys"].as_array().expect("keys").len(), 2, "{position}");
    let summary = call(&client, "motion_add", json!({ "object": object, "step": 0, "direction": 90.0, "speed_mps": 5.0, "overwrite": true })).await;
    assert_eq!(summary["can_undo"], true);
    let sample = call(&client, "field_sample", json!({ "points": [[10.0, 10.0]], "steps": [0] })).await;
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
    app.listen("view://focus", move |event| { let _ = focus_tx.send(event.payload().to_owned()); });
    app.listen("view://step", move |event| { let _ = tx.send(event.payload().to_owned()); });
    call(&client, "view_focus", json!({ "lon": -70.5, "lat": 41.0, "px_per_deg": 12.0 })).await;
    call(&client, "step_set", json!({ "step": 2 })).await;
    let focus = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("focus event");
    assert!(focus.contains("-70.5"), "{focus}");
    let step = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("step event");
    assert_eq!(step.trim(), "2");
    let message = call_err(&client, "step_set", json!({ "step": 99 })).await;
    assert!(message.contains("step"), "{message}");
    client.cancel().await.expect("close");
}
```

Check the track's field names in `crates/ve-app/src/animation.rs` (`ObjectTracks.tracks`, `TrackView.property`, `TrackView.keys`) and the property name of position (`grep -n '"position"' crates/ve-core/src`) and match the literals.

- [ ] **Step 2: Run to see them fail**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp keyframes view_tools`
Expected: fail on unknown tool.

- [ ] **Step 3: Parameter types**

Add to `tools.rs`:

```rust
#[derive(Deserialize, JsonSchema)]
pub struct KeyframeSetParams { pub object: u64, pub property: String, pub step: u32, /// The value, or null to key the current interpolated value.
    pub value: Option<Value> }
#[derive(Deserialize, JsonSchema)] pub struct KeyframeParams { pub object: u64, pub property: String, pub step: u32 }
#[derive(Deserialize, JsonSchema)] pub struct KeyframeMoveParams { pub object: u64, pub property: String, pub from: u32, pub to: u32 }
#[derive(Deserialize, JsonSchema)]
pub struct InterpolationParams { pub object: u64, pub property: String, pub step: u32, /// "linear", "hold", "ease_in", "ease_out", "ease_in_out" — the names in object_tracks.
    pub interpolation: Value }
#[derive(Deserialize, JsonSchema)]
pub struct MotionParams { pub object: u64, pub step: u32, /// True bearing toward, degrees clockwise from north.
    pub direction: f64, pub speed_mps: f64, #[serde(default)] pub overwrite: bool }
#[derive(Deserialize, JsonSchema)]
pub struct FollowParams { pub object: u64, pub property: String, /// The object to follow, or null to stop following.
    pub primary: Option<u64>, pub step: u32 }
#[derive(Deserialize, JsonSchema)]
pub struct TimelineParams { pub step_count: Option<u32>, /// Unix seconds, or null for no start time.
    pub start_unix_s: Option<Option<i64>> }
#[derive(Deserialize, JsonSchema)]
pub struct SampleParams { /// [lon, lat] pairs.
    pub points: Vec<[f64; 2]>, pub steps: Vec<u32>, /// "wind" or "current"; null samples the composite.
    pub kind: Option<String> }
#[derive(Serialize, JsonSchema)]
pub struct Sample { pub lon: f64, pub lat: f64, pub step: u32, pub speed_mps: f64, pub azimuth_toward_deg: f64, pub u_mps: f64, pub v_mps: f64, pub defined: bool }
#[derive(Serialize, JsonSchema)] pub struct Samples { pub samples: Vec<Sample> }
#[derive(Deserialize, JsonSchema)]
pub struct CaptureParams { /// A region as the interface draws it: {"kind":"rect","west":..,"south":..,"east":..,"north":..} or the polygon form in capture.rs.
    pub region: Value, pub step: u32, pub kind: Option<String> }
#[derive(Deserialize, JsonSchema)]
pub struct PasteParams { pub lon: Option<f64>, pub lat: Option<f64>, pub step: u32, pub layer: Option<u64>, #[serde(default)] pub still: bool }
#[derive(Deserialize, JsonSchema)]
pub struct MacroInsertParams { pub id: String, pub lon: f64, pub lat: f64, pub step: u32, pub layer: Option<u64> }
#[derive(Deserialize, JsonSchema)] pub struct HistoryJumpParams { pub target: usize }
#[derive(Deserialize, JsonSchema)]
pub struct FocusParams { pub lon: f64, pub lat: f64, /// Screen pixels per degree (3 is the whole world, 60 is a bay). Null keeps the zoom.
    pub px_per_deg: Option<f64> }
#[derive(Deserialize, JsonSchema)] pub struct SelectionParams { pub objects: Vec<u64> }
#[derive(Serialize, JsonSchema)] pub struct Done { pub ok: bool }
```

- [ ] **Step 4: The tools**

Add to the `#[tool_router] impl VectorEffects` block:

```rust
    // ------------------------------------------------------------------ time

    #[tool(description = "Sets a keyframe on an animated property at a step. value null keys the value the property has there now.")]
    async fn keyframe_set(&self, Parameters(p): Parameters<KeyframeSetParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        let value = p.value.map(serde_json::from_value::<crate::document::PropertyValue>).transpose().map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        self.write("keyframe_set", false, move |app| crate::animation::set_keyframe(app.state(), p.object, p.property, p.step, value)).await.map(Json)
    }

    #[tool(description = "Removes the keyframe at a step.")]
    async fn keyframe_remove(&self, Parameters(p): Parameters<KeyframeParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("keyframe_remove", false, move |app| crate::animation::remove_keyframe(app.state(), p.object, p.property, p.step)).await.map(Json)
    }

    #[tool(description = "Moves a keyframe from one step to another.")]
    async fn keyframe_move(&self, Parameters(p): Parameters<KeyframeMoveParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("keyframe_move", false, move |app| {
            let summary = crate::animation::move_keyframe(app.state(), p.object, p.property, p.from, p.to, None)?;
            crate::document::end_gesture(app.state())?;
            Ok(summary)
        }).await.map(Json)
    }

    #[tool(description = "Sets how a property interpolates out of the keyframe at a step.")]
    async fn interpolation_set(&self, Parameters(p): Parameters<InterpolationParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        let interp: crate::animation::InterpolationView = serde_json::from_value(p.interpolation).map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        self.write("interpolation_set", false, move |app| crate::animation::set_interpolation(app.state(), p.object, p.property, p.step, interp)).await.map(Json)
    }

    #[tool(description = "Constant motion from a step to the next position key: a rhumb line at a bearing and speed. Existing keys in between need overwrite true.")]
    async fn motion_add(&self, Parameters(p): Parameters<MotionParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("motion_add", false, move |app| crate::animation::add_constant_motion(app.state(), p.object, p.step, p.direction, p.speed_mps, p.overwrite)).await.map(Json)
    }

    #[tool(description = "Links a property to another object's, or unlinks it with primary null.")]
    async fn follow_set(&self, Parameters(p): Parameters<FollowParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("follow_set", false, move |app| crate::animation::set_follow(app.state(), p.object, p.property, p.primary, p.step)).await.map(Json)
    }

    #[tool(description = "Changes the step count and/or the start time (unix seconds). Shrinking drops keys beyond the end; read step_count_impact through invoke first if that matters.")]
    async fn timeline_set(&self, Parameters(p): Parameters<TimelineParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("timeline_set", false, move |app| {
            let mut last = None;
            if let Some(count) = p.step_count { last = Some(crate::animation::set_step_count(app.state(), count)?); }
            if let Some(start) = p.start_unix_s { last = Some(crate::animation::set_start_time(app.state(), start)?); }
            last.ok_or_else(|| AppError::Invalid("nothing to set".to_owned()))
        }).await.map(Json)
    }

    #[tool(description = "An object's animated properties with their keyframes and interpolation.")]
    async fn object_tracks(&self, Parameters(p): Parameters<ObjectStepParams>) -> std::result::Result<Json<crate::animation::ObjectTracks>, McpError> {
        self.run("object_tracks", move |app| crate::animation::object_tracks(app.state(), p.object, p.step)).await.map(Json)
    }

    // ----------------------------------------------------------------- field

    #[tool(description = "The evaluated field at points and steps: speed m/s, azimuth toward (degrees clockwise from north), u east, v north, and whether the cell is defined.")]
    async fn field_sample(&self, Parameters(p): Parameters<SampleParams>) -> std::result::Result<Json<Samples>, McpError> {
        let samples = self.run("field_sample", move |app| {
            let mut out = Vec::with_capacity(p.points.len() * p.steps.len());
            for &step in &p.steps {
                for &[lon, lat] in &p.points {
                    let s = crate::commands::sample_field(app.state(), lon, lat, step, p.kind.clone())?;
                    let az = s.azimuth_toward_deg.to_radians();
                    out.push(Sample { lon, lat, step, speed_mps: s.speed_mps, azimuth_toward_deg: s.azimuth_toward_deg, u_mps: s.speed_mps * az.sin(), v_mps: s.speed_mps * az.cos(), defined: s.defined });
                }
            }
            Ok(out)
        }).await?;
        Ok(Json(Samples { samples }))
    }

    #[tool(description = "Captures the field inside a region at a step onto the clipboard, for field_paste. Drops any copied objects.")]
    async fn field_capture(&self, Parameters(p): Parameters<CaptureParams>) -> std::result::Result<Json<crate::capture::CaptureState>, McpError> {
        let region: crate::capture::RegionShape = serde_json::from_value(p.region).map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        self.run("field_capture", move |app| crate::capture::capture_region(app.state(), region, p.step, p.kind)).await.map(Json)
    }

    #[tool(description = "Pastes the captured field at lon/lat (or where it was captured), from a step, onto a layer. still true pastes one frame.")]
    async fn field_paste(&self, Parameters(p): Parameters<PasteParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("field_paste", false, move |app| crate::capture::paste_capture(app.state(), p.lon, p.lat, p.step, p.layer, p.still)).await.map(Json)
    }

    #[tool(description = "The macro library: id, name, kind, frames, footprint.")]
    async fn macro_list(&self) -> std::result::Result<Json<crate::macros::MacroLibrary>, McpError> {
        self.run("macro_list", |app| crate::macros::macro_library(app.state())).await.map(Json)
    }

    #[tool(description = "Inserts a macro from the library at lon/lat starting at a step.")]
    async fn macro_insert(&self, Parameters(p): Parameters<MacroInsertParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("macro_insert", false, move |app| crate::macros::insert_macro(app.state(), p.id, p.lon, p.lat, p.step, p.layer)).await.map(Json)
    }

    // --------------------------------------------------------------- history

    #[tool(description = "Undoes the last edit.")]
    async fn undo(&self) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("undo", false, |app| crate::edit::undo(app.state())).await.map(Json)
    }

    #[tool(description = "Redoes the last undone edit.")]
    async fn redo(&self) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("redo", false, |app| crate::edit::redo(app.state())).await.map(Json)
    }

    #[tool(description = "The history list with the current position.")]
    async fn history_list(&self) -> std::result::Result<Json<crate::document::HistoryView>, McpError> {
        self.run("history_list", |app| crate::document::history_view(app.state())).await.map(Json)
    }

    #[tool(description = "Jumps to an entry of history_list.")]
    async fn history_jump(&self, Parameters(p): Parameters<HistoryJumpParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("history_jump", false, move |app| crate::document::jump_to_history(app.state(), p.target)).await.map(Json)
    }

    // ------------------------------------------------------------------ view

    #[tool(description = "Pans the map to lon/lat, optionally at a zoom.")]
    async fn view_focus(&self, Parameters(p): Parameters<FocusParams>) -> std::result::Result<Json<Done>, McpError> {
        ve_core::LonLat::new(p.lon, p.lat).map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        self.app.state::<super::McpService>().note_tool(&self.app, "view_focus");
        let _ = tauri::Emitter::emit(&self.app, events::FOCUS, events::ViewFocus { lon: p.lon, lat: p.lat, px_per_deg: p.px_per_deg });
        Ok(Json(Done { ok: true }))
    }

    #[tool(description = "Moves the timeline to a step.")]
    async fn step_set(&self, Parameters(p): Parameters<StepParams>) -> std::result::Result<Json<Done>, McpError> {
        let step = p.step;
        self.run("step_set", move |app| {
            let summary = crate::projects::current_project(app.state())?.ok_or_else(|| AppError::Invalid("no project is open".to_owned()))?;
            if step >= summary.step_count {
                return Err(AppError::Invalid(format!("step {step} is past the last step {}", summary.step_count - 1)));
            }
            let _ = tauri::Emitter::emit(app, events::STEP, step);
            Ok(())
        }).await?;
        Ok(Json(Done { ok: true }))
    }

    #[tool(description = "Selects objects in the interface; an empty list clears the selection.")]
    async fn selection_set(&self, Parameters(p): Parameters<SelectionParams>) -> std::result::Result<Json<Done>, McpError> {
        self.app.state::<super::McpService>().note_tool(&self.app, "selection_set");
        let _ = tauri::Emitter::emit(&self.app, events::SELECTION, p.objects);
        Ok(Json(Done { ok: true }))
    }
```

`ve_core::LonLat::new` returns an error type: check its name (`commands.rs` line 138 uses `?` into `AppError`, so `.map_err(AppError::from)` works too). `CaptureState`, `MacroLibrary`, `HistoryView`, `ObjectTracks` need `JsonSchema`; wrap as `Json<Value>` if a nested type resists.

- [ ] **Step 5: Run the tests, clippy, commit**

```bash
VE_FORCE_CPU=1 cargo test -p ve-app --test mcp
cargo clippy -p ve-app --all-targets -- -D warnings && cargo fmt --all
git add crates/ve-app && git commit -m "Add MCP time, field, history and view tools"
```

---

### Task 5: Files: import, export with progress, and the screenshot

**Files:**
- Create: `crates/ve-app/src/mcp/capture.rs`
- Modify: `crates/ve-app/src/mcp/tools.rs`, `crates/ve-app/src/mcp/mod.rs`, `crates/ve-app/src/lib.rs`
- Test: `crates/ve-app/tests/mcp.rs`

**Interfaces:**
- Consumes: `export::{export_grib, export_zarr, cancel_export, ExportRequest, ExportZarrRequest, ExportProgress}`, `import::import_grib`, `image::import_image`, `history::import_history`.
- Produces: `McpService::request_capture(&self, app) -> (u64, oneshot::Receiver<Vec<u8>>)`, command `deliver_capture(id: u64, png_base64: String)`, `events::CAPTURE = "view://capture"`, `events::CaptureRequest { id }`, tools `import_grib`, `import_image`, `import_history`, `export_grib`, `export_zarr`, `export_cancel`, `screenshot`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/ve-app/tests/mcp.rs`:

```rust
#[tokio::test]
async fn export_grib_writes_a_file_and_reports_progress() {
    let root = TempRoot::new("export");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("Export")).await;
    call(&client, "object_create", json!({ "tool": "circle", "gesture": { "kind": "point", "at": [0.0, 0.0] } })).await;
    let out = root.0.join("out.grib2");
    let result = call(&client, "export_grib", json!({ "path": out.to_string_lossy(), "year": 2026, "month": 9, "day": 16, "hour": 0 })).await;
    assert!(result["bytes"].as_u64().unwrap_or(0) > 0, "{result}");
    assert!(out.exists());
    client.cancel().await.expect("close");
}

#[tokio::test]
async fn a_screenshot_waits_for_the_frontend_and_returns_its_png() {
    use tauri::Listener;
    let root = TempRoot::new("shot");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("Shot")).await;
    // Stand in for MapView: answer the capture request with a 1x1 PNG.
    let handle = app.handle().clone();
    app.listen("view://capture", move |event| {
        let id: u64 = serde_json::from_str::<Value>(event.payload()).expect("json")["id"].as_u64().expect("id");
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        ve_app::mcp::capture::deliver_capture(handle.state(), id, png.to_owned()).expect("deliver");
    });
    let result: CallToolResult = client
        .call_tool(CallToolRequestParam { name: "screenshot".into(), arguments: None })
        .await
        .expect("call");
    let image = result.content.iter().find_map(|c| c.as_image()).expect("an image");
    assert_eq!(image.mime_type, "image/png");
    assert!(image.data.starts_with("iVBOR"));
    client.cancel().await.expect("close");
}
```

Check `ExportResult`'s size field name in `crates/ve-app/src/export.rs` and `Content::as_image` in `rmcp::model` (it is `as_image() -> Option<&RawImageContent>` with `data` and `mime_type`).

- [ ] **Step 2: Run to see them fail**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp export_grib a_screenshot`

- [ ] **Step 3: The capture rendezvous**

Create `crates/ve-app/src/mcp/capture.rs`:

```rust
//! The screenshot's round trip: the service asks, `MapView` reads its
//! framebuffer, and `deliver_capture` brings the bytes back.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use base64::Engine;
use serde::Serialize;
use tauri::Emitter;
use ts_rs::TS;

use crate::error::{AppError, Result};

/// The event `MapView` answers.
pub const CAPTURE: &str = "view://capture";

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "CaptureRequest.ts")]
pub struct CaptureRequest {
    pub id: u64,
}

/// Outstanding requests, keyed by id.
#[derive(Default)]
pub struct Captures {
    next: AtomicU64,
    pending: Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Vec<u8>>>>,
}

impl Captures {
    /// Asks the frontend for a picture. The receiver yields PNG bytes.
    pub fn request(&self, app: &tauri::AppHandle) -> (u64, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, tx);
        }
        let _ = app.emit(CAPTURE, CaptureRequest { id });
        (id, rx)
    }

    /// Forgets a request the tool stopped waiting for.
    pub fn forget(&self, id: u64) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&id);
        }
    }

    fn deliver(&self, id: u64, png: Vec<u8>) -> Result<()> {
        let sender = self.pending.lock().ok().and_then(|mut p| p.remove(&id));
        match sender {
            Some(tx) => {
                let _ = tx.send(png);
                Ok(())
            }
            None => Err(AppError::Invalid(format!("no capture {id} is pending"))),
        }
    }
}

/// The frontend's answer to `view://capture`.
#[tauri::command]
pub fn deliver_capture(
    service: tauri::State<'_, super::McpService>,
    id: u64,
    png_base64: String,
) -> Result<()> {
    let png = base64::engine::general_purpose::STANDARD
        .decode(png_base64.trim())
        .map_err(|e| AppError::Invalid(format!("capture {id} is not base64: {e}")))?;
    service.captures.deliver(id, png)
}
```

Add `pub captures: capture::Captures,` to `McpService` (it derives `Default`), `pub mod capture;` to `mcp/mod.rs`, `mcp::capture::deliver_capture,` to `generate_handler!`, and `CaptureRequest` to the bindings example.

- [ ] **Step 4: The file tools**

Parameter types in `tools.rs`:

```rust
#[derive(Deserialize, JsonSchema)] pub struct PathParams { pub path: String }
#[derive(Deserialize, JsonSchema)]
pub struct ImageParams { pub path: String, /// [west, south, east, north] to place an image with no georeference of its own; null uses the file's.
    pub view: Option<[f64; 4]> }
#[derive(Deserialize, JsonSchema)]
pub struct HistoryParams { /// Archive names as the History panel lists them: "era5", "globcurrent".
    pub archives: Vec<String>, pub start_unix_s: i64, pub end_unix_s: i64, #[serde(default)] pub set_start_time: bool }
#[derive(Deserialize, JsonSchema)]
pub struct ExportParams { pub path: String, /// Reference time, UTC.
    pub year: i32, pub month: u32, pub day: u32, pub hour: u32 }
```

Check `ExportRequest`'s field types in `export.rs` (year `i32` or `u32`) and match them.

Tools, in the `#[tool_router]` block:

```rust
    // ----------------------------------------------------------------- files

    #[tool(description = "Imports a GRIB2 file as a new layer of the open project. Slow for large files; progress is reported.")]
    async fn import_grib(&self, Parameters(p): Parameters<PathParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("import_grib", false, move |app| crate::import::import_grib(app.state(), p.path)).await.map(Json)
    }

    #[tool(description = "Adds an image layer (PNG, JPEG, GeoTIFF). Display only; never exported.")]
    async fn import_image(&self, Parameters(p): Parameters<ImageParams>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        self.write("import_image", false, move |app| crate::image::import_image(app.state(), p.path, p.view)).await.map(Json)
    }

    #[tool(description = "Downloads historical wind or current (ERA5, GlobCurrent) over HTTPS into a layer. The one tool that reaches the network, and only when called.")]
    async fn import_history(&self, Parameters(p): Parameters<HistoryParams>, ctx: rmcp::service::RequestContext<rmcp::RoleServer>) -> std::result::Result<Json<ProjectSummary>, McpError> {
        let relay = self.relay_progress::<crate::history::HistoryProgress>("history://progress", ctx, |h| (h.done as f64, Some(h.total as f64), h.message.clone()));
        let out = self.write("import_history", false, move |app| crate::history::import_history(app.clone(), p.archives, p.start_unix_s, p.end_unix_s, p.set_start_time)).await;
        relay.stop();
        out.map(Json)
    }

    #[tool(description = "Exports the project as GRIB2 to path. Refuses an existing file. Progress is reported; export_cancel stops it.")]
    async fn export_grib(&self, Parameters(p): Parameters<ExportParams>, ctx: rmcp::service::RequestContext<rmcp::RoleServer>) -> std::result::Result<Json<crate::export::ExportResult>, McpError> {
        let request = crate::export::ExportRequest { path: p.path, year: p.year, month: p.month, day: p.day, hour: p.hour };
        let relay = self.relay_progress::<crate::export::ExportProgress>("export://progress", ctx, |e| (e.done as f64, Some(e.total as f64), None));
        let out = self.run("export_grib", move |app| crate::export::export_grib(app.clone(), app.state(), app.state(), request)).await;
        relay.stop();
        out.map(Json)
    }

    #[tool(description = "Exports the project as a Zarr V3 directory at path (Float16, 72 h by 10° chunks, Zstd, NaN where uncovered). Refuses an existing directory.")]
    async fn export_zarr(&self, Parameters(p): Parameters<ExportParams>, ctx: rmcp::service::RequestContext<rmcp::RoleServer>) -> std::result::Result<Json<crate::export::ExportZarrResult>, McpError> {
        let request = crate::export::ExportZarrRequest { path: p.path, year: p.year, month: p.month, day: p.day, hour: p.hour };
        let relay = self.relay_progress::<crate::export::ExportProgress>("export://progress", ctx, |e| (e.done as f64, Some(e.total as f64), None));
        let out = self.run("export_zarr", move |app| crate::export::export_zarr(app.clone(), app.state(), app.state(), request)).await;
        relay.stop();
        out.map(Json)
    }

    #[tool(description = "Asks a running export to stop.")]
    async fn export_cancel(&self) -> std::result::Result<Json<Done>, McpError> {
        self.run("export_cancel", |app| { crate::export::cancel_export(app.state()); Ok(()) }).await?;
        Ok(Json(Done { ok: true }))
    }

    #[tool(description = "The map as the interface shows it, once its tiles have settled: a PNG. Needs an open project.")]
    async fn screenshot(&self) -> std::result::Result<rmcp::model::CallToolResult, McpError> {
        let service = self.app.state::<super::McpService>();
        service.note_tool(&self.app, "screenshot");
        let (id, rx) = service.captures.request(&self.app);
        // The frontend waits up to 10 s for tiles and 20 s for a frame (M78);
        // a little longer than both, then give up rather than hang the client.
        match tokio::time::timeout(std::time::Duration::from_secs(35), rx).await {
            Ok(Ok(png)) => {
                let data = base64::engine::general_purpose::STANDARD.encode(png);
                Ok(rmcp::model::CallToolResult::success(vec![rmcp::model::Content::image(data, "image/png")]))
            }
            _ => {
                service.captures.forget(id);
                Err(McpError::internal_error("the map did not answer the capture: is a project open and the window shown?", None))
            }
        }
    }
```

Check `ExportProgress` and `HistoryProgress` field names (`done`/`total` or `written`/`messages`) in `export.rs` line 93 and `history.rs`, and map them in the closures.

The progress relay, in `impl VectorEffects` (outside the router block):

```rust
    /// Forwards a Tauri progress event to the client's progress token for
    /// the life of one tool call. The command emits as it always did; the
    /// interface's own bar and the client's both see it.
    pub(crate) fn relay_progress<P: serde::de::DeserializeOwned + Send + 'static>(
        &self,
        event: &'static str,
        ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
        map: impl Fn(&P) -> (f64, Option<f64>, Option<String>) + Send + Sync + 'static,
    ) -> ProgressRelay {
        use tauri::Listener;
        let Some(token) = ctx.meta.get_progress_token() else {
            return ProgressRelay { app: self.app.clone(), id: None };
        };
        let peer = ctx.peer.clone();
        let id = self.app.listen(event, move |e| {
            if let Ok(payload) = serde_json::from_str::<P>(e.payload()) {
                let (progress, total, message) = map(&payload);
                let peer = peer.clone();
                let token = token.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = peer.notify_progress(rmcp::model::ProgressNotificationParam { progress_token: token, progress, total, message }).await;
                });
            }
        });
        ProgressRelay { app: self.app.clone(), id: Some(id) }
    }
```

with:

```rust
/// Stops the relay when the call ends.
pub(crate) struct ProgressRelay {
    app: tauri::AppHandle,
    id: Option<tauri::EventId>,
}

impl ProgressRelay {
    pub(crate) fn stop(self) {
        use tauri::Listener;
        if let Some(id) = self.id {
            self.app.unlisten(id);
        }
    }
}
```

`ProgressNotificationParam`'s field names and `get_progress_token`'s exact accessor are per `rmcp` 3.4 docs; the concept is fixed, the spelling follows the crate.

- [ ] **Step 5: Run, clippy, bindings, commit**

```bash
VE_FORCE_CPU=1 cargo test -p ve-app --test mcp
cargo clippy -p ve-app --all-targets -- -D warnings && cargo fmt --all
npm run bindings
git add crates/ve-app ui/src/generated && git commit -m "Add MCP file tools, progress relay and screenshot round trip"
```

---

### Task 6: The `invoke` escape hatch and its coverage test

**Files:**
- Create: `crates/ve-app/src/mcp/invoke.rs`
- Modify: `crates/ve-app/src/mcp/tools.rs`, `crates/ve-app/src/mcp/mod.rs`
- Test: `crates/ve-app/src/mcp/invoke.rs` (unit), `crates/ve-app/tests/mcp.rs`

**Interfaces:**
- Produces: `invoke::TABLE: &[(&str, Handler)]`, `invoke::EXCLUDED: &[&str]`, `invoke::call(app, name, args) -> Result<Value>`, tool `invoke`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/mcp.rs`:

```rust
#[tokio::test]
async fn invoke_reaches_a_command_no_curated_tool_covers() {
    let root = TempRoot::new("invoke");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("Invoke")).await;
    let renamed = call(&client, "invoke", json!({ "command": "rename_project", "args": { "name": "Renamed" } })).await;
    assert_eq!(renamed["result"]["name"], "Renamed");
    let current = ve_app::projects::current(app.state::<AppState>().inner()).expect("current").expect("open");
    assert_eq!(current.name, "Renamed");
    let message = call_err(&client, "invoke", json!({ "command": "basemap", "args": {} })).await;
    assert!(message.contains("not available"), "{message}");
    let message = call_err(&client, "invoke", json!({ "command": "no_such_command", "args": {} })).await;
    assert!(message.contains("unknown command"), "{message}");
    client.cancel().await.expect("close");
}
```

And the unit test that keeps the table complete, at the bottom of the new `invoke.rs` (write it first; it fails until the table is full):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Every command in `generate_handler!` is either in the table or
    /// deliberately excluded, so a command added later is reachable the day
    /// it lands or someone has said why not.
    #[test]
    fn the_table_covers_every_registered_command() {
        let lib = include_str!("../lib.rs");
        let start = lib.find("generate_handler![").expect("handler list");
        let end = lib[start..].find("];").expect("end of list") + start;
        let names: Vec<&str> = lib[start..end]
            .split(',')
            .filter_map(|entry| entry.trim().rsplit("::").next())
            .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .collect();
        assert!(names.len() > 100, "parsed only {} names", names.len());
        let missing: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| !EXCLUDED.contains(n) && !TABLE.iter().any(|(t, _)| t == n))
            .collect();
        assert!(missing.is_empty(), "commands neither in TABLE nor EXCLUDED: {missing:?}");
        let stale: Vec<&str> = TABLE.iter().map(|(t, _)| *t).chain(EXCLUDED.iter().copied()).filter(|t| !names.contains(t)).collect();
        assert!(stale.is_empty(), "table names no command has: {stale:?}");
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p ve-app --lib mcp::invoke` (compile error until the module exists) and `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp invoke_reaches`.

- [ ] **Step 3: The table**

Create `crates/ve-app/src/mcp/invoke.rs`:

```rust
//! `invoke`: any IPC command by name, for what no curated tool covers.
//!
//! One entry per command, calling the command function with the state the
//! handle gives. The test at the bottom holds the table equal to
//! `generate_handler!` minus `EXCLUDED`.

use serde_json::Value;
use tauri::Manager;

use crate::commands::AppState;
use crate::error::{AppError, Result};

type Handler = fn(&tauri::AppHandle, Value) -> Result<Value>;

fn bad(name: &str, e: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("{name}: {e}"))
}

/// Builds one entry: the command name, a struct of its arguments, the call.
macro_rules! command {
    ($name:ident, $path:path, { $($field:ident : $ty:ty),* $(,)? }) => {
        (stringify!($name), |app: &tauri::AppHandle, args: Value| -> Result<Value> {
            #[derive(serde::Deserialize)]
            struct Args { $($field: $ty),* }
            let Args { $($field),* } = serde_json::from_value(args).map_err(|e| bad(stringify!($name), e))?;
            let out = $path(app.state::<AppState>(), $($field),*)?;
            serde_json::to_value(out).map_err(|e| bad(stringify!($name), e))
        })
    };
}

/// Commands the escape hatch will not run: the interface's own plumbing
/// (tiles, logs, the beta gate, the service itself) and the two that take
/// a byte response rather than JSON.
pub const EXCLUDED: &[&str] = &[
    "beta_status", "app_info", "basemap", "tile_base_url", "selection_preview", "save_debug_capture",
    "frontend_log", "tile_keys", "render_ahead", "frame_readiness", "colour_gradients", "tool_palette",
    "mcp_status", "mcp_set", "mcp_rotate_token", "deliver_capture", "cancel_export", "export_grib",
    "export_zarr", "import_history",
];

/// Every command reachable by name. `cancel_export`, the exports and the
/// history import take a handle or a cancel flag and are curated tools.
pub const TABLE: &[(&str, Handler)] = &[
    command!(new_project, crate::projects::new_project, { request: crate::projects::NewProjectRequest, discard_unsaved: bool }),
    command!(open_project, crate::projects::open_project, { path: String, discard_unsaved: bool }),
    command!(save_project, crate::projects::save_project, {}),
    command!(save_project_as, crate::projects::save_project_as, { path: String }),
    command!(close_project, crate::projects::close_project, { discard_unsaved: bool }),
    command!(current_project, crate::projects::current_project, {}),
    command!(recent_projects, crate::projects::recent_projects, {}),
    command!(clear_recent_projects, crate::projects::clear_recent_projects, {}),
    command!(sample_field, crate::commands::sample_field, { lon: f64, lat: f64, step: u32, kind: Option<String> }),
    command!(create_object, crate::create::create_object, { object: crate::create::NewObject }),
    command!(add_brush_stroke, crate::edit::add_brush_stroke, { stroke: crate::edit::BrushStroke }),
    command!(undo, crate::edit::undo, {}),
    command!(redo, crate::edit::redo, {}),
    command!(export_estimate, crate::export::export_estimate, {}),
    command!(import_grib, crate::import::import_grib, { path: String }),
    command!(new_project_from_grib, crate::import::new_project_from_grib, { path: String, discard_unsaved: bool }),
    command!(document_tree, crate::document::document_tree, { step: u32 }),
    command!(object_properties, crate::document::object_properties, { object: u64, step: u32 }),
    command!(set_object_property, crate::document::set_object_property, { object: u64, property: String, value: crate::document::PropertyValue, step: u32, auto_key: bool, gesture: Option<String> }),
    command!(rename_project, crate::document::rename_project, { name: String }),
    command!(add_layer, crate::document::add_layer, { name: String }),
    command!(remove_layer, crate::document::remove_layer, { layer: u64 }),
    command!(rename_layer, crate::document::rename_layer, { layer: u64, name: String }),
    command!(set_layer_visible, crate::document::set_layer_visible, { layer: u64, visible: bool }),
    command!(set_layer_parameter, crate::document::set_layer_parameter, { layer: u64, parameter: String }),
    command!(set_layer_speed_range, crate::document::set_layer_speed_range, { layer: u64, min_mps: Option<f32>, max_mps: Option<f32>, gesture: Option<String> }),
    command!(set_layer_locked, crate::document::set_layer_locked, { layer: u64, locked: bool }),
    command!(move_layer, crate::document::move_layer, { from: usize, to: usize }),
    command!(rename_object, crate::document::rename_object, { object: u64, name: String }),
    command!(remove_object, crate::document::remove_object, { object: u64 }),
    command!(remove_objects, crate::document::remove_objects, { objects: Vec<u64> }),
    command!(move_object, crate::document::move_object, { object: u64, layer: u64, index: usize }),
    command!(duplicate_object, crate::document::duplicate_object, { object: u64 }),
    command!(set_active_range, crate::document::set_active_range, { object: u64, start: u32, end: u32 }),
    command!(object_at, crate::document::object_at, { lon: f64, lat: f64, step: u32 }),
    command!(selection_transform, crate::transform::selection_transform, { objects: Vec<u64>, step: u32 }),
    command!(begin_transform, crate::transform::begin_transform, { objects: Vec<u64>, step: u32, kind: crate::transform::TransformKind, lon: f64, lat: f64, auto_key: Option<bool> }),
    command!(preview_transform, crate::transform::preview_transform, { lon: f64, lat: f64 }),
    command!(drag_transform, crate::transform::drag_transform, { lon: f64, lat: f64 }),
    command!(objects_in_region, crate::transform::objects_in_region, { west: f64, south: f64, east: f64, north: f64, step: u32, layer: Option<u64> }),
    command!(object_outlines, crate::transform::object_outlines, { step: u32, tool: Option<crate::create::Tool>, objects: Vec<u64>, layer: Option<u64>, all_in_layer: bool }),
    command!(end_gesture, crate::document::end_gesture, {}),
    command!(copy_objects, crate::document::copy_objects, { objects: Vec<u64>, step: u32 }),
    command!(cut_objects, crate::document::cut_objects, { objects: Vec<u64>, step: u32 }),
    command!(paste_objects, crate::document::paste_objects, { layer: Option<u64>, step: u32, absolute_timing: bool, still: bool }),
    command!(erase_stroke, crate::document::erase_stroke, { stroke: crate::document::EraseStroke }),
    command!(clipboard_state, crate::document::clipboard_state, {}),
    command!(clipboard_kind, crate::document::clipboard_kind, {}),
    command!(history_view, crate::document::history_view, {}),
    command!(jump_to_history, crate::document::jump_to_history, { target: usize }),
    command!(app_settings, crate::settings::app_settings, {}),
    command!(set_shortcut, crate::settings::set_shortcut, { binding: crate::settings::Shortcut }),
    command!(reset_shortcuts, crate::settings::reset_shortcuts, {}),
    command!(set_default_scales, crate::settings::set_default_scales, { wind_knots: f64, current_knots: f64 }),
    command!(set_macro_directory, crate::settings::set_macro_directory, { directory: String }),
    command!(set_autosave_mode, crate::settings::set_autosave_mode, { mode: crate::settings::AutosaveMode }),
    command!(set_projection, crate::settings::set_projection, { projection: String }),
    command!(set_auto_scale, crate::settings::set_auto_scale, { on: bool }),
    command!(set_display_units, crate::settings::set_display_units, { distance_unit: crate::settings::DistanceUnit, speed_unit: crate::settings::SpeedUnit }),
    command!(set_glyph_appearance, crate::settings::set_glyph_appearance, { style: crate::settings::GlyphStyle, setting: crate::settings::GlyphSetting }),
    command!(set_colour_scale, crate::settings::set_colour_scale, { kind: String, max_knots: f64 }),
    command!(set_colour_gradient, crate::settings::set_colour_gradient, { kind: String, gradient: String }),
    command!(autosaves, crate::autosave::autosaves, {}),
    command!(recover_autosave, crate::autosave::recover_autosave, { id: u64, discard_unsaved: bool }),
    command!(discard_autosave, crate::autosave::discard_autosave, { id: u64 }),
    command!(import_image, crate::image::import_image, { path: String, view: Option<[f64; 4]> }),
    command!(set_image_corners, crate::image::set_image_corners, { layer: u64, top_left: [f64; 2], top_right: [f64; 2], bottom_left: [f64; 2], gesture: Option<String> }),
    command!(set_image_opacity, crate::image::set_image_opacity, { layer: u64, opacity: f64 }),
    command!(reset_image_placement, crate::image::reset_image_placement, { layer: u64 }),
    command!(measurements, crate::measure::measurements, {}),
    command!(add_measurement, crate::measure::add_measurement, { measurement: crate::measure::NewMeasurement }),
    command!(preview_measurement, crate::measure::preview_measurement, { measurement: crate::measure::NewMeasurement }),
    command!(move_measurement_handle, crate::measure::move_measurement_handle, { id: u64, index: usize, at: [f64; 2] }),
    command!(extend_measurement, crate::measure::extend_measurement, { id: u64, at: [f64; 2] }),
    command!(set_measurement_rings, crate::measure::set_measurement_rings, { id: u64, interval_km: f64, count: u32 }),
    command!(remove_measurement, crate::measure::remove_measurement, { id: u64 }),
    command!(clear_measurements, crate::measure::clear_measurements, { kind: Option<crate::measure::MeasurementKind> }),
    command!(macro_library, crate::macros::macro_library, {}),
    command!(delete_macros, crate::macros::delete_macros, { id: Option<String> }),
    command!(start_capture, crate::macros::start_capture, { region: crate::capture::RegionShape, step: u32, record_movement: bool, kind: Option<String> }),
    command!(place_capture, crate::macros::place_capture, { step: u32, lon: f64, lat: f64 }),
    command!(visit_capture, crate::macros::visit_capture, { step: u32 }),
    command!(unplace_capture, crate::macros::unplace_capture, { step: u32 }),
    command!(preview_capture, crate::macros::preview_capture, { last_step: u32 }),
    command!(place_preview, crate::macros::place_preview, { lon: f64, lat: f64 }),
    command!(edit_capture, crate::macros::edit_capture, {}),
    command!(cancel_capture, crate::macros::cancel_capture, {}),
    command!(capture_mode, crate::macros::capture_mode, { step: Option<u32> }),
    command!(finish_capture, crate::macros::finish_capture, { name: String, last_step: u32 }),
    command!(insert_macro, crate::macros::insert_macro, { id: String, lon: f64, lat: f64, step: u32, layer: Option<u64> }),
    command!(capture_region, crate::capture::capture_region, { region: crate::capture::RegionShape, step: u32, kind: Option<String> }),
    command!(paste_capture, crate::capture::paste_capture, { lon: Option<f64>, lat: Option<f64>, step: u32, layer: Option<u64>, still: bool }),
    command!(capture_state, crate::capture::capture_state, {}),
    command!(copy_grib_frames, crate::frames::copy_grib_frames, { layer: u64, steps: Vec<u32> }),
    command!(paste_grib_frames, crate::frames::paste_grib_frames, { at: u32 }),
    command!(delete_grib_frames, crate::frames::delete_grib_frames, { layer: u64, steps: Vec<u32> }),
    command!(frame_clipboard_state, crate::frames::frame_clipboard_state, {}),
    command!(shape_controls, crate::shape_animation::shape_controls, { object: u64, step: u32 }),
    command!(move_shape_point, crate::shape_animation::move_shape_point, { object: u64, step: u32, ring: usize, point: usize, lon: f64, lat: f64, revision: u64 }),
    command!(object_tracks, crate::animation::object_tracks, { object: u64, step: u32 }),
    command!(track_samples, crate::animation::track_samples, { object: u64, property: String }),
    command!(set_keyframe, crate::animation::set_keyframe, { object: u64, property: String, step: u32, value: Option<crate::document::PropertyValue> }),
    command!(add_constant_motion, crate::animation::add_constant_motion, { object: u64, step: u32, direction: f64, speed_mps: f64, overwrite: bool }),
    command!(set_motion, crate::animation::set_motion, { object: u64, property: String, on: bool }),
    command!(set_follow, crate::animation::set_follow, { object: u64, property: String, primary: Option<u64>, step: u32 }),
    command!(remove_keyframe, crate::animation::remove_keyframe, { object: u64, property: String, step: u32 }),
    command!(move_keyframe, crate::animation::move_keyframe, { object: u64, property: String, from: u32, to: u32, gesture: Option<String> }),
    command!(set_interpolation, crate::animation::set_interpolation, { object: u64, property: String, step: u32, interp: crate::animation::InterpolationView }),
    command!(step_count_impact, crate::animation::step_count_impact, { step_count: u32 }),
    command!(set_step_count, crate::animation::set_step_count, { step_count: u32 }),
    command!(set_start_time, crate::animation::set_start_time, { start_unix_s: Option<i64> }),
];

/// Runs a command by name.
pub fn call(app: &tauri::AppHandle, name: &str, args: Value) -> Result<Value> {
    if EXCLUDED.contains(&name) {
        return Err(AppError::Invalid(format!("{name} is not available through invoke")));
    }
    match TABLE.iter().find(|(n, _)| *n == name) {
        Some((_, handler)) => handler(app, args),
        None => Err(AppError::Invalid(format!("unknown command {name}"))),
    }
}
```

Every argument type named above must be `pub` and `Deserialize`; the ones the frontend sends already are. The `command!` macro needs the fn to take `tauri::State<'_, AppState>` first and the listed arguments in order; the signatures are in the command dump at the top of this plan's source, `crates/ve-app/src`. A command with no arguments, like `undo`, passes an empty `Args {}`, and `serde_json::from_value::<Args>(json!({}))` succeeds; `Value::Null` does not, so the tool below substitutes `{}` for a missing `args`.

- [ ] **Step 4: The tool**

In `tools.rs`:

```rust
#[derive(Deserialize, JsonSchema)]
pub struct InvokeParams { /// A command name from the application's IPC surface, e.g. "rename_project".
    pub command: String, /// The command's arguments, snake_case as in Rust.
    #[serde(default)] pub args: Option<Value> }
#[derive(Serialize, JsonSchema)] pub struct Invoked { pub result: Value }
```

and in the router block:

```rust
    #[tool(description = "Runs any IPC command by name with JSON arguments — the escape hatch for what no other tool covers, including the transform gesture (begin_transform, drag_transform, end_gesture). Writes are undoable and the interface follows.")]
    async fn invoke(&self, Parameters(p): Parameters<InvokeParams>) -> std::result::Result<Json<Invoked>, McpError> {
        let args = p.args.unwrap_or_else(|| Value::Object(Default::default()));
        let name = p.command.clone();
        let result = self.write("invoke", matches!(name.as_str(), "new_project" | "open_project" | "close_project" | "new_project_from_grib" | "recover_autosave"), move |app| super::invoke::call(app, &name, args)).await?;
        Ok(Json(Invoked { result }))
    }
```

Add `pub mod invoke;` to `mcp/mod.rs`.

- [ ] **Step 5: Run everything, commit**

```bash
cargo test -p ve-app --lib mcp::invoke
VE_FORCE_CPU=1 cargo test -p ve-app --test mcp
cargo clippy -p ve-app --all-targets -- -D warnings && cargo fmt --all
git add crates/ve-app && git commit -m "Add the MCP invoke escape hatch with a coverage test"
```

---

### Task 7: Frontend: IPC entries, the follow reducer, and App wiring

**Files:**
- Modify: `ui/src/ipc.ts` (after `setMacroDirectory`, line ~654; `LONG_RUNNING` at line ~105)
- Create: `ui/src/mcp/follow.ts`, `ui/src/mcp/follow.test.ts`
- Modify: `ui/src/App.tsx` (state at lines 49-178; effects near line 875)
- Modify: `ui/src/hint.ts`

**Interfaces:**
- Consumes: generated `McpStatus`, `McpSettings`, `DocumentChanged`, `ViewFocus`, `McpActivity`, `CaptureRequest`.
- Produces: `api.mcpStatus()`, `api.setMcp(enabled, port)`, `api.rotateMcpToken()`, `api.deliverCapture(id, pngBase64)`, `applyDocumentChanged(payload, actions)`, `hint.setMcpActivity(text)`, `HintSnapshot.mcp`.

- [ ] **Step 1: IPC entries**

In `ui/src/ipc.ts` imports:

```ts
import type { McpStatus } from "./generated/McpStatus";
```

In `api`, after `setMacroDirectory`:

```ts
  /** The MCP service's switch, port, token and live state (spec 8.8). */
  mcpStatus: () => call<McpStatus>("mcp_status", {}),
  /** Turns the service on or off; enabling issues a fresh token. */
  setMcp: (enabled: boolean, port: number) => call<McpStatus>("mcp_set", { enabled, port }),
  /** A new token, and the listener restarted with it. */
  rotateMcpToken: () => call<McpStatus>("mcp_rotate_token", {}),
  /** The map's answer to a `view://capture` request. */
  deliverCapture: (id: number, pngBase64: string) =>
    call<void>("deliver_capture", { id, pngBase64 }),
```

`call` sends camelCase keys; Tauri maps `pngBase64` to `png_base64`.

- [ ] **Step 2: The follow reducer and its test**

Create `ui/src/mcp/follow.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";

import type { ProjectSummary } from "../generated/ProjectSummary";
import { applyDocumentChanged, type FollowActions } from "./follow";

const summary = (name: string, revision: number): ProjectSummary =>
  ({ name, revision, step_count: 4 } as unknown as ProjectSummary);

function actions(): FollowActions & Record<string, ReturnType<typeof vi.fn>> {
  return {
    setProject: vi.fn(),
    setStep: vi.fn(),
    setSelection: vi.fn(),
    setShapeEditing: vi.fn(),
    setActiveLayer: vi.fn(),
  };
}

describe("applyDocumentChanged", () => {
  it("replaces the project on an edit and touches nothing else", () => {
    const a = actions();
    applyDocumentChanged({ project: summary("P", 2), opened: false }, a);
    expect(a.setProject).toHaveBeenCalledWith(summary("P", 2));
    expect(a.setStep).not.toHaveBeenCalled();
    expect(a.setSelection).not.toHaveBeenCalled();
  });

  it("resets the editor state when a different project opens, as the open path does", () => {
    const a = actions();
    applyDocumentChanged({ project: summary("Q", 1), opened: true }, a);
    expect(a.setSelection).toHaveBeenCalledWith([]);
    expect(a.setShapeEditing).toHaveBeenCalledWith(null);
    expect(a.setActiveLayer).toHaveBeenCalledWith(null);
    expect(a.setStep).toHaveBeenCalledWith(0);
    expect(a.setProject).toHaveBeenCalledWith(summary("Q", 1));
  });

  it("clears everything and shows the start screen on null", () => {
    const a = actions();
    applyDocumentChanged({ project: null, opened: true }, a);
    expect(a.setSelection).toHaveBeenCalledWith([]);
    expect(a.setStep).toHaveBeenCalledWith(0);
    expect(a.setProject).toHaveBeenCalledWith(null);
  });
});
```

Create `ui/src/mcp/follow.ts`:

```ts
/**
 * The interface following the MCP service (spec 8.8, M76).
 *
 * The service emits `document://changed` after every write. This applies it
 * the way the app applies the result of its own call: an edit replaces the
 * summary and the panels refresh by revision; a different project resets
 * what `openPath` resets.
 */
import type { DocumentChanged } from "../generated/DocumentChanged";
import type { ProjectSummary } from "../generated/ProjectSummary";

export interface FollowActions {
  setProject: (project: ProjectSummary | null) => void;
  setStep: (step: number) => void;
  setSelection: (objects: number[]) => void;
  setShapeEditing: (object: number | null) => void;
  setActiveLayer: (layer: number | null) => void;
}

export function applyDocumentChanged(payload: DocumentChanged, actions: FollowActions): void {
  if (payload.opened || payload.project === null) {
    actions.setSelection([]);
    actions.setShapeEditing(null);
    actions.setActiveLayer(null);
    actions.setStep(0);
  }
  actions.setProject(payload.project);
}
```

Run: `npx vitest run ui/src/mcp/follow.test.ts` (from the root: `npm run ui:test -- follow`). Expected: 3 passed.

- [ ] **Step 3: The status badge in the hint store**

In `ui/src/hint.ts`, add to `HintSnapshot`:

```ts
  /** What a connected MCP client last did, or null with no client (spec 8.8). */
  mcp: string | null;
```

initialise it `mcp: null` in `snapshot`, include `next.mcp === snapshot.mcp` in `publish`'s equality, and add:

```ts
/** Sets the MCP badge, or clears it. */
export function setMcpActivity(mcp: string | null): void {
  publish({ ...snapshot, mcp });
}
```

Find every literal `HintSnapshot` in `ui/src/hint.test.ts` and other tests and add `mcp: null`. In `App.tsx`'s `StatusHint`, before `activity` is built:

```tsx
  const mcp = state.mcp !== null ? <span className="activity mcp-badge" title="An MCP client is connected">MCP: {state.mcp}</span> : null;
```

and render `{mcp}{mcp && " · "}` immediately before `{activity}` in each of the three returns.

- [ ] **Step 4: Listen in `App.tsx`**

Beside the `history://progress` effect, in `EditorApp`:

```tsx
  // The interface follows the MCP service (spec 8.8). Each event is what the
  // app would have done itself had it made the call.
  useEffect(() => {
    const actions = { setProject, setStep, setSelection, setShapeEditing, setActiveLayer };
    const subs = [
      listen<DocumentChanged>("document://changed", (e) => applyDocumentChanged(e.payload, actions)),
      listen<number>("view://step", (e) => setStep(e.payload)),
      listen<number[]>("view://selection", (e) => {
        mapRef.current?.clearRegion();
        setSelection(e.payload);
      }),
      listen<ViewFocus>("view://focus", (e) =>
        mapRef.current?.focus(e.payload.lon, e.payload.lat, e.payload.px_per_deg ?? undefined),
      ),
      listen<McpActivity>("mcp://activity", (e) =>
        setMcpActivity(e.payload.sessions > 0 ? (e.payload.last_tool ?? "connected") : null),
      ),
    ];
    return () => {
      for (const pending of subs) void pending.then((unlisten) => unlisten());
    };
  }, []);
```

with imports for `DocumentChanged`, `ViewFocus`, `McpActivity` from `./generated`, `applyDocumentChanged` from `./mcp/follow`, and `setMcpActivity` from `./hint`. `setStep`, `setSelection`, `setShapeEditing`, `setActiveLayer` and `setProject` are React state setters, stable, so the empty dependency list is right; if the linter objects, list them.

`StartScreen` is rendered when `project` is null, outside `EditorApp`'s editor branch but inside the same component, so the listener is mounted either way. Confirm by opening a project through the service while the start screen shows (Task 11).

- [ ] **Step 5: Typecheck, test, commit**

```bash
npm run ui:typecheck && npm run ui:test
git add ui/src && git commit -m "Follow the MCP service in the interface"
```

`MapHandle.focus` does not exist yet, so the typecheck fails until Task 8; do Task 8's Step 1 first if working in order, then commit both together.

---

### Task 8: MapView: focus and the capture answer

**Files:**
- Modify: `ui/src/map/MapView.tsx` (`MapHandle` at line 458, `capture` at line 5406, `useImperativeHandle` at line 2509, `__veCapture` install at line 5505)
- Test: `ui/src/map/focus.test.ts`

**Interfaces:**
- Produces: `MapHandle.focus(lon: number, lat: number, pxPerDeg?: number): void`, `framebufferPng(): Promise<string | null>` (base64, no data-URL prefix), the `view://capture` listener.

- [ ] **Step 1: `focus` on the handle**

Add to `MapHandle`:

```ts
  /**
   * Pans to a place, optionally at a zoom, for the MCP service's
   * `view_focus` (spec 8.8). Clamped like every other camera move.
   */
  focus(lon: number, lat: number, pxPerDeg?: number): void;
```

In `MapView`, beside `bounds`:

```ts
  const focus = useCallback(
    (lon: number, lat: number, pxPerDeg?: number) => {
      cameraRef.current = clampCamera(
        { ...cameraRef.current, centerLon: lon, centerLat: lat, pxPerDeg: pxPerDeg ?? cameraRef.current.pxPerDeg },
        viewRef.current,
      );
      requestDraw();
    },
    [requestDraw],
  );
```

and add `focus` to the `useImperativeHandle` object and its dependency list. The camera keeps its `projection` because the spread copies it (CLAUDE.md: "a camera built from three literal fields silently flattens the map").

Create `ui/src/map/focus.test.ts` against the same property the pan is tested by, using the camera helpers directly:

```ts
import { describe, expect, it } from "vitest";

import { clampCamera } from "./camera";

describe("focus camera", () => {
  it("keeps the projection and clamps latitude", () => {
    const view = { width: 800, height: 600 };
    const before = { centerLon: 0, centerLat: 20, pxPerDeg: 3, projection: "mercator" as const };
    const after = clampCamera({ ...before, centerLon: -70.5, centerLat: 95, pxPerDeg: 12 }, view);
    expect(after.projection).toBe("mercator");
    expect(after.centerLon).toBeCloseTo(-70.5);
    expect(after.centerLat).toBeLessThanOrEqual(90);
    expect(after.pxPerDeg).toBe(12);
  });
});
```

Adjust the `view` shape and the projection literal to what `camera.ts` exports (`grep -n 'export function clampCamera' -A6 ui/src/map/camera.ts`).

- [ ] **Step 2: Split `capture` and answer `view://capture`**

Rename the body of `capture` from the settle loop through `const base64 = ...` into:

```ts
  /** The map's framebuffer with the overlay on it, as base64 PNG, once the tiles have settled. */
  const framebufferPng = useCallback(async (): Promise<string | null> => {
    // …the existing body up to and including `const base64 = dataUrl.slice(...)`…
    return base64;
  }, [requestDraw]);

  const capture = useCallback(async (name: string): Promise<string | null> => {
    const base64 = await framebufferPng();
    if (base64 === null) return null;
    const path = await api.saveDebugCapture(name, base64);
    void api.frontendLog("info", `capture written to ${path}`);
    return path;
  }, [framebufferPng]);
```

Then, beside the dev-only `__veCapture` effect (which stays exactly as it is), add an effect that runs in every build:

```ts
  // The MCP service's screenshot (spec 8.8). Not a window global and not
  // dev-only: it answers an event the backend emits only while the person
  // has the service on, with the same picture the capture suite takes.
  useEffect(() => {
    const pending = listen<CaptureRequest>("view://capture", async (event) => {
      const png = await framebufferPng();
      if (png === null) return;
      try {
        await api.deliverCapture(event.payload.id, png);
      } catch (err) {
        void api.frontendLog("warn", `capture ${event.payload.id} not delivered: ${String(err)}`);
      }
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [framebufferPng]);
```

with `import { listen } from "@tauri-apps/api/event";` and `import type { CaptureRequest } from "../generated/CaptureRequest";`. A `null` from `framebufferPng` leaves the tool to time out and say the map did not answer, which is the truth.

- [ ] **Step 3: Typecheck, test, commit**

```bash
npm run ui:typecheck && npm run ui:test
git add ui/src && git commit -m "Let the map follow MCP focus requests and answer screenshot captures"
```

---

### Task 9: The Settings section

**Files:**
- Create: `ui/src/settings/McpSection.tsx`, `ui/src/settings/McpSection.test.tsx`
- Modify: `ui/src/settings/SettingsDialog.tsx` (insert after the Macros `</section>` near line 383)
- Modify: `ui/src/App.css` or the stylesheet the settings dialog uses (`grep -rn 'settings-field' ui/src/*.css`)

**Interfaces:**
- Consumes: `api.mcpStatus`, `api.setMcp`, `api.rotateMcpToken`, `NumberField`.
- Produces: `<McpSection onError={report} />`.

- [ ] **Step 1: Write the failing test**

Create `ui/src/settings/McpSection.test.tsx`:

```tsx
// @vitest-environment happy-dom
/**
 * The MCP section shows the URL and a ready client configuration only while
 * the service is on, and every change goes through the command (spec 8.8).
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { McpStatus } from "../generated/McpStatus";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const held = vi.hoisted(() => {
  const off: McpStatus = { enabled: false, port: 47391, token: "", bound_port: null, bind_error: null, sessions: 0, last_tool: null };
  const on: McpStatus = { ...off, enabled: true, token: "tok_abc", bound_port: 47391 };
  return {
    status: off,
    on,
    mcpStatus: vi.fn(async () => held.status),
    setMcp: vi.fn(async (enabled: boolean, port: number) => {
      held.status = enabled ? { ...held.on, port } : { ...held.on, enabled: false, token: "", bound_port: null, port };
      return held.status;
    }),
    rotateMcpToken: vi.fn(async () => {
      held.status = { ...held.status, token: "tok_new" };
      return held.status;
    }),
  };
});

vi.mock("../ipc", () => ({
  api: { mcpStatus: held.mcpStatus, setMcp: held.setMcp, rotateMcpToken: held.rotateMcpToken },
  IpcError: class extends Error {},
}));

import McpSection from "./McpSection";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  held.status = { enabled: false, port: 47391, token: "", bound_port: null, bind_error: null, sessions: 0, last_tool: null };
});

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("McpSection", () => {
  it("shows no URL while off and one with the token once on", async () => {
    act(() => root.render(<McpSection onError={() => {}} />));
    await flush();
    expect(host.textContent).not.toContain("http://127.0.0.1");
    const toggle = host.querySelector<HTMLInputElement>('input[type="checkbox"]');
    await act(async () => {
      toggle?.click();
    });
    await flush();
    expect(held.setMcp).toHaveBeenCalledWith(true, 47391);
    expect(host.textContent).toContain("http://127.0.0.1:47391/mcp");
    const snippet = host.querySelector("pre")?.textContent ?? "";
    expect(snippet).toContain("Bearer tok_abc");
    expect(snippet).toContain("claude mcp add");
  });

  it("rotates the token through the command", async () => {
    held.status = held.on;
    act(() => root.render(<McpSection onError={() => {}} />));
    await flush();
    const rotate = Array.from(host.querySelectorAll("button")).find((b) => b.textContent?.includes("Rotate"));
    await act(async () => {
      rotate?.click();
    });
    await flush();
    expect(held.rotateMcpToken).toHaveBeenCalled();
    expect(host.textContent).toContain("tok_new");
  });
});
```

Run: `npm run ui:test -- McpSection`. Expected: fails, module not found.

- [ ] **Step 2: The component**

Create `ui/src/settings/McpSection.tsx`:

```tsx
/**
 * Settings → MCP service (spec 8.8): the switch, the port, and a client
 * configuration with the token filled in.
 */
import { useCallback, useEffect, useState } from "react";

import type { McpStatus } from "../generated/McpStatus";
import NumberField from "../NumberField";
import { api } from "../ipc";

/** The Claude Code command and a generic HTTP client entry. */
export function clientSnippets(status: McpStatus): { claudeCode: string; json: string; bridge: string } {
  const url = `http://127.0.0.1:${status.port}/mcp`;
  const auth = `Bearer ${status.token}`;
  return {
    claudeCode: `claude mcp add --transport http vectoreffects ${url} --header "Authorization: ${auth}"`,
    json: JSON.stringify({ mcpServers: { vectoreffects: { type: "http", url, headers: { Authorization: auth } } } }, null, 2),
    bridge: `npx -y mcp-remote ${url} --header "Authorization:${auth}"`,
  };
}

export default function McpSection({ onError }: { onError: (err: unknown) => void }) {
  const [status, setStatus] = useState<McpStatus | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  useEffect(() => {
    void api.mcpStatus().then(setStatus).catch(onError);
  }, [onError]);

  const apply = useCallback(
    (promise: Promise<McpStatus>) => {
      void promise.then(setStatus).catch(onError);
    },
    [onError],
  );

  const copy = useCallback((label: string, text: string) => {
    void navigator.clipboard?.writeText(text).then(() => setCopied(label)).catch(() => setCopied(null));
  }, []);

  if (status === null) return <section><h3>MCP service</h3><span>…</span></section>;
  const snippets = clientSnippets(status);
  const url = `http://127.0.0.1:${status.port}/mcp`;

  return (
    <section>
      <h3>MCP service</h3>
      <p className="settings-note">
        Lets an AI client on this computer drive the application: open and edit projects, animate, import, export and take
        pictures of the map. Nothing outside this machine can reach it, and nothing can reach it while it is off.
      </p>
      <label className="settings-field">
        <input
          type="checkbox"
          checked={status.enabled}
          onChange={(event) => apply(api.setMcp(event.target.checked, status.port))}
        />
        Enable the MCP service on this computer
      </label>
      <label className="settings-field">
        Port
        <NumberField
          value={status.port}
          min={1}
          max={65535}
          step={1}
          commitWhileTyping={false}
          onCommit={(port) => apply(api.setMcp(status.enabled, Math.round(port)))}
        />
      </label>
      {status.bind_error && <p className="settings-error">{status.bind_error}</p>}
      {status.enabled && (
        <>
          <div className="settings-field">
            <span>URL</span>
            <code>{url}</code>
          </div>
          <div className="settings-field">
            <span>Token</span>
            <code>{status.token}</code>
            <button onClick={() => apply(api.rotateMcpToken())}>Rotate token</button>
          </div>
          <div className="settings-field">
            <span>
              {status.sessions === 0
                ? "No client connected"
                : `${status.sessions} client${status.sessions === 1 ? "" : "s"} connected${status.last_tool ? `, last: ${status.last_tool}` : ""}`}
            </span>
          </div>
          <details open>
            <summary>Client configuration</summary>
            <pre className="settings-snippet">{snippets.claudeCode}</pre>
            <button onClick={() => copy("claude", snippets.claudeCode)}>Copy Claude Code command</button>
            <pre className="settings-snippet">{snippets.json}</pre>
            <button onClick={() => copy("json", snippets.json)}>Copy HTTP client JSON</button>
            <pre className="settings-snippet">{snippets.bridge}</pre>
            <button onClick={() => copy("bridge", snippets.bridge)}>Copy stdio bridge command</button>
            {copied && <span className="settings-note">Copied.</span>}
          </details>
        </>
      )}
    </section>
  );
}
```

Match `NumberField`'s prop names to `ui/src/NumberField.tsx` (`value`, `onCommit`, `min`, `max`, `step`, `commitWhileTyping` per CLAUDE.md; check the exact spelling).

In `SettingsDialog.tsx`, import `McpSection` and render `<McpSection onError={report} />` after the Macros section, where `report` is the dialog's existing error reporter (the one `setMacroDirectory` chains `.catch(report)` to).

Add to the stylesheet that holds `.settings-field`:

```css
.settings-snippet { white-space: pre-wrap; word-break: break-all; font-size: 11px; padding: 6px; background: var(--panel-inset, rgba(0,0,0,0.2)); border-radius: 4px; }
.settings-error { color: var(--error, #e66); }
.mcp-badge { font-weight: 600; }
```

- [ ] **Step 3: Test, typecheck, offline check, commit**

```bash
npm run ui:test -- McpSection && npm run ui:typecheck
npm run ui:build && npm run check:offline
git add ui/src && git commit -m "Add the MCP service section to Settings"
```

`check:offline` allows `//127.0.0.1` by name (`tools/check-offline.sh` line 32), so the URL in the component passes.

---

### Task 10: The help topic, the user guide, the spec and the rules

**Files:**
- Modify: `spec.md` (invariant 5 at line 85; §8.6 at 2959; insert §8.8 before `## 9.` at line 3177; §15 list end), `plan.md` (top dated entry and §5), `CLAUDE.md` (invariant 5 at lines 47-61; a recipe), `docs/USER-GUIDE.md` (Settings at line 132), `ui/src/help/topics.ts` (settings topic at line 128), `docs/superpowers/specs/2026-09-16-mcp-service-design.md` (§3 and §4 deviations).

- [ ] **Step 1: spec.md**

Invariant 5, after the M38 paragraph, add:

```
   **The invariant runs both ways, and has one inbound exception (M86):**
   the MCP service of §8.8 listens on `127.0.0.1` while — and only while —
   the person has switched it on in Settings, answers only to the token
   that switch issued, and lives in `ve-app`'s `mcp` module, which nothing
   calls but the setting and start-up. It is the same difference M38 drew:
   *who asked*. The WebDriver endpoint is not this: it is unauthenticated,
   drives the interface rather than the domain, has no switch, and stays
   compiled out of every shipped build.
```

§8.6: add a paragraph:

```
**MCP service.** A switch, a port (default 47391) and a token. On, the
application serves the Model Context Protocol at
`http://127.0.0.1:<port>/mcp` and shows a ready client configuration; off,
nothing listens and the token is forgotten. *Rotate token* issues a new one.
The status bar shows `MCP: <tool>` while a client is connected.
```

New §8.8 before `## 9.`:

```
### 8.8 MCP service

An MCP server inside the application (M86). Every tool calls the command
the interface calls, with the same state, so there is one implementation of
each feature and every edit is undoable like a person's. Tools that write
return the `ProjectSummary` and emit `document://changed`, which the
frontend applies as it applies its own call's result; `view://focus`,
`view://step` and `view://selection` move what is frontend state. The
`screenshot` tool asks the map for its own framebuffer through
`view://capture` and `deliver_capture`; nothing is written to disk.

Groups: project (`project_status`, `project_new`, `project_open`,
`project_save`, `project_close`, `recent_projects`, `tool_catalogue`),
structure (`layers_list`, `layer_add`, `layer_set`, `layer_move`,
`layer_remove`, `objects_list`, `object_get`, `object_create`, `object_set`,
`object_move`, `object_duplicate`, `object_remove`, `objects_in_region`),
time (`keyframe_set`, `keyframe_remove`, `keyframe_move`,
`interpolation_set`, `motion_add`, `follow_set`, `timeline_set`,
`object_tracks`), field (`field_sample`, `field_capture`, `field_paste`,
`macro_list`, `macro_insert`), files (`import_grib`, `import_image`,
`import_history`, `export_grib`, `export_zarr`, `export_cancel`), view
(`screenshot`, `view_focus`, `step_set`, `selection_set`), history (`undo`,
`redo`, `history_list`, `history_jump`) and `invoke`, which runs any IPC
command by name — the test in `mcp/invoke.rs` holds its table equal to the
handler list minus a named exclusion list, so a command added later is
reachable the day it lands.

Transport: Streamable HTTP on loopback, bearer token, `Host` and `Origin`
restricted to loopback names. Directions cross this boundary as azimuth
toward and speeds in m/s, the domain's own units; a client that wants the
project's display convention reads it from the summary.
```

§15: append a bullet:

```
- The MCP service (§8.8) is the second exception to invariant 5 and the first
  inbound one. It is switched, tokened and confined to one module; the
  WebDriver rule is unchanged.
```

- [ ] **Step 2: plan.md and CLAUDE.md**

`plan.md` top, a dated paragraph in the style of the Zarr one:

```
**2026-09-16: MCP service (M86).** An MCP server inside the application,
on loopback, switched on in Settings. Tools call the IPC commands with a
`tauri::State` from the handle, so nothing is duplicated; the frontend
follows through `document://changed` and three view events; `invoke` reaches
any command by name with a test that keeps its table complete. Design:
`docs/superpowers/specs/2026-09-16-mcp-service-design.md`.
```

`plan.md` §5, a paragraph:

```
### MCP service and invariant 5

The invariant said nothing reaches in, and the WebDriver endpoint is kept
out of shipped builds for that reason. The MCP service is allowed in because
it differs on every point that made WebDriver unshippable: it is off until a
person turns it on, it is bound to a token that switch issues, it answers
only loopback names, and it drives the domain through the same commands the
interface uses rather than the interface itself. Recorded as an exception,
not a repeal; a second inbound socket would need the same four properties
and its own entry here.
```

`CLAUDE.md` invariant 5, after the M69 paragraph:

```
   **And one inbound exception (M86):** the MCP service, `ve-app`'s `mcp`
   module, listens on loopback only while the setting is on and only for its
   token. `tests/mcp.rs`'s `off_means_no_socket_and_stop_releases_the_port`
   is the enforcement; the offline check still cannot see a listener.
```

and a recipe after "Adding a measurement":

```
### Adding an MCP tool

1. The tool calls the `#[tauri::command]` function with `app.state()`; it
   never reimplements it. If the feature has no command, add the command
   first, for the interface.
2. Parameters are a `schemars::JsonSchema` struct in `mcp/tools.rs` with a
   doc comment per field; the client reads those.
3. A tool that writes goes through `VectorEffects::write`, which emits
   `document://changed`. One that opens or closes a project passes
   `opened: true`. One that changes only frontend state emits its
   `view://*` event and nothing else.
4. A new command lands in `mcp/invoke.rs`'s table, or in `EXCLUDED` with a
   reason in the comment; the coverage test fails otherwise.
5. An integration test in `tests/mcp.rs` drives it over HTTP and checks the
   document through the interface's own read.
```

- [ ] **Step 3: User guide and help topic**

`docs/USER-GUIDE.md` Settings, after the macro directory sentence:

```
**MCP service.** Off by default. On, an AI client on this computer (Claude
Code, or anything that speaks the Model Context Protocol over HTTP) can
open, edit, animate, import, export and screenshot through the application,
and the map follows along. The section shows the address and a client
configuration with the token filled in; *Rotate token* revokes the old one.
Nothing outside this computer can reach it.
```

Replace "The application makes no network requests at any time." with:

```
The application makes no network requests of its own. It accepts none unless
you turn the MCP service on, and then only from this computer.
```

`ui/src/help/topics.ts`, add to the settings topic's `parameters`:

```ts
["MCP service", "Lets an AI client on this computer drive the application over the Model Context Protocol. Off by default; on, the section shows the local address, the token and a ready client configuration. Rotate token revokes the old one. The status bar shows MCP and the last tool while a client is connected."],
```

Run `npm run ui:test -- topics` — the help tests check parameters against the catalogue and images; a text-only parameter passes.

- [ ] **Step 4: Spec deviations**

In `docs/superpowers/specs/2026-09-16-mcp-service-design.md` §3, replace the `*_impl` paragraph with: "Every tool calls the `#[tauri::command]` function with `app.state::<AppState>()`, the same `State` Tauri passes. **No tool duplicates command logic**, and no split is made." In §4, change the screenshot timeout sentence to "the tool waits 35 s: the map's own 10 s for tiles and 20 s for a frame, and a margin."

- [ ] **Step 5: Commit**

```bash
git add spec.md plan.md CLAUDE.md docs ui/src/help
git commit -m "Record the MCP service in the spec, the rules and the guide"
```

---

### Task 11: End to end through the real application

**Files:**
- Create: `tools/webdriver/mcp-follow.mjs`
- Modify: `package.json` (script `tools:mcp-follow`)

**Interfaces:**
- Consumes: `tools/webdriver/client.mjs` (`launch`, `connect`), the running app's `/mcp`.

- [ ] **Step 1: The script**

```js
/**
 * The M76 check for the MCP service: a client moves an object and the
 * picture shows it moved. Runs the dev app with WebDriver so the driver can
 * enable the service and take the picture; the MCP calls go over HTTP.
 */
import { readFile } from "node:fs/promises";

import { launch } from "./client.mjs";

const root = new URL("../..", import.meta.url).pathname;
const app = await launch({ cwd: root });
try {
  await app.evaluate(`window.__veOpen(${JSON.stringify(root + "assets/samples/cyclone.veproj")}).then(() => __done(true))`);
  const status = await app.evaluate(`
    import("/src/ipc.ts").then(({ api }) => api.setMcp(true, 0).then((s) => __done(JSON.stringify(s))))
  `);
  const { bound_port: port, token } = JSON.parse(status);
  const headers = { "content-type": "application/json", accept: "application/json, text/event-stream", authorization: `Bearer ${token}` };
  const url = `http://127.0.0.1:${port}/mcp`;
  let session = null;
  const rpc = async (method, params, id) => {
    const res = await fetch(url, { method: "POST", headers: { ...headers, ...(session ? { "mcp-session-id": session } : {}) }, body: JSON.stringify({ jsonrpc: "2.0", id, method, params }) });
    session ??= res.headers.get("mcp-session-id");
    const text = await res.text();
    const line = text.split("\n").find((l) => l.startsWith("data:"));
    return JSON.parse(line ? line.slice(5) : text);
  };
  await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "follow", version: "0" } }, 1);
  await fetch(url, { method: "POST", headers: { ...headers, "mcp-session-id": session }, body: JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) });
  const before = await app.capture("mcp-before");
  const tree = await rpc("tools/call", { name: "layers_list", arguments: { step: 0 } }, 2);
  const object = tree.result.structuredContent.layers.flatMap((l) => l.objects)[0].id;
  await rpc("tools/call", { name: "object_set", arguments: { object, step: 0, values: { position: [-30, 20] } } }, 3);
  await new Promise((r) => setTimeout(r, 1500));
  const after = await app.capture("mcp-after");
  const same = (await readFile(before)).equals(await readFile(after));
  console.log(same ? "FAIL: the map did not change" : `OK: ${before} -> ${after}`);
  process.exitCode = same ? 1 : 0;
} finally {
  await app.stop();
}
```

`launch`, `evaluate`, `capture` and `stop` are what `client.mjs` exports today (`cli.mjs` uses them); match their names. A port of `0` in `setMcp` binds any free port, and `bound_port` reports it.

- [ ] **Step 2: Run it**

```bash
node tools/webdriver/mcp-follow.mjs
```

Expected: `OK: … -> …`, and opening the two PNGs shows the object in a different place in the second. Add `"tools:mcp-follow": "node tools/webdriver/mcp-follow.mjs"` to `package.json` scripts and commit:

```bash
git add tools/webdriver/mcp-follow.mjs package.json
git commit -m "Add the end-to-end follow check for the MCP service"
```

---

### Task 12: Full verification

- [ ] **Step 1: Everything**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
VE_FORCE_CPU=1 cargo test --workspace
npm run bindings && git diff --exit-code -- ui/src/generated
npm run ui:typecheck && npm run ui:test
npm run ui:build && npm run check:offline
cargo tree -p ve-app -e normal | grep -oE '[a-z0-9_-]+-sys v[0-9.]+' | sort -u
```

The last line must list only `aws-lc-sys`, `core-foundation-sys`, `dirs-sys`, `security-framework-sys` and `zstd-sys` on macOS, as before this work.

- [ ] **Step 2: By hand, in `npm run dev`**

1. Settings → MCP service: on. The URL and token appear. `claude mcp add …` from the copied command; `claude` lists `vectoreffects` connected.
2. From Claude Code: "open assets/samples/cyclone.veproj, move the first object to 30°W 20°N, set the step to 3, screenshot". The app shows the project, the object moved, the timeline at step 3, and the reply carries the picture.
3. `Cmd`-`Z` in the app undoes the client's move.
4. Settings → off. The client's next call fails to connect. Quit and relaunch: still off, nothing listening (`lsof -iTCP:47391` empty).
5. On again, quit, relaunch: listening, same port, new token shown.

- [ ] **Step 3: Commit any fixes, then finish the branch**

Use `superpowers:finishing-a-development-branch`.
