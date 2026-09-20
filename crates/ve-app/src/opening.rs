//! How far along an opening is, for the loading page (spec.md 4.7).
//!
//! Opening a project is reading its document and then every file its imported
//! layers name, again, because the project holds their paths and never their
//! samples (invariants 1 and 2). A month of hourly wind is most of a minute of
//! that, and a spinner cannot tell a long read from a stalled one. Every source
//! says how many frames it holds before it decodes the first, so the bar is a
//! real fraction rather than an animation.
//!
//! The sink lives on [`AppState`](crate::commands::AppState) rather than in
//! any signature: the application installs one that emits `open://progress`,
//! and a headless caller — a test, the MCP service — installs nothing and
//! pays nothing.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The share of the bar given to reading the document itself.
///
/// Small on purpose: the archive is parsed in tens of milliseconds and the
/// files its layers name are what take the time. A project with no imported
/// layer goes from here straight to the end.
const DOCUMENT_SHARE: f32 = 0.05;

/// Progress, emitted as the `open://progress` event.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export, export_to = "OpenProgress.ts")]
pub struct OpenProgress {
    /// What is being read now: the document, or a source by its file name.
    pub label: String,
    /// Frames of that source finished. Zero with a zero `total` for something
    /// that has none to count.
    pub done: u32,
    /// Frames that source holds.
    pub total: u32,
    /// The whole opening, from 0 to 1. Never decreases within one opening.
    pub fraction: f32,
}

type Listener = Arc<dyn Fn(OpenProgress) + Send + Sync>;

/// Where progress goes. Empty unless the application installed a listener.
#[derive(Default)]
pub struct Sink(Mutex<Option<Listener>>);

impl std::fmt::Debug for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sink").finish_non_exhaustive()
    }
}

impl Sink {
    /// Sends every later opening's progress to `listener`.
    pub fn on_progress(&self, listener: impl Fn(OpenProgress) + Send + Sync + 'static) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(listener));
    }

    /// Starts following one opening.
    pub fn begin(&self) -> Opening {
        Opening {
            listener: self
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            sources: Vec::new(),
            reported: Mutex::new(0.0),
        }
    }
}

/// One source's count, written from whichever threads are decoding it.
#[derive(Debug, Default)]
struct Count {
    done: AtomicU32,
    total: AtomicU32,
}

/// One opening in flight: the document, then its sources.
pub struct Opening {
    listener: Option<Listener>,
    sources: Vec<(String, Count)>,
    /// The last fraction sent. Held from the counting to the sending, so that
    /// two threads cannot deliver theirs out of order: that is what the
    /// promise that neither `fraction` nor a source's `done` ever decreases
    /// rests on.
    reported: Mutex<f32>,
}

impl std::fmt::Debug for Opening {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opening")
            .field("sources", &self.sources)
            .finish_non_exhaustive()
    }
}

impl Opening {
    /// The document is about to be read.
    pub fn document(&self, label: &str) {
        self.send(label, 0, 0, 0.0);
    }

    /// The document is read and these are the sources still to come, by the
    /// names to show for them. Every source weighs the same: what a file
    /// holds is not known until it is opened, and a bar that re-scaled itself
    /// on finding out would run backwards.
    pub fn sources(&mut self, labels: impl IntoIterator<Item = String>) {
        self.sources = labels
            .into_iter()
            .map(|label| (label, Count::default()))
            .collect();
        self.send("Reading the document", 0, 0, DOCUMENT_SHARE);
    }

    /// Source `index` has finished `done` of its `total` frames.
    ///
    /// Called from whichever threads are decoding, in whatever order they
    /// finish. The count is raised, read back and sent under one lock: raised
    /// and read outside it, a thread that finished fifth could read its five,
    /// be overtaken by the sixth, and deliver its five after the six.
    pub fn source(&self, index: usize, done: usize, total: usize) {
        let (Some(listener), Some((label, count))) = (&self.listener, self.sources.get(index))
        else {
            return;
        };
        let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        let mut reported = self.reported.lock().unwrap_or_else(PoisonError::into_inner);
        count.total.store(clamp(total), Ordering::Relaxed);
        let done = count
            .done
            .fetch_max(clamp(done), Ordering::Relaxed)
            .max(clamp(done));
        let mean = self
            .sources
            .iter()
            .map(|(_, count)| {
                let total = count.total.load(Ordering::Relaxed);
                if total == 0 {
                    0.0
                } else {
                    count.done.load(Ordering::Relaxed).min(total) as f32 / total as f32
                }
            })
            .sum::<f32>()
            / self.sources.len() as f32;
        Self::deliver(
            listener,
            &mut reported,
            label,
            done,
            clamp(total),
            DOCUMENT_SHARE + (1.0 - DOCUMENT_SHARE) * mean,
        );
    }

    /// Everything is read, whether or not every source could be.
    pub fn finished(&self) {
        self.send("Opening", 0, 0, 1.0);
    }

    fn send(&self, label: &str, done: u32, total: u32, fraction: f32) {
        let Some(listener) = &self.listener else {
            return;
        };
        let mut reported = self.reported.lock().unwrap_or_else(PoisonError::into_inner);
        Self::deliver(listener, &mut reported, label, done, total, fraction);
    }

    /// Sends one report, with the last fraction's lock held.
    fn deliver(
        listener: &Listener,
        reported: &mut f32,
        label: &str,
        done: u32,
        total: u32,
        fraction: f32,
    ) {
        let fraction = fraction.clamp(*reported, 1.0);
        *reported = fraction;
        listener(OpenProgress {
            label: label.to_owned(),
            done,
            total,
            fraction,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collected() -> (Sink, Arc<Mutex<Vec<OpenProgress>>>) {
        let sink = Sink::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let into = seen.clone();
        sink.on_progress(move |progress| into.lock().expect("lock").push(progress));
        (sink, seen)
    }

    /// Two sources of four frames each: the document's twentieth, then each
    /// frame an eighth of what is left.
    #[test]
    fn the_fraction_is_the_document_then_the_mean_of_the_sources() {
        let (sink, seen) = collected();
        let mut opening = sink.begin();
        opening.document("a.veproj");
        opening.sources(["wind.grib2".to_owned(), "routing".to_owned()]);
        opening.source(0, 2, 4);
        opening.source(1, 4, 4);
        opening.finished();
        let fractions: Vec<f32> = seen
            .lock()
            .expect("lock")
            .iter()
            .map(|p| p.fraction)
            .collect();
        assert_eq!(
            fractions,
            [0.0, 0.05, 0.05 + 0.95 * 0.25, 0.05 + 0.95 * 0.75, 1.0]
        );
        let last = &seen.lock().expect("lock")[3];
        assert_eq!(
            (last.label.as_str(), last.done, last.total),
            ("routing", 4, 4)
        );
    }

    /// A decoder's threads report as they finish, not in order.
    #[test]
    fn a_late_report_of_an_earlier_count_does_not_run_the_bar_backwards() {
        let (sink, seen) = collected();
        let mut opening = sink.begin();
        opening.sources(["wind.grib2".to_owned()]);
        opening.source(0, 3, 4);
        opening.source(0, 2, 4);
        let seen = seen.lock().expect("lock");
        assert!(seen.windows(2).all(|w| w[0].fraction <= w[1].fraction));
        assert_eq!(seen.last().map(|p| p.done), Some(3));
    }

    #[test]
    fn nobody_listening_costs_nothing_and_breaks_nothing() {
        let mut opening = Sink::default().begin();
        opening.document("a.veproj");
        opening.sources(Vec::new());
        opening.source(0, 1, 1);
        opening.finished();
    }
}
