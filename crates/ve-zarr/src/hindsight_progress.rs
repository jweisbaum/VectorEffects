//! Shared observations of a Hindsight read. Transport workers only count
//! actual bytes; the app samples this state without blocking those workers.

use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ReadProgress {
    pub phase: &'static str,
    /// Estimated share of this batch, based on completed work in each phase.
    pub fraction: f32,
    /// Bytes received from the network, including metadata and retries.
    pub downloaded_bytes: u64,
}

#[derive(Debug, Default)]
pub(crate) struct Progress(Mutex<State>);

#[derive(Debug, Default)]
struct State {
    report: ReadProgress,
    base: f32,
    share: f32,
    done: u64,
    total: u64,
}

impl Progress {
    pub fn snapshot(&self) -> ReadProgress {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).report
    }

    pub fn begin(&self, phase: &'static str, base: f32, share: f32, total: u64) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.report.phase = phase;
        state.report.fraction = base;
        state.base = base;
        state.share = share;
        state.done = 0;
        state.total = total;
    }

    pub fn advance(&self, phase: &str, amount: u64) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.report.phase == phase && state.total > 0 {
            state.done = state.done.saturating_add(amount).min(state.total);
            state.report.fraction =
                state.base + state.share * state.done as f32 / state.total as f32;
        }
    }

    pub fn received(&self, bytes: u64) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .report
            .downloaded_bytes += bytes;
    }
}
