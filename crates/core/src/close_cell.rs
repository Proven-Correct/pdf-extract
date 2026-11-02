//! `CloseCell<T>`: the document close protocol (T39, T40; ADR-0040),
//! extracted from `DocShared` so that loom can model-check the code the
//! library ships rather than a replica (tests/loom_close.rs, design D5).
//!
//! The cell holds `RwLock<Option<Arc<T>>>`. An operation takes the read
//! lock only long enough to clone the `Arc`, then works on its own clone;
//! `close()` takes the write lock and replaces the value with `None`. So
//! operations already running finish on their clone, later ones see `None`,
//! and `T` is freed when the last clone (the cell's or an operation's)
//! drops. The lock is never held while `T` is used or freed.
//!
//! [`CloseCell::close`] hands the removed `Arc` back instead of dropping it
//! itself so that the caller drops it after the write guard is released:
//! freeing a large document under the lock would stall every reader.
//!
//! Lock poisoning is ignored on purpose: the guarded value is an
//! `Option<Arc<_>>` replaced in one assignment, so a panic elsewhere cannot
//! leave it half-updated.
//!
//! The type is `pub` only so that the loom build can re-export it (see
//! `lib.rs`); in normal builds its module is private.

use std::sync::PoisonError;

use crate::sync::{Arc, RwLock};

/// A value that can be closed once while operations hold clones of it.
pub struct CloseCell<T> {
    state: RwLock<Option<Arc<T>>>,
}

impl<T> CloseCell<T> {
    /// An open cell holding `value`.
    pub fn new(value: T) -> Self {
        Self {
            state: RwLock::new(Some(Arc::new(value))),
        }
    }

    /// The value for one operation, or `None` once closed. The read lock
    /// is released before this returns.
    pub fn get(&self) -> Option<Arc<T>> {
        self.state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Closes the cell, returning its `Arc` (`None` if already closed) for
    /// the caller to drop after the lock is released. Idempotent.
    pub fn close(&self) -> Option<Arc<T>> {
        // The guard is a temporary of this statement, so it is released
        // before the caller sees (and drops) the result.
        self.state
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Whether [`Self::close`] has been called.
    pub fn is_closed(&self) -> bool {
        self.state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
    }
}
