//! The synchronisation types the close protocol is built on, from `std` in
//! every normal build and from loom's model checker under `--cfg loom`
//! (ADR-0040 addendum; tests/loom_close.rs).
//!
//! Only what [`CloseCell`](crate::close_cell::CloseCell) uses is shimmed.
//! Everything else in the crate keeps `std::sync` directly, on purpose:
//! - loom types panic outside a `loom::model` run, so a shimmed type that
//!   the open pipeline or a `static` touched would make the loom build
//!   unusable;
//! - the `Pages` cursor and the lazy-repair flag are single atomic
//!   operations with no protocol to check (design D5).
//!
//! The two sets have the same API surface for what is used here, so the
//! non-loom build is exactly the code it was before the shim.

#[cfg(loom)]
pub(crate) use loom::sync::{Arc, RwLock};
#[cfg(not(loom))]
pub(crate) use std::sync::{Arc, RwLock};
