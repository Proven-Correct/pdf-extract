//! H14: the JBIG2 decoder's memory budget in the vendored fork (run 0003
//! spec F5, F6, F8–F10, oracle §4.4).
//!
//! `hayro_jbig2::DecoderContext::memory_budget` bounds the total bytes one
//! `decode_with` call allocates for buffers whose size the file declares;
//! each is charged before it is allocated and a decode that would exceed
//! the budget fails with `Overflow(MemoryBudget)`. hayro-syntax sets the
//! budget to `JBIG2_BUDGET_BYTES_PER_PIXEL × max_image_pixels`, reports a
//! refusal as `DecodeFailure::ImageMemoryLimit` and counts it in the
//! thread-local `image_memory_limit_hits()`; hayro-interpret raises
//! `InterpreterWarning::ImageMemoryLimit` for it, and nothing else.
//!
//! The streams come from `pdf_extract_testkit::jbig2`, which assembles them
//! segment by segment. Two shapes would allocate hundreds of gigabytes
//! unbudgeted and are only ever decoded under a budget; their refusals are
//! also checked for resident memory, in a child process so that no other
//! test's allocations blur the peak.
//!
//! H17's test lives here too: export flags that cycle once the arithmetic
//! data is used up spun the decoder forever, out of the budget's reach (internal #94).
//! So does the round trip of the test kit's MQ encoder, which writes the
//! arithmetic-coded symbols the budget tests decode.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::process::Command;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterSettings, InterpreterWarning, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro_jbig2::{DecodeError, Decoder, DecoderContext, OverflowError, SymbolError};
use hayro_syntax::object::stream::{DecodeFailure, ImageDecodeParams};
use hayro_syntax::object::{ObjectIdentifier, Stream};
use hayro_syntax::{
    DecodeLimits, JBIG2_BUDGET_BYTES_PER_PIXEL, Pdf, image_memory_limit_hits,
    image_pixel_limit_hits,
};
use kurbo::{Affine, BezPath, Rect};
use pdf_extract_testkit::jbig2;

const KIB: u64 = 1 << 10;
const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

const REFUSED: DecodeError = DecodeError::Overflow(OverflowError::MemoryBudget);

/// The page as the decoder emits it: one byte (0 white, 1 black) per pixel,
/// row by row.
#[derive(Debug, Default, PartialEq, Eq)]
struct Pixels {
    pixels: Vec<u8>,
    rows: usize,
}

impl Decoder for Pixels {
    fn push_pixel(&mut self, black: bool) {
        self.pixels.push(u8::from(black));
    }

    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        let n = chunk_count as usize * 8;
        self.pixels.resize(self.pixels.len() + n, u8::from(black));
    }

    fn next_line(&mut self) {
        self.rows += 1;
    }
}

impl Pixels {
    fn black(&self) -> usize {
        self.pixels.iter().filter(|&&p| p == 1).count()
    }
}

/// Decodes an embedded JBIG2 stream through `decode_with` under `budget`.
fn decode(stream: &[u8], budget: Option<u64>) -> Result<Pixels, DecodeError> {
    let image = hayro_jbig2::Image::new_embedded(stream, None)?;
    let mut ctx = DecoderContext::default();
    ctx.memory_budget = budget;
    let mut pixels = Pixels::default();
    image.decode_with(&mut pixels, &mut ctx)?;
    Ok(pixels)
}

/// The smallest budget `stream` decodes under (binary search; the stream
/// must decode under 1 GiB).
fn min_budget(stream: &[u8]) -> u64 {
    let (mut low, mut high) = (0, GIB);
    while low < high {
        let mid = low + (high - low) / 2;
        if decode(stream, Some(mid)).is_ok() {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}

/// The smallest budget under which `stream`'s decode is not refused: it
/// succeeds, or fails some other way, which for a stream cut short marks how
/// far the charges alone get it.
fn min_unrefused_budget(stream: &[u8]) -> u64 {
    let (mut low, mut high) = (0, GIB);
    while low < high {
        let mid = low + (high - low) / 2;
        if decode(stream, Some(mid)).err() == Some(REFUSED) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

/// `larger - smaller` for two minimum budgets of a differential test,
/// asserting their order first: a mutant that inverts it then fails an
/// assert naming both budgets instead of panicking on `u64` underflow.
fn excess(larger: u64, smaller: u64) -> u64 {
    assert!(
        larger >= smaller,
        "the larger shape needs {larger} bytes, less than the smaller's {smaller}"
    );
    larger - smaller
}

/// Asserts `stream` decodes under `budget` to exactly the pixels it decodes
/// to without one (F10), and returns them.
fn assert_ok_and_unchanged(stream: &[u8], budget: u64) -> Pixels {
    let budgeted = decode(stream, Some(budget)).expect("decodes within the budget");
    let unbudgeted = decode(stream, None).expect("decodes without a budget");
    assert_eq!(budgeted, unbudgeted, "the budget changed the pixels");
    budgeted
}

// ---------------------------------------------------------------------------
// hayro-jbig2

#[test]
fn a_symbol_exported_65535_times_is_refused_before_the_copies() {
    // One 8192 × 8192 symbol (8 MiB) named 65,535 times by an exporting
    // dictionary: unbudgeted, 512 GiB of copies. Never decoded without a
    // budget.
    let bomb = jbig2::symbol_export(8192, 65_535, 64, false);
    assert_eq!(decode(&bomb, Some(64 * MIB)).err(), Some(REFUSED));
    // Generous for anything but the copies: still refused.
    assert_eq!(decode(&bomb, Some(GIB)).err(), Some(REFUSED));
}

#[test]
fn exported_copies_are_counted() {
    // Control for the shape above at a size that decodes: a 64 × 64 symbol
    // (512 bytes) exported 4 times and drawn on a 192 × 192 page. The four
    // copies tip it over 8 KiB; one copy of the same symbol fits.
    let four = jbig2::symbol_export(64, 4, 192, true);
    let pixels = assert_ok_and_unchanged(&four, MIB);
    assert_eq!(pixels.rows, 192);
    assert_eq!(pixels.black(), 1, "the placed symbol's one black pixel");
    assert_eq!(decode(&four, Some(8 * KIB)).err(), Some(REFUSED));
    let one = jbig2::symbol_export(64, 1, 192, true);
    assert_eq!(assert_ok_and_unchanged(&one, 8 * KIB).black(), 1);
}

#[test]
fn a_40000_square_gray_scale_grid_is_refused_before_any_plane() {
    // Four bytes per cell: 6.4 GB for the value plane alone, from eight
    // header bytes. Never decoded without a budget.
    let halftone = jbig2::halftone_grid(40_000);
    assert_eq!(decode(&halftone, Some(GIB)).err(), Some(REFUSED));
    // A small grid of the same shape decodes, unchanged by the budget.
    assert_ok_and_unchanged(&jbig2::halftone_grid(4), MIB);
}

#[test]
fn intermediate_regions_are_counted_together() {
    // Three stored 4096 × 4096 regions, 2 MiB each: the budget is
    // cumulative, so 4 MiB holds only one of them with the page.
    let regions = jbig2::intermediate_regions(3, 4096);
    assert_ok_and_unchanged(&regions, 8 * MIB);
    assert_eq!(decode(&regions, Some(4 * MIB)).err(), Some(REFUSED));
}

#[test]
fn a_text_region_naming_a_dictionary_1000_times_is_refused_below_its_cost() {
    // Naming a one-symbol dictionary 1,000 times costs the region at least
    // its 8,000-byte symbol list. This only bounds the total: the code table
    // the region builds per named symbol is charged too, so the refusal
    // would happen without the list's charge. The next test isolates that.
    let refs = jbig2::text_region_refs(1000);
    assert_eq!(decode(&refs, Some(8 * 1000 - 1)).err(), Some(REFUSED));
    let pixels = assert_ok_and_unchanged(&refs, MIB);
    assert_eq!(pixels.black(), 1, "symbol 999 is drawn");
}

#[test]
fn a_text_regions_symbol_list_costs_eight_bytes_per_named_symbol() {
    // The region's data stops before the instance count, so the decode ends
    // (in a parse error, not a refusal) right after the symbol list is
    // collected and before anything sized by the list is built. The least
    // budget that gets that far is therefore the list's cost on top of a
    // fixed part: 1,000 more names cost exactly 8,000 more bytes.
    let need = |refs| min_unrefused_budget(&jbig2::text_region_cut_short(refs));
    assert_eq!(excess(need(2000), need(1000)), 8 * 1000);
    let cut = jbig2::text_region_cut_short(1000);
    assert!(matches!(decode(&cut, Some(GIB)), Err(e) if e != REFUSED));
}

#[test]
fn a_symbol_list_costs_eight_bytes_per_named_symbol() {
    // A dictionary naming the base dictionary N times, adding and exporting
    // nothing: N only lengthens its input list, so 1,000 more names cost
    // exactly 8,000 more bytes of budget.
    let need = |refs| min_budget(&jbig2::referring_dictionary(refs));
    assert_eq!(excess(need(2000), need(1000)), 8 * 1000);
    assert_ok_and_unchanged(&jbig2::referring_dictionary(1000), MIB);
}

// F10 for the symbol-dictionary bitmap sources that the shapes above do not
// reach with a drawn result. Each stream is decoded at the least budget it
// fits in (so any charge a path adds or drops shows) and must match the
// unbudgeted decode; one byte less is refused.

/// Asserts `stream` decodes, unchanged, under exactly its minimum budget and
/// is refused under one byte less; returns the pixels.
fn assert_tight_and_unchanged(stream: &[u8]) -> Pixels {
    let need = min_budget(stream);
    assert_eq!(decode(stream, Some(need - 1)).err(), Some(REFUSED));
    assert_ok_and_unchanged(stream, need)
}

#[test]
fn an_aggregated_symbol_decodes_unchanged_under_its_minimum_budget() {
    // REFAGGNINST = 2: a 64 × 64 symbol built by placing the base symbol
    // twice, exported once and drawn on a 192 × 192 page.
    let pixels = assert_tight_and_unchanged(&jbig2::symbol_export(64, 1, 192, true));
    assert_eq!((pixels.rows, pixels.black()), (192, 1));
}

/// A 16 × 16 ring with a diagonal through it: a refinement target that
/// shares only its corner with the one-pixel reference.
const RING: [&str; 16] = [
    "#...............",
    ".##############.",
    ".#............#.",
    ".#.#..........#.",
    ".#..#.........#.",
    ".#...#........#.",
    ".#....#.......#.",
    ".#.....#......#.",
    ".#......#.....#.",
    ".#.......#....#.",
    ".#........#...#.",
    ".#.........#..#.",
    ".#..........#.#.",
    ".#............#.",
    ".##############.",
    "................",
];

/// The black pixels of `pixels` as (x, y), in row order.
fn black_pixels(pixels: &Pixels) -> Vec<(usize, usize)> {
    let width = pixels.pixels.len() / pixels.rows;
    (0..pixels.pixels.len())
        .filter(|&i| pixels.pixels[i] == 1)
        .map(|i| (i % width, i / width))
        .collect()
}

/// The black pixels of `rows` (`#` black) drawn with its top-left corner at
/// `at`, as (x, y) in row order.
fn drawn(rows: &[&str], at: (usize, usize)) -> Vec<(usize, usize)> {
    let mut black = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c == '#' {
                black.push((at.0 + x, at.1 + y));
            }
        }
    }
    black
}

#[test]
fn a_refined_symbol_decodes_unchanged_under_its_minimum_budget() {
    // REFAGGNINST = 1: a 16 × 16 symbol refined from the base symbol, its
    // refinement block coded by the test kit's MQ encoder, drawn at (3, 5).
    let stream = jbig2::refined_symbol(16, &jbig2::refinement_data(&["#"], &RING, (0, 0)));
    let pixels = assert_tight_and_unchanged(&stream);
    assert_eq!(black_pixels(&pixels), drawn(&RING, (3, 5)));
}

#[test]
fn an_arithmetic_symbol_decodes_unchanged_under_its_minimum_budget() {
    // SDHUFF = 0, SDREFAGG = 0: two symbols of one height class, each coded
    // with generic template 0 (6.5.8.1), the wider one drawn at (3, 5).
    let narrow: &[&str] = &["#..#", ".##.", "#..#"];
    let wide: &[&str] = &["##..##", "#.##.#", ".####."];
    let stream = jbig2::arithmetic_symbols(&[narrow, wide], 1, (3, 5), (64, 64));
    let pixels = assert_tight_and_unchanged(&stream);
    assert_eq!(black_pixels(&pixels), drawn(wide, (3, 5)));
}

/// `count` pseudo-random rows of `width` pixels (`#` black), about half
/// black, from a fixed seed, so the coder meets every context and has no
/// long runs to settle on.
fn noise(width: usize, count: usize, seed: u64) -> Vec<String> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            (0..width)
                .map(|_| {
                    // Knuth's MMIX LCG; the top bit is the pixel.
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    if state >> 63 == 1 { '#' } else { '.' }
                })
                .collect()
        })
        .collect()
}

#[test]
fn the_test_kits_mq_encoder_round_trips_through_the_decoder() {
    // Symbols chosen so the dictionary's integers (height-class and width
    // deltas, the export runs) fall in every range of Table A.1 with both
    // signs, OOB ending each class, and so the bitmaps cross a 32-pixel word
    // and run to thousands of noisy pixels. Each symbol is drawn alone and
    // must come back exactly as written. Each class's noise is seeded with
    // `seed + width`; the last class's seed was searched for (by
    // instrumenting the encoder) to make BYTEOUT carry into a 0xFE byte, its
    // rarest path, which the other data never reaches.
    let classes: [(usize, &[usize], u64); 6] = [
        (1, &[4440, 2], 10_000), // height +1; widths +4440, -4438
        (5, &[7, 40], 50_000),   // +4; +7, +33
        (105, &[3], 1_050_000),  // +100; +3
        (605, &[2], 6_050_000),  // +500; +2
        (4, &[3, 1], 40_000),    // -601; +3, -2
        (40, &[40], 400_034),    // +36; +40
    ];
    let mut symbols: Vec<Vec<String>> = Vec::new();
    for (height, widths, seed) in classes {
        for &width in widths {
            symbols.push(noise(width, height, seed + width as u64));
        }
    }
    let rows: Vec<Vec<&str>> = symbols
        .iter()
        .map(|s| s.iter().map(String::as_str).collect())
        .collect();
    let slices: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
    for (k, symbol) in slices.iter().enumerate() {
        let (width, height) = (symbol[0].len(), symbol.len());
        let page = (width as u32 + 3, height as u32 + 4);
        let stream = jbig2::arithmetic_symbols(&slices, k as u32, (1, 2), page);
        let pixels = decode(&stream, None).expect("decodes");
        assert_eq!(
            black_pixels(&pixels),
            drawn(symbol, (1, 2)),
            "symbol {k} ({width} × {height})"
        );
    }
    // The refinement coder too: a noisy 40 × 40 target against a noisy
    // 37 × 35 reference (a width that pads its stored rows), so every
    // reference pixel of the template varies, at offsets of each sign. The
    // positive-RDX offsets, which the decoder got wrong until H21 (internal #115),
    // are round-tripped in `h21_jbig2_refinement_offsets.rs`.
    let (target, reference) = (noise(40, 40, 7), noise(37, 35, 8));
    let target: Vec<&str> = target.iter().map(String::as_str).collect();
    let reference: Vec<&str> = reference.iter().map(String::as_str).collect();
    for offset in [(0, 0), (-1, 1), (-1, -1)] {
        let stream = jbig2::refined_from(&reference, &target, offset);
        let pixels = decode(&stream, None).expect("decodes");
        assert_eq!(
            black_pixels(&pixels),
            drawn(&target, (3, 5)),
            "offset {offset:?}"
        );
    }
}

#[test]
fn an_mmr_collective_bitmap_decodes_unchanged_under_its_minimum_budget() {
    // The height class's symbols are cut from one MMR-coded bitmap; the
    // wider symbol (3 rows, its first column black) is drawn.
    let pixels = assert_tight_and_unchanged(&jbig2::mmr_collective_bitmap());
    assert_eq!(pixels.black(), 3);
}

#[test]
fn a_refined_instance_of_an_aggregated_symbol_decodes_unchanged_under_its_minimum_budget() {
    // R_I = 1 inside the text region an aggregate symbol embeds (internal #65 C6):
    // the base symbol, then the base symbol refined into the ring, both at
    // the origin of a 16 × 16 symbol drawn at (3, 5). The ring's (0, 0) is
    // black, so the plain instance adds nothing to it.
    let pixels = assert_tight_and_unchanged(&jbig2::refined_aggregate((16, 16), &RING));
    assert_eq!(black_pixels(&pixels), drawn(&RING, (3, 5)));
}

#[test]
fn a_refined_instance_of_a_page_text_region_decodes_unchanged_under_its_minimum_budget() {
    // R_I = 1 in a text region of the page itself (SBREFINE = 1), the
    // refinement path an aggregate does not take: the base symbol refined
    // into the ring and drawn at (3, 5).
    let pixels = assert_tight_and_unchanged(&jbig2::refined_text_region(&RING));
    assert_eq!(black_pixels(&pixels), drawn(&RING, (3, 5)));
}

// Per-charge tightness (internal #65 B6). The minimum-budget tests above find the
// budget by search, so a bitmap charge dropped or doubled moves the minimum
// with it and they still pass. Each test below decodes one stream shape at
// two sizes of the bitmap one charge site allocates, all else fixed, and
// checks that the minimum budgets differ by what F6 says that bitmap costs,
// once per charge: dropping the charge or making it twice shows.

/// F6's charge for a `width × height` bitmap: `stride × height × 4` bytes,
/// `stride` being the 32-bit words of one row. Computed here from the spec
/// rather than by the decoder's own helper, which is what is under test.
fn bitmap_bytes(width: usize, height: usize) -> u64 {
    (width.div_ceil(32) * height * 4) as u64
}

/// `rows` as the `&str` slice the test kit takes.
fn strs(rows: &[String]) -> Vec<&str> {
    rows.iter().map(String::as_str).collect()
}

#[test]
fn an_arithmetic_symbol_costs_its_bitmap_and_its_export_copy() {
    // `arithmetic_symbols` exports every symbol, so the undrawn first
    // symbol is charged twice: once as decoded (symbol.rs, generic coding
    // of a symbol) and once as its export copy. Widening it from 4 to 100
    // pixels (one word a row to four) changes nothing else.
    let wide: &[&str] = &["##..##", "#.##.#", ".####."];
    let need = |width: usize| {
        let first = noise(width, 3, 11);
        let first = strs(&first);
        min_budget(&jbig2::arithmetic_symbols(
            &[&first, wide],
            1,
            (3, 5),
            (64, 64),
        ))
    };
    let symbol = bitmap_bytes(100, 3) - bitmap_bytes(4, 3);
    assert_eq!(excess(need(100), need(4)), 2 * symbol);
}

#[test]
fn a_refined_symbol_costs_its_bitmap_and_its_export_copy() {
    // REFAGGNINST = 1: the new symbol is the refinement bitmap itself,
    // then copied once for export; the reference and page stay as they are.
    let need = |target: &[&str]| min_budget(&jbig2::refined_from(&["#"], target, (0, 0)));
    let big = noise(40, 24, 3);
    let symbol = bitmap_bytes(40, 24) - bitmap_bytes(16, 16);
    assert_eq!(excess(need(&strs(&big)), need(&RING)), 2 * symbol);
}

#[test]
fn an_aggregated_symbol_costs_its_bitmap_and_its_export_copy() {
    // REFAGGNINST = 2: the aggregation bitmap becomes the symbol, then is
    // copied once for export; the instances it places are the base symbol
    // whatever its size.
    let need = |width, height| min_budget(&jbig2::repeated_symbol(width, height, 1, 192));
    let symbol = bitmap_bytes(100, 80) - bitmap_bytes(64, 64);
    assert_eq!(excess(need(100, 80), need(64, 64)), 2 * symbol);
}

#[test]
fn a_refined_instance_costs_its_bitmap_once() {
    // The instance's refined bitmap (text.rs) is drawn into the aggregate
    // and dropped, never copied; the aggregate stays 48 × 48.
    let need = |target: &[&str]| min_budget(&jbig2::refined_aggregate((48, 48), target));
    let mut big = noise(40, 33, 5);
    big[0].replace_range(0..1, "#"); // so the plain instance adds nothing
    let instance = bitmap_bytes(40, 33) - bitmap_bytes(16, 16);
    assert_eq!(excess(need(&strs(&big)), need(&RING)), instance);
    // The larger instance (its RDW and RDH in Table B.15's upper range)
    // decodes exactly, unchanged by the budget, too.
    let pixels = assert_tight_and_unchanged(&jbig2::refined_aggregate((48, 48), &strs(&big)));
    assert_eq!(black_pixels(&pixels), drawn(&strs(&big), (3, 5)));
}

#[test]
fn a_refined_instance_of_a_page_text_region_costs_its_bitmap_once() {
    // The same charge (text.rs) reached from a page-level text region: the
    // refined bitmap is drawn into the region and dropped, and nothing else
    // in the stream grows with it, so the minimum budgets of a 40 × 33 and a
    // 16 × 16 target differ by one such bitmap.
    let need = |target: &[&str]| min_budget(&jbig2::refined_text_region(target));
    let big = noise(40, 33, 5);
    let instance = bitmap_bytes(40, 33) - bitmap_bytes(16, 16);
    assert_eq!(excess(need(&strs(&big)), need(&RING)), instance);
    let pixels = assert_tight_and_unchanged(&jbig2::refined_text_region(&strs(&big)));
    assert_eq!(black_pixels(&pixels), drawn(&strs(&big), (3, 5)));
}

/// `decode` on its own thread, giving up after `limit`: a decode that spins
/// fails the test instead of hanging it (the spinning thread is left behind
/// and dies with the test process).
fn decode_within(
    stream: Vec<u8>,
    budget: Option<u64>,
    limit: Duration,
) -> Option<Result<Pixels, DecodeError>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(decode(&stream, budget));
    });
    rx.recv_timeout(limit).ok()
}

#[test]
fn export_flags_that_stop_advancing_after_the_data_are_refused() {
    // internal #94: with its two bytes used up, this IAEX stream decodes to runs of
    // length 0 forever, so the export loop never reached its end and the
    // decode spun at full CPU, charging nothing to any budget. A decode takes
    // microseconds; the limit only has to tell an answer from none.
    let stalled = jbig2::arithmetic_export_dictionary(&[0xAC, 0x01]);
    for budget in [None, Some(MIB)] {
        let result = decode_within(stalled.clone(), budget, Duration::from_secs(20))
            .expect("the decode returns instead of spinning");
        assert_eq!(
            result.err(),
            Some(DecodeError::Symbol(SymbolError::ExportStalled))
        );
    }
    // Control: one byte, so the decoder is out of data from the start and
    // both runs (0, then the exported symbol's 1) come from the padding. A
    // single 0-length run after the data is legitimate and still decodes.
    let pixels = assert_ok_and_unchanged(&jbig2::arithmetic_export_dictionary(&[0xA3]), MIB);
    assert_eq!(pixels.black(), 0, "the dictionary is never drawn");
}

#[test]
fn the_page_bitmap_is_counted() {
    // 1000 × 1000 pixels in 32-pixel words: 32 words × 1000 rows × 4 bytes =
    // 128,000 bytes.
    let page = jbig2::blank_page(1000, 1000);
    let pixels = assert_ok_and_unchanged(&page, 128 * KIB);
    assert_eq!((pixels.rows, pixels.pixels.len()), (1000, 1_000_000));
    assert_eq!(decode(&page, Some(100 * KIB)).err(), Some(REFUSED));
}

#[test]
fn the_budget_starts_afresh_with_each_decode() {
    // F9: one context, two decodes, each within the budget on its own and
    // not together.
    let page = jbig2::blank_page(1000, 1000);
    let image = hayro_jbig2::Image::new_embedded(&page, None).expect("parses");
    let mut ctx = DecoderContext::default();
    ctx.memory_budget = Some(128 * KIB);
    for _ in 0..2 {
        image
            .decode_with(&mut Pixels::default(), &mut ctx)
            .expect("each decode has the whole budget");
    }
}

#[test]
fn decoding_without_a_context_is_unbudgeted() {
    // `Image::decode` and a default context have no budget.
    assert_eq!(DecoderContext::default().memory_budget, None);
    let page = jbig2::blank_page(1000, 1000);
    let image = hayro_jbig2::Image::new_embedded(&page, None).expect("parses");
    let mut pixels = Pixels::default();
    image.decode(&mut pixels).expect("decodes");
    assert_eq!(pixels, decode(&page, None).expect("decodes"));
}

#[test]
fn the_refusal_is_reported_as_a_memory_budget_overflow() {
    assert_eq!(
        REFUSED.to_string(),
        "the decode would exceed its memory budget"
    );
}

// ---------------------------------------------------------------------------
// Resident memory of the two refusals that would otherwise allocate the most.
//
// Measured in a child process running only `rss_child`: peak RSS
// (`getrusage`) is process-wide and never goes down, so in the shared test
// process any earlier test's allocations would hide the decode's.

/// Selects the case `rss_child` measures; unset, `rss_child` does nothing.
const RSS_CASE: &str = "H14_RSS_CASE";

/// Peak resident set size of this process, in bytes.
fn peak_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` writes a complete `rusage` into the pointer it is
    // given when it returns 0; the pointer is to a live, properly aligned,
    // zero-initialised `rusage` that outlives the call. On failure the
    // zeroed value is never read.
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    assert_eq!(rc, 0, "getrusage");
    // SAFETY: rc == 0, so the kernel filled the struct; zeroed is a valid
    // `rusage` in any case (all fields are integers).
    let maxrss = unsafe { usage.assume_init() }.ru_maxrss;
    let maxrss = u64::try_from(maxrss).expect("non-negative");
    // Linux reports KiB, macOS bytes.
    if cfg!(target_os = "macos") {
        maxrss
    } else {
        maxrss * 1024
    }
}

#[test]
fn rss_child() {
    let Ok(case) = std::env::var(RSS_CASE) else {
        return;
    };
    let (stream, budget) = match case.as_str() {
        "symbol_export" => (jbig2::symbol_export(8192, 65_535, 64, false), 64 * MIB),
        "halftone" => (jbig2::halftone_grid(40_000), GIB),
        other => panic!("unknown case {other}"),
    };
    let before = peak_rss_bytes();
    assert_eq!(decode(&stream, Some(budget)).err(), Some(REFUSED));
    println!(
        "\nH14_RSS_GROWTH={}",
        peak_rss_bytes().saturating_sub(before)
    );
}

/// Runs `rss_child` for `case` in a child process and returns its peak RSS
/// growth over the decode.
fn rss_growth(case: &str) -> u64 {
    let exe = std::env::current_exe().expect("own path");
    let out = Command::new(exe)
        .args(["--exact", "rss_child", "--nocapture", "--test-threads=1"])
        .env(RSS_CASE, case)
        .output()
        .expect("run child");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "child failed: {stdout}");
    stdout
        .lines()
        .find_map(|l| l.split_once("H14_RSS_GROWTH=").map(|(_, n)| n))
        .unwrap_or_else(|| panic!("no measurement in {stdout}"))
        .trim()
        .parse()
        .expect("a number")
}

#[test]
fn refusals_happen_before_the_memory_is_touched() {
    // The 6.4 GB gray-scale plane is refused before any plane exists.
    let growth = rss_growth("halftone");
    assert!(
        growth < 16 * MIB,
        "halftone: peak RSS grew by {growth} bytes"
    );

    // An 8 MiB symbol, then 65,535 copies of it under a 64 MiB budget: the
    // copies are refused as a batch before the first is made. T.88 lets a
    // dictionary export each of its symbols once, so the 65,535 names go
    // through a dictionary that exported the symbol, and that one copy is
    // legitimate work: the symbol (barely touched: one black pixel) and one
    // full copy, which on macOS also faults in the copied-from pages, plus
    // the 512 KiB name list: about 18 MB on macOS, 9 MB on Linux. Refusing
    // copy by copy instead touches five more copies before the budget runs
    // out (61 MB measured on macOS); no budget, 512 GiB.
    let growth = rss_growth("symbol_export");
    assert!(
        growth < 24 * MIB,
        "symbol_export: peak RSS grew by {growth} bytes"
    );
}

// ---------------------------------------------------------------------------
// Requested heap bytes of the same two refusals.
//
// Peak RSS cannot tell "charged, then refused" from "allocated, then
// charged" when the allocation is zeroed and never touched (the OS maps it
// lazily). The allocator below counts what is requested, per thread, so a
// buffer allocated before its charge shows up whether or not it is touched.

/// The system allocator plus per-thread accounting of live heap bytes and
/// their peak (as in `h2b_decode_limits.rs`).
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

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged; see the impl comment.
        let ptr = unsafe { System.alloc_zeroed(layout) };
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
    let _ = LIVE.try_with(|live| live.set(live.get().saturating_sub(bytes)));
}

/// The peak heap bytes this thread held during `f`, above its starting
/// figure.
fn peak_heap_during(f: impl FnOnce()) -> u64 {
    let start = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(start));
    f();
    (PEAK.with(Cell::get) - start) as u64
}

#[test]
fn refusals_happen_before_the_allocation() {
    // The gray-scale planes are never requested.
    let halftone = jbig2::halftone_grid(40_000);
    let peak = peak_heap_during(|| {
        assert_eq!(decode(&halftone, Some(GIB)).err(), Some(REFUSED));
    });
    assert!(peak < MIB, "halftone: {peak} bytes requested");
    // The symbol (8 MiB), its one legitimate exported copy (8 MiB) and the
    // 65,535-entry name list (512 KiB); none of the refused copies.
    let bomb = jbig2::symbol_export(8192, 65_535, 64, false);
    let peak = peak_heap_during(|| {
        assert_eq!(decode(&bomb, Some(64 * MIB)).err(), Some(REFUSED));
    });
    assert!(peak < 17 * MIB, "symbol_export: {peak} bytes requested");
}

// ---------------------------------------------------------------------------
// hayro-syntax

/// The pixel cap of the hayro-syntax tests: a 2,000,000-byte JBIG2 budget.
const CAP: u64 = 1_000_000;

fn limits(max_image_pixels: Option<u64>) -> DecodeLimits {
    DecodeLimits {
        max_image_pixels,
        ..DecodeLimits::default()
    }
}

/// Decodes the image XObject of `jbig2::image_pdf` (object 1) as a
/// `width × height` bilevel image under `max_image_pixels`.
fn decode_image(
    jbig2_stream: &[u8],
    width: u32,
    height: u32,
    max_image_pixels: Option<u64>,
) -> Result<Vec<u8>, DecodeFailure> {
    let pdf = Pdf::new_with_options(
        jbig2::image_pdf(jbig2_stream, width, height),
        "",
        limits(max_image_pixels),
    )
    .expect("opens");
    let image = pdf
        .xref()
        .get::<Stream<'_>>(ObjectIdentifier::new(1, 0))
        .expect("image stream");
    let params = ImageDecodeParams {
        bpc: Some(1),
        num_components: Some(1),
        width,
        height,
        ..ImageDecodeParams::default()
    };
    image.decoded_image(&params).map(|r| r.data.into_owned())
}

#[test]
fn the_budget_is_two_bytes_per_pixel_of_the_cap() {
    assert_eq!(JBIG2_BUDGET_BYTES_PER_PIXEL, 2);
    // Three stored 4096 × 4096 regions on a 64 × 64 page need about 6 MB, far
    // more than the page's own pixels: the smallest cap that holds them is
    // half that (rounded up), and one pixel less is refused.
    let regions = jbig2::intermediate_regions(3, 4096);
    let cap = min_budget(&regions).div_ceil(JBIG2_BUDGET_BYTES_PER_PIXEL);
    assert!(decode_image(&regions, 64, 64, Some(cap)).is_ok());
    let before = image_memory_limit_hits();
    assert!(matches!(
        decode_image(&regions, 64, 64, Some(cap - 1)),
        Err(DecodeFailure::ImageMemoryLimit)
    ));
    assert_eq!(image_memory_limit_hits() - before, 1);
}

#[test]
fn a_symbol_export_bomb_is_an_image_memory_limit() {
    let bomb = jbig2::symbol_export(8192, 65_535, 64, false);
    let (memory, pixels) = (image_memory_limit_hits(), image_pixel_limit_hits());
    assert!(matches!(
        decode_image(&bomb, 64, 64, Some(CAP)),
        Err(DecodeFailure::ImageMemoryLimit)
    ));
    assert_eq!(image_memory_limit_hits() - memory, 1);
    assert_eq!(image_pixel_limit_hits(), pixels);
}

#[test]
fn the_pixel_cap_still_fires_first() {
    // A 20000 × 20000 page under a 64 × 64 dictionary: H13's header check
    // refuses it before the decoder (and its budget) is reached.
    let page = jbig2::blank_page(20_000, 20_000);
    let (memory, pixels) = (image_memory_limit_hits(), image_pixel_limit_hits());
    assert!(matches!(
        decode_image(&page, 64, 64, Some(CAP)),
        Err(DecodeFailure::ImagePixelLimit)
    ));
    assert_eq!(image_pixel_limit_hits() - pixels, 1);
    assert_eq!(image_memory_limit_hits(), memory);
}

#[test]
fn a_malformed_stream_is_still_a_decode_failure() {
    // Its region is too short for its own header, so decoding (not parsing)
    // fails, inside the budgeted decode.
    let broken = jbig2::truncated_region();
    assert!(hayro_jbig2::Image::new_embedded(&broken, None).is_ok());
    let (memory, pixels) = (image_memory_limit_hits(), image_pixel_limit_hits());
    assert!(matches!(
        decode_image(&broken, 64, 64, Some(CAP)),
        Err(DecodeFailure::ImageDecode)
    ));
    assert_eq!(image_memory_limit_hits(), memory);
    assert_eq!(image_pixel_limit_hits(), pixels);
}

#[test]
fn an_image_within_the_budget_decodes_as_before() {
    // The exported-copies control drawn as a 192 × 192 image: identical
    // bytes with the cap (budget 2 MB) and without.
    let control = jbig2::symbol_export(64, 4, 192, true);
    let capped = decode_image(&control, 192, 192, Some(CAP)).expect("decodes");
    let uncapped = decode_image(&control, 192, 192, None).expect("decodes");
    assert_eq!(capped, uncapped);
    assert_eq!(capped.len(), 192 * 192);
    assert_eq!(
        capped.iter().filter(|&&b| b == 0).count(),
        1,
        "one black pixel"
    );
}

#[test]
fn hits_are_counted_per_thread() {
    let bomb = jbig2::symbol_export(8192, 65_535, 64, false);
    let before = image_memory_limit_hits();
    let theirs = std::thread::scope(|s| {
        s.spawn(|| {
            let mine = image_memory_limit_hits();
            for _ in 0..2 {
                let _ = decode_image(&bomb, 64, 64, Some(CAP));
            }
            image_memory_limit_hits() - mine
        })
        .join()
        .expect("decoding thread")
    });
    assert_eq!(theirs, 2);
    assert_eq!(image_memory_limit_hits(), before);
}

// ---------------------------------------------------------------------------
// hayro-interpret

/// A device that asks for every image's pixels, as a renderer does, and
/// counts the images whose pixels arrived.
#[derive(Default)]
struct Decoding {
    decoded: usize,
}

impl<'a> Device<'a> for Decoding {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(
        &mut self,
        _: &Glyph<'a>,
        _: Affine,
        _: Affine,
        _: &Paint<'a>,
        _: &GlyphDrawMode,
    ) {
    }
    fn draw_image(&mut self, image: Image<'a, '_>, _: Affine) {
        let mut got = false;
        match image {
            Image::Raster(r) => r.with_rgba(|_, _| got = true, None),
            Image::Stencil(s) => s.with_stencil(|_, _| got = true, None),
        }
        self.decoded += usize::from(got);
    }
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// Interprets page 0 of `bytes` with `Decoding` and returns the
/// interpreter's warnings and how many images were decoded.
fn render(bytes: Vec<u8>, decode: DecodeLimits) -> (Vec<String>, usize) {
    let pdf = Pdf::new_with_options(bytes, "", decode).expect("opens");
    let page = &pdf.pages()[0];
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let settings = InterpreterSettings {
        warning_sink: {
            let warnings = warnings.clone();
            Arc::new(move |w: InterpreterWarning| {
                warnings.lock().expect("sink").push(format!("{w:?}"));
            })
        },
        ..InterpreterSettings::default()
    };
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(-1e5, -1e5, 1e5, 1e5),
        &cache,
        pdf.xref(),
        settings,
    );
    let mut device = Decoding::default();
    interpret_page(page, &mut context, &mut device);
    let warnings = warnings.lock().expect("sink").clone();
    (warnings, device.decoded)
}

#[test]
fn an_xobject_over_the_budget_is_one_memory_limit_warning() {
    let (memory, pixels) = (image_memory_limit_hits(), image_pixel_limit_hits());
    let (warnings, decoded) = render(jbig2::symbol_export_bomb_pdf(), limits(Some(CAP)));
    assert_eq!(warnings, ["ImageMemoryLimit"]);
    assert_eq!(decoded, 0);
    assert_eq!(image_memory_limit_hits() - memory, 1);
    assert_eq!(image_pixel_limit_hits(), pixels);
}

#[test]
fn an_xobject_within_the_budget_still_draws() {
    let control = jbig2::image_pdf(&jbig2::symbol_export(64, 4, 192, true), 192, 192);
    let (warnings, decoded) = render(control, limits(Some(CAP)));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(decoded, 1);
}

// JBIG2 is not an inline-image filter (ISO 32000-2 Table 92), so there is
// no inline counterpart of these tests.

// ---------------------------------------------------------------------------
// core plumbing

/// `Limits::max_image_pixels` sets the budget: a document opened through core
/// refuses the bomb through the rendering hook's xref.
#[cfg(feature = "render-support")]
#[test]
fn core_opens_documents_with_the_budget() {
    use pdf_extract_core::{Document, Limits, OpenOptions};

    let opts = OpenOptions {
        limits: Limits {
            max_image_pixels: CAP,
            ..Limits::default()
        },
        ..OpenOptions::default()
    };
    let doc = Document::from_bytes(jbig2::symbol_export_bomb_pdf(), &opts).expect("opens");
    let result = doc
        .page(0)
        .expect("page 0")
        .with_hayro_page(|input| {
            let stream = input
                .xref
                .get::<Stream<'_>>(ObjectIdentifier::new(1, 0))
                .expect("image stream");
            let params = ImageDecodeParams {
                bpc: Some(1),
                num_components: Some(1),
                width: 64,
                height: 64,
                ..ImageDecodeParams::default()
            };
            stream.decoded_image(&params).map(|_| ())
        })
        .expect("open document");
    assert!(matches!(result, Err(DecodeFailure::ImageMemoryLimit)));
}
