//! Unit tests of the text path. The device tests (spec §4.9) are driven
//! through `Page::extract_chars` on documents built with the test kit, one
//! file per §4.9 bullet group; `layout` feeds the line assembler hand-made
//! records (spec §4.10). This file holds the shared helpers.
//!
//! These are unit tests (inside the crate) because `extract_chars`,
//! `CharRecord` and the assembler are internal; the public text API is
//! tested from `tests/text_*.rs` and `tests/concurrency.rs`.

// The helpers shared by the integration tests of the fork: hand-assembled
// PDFs, `run_device`.
#[path = "../../../tests/common/helpers.rs"]
mod common;

// The integration tests' small-stack runner, which measures each thread's
// real stack and refuses one glibc served from its cache at the wrong size
// (internal issue #70). It needs pthread FFI; the unit-test build only denies
// `unsafe` (lib.rs), so this module is the one explicit exception (ADR 0089).
#[allow(unsafe_code)]
#[path = "../../../tests/common/stack.rs"]
mod stack;

use stack::on_small_stack;

// The limits tests run their deep inputs through `on_small_stack`; this pins
// that the runner they get is the measuring one, on a stack of the promised
// size. Skipped by name under the sanitizers (tools/sanitizers/run.sh), where
// the runner hands out 64 MiB without measuring.
#[test]
fn the_unit_tests_small_stack_is_measured() {
    let measured = on_small_stack(stack::current_thread_stack_size);
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        let size = measured.expect("measurable on this platform");
        assert!(
            (stack::SMALL_STACK..=stack::SMALL_STACK + stack::STACK_SLACK).contains(&size),
            "{size}"
        );
    }
}

mod actual_text;
mod content;
mod filtering;
mod geometry;
mod layout;
mod limits;
mod unicode_chain;

use super::options::Unmapped;
use super::record::PageChars;
use crate::{Document, Limits, OpenOptions, PageError};

/// Opens `bytes` with `limits` and extracts page 0.
fn extract_with(
    bytes: Vec<u8>,
    limits: Limits,
    unmapped: Unmapped,
) -> Result<PageChars, PageError> {
    let opts = OpenOptions {
        limits,
        ..OpenOptions::default()
    };
    let doc = Document::from_bytes(bytes, &opts).expect("fixture opens");
    doc.page(0)?.extract_chars(unmapped)
}

/// Extracts page 0 with default limits; the extraction must succeed.
fn extract(bytes: Vec<u8>) -> PageChars {
    extract_with(bytes, Limits::default(), Unmapped::Cid).expect("page extracts")
}

/// The records' texts.
fn texts(page: &PageChars) -> Vec<&str> {
    page.chars.iter().map(|c| c.text.as_str()).collect()
}

/// All record texts concatenated.
fn joined(page: &PageChars) -> String {
    texts(page).concat()
}

/// The warning kinds of a page, in order.
fn kinds(page: &PageChars) -> Vec<crate::WarningKind> {
    page.warnings.iter().map(|w| w.kind).collect()
}

/// Asserts two points agree to 1/1000 pt.
#[track_caller]
fn assert_near(got: (f32, f32), want: (f64, f64)) {
    let ok = (f64::from(got.0) - want.0).abs() < 1e-3 && (f64::from(got.1) - want.1).abs() < 1e-3;
    assert!(ok, "got {got:?}, want {want:?}");
}

/// Asserts two lengths agree to 1/1000 pt.
#[track_caller]
fn assert_close(got: f32, want: f64) {
    assert!(
        (f64::from(got) - want).abs() < 1e-3,
        "got {got}, want {want}"
    );
}
