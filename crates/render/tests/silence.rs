// Nothing on the render path writes to fds 1 or 2 (spec 0002 O6, §4.7
// "Silence"; run 0001 T48 extended to rendering).
//
// The parent run re-executes this binary as a child with stdout and stderr
// captured; the child renders the hostile set (committed and runtime), the
// codec images, an image over the pixel cap and a broken image — the paths
// where hayro, vello or a codec would complain if anything did — and must
// write nothing. `harness = false` (Cargo.toml) so the child's only output
// could be the libraries'; libtest would print its own banner.

mod common;

use std::process::{Command, ExitCode};

use common::codecs::{SMALL_GRAY, codec_images};
use common::{images_pdf, manifest_fixtures, on_small_stack, options, render_with};
use pdf_extract_core::{Document, Limits, OpenOptions};
use pdf_extract_render::render_gray;
use pdf_extract_testkit::hostile;

const CHILD_ENV: &str = "PDF_EXTRACT_RENDER_SILENCE_CHILD";

/// Renders every page of `bytes` that opens; outcomes are not checked here
/// (`hostile.rs` does), only that nothing is printed.
fn render_all(bytes: Vec<u8>) {
    on_small_stack(move || {
        if let Ok(doc) = Document::from_bytes(bytes, &OpenOptions::default()) {
            for page in doc.pages().take(4) {
                let _ = render_gray(&page, &options(36));
            }
        }
    });
}

fn child() {
    for path in manifest_fixtures("hostile") {
        render_all(std::fs::read(path).expect("fixture reads"));
    }
    for bytes in [
        hostile::deep_nesting_in_content(200_000),
        hostile::deep_nesting_in_resources(200_000),
        hostile::deep_nesting_in_type4_function(200_000),
        hostile::flate_bomb(50 << 20),
        hostile::form_self_recursion(),
        hostile::nested_forms(60),
        codec_images(),
        images_pdf(&[(&format!("{SMALL_GRAY} /Filter /DCTDecode"), vec![0; 32])]),
    ] {
        render_all(bytes);
    }
    let capped = Limits {
        max_image_pixels: 1,
        ..Limits::default()
    };
    let r = render_with(codec_images(), capped, &options(72)).expect("renders");
    assert!(!r.warnings.is_empty());
}

fn main() -> ExitCode {
    if std::env::var_os(CHILD_ENV).is_some() {
        child();
        return ExitCode::SUCCESS;
    }
    // libtest passes filter/flag arguments to every test binary; this one
    // runs its single check regardless (`--list` included, which prints
    // nothing and exits, so `cargo test -- --list` stays well-formed).
    if std::env::args().any(|a| a == "--list") {
        return ExitCode::SUCCESS;
    }
    let out = Command::new(std::env::current_exe().expect("own path"))
        .env(CHILD_ENV, "1")
        .output()
        .expect("run child");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() || !stdout.is_empty() || !stderr.is_empty() {
        eprintln!(
            "silence: FAILED (status {})\n--- child stdout ---\n{stdout}\n--- child stderr ---\n{stderr}",
            out.status
        );
        return ExitCode::FAILURE;
    }
    println!("silence: child wrote nothing to fd 1 or fd 2: ok");
    ExitCode::SUCCESS
}
