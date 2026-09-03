//! Application directory layout.
//!
//! The render cache lives under the OS cache directory and never inside a
//! project file (invariant 1). Deleting the whole cache directory while the app
//! is closed must always be safe and lossless.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::error::{AppError, Result};

/// Resolved locations for everything the app writes outside a project file.
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// User settings.
    pub config_dir: PathBuf,
    /// Evictable render cache. Safe to delete when the app is closed.
    pub cache_dir: PathBuf,
    /// Crash-recovery snapshots.
    pub autosave_dir: PathBuf,
    /// Rolling log files.
    pub log_dir: PathBuf,
}

impl AppPaths {
    /// Resolves the platform-appropriate directories, creating them if needed.
    pub fn resolve() -> Result<Self> {
        let dirs = ProjectDirs::from("com", "VectorEffects", "VectorEffects")
            .ok_or_else(|| AppError::Internal("could not determine a home directory".to_owned()))?;

        let data = dirs.data_dir();
        let paths = Self {
            config_dir: dirs.config_dir().to_path_buf(),
            cache_dir: dirs.cache_dir().join("render"),
            autosave_dir: data.join("autosave"),
            log_dir: data.join("logs"),
        };
        paths.create_all()?;
        Ok(paths)
    }

    /// Builds a layout rooted at `root`, for tests.
    ///
    /// Keeps a lifecycle test out of the real application directories, where it
    /// would clobber the developer's own recent-files list.
    pub fn in_directory(root: &Path) -> Result<Self> {
        let paths = Self {
            config_dir: root.join("config"),
            cache_dir: root.join("cache"),
            autosave_dir: root.join("autosave"),
            log_dir: root.join("logs"),
        };
        paths.create_all()?;
        Ok(paths)
    }

    fn create_all(&self) -> Result<()> {
        for dir in [
            &self.config_dir,
            &self.cache_dir,
            &self.autosave_dir,
            &self.log_dir,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// The settings file path.
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }
}

/// Renders a path for display in the UI. Lossy, and never used to reopen a file.
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
