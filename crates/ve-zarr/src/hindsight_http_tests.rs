//! Local HTTP tests: request counts and data correctness, independent of a
//! mirror's availability, credentials or network timing.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(
    clippy::single_range_in_vec_init,
    reason = "the HTTP interface takes lists of byte ranges"
)]

use super::*;
use std::io::{Read, Write};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use zarrs_storage::byte_range::ByteRange;

struct Server {
    url: String,
    body: Arc<Mutex<(String, Vec<u8>)>>,
    requests: Arc<Mutex<Vec<String>>>,
    peak: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    hold_body: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn new() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        listener.set_nonblocking(true).unwrap();
        let body = Arc::new(Mutex::new((
            String::from("\"v1\""),
            (0..100_000).map(|i| (i % 251) as u8).collect::<Vec<_>>(),
        )));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let peak = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let hold_body = Arc::new(AtomicBool::new(false));
        let hold = hold_body.clone();
        let (b, r, p, a, s) = (
            body.clone(),
            requests.clone(),
            peak.clone(),
            active.clone(),
            stop.clone(),
        );
        let thread = std::thread::spawn(move || {
            let mut connections = Vec::new();
            while !s.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                };
                let (b, r, p, a) = (b.clone(), r.clone(), p.clone(), a.clone());
                let hold = hold.clone();
                connections.push(std::thread::spawn(move || {
                    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") { stream.read_exact(&mut byte).unwrap(); request.push(byte[0]); }
                    let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                    r.lock().unwrap().push(request.clone());
                    let (etag, bytes) = b.lock().unwrap().clone();
                    let header = |name: &str| request.lines().find_map(|l| l.strip_prefix(name)).map(str::trim);
                    let (mut status, mut payload, mut extra) = ("200 OK", bytes.clone(), String::new());
                    if header("if-none-match:") == Some(etag.as_str()) {
                        status = "304 Not Modified"; payload.clear();
                    } else if header("if-match:").is_some_and(|h| h != etag) {
                        status = "412 Precondition Failed"; payload.clear();
                    } else if let Some(range) = header("range:") {
                        let (start, end) = range.strip_prefix("bytes=").unwrap().split_once('-').unwrap();
                        let (start, end) = (start.parse::<usize>().unwrap(), end.parse::<usize>().unwrap());
                        status = "206 Partial Content";
                        payload = bytes[start..=end].to_vec();
                        extra = format!("Content-Range: bytes {start}-{end}/{}\r\n", bytes.len());
                        let count = a.fetch_add(1, Ordering::SeqCst) + 1;
                        p.fetch_max(count, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(30));
                        a.fetch_sub(1, Ordering::SeqCst);
                    }
                    let response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nETag: {etag}\r\n{extra}Connection: close\r\n\r\n", payload.len());
                    stream.write_all(response.as_bytes()).unwrap();
                    if !request.starts_with("head ") {
                        let half = payload.len() / 2;
                        stream.write_all(&payload[..half]).unwrap();
                        stream.flush().unwrap();
                        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
                        while hold.load(Ordering::Relaxed) && std::time::Instant::now() < until {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        stream.write_all(&payload[half..]).unwrap();
                    }
                }));
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
            url,
            body,
            requests,
            peak,
            stop,
            hold_body,
            thread: Some(thread),
        }
    }

    fn ranges(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.contains("\r\nrange:"))
            .count()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn read(store: &HttpStore, key: &str, ranges: &[Range<u64>]) -> Vec<Bytes> {
    store
        .get_partial_many(
            &StoreKey::new(key).unwrap(),
            Box::new(ranges.iter().cloned().map(ByteRange::from)),
        )
        .unwrap()
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn ranges_are_coalesced_parallel_ordered_and_cached_only_for_hindsight() {
    let server = Server::new();
    let store = HttpStore::s3(&server.url, None).unwrap();
    let ranges = [50_000..50_004, 2..5, 7..10, 2..5];
    let result = read(&store, "data", &ranges);
    let body = server.body.lock().unwrap().1.clone();
    for (range, result) in ranges.iter().zip(result) {
        assert_eq!(
            result.as_ref(),
            &body[range.start as usize..range.end as usize]
        );
    }
    assert_eq!(server.ranges(), 2, "four ranges become two GETs");
    assert!(
        server.peak.load(Ordering::Relaxed) >= 2,
        "independent ranges overlap"
    );
    read(&store, "data", &ranges);
    assert_eq!(server.ranges(), 2, "memory hits issue no GETs");
    let open = HttpStore::new(&server.url).unwrap();
    assert!(open.disk.is_none());
    assert!(open.cache.is_none());
    read(&open, "open", &[2..5, 7..10]);
    assert_eq!(
        server.ranges(),
        4,
        "Open Data keeps separate range requests"
    );
}

#[test]
fn reports_partial_transfer_before_completion_and_credits_cache_hits() {
    let server = Server::new();
    let progress = Arc::new(crate::hindsight_progress::Progress::default());
    let store = HttpStore::s3(&server.url, None)
        .unwrap()
        .with_progress(progress.clone());
    progress.begin("downloading", 0.0, 1.0, 100_000);
    server.hold_body.store(true, Ordering::Relaxed);
    let partial = std::thread::scope(|scope| {
        let worker = scope.spawn(|| read(&store, "data", &[0..100_000]));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while progress.snapshot().downloaded_bytes == 0 && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let partial = progress.snapshot();
        let finished = worker.is_finished();
        server.hold_body.store(false, Ordering::Relaxed);
        assert_eq!(worker.join().unwrap()[0].len(), 100_000);
        assert!(!finished, "progress must precede the response's end");
        partial
    });
    assert!(partial.downloaded_bytes > 0 && partial.downloaded_bytes < 100_000);
    assert!(partial.fraction > 0.0 && partial.fraction < 1.0);
    assert_eq!(progress.snapshot().fraction, 1.0);
    assert_eq!(progress.snapshot().downloaded_bytes, 100_000);
    progress.begin("downloading", 0.0, 1.0, 100_000);
    read(&store, "data", &[0..100_000]);
    assert_eq!(
        progress.snapshot().fraction,
        1.0,
        "cached work still advances"
    );
    assert_eq!(
        progress.snapshot().downloaded_bytes,
        100_000,
        "cache hits are not network bytes"
    );
    assert_eq!(server.ranges(), 1);
}

#[test]
fn disk_bytes_revalidate_across_readers_and_upstream_changes() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let open = || {
        HttpStore::s3(&server.url, None)
            .unwrap()
            .with_disk_cache(root.path())
    };
    let key = StoreKey::new("metadata").unwrap();
    let first = open();
    let original = first.get(&key).unwrap().unwrap();
    assert_eq!(read(&first, "data", &[0..3])[0].as_ref(), &[0, 1, 2]);
    drop(first);
    let second = open();
    assert_eq!(second.get(&key).unwrap().unwrap(), original);
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("if-none-match: \"v1\""))
    );
    assert_eq!(read(&second, "data", &[0..3])[0].as_ref(), &[0, 1, 2]);
    assert_eq!(
        server.ranges(),
        1,
        "new reader validates HEAD then reuses disk bytes"
    );
    drop(second);
    *server.body.lock().unwrap() = ("\"v2\"".into(), vec![42; 100_000]);
    let third = open();
    assert_eq!(third.get(&key).unwrap().unwrap()[0], 42);
    assert_eq!(read(&third, "data", &[0..3])[0].as_ref(), &[42, 42, 42]);
    assert_eq!(server.ranges(), 2, "changed ETag downloads fresh bytes");
    *server.body.lock().unwrap() = ("\"v3\"".into(), vec![43; 100_000]);
    let err = third.get_partial_many(
        &StoreKey::new("data").unwrap(),
        Box::new(std::iter::once(ByteRange::from(3..6))),
    );
    assert!(
        err.err().unwrap().to_string().contains("412"),
        "mid-import replacement cannot mix shard versions"
    );
}

#[test]
fn merged_requests_remain_bounded() {
    let groups = merge_ranges(&[(0, 0..MERGE_BYTES), (1, MERGE_BYTES..2 * MERGE_BYTES)]);
    assert_eq!(groups.len(), 2);
}
