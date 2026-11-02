//! H2b: the per-stream decoded-size cap in the vendored hayro fork (spec
//! §3.2, §4.4).
//!
//! `Pdf::new_with_options(data, password, DecodeLimits)` bounds how many bytes
//! each flate, LZW, run-length, ASCII hex or ASCII base-85 filter may produce
//! for one stream. A stream over the limit fails with
//! `DecodeFailure::LimitExceeded`, hayro treats it as absent, and
//! `Pdf::decode_stats()` counts it so the library can attach a warning (unit
//! 10). Streams within the limit must decode exactly as before: the cap must
//! never truncate. And the decoders must stop early, not decode everything and
//! then compare: a counting allocator checks the peak memory of a decode.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::io::Write as _;

use common::{interpret, on_small_stack, one_page_pdf};
use flate2::Compression;
use flate2::write::{DeflateEncoder, ZlibEncoder};
use hayro_syntax::object::stream::DecodeFailure;
use hayro_syntax::object::{FromBytes, Object, Stream};
use hayro_syntax::{DecodeLimits, DecodeStats, Pdf};
use pdf_extract_testkit::hostile;

const MIB: u64 = 1 << 20;

/// The system allocator plus per-thread accounting of live heap bytes and
/// their peak, so a test can measure what one decode allocates while other
/// tests run on other threads.
struct PeakCounting;

thread_local! {
    // Const-initialised `Cell`s without destructors: touching them from
    // inside the allocator never allocates.
    static LIVE: Cell<usize> = const { Cell::new(0) };
    static PEAK: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: every method forwards to `System` with the caller's arguments
// unchanged, so `System`'s guarantees carry over; the bookkeeping only
// touches thread-local `Cell`s and never allocates. `try_with` makes the
// accounting a no-op during thread teardown instead of panicking.
unsafe impl GlobalAlloc for PeakCounting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged; see the impl comment.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            grew(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged; see the impl comment.
        unsafe { System.dealloc(ptr, layout) };
        shrank(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded unchanged; see the impl comment.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            shrank(layout.size());
            grew(new_size);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: PeakCounting = PeakCounting;

fn grew(bytes: usize) {
    let _ = LIVE.try_with(|live| {
        let now = live.get().saturating_add(bytes);
        live.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
    });
}

fn shrank(bytes: usize) {
    // Memory freed on another thread than it was allocated on can make one
    // thread's figure drift; saturating keeps it meaningful for a test that
    // allocates and frees on its own thread.
    let _ = LIVE.try_with(|live| live.set(live.get().saturating_sub(bytes)));
}

/// Runs `f` and returns its result with the peak number of heap bytes this
/// thread held during the call, above what it held at the start.
fn peak_heap_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let start = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(start));
    let result = f();
    (result, PEAK.with(Cell::get) - start)
}

fn limits(max_stream_decoded_bytes: u64) -> DecodeLimits {
    DecodeLimits {
        max_stream_decoded_bytes,
        ..DecodeLimits::default()
    }
}

/// The first page's content stream.
fn content_stream<'a>(pdf: &'a Pdf) -> Stream<'a> {
    pdf.pages()[0]
        .raw()
        .get::<Stream<'_>>("Contents")
        .expect("page has a content stream")
}

fn over_limit(pdf: &Pdf) -> u64 {
    pdf.decode_stats().streams_over_limit
}

#[test]
fn default_limits_are_unlimited() {
    assert_eq!(DecodeLimits::default(), limits(u64::MAX));
    assert_eq!(DecodeStats::default().streams_over_limit, 0);
}

#[test]
fn flate_bomb_over_the_limit_is_dropped_and_counted() {
    on_small_stack(|| {
        let pdf = Pdf::new_with_options(hostile::flate_bomb(50 * MIB), "", limits(16 * MIB))
            .expect("opens");
        assert_eq!(over_limit(&pdf), 0, "opening decodes no content");

        let collected = interpret(&pdf, &pdf.pages()[0]);
        assert_eq!(collected.text, "");
        assert_eq!(collected.paths, 0);
        assert_eq!(over_limit(&pdf), 1, "one content stream hit the limit");

        assert!(matches!(
            content_stream(&pdf).decoded(),
            Err(DecodeFailure::LimitExceeded)
        ));
        // hayro does not cache decoded streams: each decode is counted.
        assert_eq!(over_limit(&pdf), 2);
    });
}

#[test]
fn flate_bomb_within_the_limit_decodes() {
    on_small_stack(|| {
        for pdf in [
            Pdf::new(hostile::flate_bomb(50 * MIB)).expect("opens"),
            Pdf::new_with_options(hostile::flate_bomb(50 * MIB), "", limits(256 * MIB))
                .expect("opens"),
        ] {
            let collected = interpret(&pdf, &pdf.pages()[0]);
            // The content is 50 MiB of spaces: no glyphs.
            assert_eq!(collected.text, "");
            let decoded = content_stream(&pdf).decoded().expect("decodes");
            assert_eq!(decoded.len() as u64, 50 * MIB);
            assert_eq!(over_limit(&pdf), 0);
        }
    });
}

/// Deterministic, moderately compressible test data (a linear congruential
/// generator over a small alphabet), so the compressed streams have real
/// back-references rather than one long run.
fn sample_data(len: usize) -> Vec<u8> {
    let mut state: u32 = 0x2545_f491;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            b"abcdefgh \n0"[(state >> 24) as usize % 11]
        })
        .collect()
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).expect("Vec write");
    enc.finish().expect("Vec write")
}

/// Raw deflate data that only hayro's fallback decoder (the pdf.js port)
/// accepts: it starts with an empty stored block whose length check field is
/// wrong (LEN 0, NLEN 0 instead of 0xFFFF). flate2 rejects that at once, as
/// zlib and as raw deflate; the fallback decoder ignores the mismatch for an
/// empty block and goes on to decode `data`. A stored block ends on a byte
/// boundary, so the real data can simply be appended.
fn deflate_for_fallback(data: &[u8]) -> Vec<u8> {
    // BFINAL 0, BTYPE 00 and padding; LEN 0; NLEN 0.
    let mut out = vec![0u8; 5];
    out.extend_from_slice(&raw_deflate(data));
    out
}

fn raw_deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).expect("Vec write");
    enc.finish().expect("Vec write")
}

/// LZW-encode `data` the way PDF writers do (8-bit symbols, early change,
/// MSB-first codes of 9 to 12 bits), clearing the table before it fills.
///
/// The decoder adds a table entry one code later than the encoder (it needs
/// the next code's first byte), and with early change widens one code early;
/// the two cancel, so the width of each code follows the encoder's own next
/// free code: 9 bits below 512, 10 below 1024, 11 below 2048, else 12.
fn lzw(data: &[u8]) -> Vec<u8> {
    const CLEAR: u32 = 256;
    const EOD: u32 = 257;
    const FIRST: u32 = 258;
    // Clear well before 4096 entries: hayro's table refuses to grow past
    // that.
    const LAST: u32 = 4000;

    fn width(next_code: u32) -> u32 {
        match next_code {
            n if n >= 2048 => 12,
            n if n >= 1024 => 11,
            n if n >= 512 => 10,
            _ => 9,
        }
    }

    let (mut out, mut acc, mut bits) = (Vec::new(), 0u64, 0u32);
    let mut emit = |code: u32, w: u32| {
        acc = (acc << w) | u64::from(code);
        bits += w;
        while bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
        acc &= (1 << bits) - 1;
    };

    let mut dict: HashMap<(u32, u8), u32> = HashMap::new();
    let mut next = FIRST;
    emit(CLEAR, width(next));
    let mut current: Option<u32> = None;
    for &b in data {
        let Some(prefix) = current else {
            current = Some(u32::from(b));
            continue;
        };
        if let Some(&code) = dict.get(&(prefix, b)) {
            current = Some(code);
            continue;
        }
        emit(prefix, width(next));
        dict.insert((prefix, b), next);
        next += 1;
        if next >= LAST {
            emit(CLEAR, width(next));
            dict.clear();
            next = FIRST;
        }
        current = Some(u32::from(b));
    }
    if let Some(prefix) = current {
        emit(prefix, width(next));
        next += 1;
    }
    emit(EOD, width(next));
    if bits > 0 {
        out.push((acc << (8 - bits)) as u8);
    }
    out
}

/// Run-length encoding: runs of three or more equal bytes as repeat runs,
/// everything else as literal runs of up to 128 bytes.
fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut literal: Vec<u8> = Vec::new();
    let flush = |literal: &mut Vec<u8>, out: &mut Vec<u8>| {
        for chunk in literal.chunks(128) {
            out.push((chunk.len() - 1) as u8);
            out.extend_from_slice(chunk);
        }
        literal.clear();
    };
    let mut i = 0;
    while i < data.len() {
        let run = data[i..]
            .iter()
            .take(128)
            .take_while(|&&b| b == data[i])
            .count();
        if run >= 3 {
            flush(&mut literal, &mut out);
            out.push((257 - run) as u8);
            out.push(data[i]);
            i += run;
        } else {
            literal.push(data[i]);
            i += 1;
        }
    }
    flush(&mut literal, &mut out);
    out.push(128);
    out
}

fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = data
        .iter()
        .flat_map(|b| format!("{b:02X}").into_bytes())
        .collect();
    out.push(b'>');
    out
}

/// ASCII base-85 with the `z` shorthand for all-zero groups.
fn ascii_85(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in data.chunks(4) {
        let mut group = [0u8; 4];
        group[..chunk.len()].copy_from_slice(chunk);
        let mut value = u32::from_be_bytes(group);
        if chunk.len() == 4 && value == 0 {
            out.push(b'z');
            continue;
        }
        let mut digits = [0u8; 5];
        for d in digits.iter_mut().rev() {
            *d = b'!' + (value % 85) as u8;
            value /= 85;
        }
        out.extend_from_slice(&digits[..chunk.len() + 1]);
    }
    out.extend_from_slice(b"~>");
    out
}

/// One filter case: a name for messages, the stream dictionary's filter
/// entries, the encoded bytes, and what they decode to.
struct Case {
    name: &'static str,
    dict: &'static str,
    encoded: Vec<u8>,
    plain: Vec<u8>,
}

fn cases() -> Vec<Case> {
    let mib = sample_data(MIB as usize);
    let small = sample_data(20_000);
    let mut zeros_and_text = vec![0u8; 4_000];
    zeros_and_text.extend_from_slice(&small[..1_000]);
    vec![
        Case {
            name: "flate 1 MiB (zlib)",
            dict: "/Filter /FlateDecode",
            encoded: zlib(&mib),
            plain: mib.clone(),
        },
        Case {
            name: "flate (raw deflate)",
            dict: "/Filter /FlateDecode",
            encoded: raw_deflate(&small),
            plain: small.clone(),
        },
        Case {
            name: "flate (fallback decoder)",
            dict: "/Filter /FlateDecode",
            encoded: deflate_for_fallback(&mib),
            plain: mib,
        },
        Case {
            name: "LZW",
            dict: "/Filter /LZWDecode",
            encoded: lzw(&small),
            plain: small.clone(),
        },
        Case {
            name: "LZW (long runs, table resets)",
            dict: "/Filter /LZWDecode",
            encoded: lzw(&zeros_and_text.repeat(200)),
            plain: zeros_and_text.repeat(200),
        },
        Case {
            name: "RunLength",
            dict: "/Filter /RunLengthDecode",
            encoded: run_length(&zeros_and_text),
            plain: zeros_and_text.clone(),
        },
        Case {
            name: "ASCIIHex",
            dict: "/Filter /ASCIIHexDecode",
            encoded: ascii_hex(&small),
            plain: small.clone(),
        },
        Case {
            name: "ASCII85",
            dict: "/Filter /ASCII85Decode",
            encoded: ascii_85(&zeros_and_text),
            plain: zeros_and_text,
        },
        Case {
            name: "ASCIIHex then flate",
            dict: "/Filter [/ASCIIHexDecode /FlateDecode]",
            encoded: ascii_hex(&zlib(&small)),
            plain: small,
        },
    ]
}

/// Decodes the content stream of a one-page PDF carrying `case`, opened with
/// `limits` (or with plain `Pdf::new` for `None`); returns the result and the
/// document's over-limit count afterwards.
fn decode(case: &Case, limits: Option<DecodeLimits>) -> (Result<Vec<u8>, DecodeFailure>, u64) {
    let bytes = one_page_pdf(case.dict, &case.encoded);
    let pdf = match limits {
        None => Pdf::new(bytes),
        Some(l) => Pdf::new_with_options(bytes, "", l),
    }
    .expect("opens");
    let result = content_stream(&pdf).decoded().map(|d| d.into_owned());
    (result, over_limit(&pdf))
}

#[test]
fn streams_within_the_limit_decode_unchanged() {
    for case in cases() {
        let exact = case.plain.len() as u64;
        // `None` is the pre-H2b entry point; the rest are limits at and
        // above the decoded size, including one exactly equal to it.
        for limit in [None, Some(u64::MAX), Some(256 * MIB), Some(exact)] {
            let (result, hits) = decode(&case, limit.map(limits));
            let decoded = result.unwrap_or_else(|e| panic!("{} at {limit:?}: {e:?}", case.name));
            assert!(
                decoded == case.plain,
                "{} at {limit:?}: decoded {} bytes, expected {}",
                case.name,
                decoded.len(),
                case.plain.len()
            );
            assert_eq!(hits, 0, "{} at {limit:?}", case.name);
        }
    }
}

#[test]
fn one_byte_over_the_limit_fails() {
    for case in cases() {
        let exact = case.plain.len() as u64;
        for limit in [exact - 1, exact / 2, 1, 0] {
            let (result, hits) = decode(&case, Some(limits(limit)));
            assert!(
                matches!(result, Err(DecodeFailure::LimitExceeded)),
                "{} at limit {limit}: {:?}",
                case.name,
                result.map(|d| d.len())
            );
            assert_eq!(hits, 1, "{} at limit {limit}", case.name);
        }
    }
}

#[test]
fn the_limit_applies_to_each_filter_of_a_chain() {
    // ASCIIHex output (the zlib bytes, doubled back to binary) is far
    // smaller than the final flate output; a limit between the two lets the
    // first filter through and stops the second.
    let plain = sample_data(100_000);
    let compressed = zlib(&plain);
    let case = Case {
        name: "chain",
        dict: "/Filter [/ASCIIHexDecode /FlateDecode]",
        encoded: ascii_hex(&compressed),
        plain,
    };
    let between = compressed.len() as u64 + 1;
    let (result, hits) = decode(&case, Some(limits(between)));
    assert!(matches!(result, Err(DecodeFailure::LimitExceeded)));
    assert_eq!(hits, 1);
}

#[test]
fn streams_without_a_document_are_not_limited() {
    // Objects parsed on their own have no document and so no limits; only
    // `Pdf::new_with_options` sets them.
    let plain = sample_data(100_000);
    let encoded = zlib(&plain);
    let mut src = format!(
        "<< /Length {} /Filter /FlateDecode >>\nstream\n",
        encoded.len()
    )
    .into_bytes();
    src.extend_from_slice(&encoded);
    src.extend_from_slice(b"\nendstream");
    let Some(Object::Stream(stream)) = Object::from_bytes(&src) else {
        panic!("stream object parses");
    };
    assert_eq!(
        stream.decoded().expect("decodes").as_ref(),
        plain.as_slice()
    );
}

/// A one-page PDF whose cross-reference section is a flate-compressed xref
/// stream, followed by `padding` zero bytes inside the compressed data (past
/// the entries hayro reads, so harmless when decoded, but they count toward
/// the stream's decoded size).
fn xref_stream_pdf(padding: usize) -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    offsets.push(xref_at);

    // /W [1 4 2]: type, offset, generation.
    let mut entries = vec![0u8, 0, 0, 0, 0, 0xff, 0xff];
    for offset in &offsets {
        entries.push(1);
        entries.extend_from_slice(&u32::try_from(*offset).expect("small").to_be_bytes());
        entries.extend_from_slice(&[0, 0]);
    }
    entries.resize(entries.len() + padding, 0);
    let data = zlib(&entries);

    out.extend_from_slice(
        format!(
            "4 0 obj\n<< /Type /XRef /Size 5 /W [1 4 2] /Root 1 0 R \
             /Filter /FlateDecode /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    out
}

#[test]
fn xref_streams_are_limited_too() {
    // Xref streams are decoded while the document is being opened, before
    // there is a document context to take the limits from; they get them
    // passed explicitly.
    let bytes = xref_stream_pdf(8 * MIB as usize);

    let pdf = Pdf::new_with_options(bytes.clone(), "", limits(256 * MIB)).expect("opens");
    assert_eq!(pdf.pages().len(), 1);
    assert_eq!(over_limit(&pdf), 0);

    // Over the limit the xref stream is unusable, hayro rebuilds the xref by
    // scanning the file, and the document still opens.
    let pdf = Pdf::new_with_options(bytes, "", limits(MIB)).expect("opens via repair");
    assert_eq!(pdf.pages().len(), 1);
    assert!(
        over_limit(&pdf) >= 1,
        "the xref stream's refusal is counted"
    );
}

#[test]
fn counting_is_shared_across_threads() {
    // `Pdf` is shared between threads by the library; the counter is
    // per-document, not per-thread.
    let pdf = Pdf::new_with_options(hostile::flate_bomb(4 * MIB), "", limits(MIB)).expect("opens");
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                assert!(content_stream(&pdf).decoded().is_err());
            });
        }
    });
    assert_eq!(over_limit(&pdf), 4);
}

#[test]
fn decoders_stop_early() {
    // 8 MiB of output from each amplifying filter, decoded under a 256 KiB
    // limit. Decoding in full and checking afterwards would hold 8 MiB at the
    // peak; stopping at the limit holds about twice the limit (the output
    // buffer grows by doubling) plus the decoder's own state.
    const OUT: usize = 8 << 20;
    const LIMIT: u64 = 256 << 10;
    let zeros = vec![0u8; OUT];
    let bombs = [
        ("flate", "/Filter /FlateDecode", zlib(&zeros)),
        (
            "flate (fallback decoder)",
            "/Filter /FlateDecode",
            deflate_for_fallback(&zeros),
        ),
        ("LZW", "/Filter /LZWDecode", lzw(&zeros)),
        // 2 bytes per 128 zeros.
        ("RunLength", "/Filter /RunLengthDecode", {
            let mut rl = [129u8, 0].repeat(OUT / 128);
            rl.push(128);
            rl
        }),
        // One `z` per 4 zeros.
        ("ASCII85", "/Filter /ASCII85Decode", {
            let mut a85 = vec![b'z'; OUT / 4];
            a85.extend_from_slice(b"~>");
            a85
        }),
    ];
    drop(zeros);
    for (name, dict, encoded) in bombs {
        let pdf =
            Pdf::new_with_options(one_page_pdf(dict, &encoded), "", limits(LIMIT)).expect("opens");
        let stream = content_stream(&pdf);
        let (result, peak) = peak_heap_during(|| stream.decoded().map(|d| d.len()));
        assert!(
            matches!(result, Err(DecodeFailure::LimitExceeded)),
            "{name}: {result:?}"
        );
        assert!(
            peak < 2 * MIB as usize,
            "{name}: decoding held {peak} bytes at its peak"
        );
    }
}
