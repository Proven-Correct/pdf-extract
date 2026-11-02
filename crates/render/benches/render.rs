//! Render benchmark (spec 0002 §4.7 "Bench"): wall-clock of `render_gray`
//! on an A4 text page at 72 and 300 dpi.
//!
//! REPORT-ONLY. The CI `bench` job runs it and uploads the bencher-format
//! output beside the P1 numbers; there is no baseline yet and nothing here
//! gates (shared runners are too noisy to decide pass/fail on timings).

use std::hint::black_box;
use std::time::Duration;

use criterion::Criterion;
use pdf_extract_core::{Document, OpenOptions};
use pdf_extract_render::{RenderOptions, render_gray};
use pdf_extract_testkit::{PdfBuilder, TextStyle};

/// An A4 page of 40 lines of 11 pt Helvetica: a dense text page, the kind
/// OCR is asked to read.
fn text_page() -> Vec<u8> {
    let mut doc = PdfBuilder::new();
    let page = doc.add_page_a4();
    for i in 0..40u8 {
        let y = 60.0 + 18.0 * f32::from(i);
        page.insert_text(
            (60.0, y),
            "The quick brown fox jumps over the lazy dog, 0123456789 times.",
            TextStyle::size(11.0),
        )
        .expect("text");
    }
    doc.to_bytes()
}

fn main() {
    let doc = Document::from_bytes(text_page(), &OpenOptions::default()).expect("opens");
    let page = doc.page(0).expect("page 0");
    let mut c = Criterion::default().configure_from_args();
    let mut group = c.benchmark_group("render");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    for dpi in [72u32, 300] {
        let opts = RenderOptions {
            dpi,
            ..RenderOptions::default()
        };
        group.bench_function(format!("a4_text/{dpi}dpi"), |b| {
            b.iter(|| black_box(render_gray(&page, &opts).expect("renders")));
        });
    }
    group.finish();
    c.final_summary();
}
