//! The person's NASA Earthdata token (spec.md 4.10, M97): what CMC's
//! downloads from PO.DAAC are made with.
//!
//! **It is not in the settings file.** The settings travel whole to the
//! webview and, through `invoke`'s `app_settings`, to any agent the MCP
//! service admits; a NASA credential belongs to neither. So it lives in a
//! file of its own beside the settings, readable by its owner only, and
//! what leaves this module is whether one is set — never the token. It is
//! never logged and never enters a project.

use std::path::Path;

use crate::commands::AppState;
use crate::error::Result;
use crate::paths::AppPaths;

const FILE: &str = "earthdata-token";

fn path(paths: &AppPaths) -> std::path::PathBuf {
    paths.config_dir.join(FILE)
}

/// The token, if one is set.
pub fn read(paths: &AppPaths) -> Option<String> {
    let token = std::fs::read_to_string(path(paths)).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

/// Sets the token; an empty one removes it. Answers whether one is set.
pub fn write(paths: &AppPaths, token: &str) -> Result<bool> {
    let file = path(paths);
    let token = token.trim();
    if token.is_empty() {
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        return Ok(false);
    }
    std::fs::create_dir_all(&paths.config_dir)?;
    owner_only_write(&file, token.as_bytes())?;
    Ok(true)
}

/// Writes a file only its owner can read: created with that mode, so there
/// is no moment it is readable by anyone else, and narrowed again in case
/// it existed already.
#[cfg(unix)]
fn owner_only_write(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(file)?;
    out.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    out.write_all(bytes)
}

/// On Windows a file in the profile's application data is the user's own.
#[cfg(not(unix))]
fn owner_only_write(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(file, bytes)
}

/// Whether a token is set — all the interface is told.
#[tauri::command]
pub fn earthdata_status(state: tauri::State<'_, AppState>) -> Result<bool> {
    Ok(read(&state.paths).is_some())
}

/// Sets the token, or clears it when empty. Answers whether one is set.
#[tauri::command]
pub fn set_earthdata_token(state: tauri::State<'_, AppState>, token: String) -> Result<bool> {
    write(&state.paths, &token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_kept_trimmed_and_cleared_by_an_empty_one() {
        let dir = tempfile::tempdir().expect("dir");
        let paths = AppPaths::in_directory(dir.path()).expect("paths");
        assert_eq!(read(&paths), None);
        assert!(write(&paths, "  eyJ0eXAi.secret \n").expect("set"));
        assert_eq!(read(&paths).as_deref(), Some("eyJ0eXAi.secret"));
        assert!(!write(&paths, "   ").expect("cleared"));
        assert_eq!(read(&paths), None);
        assert!(
            !write(&paths, "").expect("cleared twice"),
            "clearing nothing is fine"
        );
    }

    /// The settings file never holds it, whatever is saved after.
    #[test]
    fn the_token_is_not_in_the_settings_file() {
        let dir = tempfile::tempdir().expect("dir");
        let paths = AppPaths::in_directory(dir.path()).expect("paths");
        write(&paths, "secret-token").expect("set");
        assert_ne!(path(&paths), paths.settings_file());
        let settings =
            serde_json::to_string(&crate::settings::AppSettings::default()).expect("json");
        assert!(!settings.contains("secret-token"));
        assert!(
            !settings.contains("earthdata"),
            "no field for it: {settings}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_its_owner_can_read_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("dir");
        let paths = AppPaths::in_directory(dir.path()).expect("paths");
        std::fs::create_dir_all(&paths.config_dir).expect("dir");
        std::fs::write(path(&paths), "old").expect("a looser file first");
        std::fs::set_permissions(path(&paths), std::fs::Permissions::from_mode(0o644))
            .expect("chmod");
        write(&paths, "secret-token").expect("set");
        let mode = std::fs::metadata(path(&paths))
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
