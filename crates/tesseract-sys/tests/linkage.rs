// What the vendored build links (spec 0002 §4.6, D-4): the test binary
// inspects its own dynamic dependencies with `otool -L` (macOS) or `ldd`
// (Linux) and its undefined symbols with `nm -u`. No OpenMP runtime (C5: no
// hidden thread pool), no image codecs, archive or network libraries, and no
// dynamically linked Tesseract/Leptonica: both are static archives built
// from vendor/. The `system-tesseract` build links the distribution's shared
// libraries by design, so this file checks only the vendored build.
#![cfg(feature = "vendored")]

use std::process::Command;

use pdf_extract_tesseract_sys::{LINKAGE, Linkage};

const FORBIDDEN_LIBS: &[&str] = &[
    "libgomp",
    "libomp",
    "libiomp",
    "libtiff",
    "libarchive",
    "libcurl",
    "libpng",
    "libjpeg",
    "libwebp",
    "libgif",
    "libopenjp2",
    "libz.",
    "libtesseract",
    "liblept",
];

fn run(cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("run {cmd}: {e}"));
    assert!(
        out.status.success(),
        "{cmd} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn links_no_openmp_codec_or_shared_tesseract() {
    assert_eq!(LINKAGE, Linkage::Vendored);
    // Make sure the native code is actually in this binary: the linker
    // would otherwise be free to drop the archives and the test would pass
    // vacuously.
    assert!(!pdf_extract_tesseract_sys::version().is_empty());

    let exe = std::env::current_exe().expect("own path");
    let exe = exe.to_str().expect("UTF-8 path");
    let deps = if cfg!(target_os = "macos") {
        run("otool", &["-L", exe])
    } else {
        run("ldd", &[exe])
    };
    for lib in FORBIDDEN_LIBS {
        assert!(!deps.contains(lib), "test binary links {lib}:\n{deps}");
    }

    let undefined = run("nm", &["-u", exe]);
    for sym in [
        "GOMP_",
        "omp_get_",
        "__kmpc_",
        "TIFF",
        "archive_read",
        "curl_",
    ] {
        assert!(
            !undefined.lines().any(|l| l.contains(sym)),
            "test binary imports a {sym} symbol"
        );
    }
}
