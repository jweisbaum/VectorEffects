//! The stdio bridge, `history-syncer --mcp` (MCP plugins spec, section 3). It answers the
//! handshake, `ping` and `tools/list` from the compiled catalog without the app, and forwards each
//! `tools/call` to the app's server on `127.0.0.1`, launching the app when it is not running.
//! It never reads `settings.json` and never sees a credential; stdout carries only JSON-RPC.
use super::endpoint::{self, Endpoint};
use super::text;
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, JsonObject, ListToolsResult,
        PaginatedRequestParams, ServerConfig,
    },
    service::RequestContext,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Starts the app. `AppLauncher` in the product; a fake in tests.
pub trait Launch: Send + Sync + 'static {
    fn launch(&self) -> Result<(), String>;
}

/// Waits; tests shorten them.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub poll: Duration,
    /// After a launch, how long the app may take to hold its lock (section 3.4 step 2).
    pub launch_wait: Duration,
    /// With the lock held, how long the endpoint file may take to appear.
    pub access_wait: Duration,
}

impl Timing {
    pub const DEFAULT: Timing = Timing {
        poll: Duration::from_millis(250),
        launch_wait: Duration::from_secs(45),
        access_wait: Duration::from_secs(10),
    };
}

/// What one POST to the app returned.
enum Posted {
    Result(Value),
    Unauthorized,
    Refused,
    Failed(String),
}

#[derive(Clone)]
pub struct Bridge {
    dir: PathBuf,
    connection: Option<PathBuf>,
    info: ServerConfig,
    tools: Vec<rmcp::model::Tool>,
    launcher: Arc<dyn Launch>,
    timing: Timing,
    client: crate::http::Client,
    ids: Arc<AtomicU64>,
    /// Held while one call launches the app and waits for it, so parallel calls launch it once.
    launching: Arc<tokio::sync::Mutex<()>>,
}

impl Bridge {
    pub fn with_connection(mut self, path: PathBuf) -> Self {
        self.connection = Some(path);
        self
    }

    /// Install the app's compiled catalog; handshake/discovery never launch the app.
    pub fn with_catalog(mut self, info: ServerConfig, tools: Vec<rmcp::model::Tool>) -> Self {
        self.info = info;
        self.tools = tools;
        self
    }

    /// Fails only when the HTTP client cannot be built; there is no fallback client, because the
    /// default one would honour proxies from the environment.
    pub fn new(dir: PathBuf, launcher: Arc<dyn Launch>, timing: Timing) -> anyhow::Result<Self> {
        // Loopback only: never through a proxy from the environment, never redirected.
        let client = crate::http::Client::new();
        Ok(Bridge {
            connection: None,
            info: ServerConfig::new(
                rmcp::model::ServerCapabilities::builder()
                    .enable_tools()
                    .build(),
            ),
            tools: Vec::new(),
            dir,
            launcher,
            timing,
            client,
            ids: Arc::new(AtomicU64::new(1)),
            launching: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// With the lock held, waits for an endpoint file whose `pid` is not `stale`.
    async fn wait_for_endpoint(&self, stale: Option<u32>) -> Result<Endpoint, String> {
        let deadline = Instant::now() + self.timing.access_wait;
        loop {
            if let Some(found) = endpoint::read(&self.dir)
                && Some(found.pid) != stale
            {
                return Ok(found);
            }
            if Instant::now() >= deadline {
                return Err(text::ACCESS_OFF.into());
            }
            tokio::time::sleep(self.timing.poll).await;
        }
    }

    /// The running app's endpoint, or `None` when the app is not running.
    async fn running(&self) -> Option<Result<Endpoint, String>> {
        let current = endpoint::read(&self.dir);
        if !endpoint::lock_held(&self.dir) {
            return None;
        }
        Some(match current {
            Some(found) => Ok(found),
            None => self.wait_for_endpoint(None).await,
        })
    }

    /// Section 3.4 steps 1 and 2: the running app's endpoint, launching the app if needed.
    pub async fn find(&self) -> Result<Endpoint, String> {
        if let Some(running) = self.running().await {
            return running;
        }
        let _launching = self.launching.lock().await;
        // A parallel call may have launched the app while this one waited for the mutex.
        if let Some(running) = self.running().await {
            return running;
        }
        let stale = endpoint::read(&self.dir).map(|found| found.pid);
        self.launcher
            .launch()
            .map_err(|_| text::NOT_STARTED.to_string())?;
        let deadline = Instant::now() + self.timing.launch_wait;
        loop {
            tokio::time::sleep(self.timing.poll).await;
            if endpoint::lock_held(&self.dir) {
                return self.wait_for_endpoint(stale).await;
            }
            if Instant::now() >= deadline {
                return Err(text::NOT_STARTED.into());
            }
        }
    }

    /// One `tools/call` POST (section 3.4 steps 3 and 4).
    async fn post(&self, to: &Endpoint, name: &str, arguments: &Option<JsonObject>) -> Posted {
        let id = self.ids.fetch_add(1, Ordering::Relaxed);
        let body = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": arguments.clone().unwrap_or_default()}});
        let response = match self
            .client
            .post(
                to.port,
                &to.token,
                body.to_string(),
                Duration::from_secs(300),
            )
            .await
        {
            Ok(response) => response,
            Err(crate::http::Error::Connect) => return Posted::Refused,
            Err(_) => return Posted::Failed(text::NOT_ANSWERING.into()),
        };
        if response.status == 401 {
            return Posted::Unauthorized;
        }
        let event_stream = response.event_stream;
        let body = response.body;
        let reply = if event_stream {
            body.lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
                .find(|message| message["id"] == id)
        } else {
            serde_json::from_str::<Value>(&body).ok()
        };
        match reply {
            Some(reply) if reply.get("result").is_some() => Posted::Result(reply["result"].clone()),
            Some(reply) => Posted::Failed(
                reply["error"]["message"]
                    .as_str()
                    .unwrap_or(text::UNEXPECTED_ANSWER)
                    .to_string(),
            ),
            None => Posted::Failed(text::UNEXPECTED_ANSWER.into()),
        }
    }

    /// Forwards one call starting from `to`; one retry after a 401 (the token rotated) and one
    /// after a refused connection (a stale endpoint).
    pub async fn forward_from(
        &self,
        mut to: Endpoint,
        name: &str,
        arguments: &Option<JsonObject>,
    ) -> Result<Value, String> {
        let (mut reread, mut refound) = (false, false);
        loop {
            match self.post(&to, name, arguments).await {
                Posted::Result(result) => {
                    if let Some(path) = &self.connection {
                        let stamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs());
                        let _ = crate::install::atomic_write(
                            path,
                            &serde_json::to_vec(
                                &json!({"pid":to.pid,"port":to.port,"verified_at":stamp}),
                            )
                            .unwrap_or_default(),
                        );
                    }
                    return Ok(result);
                }
                Posted::Unauthorized if !reread => {
                    reread = true;
                    to = endpoint::read(&self.dir).ok_or(text::ACCESS_RESTARTED)?;
                }
                Posted::Unauthorized => return Err(text::ACCESS_RESTARTED.into()),
                Posted::Refused if !refound => {
                    refound = true;
                    to = self.find().await?;
                }
                Posted::Refused => return Err(text::NOT_ANSWERING.into()),
                Posted::Failed(message) => return Err(message),
            }
        }
    }

    /// The tool result for one call; every failure is a tool error the host shows.
    pub async fn forward(&self, name: &str, arguments: Option<JsonObject>) -> CallToolResult {
        let result = match self.find().await {
            Ok(to) => self.forward_from(to, name, &arguments).await,
            Err(message) => Err(message),
        };
        match result.and_then(|value| {
            serde_json::from_value::<CallToolResult>(value)
                .map_err(|_| text::UNEXPECTED_ANSWER.to_string())
        }) {
            Ok(result) => result,
            Err(message) => crate::results::refused(message),
        }
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        self.info.clone()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if !self.tools.iter().any(|tool| tool.name == request.name) {
            return Err(ErrorData::invalid_params(text::UNKNOWN_TOOL, None));
        }
        Ok(self.forward(&request.name, request.arguments).await.into())
    }
}

/// The most a reply from the app may hold; anything larger is not a tool result.
pub const MAX_REPLY: usize = 16 << 20;

/// The program to start (section 3.5). Inside its own AppImage, `current_exe` lives in a temporary
/// mount under `APPDIR` and `APPIMAGE` is the file; an `APPIMAGE` inherited from a host that is
/// itself an AppImage names the host and is ignored.
pub fn launch_program(appimage: Option<PathBuf>, appdir: Option<PathBuf>, exe: PathBuf) -> PathBuf {
    match (appimage, appdir) {
        (Some(appimage), Some(appdir)) if exe.starts_with(&appdir) => appimage,
        _ => exe,
    }
}

/// Starts the same installed binary in GUI mode, without inheriting host proxies or stdio.
#[derive(Debug, Clone)]
pub struct NativeLauncher {
    pub executable: PathBuf,
    pub identifier: String,
    pub args: Vec<String>,
}
impl Launch for NativeLauncher {
    fn launch(&self) -> Result<(), String> {
        use std::process::{Command, Stdio};
        #[cfg(target_os = "macos")]
        if self.args.is_empty()
            && Command::new("/usr/bin/open")
                .args(["-b", &self.identifier])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        {
            return Ok(());
        }
        let program = launch_program(
            std::env::var_os("APPIMAGE").map(PathBuf::from),
            std::env::var_os("APPDIR").map(PathBuf::from),
            self.executable.clone(),
        );
        let mut command = Command::new(program);
        command
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"] {
            command
                .env_remove(name)
                .env_remove(name.to_ascii_lowercase());
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0000_0008 | 0x0000_0200);
        }
        command.spawn().map(drop).map_err(|e| e.to_string())
    }
}

pub fn run(bridge: Bridge) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let service = match bridge.serve(rmcp::transport::stdio()).await {
                Ok(service) => service,
                Err(rmcp::service::ServerInitializeError::ConnectionClosed(_)) => return Ok(()),
                Err(error) => return Err(anyhow::anyhow!("{error}")),
            };
            service.waiting().await?;
            Ok(())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    struct NeverLaunch;
    impl Launch for NeverLaunch {
        fn launch(&self) -> Result<(), String> {
            panic!("A running app must not be launched again")
        }
    }
    #[tokio::test]
    async fn rotation_retries_authorization_once_without_reinstalling() {
        let dir = tempfile::tempdir().unwrap();
        let _lock = endpoint::try_lock(dir.path()).unwrap().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let initial = Endpoint {
            version: 1,
            port: listener.local_addr().unwrap().port(),
            token: "old".into(),
            pid: std::process::id(),
        };
        endpoint::write(dir.path(), &initial).unwrap();
        let folder = dir.path().to_owned();
        let mut rotated = initial.clone();
        rotated.token = "new".into();
        let server = tokio::spawn(async move {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 16384];
                let size = stream.read(&mut bytes).await.unwrap();
                let request = String::from_utf8_lossy(&bytes[..size]).to_lowercase();
                if index == 0 {
                    assert!(request.contains("bearer old"));
                    endpoint::write(&folder, &rotated).unwrap();
                    stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                } else {
                    assert!(request.contains("bearer new"));
                    let body = r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"ok"}],"isError":false}}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
            }
        });
        let bridge = Bridge::new(
            dir.path().to_owned(),
            Arc::new(NeverLaunch),
            Timing::DEFAULT,
        )
        .unwrap()
        .with_connection(dir.path().join("client-codex.json"));
        let result = bridge.forward("read", None).await;
        assert_ne!(result.is_error, Some(true));
        server.await.unwrap();
        let stamp: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("client-codex.json")).unwrap())
                .unwrap();
        assert_eq!(stamp["pid"], std::process::id());
    }
    #[test]
    fn private_endpoint_roundtrip_and_invalid_versions() {
        let dir = tempfile::tempdir().unwrap();
        let value = Endpoint {
            version: 1,
            port: 42,
            token: "secret".into(),
            pid: 7,
        };
        endpoint::write(dir.path(), &value).unwrap();
        assert_eq!(endpoint::read(dir.path()), Some(value));
        assert!(!format!("{:?}", endpoint::read(dir.path())).contains("secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path().join(endpoint::FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let mut invalid = endpoint::read(dir.path()).unwrap();
        invalid.version = 999;
        endpoint::write(dir.path(), &invalid).unwrap();
        assert!(endpoint::read(dir.path()).is_none());
    }
}
