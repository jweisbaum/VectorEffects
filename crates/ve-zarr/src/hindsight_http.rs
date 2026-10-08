//! Hindsight-only transport: version checks, persistent bytes, coalesced
//! ranges and bounded parallel GETs. Open Data never enters these methods.

use std::hash::{Hash, Hasher};
use std::io::Read;
use std::ops::Range;

use super::*;
use reqwest::header::{CONTENT_RANGE, ETAG, IF_MATCH, IF_NONE_MATCH};

const REQUESTS: usize = 8;
const MERGE_BYTES: u64 = 8 * 1024 * 1024;
const MERGE_GAP: u64 = 16 * 1024;

fn refusal(status: StatusCode, key: &StoreKey) -> Retry {
    let err = StorageError::Other(format!(
        "the archive answered {status} for {}",
        key.as_str()
    ));
    if transient(status) {
        Retry::Later(err)
    } else {
        Retry::No(err)
    }
}

fn etag(response: &reqwest::blocking::Response) -> Option<String> {
    response
        .headers()
        .get(ETAG)?
        .to_str()
        .ok()
        .map(str::to_owned)
}

impl HttpStore {
    fn gate(&self, key: &StoreKey) -> std::sync::MutexGuard<'_, ()> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        key.as_str().hash(&mut hash);
        self.range_gates[hash.finish() as usize % self.range_gates.len()]
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    fn disk_get(&self, key: &str) -> Option<crate::hindsight_cache::Cached> {
        self.disk
            .as_ref()?
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(key)
    }

    fn disk_put(&self, key: &str, etag: &str, bytes: &Bytes) {
        if let Some(cache) = &self.disk {
            cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .put(key, etag, bytes);
        }
    }

    pub(super) fn hindsight_get(
        &self,
        key: &StoreKey,
    ) -> std::result::Result<MaybeBytes, StorageError> {
        let url = self.url(key)?;
        let cache_key = format!("get|{url}");
        let cached = self.disk_get(&cache_key);
        retrying(|| {
            let _permit = self.lane.permit();
            let mut request = self
                .request(reqwest::Method::GET, &url)
                .map_err(Retry::No)?;
            if let Some(cached) = &cached {
                request = request.header(IF_NONE_MATCH, &cached.etag);
            }
            let response = request.send().map_err(|e| Retry::Later(failed(e)))?;
            match response.status() {
                StatusCode::NOT_MODIFIED if cached.is_some() => {
                    Ok(cached.as_ref().map(|c| c.bytes.clone()))
                }
                StatusCode::NOT_FOUND => Ok(None),
                StatusCode::OK => {
                    let version = etag(&response);
                    let bytes = response.bytes().map_err(|e| Retry::Later(failed(e)))?;
                    if let Some(progress) = &self.progress {
                        progress.received(bytes.len() as u64);
                    }
                    if let Some(version) = version {
                        self.disk_put(&cache_key, &version, &bytes);
                    }
                    Ok(Some(bytes))
                }
                status => Err(refusal(status, key)),
            }
        })
    }

    pub(super) fn hindsight_head(
        &self,
        key: &StoreKey,
    ) -> std::result::Result<Option<ObjectVersion>, StorageError> {
        let _gate = self.gate(key);
        if let Some(cache) = &self.cache
            && let Some(version) = cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .versions
                .get(key.as_str())
        {
            return Ok(version.clone());
        }
        let url = self.url(key)?;
        let version = retrying(|| {
            let _permit = self.lane.permit();
            let response = self
                .request(reqwest::Method::HEAD, &url)
                .map_err(Retry::No)?
                .send()
                .map_err(|e| Retry::Later(failed(e)))?;
            match response.status() {
                StatusCode::OK => {
                    let size = response
                        .headers()
                        .get(CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| {
                            Retry::No(StorageError::Other("no content length".into()))
                        })?;
                    Ok(Some(ObjectVersion {
                        size,
                        etag: etag(&response).filter(|s| !s.starts_with("W/")),
                    }))
                }
                StatusCode::NOT_FOUND => Ok(None),
                status => Err(refusal(status, key)),
            }
        })?;
        if let Some(cache) = &self.cache {
            let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
            if cache.versions.len() >= 4096 {
                cache.versions.pop_lru();
            }
            cache.versions.put(key.as_str().to_owned(), version.clone());
        }
        Ok(version)
    }

    fn fetch_range(
        &self,
        key: &StoreKey,
        version: &ObjectVersion,
        range: &Range<u64>,
        missing: &[(usize, Range<u64>)],
    ) -> std::result::Result<Bytes, StorageError> {
        let url = self.url(key)?;
        // A retried transfer may replay its first bytes. Count useful work
        // only past the previous high-water mark, and never count merge gaps.
        let mut credited_end = range.start;
        retrying(|| {
            let _permit = self.lane.permit();
            let mut request = self
                .request(reqwest::Method::GET, &url)
                .map_err(Retry::No)?
                .header(RANGE, format!("bytes={}-{}", range.start, range.end - 1));
            if let Some(etag) = &version.etag {
                request = request.header(IF_MATCH, etag);
            }
            let mut response = request.send().map_err(|e| Retry::Later(failed(e)))?;
            let status = response.status();
            if status != StatusCode::PARTIAL_CONTENT && status != StatusCode::OK {
                return Err(refusal(status, key));
            }
            if let (Some(expected), Some(actual)) = (&version.etag, etag(&response))
                && expected != &actual
            {
                return Err(Retry::No(StorageError::Other(
                    "Hindsight object changed during the import; retry the import".into(),
                )));
            }
            if status == StatusCode::PARTIAL_CONTENT {
                let expected = format!("bytes {}-{}/{}", range.start, range.end - 1, version.size);
                if response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    != Some(expected.as_str())
                {
                    return Err(Retry::No(StorageError::Other(
                        "invalid Hindsight Content-Range".into(),
                    )));
                }
            }
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 64 * 1024];
            let mut cursor = if status == StatusCode::OK {
                0
            } else {
                range.start
            };
            loop {
                let count = response
                    .read(&mut buffer)
                    .map_err(|e| Retry::Later(StorageError::Other(e.to_string())))?;
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
                let end = cursor + count as u64;
                if let Some(progress) = &self.progress {
                    progress.received(count as u64);
                    let start = cursor.max(credited_end);
                    let ready: u64 = missing
                        .iter()
                        .map(|(_, wanted)| {
                            end.min(wanted.end)
                                .min(range.end)
                                .saturating_sub(start.max(wanted.start).max(range.start))
                        })
                        .sum();
                    progress.advance("downloading", ready);
                }
                credited_end = credited_end.max(end.min(range.end));
                cursor = end;
            }
            let bytes = Bytes::from(bytes);
            if status == StatusCode::OK {
                if bytes.len() as u64 != version.size {
                    return Err(Retry::No(StorageError::Other(
                        "incorrect Hindsight object length".into(),
                    )));
                }
                Ok(Bytes::copy_from_slice(
                    &bytes[range.start as usize..range.end as usize],
                ))
            } else if bytes.len() as u64 == range.end - range.start {
                Ok(bytes)
            } else {
                Err(Retry::Later(StorageError::Other(
                    "truncated Hindsight range".into(),
                )))
            }
        })
    }

    pub(super) fn hindsight_ranges<'a>(
        &'a self,
        key: &StoreKey,
        ranges: ByteRangeIterator<'a>,
    ) -> std::result::Result<MaybeBytesIterator<'a>, StorageError> {
        let Some(version) = self.hindsight_head(key)? else {
            return Ok(None);
        };
        let ranges: Vec<_> = ranges
            .map(|r| r.start(version.size)..r.end(version.size))
            .collect();
        if ranges
            .iter()
            .any(|r| r.start > r.end || r.end > version.size)
        {
            return Err(StorageError::Other(
                "Hindsight range is outside the object".into(),
            ));
        }
        // A shard's prefetch supplies all its wanted ranges together. Coalesce
        // equal misses between workers, without blocking unrelated shards.
        let _gate = self.gate(key);
        let mut out = vec![None; ranges.len()];
        let mut missing = Vec::new();
        for (i, range) in ranges.iter().enumerate() {
            if range.is_empty() {
                out[i] = Some(Bytes::new());
                continue;
            }
            let memory_key = (key.as_str().to_owned(), range.start, range.end);
            if let Some(cache) = &self.cache {
                out[i] = cache
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .ranges
                    .get(&memory_key)
                    .cloned();
            }
            if out[i].is_none()
                && let Some(etag) = &version.etag
            {
                let disk_key = format!(
                    "range|{}|{}|{etag}|{}|{}",
                    self.base,
                    key.as_str(),
                    range.start,
                    range.end
                );
                out[i] = self
                    .disk_get(&disk_key)
                    .filter(|c| c.etag == *etag && c.bytes.len() as u64 == range.end - range.start)
                    .map(|c| c.bytes);
            }
            if let Some(bytes) = &out[i] {
                if let Some(progress) = &self.progress {
                    progress.advance("downloading", bytes.len() as u64);
                }
                if let Some(cache) = &self.cache {
                    cache
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(memory_key, bytes.clone());
                }
            } else {
                missing.push((i, range.clone()));
            }
        }
        let merged = merge_ranges(&missing);
        for group in merged.chunks(REQUESTS) {
            let downloaded = std::thread::scope(|scope| {
                let jobs: Vec<_> = group
                    .iter()
                    .map(|range| scope.spawn(|| self.fetch_range(key, &version, range, &missing)))
                    .collect();
                jobs.into_iter()
                    .map(|j| j.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
                    .collect::<std::result::Result<Vec<_>, _>>()
            })?;
            for (range, bytes) in group.iter().zip(downloaded) {
                for (i, wanted) in &missing {
                    if out[*i].is_none() && wanted.start >= range.start && wanted.end <= range.end {
                        // Copy so accounting includes the entire allocation,
                        // even after neighbouring ranges have been evicted.
                        let part = Bytes::copy_from_slice(
                            &bytes[(wanted.start - range.start) as usize
                                ..(wanted.end - range.start) as usize],
                        );
                        if let Some(cache) = &self.cache {
                            cache.lock().unwrap_or_else(|p| p.into_inner()).insert(
                                (key.as_str().to_owned(), wanted.start, wanted.end),
                                part.clone(),
                            );
                        }
                        if let Some(etag) = &version.etag {
                            let disk_key = format!(
                                "range|{}|{}|{etag}|{}|{}",
                                self.base,
                                key.as_str(),
                                wanted.start,
                                wanted.end
                            );
                            self.disk_put(&disk_key, etag, &part);
                        }
                        out[*i] = Some(part);
                    }
                }
            }
        }
        tracing::debug!(
            key = key.as_str(),
            ranges = ranges.len(),
            misses = missing.len(),
            requests = merged.len(),
            "Hindsight ranges"
        );
        Ok(Some(Box::new(out.into_iter().map(|b| {
            b.ok_or_else(|| StorageError::Other("Hindsight range was not returned".into()))
        }))))
    }
}

fn merge_ranges(missing: &[(usize, Range<u64>)]) -> Vec<Range<u64>> {
    let mut ranges: Vec<_> = missing.iter().map(|(_, r)| r.clone()).collect();
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<u64>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end.saturating_add(MERGE_GAP)
            && range.end.max(last.end) - last.start <= MERGE_BYTES
        {
            last.end = last.end.max(range.end);
            continue;
        }
        merged.push(range);
    }
    merged
}

#[cfg(test)]
#[path = "hindsight_http_tests.rs"]
mod tests;
