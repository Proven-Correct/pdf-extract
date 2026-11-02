//! Minimal image streams for each codec hayro decodes (DCT, JPEG 2000,
//! JBIG2, CCITT), shared by the image and golden tests.

use super::images_pdf;

/// A 64 × 64 gray baseline JPEG (the H13 fixture of
/// `crates/core/tests/h13_image_pixels.rs`).
pub const JPEG_64: [&str; 10] = [
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

/// A 64 × 64 gray JPEG 2000 codestream (the same H13 fixture file).
pub const J2K_64: [&str; 3] = [
    "ff4fff510029000000000040000000400000000000000000000000400000004000000000000000000001070101ff5200",
    "0c00000001000504040001ff5c00134040484850484850484850484850484850ff90000a0000000000290001ff93df18",
    "40078b735dc7c0180d9c4edf184024b516ebdc183951488080ffd9",
];

/// An 8 × 8 all-black bitmap in T.6 (Group 4 / MMR) coding: the first row
/// is horizontal mode (`001`), a white run of 0 (`00110101`) and a black run
/// of 8 (`000101`); each later row repeats the one above, `V0 V0` (`11`).
/// 31 bits, padded with one zero.
pub const MMR_BLACK_8X8: [u8; 4] = [0x26, 0xA2, 0xFF, 0xFE];

pub fn unhex(parts: &[&str]) -> Vec<u8> {
    let hex: String = parts.concat();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect()
}

/// One embedded-JBIG2 segment (T.88 7.2): number, type, no referred-to
/// segments, page 1, data length, data.
fn jbig2_segment(number: u32, kind: u8, data: &[u8]) -> Vec<u8> {
    let mut out = number.to_be_bytes().to_vec();
    out.extend_from_slice(&[kind, 0, 1]);
    out.extend_from_slice(&u32::try_from(data.len()).expect("small").to_be_bytes());
    out.extend_from_slice(data);
    out
}

/// An embedded JBIG2 stream: an 8 × 8 page-information segment (type 48)
/// and an immediate generic region (type 38) covering it, MMR-coded black.
pub fn jbig2_black_8x8() -> Vec<u8> {
    let mut page = 8u32.to_be_bytes().to_vec();
    page.extend_from_slice(&8u32.to_be_bytes());
    page.extend_from_slice(&[0; 8]); // resolutions unknown
    page.push(0); // page flags
    page.extend_from_slice(&[0, 0]); // not striped
    let mut region = Vec::new();
    for v in [8u32, 8, 0, 0] {
        region.extend_from_slice(&v.to_be_bytes()); // width, height, x, y
    }
    region.push(0); // combination operator OR
    region.push(1); // generic region flags: MMR
    region.extend_from_slice(&MMR_BLACK_8X8);
    let mut data = jbig2_segment(0, 48, &page);
    data.extend(jbig2_segment(1, 38, &region));
    data
}

pub const SMALL_GRAY: &str = "/Width 64 /Height 64 /ColorSpace /DeviceGray /BitsPerComponent 8";
pub const BILEVEL_8: &str = "/Width 8 /Height 8 /ColorSpace /DeviceGray /BitsPerComponent 1";

/// A 200 × 200 pt page drawing the DCT, JPX, JBIG2 and CCITT images in a
/// 2 × 2 grid, in that order (see [`images_pdf`]).
pub fn codec_images() -> Vec<u8> {
    images_pdf(&[
        (&format!("{SMALL_GRAY} /Filter /DCTDecode"), unhex(&JPEG_64)),
        (&format!("{SMALL_GRAY} /Filter /JPXDecode"), unhex(&J2K_64)),
        (
            &format!("{BILEVEL_8} /Filter /JBIG2Decode"),
            jbig2_black_8x8(),
        ),
        (
            &format!(
                "{BILEVEL_8} /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8 /Rows 8 >>"
            ),
            MMR_BLACK_8X8.to_vec(),
        ),
    ])
}
