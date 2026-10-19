//! Results from decoding filtered data streams.

mod ascii_85;
pub(crate) mod ascii_hex;
#[cfg(feature = "images")]
mod ccitt;
#[cfg(feature = "images")]
mod dct;
#[cfg(feature = "images")]
mod jbig2;
#[cfg(feature = "images")]
mod jpx;
mod lzw_flate;
mod run_length;

use crate::object::Dict;
use crate::object::Name;
use crate::object::dict::keys::*;
use crate::object::stream::{DecodeFailure, FilterResult, ImageDecodeParams};
use alloc::vec::Vec;
use core::ops::Deref;
use core::sync::atomic::{AtomicU64, Ordering};

/// Limits on the output of stream filters, set per document with
/// [`Pdf::new_with_options`](crate::Pdf::new_with_options).
///
/// A compressed stream can expand by a factor of a thousand or more, so a
/// small file can demand gigabytes when it is decoded. The limit applies to
/// each filter of a stream's filter chain separately, and covers the flate,
/// LZW, run-length, ASCII hex and ASCII base-85 filters (image codecs are not
/// limited). A stream that would exceed it fails to decode with
/// [`DecodeFailure::LimitExceeded`], which hayro treats like any other stream
/// it cannot decode, and is counted in [`DecodeStats`].
///
/// Streams read without a document, such as objects parsed with
/// [`FromBytes`](crate::object::FromBytes) or inline images in a content
/// stream, are not limited, with one exception: an inline image decoded with
/// [`Stream::decoded_inline_image`](crate::object::Stream::decoded_inline_image)
/// is held to its document's `max_image_pixels`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DecodeLimits {
    /// The maximum number of bytes one filter may produce for one stream.
    /// The default, `u64::MAX`, means no limit.
    pub max_stream_decoded_bytes: u64,
    /// The largest image, by the `width × height` its decode parameters
    /// declare, that may be decoded at all. A larger one fails with
    /// [`DecodeFailure::ImagePixelLimit`] before any filter runs, so the cap
    /// bounds what the decoders allocate, not only what they return. Streams
    /// decoded without image parameters declare 0 × 0 and are never refused.
    ///
    /// The cap also sets the memory budgets of the JBIG2 and JPEG 2000
    /// decoders, [`JBIG2_BUDGET_BYTES_PER_PIXEL`] and
    /// [`JPX_BUDGET_BYTES_PER_PIXEL`] bytes per pixel of the cap, whose working
    /// sets (symbol and region bitmaps, gray-scale planes; tile, precinct and
    /// code-block records, channel planes) are not bounded by the image size;
    /// a decode that would exceed its budget fails with
    /// [`DecodeFailure::ImageMemoryLimit`]. It sets the JBIG2 decoder's work
    /// allowance for placing symbol instances too,
    /// [`JBIG2_WORK_PIXELS_PER_PIXEL`] pixels per pixel of the cap, refused
    /// the same way.
    ///
    /// The default, `None`, means no limit and no budget.
    pub max_image_pixels: Option<u64>,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_stream_decoded_bytes: u64::MAX,
            max_image_pixels: None,
        }
    }
}

/// What has happened while decoding a document's streams so far; see
/// [`Pdf::decode_stats`](crate::Pdf::decode_stats).
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct DecodeStats {
    /// How many stream decodes failed because a filter would have produced
    /// more than [`DecodeLimits::max_stream_decoded_bytes`]. hayro does not
    /// cache decoded streams, so a stream decoded twice counts twice.
    pub streams_over_limit: u64,
}

/// A document's decode limits and the counters behind its [`DecodeStats`].
/// Shared by everything that decodes the document's streams, including the
/// xref streams read before the document's `XRef` exists.
#[derive(Debug)]
pub(crate) struct DecodeBudget {
    pub(crate) limits: DecodeLimits,
    streams_over_limit: AtomicU64,
}

impl DecodeBudget {
    pub(crate) fn new(limits: DecodeLimits) -> Self {
        Self {
            limits,
            streams_over_limit: AtomicU64::new(0),
        }
    }

    pub(crate) fn note_over_limit(&self) {
        self.streams_over_limit.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn stats(&self) -> DecodeStats {
        DecodeStats {
            streams_over_limit: self.streams_over_limit.load(Ordering::Relaxed),
        }
    }
}

#[cfg(feature = "std")]
std::thread_local! {
    // Per thread rather than per document: [`DecodeStats`] belong to the document, so
    // two threads working on different pages of one document cannot tell their own
    // events apart in them. A caller samples this before and after an operation on the
    // same thread instead.
    static STREAM_DECODE_LIMIT_HITS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Return how many stream decodes on the current thread failed with
/// [`DecodeFailure::LimitExceeded`](crate::object::stream::DecodeFailure::LimitExceeded).
///
/// The same events are counted per document in [`DecodeStats`]; this counter tells
/// them apart by thread, so that an operation can learn about its own decodes while
/// other threads decode streams of the same document. It only ever increases: read it
/// before and after the operation, on the same thread. Without the `std` feature
/// nothing is counted and this always returns 0.
pub fn stream_decode_limit_hits() -> u64 {
    #[cfg(feature = "std")]
    {
        STREAM_DECODE_LIMIT_HITS.with(core::cell::Cell::get)
    }

    #[cfg(not(feature = "std"))]
    {
        0
    }
}

pub(crate) fn note_stream_decode_limit_hit() {
    #[cfg(feature = "std")]
    STREAM_DECODE_LIMIT_HITS.with(|n| n.set(n.get().saturating_add(1)));
}

#[cfg(feature = "std")]
std::thread_local! {
    // Per thread for the same reason as `STREAM_DECODE_LIMIT_HITS`.
    static IMAGE_PIXEL_LIMIT_HITS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Return how many image decodes on the current thread were refused with
/// [`DecodeFailure::ImagePixelLimit`](crate::object::stream::DecodeFailure::ImagePixelLimit)
/// because the image declared more pixels than [`DecodeLimits::max_image_pixels`].
///
/// It only ever increases: read it before and after an operation, on the same thread.
/// Without the `std` feature nothing is counted and this always returns 0.
pub fn image_pixel_limit_hits() -> u64 {
    #[cfg(feature = "std")]
    {
        IMAGE_PIXEL_LIMIT_HITS.with(core::cell::Cell::get)
    }

    #[cfg(not(feature = "std"))]
    {
        0
    }
}

pub(crate) fn note_image_pixel_limit_hit() {
    #[cfg(feature = "std")]
    IMAGE_PIXEL_LIMIT_HITS.with(|n| n.set(n.get().saturating_add(1)));
}

/// The JBIG2 decoder's memory budget, in bytes per pixel of
/// [`DecodeLimits::max_image_pixels`]. A text page of `P` pixels needs about
/// half a byte per pixel in total (its page bitmap, regions tiling the page,
/// a symbol dictionary of a few megabytes), so 2 leaves a fourfold margin at
/// the cap and far more for ordinary scans; the hostile shapes (a symbol
/// exported thousands of times, a gray-scale plane of 16 GiB from eight header
/// bytes) cost hundreds of bytes per pixel or are unbounded.
pub const JBIG2_BUDGET_BYTES_PER_PIXEL: u64 = 2;

/// The JBIG2 decoder's work allowance for placing symbol instances, in
/// pixels per pixel of [`DecodeLimits::max_image_pixels`]: each instance a
/// text region or an aggregate symbol places costs its symbol's area plus 64.
/// A text page draws each glyph once, so its instances cover the page about
/// once, and at most a few times counting the per-instance charge for small
/// glyphs and overlapping glyph boxes; 16 is as many pixels as the memory
/// budget can allocate as bitmaps (2 bytes, 16 one-bit pixels, per pixel of
/// the cap), so a decode may draw each pixel it could hold once, a fourfold
/// margin at the cap and far more for ordinary scans. The hostile shape (a
/// large symbol placed 10,000 times per region, region after region) is
/// unbounded without it; with it, the placing a decode can do at the default
/// cap takes a few seconds (measured at about 0.45 ns per charged pixel for
/// the costliest shape, one-pixel-wide symbols). A refusal is reported like
/// the memory budget's.
pub const JBIG2_WORK_PIXELS_PER_PIXEL: u64 = 16;

/// The JPEG 2000 decoder's memory budget, in bytes per pixel of
/// [`DecodeLimits::max_image_pixels`]: about 9 bytes per pixel per component
/// in channel planes, coefficients and output, so 64 covers a single-tile
/// RGBA image at the cap with room to spare; the hostile shapes (1 × 1 tiles
/// over a subsampled reference grid, 1 × 1 code-blocks, thousands of
/// components) cost hundreds of bytes per pixel or are unbounded.
pub const JPX_BUDGET_BYTES_PER_PIXEL: u64 = 64;

#[cfg(feature = "std")]
std::thread_local! {
    // Per thread for the same reason as `STREAM_DECODE_LIMIT_HITS`.
    static IMAGE_MEMORY_LIMIT_HITS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Return how many image decodes on the current thread were refused with
/// [`DecodeFailure::ImageMemoryLimit`](crate::object::stream::DecodeFailure::ImageMemoryLimit)
/// because the codec would have exceeded its memory budget.
///
/// It only ever increases: read it before and after an operation, on the same thread.
/// Without the `std` feature nothing is counted and this always returns 0.
pub fn image_memory_limit_hits() -> u64 {
    #[cfg(feature = "std")]
    {
        IMAGE_MEMORY_LIMIT_HITS.with(core::cell::Cell::get)
    }

    #[cfg(not(feature = "std"))]
    {
        0
    }
}

/// Count one refusal by a codec's memory budget and return the failure that
/// reports it.
#[cfg(feature = "images")]
pub(crate) fn image_memory_limit_hit() -> DecodeFailure {
    #[cfg(feature = "std")]
    IMAGE_MEMORY_LIMIT_HITS.with(|n| n.set(n.get().saturating_add(1)));

    DecodeFailure::ImageMemoryLimit
}

/// The memory budget of a codec allowed `bytes_per_pixel` bytes for each
/// pixel of `max_pixels`; none when there is no pixel cap.
#[cfg(feature = "images")]
pub(crate) fn image_memory_budget(max_pixels: Option<u64>, bytes_per_pixel: u64) -> Option<u64> {
    max_pixels.map(|pixels| pixels.saturating_mul(bytes_per_pixel))
}

/// Refuse an image of `width × height` pixels when that exceeds `max_pixels`,
/// counting the refusal. Called wherever an image's size first becomes known
/// before the allocation it governs: the declared dimensions, each codec's
/// header, and the dimensions a codec reports. The product saturates, so a
/// size too large for a `u64` is refused rather than wrapping under the cap.
pub(crate) fn check_image_pixels(
    width: u64,
    height: u64,
    max_pixels: Option<u64>,
) -> Result<(), DecodeFailure> {
    match max_pixels {
        Some(max) if width.saturating_mul(height) > max => {
            note_image_pixel_limit_hit();
            Err(DecodeFailure::ImagePixelLimit)
        }
        _ => Ok(()),
    }
}

/// H18: run one call into an image codec crate (zune-jpeg, hayro-ccitt,
/// hayro-jbig2, hayro-jpeg2000), turning a panic inside it into
/// [`DecodeFailure::ImageDecode`], so a codec bug skips one image instead of
/// failing the page.
///
/// Only the codec call goes in here: resolving the image's parameters, the
/// `JBIG2Globals` stream and the rest of hayro-syntax stay outside, so a
/// parser bug still panics. `AssertUnwindSafe` holds because `f` borrows only
/// the encoded data, decoder state local to the one decode, and copies of its
/// settings. The thread-local counters are bumped inside the guard only after
/// a codec call has returned, immediately before returning the error (the
/// JPEG frame check, the memory-budget mappings), and a decoder left
/// half-written by a panic is dropped unread. Our own code that does run
/// inside (the pixel sinks the CCITT and JBIG2 codecs call back into, the
/// JPEG frame check and YCCK fix-up) touches only that local state.
///
/// Fuzzing builds (`cfg(fuzzing)`, set by `cargo fuzz` for every crate) and
/// `no_std` builds compile the catch out, so there a codec panic propagates.
#[cfg(feature = "images")]
pub(crate) fn guard_codec<T>(
    f: impl FnOnce() -> Result<T, DecodeFailure>,
) -> Result<T, DecodeFailure> {
    #[cfg(all(feature = "std", not(fuzzing)))]
    {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fire_armed_panic(ArmedPanic::InsideGuard);
            f()
        }))
        .unwrap_or(Err(DecodeFailure::ImageDecode))
    }

    #[cfg(not(all(feature = "std", not(fuzzing))))]
    {
        fire_armed_panic(ArmedPanic::InsideGuard);
        f()
    }
}

/// Whether [`guard_codec`] catches codec panics in this build: false under
/// `cfg(fuzzing)`, without `std`, and without `images` (no codecs). Lets a test assert which build it runs in
/// without naming the `fuzzing` cfg itself.
#[doc(hidden)]
pub const CODEC_PANICS_CAUGHT: bool =
    cfg!(all(feature = "std", feature = "images", not(fuzzing)));

/// Where an armed test panic fires (feature `codec-panic-hook`).
#[cfg_attr(not(feature = "codec-panic-hook"), allow(dead_code))]
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum ArmedPanic {
    /// Inside [`guard_codec`], as if the codec itself had panicked.
    InsideGuard,
    /// In a codec module, after the image's parameters (and JBIG2's
    /// globals) are resolved and just before the first guarded call: a panic
    /// on the image path that the guard must not catch. A guard widened to
    /// the module's body, or to [`Filter::apply`], would catch it.
    OutsideGuard,
}

#[cfg(feature = "codec-panic-hook")]
std::thread_local! {
    // Per thread so that a test arming it cannot disturb decodes running on
    // other threads of the same process.
    static ARMED_PANIC: core::cell::Cell<Option<ArmedPanic>> = const { core::cell::Cell::new(None) };
}

/// Make the next guarded image-codec call on this thread panic inside the
/// codec guard, as a codec bug would (feature `codec-panic-hook`). The flag
/// clears when it fires, so exactly one decode fails. For tests of the codec
/// panic boundary: no known input panics a codec reliably, and fuzzing exists
/// to find and fix any that does.
#[cfg(feature = "codec-panic-hook")]
#[doc(hidden)]
pub fn arm_codec_panic_for_test() {
    ARMED_PANIC.with(|a| a.set(Some(ArmedPanic::InsideGuard)));
}

/// Make the next image-codec decode on this thread panic just before the codec
/// guard is entered, after the image's parameters are resolved: on the image
/// path but outside the codec call (feature `codec-panic-hook`). The flag clears when it fires. For tests that
/// the guard catches codec panics only.
#[cfg(feature = "codec-panic-hook")]
#[doc(hidden)]
pub fn arm_unguarded_image_panic_for_test() {
    ARMED_PANIC.with(|a| a.set(Some(ArmedPanic::OutsideGuard)));
}

/// Panic if a test armed a panic for `site`, clearing the flag first.
#[cfg(feature = "images")]
#[cfg_attr(not(feature = "codec-panic-hook"), allow(unused_variables))]
#[inline]
pub(crate) fn fire_armed_panic(site: ArmedPanic) {
    #[cfg(feature = "codec-panic-hook")]
    if ARMED_PANIC.with(|a| a.get()) == Some(site) {
        ARMED_PANIC.with(|a| a.set(None));
        match site {
            ArmedPanic::InsideGuard => panic!("codec panic injected for testing"),
            ArmedPanic::OutsideGuard => panic!("image-path panic injected for testing"),
        }
    }
}

/// A data filter.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Filter {
    /// ASCII hexadecimal encoding.
    AsciiHexDecode,
    /// ASCII base-85 encoding.
    Ascii85Decode,
    /// Lempel-Ziv-Welch (LZW) compression.
    LzwDecode,
    /// DEFLATE compression (zlib/gzip).
    FlateDecode,
    /// Run-length encoding compression.
    RunLengthDecode,
    /// CCITT Group 3 or Group 4 fax compression.
    CcittFaxDecode,
    /// JBIG2 compression for bi-level images.
    Jbig2Decode,
    /// JPEG (DCT) compression.
    DctDecode,
    /// JPEG 2000 compression.
    JpxDecode,
    /// Encryption filter.
    Crypt,
}

impl Filter {
    fn debug_name(&self) -> &'static str {
        match self {
            Self::AsciiHexDecode => "ascii_hex",
            Self::Ascii85Decode => "ascii_85",
            Self::LzwDecode => "lzw",
            Self::FlateDecode => "flate",
            Self::RunLengthDecode => "run-length",
            Self::CcittFaxDecode => "ccit_fax",
            Self::Jbig2Decode => "jbig2",
            Self::DctDecode => "dct",
            Self::JpxDecode => "jpx",
            Self::Crypt => "crypt",
        }
    }

    pub(crate) fn from_name(name: Name<'_>) -> Option<Self> {
        match name.deref() {
            ASCII_HEX_DECODE | ASCII_HEX_DECODE_ABBREVIATION => Some(Self::AsciiHexDecode),
            ASCII85_DECODE | ASCII85_DECODE_ABBREVIATION => Some(Self::Ascii85Decode),
            LZW_DECODE | LZW_DECODE_ABBREVIATION => Some(Self::LzwDecode),
            FLATE_DECODE | FLATE_DECODE_ABBREVIATION => Some(Self::FlateDecode),
            RUN_LENGTH_DECODE | RUN_LENGTH_DECODE_ABBREVIATION => Some(Self::RunLengthDecode),
            CCITTFAX_DECODE | CCITTFAX_DECODE_ABBREVIATION => Some(Self::CcittFaxDecode),
            JBIG2_DECODE => Some(Self::Jbig2Decode),
            DCT_DECODE | DCT_DECODE_ABBREVIATION => Some(Self::DctDecode),
            JPX_DECODE => Some(Self::JpxDecode),
            CRYPT => Some(Self::Crypt),
            _ => {
                warn!("unknown filter: {}", name.as_str());

                None
            }
        }
    }

    /// Apply the filter. `limit` is the maximum output length of the
    /// non-image filters (see [`DecodeLimits`]); `max_pixels` is the image
    /// pixel cap, which the image codecs check against the size their headers
    /// give before they allocate for it (CCITT, JBIG2 and JPEG 2000 size their
    /// output from the encoded data, not from the image dictionary).
    pub(crate) fn apply(
        &self,
        data: &[u8],
        params: &Dict<'_>,
        #[cfg_attr(not(feature = "images"), allow(unused))] image_params: &ImageDecodeParams,
        limit: u64,
        #[cfg_attr(not(feature = "images"), allow(unused))] max_pixels: Option<u64>,
    ) -> Result<FilterResult<'static>, DecodeFailure> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);

        let res = match self {
            // ASCII hex output is at most half its input, so the decoder needs
            // no early stop; `limited` still enforces the limit.
            Self::AsciiHexDecode => limited(ascii_hex::decode(data), limit),
            Self::Ascii85Decode => limited(ascii_85::decode(data, limit), limit),
            Self::RunLengthDecode => limited(run_length::decode(data, limit), limit),
            Self::LzwDecode => limited(lzw_flate::lzw::decode(data, params, limit), limit),
            Self::FlateDecode => limited(lzw_flate::flate::decode(data, params, limit), limit),
            #[cfg(feature = "images")]
            Self::DctDecode => dct::decode(data, params, image_params, max_pixels),
            #[cfg(feature = "images")]
            Self::CcittFaxDecode => ccitt::decode(data, params, image_params, max_pixels),
            #[cfg(feature = "images")]
            Self::Jbig2Decode => jbig2::decode(data, params, image_params, max_pixels),
            #[cfg(feature = "images")]
            Self::JpxDecode => jpx::decode(data, image_params, max_pixels),
            #[cfg(not(feature = "images"))]
            Self::DctDecode | Self::CcittFaxDecode | Self::Jbig2Decode | Self::JpxDecode => {
                warn!("image decoding is not supported (enable the `images` feature)");
                Err(DecodeFailure::ImageDecode)
            }
            _ => Err(DecodeFailure::StreamDecode),
        };

        if res.is_err() {
            warn!("failed to apply filter {}", self.debug_name());
        }

        res
    }
}

/// Turn a decoder's output into a filter result. Decoders stop as soon as
/// their output is longer than `limit`, so longer output means the limit was
/// hit, not that the data is broken.
fn limited(decoded: Option<Vec<u8>>, limit: usize) -> Result<FilterResult<'static>, DecodeFailure> {
    match decoded {
        Some(data) if data.len() > limit => {
            warn!("decoded stream exceeds the limit of {limit} bytes");

            Err(DecodeFailure::LimitExceeded)
        }
        Some(data) => Ok(FilterResult::from_data(data)),
        None => Err(DecodeFailure::StreamDecode),
    }
}
