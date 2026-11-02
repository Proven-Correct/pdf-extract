// Nothing reaches fds 1 or 2 (spec 0002 O14, O40; §4.6 "Silence").
//
// The parent run re-executes this binary as a child with stdout and stderr
// captured; the child performs the operations Tesseract and Leptonica
// would otherwise report on — a failing Init (bad path: "Error opening data
// file ..."), a refused attempt to reset `debug_file` (spec 0003 F18), a
// recognition at resolution 0 ("Invalid resolution ..."), an image over
// Leptonica's size limit, and a successful recognition — and must write
// nothing. `harness = false` (Cargo.toml) so the child's only output could
// be the libraries'; libtest would print its own banner.

mod common;

use std::path::Path;
use std::process::{Command, ExitCode};
use std::time::Duration;

use pdf_extract_tesseract_sys::{EngineMode, Instance};

const CHILD_ENV: &str = "PDF_EXTRACT_TESSERACT_SYS_SILENCE_CHILD";

fn child() {
    let dir = common::tessdata();
    assert!(
        Instance::new(
            Path::new("/nonexistent/tessdata"),
            "eng",
            EngineMode::LstmOnly
        )
        .is_err()
    );
    // Spec 0003 F18: ("debug_file", "") would send every tprintf back to
    // stderr; it is refused. The failing Init after it prints through
    // tprintf ("Error opening data file ..."), so a reset that got through
    // would show up there.
    assert!(
        Instance::with_variables(&dir, "eng", EngineMode::LstmOnly, &[("debug_file", "")]).is_err()
    );
    assert!(Instance::new(&dir, "zz_missing", EngineMode::LstmOnly).is_err());

    let mut inst = Instance::new(&dir, "eng", EngineMode::LstmOnly).expect("Init");
    let img = common::hello();
    inst.set_image(&img.data, img.width, img.height, img.width)
        .expect("set_image");
    inst.set_source_resolution(0).expect("resolution 0");
    let _ = inst.recognize(Duration::from_secs(60));
    inst.clear();

    let side = 46_341u32;
    let huge = vec![0u8; (side as usize) * (side as usize)];
    assert!(inst.set_image(&huge, side, side, side).is_err());
    drop(huge);

    inst.set_image(&img.data, img.width, img.height, img.width)
        .expect("set_image");
    inst.set_source_resolution(300).expect("resolution");
    inst.recognize(Duration::from_secs(60)).expect("recognize");
    let words = inst.blocks().expect("blocks");
    assert_eq!(words.len(), 1);
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
