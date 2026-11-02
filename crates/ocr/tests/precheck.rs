//! `precheck::classify` over corpus D (spec 0003 §4.2 "classify", F16's
//! no-regression claim at the level of the verdict): the 40 committed pages
//! and the 12 noise variants built at test time, rendered at 300 and 72
//! dpi. Only `sparse_below_min_chars_noise` at 300 dpi is speckle, and its
//! half-resolution raster is clear. Then the realistic adversarial pages of
//! Appendix A.6 (revision (3)): large dark regions, figures, slides and
//! inverted pages are clear, inverted noise is speckle. Needs no tessdata
//! (the `rust` job).

mod common;

use std::path::{Path, PathBuf};

use pdf_extract_core::{Document, OpenOptions};
use pdf_extract_ocr::GrayImage;
use pdf_extract_ocr::precheck::{self, OtsuClass, Verdict};
use pdf_extract_render::{RenderOptions, render_gray};
use pdf_extract_testkit::corpus_d;

/// The one corpus-D page the pre-check acts on.
const SPECKLED: &str = "sparse_below_min_chars_noise";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// `(name, path)` of the 40 committed files and the 12 built noise
/// variants (built, or reused when they match their `[[built]]` pins).
fn corpus() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(fixtures().join("corpus-d"))
        .expect("corpus-d")
        .filter_map(|e| {
            let p = e.expect("entry").path();
            let name = p.file_stem()?.to_str()?.to_owned();
            (p.extension()? == "pdf").then_some((name, p))
        })
        .collect();
    assert_eq!(out.len(), 40, "corpus D commits 40 files");
    let text = std::fs::read_to_string(fixtures().join("manifest.toml")).expect("manifest");
    let manifest: toml::Table = toml::from_str(&text).expect("manifest parses");
    let rows = manifest["built"].as_array().expect("[[built]]").clone();
    let pins: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| {
            (
                r["path"].as_str().expect("path"),
                r["sha256"].as_str().expect("sha256"),
            )
        })
        .collect();
    let built = corpus_d::build_noise(&fixtures(), &corpus_d::default_build_dir(), &pins)
        .expect("noise variants build");
    assert_eq!(built.len(), 12);
    out.extend(built.into_iter().map(|b| (b.name, b.path)));
    out.sort();
    out
}

fn render(path: &Path, dpi: u32) -> GrayImage {
    let doc = Document::open(path, &OpenOptions::default()).expect("opens");
    let page = doc.page(0).expect("page 0");
    render_gray(
        &page,
        &RenderOptions {
            dpi,
            max_render_pixels: 40_000_000,
            render_annotations: true,
        },
    )
    .expect("renders")
    .image
}

/// The probe's box filter (spec 0003 Appendix A), kept verbatim in spirit:
/// `(a + b + c + d + 2) / 4` over each 2 x 2 block.
fn probe_box2(img: &GrayImage) -> GrayImage {
    let (w, h) = (img.width as usize / 2, img.height as usize / 2);
    let stride = img.width as usize;
    let mut d = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let i = 2 * y * stride + 2 * x;
            let s = u32::from(img.data[i])
                + u32::from(img.data[i + 1])
                + u32::from(img.data[i + stride])
                + u32::from(img.data[i + stride + 1]);
            d.push(u8::try_from((s + 2) / 4).expect("mean of bytes"));
        }
    }
    GrayImage {
        width: u32::try_from(w).expect("w"),
        height: u32::try_from(h).expect("h"),
        data: d,
    }
}

#[test]
fn only_the_sparse_noise_page_is_speckle_at_300_dpi_and_none_at_72() {
    let pages = corpus();
    assert_eq!(pages.len(), 52);
    // Rendering 104 rasters is the cost; spread it over a few threads.
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let chunks: Vec<&[(String, PathBuf)]> = pages.chunks(pages.len().div_ceil(workers)).collect();
    let verdicts: Vec<(String, Verdict, Verdict)> = std::thread::scope(|s| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|(name, path)| {
                            let at300 = precheck::classify(&render(path, 300));
                            let at72 = precheck::classify(&render(path, 72));
                            (name.clone(), at300, at72)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker"))
            .collect()
    });
    assert_eq!(verdicts.len(), 52);
    let mut wrong = Vec::new();
    for (name, at300, at72) in &verdicts {
        let expected = if name == SPECKLED {
            Verdict::Speckle
        } else {
            Verdict::Clear
        };
        if *at300 != expected {
            wrong.push(format!(
                "{name} at 300 dpi: {at300:?}, expected {expected:?}"
            ));
        }
        if *at72 != Verdict::Clear {
            wrong.push(format!("{name} at 72 dpi: {at72:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_sparse_noise_page_is_clear_after_one_downsample() {
    let doc = common::doc(common::sparse_noise_page());
    let page = doc.page(0).expect("page");
    let img = render_gray(
        &page,
        &RenderOptions {
            dpi: 300,
            max_render_pixels: 40_000_000,
            render_annotations: true,
        },
    )
    .expect("renders")
    .image;
    assert_eq!((img.width, img.height), (2480, 3509));
    let s = precheck::stats(&img);
    // Spec 0003 Appendix A.1: t = 245, p = 0.217, isolation 0.376.
    assert_eq!(s.class, OtsuClass::ForegroundDark);
    assert_eq!(s.threshold, 245);
    assert!((s.foreground - 0.217).abs() < 0.005, "{s:?}");
    assert!(
        (s.isolation.expect("computed") - 0.376).abs() < 0.01,
        "{s:?}"
    );
    assert_eq!(s.verdict(), Verdict::Speckle);

    let half = precheck::downsample2(&img).expect("halves");
    assert_eq!((half.width, half.height), (1240, 1754));
    // Byte-identical to the probe's filter (checked everywhere, which
    // includes the three positions the oracle samples).
    let probe = probe_box2(&img);
    for (x, y) in [(0usize, 0usize), (620, 877), (1239, 1753)] {
        assert_eq!(
            half.data[y * 1240 + x],
            probe.data[y * 1240 + x],
            "({x}, {y})"
        );
    }
    assert_eq!(half, probe);
    let s = precheck::stats(&half);
    assert_eq!(s.class, OtsuClass::ForegroundDark);
    assert!(s.foreground < precheck::MIN_FOREGROUND, "{s:?}");
    assert_eq!(precheck::classify(&half), Verdict::Clear);
}

#[test]
fn the_adversarial_pages_get_their_verdicts_at_both_resolutions() {
    let mut wrong = Vec::new();
    for r in common::adversarial::recipes() {
        let s = precheck::stats(&r.image);
        let half = precheck::downsample2(&r.image).expect("halves");
        let h = precheck::stats(&half);
        if s.verdict() != r.expected || h.verdict() != r.expected {
            wrong.push(format!(
                "{}: expected {:?}, got {:?} ({s:?}) and after downsample2 {:?} ({h:?})",
                r.name,
                r.expected,
                s.verdict(),
                h.verdict()
            ));
        }
        if r.name == "full-page gradient + noise" {
            assert_eq!(s.class, OtsuClass::None, "{s:?}");
            assert!(s.isolation.expect("computed") <= 0.02, "{s:?}");
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn inverted_noise_is_speckle_and_the_inverted_sparse_page_recovers() {
    use common::adversarial::{invert, render300};
    // The pure noise field, inverted: speckle at 300 and at 150.
    let field = invert(render300(common::pure_noise_pdf()), 0, 100);
    let s = precheck::stats(&field);
    assert_eq!(
        (s.class, s.foreground_dark),
        (OtsuClass::BackgroundDark, false)
    );
    assert_eq!(s.verdict(), Verdict::Speckle);
    let half = precheck::downsample2(&field).expect("halves");
    assert_eq!(precheck::classify(&half), Verdict::Speckle);
    // The sparse noise page, inverted: speckle, then clear after one step.
    let sparse = invert(render300(common::sparse_noise_page()), 0, 100);
    let s = precheck::stats(&sparse);
    assert_eq!(
        (s.class, s.foreground_dark),
        (OtsuClass::BackgroundDark, false)
    );
    assert_eq!(s.verdict(), Verdict::Speckle, "{s:?}");
    let half = precheck::downsample2(&sparse).expect("halves");
    assert_eq!(precheck::classify(&half), Verdict::Clear);
}

#[test]
fn an_adversarial_page_renders_back_to_its_raster() {
    // The quality gate recognises these rasters through `to_pdf`; the
    // verdicts above only carry over if the render is the raster.
    use common::adversarial::{invert, render300, to_pdf};
    let img = invert(render300(common::sparse_noise_page()), 0, 100);
    let back = render300(to_pdf(&img));
    assert_eq!((back.width, back.height), (img.width, img.height));
    let differ = img
        .data
        .iter()
        .zip(&back.data)
        .filter(|(a, b)| a != b)
        .count();
    assert!(differ * 1000 < img.data.len(), "{differ} pixels differ");
    assert_eq!(precheck::classify(&back), Verdict::Speckle);
}
