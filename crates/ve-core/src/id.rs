//! Stable identifiers for documents, layers and objects.
//!
//! Ids are opaque `u64`s allocated from a per-process counter. They are stable
//! across every edit short of deletion, which is what lets a keyframe, a
//! history entry and a render snapshot all refer to the same object without
//! holding it: moving or renaming one must not change its id.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// An opaque, stable identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(u64);

impl Id {
    /// Allocates a fresh id, unique within this process.
    pub fn new() -> Self {
        Self(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// Reconstructs an id from its raw value, for deserialisation.
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Raises the allocator so future ids exceed `raw`.
    ///
    /// Must be called for every id in a project after loading it. Otherwise the
    /// counter, which starts at 1 each run, would hand out ids that a loaded
    /// document is already using -- producing two objects with the same id and
    /// an undo stack that edits the wrong one.
    pub fn reserve_above(raw: u64) {
        NEXT_ID.fetch_max(raw.saturating_add(1), Ordering::Relaxed);
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let a = Id::new();
        let b = Id::new();
        assert_ne!(a, b);
    }

    #[test]
    fn raw_round_trips() {
        assert_eq!(Id::from_raw(42).raw(), 42);
    }

    /// Loading a project must not let the allocator hand out an id the document
    /// already uses.
    #[test]
    fn reserving_pushes_the_allocator_past_loaded_ids() {
        let high = 9_000_000;
        Id::reserve_above(high);
        assert!(Id::new().raw() > high);
        // Reserving below the current mark must not roll the allocator back.
        Id::reserve_above(1);
        assert!(Id::new().raw() > high);
    }
}
