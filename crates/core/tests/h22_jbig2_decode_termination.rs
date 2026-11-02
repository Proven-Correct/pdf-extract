//! H22 and H23: JBIG2 decode loops that ran on without bound once the data
//! was used up, or without bound on CPU (internal #116).
//!
//! H22: a symbol dictionary's height-class loop advances only when a class
//! adds a symbol. Once the arithmetic decoder has run out of data it is a
//! finite state machine reading nothing, and some streams then cycle through
//! empty height classes (IADH, then IADW = OOB) forever, as the export flags
//! of internal #94 (H17) did. More than 64 empty classes in a row after the data is
//! used up are now refused with `SymbolError::HeightClassStalled`.
//!
//! H23: a text region places up to 10,000 symbol instances, and so does each
//! aggregate symbol of a dictionary; each costs its symbol's area to draw,
//! so a few hundred bytes could draw a large symbol over and over. Every
//! instance is now charged its symbol's `width × height` plus 64 against
//! `DecoderContext::work_budget`, which hayro-syntax sets to
//! `JBIG2_WORK_PIXELS_PER_PIXEL × max_image_pixels` and whose refusal it
//! reports as `DecodeFailure::ImageMemoryLimit`, like the memory budget's.
//!
//! The streams come from `pdf_extract_testkit::jbig2`.

use std::sync::mpsc;
use std::time::Duration;

use hayro_jbig2::{DecodeError, Decoder, DecoderContext, OverflowError, SymbolError};
use hayro_syntax::object::stream::{DecodeFailure, ImageDecodeParams};
use hayro_syntax::object::{ObjectIdentifier, Stream};
use hayro_syntax::{
    DecodeLimits, JBIG2_WORK_PIXELS_PER_PIXEL, Pdf, image_memory_limit_hits, image_pixel_limit_hits,
};
use pdf_extract_testkit::jbig2;

const MIB: u64 = 1 << 20;

const WORK_REFUSED: DecodeError = DecodeError::Overflow(OverflowError::WorkBudget);

/// What each placed instance costs on top of its symbol's area.
const PER_INSTANCE: u64 = 64;

/// What the aggregate symbol of `jbig2::repeated_symbol` costs to draw: the
/// one-pixel base symbol placed twice.
const AGGREGATE: u64 = 2 * (1 + PER_INSTANCE);

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

/// Decodes an embedded JBIG2 stream through `decode_with` under a memory
/// `budget` and a `work` allowance.
fn decode(stream: &[u8], budget: Option<u64>, work: Option<u64>) -> Result<Pixels, DecodeError> {
    let image = hayro_jbig2::Image::new_embedded(stream, None)?;
    let mut ctx = DecoderContext::default();
    ctx.memory_budget = budget;
    ctx.work_budget = work;
    let mut pixels = Pixels::default();
    image.decode_with(&mut pixels, &mut ctx)?;
    Ok(pixels)
}

/// `decode` (no work allowance) on its own thread, giving up after `limit`:
/// a decode that spins fails the test instead of hanging it (the spinning
/// thread is left behind and dies with the test process).
fn decode_within(
    stream: Vec<u8>,
    budget: Option<u64>,
    limit: Duration,
) -> Option<Result<Pixels, DecodeError>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(decode(&stream, budget, None));
    });
    rx.recv_timeout(limit).ok()
}

/// The smallest work allowance `stream` decodes under (binary search; the
/// stream must decode under `u32::MAX` pixels of work).
fn min_work(stream: &[u8]) -> u64 {
    let (mut low, mut high) = (0, u64::from(u32::MAX));
    while low < high {
        let mid = low + (high - low) / 2;
        if decode(stream, None, Some(mid)).is_ok() {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}

// ---------------------------------------------------------------------------
// H22: the height-class loop

#[test]
fn height_classes_that_stop_adding_symbols_after_the_data_are_refused() {
    // internal #116: three new symbols declared, and once its two bytes are used up
    // this stream decodes to height classes that hold no symbol forever, so
    // the loop never reached its third symbol and the decode spun at full
    // CPU, charging nothing to any budget. A decode takes microseconds; the
    // limit only has to tell an answer from none.
    let stalled = jbig2::arithmetic_new_symbols_dictionary(3, &[0x06, 0xC4]);
    for budget in [None, Some(MIB)] {
        let result = decode_within(stalled.clone(), budget, Duration::from_secs(20))
            .expect("the decode returns instead of spinning");
        assert_eq!(
            result.err(),
            Some(DecodeError::Symbol(SymbolError::HeightClassStalled))
        );
    }
}

#[test]
fn a_short_stream_whose_padding_decodes_empty_height_classes_still_decodes() {
    // Control: this two-byte stream decodes three empty height classes after
    // its data, then its one symbol and its export flags, all from the
    // padding. Refusing at the first empty class, or at two in a row as the
    // H17 export guard does, would refuse it.
    let short = jbig2::arithmetic_new_symbols_dictionary(1, &[0xA0, 0xB6]);
    let budgeted = decode(&short, Some(MIB), None).expect("decodes");
    let unbudgeted = decode(&short, None, None).expect("decodes");
    assert_eq!(budgeted, unbudgeted);
    assert_eq!(budgeted.rows, 64);
    assert_eq!(budgeted.black(), 0, "the dictionary is never drawn");
}

#[test]
fn the_height_class_refusal_has_its_own_message() {
    assert_eq!(
        DecodeError::Symbol(SymbolError::HeightClassStalled).to_string(),
        "height classes stopped adding symbols"
    );
}

// ---------------------------------------------------------------------------
// H23: the work allowance, in hayro-jbig2

#[test]
fn each_instance_costs_its_symbols_area_plus_a_fixed_charge() {
    // A 5 × 3 symbol placed 7 times: 7 × (15 + 64), plus the aggregate that
    // draws the symbol. One pixel less is refused.
    let stream = jbig2::repeated_symbol(5, 3, 7, 64);
    let needed = AGGREGATE + 7 * (5 * 3 + PER_INSTANCE);
    assert_eq!(min_work(&stream), needed);
    assert_eq!(
        decode(&stream, None, Some(needed - 1)).err(),
        Some(WORK_REFUSED)
    );
}

#[test]
fn the_instances_of_an_aggregate_symbol_are_charged() {
    // A dictionary whose one new symbol is drawn by placing the base symbol
    // twice (6.5.8.2), never drawn on the page itself.
    let stream = jbig2::symbol_export(16, 1, 64, false);
    assert_eq!(min_work(&stream), AGGREGATE);
    assert_eq!(
        decode(&stream, None, Some(AGGREGATE - 1)).err(),
        Some(WORK_REFUSED)
    );
}

#[test]
fn an_empty_symbol_still_costs_the_fixed_charge() {
    // A symbol 0 pixels wide draws nothing, but decoding each placement still
    // costs time, so 10,000 of them are not free.
    let stream = jbig2::repeated_symbol(0, 3, 10_000, 64);
    assert_eq!(min_work(&stream), AGGREGATE + 10_000 * PER_INSTANCE);
}

#[test]
fn a_large_symbol_placed_10000_times_is_refused_early() {
    // A 1024 × 1024 symbol placed 10,000 times costs over 10^10 pixels of
    // drawing from a stream of a few kilobytes, all within the memory budget.
    // Under an allowance of 16 such symbols it is refused at the 16th.
    let stream = jbig2::repeated_symbol(1024, 1024, 10_000, 1024);
    assert!(stream.len() < 20_000);
    let allowance = 16 * 1024 * 1024;
    assert_eq!(
        decode(&stream, None, Some(allowance)).err(),
        Some(WORK_REFUSED)
    );
}

#[test]
fn the_memory_budget_and_the_work_allowance_are_separate() {
    // The same decode under a work allowance that holds it and a memory
    // budget that holds it, each without the other: neither counts against
    // the other.
    let stream = jbig2::repeated_symbol(32, 32, 100, 64);
    let work = AGGREGATE + 100 * (32 * 32 + PER_INSTANCE);
    let unbudgeted = decode(&stream, None, None).expect("decodes");
    assert_eq!(
        decode(&stream, Some(MIB), None).expect("decodes"),
        unbudgeted
    );
    assert_eq!(
        decode(&stream, None, Some(work)).expect("decodes"),
        unbudgeted
    );
    assert_eq!(
        decode(&stream, Some(MIB), Some(work)).expect("decodes"),
        unbudgeted
    );
}

#[test]
fn decoding_without_a_context_has_no_work_allowance() {
    // `Image::decode` and a default context have no allowance: the decode
    // that the allowance refuses above runs to the end without one.
    assert_eq!(DecoderContext::default().work_budget, None);
    let stream = jbig2::repeated_symbol(64, 64, 10_000, 64);
    let image = hayro_jbig2::Image::new_embedded(&stream, None).expect("parses");
    let mut pixels = Pixels::default();
    image.decode(&mut pixels).expect("decodes");
    assert_eq!(pixels, decode(&stream, None, None).expect("decodes"));
}

#[test]
fn the_work_refusal_has_its_own_message() {
    assert_eq!(
        WORK_REFUSED.to_string(),
        "the decode would exceed its work budget"
    );
}

// ---------------------------------------------------------------------------
// H23: the allowance as hayro-syntax sets it

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
        DecodeLimits {
            max_image_pixels,
            ..DecodeLimits::default()
        },
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

/// The pixel cap of these tests: a 64 × 64 image, so an allowance of
/// 65,536 pixels and a memory budget of 8,192 bytes.
const CAP: u64 = 64 * 64;

#[test]
fn a_region_over_the_work_allowance_is_an_image_memory_limit() {
    // A 64 × 64 symbol placed 10,000 times on a 64 × 64 image: 41.6 million
    // pixels of work, within the memory budget, refused like it.
    let stream = jbig2::repeated_symbol(64, 64, 10_000, 64);
    let (memory, pixels) = (image_memory_limit_hits(), image_pixel_limit_hits());
    assert!(matches!(
        decode_image(&stream, 64, 64, Some(CAP)),
        Err(DecodeFailure::ImageMemoryLimit)
    ));
    assert_eq!(image_memory_limit_hits() - memory, 1);
    assert_eq!(image_pixel_limit_hits(), pixels);
}

#[test]
fn the_work_allowance_is_sixteen_pixels_per_pixel_of_the_cap() {
    assert_eq!(JBIG2_WORK_PIXELS_PER_PIXEL, 16);
    // 15 placements of the 64 × 64 symbol (62,530 pixels with the aggregate)
    // fit in 16 × 4096 = 65,536, and 16 (66,690) do not.
    let fits = jbig2::repeated_symbol(64, 64, 15, 64);
    let capped = decode_image(&fits, 64, 64, Some(CAP)).expect("decodes");
    assert_eq!(
        capped,
        decode_image(&fits, 64, 64, None).expect("decodes"),
        "the allowance changed the pixels"
    );
    let over = jbig2::repeated_symbol(64, 64, 16, 64);
    assert!(matches!(
        decode_image(&over, 64, 64, Some(CAP)),
        Err(DecodeFailure::ImageMemoryLimit)
    ));
    // Raising the cap admits it.
    assert!(decode_image(&over, 64, 64, Some(2 * CAP)).is_ok());
}

#[test]
fn a_text_region_within_the_allowance_draws_as_before() {
    // Control: an 8 × 8 symbol placed 50 times, a row apart, down the image
    // (6,530 pixels of work), and a real arithmetic-coded symbol drawn once,
    // decode to the same bytes with the cap as without.
    let streams = [
        (jbig2::repeated_symbol(8, 8, 50, 64), 50),
        (
            jbig2::arithmetic_symbols(&[&["##.", ".##", "#.#"]], 0, (3, 5), (64, 64)),
            6,
        ),
    ];
    for (stream, black) in streams {
        let capped = decode_image(&stream, 64, 64, Some(CAP)).expect("decodes");
        let uncapped = decode_image(&stream, 64, 64, None).expect("decodes");
        assert_eq!(capped, uncapped);
        assert_eq!(capped.iter().filter(|&&b| b == 0).count(), black);
    }
}
