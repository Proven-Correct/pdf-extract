//! Tests of fork change H10f: every `DecodeFailure::LimitExceeded` decode
//! also bumps the thread-local `stream_decode_limit_hits()` (spec §3.2,
//! oracle §4.9b).
//!
//! `Pdf::decode_stats()` counts the same events per document, so two threads
//! extracting different pages of one document could not tell whose decode
//! went over the limit; the thread-local can.

use hayro_syntax::object::Stream;
use hayro_syntax::{DecodeLimits, Pdf, stream_decode_limit_hits};
use pdf_extract_testkit::hostile;

const MIB: u64 = 1 << 20;

/// The flate bomb, opened with a 1 MiB limit.
fn bomb() -> Pdf {
    Pdf::new_with_options(
        hostile::flate_bomb(4 * MIB),
        "",
        DecodeLimits {
            max_stream_decoded_bytes: MIB,
            ..DecodeLimits::default()
        },
    )
    .expect("opens")
}

fn content(pdf: &Pdf) -> Stream<'_> {
    pdf.pages()[0]
        .raw()
        .get::<Stream<'_>>("Contents")
        .expect("page has a content stream")
}

#[test]
fn counts_each_over_limit_decode_on_this_thread() {
    let pdf = bomb();
    let before = stream_decode_limit_hits();
    for _ in 0..3 {
        assert!(content(&pdf).decoded().is_err());
    }
    assert_eq!(stream_decode_limit_hits() - before, 3);
    assert_eq!(pdf.decode_stats().streams_over_limit, 3);
}

#[test]
fn a_decode_within_the_limit_is_not_counted() {
    let pdf = Pdf::new_with_options(
        hostile::flate_bomb(MIB / 2),
        "",
        DecodeLimits {
            max_stream_decoded_bytes: MIB,
            ..DecodeLimits::default()
        },
    )
    .expect("opens");
    let before = stream_decode_limit_hits();
    assert!(content(&pdf).decoded().is_ok());
    assert_eq!(stream_decode_limit_hits(), before);
}

#[test]
fn another_threads_decodes_are_not_visible() {
    let pdf = bomb();
    let before = stream_decode_limit_hits();
    // The other thread only reports; every assertion runs after the join,
    // so a failure fails the test instead of leaving a thread waiting.
    let (theirs, all_failed) = std::thread::scope(|s| {
        s.spawn(|| {
            let mine = stream_decode_limit_hits();
            let all_failed = (0..5).all(|_| content(&pdf).decoded().is_err());
            (stream_decode_limit_hits() - mine, all_failed)
        })
        .join()
        .expect("decoding thread")
    });
    assert!(all_failed);
    assert_eq!(theirs, 5);
    assert_eq!(stream_decode_limit_hits(), before);
    assert_eq!(pdf.decode_stats().streams_over_limit, 5);
}
