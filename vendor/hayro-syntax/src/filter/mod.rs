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
    /// The default, `None`, means no limit.
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
