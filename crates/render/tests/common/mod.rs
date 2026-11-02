//! Helpers shared by the render crate's integration tests: opening a
//! document and rendering a page, a 512 KiB-stack runner (spec §2.3), raster
//! probes, and small hand-assembled PDFs for the image fixtures.

// Each test crate includes this module and uses only part of it.
#![allow(dead_code)]

use std::fmt::Write as _;

pub mod codecs;

use pdf_extract_core::{Document, Limits, OpenOptions, WarningKind};
use pdf_extract_render::{GrayImage, RenderError, RenderOptions, Rendered, render_gray};

/// The runner and `SMALL_STACK` (the size every hostile render runs on, spec
/// §2.3) are core's: one measuring runner for both crates' tests, included by
/// path so there is a single copy of the pthread FFI.
#[path = "../../../core/tests/common/stack.rs"]
pub mod stack;
#[allow(unused_imports)] // each test crate uses part of it
pub use stack::{SMALL_STACK, on_small_stack};

/// Opens `bytes` under `limits`.
pub fn open(bytes: Vec<u8>, limits: Limits) -> Document {
    let opts = OpenOptions {
        limits,
        ..OpenOptions::default()
    };
    Document::from_bytes(bytes, &opts).expect("fixture opens")
}

/// Renders page 0 of `bytes` (default limits) at `dpi`.
pub fn render_at(bytes: Vec<u8>, dpi: u32) -> Rendered {
    render_with(bytes, Limits::default(), &options(dpi)).expect("renders")
}

/// Renders page 0 of `bytes` opened under `limits`.
pub fn render_with(
    bytes: Vec<u8>,
    limits: Limits,
    opts: &RenderOptions,
) -> Result<Rendered, RenderError> {
    let doc = open(bytes, limits);
    render_gray(&doc.page(0).expect("page 0"), opts)
}

/// Default options at `dpi`.
pub fn options(dpi: u32) -> RenderOptions {
    RenderOptions {
        dpi,
        ..RenderOptions::default()
    }
}

/// The gray value at pixel `(x, y)` (top-left origin).
pub fn at(image: &GrayImage, x: u32, y: u32) -> u8 {
    image.data[(y * image.width + x) as usize]
}

/// The number of pixels darker than 250 in the rectangle `[x0, x1) × [y0, y1)`.
pub fn dark_in(image: &GrayImage, x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| at(image, x, y) < 250)
        .count()
}

/// The warning kinds of a render, in order.
pub fn kinds(r: &Rendered) -> Vec<WarningKind> {
    r.warnings.iter().map(|w| w.kind).collect()
}

/// A classic-xref PDF whose object `i + 1` is `bodies[i]` and whose catalog
/// is object 1 (exact offsets, so hayro opens it without repair).
pub fn assemble_pdf<B: AsRef<[u8]>>(bodies: &[B]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(bodies.len());
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body.as_ref());
        out.extend_from_slice(b"\nendobj\n");
    }
    let mut tail = String::new();
    let _ = write!(tail, "xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1);
    for offset in offsets {
        let _ = writeln!(tail, "{offset:010} 00000 n ");
    }
    let _ = write!(
        tail,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        bodies.len() + 1,
        out.len()
    );
    out.extend_from_slice(tail.as_bytes());
    out
}

/// A stream object body with `data` verbatim.
pub fn stream_obj(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut s = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    s.extend_from_slice(data);
    s.extend_from_slice(b"\nendstream");
    s
}

/// A 200 × 200 pt page drawing image XObjects: `images[i]` is `(dict, data)`
/// for `/Im<i>`, drawn as a 100 × 100 pt square in a 2 × 2 grid (image `i`
/// at column `i % 2`, row `i / 2`, from the top left).
pub fn images_pdf(images: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut content = String::new();
    let mut xobjects = String::new();
    for (i, _) in images.iter().enumerate() {
        let x = (i % 2) * 100;
        let y = 100 - (i / 2) * 100;
        let _ = write!(content, "q 100 0 0 100 {x} {y} cm /Im{i} Do Q ");
        let _ = write!(xobjects, "/Im{i} {} 0 R ", 5 + i);
    }
    let mut bodies: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] \
             /Resources << /XObject << {xobjects}>> >> /Contents 4 0 R >>"
        )
        .into_bytes(),
        stream_obj("", content.as_bytes()),
    ];
    for (dict, data) in images {
        bodies.push(stream_obj(
            &format!("/Type /XObject /Subtype /Image {dict}"),
            data,
        ));
    }
    assemble_pdf(&bodies)
}

/// A one-page PDF from raw object bodies, numbered from 4: objects 1–3 are
/// the catalog, the page tree and a 200 × 200 page whose `/Resources` is
/// object 4 and whose `/Contents` is object 5.
pub fn raw_pdf<B: AsRef<[u8]>>(objects: &[B]) -> Vec<u8> {
    let mut bodies: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 4 0 R /Contents 5 0 R >>"
            .to_vec(),
    ];
    bodies.extend(objects.iter().map(|o| o.as_ref().to_vec()));
    assemble_pdf(&bodies)
}

/// The workspace's `tests/fixtures` directory.
pub fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// The committed fixtures whose manifest gate is `gate`, as paths.
pub fn manifest_fixtures(gate: &str) -> Vec<std::path::PathBuf> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        fixture: Vec<Entry>,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        path: String,
        gate: Option<String>,
    }
    let dir = fixtures_dir();
    let text = std::fs::read_to_string(dir.join("manifest.toml")).expect("manifest");
    let manifest: Manifest = toml::from_str(&text).expect("manifest parses");
    manifest
        .fixture
        .into_iter()
        .filter(|e| e.gate.as_deref() == Some(gate))
        .map(|e| dir.join(e.path))
        .collect()
}
