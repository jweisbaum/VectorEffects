//! Evictable, versioned HTTP bytes used only by Hindsight. Cache failures are
//! misses, never import failures. No credentials or signed URLs are stored.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::SystemTime;

use sha2::{Digest, Sha256};
use zarrs_storage::Bytes;

const LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRY: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 16_384;
type Shared = Arc<Mutex<DiskCache>>;
static CACHES: LazyLock<Mutex<HashMap<PathBuf, Weak<Mutex<DiskCache>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug)]
pub(crate) struct Cached {
    pub etag: String,
    pub bytes: Bytes,
}

#[derive(Debug)]
pub(crate) struct DiskCache {
    root: PathBuf,
    entries: lru::LruCache<String, u64>,
    bytes: u64,
    limit: u64,
}

impl DiskCache {
    pub fn open(root: &Path) -> Option<Shared> {
        std::fs::create_dir_all(root).ok()?;
        let root = root.canonicalize().ok()?;
        let mut shared = CACHES.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(cache) = shared.get(&root).and_then(Weak::upgrade) {
            return Some(cache);
        }
        shared.retain(|_, cache| cache.strong_count() > 0);
        let cache = Arc::new(Mutex::new(Self::load(root.clone(), LIMIT)?));
        shared.insert(root, Arc::downgrade(&cache));
        Some(cache)
    }

    fn load(root: PathBuf, limit: u64) -> Option<Self> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&root).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_file() {
                files.push((
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    name,
                    meta.len(),
                ));
            }
        }
        files.sort();
        let mut cache = Self {
            root,
            entries: lru::LruCache::unbounded(),
            bytes: 0,
            limit,
        };
        for (_, name, len) in files {
            cache.entries.put(name, len);
            cache.bytes += len;
        }
        cache.prune(0);
        Some(cache)
    }

    pub fn get(&mut self, key: &str) -> Option<Cached> {
        let name = digest(key.as_bytes());
        let len = *self.entries.get(&name)?;
        let path = self.root.join(&name);
        let read = || -> Option<Cached> {
            if len > (MAX_ENTRY + 4096) as u64 {
                return None;
            }
            let data = std::fs::read(&path).ok()?;
            if data.get(..5)? != b"VEHC1" {
                return None;
            }
            let n = u32::from_le_bytes(data.get(5..9)?.try_into().ok()?) as usize;
            if n > 4096 {
                return None;
            }
            let etag = String::from_utf8(data.get(9..9 + n)?.to_vec()).ok()?;
            reqwest::header::HeaderValue::from_bytes(etag.as_bytes()).ok()?;
            let checksum = data.get(9 + n..41 + n)?;
            let body = data.get(41 + n..)?;
            if fingerprint(&etag, body).as_slice() != checksum {
                return None;
            }
            Some(Cached {
                etag,
                bytes: Bytes::copy_from_slice(body),
            })
        };
        match read() {
            Some(value) => {
                if let Ok(file) = std::fs::File::options().write(true).open(path) {
                    let _ = file.set_modified(SystemTime::now());
                }
                Some(value)
            }
            None => {
                self.remove(&name);
                None
            }
        }
    }

    pub fn put(&mut self, key: &str, etag: &str, bytes: &Bytes) {
        if bytes.len() > MAX_ENTRY || etag.len() > 4096 {
            return;
        }
        let size = (41 + etag.len() + bytes.len()) as u64;
        if size > self.limit {
            return;
        }
        let name = digest(key.as_bytes());
        let write = || -> std::io::Result<()> {
            let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
            file.write_all(b"VEHC1")?;
            file.write_all(&(etag.len() as u32).to_le_bytes())?;
            file.write_all(etag.as_bytes())?;
            file.write_all(&fingerprint(etag, bytes))?;
            file.write_all(bytes)?;
            file.persist(self.root.join(&name)).map_err(|e| e.error)?;
            Ok(())
        };
        if write().is_ok() {
            if let Some(old) = self.entries.pop(&name) {
                self.bytes -= old;
            }
            self.prune(size);
            self.entries.put(name, size);
            self.bytes += size;
        }
    }

    fn remove(&mut self, name: &str) {
        if let Some(size) = self.entries.pop(name) {
            self.bytes -= size;
        }
        let _ = std::fs::remove_file(self.root.join(name));
    }

    fn prune(&mut self, extra: u64) {
        while self.bytes + extra > self.limit || self.entries.len() >= MAX_ENTRIES {
            let Some((name, size)) = self.entries.pop_lru() else {
                break;
            };
            self.bytes -= size;
            let _ = std::fs::remove_file(self.root.join(name));
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn fingerprint(etag: &str, bytes: &[u8]) -> [u8; 32] {
    Sha256::new()
        .chain_update((etag.len() as u32).to_le_bytes())
        .chain_update(etag.as_bytes())
        .chain_update(bytes)
        .finalize()
        .into()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn cache_survives_reopening_evicts_and_rejects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = DiskCache::load(dir.path().to_owned(), 100).unwrap();
        cache.put("provider/a", "v1", &Bytes::from_static(b"a"));
        cache.put("provider/b", "v1", &Bytes::from_static(b"b"));
        assert!(cache.get("provider/a").is_some());
        cache.put("provider/c", "v1", &Bytes::from_static(b"c"));
        assert!(cache.get("provider/b").is_none());
        drop(cache);
        let mut cache = DiskCache::load(dir.path().to_owned(), 100).unwrap();
        assert_eq!(cache.get("provider/a").unwrap().bytes.as_ref(), b"a");
        assert!(cache.get("other-provider/a").is_none());
        let path = dir.path().join(digest(b"provider/c"));
        let mut corrupt = std::fs::read(&path).unwrap();
        corrupt[10] = b'2'; // A valid-looking but incorrect object version.
        std::fs::write(&path, corrupt).unwrap();
        assert!(
            cache.get("provider/c").is_none(),
            "the checksum protects the version as well as the body"
        );
        std::fs::write(dir.path().join(digest(b"provider/a")), b"broken").unwrap();
        assert!(cache.get("provider/a").is_none());
        assert!(cache.bytes <= 100);
    }
}
