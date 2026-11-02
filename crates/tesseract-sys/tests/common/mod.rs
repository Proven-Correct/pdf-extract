// Shared helpers for the pdf-extract-tesseract-sys integration tests: where
// the model lives, the committed test image, and synthetic images.
#![allow(dead_code)] // each test binary uses a different subset

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use pdf_extract_tesseract_sys::TESSDATA_KNOWN_DIRS;

/// The tessdata directory the tests load `eng` from.
///
/// CI exports `TESSDATA_PREFIX` pointing at the pinned `tessdata_fast` model
/// (spec D-17); locally the same variable, or the first of the usual
/// install locations that has `eng.traineddata`. No model at all is a test
/// failure, not a skip: a silently skipped OCR suite would look green.
pub fn tessdata() -> PathBuf {
    tessdata_candidates(std::env::var_os("TESSDATA_PREFIX"))
        .into_iter()
        .find(|d| d.join("eng.traineddata").is_file())
        .unwrap_or_else(|| {
            panic!(
                "no eng.traineddata found: set TESSDATA_PREFIX to a directory holding it \
                 (CI installs tessdata_fast; see .github/actions/tessdata)"
            )
        })
}

/// The directories `tessdata` tries, in order: the prefix and its `tessdata`
/// subdirectory, then [`TESSDATA_KNOWN_DIRS`].
///
/// This mirrors `pdf_extract_ocr::Candidates::ordered` (this crate sits below
/// the ocr crate and cannot depend on it), including its rule that an empty
/// prefix counts as unset, as Tesseract treats it: an empty prefix would
/// otherwise contribute `""` and a relative `tessdata`, i.e. whatever the
/// test's working directory holds.
pub fn tessdata_candidates(prefix: Option<OsString>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(p) = prefix.filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        candidates.push(p.clone());
        candidates.push(p.join("tessdata"));
    }
    candidates.extend(TESSDATA_KNOWN_DIRS.iter().map(PathBuf::from));
    candidates
}

/// A gray image: width, height, row-major bytes (stride = width).
pub struct Gray {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// `tests/data/hello-300dpi.png`: the word "Hello" in Helvetica 28 pt,
/// rasterised at 300 dpi to 8-bit gray (750 x 300) by PyMuPDF 1.28.2 —
/// a renderer other than the one under test — with
/// `page.insert_text((24, 46), "Hello", fontname="helv", fontsize=28)` on a
/// 180 x 72 pt page and `get_pixmap(dpi=300, colorspace=csGRAY)`.
pub fn hello() -> Gray {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/hello-300dpi.png");
    let file = std::fs::File::open(&path).expect("open hello-300dpi.png");
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .expect("png header");
    let mut data = vec![0; reader.output_buffer_size().expect("buffer size")];
    let info = reader.next_frame(&mut data).expect("png frame");
    assert_eq!(info.color_type, png::ColorType::Grayscale);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    data.truncate(info.buffer_size());
    Gray {
        width: info.width,
        height: info.height,
        data,
    }
}

/// A uniformly white image.
pub fn white(width: u32, height: u32) -> Gray {
    Gray {
        width,
        height,
        data: vec![255; (width * height) as usize],
    }
}

/// Deterministic noise: a 64-bit LCG seeded with `seed`, thresholded so
/// about one pixel in three is black. Clumps of noise are what
/// Tesseract treats as candidate words, which is what keeps it busy (and
/// polling its deadline) on such an image.
pub fn noise(width: u32, height: u32, seed: u64) -> Gray {
    let mut state = seed;
    let data = (0..width * height)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            if (state >> 33).is_multiple_of(3) {
                0
            } else {
                255
            }
        })
        .collect();
    Gray {
        width,
        height,
        data,
    }
}

/// The "Hello" image tiled `cols` x `rows` times: a page full of words.
pub fn tiled_hello(cols: u32, rows: u32) -> Gray {
    let h = hello();
    let width = h.width * cols;
    let height = h.height * rows;
    let mut data = Vec::with_capacity((width * height) as usize);
    for _ in 0..rows {
        for y in 0..h.height {
            let row = &h.data[(y * h.width) as usize..((y + 1) * h.width) as usize];
            for _ in 0..cols {
                data.extend_from_slice(row);
            }
        }
    }
    Gray {
        width,
        height,
        data,
    }
}
