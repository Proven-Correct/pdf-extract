//! Helper discovery through the engine (spec 0002 O28, D-16; §4.9
//! "Discovery"). The variable and `PATH` are this process's own
//! environment, so the checks run in sequence from one `main`
//! (`harness = false`): no other thread reads the environment meanwhile.

mod common;

use std::path::Path;

use pdf_extract_core::{OcrUnavailable, UnavailableReason};
use pdf_extract_ocr::isolation::client::helper_log;
use pdf_extract_ocr::{OcrConfig, OcrEngine};

const VAR: &str = "PDF_EXTRACT_OCR_HELPER";

fn build(cfg: OcrConfig) -> Result<OcrEngine, OcrUnavailable> {
    OcrEngine::tesseract_with_candidates(cfg, &common::nowhere())
}

fn set(name: &str, value: impl AsRef<std::ffi::OsStr>) {
    // SAFETY: single-threaded (harness = false, no threads spawned yet or
    // alive between these calls), so nothing reads the environment
    // concurrently.
    unsafe { std::env::set_var(name, value) };
}

fn unset(name: &str) {
    // SAFETY: as in `set`.
    unsafe { std::env::remove_var(name) };
}

fn not_built(r: Result<OcrEngine, OcrUnavailable>, what: &str) -> String {
    match r {
        Err(u) if u.reason == UnavailableReason::EngineNotBuilt => u.detail,
        Err(u) => panic!("{what}: expected EngineNotBuilt, got {u}"),
        Ok(_) => panic!("{what}: expected EngineNotBuilt, got an engine"),
    }
}

#[cfg(unix)]
fn main() {
    use std::os::unix::fs::PermissionsExt;

    if std::env::args().any(|a| a == "--list") {
        return;
    }
    let helper = Path::new(env!("CARGO_BIN_EXE_pdf-extract-ocr-helper"));
    // No PATH at all: discovery must never need it.
    set("PATH", "");
    let spawned = || helper_log().0.len();

    // A relative variable is refused (not searched), even though helper_dir
    // would have found the binary.
    set(VAR, "relative/path");
    let before = spawned();
    let d = not_built(build(common::process_config()), "relative");
    assert!(d.contains("absolute"), "{d}");
    assert_eq!(spawned(), before, "refused before spawning");

    // An absolute path to a file without the executable bit.
    let dir = common::scratch("discovery");
    let plain = dir.join("not-executable");
    std::fs::write(&plain, b"").expect("write");
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    set(VAR, &plain);
    let d = not_built(build(common::process_config()), "non-executable");
    assert!(d.contains("not executable"), "{d}");

    // An absolute path to the real helper works, whatever helper_dir says.
    set(VAR, helper);
    let e = build(OcrConfig {
        helper_dir: Some(dir.clone()),
        ..common::process_config()
    })
    .expect("absolute variable");
    drop(e);
    unset(VAR);

    // helper_dir holding the binary works with an empty PATH.
    let e = build(common::process_config()).expect("helper_dir");
    drop(e);

    // Nothing found: refused at construction, not at the first page.
    let d = not_built(
        build(OcrConfig {
            helper_dir: Some(dir),
            ..common::process_config()
        }),
        "empty helper_dir",
    );
    assert!(d.contains("not found"), "{d}");

    // A relative or empty helper_dir is refused, even with the helper in
    // the working directory (it would be exec'd through the default search
    // path, not the file checked).
    let cwd = common::scratch("discovery-cwd");
    std::fs::copy(helper, cwd.join("pdf-extract-ocr-helper")).expect("copy");
    std::env::set_current_dir(&cwd).expect("chdir");
    for rel in ["", "."] {
        let d = not_built(
            build(OcrConfig {
                helper_dir: Some(rel.into()),
                ..common::process_config()
            }),
            "relative helper_dir",
        );
        assert!(d.contains("must be an absolute path"), "{rel:?}: {d}");
    }

    // No helper_dir: beside the current executable (target/*/deps here),
    // where cargo does not put the helper.
    let d = not_built(
        build(OcrConfig {
            helper_dir: None,
            ..common::process_config()
        }),
        "beside the executable",
    );
    assert!(d.contains("the executable's directory"), "{d}");
    println!("process discovery: ok");
}

#[cfg(not(unix))]
fn main() {}
