//! Tracing setup.
//!
//! Logs go to stderr and to a daily-rolling file under the app log directory,
//! whose location is surfaced in the UI so a user can find it without being
//! told where their platform hides it.

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

/// Initialises logging. The returned guard must be held for the process
/// lifetime; dropping it stops the background writer and loses buffered lines.
#[must_use]
pub fn init(log_dir: &std::path::Path) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_env("VE_LOG")
        .or_else(|_| EnvFilter::try_new("info,ve_app=debug,ve_render=debug"))
        .unwrap_or_default();

    let file_appender = tracing_appender::rolling::daily(log_dir, "vectoreffects.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(file_writer),
        );

    if registry.try_init().is_err() {
        // Already initialised, which happens under `cargo test`. Not fatal.
        return None;
    }
    Some(guard)
}
