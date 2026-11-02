//! The realistic adversarial pages of spec 0003 Appendix A.6: 300-dpi
//! corpus-D renders transformed pixel by pixel (inverted regions, shaded
//! boxes, flat figures, gradients, slides, inverted noise), shared by the
//! verdict sweep (`tests/precheck.rs`) and the Tesseract pins of the quality
//! gate. Each can also be wrapped back into a one-page PDF whose 300-dpi
//! render is the raster itself.

use pdf_extract_ocr::GrayImage;
use pdf_extract_ocr::precheck::Verdict;
use pdf_extract_render::{RenderOptions, render_gray};
use pdf_extract_testkit::corpus_d::{NOISE_SEED, ScanPage, add_noise, scan_pdf};

/// One recipe: its name, the raster, and the verdict expected both as built
/// and after `downsample2`.
pub struct Recipe {
    pub name: &'static str,
    pub image: GrayImage,
    pub expected: Verdict,
}

/// Page 0 of `pdf` rendered at 300 dpi.
pub fn render300(pdf: Vec<u8>) -> GrayImage {
    let doc = super::doc(pdf);
    let page = doc.page(0).expect("page 0");
    render_gray(
        &page,
        &RenderOptions {
            dpi: 300,
            max_render_pixels: 40_000_000,
            render_annotations: true,
        },
    )
    .expect("renders")
    .image
}

/// The committed corpus-D page `name` at 300 dpi.
pub fn corpus300(name: &str) -> GrayImage {
    render300(super::corpus_d(name))
}

/// A one-page PDF whose image is `img` on a page of `img`'s size at 300
/// dpi, so a 300-dpi render maps image pixels 1:1 and gives `img` back.
/// (A box only close to it — A4's 595 × 842 pt for 2480 × 3509 px —
/// resamples the image, which smooths pixel noise away.) The box is a
/// thousandth of a point short: `595.2` itself reads back a hair over
/// 2480 px and the renderer's `ceil` would add a column.
pub fn to_pdf(img: &GrayImage) -> Vec<u8> {
    #[allow(clippy::cast_precision_loss)]
    let pt = |px: u32| px as f32 * 0.24 - 0.001;
    scan_pdf(&[ScanPage {
        media_box: [0.0, 0.0, pt(img.width), pt(img.height)],
        crop_box: None,
        rotate: 0,
        width: img.width,
        height: img.height,
        pixels: img.data.clone(),
    }])
}

/// The byte range of rows `[from, to)` percent of the height.
fn rows(img: &GrayImage, from: usize, to: usize) -> std::ops::Range<usize> {
    let (w, h) = (img.width as usize, img.height as usize);
    w * (h * from / 100)..w * (h * to / 100)
}

/// `255 − v` over rows `[from, to)` percent.
pub fn invert(mut img: GrayImage, from: usize, to: usize) -> GrayImage {
    let r = rows(&img, from, to);
    for p in &mut img.data[r] {
        *p = 255 - *p;
    }
    img
}

/// White mapped to `bg` (`v × bg / 255`) over rows `[from, to)` percent.
pub fn on_background(mut img: GrayImage, bg: u32, from: usize, to: usize) -> GrayImage {
    let r = rows(&img, from, to);
    for p in &mut img.data[r] {
        *p = u8::try_from(u32::from(*p) * bg / 255).expect("≤ 255");
    }
    img
}

/// Rows `[from, to)` percent replaced by flat `value`.
pub fn flat(mut img: GrayImage, value: u8, from: usize, to: usize) -> GrayImage {
    let r = rows(&img, from, to);
    img.data[r].fill(value);
    img
}

/// Rows `[from, to)` percent replaced by a left-to-right gradient, 0 to 255.
pub fn gradient(mut img: GrayImage, from: usize, to: usize) -> GrayImage {
    let w = img.width as usize;
    let r = rows(&img, from, to);
    for (i, p) in img.data[r.clone()].iter_mut().enumerate() {
        let x = (r.start + i) % w;
        *p = u8::try_from(x * 255 / (w - 1)).expect("≤ 255");
    }
    img
}

/// Background mapped to `bg` and ink to `ink` (a slide).
pub fn remap(mut img: GrayImage, bg: u32, ink: u32) -> GrayImage {
    for p in &mut img.data {
        let v = u32::from(*p);
        *p = u8::try_from((ink * (255 - v) + bg * v) / 255).expect("≤ 255");
    }
    img
}

/// Corpus D's σ-12 noise field with seed `NOISE_SEED + k` added.
pub fn noisy(mut img: GrayImage, k: u64) -> GrayImage {
    add_noise(&mut img.data, NOISE_SEED + k);
    img
}

/// The full catalogue of spec 0003 §4.2's realistic adversarial pages.
pub fn recipes() -> Vec<Recipe> {
    let body = corpus300("clean_body_11pt");
    let heading = corpus300("clean_heading_footnote");
    let sparse = corpus300("sparse_below_min_chars");
    let one_line = corpus300("clean_one_line");
    let clear = |name, image| Recipe {
        name,
        image,
        expected: Verdict::Clear,
    };
    vec![
        clear("banner", invert(heading, 0, 8)),
        clear("inverted upper 45 %", invert(body.clone(), 0, 45)),
        clear(
            "inverted upper 45 % + noise",
            noisy(invert(body.clone(), 0, 45), 1),
        ),
        clear(
            "shaded box 30-60 %",
            on_background(body.clone(), 200, 30, 60),
        ),
        clear(
            "shaded 25-75 % + noise",
            noisy(on_background(body.clone(), 200, 25, 75), 2),
        ),
        clear("gray figure 40-70 %", flat(body.clone(), 128, 40, 70)),
        clear("gray upper half", flat(body.clone(), 128, 0, 50)),
        clear(
            "gray upper half + noise",
            noisy(flat(body.clone(), 128, 0, 50), 3),
        ),
        clear(
            "photo band 40-70 % + noise",
            noisy(gradient(body.clone(), 40, 70), 4),
        ),
        clear(
            "full-page gradient + noise",
            noisy(gradient(body.clone(), 0, 100), 5),
        ),
        clear("slide", remap(body.clone(), 40, 220)),
        clear("slide + noise", noisy(remap(body.clone(), 40, 220), 6)),
        clear("inverted page", invert(body, 0, 100)),
        Recipe {
            name: "sparse on 128 + noise",
            image: noisy(on_background(sparse, 128, 0, 100), 7),
            expected: Verdict::Speckle,
        },
        Recipe {
            name: "one line on 128 + noise",
            image: noisy(on_background(one_line, 128, 0, 100), 8),
            expected: Verdict::Speckle,
        },
    ]
}
