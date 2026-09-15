//! Application-level error type and its IPC wire form.
//!
//! Library crates return typed errors (`thiserror`); this is where they are
//! collapsed for the frontend. The wire form keeps a machine-readable `kind`
//! alongside the message so the UI can branch without parsing English.
//!
//! **A message names three things** (M59): what the application was trying to
//! do, what it was doing it to, and what went wrong. A library error is only
//! ever the third — `No such file or directory (os error 2)` is true and
//! useless — so the first two are attached at the call site with
//! [`Context::doing`], which is the only place that knows them.

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

    /// Filesystem access failed.
    #[error("A file could not be read or written: {0}")]
    Io(#[from] std::io::Error),

    /// Reading or writing application settings failed.
    #[error("The application settings file could not be read: {0}")]
    Settings(#[from] serde_json::Error),

    /// An operation needed an open project and there was none.
    #[error("No project is open. Open one, or start a new one, and try again.")]
    NoProjectOpen,

    /// A saved-project operation needed a path and the project has never been
    /// saved.
    #[error(
        "This project has never been saved, so there is no file to write to. Use Save As to choose where it should live."
    )]
    ProjectNeverSaved,

    /// Replacing the open project would have discarded unsaved changes.
    ///
    /// The prompt that offers to save lives in the frontend, but the refusal
    /// lives here: a command that silently drops a user's unsaved work should
    /// not be reachable at all, whatever the caller forgot to ask.
    #[error(
        "\"{name}\" has changes that are not saved. Save it, or close it without saving, before opening another."
    )]
    UnsavedChanges {
        /// The open project's name, for the message.
        name: String,
    },

    /// A value from the frontend was not one of the accepted options.
    #[error("The {field} sent was {value:?}, which is not something this accepts.")]
    BadOption {
        /// Which field.
        field: &'static str,
        /// What was received.
        value: String,
    },

    /// The user stopped an export while it was running.
    #[error("The export was stopped before it finished. Nothing was written.")]
    ExportCancelled,

    /// A named step failed, on a named thing.
    ///
    /// The general-purpose contextual error: `doing` is the action in the
    /// infinitive without its "to" ("save the project", "read the image"),
    /// `what` is the file or object it acted on, and `why` is whatever the
    /// underlying failure said. Kept as three fields rather than one formatted
    /// string so the wording stays in one place.
    #[error("Could not {doing} {what}: {why}")]
    Doing {
        /// The action, e.g. `"write"`.
        doing: &'static str,
        /// What it acted on, e.g. a file name.
        what: String,
        /// What the underlying failure said.
        why: String,
    },

    /// A failure with no more specific classification.
    #[error("{0}")]
    Internal(String),
}

/// Attaches what was being done, and to what, to a failure that knows neither.
///
/// `std::io::Error` says a file was not found and never which file; a decoder
/// says a byte was unexpected and never which import it came from. The caller
/// is the only place both halves are known, so this is where they meet.
pub trait Context<T> {
    /// Names the action and its subject. `doing` reads as an infinitive
    /// without "to", so the message comes out as "Could not `doing` `what`".
    fn doing(self, doing: &'static str, what: impl std::fmt::Display) -> Result<T>;
}

impl<T, E: std::fmt::Display> Context<T> for std::result::Result<T, E> {
    fn doing(self, doing: &'static str, what: impl std::fmt::Display) -> Result<T> {
        self.map_err(|why| AppError::Doing {
            doing,
            what: what.to_string(),
            why: why.to_string(),
        })
    }
}

impl AppError {
    /// A stable, machine-readable discriminant for the frontend.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Core(ve_core::CoreError::MissingObject(_)) => "missing-object",
            Self::Core(_) => "core",
            Self::Render(_) => "render",
            Self::Grib(_) => "grib",
            Self::Io(_) => "io",
            Self::Settings(_) => "settings",
            Self::NoProjectOpen => "no-project",
            Self::ProjectNeverSaved => "never-saved",
            Self::UnsavedChanges { .. } => "unsaved-changes",
            Self::BadOption { .. } => "bad-option",
            Self::ExportCancelled => "cancelled",
            Self::Doing { .. } => "doing",
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

    /// The three things a message names (M59): the action, the subject, and
    /// what went wrong. A bare `io::Error` says only the third.
    #[test]
    fn context_names_what_was_being_done_and_to_what() {
        let bare: std::result::Result<(), std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        ));
        let message = bare
            .doing("save the project to", "/tmp/a.veproj")
            .expect_err("the failure carries through")
            .to_string();

        assert!(message.contains("save the project to"), "{message}");
        assert!(message.contains("/tmp/a.veproj"), "{message}");
        assert!(message.contains("permission denied"), "{message}");
    }

    /// A context error is still a serialisable error with a stable kind, so
    /// the frontend branches on it the way it branches on every other one.
    #[test]
    fn context_crosses_ipc_like_any_other() {
        let err: std::result::Result<(), &str> = Err("no such file");
        let err = err.doing("read", "x").expect_err("an error");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "doing");
        assert_eq!(json["message"], "Could not read x: no such file");
    }

    #[test]
    fn wraps_library_errors_with_their_own_kind() {
        let err: AppError = ve_core::CoreError::LatitudeOutOfRange(91.0).into();
        assert_eq!(err.kind(), "core");
    }

    #[test]
    fn missing_object_reads_have_a_distinct_wire_kind() {
        let err: AppError = ve_core::CoreError::MissingObject(50).into();
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "missing-object");
        assert!(json["message"].as_str().unwrap().contains("#50"));
    }
}
