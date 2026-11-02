//! Fuzzing support (cargo feature `fuzzing`; design D6, D7): the tightened
//! limits every fuzz target runs under, and the drivers that feed raw bytes to
//! the hostile-input entry points. Not a stable interface.
//!
//! The drivers live here, not in the `fuzz/` crate, because two consumers must
//! agree on them exactly: the libFuzzer targets in `fuzz/` (nightly, their own
//! workspace) and the stable regression replay in `tests/fuzz_regressions.rs`.
//! A reproducer that crashed a target has to reach the same code with the same
//! limits when it is replayed, or the replay proves nothing. Rendering is the
//! one target whose entry point is outside this crate (`pdf-extract-render`
//! depends on core, not the reverse); its options are the constants below.
//!
//! The drivers never catch panics: a panic is the finding (spec "Any panic,
//! overrun or crash is a finding"). Errors and `None` results are ignored,
//! because a typed refusal is exactly what hostile input should get.

use std::time::Duration;

use crate::document::{Document, OpenOptions};
use crate::limits::Limits;
use crate::text::{self, BlockSeparator, TextOptions, Unmapped};

/// `OpenOptions::max_pages` under fuzzing (D6). High enough that long
/// documents (real documents run to hundreds of pages) open and their page trees and
/// lazy page index are fuzzed; the 4 MiB input cap bounds the walk anyway.
pub const FUZZ_MAX_PAGES: usize = 1_000;

/// The text target extracts the first `FUZZ_TEXT_HEAD_PAGES` pages and the
/// last page, not every page. With the 2 s page time budget that is at most
/// 7 × 2 s = 14 s, under libFuzzer's 20 s `-timeout`, so a slow input
/// surfaces as the library's own limit, never as a fuzzer timeout (fuzzing
/// spec: limits bind before the fuzzer's ceilings), whatever the page count.
/// The last page reaches the far end of a long page tree through the lazy
/// index.
pub const FUZZ_TEXT_HEAD_PAGES: usize = 6;

/// `RenderOptions::max_render_pixels` under fuzzing (D6).
pub const FUZZ_MAX_RENDER_PIXELS: u64 = 4_000_000;

/// The resolution the render target draws page 0 at (design spec §13.6).
pub const FUZZ_RENDER_DPI: u32 = 36;

/// The limits every fuzz target opens documents with (D6).
///
/// Tight enough that the library's own budgets bind before libFuzzer's
/// per-input ceilings (`-timeout=20 -rss_limit_mb=2560`): a limit hit is a
/// typed error or warning, while a fuzzer timeout or OOM is a finding. A
/// struct literal with no `..Default::default()`, so a new `Limits` field
/// fails to compile here until it is given a fuzzing value.
#[must_use]
pub fn fuzz_limits() -> Limits {
    Limits {
        max_file_size: 4 * 1024 * 1024,
        // Not in D6's table: the defaults already sit far inside what a
        // 4 MiB file can express, so tightening them buys no headroom.
        max_page_tree_depth: 256,
        max_objects: 10_000_000,
        max_stream_decoded_bytes: 32 * 1024 * 1024,
        max_ops_per_page: 200_000,
        page_time_budget: Duration::from_secs(2),
        max_chars_per_page: 200_000,
        max_image_pixels: 4_000_000,
    }
}

/// [`fuzz_limits`] plus [`FUZZ_MAX_PAGES`].
#[must_use]
pub fn fuzz_open_options() -> OpenOptions {
    OpenOptions {
        max_pages: Some(FUZZ_MAX_PAGES),
        limits: fuzz_limits(),
    }
}

/// Target `open`: open the bytes and read the page count and the
/// document-level warnings.
pub fn open_and_count(bytes: Vec<u8>) {
    if let Ok(doc) = Document::from_bytes(bytes, &fuzz_open_options()) {
        let _ = doc.page_count();
        let _ = doc.warnings();
    }
}

/// Target `text`: open the bytes and extract and serialise the text page of
/// the first [`FUZZ_TEXT_HEAD_PAGES`] pages and of the last page.
pub fn text_sampled_pages(bytes: Vec<u8>) {
    let Ok(doc) = Document::from_bytes(bytes, &fuzz_open_options()) else {
        return;
    };
    let opts = TextOptions::default();
    for index in sampled_page_indices(doc.page_count()) {
        if let Ok(text_page) = doc.page(index).and_then(|page| page.text_page(&opts)) {
            let _ = text_page.to_text(BlockSeparator::BlankLine);
        }
    }
}

/// The page indices the text target extracts: the first
/// [`FUZZ_TEXT_HEAD_PAGES`], then the last page if it is not among them.
fn sampled_page_indices(count: usize) -> impl Iterator<Item = usize> {
    let head = count.min(FUZZ_TEXT_HEAD_PAGES);
    let tail = (count > head).then(|| count - 1);
    (0..head).chain(tail)
}

/// Target `cmap`: parse the bytes as a CMap, resolving `usecmap` against the
/// embedded predefined CMaps exactly as the interpreter does, then look up a
/// few codes (some from the input, so lookups follow what was parsed).
///
/// Data starting with `bcmap` is dropped first, as the interpreter refuses a
/// document's CMap stream in hayro-cmap's private binary format (patch 0034,
/// H24, internal #121): hostile input reaches only the text parser, and the binary
/// parser sees only the trusted embedded CMaps that `usecmap` names.
pub fn parse_cmap(data: &[u8]) {
    if data.starts_with(b"bcmap") {
        return;
    }
    let Some(cmap) = hayro_cmap::CMap::parse(data, hayro_cmap::load_embedded) else {
        return;
    };
    let _ = cmap.metadata();
    let mut head = [0u8; 4];
    let n = data.len().min(4);
    head[..n].copy_from_slice(&data[..n]);
    for code in [0, 0x20, 0x41, 0xFFFF, u32::from_be_bytes(head)] {
        for byte_len in 1..=4 {
            let _ = cmap.lookup_cid_code(code, byte_len);
        }
        let _ = cmap.lookup_bf_string(code);
    }
}

/// Target `unicode`: the font-to-Unicode fallbacks of `text/unicode.rs`. The
/// first byte selects the fallback (modulo the count, so every byte value
/// selects one) and the rest is its input:
///
/// 0. a glyph name through the glyph list and the `uniXXXX`/`uXXXX` parser;
/// 1. a 4-byte code plus a name through the KP15 encoding positions;
/// 2. a policy byte plus a 4-byte code through the unmapped-glyph policy;
/// 3. a 2-byte character budget plus a PDF text string through the decoder,
///    then the D-10 sanitiser;
/// 4. arbitrary text through the sanitiser.
pub fn unicode_fallbacks(data: &[u8]) {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };
    match selector % 5 {
        0 => {
            let name = String::from_utf8_lossy(payload);
            let _ = text::name_to_unicode(&name);
            let _ = text::agl_lookup(&name);
            let _ = text::unicode_from_name(&name);
        }
        1 => {
            let (code, rest) = take_u32(payload);
            let _ = text::kp15_position(code, &String::from_utf8_lossy(rest));
        }
        2 => {
            let Some((&policy, rest)) = payload.split_first() else {
                return;
            };
            let policy = match policy % 3 {
                0 => Unmapped::Cid,
                1 => Unmapped::Replacement,
                _ => Unmapped::Skip,
            };
            let _ = text::unmapped_text(take_u32(rest).0, policy);
        }
        3 => {
            let (budget, bytes) = match payload {
                [a, b, rest @ ..] => (usize::from(u16::from_be_bytes([*a, *b])), rest),
                _ => (0, payload),
            };
            let decoded = text::decode_text_string(bytes, budget);
            let _ = text::sanitise(&decoded);
        }
        _ => {
            let _ = text::sanitise(&String::from_utf8_lossy(payload));
        }
    }
}

/// A big-endian `u32` from up to the first four bytes (zero-padded), and the
/// bytes after them.
fn take_u32(bytes: &[u8]) -> (u32, &[u8]) {
    let n = bytes.len().min(4);
    let mut word = [0u8; 4];
    word[..n].copy_from_slice(&bytes[..n]);
    (u32::from_be_bytes(word), &bytes[n..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzz_limits_sets_every_field_to_the_d6_value() {
        // Destructured without `..`: a new `Limits` field fails to compile
        // here, so the fuzzing limits cannot silently fall back to a default.
        let Limits {
            max_file_size,
            max_page_tree_depth,
            max_objects,
            max_stream_decoded_bytes,
            max_ops_per_page,
            page_time_budget,
            max_chars_per_page,
            max_image_pixels,
        } = fuzz_limits();
        assert_eq!(max_file_size, 4 << 20);
        assert_eq!(max_page_tree_depth, 256);
        assert_eq!(max_objects, 10_000_000);
        assert_eq!(max_stream_decoded_bytes, 32 << 20);
        assert_eq!(max_ops_per_page, 200_000);
        assert_eq!(page_time_budget, Duration::from_secs(2));
        assert_eq!(max_chars_per_page, 200_000);
        assert_eq!(max_image_pixels, 4_000_000);
        let opts = fuzz_open_options();
        assert_eq!(opts.max_pages, Some(1_000));
        assert_eq!(opts.limits, fuzz_limits());
    }

    #[test]
    fn the_text_sample_is_the_head_and_the_last_page() {
        let sample = |n| sampled_page_indices(n).collect::<Vec<_>>();
        assert_eq!(sample(0), Vec::<usize>::new());
        assert_eq!(sample(1), [0]);
        assert_eq!(sample(6), [0, 1, 2, 3, 4, 5]);
        assert_eq!(sample(7), [0, 1, 2, 3, 4, 5, 6]);
        // A 300-page document: never more than 7 pages, so 7 x 2 s < 20 s.
        assert_eq!(sample(300), [0, 1, 2, 3, 4, 5, 299]);
        const { assert!(FUZZ_TEXT_HEAD_PAGES < FUZZ_MAX_PAGES) };
    }

    #[test]
    fn take_u32_pads_short_input() {
        assert_eq!(take_u32(&[]), (0, &[][..]));
        assert_eq!(take_u32(&[1, 2]), (0x0102_0000, &[][..]));
        assert_eq!(take_u32(&[1, 2, 3, 4, 5]), (0x0102_0304, &[5][..]));
    }

    #[test]
    fn document_drivers_accept_valid_and_garbage_input() {
        let pdf = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/corpus-c/enc_aes_128_owner.pdf"
        ))
        .expect("fixture reads");
        open_and_count(pdf.clone());
        text_sampled_pages(pdf);
        open_and_count(b"not a pdf".to_vec());
        text_sampled_pages(Vec::new());
    }

    #[test]
    fn cmap_driver_parses_and_looks_up() {
        let cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
            /CMapName /Test def 1 begincodespacerange <00> <FF> endcodespacerange \
            1 beginbfchar <41> <0042> endbfchar endcmap CMapName currentdict \
            /CMap defineresource pop end end";
        parse_cmap(cmap);
        parse_cmap(b"/Identity-H usecmap");
        parse_cmap(b"");
    }

    #[test]
    fn unicode_driver_reaches_every_selector() {
        unicode_fallbacks(b"");
        for selector in 0..5u8 {
            unicode_fallbacks(&[selector]);
            let mut input = vec![selector];
            input.extend_from_slice(b"\x00\xA0nbspace uni0041 \xFE\xFF\x00A");
            unicode_fallbacks(&input);
        }
        unicode_fallbacks(&[3, 0]);
        unicode_fallbacks(&[2, 7]);
        unicode_fallbacks(&[2, 8, 0, 0, 0, 0x41]);
    }
}
