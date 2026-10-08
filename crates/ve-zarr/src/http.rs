//! A read-only HTTP store for `zarrs`.
//!
//! `zarrs_http` does this already, but it takes `reqwest` with its default
//! features — `native-tls`, which on Linux is `openssl-sys`, a system C
//! library the three-platform build forbids (CLAUDE.md, invariant 5). The
//! trait is three methods over `GET`, so it is written here against the
//! `reqwest` already in the tree, with rustls.
//!
//! Reads only. Open-data stores are anonymous; Hindsight readers sign only
//! their own endpoint's requests, with redirects disabled.

use std::str::FromStr;

use reqwest::header::{CONTENT_LENGTH, HeaderValue, RANGE};
use reqwest::{StatusCode, Url};
use zarrs_storage::Bytes;
use zarrs_storage::byte_range::ByteRangeIterator;
use zarrs_storage::{
    MaybeBytes, MaybeBytesIterator, ReadableStorageTraits, StorageError, StoreKey,
};

use crate::error::{Result, ZarrError};

#[path = "hindsight_http.rs"]
mod hindsight_http;
#[path = "hindsight_inventory.rs"]
mod hindsight_inventory;

/// A Zarr store read over HTTPS.
#[derive(Debug)]
pub struct HttpStore {
    base: Url,
    client: reqwest::blocking::Client,
    /// How many requests the host has in flight, and the cap on it. Shared
    /// by every store on the host: the cap is the host's, and a product read
    /// from four stores at once is four stores on one host.
    lane: &'static Lane,
    credentials: Option<crate::s3::Credentials>,
    strict_errors: bool,
    cache: Option<std::sync::Mutex<ReadCache>>,
    range_gates: Vec<std::sync::Mutex<()>>,
    disk: Option<std::sync::Arc<std::sync::Mutex<crate::hindsight_cache::DiskCache>>>,
    progress: Option<std::sync::Arc<crate::hindsight_progress::Progress>>,
}

/// Per-import compressed bytes. A Hindsight inner chunk contains 72 hours
/// and every parameter; adjacent hour/component reads reuse its range.
/// The limit bounds global downloads, and dropping the source releases it.
#[derive(Debug)]
struct ReadCache {
    ranges: lru::LruCache<(String, u64, u64), Bytes>,
    sizes: lru::LruCache<String, Option<u64>>,
    versions: lru::LruCache<String, Option<ObjectVersion>>,
    bytes: usize,
}

#[derive(Debug, Clone)]
struct ObjectVersion {
    size: u64,
    etag: Option<String>,
}

const READ_CACHE_BYTES: usize = 512 * 1024 * 1024;

impl ReadCache {
    fn insert(&mut self, key: (String, u64, u64), bytes: Bytes) {
        if bytes.len() > READ_CACHE_BYTES {
            return;
        }
        if let Some(before) = self.ranges.pop(&key) {
            self.bytes -= before.len();
        }
        while self.bytes + bytes.len() > READ_CACHE_BYTES || self.ranges.len() >= 16_384 {
            let Some((_, before)) = self.ranges.pop_lru() else {
                break;
            };
            self.bytes -= before.len();
        }
        self.bytes += bytes.len();
        self.ranges.put(key, bytes);
    }
}

/// How many requests the Marine Data Store is sent at once, by this whole
/// process.
///
/// A 0.125 degree field is nine chunks per component, read for two
/// components and several steps at a time, and with nothing holding them
/// back that object store answered a burst of seventy with closed
/// connections and 408s. Eight is what the GlobCurrent import always ran
/// at — four hours, u and v together — and was never refused; the ASCAT
/// import ran at up to twenty-four across its four stores and was not
/// refused either. Twelve, shared by every store on the host.
const IN_FLIGHT: usize = 12;

/// The Marine Data Store's lane, one for the process.
static MARINE_LANE: std::sync::LazyLock<Lane> = std::sync::LazyLock::new(|| Lane::new(IN_FLIGHT));

/// Every other host's, as good as none.
static OPEN_LANE: std::sync::LazyLock<Lane> = std::sync::LazyLock::new(|| Lane::new(UNCAPPED));

// Index reads and field ranges share this cap even when several shards are
// being prefetched at once. Open Data retains its original host limits.
static HINDSIGHT_LANE: std::sync::LazyLock<Lane> = std::sync::LazyLock::new(|| Lane::new(8));

/// The cap for every other host: as good as none. ERA5 on Google is read
/// as many small range requests multiplexed over one connection, and a cap
/// on those doubled the time an hour took.
const UNCAPPED: usize = 256;

/// A counting gate on the requests a store has in flight.
#[derive(Debug)]
struct Lane {
    cap: usize,
    busy: std::sync::Mutex<usize>,
    freed: std::sync::Condvar,
}

/// A place in the lane, given back when dropped.
struct Permit<'a>(&'a Lane);

impl Lane {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            busy: std::sync::Mutex::new(0),
            freed: std::sync::Condvar::new(),
        }
    }

    fn permit(&self) -> Permit<'_> {
        let mut busy = self
            .busy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while *busy >= self.cap {
            busy = self
                .freed
                .wait(busy)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *busy += 1;
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut busy = self
            .0
            .busy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *busy -= 1;
        self.0.freed.notify_one();
    }
}

/// A transport failure, with its cause.
///
/// `reqwest`'s own message stops at "error sending request"; the reason — a
/// reset connection, a timeout — is in the chain beneath it, and is the only
/// part a person can act on.
fn failed(err: reqwest::Error) -> StorageError {
    StorageError::Other(with_causes(&err))
}

pub(crate) fn with_causes(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut cause = err.source();
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

/// How many times a request is tried before its failure is reported.
const ATTEMPTS: u32 = 6;

/// Runs a request again when it fails in transit.
///
/// A few days of a field is thousands of chunk requests, many in flight at
/// once, and a public object store drops one now and then — a reset, a
/// timeout, a 503. Each such loss cost the whole import. So a request that
/// fails to complete, or that the server refuses with a status that says
/// "not now", is tried again a few times with a short, growing pause; a
/// status that says "no" (404, 403 — a key that is not there) is answered
/// at once, since asking again would get the same answer.
pub(crate) fn retrying<T>(
    request: impl FnMut() -> std::result::Result<T, Retry>,
) -> std::result::Result<T, StorageError> {
    retrying_paced(request, std::thread::sleep)
}

/// [`retrying`] with the pause between tries handed out, so a test need not
/// wait through it.
fn retrying_paced<T>(
    mut request: impl FnMut() -> std::result::Result<T, Retry>,
    mut wait: impl FnMut(std::time::Duration),
) -> std::result::Result<T, StorageError> {
    let mut attempt = 1;
    loop {
        match request() {
            Ok(value) => return Ok(value),
            Err(Retry::No(err)) => return Err(err),
            Err(Retry::Later(err)) if attempt >= ATTEMPTS => return Err(err),
            Err(Retry::Later(err)) => {
                // One, two, four, eight, sixteen seconds: a store that is
                // shedding load is given time to stop.
                let pause = std::time::Duration::from_secs(1 << (attempt - 1));
                tracing::warn!(attempt, pause_s = pause.as_secs(), %err, "request failed; trying again");
                wait(pause);
                attempt += 1;
            }
        }
    }
}

/// Whether a failure is worth another try.
pub(crate) enum Retry {
    /// Try again shortly.
    Later(StorageError),
    /// The answer will not change.
    No(StorageError),
}

/// Whether a status is the server saying "not now".
pub(crate) fn transient(status: StatusCode) -> bool {
    status.is_server_error()
        || status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
}

impl HttpStore {
    /// Opens a store at `base`, which must be an absolute URL.
    ///
    /// The timeout is generous: a chunk is a megabyte or so from a public
    /// archive that is occasionally slow, and an import that gives up early
    /// costs the whole range.
    pub fn new(base: &str) -> Result<Self> {
        let base = Url::from_str(base)
            .map_err(|err| ZarrError::Open(format!("{base} is not a URL: {err}")))?;
        // The Marine Data Store is spoken to differently. It closed HTTP/2
        // connections carrying several megabyte-sized chunk bodies at once —
        // "connection closed before message completed", whatever the pause
        // between tries — and the same reads over one request per connection
        // complete; its idle connections are let go early, since a pooled
        // one it has already closed fails on reuse with the same message;
        // and it is sent a bounded number of requests at a time. ERA5 on
        // Google wants none of that: it is read as many small range requests
        // multiplexed over one connection, and the same settings doubled the
        // time an hour of it took.
        let marine = base.host_str() == Some(crate::stac::STORE_HOST);
        let mut builder =
            reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(120));
        if marine {
            builder = builder
                .http1_only()
                .pool_idle_timeout(std::time::Duration::from_secs(5));
        }
        let client = builder
            .build()
            .map_err(|err| ZarrError::Open(format!("no HTTP client: {err}")))?;
        Ok(Self {
            base,
            client,
            lane: if marine { &MARINE_LANE } else { &OPEN_LANE },
            credentials: None,
            strict_errors: false,
            cache: None,
            range_gates: Vec::new(),
            disk: None,
            progress: None,
        })
    }

    /// A Hindsight object store. A forbidden read is an error, never missing
    /// data; redirects are disabled so signed requests stay on this endpoint.
    pub(crate) fn s3(base: &str, credentials: Option<crate::s3::Credentials>) -> Result<Self> {
        let mut store = Self::new(base)?;
        store.client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| ZarrError::Open(format!("no HTTP client: {err}")))?;
        store.credentials = credentials;
        store.strict_errors = true;
        store.lane = &HINDSIGHT_LANE;
        store.cache = Some(std::sync::Mutex::new(ReadCache {
            ranges: lru::LruCache::unbounded(),
            sizes: lru::LruCache::unbounded(),
            versions: lru::LruCache::unbounded(),
            bytes: 0,
        }));
        store.range_gates = (0..64).map(|_| std::sync::Mutex::new(())).collect();
        Ok(store)
    }

    pub(crate) fn with_disk_cache(mut self, root: &std::path::Path) -> Self {
        self.disk = crate::hindsight_cache::DiskCache::open(root);
        self
    }

    pub(crate) fn with_progress(
        mut self,
        progress: std::sync::Arc<crate::hindsight_progress::Progress>,
    ) -> Self {
        self.progress = Some(progress);
        self
    }

    fn request(
        &self,
        method: reqwest::Method,
        url: &Url,
    ) -> std::result::Result<reqwest::blocking::RequestBuilder, StorageError> {
        let builder = self.client.request(method.clone(), url.clone());
        match &self.credentials {
            Some(credentials) => credentials.sign(builder, &method, url),
            None => Ok(builder),
        }
    }

    /// The URL a key is read from.
    fn url(&self, key: &StoreKey) -> std::result::Result<Url, StorageError> {
        let mut url = self.base.as_str().trim_end_matches('/').to_owned();
        let key = key.as_str();
        if !key.is_empty() {
            url.push('/');
            url.push_str(key.strip_prefix('/').unwrap_or(key));
        }
        Url::parse(&url).map_err(|err| StorageError::Other(err.to_string()))
    }
}

/// Reads one small text document, for a catalogue entry rather than a chunk.
///
/// A failure names the address, because the only thing a person can do about
/// one is try it.
pub fn get_text(url: &str) -> Result<String> {
    let failed = |why: String| ZarrError::Open(format!("could not read {url}: {why}"));
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|err| failed(err.to_string()))?;
    let response = client
        .get(url)
        .send()
        .map_err(|err| failed(err.to_string()))?;
    if response.status() != StatusCode::OK {
        return Err(failed(format!("the server answered {}", response.status())));
    }
    response.text().map_err(|err| failed(err.to_string()))
}

impl ReadableStorageTraits for HttpStore {
    fn get(&self, key: &StoreKey) -> std::result::Result<MaybeBytes, StorageError> {
        if self.strict_errors {
            return self.hindsight_get(key);
        }
        let url = self.url(key)?;
        retrying(|| {
            let _place = self.lane.permit();
            let response = self
                .request(reqwest::Method::GET, &url)
                .map_err(Retry::No)?
                .send()
                .map_err(|err| Retry::Later(failed(err)))?;
            match response.status() {
                StatusCode::OK => response
                    .bytes()
                    .map(Some)
                    .map_err(|err| Retry::Later(failed(err))),
                // A key that is not there. The S3 endpoint behind Copernicus
                // Marine answers a missing key with 403 rather than 404, and
                // a Zarr reader must read both as "no such chunk" or every
                // probe for an optional key becomes an error.
                StatusCode::NOT_FOUND => Ok(None),
                StatusCode::FORBIDDEN if !self.strict_errors => Ok(None),
                status => {
                    let err = StorageError::Other(format!(
                        "the archive answered {status} for {}",
                        key.as_str()
                    ));
                    Err(if transient(status) {
                        Retry::Later(err)
                    } else {
                        Retry::No(err)
                    })
                }
            }
        })
    }

    /// One range request per range, rather than one multipart request for all
    /// of them.
    ///
    /// Multipart ranges are what `zarrs_http` sends and are faster, but a
    /// server may answer one with the whole object; the ranges asked for here
    /// are few and large — a chunk index, an axis — so the simple form costs
    /// little and cannot be misread.
    fn get_partial_many<'a>(
        &'a self,
        key: &StoreKey,
        byte_ranges: ByteRangeIterator<'a>,
    ) -> std::result::Result<MaybeBytesIterator<'a>, StorageError> {
        if self.strict_errors {
            return self.hindsight_ranges(key, byte_ranges);
        }
        let Some(size) = self.size_key(key)? else {
            return Ok(None);
        };
        let url = self.url(key)?;
        let mut out: Vec<std::result::Result<Bytes, StorageError>> = Vec::new();
        for range in byte_ranges {
            let (start, end) = (range.start(size), range.end(size));
            let cache_key = (key.as_str().to_owned(), start, end);
            // Adjacent hours are read by different workers. Coalesce equal
            // range misses without serializing requests for distinct chunks.
            let _gate = if self.range_gates.is_empty() {
                None
            } else {
                use std::hash::{Hash, Hasher};
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                cache_key.hash(&mut hash);
                Some(
                    self.range_gates[hash.finish() as usize % self.range_gates.len()]
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()),
                )
            };
            if let Some(cache) = &self.cache {
                let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(bytes) = cache.ranges.get(&cache_key) {
                    out.push(Ok(bytes.clone()));
                    continue;
                }
            }
            let header = HeaderValue::from_str(&format!("bytes={start}-{}", end.saturating_sub(1)))
                .map_err(|err| StorageError::Other(err.to_string()))?;
            // The same retry and the same cap as a whole read: this is the
            // path a chunk is actually read by.
            let bytes = retrying(|| {
                let _place = self.lane.permit();
                let response = self
                    .request(reqwest::Method::GET, &url)
                    .map_err(Retry::No)?
                    .header(RANGE, header.clone())
                    .send()
                    .map_err(|err| Retry::Later(failed(err)))?;
                match response.status() {
                    StatusCode::PARTIAL_CONTENT => {
                        response.bytes().map_err(|err| Retry::Later(failed(err)))
                    }
                    // A server that ignored the range sent the whole object.
                    StatusCode::OK => {
                        let whole = response.bytes().map_err(|err| Retry::Later(failed(err)))?;
                        let (start, end) = (start as usize, end as usize);
                        if end > whole.len() {
                            return Err(Retry::No(StorageError::Other(
                                "the archive returned less than the range asked for".to_owned(),
                            )));
                        }
                        // A cached slice must not retain the entire shard
                        // while only its small visible range counts toward
                        // the byte limit.
                        Ok(Bytes::copy_from_slice(&whole[start..end]))
                    }
                    status => {
                        let err = StorageError::Other(format!(
                            "the archive answered {status} for a range of {}",
                            key.as_str()
                        ));
                        Err(if transient(status) {
                            Retry::Later(err)
                        } else {
                            Retry::No(err)
                        })
                    }
                }
            })?;
            if let Some(cache) = &self.cache {
                cache
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(cache_key, bytes.clone());
            }
            out.push(Ok(bytes));
        }
        Ok(Some(Box::new(out.into_iter())))
    }

    fn size_key(&self, key: &StoreKey) -> std::result::Result<Option<u64>, StorageError> {
        if self.strict_errors {
            return self.hindsight_head(key).map(|v| v.map(|v| v.size));
        }
        if let Some(cache) = &self.cache {
            let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(size) = cache.sizes.get(key.as_str()) {
                return Ok(*size);
            }
        }
        let url = self.url(key)?;
        let size = retrying(|| {
            let _place = self.lane.permit();
            let response = self
                .request(reqwest::Method::HEAD, &url)
                .map_err(Retry::No)?
                .send()
                .map_err(|err| Retry::Later(failed(err)))?;
            match response.status() {
                StatusCode::OK => response
                    .headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| u64::from_str(value).ok())
                    .map(Some)
                    .ok_or_else(|| Retry::No(StorageError::Other("no content length".to_owned()))),
                StatusCode::NOT_FOUND => Ok(None),
                StatusCode::FORBIDDEN if !self.strict_errors => Ok(None),
                status => {
                    let err = StorageError::Other(format!(
                        "the archive answered {status} for the size of {}",
                        key.as_str()
                    ));
                    Err(if transient(status) {
                        Retry::Later(err)
                    } else {
                        Retry::No(err)
                    })
                }
            }
        })?;
        if let Some(cache) = &self.cache {
            let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
            if cache.sizes.len() >= 4096 {
                cache.sizes.pop_lru();
            }
            cache.sizes.put(key.as_str().to_owned(), size);
        }
        Ok(size)
    }

    fn supports_get_partial(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure in transit is tried again, a refusal is not, and a request
    /// that keeps failing is given up on after a few tries with the last
    /// failure reported.
    #[test]
    fn a_request_is_retried_when_it_fails_in_transit_and_not_when_refused() {
        let mut calls = 0;
        let value = retrying_paced(
            || {
                calls += 1;
                if calls < 3 {
                    Err(Retry::Later(StorageError::Other("reset".into())))
                } else {
                    Ok(calls)
                }
            },
            |_| {},
        )
        .expect("the third try");
        assert_eq!(value, 3);

        let mut calls = 0;
        let refused = retrying(|| -> std::result::Result<(), Retry> {
            calls += 1;
            Err(Retry::No(StorageError::Other("404".into())))
        })
        .expect_err("refused at once");
        assert_eq!(calls, 1);
        assert!(refused.to_string().contains("404"));

        let mut calls = 0;
        let mut pauses = Vec::new();
        let gave_up = retrying_paced(
            || -> std::result::Result<(), Retry> {
                calls += 1;
                Err(Retry::Later(StorageError::Other(format!(
                    "timeout {calls}"
                ))))
            },
            |pause| pauses.push(pause.as_secs()),
        )
        .expect_err("given up");
        assert_eq!(calls, ATTEMPTS);
        assert!(gave_up.to_string().contains("timeout 6"), "{gave_up}");
        assert_eq!(pauses, [1, 2, 4, 8, 16], "the pauses double");
    }

    /// No more than the cap is ever in the lane at once, however many
    /// threads ask, and every place given is given back.
    #[test]
    fn the_lane_admits_a_bounded_number_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let lane = Lane::new(IN_FLIGHT);
        let inside = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..24 {
                scope.spawn(|| {
                    let _place = lane.permit();
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    inside.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        assert!(peak.load(Ordering::SeqCst) <= IN_FLIGHT);
        assert!(peak.load(Ordering::SeqCst) >= 2, "they did overlap");
        assert_eq!(*lane.busy.lock().unwrap(), 0);
    }

    #[test]
    fn transient_statuses_are_the_server_s_not_now() {
        assert!(transient(StatusCode::SERVICE_UNAVAILABLE));
        assert!(transient(StatusCode::TOO_MANY_REQUESTS));
        assert!(transient(StatusCode::REQUEST_TIMEOUT));
        assert!(!transient(StatusCode::NOT_FOUND));
        assert!(!transient(StatusCode::BAD_REQUEST));
    }

    /// The store is read-only and anonymous, and its keys hang off the base
    /// URL exactly as the archive lays them out. A key joined wrongly reads
    /// somebody else's object or nothing at all, and the failure would look
    /// like an empty archive rather than a bug.
    #[test]
    fn a_key_hangs_off_the_base_url() {
        let store = HttpStore::new("https://example.invalid/store.zarr").expect("a URL");
        let url = |key: &str| {
            store
                .url(&StoreKey::new(key).expect("a key"))
                .expect("a URL")
                .to_string()
        };
        assert_eq!(
            url("10m_u_component_of_wind/.zarray"),
            "https://example.invalid/store.zarr/10m_u_component_of_wind/.zarray"
        );
        assert_eq!(
            url(".zmetadata"),
            "https://example.invalid/store.zarr/.zmetadata"
        );
    }

    #[test]
    fn a_base_that_is_not_a_url_is_refused() {
        assert!(HttpStore::new("not a url").is_err());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod hindsight_transport_tests {
    use super::*;
    use std::io::{Read, Write};
    use zarrs_storage::byte_range::ByteRange;

    #[test]
    fn hindsight_reuses_byte_ranges_and_surfaces_access_denied() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            // Three requests: one size, one range, one forbidden object.
            // Re-reading the range must not need another connection.
            for expected in ["HEAD /data", "GET /data", "GET /forbidden"] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(expected), "{request}");
                let response = if expected.starts_with("HEAD") {
                    "HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n"
                } else if expected.ends_with("/data") {
                    assert!(request.to_ascii_lowercase().contains("range: bytes=2-4"));
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 2-4/8\r\nConnection: close\r\n\r\ncde"
                } else {
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let store = HttpStore::s3(&format!("http://127.0.0.1:{port}"), None).unwrap();
        let key = StoreKey::new("data").unwrap();
        for _ in 0..2 {
            let ranges = Box::new(std::iter::once(ByteRange::from(2..5)));
            let bytes = store
                .get_partial_many(&key, ranges)
                .unwrap()
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            assert_eq!(bytes.as_ref(), b"cde");
        }
        let err = store.get(&StoreKey::new("forbidden").unwrap()).unwrap_err();
        assert!(err.to_string().contains("403"));
        server.join().unwrap();
    }
}
