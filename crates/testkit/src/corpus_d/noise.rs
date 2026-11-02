//! Corpus D's noise variants, built at test time (run 0002 §3.8, D-22).
//!
//! The twelve noise files are not committed: at σ = 12 the noise does not
//! compress (55.1 MB measured against a 25 MB cap). Instead each one is
//! rebuilt here from the committed clean file: its image is Flate-decoded
//! (the oracle's pixels, so no pixel is ours, D-9), noise is added, and a new
//! one-page PDF is assembled. `manifest.toml` pins each result's SHA-256 in
//! a `[[built]]` row; this module carries no hashes.
//!
//! Byte identity on every host is what makes a pin possible, so:
//! - the deviates are integer Irwin–Hall over SplitMix64 (no `ln`/`cos`,
//!   whose last bits differ between libms);
//! - the image stream is unfiltered (deflate output differs between
//!   compressor versions, and noise would not shrink much anyway);
//! - the file has no `/ID`, `/Info` or dates, and fixed object numbers.
//!
//! The variant depends on the clean file's pixels and boxes, never on its
//! bytes, so regenerating the clean files (new random `/ID`) keeps the pins.
//!
//! The same assembly ([`scan_pdf`] over [`ScanPage`]s) builds the pure-noise
//! page of run 0003's noise pre-check tests ([`pure_noise_page`]: the noise
//! field on white, nothing else) and multi-page documents mixing such pages.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use hayro_syntax::Pdf;
use hayro_syntax::object::{Array, Dict, Name, Stream};
use hayro_syntax::page::Rotation;
use sha2::{Digest, Sha256};

use super::sources;
use crate::BuildError;
use crate::pdfwriter::{Objects, num, stream_body};

/// Seed of every noise field; the generator is reset to it for each file.
pub const NOISE_SEED: u64 = 20_261_003;

/// `%PDF-1.4` plus the binary-marker comment.
const HEADER: &[u8] = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n";

/// SplitMix64 (Steele, Lea, Flood 2014): tiny, full-period, and specified
/// entirely by wrapping `u64` arithmetic.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// One deviate with σ ≈ 12: the sum of 12 uniform 20-bit draws, centred
/// (variance `2^40 - 1`), scaled by `12 / 2^20` and rounded to nearest with
/// ties away from zero. `|n| <= 72`.
fn deviate(rng: &mut SplitMix64) -> i32 {
    const ONE: i64 = 1 << 20;
    let sum: i64 = (0..12)
        .map(|_| i64::try_from(rng.next() >> 44).expect("20 bits"))
        .sum();
    let d = sum - 6 * (ONE - 1);
    let scaled = 12 * d;
    // Rust's `/` truncates toward zero, so adding half with the sign of
    // `d` rounds half away from zero.
    let n = if scaled >= 0 {
        (scaled + ONE / 2) / ONE
    } else {
        (scaled - ONE / 2) / ONE
    };
    i32::try_from(n).expect("|n| <= 72")
}

/// The first `count` deviates for `seed` (for the pinned-values test).
#[cfg(test)]
fn deviates(seed: u64, count: usize) -> Vec<i32> {
    let mut rng = SplitMix64(seed);
    (0..count).map(|_| deviate(&mut rng)).collect()
}

/// Adds the noise field for `seed` to 8-bit gray samples in place, one
/// deviate per sample in order (row-major for an image), clamped to 0–255.
pub fn add_noise(gray: &mut [u8], seed: u64) {
    let mut rng = SplitMix64(seed);
    for p in gray {
        let v = i32::from(*p) + deviate(&mut rng);
        *p = u8::try_from(v.clamp(0, 255)).expect("clamped");
    }
}

/// `"<source>_noise"` for each source page, in catalogue order.
pub fn noise_names() -> Vec<String> {
    sources()
        .iter()
        .map(|s| format!("{}_noise", s.name))
        .collect()
}

/// `/DeviceGray`, or `[/ICCBased <profile with /N 1>]`, which is what
/// PyMuPDF writes for a gray pixmap: a one-component profile is a gray space,
/// and the built file says `/DeviceGray` either way.
fn is_gray(image: &Dict<'_>) -> bool {
    if let Some(n) = image.get::<Name<'_>>("ColorSpace") {
        return n.as_str() == "DeviceGray";
    }
    let Some(array) = image.get::<Array<'_>>("ColorSpace") else {
        return false;
    };
    let mut it = array.flex_iter();
    let family = it.next::<Name<'_>>();
    let profile = it.next::<Stream<'_>>();
    matches!((family, profile), (Some(f), Some(p))
        if f.as_str() == "ICCBased" && p.dict().get::<u32>("N") == Some(1) && it.next::<Name<'_>>().is_none())
}

/// `/Rotate` in degrees (hayro's `Rotation` has no `PartialEq`).
fn degrees(r: Rotation) -> u32 {
    match r {
        Rotation::None => 0,
        Rotation::Horizontal => 90,
        Rotation::Flipped => 180,
        Rotation::FlippedHorizontal => 270,
    }
}

fn not_a_scan(what: &str) -> BuildError {
    BuildError::CorpusD(format!("not a one-image gray scan: {what}"))
}

/// `[x0 y0 x1 y1]` with [`num`]'s spelling.
fn spell(v: [f32; 4]) -> String {
    format!("[{} {} {} {}]", num(v[0]), num(v[1]), num(v[2]), num(v[3]))
}

/// A hayro box as `f32`s.
fn rect(r: hayro_syntax::object::Rect) -> [f32; 4] {
    #[allow(clippy::cast_possible_truncation)] // boxes are f32-sized numbers
    [r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32]
}

/// One page of an image-only PDF: an unfiltered 8-bit `/DeviceGray` image
/// drawn over the whole MediaBox, as a scanner (or corpus D) makes them.
#[derive(Clone, Debug, PartialEq)]
pub struct ScanPage {
    /// `[x0 y0 x1 y1]` in points.
    pub media_box: [f32; 4],
    /// Written only when `Some` (and then whatever its value).
    pub crop_box: Option<[f32; 4]>,
    /// `/Rotate` in degrees; written only when non-zero.
    pub rotate: u32,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// `width × height` gray samples, row-major, top row first.
    pub pixels: Vec<u8>,
}

/// Assembles `pages` into one PDF: catalog 1, page tree 2, then page `i`
/// at object `3 + 3i`, its image at `4 + 3i` and its content stream at
/// `5 + 3i`. Fixed object numbers, no `/ID`, `/Info` or dates, unfiltered
/// streams: the bytes depend on the pages alone (the pins of the noise
/// variants rely on it).
pub fn scan_pdf(pages: &[ScanPage]) -> Vec<u8> {
    let first = |i: usize| 3 + 3 * i;
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", first(i)))
        .collect();
    let mut objs = Objects::new();
    objs.add("<< /Type /Catalog /Pages 2 0 R >>");
    objs.add(format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    ));
    for (i, p) in pages.iter().enumerate() {
        let (image, content) = (first(i) + 1, first(i) + 2);
        let media = p.media_box;
        let mut page_dict = format!("<< /Type /Page /Parent 2 0 R /MediaBox {}", spell(media));
        if let Some(crop) = p.crop_box {
            page_dict.push_str(&format!(" /CropBox {}", spell(crop)));
        }
        if p.rotate != 0 {
            page_dict.push_str(&format!(" /Rotate {}", p.rotate));
        }
        page_dict.push_str(&format!(
            " /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {content} 0 R >>"
        ));
        let draw = format!(
            "q {} 0 0 {} {} {} cm /Im0 Do Q",
            num(media[2] - media[0]),
            num(media[3] - media[1]),
            num(media[0]),
            num(media[1])
        );
        objs.add(page_dict);
        objs.add(stream_body(
            &format!(
                "/Type /XObject /Subtype /Image /Width {} /Height {} \
                 /ColorSpace /DeviceGray /BitsPerComponent 8",
                p.width, p.height
            ),
            &p.pixels,
        ));
        objs.add(stream_body("", draw.as_bytes()));
    }
    objs.to_bytes_with_header(1, "", HEADER)
}

/// The noise field of [`NOISE_SEED`] on a white `width × height` raster.
pub fn noise_field(width: u32, height: u32) -> Vec<u8> {
    let len = usize::try_from(u64::from(width) * u64::from(height)).unwrap_or(0);
    let mut pixels = vec![u8::MAX; len];
    add_noise(&mut pixels, NOISE_SEED);
    pixels
}

/// An A4 page (595 × 842 pt) whose only content is a 2480 × 3509 image of
/// [`noise_field`]: what a scan of a blank sheet under corpus D's noise
/// looks like, rendered at 300 dpi pixel for pixel. Run 0003's noise
/// pre-check skips it at 300 dpi and at 150.
pub fn pure_noise_page() -> ScanPage {
    let (width, height) = (2480, 3509);
    ScanPage {
        media_box: [0.0, 0.0, 595.0, 842.0],
        crop_box: None,
        rotate: 0,
        width,
        height,
        pixels: noise_field(width, height),
    }
}

/// The noise variant of a committed clean corpus-D file: its single
/// 8-bit `/DeviceGray` image decoded, noise added with [`NOISE_SEED`], and
/// re-wrapped on a page with the clean page's MediaBox, CropBox (when it
/// differs) and `/Rotate` (when non-zero).
///
/// # Errors
/// As [`noise_variant_page`].
pub fn noise_variant(clean_pdf: &[u8]) -> Result<Vec<u8>, BuildError> {
    Ok(scan_pdf(&[noise_variant_page(clean_pdf)?]))
}

/// The page [`noise_variant`] writes, for assembling it into a larger
/// document with [`scan_pdf`].
///
/// # Errors
/// [`BuildError::CorpusD`] when the file does not parse, has other than one
/// page, carries a text layer, has other than one image XObject, or the
/// image is not unpredicted 8-bit DeviceGray of `width × height` bytes.
pub fn noise_variant_page(clean_pdf: &[u8]) -> Result<ScanPage, BuildError> {
    let pdf = Pdf::new(clean_pdf.to_vec()).map_err(|e| not_a_scan(&format!("{e:?}")))?;
    let pages = pdf.pages();
    if pages.len() != 1 {
        return Err(not_a_scan(&format!("{} pages", pages.len())));
    }
    let page = &pages[0];
    // A lending iterator (each instruction borrows the iterator), so no
    // `Iterator::any`.
    let mut ops = page.operations();
    let mut has_text = false;
    while let Some(op) = ops.next() {
        has_text |= &**op.operator == b"BT";
    }
    if has_text || !page.resources().fonts.is_empty() {
        return Err(not_a_scan("the page has a text layer"));
    }
    let xobjects = &page.resources().x_objects;
    if xobjects.len() != 1 {
        return Err(not_a_scan(&format!("{} XObjects", xobjects.len())));
    }
    let key = xobjects.keys().next().expect("one entry");
    let image: Stream<'_> = xobjects
        .get(key.as_ref())
        .ok_or_else(|| not_a_scan("the XObject is not a stream"))?;
    let dict = image.dict();
    let name = |k: &str| dict.get::<Name<'_>>(k).map(|n| n.as_str().to_owned());
    if name("Subtype").as_deref() != Some("Image") {
        return Err(not_a_scan("the XObject is not an image"));
    }
    if !is_gray(dict) {
        return Err(not_a_scan("the image is not one-component gray"));
    }
    if dict.get::<u32>("BitsPerComponent") != Some(8) {
        return Err(not_a_scan("the image is not 8 bits per component"));
    }
    let (Some(w), Some(h)) = (dict.get::<u32>("Width"), dict.get::<u32>("Height")) else {
        return Err(not_a_scan("the image has no /Width or /Height"));
    };
    let mut pixels = image
        .decoded()
        .map_err(|e| not_a_scan(&format!("decode failed: {e:?}")))?
        .into_owned();
    if pixels.len() as u64 != u64::from(w) * u64::from(h) {
        return Err(not_a_scan(&format!(
            "{} sample bytes for {w} x {h}",
            pixels.len()
        )));
    }
    add_noise(&mut pixels, NOISE_SEED);

    let media = rect(page.media_box());
    let crop = rect(page.crop_box());
    Ok(ScanPage {
        media_box: media,
        // Compared as written, as the pinned files were made.
        crop_box: (spell(crop) != spell(media)).then_some(crop),
        rotate: degrees(page.rotation()),
        width: w,
        height: h,
        pixels,
    })
}

/// Where the noise variants are built: `$CARGO_TARGET_DIR/corpus-d`, else
/// `<workspace>/target/corpus-d` (a relative `CARGO_TARGET_DIR` is taken
/// against the workspace, as cargo does). Never under `tests/fixtures/`.
pub fn default_build_dir() -> PathBuf {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace.join("target"), |d| workspace.join(d))
        .join("corpus-d")
}

/// One built noise variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// `<source>_noise`.
    pub name: String,
    /// The file in the build directory.
    pub path: PathBuf,
    /// SHA-256 of the file as built (or reused), lower-case hex.
    pub sha256: String,
    /// The pin the caller passed for `<name>.pdf`, if any.
    pub pinned: Option<String>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn io_err(path: &Path, e: &std::io::Error) -> BuildError {
    BuildError::CorpusD(format!("{}: {e}", path.display()))
}

/// Builds all twelve noise variants into `out` from
/// `<fixtures>/corpus-d/<source>.pdf`. `pins` are `(path, sha256)` pairs
/// from `manifest.toml` `[[built]]` (`path` is `<name>.pdf`). A file
/// already in `out` whose hash equals its pin is reused untouched; anything
/// else is rebuilt and written to a temporary name, then renamed, so a
/// concurrent reader never sees a partial file.
///
/// Never fails on a hash mismatch: the caller compares
/// [`Built::sha256`] with [`Built::pinned`] and decides.
///
/// # Errors
/// [`BuildError::CorpusD`] when a clean file is missing or not a scan, or
/// a file cannot be written.
pub fn build_noise(
    fixtures: &Path,
    out: &Path,
    pins: &[(&str, &str)],
) -> Result<Vec<Built>, BuildError> {
    std::fs::create_dir_all(out).map_err(|e| io_err(out, &e))?;
    let mut built = Vec::new();
    for name in noise_names() {
        let file = format!("{name}.pdf");
        let pinned = pins
            .iter()
            .find(|(p, _)| *p == file)
            .map(|(_, h)| (*h).to_owned());
        let path = out.join(&file);
        if let (Some(pin), Ok(existing)) = (&pinned, std::fs::read(&path))
            && sha256_hex(&existing) == *pin
        {
            built.push(Built {
                name,
                path,
                sha256: pin.clone(),
                pinned,
            });
            continue;
        }
        let source = name.strip_suffix("_noise").expect("noise name");
        let clean_path = fixtures.join("corpus-d").join(format!("{source}.pdf"));
        let clean = std::fs::read(&clean_path).map_err(|e| io_err(&clean_path, &e))?;
        let bytes = noise_variant(&clean)
            .map_err(|e| BuildError::CorpusD(format!("{}: {e}", clean_path.display())))?;
        // Unique per process and per call, so neither two test binaries nor
        // two threads of one ever share a temporary file.
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = out.join(format!(".{file}.{}.{n}.tmp", std::process::id()));
        std::fs::write(&tmp, &bytes).map_err(|e| io_err(&tmp, &e))?;
        std::fs::rename(&tmp, &path).map_err(|e| io_err(&path, &e))?;
        built.push(Built {
            name,
            path,
            sha256: sha256_hex(&bytes),
            pinned,
        });
    }
    Ok(built)
}

#[cfg(test)]
#[path = "noise_tests.rs"]
mod tests;
