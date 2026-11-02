//! Rust callers keep their own panic hook (internal #76; error-model "Rust callers
//! keep their own panic hook"): the library never installs or replaces the
//! process-global hook, so when the codec boundary (H18) catches a codec
//! panic, the application's hook is the one that ran.
//!
//! Its own test binary with a single test: `panic::set_hook` is process-wide,
//! and the hook installed here must not leak into, or race with, other tests.

mod common;

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::codecs::{BILEVEL_8, MMR_BLACK_8X8};
use common::{images_pdf, kinds, options, render_with};
use hayro_syntax::arm_codec_panic_for_test;
use pdf_extract_core::{Limits, WarningKind};

#[test]
fn a_caught_codec_panic_runs_the_applications_hook() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    // Counts and stays silent; the message is what the codec hook panics with.
    panic::set_hook(Box::new(move |info| {
        let message = info.payload().downcast_ref::<&str>().copied();
        if message == Some("codec panic injected for testing") {
            seen.fetch_add(1, Ordering::SeqCst);
        }
    }));

    let dict =
        format!("{BILEVEL_8} /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8 /Rows 8 >>");
    arm_codec_panic_for_test();
    // Caught, so that a render that panicked after all reaches the asserts
    // below with the default hook back in place, instead of failing silently.
    let r = panic::catch_unwind(AssertUnwindSafe(|| {
        render_with(
            images_pdf(&[(&dict, MMR_BLACK_8X8.to_vec())]),
            Limits::default(),
            &options(72),
        )
    }));

    // Restore the default before asserting, so a failure reports normally.
    drop(panic::take_hook());
    let r = r
        .expect("the codec panic was caught inside the render")
        .expect("renders");
    assert_eq!(kinds(&r), [WarningKind::ImageDecodeFailure]);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the application's hook ran once"
    );
}
