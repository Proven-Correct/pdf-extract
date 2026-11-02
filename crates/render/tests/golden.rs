//! Render determinism (spec 0002 §4.7 "Determinism", O5, D-12).
//!
//! Per host, rendering is exact: the same bytes and options give the same
//! pixels, checked here twice in-process and against a committed SHA-256
//! per fixture and operating system, `render-golden/<name>.<os>.sha256`
//! (`os` is `std::env::consts::OS`: `linux`, `macos`). Across hosts it is
//! not: vello_cpu and pic-scale pick SIMD kernels at run time (NEON on the
//! arm64 macOS runner, AVX2/SSE on the x86-64 Linux one), so CI compares the
//! two hosts' rasters with a tolerance instead — at least 99.9 % identical
//! pixels and no pixel more than 8 gray levels apart — in
//! [`cross_os_rasters_agree`], an ignored test the `render-cross-os` job runs
//! on both hosts' artefacts.
//!
//! Environment:
//!
//! - `RENDER_GOLDEN_OUT=<dir>`: also write `<name>.png` (the gray raster)
//!   and `<name>.<os>.sha256` there; CI uploads the directory, which is both
//!   the cross-OS input and, on a mismatch, the file to commit.
//! - `RENDER_GOLDEN_BLESS=1`: rewrite the committed hash for this OS.
//! - `RENDER_GOLDEN_A`, `RENDER_GOLDEN_B`: the two artefact directories
//!   [`cross_os_rasters_agree`] compares.
//!
//! A hash records a host's renderer as built from the locked dependencies;
//! a lockfile change that moves hayro, vello or pic-scale is expected to
//! need a re-bless (and the cross-OS check says whether the change is
//! benign).

mod common;

use std::path::{Path, PathBuf};

use common::codecs::codec_images;
use common::{fixtures_dir, render_at};
use pdf_extract_render::GrayImage;
use pdf_extract_testkit::{PdfBuilder, TextStyle};
use sha2::{Digest, Sha256};

/// The fixtures, each with the resolution it is rendered at.
fn fixtures() -> Vec<(&'static str, Vec<u8>, u32)> {
    let mut text = PdfBuilder::new();
    let page = text.add_page_a4();
    for (i, line) in [
        "The quick brown fox jumps over the lazy dog.",
        "Pack my box with five dozen liquor jugs: 0123456789",
        "Sphinx of black quartz, judge my vow! (& ; ? / @ #)",
    ]
    .into_iter()
    .enumerate()
    {
        let y = 100.0 + 30.0 * f32::from(u8::try_from(i).expect("few lines"));
        page.insert_text((72.0, y), line, TextStyle::size(14.0))
            .expect("text");
    }
    let mut shapes = PdfBuilder::new();
    shapes
        .add_page(300.0, 200.0)
        .raw_content(
            "0.25 g 10 10 120 80 re f \
             0.6 g 60 40 m 150 190 l 290 20 l h f \
             0 G 3 w 1 J 20 180 m 100 100 200 220 280 120 c S \
             q 150 20 100 60 re W n 0.1 g 0 0 300 200 re f Q",
        )
        .set_rotation(90);
    let annotations = std::fs::read(fixtures_dir().join("corpus-b/pdfbox/Annotations.pdf"))
        .expect("corpus-B Annotations.pdf");
    vec![
        ("text_a4_150dpi", text.to_bytes(), 150),
        ("shapes_rotated_100dpi", shapes.to_bytes(), 100),
        ("codecs_150dpi", codec_images(), 150),
        ("annotations_72dpi", annotations, 72),
    ]
}

/// SHA-256 over the dimensions and the pixels, so two rasters of different
/// shape can never share a hash.
fn digest(image: &GrayImage) -> String {
    let mut h = Sha256::new();
    h.update(image.width.to_le_bytes());
    h.update(image.height.to_le_bytes());
    h.update(&image.data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/render-golden")
}

fn write_png(path: &Path, image: &GrayImage) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().expect("png header");
    writer.write_image_data(&image.data).expect("png data");
}

fn read_png(path: &Path) -> GrayImage {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().expect("png header");
    let mut data = vec![0; reader.output_buffer_size().expect("png size")];
    let info = reader.next_frame(&mut data).expect("png frame");
    assert_eq!(
        info.color_type,
        png::ColorType::Grayscale,
        "{}",
        path.display()
    );
    data.truncate(info.buffer_size());
    GrayImage {
        width: info.width,
        height: info.height,
        data,
    }
}

#[test]
fn rendering_is_deterministic_and_matches_the_golden_hash() {
    let os = std::env::consts::OS;
    let out = std::env::var_os("RENDER_GOLDEN_OUT").map(PathBuf::from);
    let bless = std::env::var_os("RENDER_GOLDEN_BLESS").is_some();
    if let Some(dir) = &out {
        std::fs::create_dir_all(dir).expect("create RENDER_GOLDEN_OUT");
    }
    let mut problems = Vec::new();
    for (name, bytes, dpi) in fixtures() {
        let first = render_at(bytes.clone(), dpi);
        let second = render_at(bytes, dpi);
        assert_eq!(first.image, second.image, "{name}: two renders differ");
        let hash = digest(&first.image);
        let file = format!("{name}.{os}.sha256");
        if let Some(dir) = &out {
            write_png(&dir.join(format!("{name}.png")), &first.image);
            std::fs::write(dir.join(&file), format!("{hash}\n")).expect("write hash");
        }
        let golden = golden_dir().join(&file);
        if bless {
            std::fs::write(&golden, format!("{hash}\n")).expect("bless");
            continue;
        }
        match std::fs::read_to_string(&golden) {
            Ok(want) if want.trim() == hash => {}
            Ok(want) => problems.push(format!("{file}: got {hash}, committed {}", want.trim())),
            Err(_) => problems.push(format!("{file}: missing (got {hash})")),
        }
    }
    assert!(
        problems.is_empty(),
        "golden hashes differ on {os} (re-bless with RENDER_GOLDEN_BLESS=1 after checking \
         the cross-OS comparison):\n{}",
        problems.join("\n")
    );
}

/// How two rasters of one fixture differ: the share of identical pixels
/// and the largest absolute difference.
fn compare(a: &GrayImage, b: &GrayImage) -> Result<(f64, u8), String> {
    if (a.width, a.height) != (b.width, b.height) {
        return Err(format!(
            "dimensions {}×{} vs {}×{}",
            a.width, a.height, b.width, b.height
        ));
    }
    let mut same = 0usize;
    let mut max_delta = 0u8;
    for (&x, &y) in a.data.iter().zip(&b.data) {
        let d = x.abs_diff(y);
        same += usize::from(d == 0);
        max_delta = max_delta.max(d);
    }
    #[allow(clippy::cast_precision_loss)]
    let share = same as f64 / a.data.len().max(1) as f64;
    Ok((share, max_delta))
}

/// D-12's tolerance.
fn within_tolerance(share: f64, max_delta: u8) -> bool {
    share >= 0.999 && max_delta <= 8
}

#[test]
fn the_cross_os_tolerance() {
    let img = |data: Vec<u8>| GrayImage {
        width: 100,
        height: 10,
        data,
    };
    let base = img(vec![128; 1000]);
    let (share, delta) = compare(&base, &base).expect("same shape");
    assert!(within_tolerance(share, delta));
    // One pixel in a thousand may differ, by up to 8 levels.
    let mut one = base.data.clone();
    one[0] = 136;
    let (share, delta) = compare(&base, &img(one.clone())).expect("same shape");
    assert_eq!((share, delta), (0.999, 8));
    assert!(within_tolerance(share, delta));
    one[1] = 129;
    let (share, delta) = compare(&base, &img(one)).expect("same shape");
    assert!(!within_tolerance(share, delta), "two pixels differ");
    let mut far = base.data.clone();
    far[0] = 137;
    let (share, delta) = compare(&base, &img(far)).expect("same shape");
    assert!(!within_tolerance(share, delta), "nine levels apart");
    let narrow = GrayImage {
        width: 10,
        height: 100,
        data: vec![128; 1000],
    };
    assert!(compare(&base, &narrow).is_err());
}

#[test]
fn png_artefacts_round_trip() {
    let dir = std::env::temp_dir().join(format!("render-golden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let image = render_at(codec_images(), 36).image;
    let path = dir.join("codecs.png");
    write_png(&path, &image);
    assert_eq!(read_png(&path), image);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `render-cross-os` CI job: every fixture's raster from host A agrees
/// with host B's within D-12's tolerance.
#[test]
#[ignore = "needs RENDER_GOLDEN_A and RENDER_GOLDEN_B (the render-cross-os CI job)"]
fn cross_os_rasters_agree() {
    let dir = |var: &str| -> PathBuf {
        std::env::var_os(var)
            .map(PathBuf::from)
            .unwrap_or_else(|| panic!("{var} is not set"))
    };
    let (a, b) = (dir("RENDER_GOLDEN_A"), dir("RENDER_GOLDEN_B"));
    let mut failures = Vec::new();
    for (name, _, _) in fixtures() {
        let file = format!("{name}.png");
        let (ia, ib) = (read_png(&a.join(&file)), read_png(&b.join(&file)));
        match compare(&ia, &ib) {
            Ok((share, delta)) => {
                println!("{name}: {:.5} identical, max |Δ| {delta}", share);
                if !within_tolerance(share, delta) {
                    failures.push(format!("{name}: {share:.5} identical, max |Δ| {delta}"));
                }
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
