//! The synchronisation types the instance pool is built on, from `std` in
//! every normal build and from loom's model checker under `--cfg loom`
//! (ADR-0040 addendum; tests/loom_pool.rs).
//!
//! Only the pool's lock and condvar are shimmed. The factory `Arc` and the
//! `FORK_RESETS` counter stay `std`: loom forbids statics of its types, and
//! the `Arc` is shared with engine code that runs outside any loom model
//! (design D5). The two sets have the same API surface for what is used
//! here, so the non-loom build is exactly the code it was before the shim.

#[cfg(loom)]
pub(crate) use loom::sync::{Condvar, Mutex, MutexGuard};
#[cfg(not(loom))]
pub(crate) use std::sync::{Condvar, Mutex, MutexGuard};
