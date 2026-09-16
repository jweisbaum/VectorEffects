//! Bind, accept, check the bearer, hand the request to `rmcp`.
//!
//! **This is the one inbound socket in the application** (invariant 5,
//! second exception). It exists only between `start` and `Running::stop`,
//! and `McpService::apply` is the only caller of both.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::BoxBody};
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
#[derive(Debug)]
pub struct Running {
    pub port: u16,
    pub cancel: CancellationToken,
    /// Signalled once the accept loop has dropped the socket. `stop` waits
    /// on it so the port is free the instant it returns, rather than racing
    /// the task that actually owns the listener — `off_means_no_socket`
    /// rebinds the same port immediately after `apply` stops it.
    stopped: std::sync::mpsc::Receiver<()>,
}

/// How long `stop` waits for the accept loop to confirm its socket closed,
/// before giving up and returning anyway. The confirmation normally arrives
/// in milliseconds; this is a backstop, not the expected path, so `stop`
/// never blocks its caller — the main thread included — unboundedly.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

impl Running {
    /// Cancels the listener and waits (briefly) for its socket to close.
    pub fn stop(self) {
        self.cancel.cancel();
        if self.stopped.recv_timeout(STOP_TIMEOUT).is_err() {
            tracing::warn!(
                port = self.port,
                "mcp listener did not confirm shutdown within {STOP_TIMEOUT:?}"
            );
        }
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
/// logged from a task nobody watches. Generic over the Tauri runtime so the
/// mock application used by the integration tests can start the same
/// listener the shipped `tauri::Wry` build does.
pub fn start<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    port: u16,
    token: String,
) -> Result<Running, String> {
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
    let origins: Vec<String> = hosts.iter().map(|h| format!("http://{h}")).collect();
    let config = StreamableHttpServerConfig::default()
        .with_sse_keep_alive(Some(Duration::from_secs(15)))
        .with_allowed_hosts(hosts)
        .with_allowed_origins(origins)
        .with_cancellation_token(cancel.child_token());
    let handler_app = app.clone();
    let service = Arc::new(StreamableHttpService::new(
        move || Ok(VectorEffects::new(handler_app.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    ));
    let token: Arc<str> = token.into();
    let loop_cancel = cancel.clone();
    let (stopped_tx, stopped_rx) = std::sync::mpsc::channel::<()>();

    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(error = %e, "mcp listener could not join the runtime");
                let _ = stopped_tx.send(());
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
                        let auth = req
                            .headers()
                            .get("authorization")
                            .and_then(|v| v.to_str().ok());
                        if !super::token::matches(auth, &token) {
                            return Ok::<_, std::convert::Infallible>(plain(
                                StatusCode::UNAUTHORIZED,
                                "unauthorized",
                            ));
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
        // Dropped before the send, so a caller unblocked by `stopped.recv()`
        // never races the fd still being open.
        drop(listener);
        let _ = stopped_tx.send(());
        tracing::info!(port = bound, "mcp listener stopped");
    });

    tracing::info!(port = bound, "mcp listener started");
    Ok(Running {
        port: bound,
        cancel,
        stopped: stopped_rx,
    })
}
