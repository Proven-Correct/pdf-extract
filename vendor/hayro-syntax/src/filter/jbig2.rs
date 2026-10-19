use crate::bit_reader::BitWriter;
use crate::object::Dict;
use crate::object::Stream;
use crate::object::dict::keys::JBIG2_GLOBALS;
use crate::filter::{
    JBIG2_BUDGET_BYTES_PER_PIXEL, JBIG2_WORK_PIXELS_PER_PIXEL, check_image_pixels, guard_codec,
    image_memory_budget, image_memory_limit_hit,
};
use crate::object::stream::{
    DecodeFailure, FilterResult, ImageColorSpace, ImageData, ImageDecodeParams,
};
use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;
use core::iter;

/// Decode JBIG2 data from a PDF stream.
///
/// The `params` dictionary may contain a `JBIG2Globals` entry pointing to
/// a stream with shared symbol dictionaries.
pub(crate) fn decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
    max_pixels: Option<u64>,
) -> Result<FilterResult<'static>, DecodeFailure> {
    let globals = params
        .get::<Stream<'_>>(JBIG2_GLOBALS)
        .and_then(|g| g.decoded().ok());
    super::fire_armed_panic(super::ArmedPanic::OutsideGuard);

    // H18: the globals above are hayro-syntax's (xref and Flate) and stay
    // outside the codec guard; only hayro-jbig2's own calls are inside it.
    let image = guard_codec(|| {
        hayro_jbig2::Image::new_embedded(data, globals.as_deref())
            .map_err(|_| DecodeFailure::ImageDecode)
    })?;

    // H13: `new_embedded` read only the segment headers; the page size (an
    // unknown height already resolved from the end-of-stripe segments) is
    // what `decode` and the buffers below allocate for.
    check_image_pixels(
        u64::from(image.width()),
        u64::from(image.height()),
        max_pixels,
    )?;

    // Whenever possible (if we don't have an indexed color space), we convert
    // the data as 8-bit instead of 1-bit, so that it can be easier converted
    // into an RGBA8 image.

    // We need to invert the color because JBIG2 uses black = 1 and
    // white = 0, but PDF uses the opposite.
    let (decoded, bpc) = if image_params.is_indexed {
        let row_bytes = (image.width() as usize).div_ceil(8);
        let mut packed = vec![0_u8; row_bytes * image.height() as usize];

        struct BitWriterDecoder<'a> {
            writer: BitWriter<'a>,
        }

        impl hayro_jbig2::Decoder for BitWriterDecoder<'_> {
            fn push_pixel(&mut self, black: bool) {
                let _ = self.writer.write(u32::from(!black));
            }

            fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
                let byte_value = if black { 0x00 } else { 0xFF };
                let _ = self.writer.fill_bytes(byte_value, chunk_count as usize);
            }

            fn next_line(&mut self) {
                // Images need to be padded to the byte boundary after each row.
                self.writer.align();
            }
        }

        let writer = BitWriter::new(&mut packed, 1).ok_or(DecodeFailure::ImageDecode)?;
        let mut decoder = BitWriterDecoder { writer };
        decode_within_budget(&image, &mut decoder, max_pixels)?;

        (packed, 1)
    } else {
        struct Luma8Decoder {
            output: Vec<u8>,
        }

        impl hayro_jbig2::Decoder for Luma8Decoder {
            fn push_pixel(&mut self, black: bool) {
                self.output.push(if black { 0x00 } else { 0xFF });
            }

            fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
                let byte = if black { 0x00 } else { 0xFF };
                self.output
                    .extend(iter::repeat_n(byte, chunk_count as usize * 8));
            }

            fn next_line(&mut self) {}
        }

        let mut decoder = Luma8Decoder { output: Vec::new() };
        decode_within_budget(&image, &mut decoder, max_pixels)?;

        (decoder.output, 8)
    };

    Ok(FilterResult {
        data: Cow::Owned(decoded),
        image_data: Some(ImageData {
            alpha: None,
            color_space: Some(ImageColorSpace::Gray),
            bits_per_component: bpc,
            width: image.width(),
            height: image.height(),
        }),
    })
}

/// H14: decode under the memory budget the pixel cap implies. The page size
/// was checked against the cap above, but the decoder's working set (symbol
/// dictionaries and their exported copies, intermediate regions, gray-scale
/// planes) is not bounded by the page size; the budget bounds its total. A
/// refusal by the budget is its own failure, counted once; every other codec
/// error is a broken image.
///
/// H23: the cap also sets the work allowance for placing symbol instances
/// (#116), whose refusal goes the same way: both are the decoder outgrowing
/// what the cap allows, and raising the cap admits both.
fn decode_within_budget<D: hayro_jbig2::Decoder>(
    image: &hayro_jbig2::Image<'_>,
    decoder: &mut D,
    max_pixels: Option<u64>,
) -> Result<(), DecodeFailure> {
    let mut ctx = hayro_jbig2::DecoderContext::default();
    ctx.memory_budget = image_memory_budget(max_pixels, JBIG2_BUDGET_BYTES_PER_PIXEL);
    ctx.work_budget = image_memory_budget(max_pixels, JBIG2_WORK_PIXELS_PER_PIXEL);

    guard_codec(|| {
        image.decode_with(decoder, &mut ctx).map_err(|e| match e {
            hayro_jbig2::DecodeError::Overflow(
                hayro_jbig2::OverflowError::MemoryBudget | hayro_jbig2::OverflowError::WorkBudget,
            ) => image_memory_limit_hit(),
            _ => DecodeFailure::ImageDecode,
        })
    })
}
