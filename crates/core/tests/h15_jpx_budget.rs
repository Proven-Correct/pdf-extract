//! H15: the JPEG 2000 decoder's memory budget in the vendored fork (run 0003
//! spec F5, F7–F10, oracle §4.5).
//!
//! `hayro_jpeg2000::DecodeSettings::max_memory_bytes` bounds the total bytes
//! one decode allocates for buffers whose size the codestream declares; each
//! is charged before it is allocated and a decode that would exceed the
//! budget fails with `Validation(MemoryBudget)`. `Image::new` already refuses
//! a header whose tile records alone exceed it. hayro-syntax sets the budget
//! to `JPX_BUDGET_BYTES_PER_PIXEL × max_image_pixels`, reports a refusal as
//! `DecodeFailure::ImageMemoryLimit` and counts it in the thread-local
//! `image_memory_limit_hits()` (the H14 plumbing); hayro-interpret raises
//! `InterpreterWarning::ImageMemoryLimit` for it, and nothing else.
//!
//! The codestreams come from `pdf_extract_testkit::jpx`: header-patched
//! copies of a real 64 × 64 image for the refusals from the header, and
//! assembled codestreams with empty tile-parts (which decode, flat) for
//! shapes that must also decode. `assets/precinct1_gray_128.j2k` is the one
//! encoder-written image with 1 × 1 code-blocks (see `assets/PROVENANCE.md`).
//! Shapes that would allocate gigabytes unbudgeted are only ever decoded
//! under a budget; the largest refusal is also checked for resident memory,
//! in a child process.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::process::Command;
use std::sync::{Arc, Mutex};

use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterSettings, InterpreterWarning, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro_jpeg2000::{DecodeError, DecodeSettings, DecoderContext, ValidationError};
use hayro_syntax::object::stream::{DecodeFailure, ImageDecodeParams};
use hayro_syntax::object::{ObjectIdentifier, Stream};
use hayro_syntax::{
    DecodeLimits, JPX_BUDGET_BYTES_PER_PIXEL, Pdf, image_memory_limit_hits, image_pixel_limit_hits,
};
use kurbo::{Affine, BezPath, Rect};
use pdf_extract_testkit::jpx::{self, Shape};

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

const REFUSED: DecodeError = DecodeError::Validation(ValidationError::MemoryBudget);

/// A 128 × 128 gray image with precinct exponent 1 at every resolution, so
/// 1 × 1 code-blocks at every resolution above the lowest, written by
/// OpenJPEG 2.5.4 (`assets/PROVENANCE.md`).
const PRECINCT_1_GRAY_128: &[u8] = include_bytes!("assets/precinct1_gray_128.j2k");

/// What 1 × 1 code-blocks cost, in bytes of budget per image pixel: each
/// coefficient above the lowest resolution gets its own precinct, code-block,
/// layer record and two one-node tag trees. Measured at 242.1 for both
/// `PRECINCT_1_GRAY_128` and the 256 × 256 test-kit shape below (64-bit
/// targets), so above `JPX_BUDGET_BYTES_PER_PIXEL`: such an image is refused
/// once its pixels exceed about a quarter of `max_image_pixels`. No encoder
/// picks this geometry by default (OpenJPEG writes it only when told, and
/// its file is 12 times the size of the same image, losslessly coded, at the
/// default geometry); the ordinary configurations measured for unit 05 need
/// 15-42 bytes per pixel (512 × 512 OpenJPEG output: gray 15, RGB 33, RGB
/// in 128 × 128 tiles 17, four components 42).
const PRECINCT_1_BYTES_PER_PIXEL: u64 = 256;

fn settings(max_memory_bytes: Option<u64>) -> DecodeSettings {
    DecodeSettings {
        resolve_palette_indices: false,
        strict: false,
        target_resolution: None,
        max_memory_bytes,
    }
}

/// Parses and decodes `data` under `budget`, as hayro-syntax does.
fn decode(data: &[u8], budget: Option<u64>) -> Result<Vec<u8>, DecodeError> {
    hayro_jpeg2000::Image::new(data, &settings(budget))?.decode()
}

/// The smallest budget `data` decodes under (binary search; the image must
/// decode under 1 GiB).
fn min_budget(data: &[u8]) -> u64 {
    let (mut low, mut high) = (0, GIB);
    while low < high {
        let mid = low + (high - low) / 2;
        if decode(data, Some(mid)).is_ok() {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}

/// Asserts `data` decodes under `budget` to exactly the pixels it decodes to
/// without one (F10), and returns them.
fn assert_ok_and_unchanged(data: &[u8], budget: u64) -> Vec<u8> {
    let budgeted = decode(data, Some(budget)).expect("decodes within the budget");
    let unbudgeted = decode(data, None).expect("decodes without a budget");
    assert_eq!(budgeted, unbudgeted, "the budget changed the pixels");
    budgeted
}

/// The 1 × 1-code-block geometry over a `side`-square single tile: five
/// decompositions, precinct exponent 1 at every resolution.
fn precinct_1(side: u32) -> Shape {
    Shape {
        decompositions: 5,
        precinct_exponent: Some(1),
        ..Shape::new(side, side)
    }
}

// ---------------------------------------------------------------------------
// hayro-jpeg2000: tile records

#[test]
fn one_by_one_tiles_over_4000_square_are_refused_from_the_header() {
    // 16 million tiles, each a record of over a hundred bytes: refused by
    // `Image::new`, before any `Tile` is built. Never decoded unbudgeted.
    let tiles = jpx::patched_gray_64((4000, 4000), (1, 1), 1);
    assert_eq!(
        hayro_jpeg2000::Image::new(&tiles, &settings(Some(64 * MIB))).err(),
        Some(REFUSED)
    );
}

#[test]
fn a_one_by_one_tiled_image_decodes_unchanged_and_its_tiles_are_counted() {
    // 4,096 tiles of one pixel, each with its tile-part: about 171 bytes of
    // tile records per tile against 4 bytes of channel plane per pixel.
    let tiled = jpx::codestream(&Shape {
        tile: (1, 1),
        ..Shape::new(64, 64)
    });
    let pixels = assert_ok_and_unchanged(&tiled, 64 * MIB);
    assert_eq!(pixels, vec![128; 64 * 64], "empty tiles decode mid-gray");
    let need = min_budget(&tiled);
    assert!(need > 4096 * 100, "tile records are counted: {need}");
    assert_eq!(decode(&tiled, Some(4096 * 100)).err(), Some(REFUSED));
}

#[test]
fn a_tile_count_over_u32_is_refused_not_wrapped() {
    // 15.3 million tiles a side (60,000 pixels subsampled by 255): the
    // product does not fit the u32 the count is kept in. This test runs
    // unoptimised, with overflow checks: an unchecked multiply would panic.
    let wide = jpx::patched_gray_64((15_300_000, 15_300_000), (1, 1), 255);
    let too_large = DecodeError::Validation(ValidationError::ImageTooLarge);
    for budget in [None, Some(64 * MIB)] {
        assert_eq!(
            hayro_jpeg2000::Image::new(&wide, &settings(budget)).err(),
            Some(too_large)
        );
    }
}

#[test]
fn a_subsampled_grid_tiled_one_by_one_is_refused_from_the_header() {
    // A 200 × 200 image (well inside the 60,000 ceiling and any pixel cap)
    // on a 51,000-square reference grid of 1 × 1 tiles: 2.6 billion tiles.
    let grid = jpx::patched_gray_64((51_000, 51_000), (1, 1), 255);
    let image = hayro_jpeg2000::Image::new(&grid, &settings(None)).expect("parses");
    assert_eq!((image.width(), image.height()), (200, 200));
    let budget = JPX_BUDGET_BYTES_PER_PIXEL * 200 * 200;
    assert_eq!(
        hayro_jpeg2000::Image::new(&grid, &settings(Some(budget))).err(),
        Some(REFUSED)
    );
    // The test kit's hostile fixture, 3.6 billion tiles: refused under the
    // default cap's budget (16.4 GB) too.
    let bomb = jpx::tile_bomb();
    assert_eq!(
        hayro_jpeg2000::Image::new(&bomb, &settings(Some(64 * 256_000_000))).err(),
        Some(REFUSED)
    );
}

// ---------------------------------------------------------------------------
// hayro-jpeg2000: structural records, channel planes, output

#[test]
fn one_by_one_code_blocks_are_refused_as_they_are_built() {
    // 2048 × 2048 with 1 × 1 code-blocks: the header is cheap, so `Image::new`
    // accepts it, and the decode is refused while building the precincts
    // and code-blocks, before any packet is read.
    let shape = jpx::codestream(&precinct_1(2048));
    let budget = 16 * 2048 * 2048;
    assert!(hayro_jpeg2000::Image::new(&shape, &settings(Some(budget))).is_ok());
    assert_eq!(decode(&shape, Some(budget)).err(), Some(REFUSED));
}

#[test]
fn one_by_one_code_blocks_decode_unchanged_within_their_measured_cost() {
    // An encoder-written image (real content) and the test kit's shape at
    // 256 × 256 both need more than 64 bytes per pixel, and no more than
    // `PRECINCT_1_BYTES_PER_PIXEL`.
    for (data, side) in [
        (PRECINCT_1_GRAY_128.to_vec(), 128_u64),
        (jpx::codestream(&precinct_1(256)), 256),
    ] {
        let pixels = side * side;
        let pixels_out = assert_ok_and_unchanged(&data, PRECINCT_1_BYTES_PER_PIXEL * pixels);
        assert_eq!(pixels_out.len() as u64, pixels);
        assert_eq!(
            decode(&data, Some(JPX_BUDGET_BYTES_PER_PIXEL * pixels)).err(),
            Some(REFUSED)
        );
    }
    // The encoder's image is the real thing: a gradient with a dark square.
    let pixels = decode(PRECINCT_1_GRAY_128, None).expect("decodes");
    let distinct: std::collections::BTreeSet<u8> = pixels.iter().copied().collect();
    assert!(distinct.len() > 200, "{} distinct values", distinct.len());
}

#[test]
fn the_channel_planes_of_a_palette_image_are_charged() {
    // A palette image keeps one channel plane (its indices) of 512 × 512
    // f32, 1 MiB: the smallest budget it decodes under covers that plane
    // and the 256 KiB output. Under 1 MiB the header passes and the plane
    // is refused before it is allocated. (A palette image with more than
    // one component, the shape that once multiplied the planes, is now
    // refused from the header: see the internal #63 tests below.)
    let palette = jpx::jp2_palette(1, 512);
    assert!(min_budget(&palette) > 4 * 512 * 512 + 512 * 512);
    assert!(hayro_jpeg2000::Image::new(&palette, &settings(Some(MIB))).is_ok());
    assert_eq!(decode(&palette, Some(MIB)).err(), Some(REFUSED));
    let peak = peak_heap_during(|| {
        assert_eq!(decode(&palette, Some(MIB)).err(), Some(REFUSED));
    });
    assert!(peak < MIB, "palette: {peak} bytes requested");
    // The same palette over 64 × 64 decodes, unchanged.
    let small = jpx::jp2_palette(1, 64);
    assert_eq!(assert_ok_and_unchanged(&small, MIB).len(), 64 * 64);
}

#[test]
fn a_legitimate_image_needs_far_less_than_the_multiplier() {
    // The real 64 × 64 image (six resolutions, 64 × 64 code-blocks) needs
    // about 17 bytes per pixel, most of it the fixed cost of the
    // structures of so small an image.
    let gray = jpx::gray_64();
    let need = min_budget(&gray);
    assert!(need < 4096 * 32, "{need}");
    assert_ok_and_unchanged(&gray, need);
    assert_eq!(decode(&gray, Some(need - 1)).err(), Some(REFUSED));
}

#[test]
fn the_output_buffer_is_counted_and_a_callers_buffer_is_not() {
    // `decode` allocates the 4,096-byte output under the budget;
    // `decode_into` writes to the caller's buffer, which is not charged.
    let gray = jpx::gray_64();
    let need = min_budget(&gray);
    let image = hayro_jpeg2000::Image::new(&gray, &settings(Some(need - 4096))).expect("parses");
    assert_eq!(image.decode().err(), Some(REFUSED));
    let mut buf = vec![0; 4096];
    image
        .decode_into(&mut buf, &mut DecoderContext::default())
        .expect("the caller's buffer is free");
    assert_eq!(buf, decode(&gray, None).expect("decodes"));
    let image = hayro_jpeg2000::Image::new(&gray, &settings(Some(need - 4097))).expect("parses");
    assert_eq!(
        image
            .decode_into(&mut buf, &mut DecoderContext::default())
            .err(),
        Some(REFUSED)
    );
}

#[test]
fn the_budget_starts_afresh_with_each_decode() {
    // F9: one context, two decodes, each within the budget on its own and
    // not together.
    let gray = jpx::gray_64();
    let need = min_budget(&gray) - 4096;
    let image = hayro_jpeg2000::Image::new(&gray, &settings(Some(need))).expect("parses");
    let mut ctx = DecoderContext::default();
    let mut buf = vec![0; 4096];
    for _ in 0..2 {
        image
            .decode_into(&mut buf, &mut ctx)
            .expect("each decode has the whole budget");
    }
}

#[test]
fn decoding_without_a_limit_is_unbudgeted() {
    assert_eq!(DecodeSettings::default().max_memory_bytes, None);
    let gray = jpx::gray_64();
    let image = hayro_jpeg2000::Image::new(&gray, &DecodeSettings::default()).expect("parses");
    assert_eq!(
        image.decode().expect("decodes"),
        decode(&gray, None).expect("decodes")
    );
}

// ---------------------------------------------------------------------------
// hayro-jpeg2000: component counts the output cannot hold (internal #63)
//
// Both shapes panicked inside the decoder before the fork refused them, at
// any budget: the budget does not bound how many planes are interleaved
// into the output or how a component count is narrowed. Each is now refused
// by `Image::new` with `TooManyChannels`, which hayro-syntax reports as a
// broken image (`ImageDecode`).

const TOO_MANY_CHANNELS: DecodeError = DecodeError::Validation(ValidationError::TooManyChannels);

/// The budget hayro-syntax sets at the default `max_image_pixels`.
const DEFAULT_BUDGET: u64 = JPX_BUDGET_BYTES_PER_PIXEL * 256_000_000;

/// `settings(budget)` with palette indices resolved or not.
fn palette_settings(resolve_palette_indices: bool, budget: Option<u64>) -> DecodeSettings {
    DecodeSettings {
        resolve_palette_indices,
        ..settings(budget)
    }
}

#[test]
fn a_palette_image_of_more_than_one_component_is_refused_with_indices_unresolved() {
    // With indices unresolved the output is one gray channel; a second
    // component used to be interleaved past the end of that buffer.
    for components in [2, 3, 4, 255, 256] {
        let palette = jpx::jp2_palette(components, 8);
        for budget in [None, Some(DEFAULT_BUDGET)] {
            assert_eq!(
                hayro_jpeg2000::Image::new(&palette, &settings(budget)).err(),
                Some(TOO_MANY_CHANNELS),
                "{components} components, budget {budget:?}"
            );
        }
    }
    // One component, the shape a palette image has, decodes.
    for budget in [None, Some(DEFAULT_BUDGET)] {
        assert_eq!(
            decode(&jpx::jp2_palette(1, 8), budget).map(|p| p.len()),
            Ok(64)
        );
    }
}

#[test]
fn more_than_255_components_are_refused_not_narrowed() {
    // The progression iterator keeps the component count in a `u8`: 256
    // narrowed to 0 (an assertion failed) and 257 to 1 (the rest were
    // silently dropped). Resolving the palette skips the unresolved-index
    // refusal above, so this is the path that reached the narrowing.
    for resolve in [true, false] {
        for components in [256, 257, 511, 512] {
            let palette = jpx::jp2_palette(components, 8);
            for budget in [None, Some(DEFAULT_BUDGET)] {
                assert_eq!(
                    hayro_jpeg2000::Image::new(&palette, &palette_settings(resolve, budget)).err(),
                    Some(TOO_MANY_CHANNELS),
                    "{components} components, resolve {resolve}, budget {budget:?}"
                );
            }
        }
    }
    // A raw codestream was already refused (no colour space has that many
    // channels), and still is.
    let raw = jpx::codestream(&Shape {
        components: 256,
        ..Shape::new(8, 8)
    });
    assert_eq!(decode(&raw, None).err(), Some(TOO_MANY_CHANNELS));
}

#[test]
fn the_test_kits_palette_images_decode_with_indices_resolved() {
    // internal #77: the test kit's palette used to hold two entries, and its images
    // (no packets: every index is the mid-gray 128) mapped no index, so
    // every one failed with `PaletteResolutionFailed`. Its palette is now
    // the identity over 256 entries: resolving the indices gives what
    // leaving them unresolved does, budgeted or not.
    for side in [8, 64] {
        let palette = jpx::jp2_palette(1, side);
        let unresolved = decode(&palette, None).expect("decodes unresolved");
        for budget in [None, Some(MIB)] {
            let resolved = hayro_jpeg2000::Image::new(&palette, &palette_settings(true, budget))
                .and_then(|image| image.decode());
            assert_eq!(
                resolved.as_ref(),
                Ok(&unresolved),
                "side {side}, budget {budget:?}"
            );
        }
    }
}

#[test]
fn a_palette_that_moves_the_index_resolves_to_its_entry() {
    // The identity above cannot tell resolved indices from unresolved ones.
    // Here entry 128 (every index: no packets) holds 205, so resolving must
    // give 205 everywhere while the unresolved index stays 128.
    let shifted = |i: u8| i.wrapping_add(77);
    for side in [8, 64] {
        let palette = jpx::jp2_palette_mapped(1, side, shifted);
        let pixels = (side * side) as usize;
        assert_eq!(
            decode(&palette, None),
            Ok(vec![128; pixels]),
            "side {side}, unresolved"
        );
        for budget in [None, Some(MIB)] {
            let resolved = hayro_jpeg2000::Image::new(&palette, &palette_settings(true, budget))
                .and_then(|image| image.decode());
            assert_eq!(
                resolved,
                Ok(vec![shifted(128); pixels]),
                "side {side}, budget {budget:?}"
            );
        }
    }
}

#[test]
fn the_refusal_is_reported_as_a_memory_budget_validation_error() {
    assert_eq!(
        REFUSED.to_string(),
        "the decode would exceed its memory budget"
    );
}

// ---------------------------------------------------------------------------
// Resident memory and requested heap of the refusal from the header.
//
// As in `h14_jbig2_budget.rs`: peak RSS in a child process running only
// `rss_child` (it is process-wide and never goes down), and the bytes
// requested from the heap, counted per thread by the allocator below, which
// also sees an allocation that was made and never touched.

/// Selects the case `rss_child` measures; unset, `rss_child` does nothing.
const RSS_CASE: &str = "H15_RSS_CASE";

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
    let (data, budget) = match case.as_str() {
        "tiles" => (jpx::patched_gray_64((4000, 4000), (1, 1), 1), 64 * MIB),
        "bomb" => (jpx::tile_bomb(), 16 * GIB),
        other => panic!("unknown case {other}"),
    };
    let before = peak_rss_bytes();
    assert_eq!(decode(&data, Some(budget)).err(), Some(REFUSED));
    println!(
        "\nH15_RSS_GROWTH={}",
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
        .find_map(|l| l.split_once("H15_RSS_GROWTH=").map(|(_, n)| n))
        .unwrap_or_else(|| panic!("no measurement in {stdout}"))
        .trim()
        .parse()
        .expect("a number")
}

#[test]
fn header_refusals_happen_before_the_memory_is_touched() {
    // 16 million tile records (2.7 GB) under 64 MiB, and the 3.6 billion of
    // the hostile fixture under the default cap's 16.4 GB: neither grows the
    // resident set.
    for case in ["tiles", "bomb"] {
        let growth = rss_growth(case);
        assert!(growth < 16 * MIB, "{case}: peak RSS grew by {growth} bytes");
    }
}

/// The system allocator plus per-thread accounting of live heap bytes and
/// their peak (as in `h14_jbig2_budget.rs`).
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
fn header_refusals_happen_before_the_allocation() {
    // Not one tile record is requested.
    let tiles = jpx::patched_gray_64((4000, 4000), (1, 1), 1);
    let peak = peak_heap_during(|| {
        assert_eq!(decode(&tiles, Some(64 * MIB)).err(), Some(REFUSED));
    });
    assert!(peak < MIB, "tiles: {peak} bytes requested");
}

#[test]
fn charged_bytes_are_the_bytes_allocated() {
    // The decoder grows its reused buffers itself, by exactly what it
    // charged, so what an unbudgeted decode of the 1 × 1-code-block image
    // holds at its peak is close to the smallest budget it decodes under
    // (the remainder is uncounted, input-linear: the codestream's segment
    // lists, the progression state).
    let need = min_budget(PRECINCT_1_GRAY_128);
    let peak = peak_heap_during(|| {
        decode(PRECINCT_1_GRAY_128, None).expect("decodes");
    });
    assert!(
        peak < need + need / 10 && need < peak + peak / 10,
        "charged {need}, peak heap {peak}"
    );
}

// ---------------------------------------------------------------------------
// hayro-syntax

/// The pixel cap of the hayro-syntax tests: a 64,000,000-byte JPEG 2000
/// budget.
const CAP: u64 = 1_000_000;

fn limits(max_image_pixels: Option<u64>) -> DecodeLimits {
    DecodeLimits {
        max_image_pixels,
        ..DecodeLimits::default()
    }
}

/// Decodes the image XObject of `jpx::image_pdf` (object 1) as a
/// `width × height` 8-bit gray image under `max_image_pixels`, at
/// `target` resolution if given.
fn decode_image(
    jpx_data: &[u8],
    width: u32,
    height: u32,
    max_image_pixels: Option<u64>,
    target: Option<(u32, u32)>,
) -> Result<Vec<u8>, DecodeFailure> {
    let pdf = Pdf::new_with_options(
        jpx::image_pdf(jpx_data, width, height),
        "",
        limits(max_image_pixels),
    )
    .expect("opens");
    let image = pdf
        .xref()
        .get::<Stream<'_>>(ObjectIdentifier::new(1, 0))
        .expect("image stream");
    let params = ImageDecodeParams {
        bpc: Some(8),
        num_components: Some(1),
        width,
        height,
        target_dimension: target,
        ..ImageDecodeParams::default()
    };
    image.decoded_image(&params).map(|r| r.data.into_owned())
}

/// Asserts `decode_image` fails with `ImageMemoryLimit`, counted once as a
/// memory-limit hit and never as a pixel-limit hit.
fn assert_memory_limit(result: impl FnOnce() -> Result<Vec<u8>, DecodeFailure>) {
    let (memory, pixels) = (image_memory_limit_hits(), image_pixel_limit_hits());
    assert!(matches!(result(), Err(DecodeFailure::ImageMemoryLimit)));
    assert_eq!(image_memory_limit_hits() - memory, 1);
    assert_eq!(image_pixel_limit_hits(), pixels);
}

#[test]
fn the_budget_is_64_bytes_per_pixel_of_the_cap() {
    assert_eq!(JPX_BUDGET_BYTES_PER_PIXEL, 64);
    // The 128 × 128 image with 1 × 1 code-blocks needs about 4 MB: the
    // smallest cap that holds it is a 64th of that (rounded up), far above
    // its 16,384 pixels, and one pixel less is refused.
    let need = min_budget(PRECINCT_1_GRAY_128);
    let cap = need.div_ceil(JPX_BUDGET_BYTES_PER_PIXEL);
    assert!(cap > 128 * 128);
    let capped = decode_image(PRECINCT_1_GRAY_128, 128, 128, Some(cap), None).expect("decodes");
    let uncapped = decode_image(PRECINCT_1_GRAY_128, 128, 128, None, None).expect("decodes");
    assert_eq!(capped, uncapped);
    assert_memory_limit(|| decode_image(PRECINCT_1_GRAY_128, 128, 128, Some(cap - 1), None));
}

#[test]
fn both_images_hayro_syntax_builds_carry_the_budget() {
    // The 256 × 256 test-kit shape with 1 × 1 code-blocks (about 16 MB) is
    // refused at a cap of 100,000 pixels (6.4 MB) and decodes at 10,000,000
    // (640 MB). With a target resolution the decoding image is the second
    // one built, and the refusal is the decode's (the header is cheap), so
    // that image carries the budget as well.
    let shape = jpx::codestream(&precinct_1(256));
    for target in [None, Some((64, 64))] {
        assert_memory_limit(|| decode_image(&shape, 256, 256, Some(100_000), target));
        let decoded = decode_image(&shape, 256, 256, Some(10_000_000), target).expect("decodes");
        let unbudgeted = decode_image(&shape, 256, 256, None, target).expect("decodes");
        assert_eq!(decoded, unbudgeted);
    }
}

#[test]
fn a_tile_bomb_is_an_image_memory_limit() {
    let side = jpx::TILE_BOMB_GRID.div_ceil(255);
    assert_memory_limit(|| decode_image(&jpx::tile_bomb(), side, side, Some(CAP), None));
    // The default cap's 16.4 GB budget refuses it too.
    assert_memory_limit(|| decode_image(&jpx::tile_bomb(), side, side, Some(256_000_000), None));
}

#[test]
fn the_pixel_cap_still_fires_first() {
    // 20,000 × 20,000 in 64 × 64 tiles: 98,000 tile records fit the budget,
    // so the header passes and H13 refuses the size.
    let big = jpx::patched_gray_64((20_000, 20_000), (64, 64), 1);
    let (memory, pixels) = (image_memory_limit_hits(), image_pixel_limit_hits());
    assert!(matches!(
        decode_image(&big, 64, 64, Some(CAP), None),
        Err(DecodeFailure::ImagePixelLimit)
    ));
    assert_eq!(image_pixel_limit_hits() - pixels, 1);
    assert_eq!(image_memory_limit_hits(), memory);
}

#[test]
fn a_malformed_stream_is_still_a_decode_failure() {
    // A zero-width reference grid: `Image::new` refuses it as invalid.
    let broken = jpx::patched_gray_64((0, 64), (64, 64), 1);
    let (memory, pixels) = (image_memory_limit_hits(), image_pixel_limit_hits());
    assert!(matches!(
        decode_image(&broken, 64, 64, Some(CAP), None),
        Err(DecodeFailure::ImageDecode)
    ));
    assert_eq!(image_memory_limit_hits(), memory);
    assert_eq!(image_pixel_limit_hits(), pixels);
}

#[test]
fn a_component_count_the_output_cannot_hold_is_a_decode_failure() {
    // internal #63: refused by the codec as `TooManyChannels`, a broken image, never
    // a memory-limit hit, at the test cap and at the default.
    for components in [2, 256, 257] {
        let palette = jpx::jp2_palette(components, 8);
        for cap in [Some(CAP), Some(256_000_000), None] {
            let (memory, pixels) = (image_memory_limit_hits(), image_pixel_limit_hits());
            assert!(matches!(
                decode_image(&palette, 8, 8, cap, None),
                Err(DecodeFailure::ImageDecode)
            ));
            assert_eq!(image_memory_limit_hits(), memory);
            assert_eq!(image_pixel_limit_hits(), pixels);
        }
    }
}

#[test]
fn an_image_within_the_budget_decodes_as_before() {
    let gray = jpx::gray_64();
    let capped = decode_image(&gray, 64, 64, Some(CAP), None).expect("decodes");
    let uncapped = decode_image(&gray, 64, 64, None, None).expect("decodes");
    assert_eq!(capped, uncapped);
    assert_eq!(capped, decode(&gray, None).expect("decodes"));
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
    let (warnings, decoded) = render(jpx::tile_bomb_pdf(), limits(Some(CAP)));
    assert_eq!(warnings, ["ImageMemoryLimit"]);
    assert_eq!(decoded, 0);
    assert_eq!(image_memory_limit_hits() - memory, 1);
    assert_eq!(image_pixel_limit_hits(), pixels);
}

#[test]
fn an_xobject_within_the_budget_still_draws() {
    let (warnings, decoded) = render(jpx::image_pdf(&jpx::gray_64(), 64, 64), limits(Some(CAP)));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(decoded, 1);
}

// JPXDecode, like JBIG2Decode, has no inline-image abbreviation (ISO
// 32000-2 Table 92), so there is no inline counterpart of these tests.

// ---------------------------------------------------------------------------
// core plumbing

/// `Limits::max_image_pixels` sets the budget: a document opened through
/// core refuses the tile bomb through the rendering hook's xref.
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
    let doc = Document::from_bytes(jpx::tile_bomb_pdf(), &opts).expect("opens");
    let side = jpx::TILE_BOMB_GRID.div_ceil(255);
    let result = doc
        .page(0)
        .expect("page 0")
        .with_hayro_page(|input| {
            let stream = input
                .xref
                .get::<Stream<'_>>(ObjectIdentifier::new(1, 0))
                .expect("image stream");
            let params = ImageDecodeParams {
                bpc: Some(8),
                num_components: Some(1),
                width: side,
                height: side,
                ..ImageDecodeParams::default()
            };
            stream.decoded_image(&params).map(|_| ())
        })
        .expect("open document");
    assert!(matches!(result, Err(DecodeFailure::ImageMemoryLimit)));
}
