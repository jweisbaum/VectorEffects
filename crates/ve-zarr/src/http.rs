//! A read-only HTTP store for `zarrs`.
//!
//! `zarrs_http` does this already, but it takes `reqwest` with its default
//! features — `native-tls`, which on Linux is `openssl-sys`, a system C
//! library the three-platform build forbids (CLAUDE.md, invariant 5). The
//! trait is three methods over `GET`, so it is written here against the
//! `reqwest` already in the tree, with rustls.
//!
//! Reads only. Open-data stores are anonymous.

use std::str::FromStr;

use reqwest::header::{CONTENT_LENGTH, HeaderValue, RANGE};
use reqwest::{StatusCode, Url};
use zarrs_storage::Bytes;
use zarrs_storage::byte_range::ByteRangeIterator;
use zarrs_storage::{
    MaybeBytes, MaybeBytesIterator, ReadableStorageTraits, StorageError, StoreKey,
};

use crate::error::{Result, ZarrError};

/// A Zarr store read over HTTPS.
#[derive(Debug)]
pub struct HttpStore {
    base: Url,
    client: reqwest::blocking::Client,
    /// How many requests the host has in flight, and the cap on it. Shared
    /// by every store on the host: the cap is the host's, and a product read
    /// from four stores at once is four stores on one host.
    lane: &'static Lane,
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
        })
    }

    fn request(
        &self,
        method: reqwest::Method,
        url: &Url,
    ) -> std::result::Result<reqwest::blocking::RequestBuilder, StorageError> {
        Ok(self.client.request(method, url.clone()))
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
                StatusCode::NOT_FOUND => Ok(None),
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
        let Some(size) = self.size_key(key)? else {
            return Ok(None);
        };
        let url = self.url(key)?;
        let mut out: Vec<std::result::Result<Bytes, StorageError>> = Vec::new();
        for range in byte_ranges {
            let (start, end) = (range.start(size), range.end(size));
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
            out.push(Ok(bytes));
        }
        Ok(Some(Box::new(out.into_iter())))
    }

    fn size_key(&self, key: &StoreKey) -> std::result::Result<Option<u64>, StorageError> {
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
