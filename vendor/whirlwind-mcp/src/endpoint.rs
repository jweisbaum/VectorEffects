//! The endpoint file and the app lock (MCP plugins spec, sections 2.1 and 3.1).
use anyhow::Error as AppError;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

pub const DIR: &str = "mcp";
pub const FILE: &str = "endpoint.json";
pub const PARTIAL: &str = "endpoint.json.partial";
pub const LOCK: &str = "app.lock";

/// `{"version":1,"port":…,"token":"…","pid":…}`; no other fields.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub version: u32,
    pub port: u16,
    pub token: String,
    pub pid: u32,
}

// The token never reaches a log or a test failure message.
impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("version", &self.version)
            .field("port", &self.port)
            .field("token", &"[hidden]")
            .field("pid", &self.pid)
            .finish()
    }
}

/// 64 lowercase hex characters (244 random bits) from two v4 UUIDs.
pub fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Creates the directory, mode 0700 on Unix.
pub fn create_dir(dir: &Path) -> Result<(), AppError> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Writes `endpoint.json` through a 0600 `.partial` file that is synced and renamed into place,
/// so the token is never world-readable, not even briefly.
pub fn write(dir: &Path, endpoint: &Endpoint) -> Result<(), AppError> {
    create_dir(dir)?;
    let partial = dir.join(PARTIAL);
    let _ = fs::remove_file(&partial);
    let result = (|| -> Result<(), AppError> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&partial)?;
        file.write_all(&serde_json::to_vec(endpoint)?)?;
        file.sync_all()?;
        fs::rename(&partial, dir.join(FILE))?;
        #[cfg(unix)]
        fs::File::open(dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

/// The endpoint, or `None` when the file is missing or unparsable.
/// A file of another version counts as missing.
pub fn read(dir: &Path) -> Option<Endpoint> {
    serde_json::from_slice::<Endpoint>(&fs::read(dir.join(FILE)).ok()?)
        .ok()
        .filter(|found| found.version == 1)
}

pub fn remove(dir: &Path) {
    let _ = fs::remove_file(dir.join(FILE));
}

/// Takes the app lock, `{dir}/app.lock`. `Ok(None)` means another process holds it. The app
/// keeps the returned file open for its whole life, so the bridge can tell it is running.
pub fn try_lock(dir: &Path) -> Result<Option<fs::File>, AppError> {
    lock_with(dir, FileExt::try_lock_exclusive)
}

/// The bridge's probe: a shared lock on the app lock file, so two bridges probing at once do not
/// see each other as the app. The app's exclusive `take_app_lock` retries past a probe.
pub fn probe_lock(dir: &Path) -> Result<Option<fs::File>, AppError> {
    lock_with(dir, FileExt::try_lock_shared)
}

fn lock_with(
    dir: &Path,
    lock: fn(&fs::File) -> std::io::Result<()>,
) -> Result<Option<fs::File>, AppError> {
    create_dir(dir)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK))?;
    match lock(&file) {
        Ok(()) => Ok(Some(file)),
        // Compare OS codes: on Windows the contended error has no specific `ErrorKind`.
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

/// Whether another process holds the app lock, which means the app is running. A free lock is
/// taken shared and released at once.
pub fn lock_held(dir: &Path) -> bool {
    matches!(probe_lock(dir), Ok(None))
}
