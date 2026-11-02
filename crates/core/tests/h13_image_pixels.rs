//! H13: the image pixel cap in the vendored hayro fork (run 0002 spec §3.2,
//! oracle §4.4).
//!
//! `DecodeLimits::max_image_pixels` refuses an image whose *declared*
//! `width × height` is over the cap, before any filter runs, with
//! `DecodeFailure::ImagePixelLimit`, and counts the refusal in the
//! thread-local `image_pixel_limit_hits()`. In hayro-interpret the refusal is
//! the warning `InterpreterWarning::ImagePixelLimit`, never also an
//! `ImageDecodeFailure`; inline images, whose streams are parsed without their
//! document, are held to the same cap.
//!
//! The declared-size image is 64 × 64 gray pixels (4 KiB flate-compressed)
//! declaring 20000 × 20000: the declared product is what is checked, so the
//! tests never need the 400 MB the declaration implies. The codec fixtures
//! (CCITT, JBIG2, JPEG 2000, JPEG) declare 64 × 64 and carry a codec header
//! that says otherwise; the cap is checked on that header before the codec
//! allocates, and on the size the codec reports afterwards.

mod common;

use std::io::Write as _;
use std::sync::{Arc, Mutex};

use common::assemble_pdf;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterSettings, InterpreterWarning, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro_syntax::object::stream::{DecodeFailure, ImageDecodeParams};
use hayro_syntax::object::{ObjectIdentifier, Stream};
use hayro_syntax::{DecodeLimits, Pdf, image_pixel_limit_hits, stream_decode_limit_hits};
use kurbo::{Affine, BezPath, Rect};

const SIDE: u32 = 64;
const CAP: u64 = 1_000_000;

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).expect("compress");
    e.finish().expect("compress")
}

fn limits(max_image_pixels: Option<u64>) -> DecodeLimits {
    DecodeLimits {
        max_image_pixels,
        ..DecodeLimits::default()
    }
}

/// A one-page PDF whose content (object 4) draws the image XObject `/Im1`
/// (object 6) over the whole page. `image_dict` is spliced into the image's
/// dictionary (dimensions, colour, filter) and `data` is its body.
fn image_pdf(image_dict: &str, data: &[u8]) -> Vec<u8> {
    let content = b"q 200 0 0 200 0 0 cm /Im1 Do Q";
    let mut content_obj = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    content_obj.extend_from_slice(content);
    content_obj.extend_from_slice(b"\nendstream");
    let mut image = format!(
        "<< /Type /XObject /Subtype /Image {image_dict} /Length {} >>\nstream\n",
        data.len()
    )
    .into_bytes();
    image.extend_from_slice(data);
    image.extend_from_slice(b"\nendstream");
    assemble_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] \
           /Resources 5 0 R /Contents 4 0 R >>"
            .to_vec(),
        content_obj,
        b"<< /XObject << /Im1 6 0 R >> >>".to_vec(),
        image,
    ])
}

/// A one-page PDF whose content is `content` (an inline image).
fn content_pdf(content: &str) -> Vec<u8> {
    let content_obj = format!(
        "<< /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    );
    assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>".to_owned(),
        content_obj,
    ])
}

const GRAY: &str = "/ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode";
/// A body no JPEG decoder accepts, so the filter fails if it runs.
const BROKEN_JPEG: &str = "/ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode";

fn declared_huge() -> String {
    format!("/Width 20000 /Height 20000 {GRAY}")
}

fn pixels() -> Vec<u8> {
    (0..SIDE * SIDE).map(|i| (i % 251) as u8).collect()
}

fn open(bytes: Vec<u8>, decode: DecodeLimits) -> Pdf {
    Pdf::new_with_options(bytes, "", decode).expect("opens")
}

fn image(pdf: &Pdf) -> Stream<'_> {
    pdf.xref()
        .get::<Stream<'_>>(ObjectIdentifier::new(6, 0))
        .expect("image stream")
}

fn params(width: u32, height: u32) -> ImageDecodeParams {
    ImageDecodeParams {
        bpc: Some(8),
        num_components: Some(1),
        width,
        height,
        ..ImageDecodeParams::default()
    }
}

// ---------------------------------------------------------------------------
// hayro-syntax

#[test]
fn an_image_over_the_cap_is_refused_and_counted() {
    let pdf = open(
        image_pdf(&declared_huge(), &zlib(&pixels())),
        limits(Some(CAP)),
    );
    let before = image_pixel_limit_hits();
    let result = image(&pdf).decoded_image(&params(20_000, 20_000));
    assert!(matches!(result, Err(DecodeFailure::ImagePixelLimit)));
    assert_eq!(image_pixel_limit_hits() - before, 1);
}

#[test]
fn the_refusal_comes_before_any_filter_runs() {
    // Evidence that the filter never ran: (1) a body the JPEG decoder
    // rejects is still refused by the cap, not reported as a broken stream;
    // (2) a body whose flate output would exceed the byte limit is refused by
    // the cap, and the byte limit's counters do not move, so the filter
    // never produced (or attempted) any output.
    let broken = open(
        image_pdf(
            &format!("/Width 20000 /Height 20000 {BROKEN_JPEG}"),
            b"not a jpeg",
        ),
        limits(Some(CAP)),
    );
    assert!(matches!(
        image(&broken).decoded_image(&params(20_000, 20_000)),
        Err(DecodeFailure::ImagePixelLimit)
    ));
    // Control: under no cap the same body fails in the filter.
    let broken_uncapped = open(
        image_pdf(
            &format!("/Width 20000 /Height 20000 {BROKEN_JPEG}"),
            b"not a jpeg",
        ),
        limits(None),
    );
    assert!(matches!(
        image(&broken_uncapped).decoded_image(&params(20_000, 20_000)),
        Err(DecodeFailure::ImageDecode)
    ));

    let tight = DecodeLimits {
        max_stream_decoded_bytes: 16,
        max_image_pixels: Some(CAP),
    };
    let pdf = open(image_pdf(&declared_huge(), &zlib(&pixels())), tight);
    let decode_hits = stream_decode_limit_hits();
    assert!(matches!(
        image(&pdf).decoded_image(&params(20_000, 20_000)),
        Err(DecodeFailure::ImagePixelLimit)
    ));
    assert_eq!(stream_decode_limit_hits(), decode_hits);
    assert_eq!(pdf.decode_stats().streams_over_limit, 0);
}

#[test]
fn without_a_cap_the_same_stream_decodes() {
    let pdf = open(image_pdf(&declared_huge(), &zlib(&pixels())), limits(None));
    let before = image_pixel_limit_hits();
    let decoded = image(&pdf)
        .decoded_image(&params(20_000, 20_000))
        .unwrap_or_else(|_| panic!("decodes"));
    assert_eq!(decoded.data.len(), (SIDE * SIDE) as usize);
    assert_eq!(image_pixel_limit_hits(), before);
    // The default limits carry no cap.
    assert_eq!(DecodeLimits::default().max_image_pixels, None);
}

#[test]
fn the_cap_is_inclusive_and_checks_the_product() {
    let pdf = open(
        image_pdf(&format!("/Width 64 /Height 64 {GRAY}"), &zlib(&pixels())),
        limits(Some(u64::from(SIDE * SIDE))),
    );
    assert!(image(&pdf).decoded_image(&params(SIDE, SIDE)).is_ok());
    // One more pixel either way is over; a long thin image is judged by its
    // area, not its longest side.
    assert!(matches!(
        image(&pdf).decoded_image(&params(SIDE + 1, SIDE)),
        Err(DecodeFailure::ImagePixelLimit)
    ));
    assert!(image(&pdf).decoded_image(&params(SIDE * SIDE, 1)).is_ok());
    // The largest declarable image does not overflow the product.
    assert!(matches!(
        image(&pdf).decoded_image(&params(u32::MAX, u32::MAX)),
        Err(DecodeFailure::ImagePixelLimit)
    ));
}

#[test]
fn a_non_image_stream_is_never_refused() {
    // `decoded()` declares 0 × 0, which no cap is below.
    let pdf = open(
        image_pdf(&declared_huge(), &zlib(&pixels())),
        limits(Some(0)),
    );
    let content = pdf
        .xref()
        .get::<Stream<'_>>(ObjectIdentifier::new(4, 0))
        .expect("content stream");
    let before = image_pixel_limit_hits();
    assert_eq!(
        content.decoded().expect("decodes").as_ref(),
        b"q 200 0 0 200 0 0 cm /Im1 Do Q"
    );
    // Even the image stream itself, read as plain data.
    assert_eq!(
        image(&pdf).decoded().expect("decodes").len(),
        (SIDE * SIDE) as usize
    );
    assert_eq!(image_pixel_limit_hits(), before);
}

#[test]
fn hits_are_counted_per_thread() {
    let pdf = open(
        image_pdf(&declared_huge(), &zlib(&pixels())),
        limits(Some(CAP)),
    );
    let before = image_pixel_limit_hits();
    let theirs = std::thread::scope(|s| {
        s.spawn(|| {
            let mine = image_pixel_limit_hits();
            for _ in 0..3 {
                let _ = image(&pdf).decoded_image(&params(20_000, 20_000));
            }
            image_pixel_limit_hits() - mine
        })
        .join()
        .expect("decoding thread")
    });
    assert_eq!(theirs, 3);
    assert_eq!(image_pixel_limit_hits(), before);
}

// ---------------------------------------------------------------------------
// hayro-syntax, codec header and reported sizes (spec Appendix B.4)
//
// CCITT, JBIG2 and JPEG 2000 size their output from the encoded data, not
// from the image dictionary, so every fixture here declares 64 × 64 (passing
// the declared check) and carries a codec header that says otherwise. The
// bodies need not decode: the cap must fire before decoding starts.

/// The 64 × 64 image dimensions every codec fixture declares.
const SMALL: &str = "/Width 64 /Height 64";

/// Decodes object 6 of `bytes` as a 64 × 64 image under `max_image_pixels`.
fn decode_codec(
    bytes: Vec<u8>,
    max_image_pixels: Option<u64>,
    target_dimension: Option<(u32, u32)>,
) -> Result<usize, DecodeFailure> {
    let pdf = open(bytes, limits(max_image_pixels));
    let params = ImageDecodeParams {
        target_dimension,
        ..params(SIDE, SIDE)
    };
    image(&pdf).decoded_image(&params).map(|r| r.data.len())
}

/// Asserts `bytes` is refused under the cap with one hit.
fn assert_refused(bytes: Vec<u8>, target_dimension: Option<(u32, u32)>) {
    let before = image_pixel_limit_hits();
    assert!(matches!(
        decode_codec(bytes, Some(CAP), target_dimension),
        Err(DecodeFailure::ImagePixelLimit)
    ));
    assert_eq!(image_pixel_limit_hits() - before, 1);
}

/// Asserts `bytes` is not refused without a cap (the hand-made bodies may
/// fail to decode, but never by the cap) and that nothing is counted.
fn assert_uncapped_not_refused(bytes: Vec<u8>, target_dimension: Option<(u32, u32)>) {
    let before = image_pixel_limit_hits();
    let result = decode_codec(bytes, None, target_dimension);
    assert!(
        matches!(result, Ok(_) | Err(DecodeFailure::ImageDecode)),
        "{result:?}"
    );
    assert_eq!(image_pixel_limit_hits(), before);
}

/// A CCITT image declaring 64 × 64 whose `/DecodeParms` are `parms`.
fn ccitt(parms: &str, body: &[u8]) -> Vec<u8> {
    ccitt_declaring(SMALL, parms, body)
}

fn ccitt_declaring(dims: &str, parms: &str, body: &[u8]) -> Vec<u8> {
    image_pdf(
        &format!(
            "{dims} /ColorSpace /DeviceGray /BitsPerComponent 1 \
             /Filter /CCITTFaxDecode /DecodeParms << {parms} >>"
        ),
        body,
    )
}

#[test]
fn ccitt_is_checked_on_columns_times_rows() {
    let parms = "/K -1 /Columns 20000 /Rows 20000";
    assert_refused(ccitt(parms, &[0; 16]), None);
    assert_uncapped_not_refused(ccitt(parms, &[0; 16]), None);
}

#[test]
fn ccitt_without_rows_is_bounded_by_its_data() {
    // `/Rows 0`: Group 3 decodes until the data runs out, at no fewer than
    // two bits per row, so 4 rows per byte count: 20000 × 4 × 16 = 1,280,000
    // is over the cap, 20000 × 4 × 8 = 640,000 is not. (These all-zero
    // bodies fail to decode, so the 8-byte one never reaches the reported
    // check, whose 20000 × 64 would refuse it.)
    let parms = "/K 0 /Columns 20000 /Rows 0";
    assert_refused(ccitt(parms, &[0; 16]), None);
    assert_uncapped_not_refused(ccitt(parms, &[0; 16]), None);
    let before = image_pixel_limit_hits();
    assert!(matches!(
        decode_codec(ccitt(parms, &[0; 8]), Some(CAP), None),
        Err(DecodeFailure::ImageDecode)
    ));
    assert_eq!(image_pixel_limit_hits(), before);
}

#[test]
fn ccitt_is_checked_on_its_reported_size() {
    // Declared 1 × 20000 and decoded 20000 × 1 are both 20,000 pixels, but
    // hayro reports `/Columns` × the declared height, 20000 × 20000, which is
    // what hayro-interpret pads to and converts. One white Group 4 row is a
    // single `V0` code (bit 1).
    let parms = "/K -1 /Columns 20000 /Rows 1";
    let bytes = || ccitt_declaring("/Width 1 /Height 20000", parms, &[0x80]);
    let pdf = open(bytes(), limits(Some(CAP)));
    let before = image_pixel_limit_hits();
    assert!(matches!(
        image(&pdf).decoded_image(&params(1, 20_000)),
        Err(DecodeFailure::ImagePixelLimit)
    ));
    assert_eq!(image_pixel_limit_hits() - before, 1);
    // Uncapped, the row decodes and is reported at that size.
    let pdf = open(bytes(), limits(None));
    let decoded = image(&pdf)
        .decoded_image(&params(1, 20_000))
        .unwrap_or_else(|_| panic!("decodes"));
    let info = decoded.image_data.expect("CCITT reports its size");
    assert_eq!((info.width, info.height), (20_000, 20_000));
    assert_eq!(decoded.data.len(), 20_000);
}

/// One embedded-JBIG2 segment: number, type, no referred-to segments, page
/// 1, and its data (T.88 7.2).
fn jbig2_segment(number: u32, kind: u8, data: &[u8]) -> Vec<u8> {
    let mut out = number.to_be_bytes().to_vec();
    out.extend_from_slice(&[kind, 0, 1]);
    out.extend_from_slice(&u32::try_from(data.len()).expect("small").to_be_bytes());
    out.extend_from_slice(data);
    out
}

/// An embedded JBIG2 stream holding a page-information segment (type 48)
/// for a `width × height` page and, when `stripe_end` is given, an
/// end-of-stripe segment (type 50) at that row, then a broken region.
fn jbig2(width: u32, height: u32, stripe_end: Option<u32>) -> Vec<u8> {
    let mut info = width.to_be_bytes().to_vec();
    info.extend_from_slice(&height.to_be_bytes());
    info.extend_from_slice(&[0; 8]); // resolutions unknown
    info.push(0); // page flags
    // Striped (bit 15) when the height is unknown, as T.88 7.4.8.2 requires.
    let striping: u16 = if stripe_end.is_some() {
        0x8000 | 128
    } else {
        0
    };
    info.extend_from_slice(&striping.to_be_bytes());
    let mut data = jbig2_segment(0, 48, &info);
    if let Some(row) = stripe_end {
        data.extend(jbig2_segment(1, 50, &row.to_be_bytes()));
    }
    // An immediate generic region (type 38) too short to hold its region
    // header: decoding fails on it, so a refusal by the cap is visibly the
    // header check, not the reported-size check after a successful decode.
    data.extend(jbig2_segment(2, 38, &[0; 3]));
    image_pdf(
        &format!("{SMALL} /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode"),
        &data,
    )
}

#[test]
fn jbig2_is_checked_on_its_page_size() {
    // Within the cap the fixture reaches the decoder and fails there.
    assert!(matches!(
        decode_codec(jbig2(64, 64, None), Some(CAP), None),
        Err(DecodeFailure::ImageDecode)
    ));
    assert_refused(jbig2(20_000, 20_000, None), None);
    // Height unknown (0xFFFFFFFF): the page is as high as its last stripe.
    assert_refused(jbig2(20_000, 0xFFFF_FFFF, Some(19_999)), None);
    // Controls without a cap, at a size just over it (1001 × 1000) so that
    // decoding stays cheap: refused under the cap, not refused without it.
    assert_refused(jbig2(1_001, 1_000, None), None);
    assert_uncapped_not_refused(jbig2(1_001, 1_000, None), None);
    assert_uncapped_not_refused(jbig2(1_001, 0xFFFF_FFFF, Some(999)), None);
    assert_refused(jbig2(1_001, 0xFFFF_FFFF, Some(999)), None);
}

/// A 64 × 64 gray JPEG 2000 codestream with six resolution levels (five
/// decompositions), one 64 × 64 tile, reversible 5/3 wavelet: written by
/// OpenJPEG through Pillow (`save(..., "JPEG2000", no_jp2=True,
/// num_resolutions=6, irreversible=False)`) with its comment marker removed.
const J2K_64: [&str; 3] = [
    "ff4fff510029000000000040000000400000000000000000000000400000004000000000000000000001070101ff5200",
    "0c00000001000504040001ff5c00134040484850484850484850484850484850ff90000a0000000000290001ff93df18",
    "40078b735dc7c0180d9c4edf184024b516ebdc183951488080ffd9",
];

/// A 64 × 64 gray baseline JPEG (Pillow, quality 50, optimised tables).
const JPEG_64: [&str; 10] = [
    "ffd8ffe000104a46494600010100000100010000ffdb004300100b0c0e0c0a100e0d0e1211101318281a181616183123",
    "251d283a333d3c3933383740485c4e404457453738506d51575f626768673e4d71797064785c656763ffc0000b080040",
    "004001011100ffc4001800010101010100000000000000000000000507060403ffc40025100001040201030403000000",
    "00000000000300021112010406051481133132610741a1ffda0008010100003f00c401a900352006a400d4801a900352",
    "006ae8dad8ecba76cedd2fe809c4acc5ab8ce627f5eca3606a400d4801a900352006a400d4801a87e7db1dbf1778e96e",
    "e0ac1ccc563379fbf8c7953b0352006a400d4801a900352006a400d587fc97b17ea3a1a948f484e25a7ded9888faa7f5",
    "66c0d4801a900352006a400d4801a9003549f93ec777ca3a81294a9723899f8630c9f359f2bcc0d4801a900352006a40",
    "0d4801aba36b63b2e9db3b74bfa02712b316ae33989fd7b28b81a940352006a400d4801a900352006a1f9f6c76fc5de3",
    "a5bb82b0733158cde7efe31e54dc0d4a01a900352006a400d4801a9003561ff25ec5fa8e86a523d2138969f7b66223ea",
    "9fd5970352806a400d4801a900352006a400d527e4fb1ddf28ea04a52a5c8e267e18c327cd67cae70357ffd9",
];

fn unhex(parts: &[&str]) -> Vec<u8> {
    let hex: String = parts.concat();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect()
}

/// The JPEG 2000 fixture with its `SIZ` image size patched to
/// `width × height` (Xsiz and Ysiz, bytes 8-15). The tile size stays 64, so
/// the body no longer matches; the cap fires before decoding.
fn j2k(width: u32, height: u32) -> Vec<u8> {
    let mut data = unhex(&J2K_64);
    assert_eq!(&data[..6], [0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x29], "SOC, SIZ");
    data[8..12].copy_from_slice(&width.to_be_bytes());
    data[12..16].copy_from_slice(&height.to_be_bytes());
    image_pdf(
        &format!("{SMALL} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /JPXDecode"),
        &data,
    )
}

#[test]
fn jpeg2000_is_checked_on_its_full_resolution_size() {
    // The unpatched fixture decodes: the codestream is real.
    assert!(matches!(
        decode_codec(j2k(64, 64), Some(CAP), None),
        Ok(4096)
    ));
    assert_refused(j2k(20_000, 20_000), None);
    // Asking for 64 × 64 selects a reduced resolution of 625 × 625 (five
    // halvings), which alone would pass; the decoder's working set follows
    // the full resolution, so that is what is checked.
    assert_refused(j2k(20_000, 20_000), Some((64, 64)));
    // Controls without a cap at a size just over it, cheap to decode.
    assert_refused(j2k(1_001, 1_000), None);
    assert_uncapped_not_refused(j2k(1_001, 1_000), None);
    assert_uncapped_not_refused(j2k(1_001, 1_000), Some((64, 64)));
}

#[test]
fn jpeg_frames_are_patched_down_to_the_declared_size() {
    // A frame header claiming 20000 × 20000 under a 64 × 64 dictionary:
    // hayro rewrites the frame to the declared size before zune-jpeg sees
    // it, so the image decodes at 64 × 64 and the cap never fires.
    let mut data = unhex(&JPEG_64);
    let sof = data
        .windows(2)
        .position(|w| w == [0xFF, 0xC0])
        .expect("baseline SOF0");
    assert_eq!(&data[sof + 5..sof + 9], [0, 64, 0, 64], "64 × 64 frame");
    data[sof + 5..sof + 9].copy_from_slice(&[0x4E, 0x20, 0x4E, 0x20]);
    let bytes = || {
        image_pdf(
            &format!("{SMALL} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode"),
            &data,
        )
    };
    let before = image_pixel_limit_hits();
    assert!(matches!(decode_codec(bytes(), Some(CAP), None), Ok(4096)));
    assert!(matches!(decode_codec(bytes(), None, None), Ok(4096)));
    assert_eq!(image_pixel_limit_hits(), before);
}

// ---------------------------------------------------------------------------
// hayro-interpret

/// A device that asks for every image's pixels, as a renderer does: a raster
/// image through `with_rgba`, a stencil mask through `with_stencil`. Counts
/// the images whose pixels arrived.
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

/// Interprets page 0 with `Decoding` and returns the interpreter's warnings
/// and how many images were decoded.
fn render(bytes: Vec<u8>, decode: DecodeLimits) -> (Vec<String>, usize) {
    let pdf = open(bytes, decode);
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
fn an_xobject_over_the_cap_is_one_pixel_limit_warning() {
    let before = image_pixel_limit_hits();
    let (warnings, decoded) = render(
        image_pdf(&declared_huge(), &zlib(&pixels())),
        limits(Some(CAP)),
    );
    assert_eq!(warnings, ["ImagePixelLimit"]);
    assert_eq!(decoded, 0);
    assert_eq!(image_pixel_limit_hits() - before, 1);
}

#[test]
fn a_stencil_mask_over_the_cap_is_one_pixel_limit_warning() {
    let (warnings, decoded) = render(
        image_pdf(
            "/Width 20000 /Height 20000 /ImageMask true /Filter /FlateDecode",
            &zlib(&pixels()),
        ),
        limits(Some(CAP)),
    );
    assert_eq!(warnings, ["ImagePixelLimit"]);
    assert_eq!(decoded, 0);
}

#[test]
fn an_inline_image_over_the_cap_is_one_pixel_limit_warning() {
    // ASCII hex so the inline data is plain text; the stream holds one pixel
    // while declaring 400 million.
    let content = "q 200 0 0 200 0 0 cm BI /W 20000 /H 20000 /BPC 8 /CS /G /F /AHx ID 7f> EI Q";
    let before = image_pixel_limit_hits();
    let (warnings, decoded) = render(content_pdf(content), limits(Some(CAP)));
    assert_eq!(warnings, ["ImagePixelLimit"]);
    assert_eq!(decoded, 0);
    assert_eq!(image_pixel_limit_hits() - before, 1);
}

#[test]
fn a_codec_header_over_the_cap_is_one_pixel_limit_warning() {
    // Declared 64 × 64, so only the JBIG2 page header shows the size.
    let before = image_pixel_limit_hits();
    let (warnings, decoded) = render(jbig2(20_000, 20_000, None), limits(Some(CAP)));
    assert_eq!(warnings, ["ImagePixelLimit"]);
    assert_eq!(decoded, 0);
    assert_eq!(image_pixel_limit_hits() - before, 1);
}

#[test]
fn images_within_the_cap_still_decode() {
    let (warnings, decoded) = render(
        image_pdf(&format!("/Width 64 /Height 64 {GRAY}"), &zlib(&pixels())),
        limits(Some(CAP)),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(decoded, 1);
    let inline = "q 200 0 0 200 0 0 cm BI /W 2 /H 1 /BPC 8 /CS /G /F /AHx ID 00ff> EI Q";
    let (warnings, decoded) = render(content_pdf(inline), limits(Some(CAP)));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(decoded, 1);
}

#[test]
fn a_broken_image_within_the_cap_is_still_a_decode_failure() {
    let (warnings, decoded) = render(
        image_pdf(
            &format!("/Width 64 /Height 64 {BROKEN_JPEG}"),
            b"not a jpeg",
        ),
        limits(Some(CAP)),
    );
    assert_eq!(warnings, ["ImageDecodeFailure"]);
    assert_eq!(decoded, 0);
}

// ---------------------------------------------------------------------------
// core plumbing

/// `Limits::max_image_pixels` reaches hayro: a document opened through core
/// with the cap refuses the image through the rendering hook's xref.
#[cfg(feature = "render-support")]
#[test]
fn core_opens_documents_with_the_cap() {
    use pdf_extract_core::{Document, Limits, OpenOptions};

    let open_with = |max_image_pixels| {
        let opts = OpenOptions {
            limits: Limits {
                max_image_pixels,
                ..Limits::default()
            },
            ..OpenOptions::default()
        };
        Document::from_bytes(image_pdf(&declared_huge(), &zlib(&pixels())), &opts).expect("opens")
    };
    let decode = |doc: &Document| {
        doc.page(0)
            .expect("page 0")
            .with_hayro_page(|input| {
                let stream = input
                    .xref
                    .get::<Stream<'_>>(ObjectIdentifier::new(6, 0))
                    .expect("image stream");
                stream.decoded_image(&params(20_000, 20_000)).map(|_| ())
            })
            .expect("open document")
    };
    assert!(matches!(
        decode(&open_with(CAP)),
        Err(DecodeFailure::ImagePixelLimit)
    ));
    // 20000 × 20000 is 400 MP, over the default 256 MP cap too.
    assert!(matches!(
        decode(&open_with(Limits::default().max_image_pixels)),
        Err(DecodeFailure::ImagePixelLimit)
    ));
    assert!(decode(&open_with(400_000_000)).is_ok());
}
