//! Entry point of the helpers shared by the integration tests: everything in
//! `helpers.rs`, plus the small-stack runner.
//!
//! The split exists because the crate's own unit tests (`src/text/tests`)
//! compile `helpers.rs` too, and the crate is `forbid(unsafe_code)`: the
//! runner measures its stack through pthread FFI (`stack.rs`), so it can only
//! be compiled into the integration-test crates. The unit tests have a plain,
//! unmeasured runner of their own.

// Each test crate includes this module and uses only part of it.
#![allow(dead_code)]

mod helpers;
pub mod stack;

pub use helpers::*;
#[allow(unused_imports)] // each test crate uses part of it
pub use stack::{SMALL_STACK, on_small_stack, try_on_small_stack};

use hayro_syntax::Pdf;

/// Opens `bytes` and runs page 0 on a [`SMALL_STACK`] thread (see
/// [`on_small_stack`]).
pub fn run_on_small_stack(bytes: Vec<u8>, run: Run) -> Output {
    on_small_stack(move || {
        let pdf = Pdf::new(bytes).expect("fixture opens");
        run_page(&pdf, &run)
    })
}
