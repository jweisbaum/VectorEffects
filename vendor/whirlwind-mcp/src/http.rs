//! HTTP-only loopback client. No proxy, DNS destination, redirects, TLS or crypto-provider state.
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, header};
use hyper_util::{
    client::legacy::{Client as HyperClient, connect::HttpConnector},
    rt::TokioExecutor,
};
use std::time::Duration;
#[derive(Clone)]
pub(crate) struct Client(HyperClient<HttpConnector, Full<Bytes>>);
pub(crate) struct Response {
    pub status: u16,
    pub event_stream: bool,
    pub body: String,
}
#[derive(Debug)]
pub(crate) enum Error {
    Connect,
    Failed,
}
impl Client {
    pub fn new() -> Self {
        let mut connector = HttpConnector::new();
        connector.enforce_http(true);
        connector.set_connect_timeout(Some(Duration::from_secs(3)));
        Self(HyperClient::builder(TokioExecutor::new()).build(connector))
    }
    pub async fn post(
        &self,
        port: u16,
        token: &str,
        body: String,
        timeout: Duration,
    ) -> Result<Response, Error> {
        tokio::time::timeout(timeout, async {
            let mut authorization = header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| Error::Failed)?;
            authorization.set_sensitive(true);
            let request = Request::builder()
                .method("POST")
                .uri(format!("http://127.0.0.1:{port}/mcp"))
                .header(header::AUTHORIZATION, authorization)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json, text/event-stream")
                .header("MCP-Protocol-Version", "2025-11-25")
                .body(Full::new(Bytes::from(body)))
                .map_err(|_| Error::Failed)?;
            let response = self.0.request(request).await.map_err(|e| {
                if e.is_connect() {
                    Error::Connect
                } else {
                    Error::Failed
                }
            })?;
            let status = response.status().as_u16();
            let event_stream = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("text/event-stream"));
            if response
                .headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .is_some_and(|v| v > crate::bridge::MAX_REPLY as u64)
            {
                return Err(Error::Failed);
            }
            let mut incoming = response.into_body();
            let mut bytes = Vec::new();
            while let Some(frame) = incoming.frame().await {
                let frame = frame.map_err(|_| Error::Failed)?;
                if let Some(data) = frame.data_ref() {
                    if bytes.len() + data.len() > crate::bridge::MAX_REPLY {
                        return Err(Error::Failed);
                    }
                    bytes.extend_from_slice(data);
                }
            }
            Ok(Response {
                status,
                event_stream,
                body: String::from_utf8(bytes).map_err(|_| Error::Failed)?,
            })
        })
        .await
        .map_err(|_| Error::Failed)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn reply(response: &'static [u8], delay: Duration) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let count = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]).to_lowercase();
            assert!(request.starts_with("post /mcp http/1.1"));
            assert!(request.contains("authorization: bearer test-token"));
            tokio::time::sleep(delay).await;
            let _ = stream.write_all(response).await;
        });
        (port, task)
    }
    #[tokio::test]
    async fn redirect_is_returned_without_following_its_destination() {
        let (port, task) = reply(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", Duration::ZERO).await;
        let result = Client::new()
            .post(port, "test-token", "{}".into(), Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(result.status, 302);
        task.await.unwrap();
    }
    #[tokio::test]
    async fn oversized_response_is_refused_before_reading_the_body() {
        let (port, task) = reply(
            b"HTTP/1.1 200 OK\r\nContent-Length: 16777217\r\nConnection: close\r\n\r\n",
            Duration::ZERO,
        )
        .await;
        assert!(matches!(
            Client::new()
                .post(port, "test-token", "{}".into(), Duration::from_secs(2))
                .await,
            Err(Error::Failed)
        ));
        task.await.unwrap();
    }
    #[tokio::test]
    async fn timeout_bounds_the_entire_response() {
        let (port, task) = reply(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
            Duration::from_millis(200),
        )
        .await;
        assert!(matches!(
            Client::new()
                .post(port, "test-token", "{}".into(), Duration::from_millis(50))
                .await,
            Err(Error::Failed)
        ));
        task.await.unwrap();
    }
}
