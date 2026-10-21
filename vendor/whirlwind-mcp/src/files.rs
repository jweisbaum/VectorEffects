//! Recoverable replacement for generated files. Never silently destroy the old output.
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
pub fn preserve_before_replace(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let path = std::fs::canonicalize(path)?;
    ensure!(
        path.is_file(),
        "The output already exists and is not a regular file"
    );
    let parent = path.parent().context("Output has no parent")?;
    let name = path
        .file_name()
        .context("Output has no name")?
        .to_string_lossy();
    let backup = parent.join(format!(".{name}.whirlwind-backup-{}", uuid::Uuid::new_v4()));
    std::fs::copy(&path, &backup)?;
    std::fs::File::open(&backup)?.sync_all()?;
    Ok(Some(backup))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_keeps_original() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("output");
        std::fs::write(&p, "original").unwrap();
        let b = preserve_before_replace(&p).unwrap().unwrap();
        std::fs::write(p, "new").unwrap();
        assert_eq!(std::fs::read_to_string(b).unwrap(), "original");
    }
}
