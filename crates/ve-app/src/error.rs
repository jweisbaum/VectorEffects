//! Application-level error type and its IPC wire form.
//!
//! Library crates return typed errors (`thiserror`); this is where they are
//! collapsed for the frontend. The wire form keeps a machine-readable `kind`
//! alongside the message so the UI can branch without parsing English.

use serde::Serialize;
use thiserror::Error;
use ts_rs::TS;

/// Every error that can reach the IPC boundary.
#[derive(Debug, Error)]
pub enum AppError {
    /// A core model or geodesy failure.
    #[error(transparent)]
    Core(#[from] ve_core::CoreError),

    /// A field evaluation or render-cache failure.
    #[error(transparent)]
    Render(#[from] ve_render::RenderError),

    /// A GRIB encoding failure.
    #[error(transparent)]
    Grib(#[from] ve_grib::GribError),

    /// A polar parsing or route solving failure.
    #[error(transparent)]
    Polar(#[from] ve_polar::PolarError),

    /// Filesystem access failed.
    #[error("i/o failed: {0}")]
    Io(#[from] std::io::Error),

    /// Reading or writing application settings failed.
    #[error("settings are malformed: {0}")]
    Settings(#[from] serde_json::Error),

    /// An operation needed an open project and there was none.
    #[error("no project is open")]
    NoProjectOpen,

    /// A saved-project operation needed a path and the project has never been
    /// saved.
    #[error("this project has not been saved yet; choose a location first")]
    ProjectNeverSaved,

    /// Replacing the open project would have discarded unsaved changes.
    ///
    /// The prompt that offers to save lives in the frontend, but the refusal
    /// lives here: a command that silently drops a user's unsaved work should
    /// not be reachable at all, whatever the caller forgot to ask.
    #[error("\"{name}\" has unsaved changes; save or close it first")]
    UnsavedChanges {
        /// The open project's name, for the message.
        name: String,
    },

    /// A value from the frontend was not one of the accepted options.
    #[error("{field} value {value:?} is not valid")]
    BadOption {
        /// Which field.
        field: &'static str,
        /// What was received.
        value: String,
    },

    /// The user stopped an export while it was running.
    #[error("export cancelled")]
    ExportCancelled,

    /// A failure with no more specific classification.
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    /// A stable, machine-readable discriminant for the frontend.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Core(_) => "core",
            Self::Render(_) => "render",
            Self::Grib(_) => "grib",
            Self::Polar(_) => "polar",
            Self::Io(_) => "io",
            Self::Settings(_) => "settings",
            Self::NoProjectOpen => "no-project",
            Self::ProjectNeverSaved => "never-saved",
            Self::UnsavedChanges { .. } => "unsaved-changes",
            Self::BadOption { .. } => "bad-option",
            Self::ExportCancelled => "cancelled",
            Self::Internal(_) => "internal",
        }
    }
}

/// The shape an [`AppError`] takes when it crosses IPC.
#[derive(Debug, Serialize, TS)]
#[ts(export, export_to = "AppErrorPayload.ts")]
pub struct AppErrorPayload {
    /// Stable discriminant, e.g. `"grib"`.
    pub kind: String,
    /// Human-readable message. Not intended for programmatic branching.
    pub message: String,
}

impl Serialize for AppError {
    // Fully qualified: the `Result` alias below shadows `std::result::Result`.
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        AppErrorPayload {
            kind: self.kind().to_owned(),
            message: self.to_string(),
        }
        .serialize(serializer)
    }
}

/// Convenience alias for command results.
pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_with_kind_and_message() {
        let err = AppError::Internal("boom".to_owned());
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "internal");
        assert_eq!(json["message"], "boom");
    }

    #[test]
    fn wraps_library_errors_with_their_own_kind() {
        let err: AppError = ve_core::CoreError::LatitudeOutOfRange(91.0).into();
        assert_eq!(err.kind(), "core");
    }
}
