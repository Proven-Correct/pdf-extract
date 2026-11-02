// Build script of pdf-extract-tesseract-sys (spec 0002 §3.4). Produces the
// native half of the crate: either a static Leptonica + Tesseract built from
// vendor/ with CMake (feature `vendored`), or the distribution's libraries
// found by pkg-config (feature `system-tesseract`); in both cases it compiles
// csrc/shim.cpp against the matching headers. The vendored build never
// touches the network and never consults an installed Leptonica or
// Tesseract: CMake is pointed at our own Leptonica install and pkg-config is
// disabled inside both CMake projects.

use std::env;
use std::path::{Path, PathBuf};

const BOTH_OR_NEITHER: &str = "pdf-extract-tesseract-sys: enable exactly one of the features \
     `vendored` (the default) and `system-tesseract`";

fn main() {
    let vendored = env::var_os("CARGO_FEATURE_VENDORED").is_some();
    let system = env::var_os("CARGO_FEATURE_SYSTEM_TESSERACT").is_some();
    if vendored == system {
        // Plain stderr plus a failing exit: cargo prints a failed build
        // script's stderr verbatim, on every cargo version we support.
        eprintln!("error: {BOTH_OR_NEITHER}");
        std::process::exit(1);
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=csrc");
    rerun_on_compiler_flags();

    let headers = if vendored {
        vendored_build(&manifest)
    } else {
        system_libraries()
    };
    compile_shim(&manifest, &headers);
    link_runtime();
}

/// Rebuilds the native code when the C or C++ flags change, which is how the
/// sanitizer runs (tools/sanitizers/run.sh) instrument Leptonica, Tesseract
/// and the shim. The cmake crate forwards these variables into
/// `CMAKE_{C,CXX}_FLAGS` through `cc::Build::get_compiler`, but builds that
/// `cc::Build` with `cargo_metadata(false)`, so nothing tells cargo it read
/// them: without these lines a changed `CFLAGS` would reuse the old
/// Leptonica. The list is cc's own lookup order (`Build::target_envs`):
/// target-scoped, then `HOST_`/`TARGET_`-prefixed, then plain.
fn rerun_on_compiler_flags() {
    let target = env::var("TARGET").expect("set by cargo");
    let target_u = target.replace(['-', '.'], "_");
    for var in ["CFLAGS", "CXXFLAGS"] {
        for name in [
            format!("{var}_{target}"),
            format!("{var}_{target_u}"),
            format!("HOST_{var}"),
            format!("TARGET_{var}"),
            var.to_owned(),
        ] {
            println!("cargo:rerun-if-env-changed={name}");
        }
    }
}

/// Include directories the shim needs, and nothing else.
struct Headers {
    /// Directories holding `tesseract/baseapi.h` and the generated
    /// `tesseract/version.h`.
    tesseract: Vec<PathBuf>,
    /// The directory holding `allheaders.h` itself (Tesseract's own
    /// convention: `#include <allheaders.h>`).
    leptonica: Vec<PathBuf>,
}

#[cfg(feature = "vendored")]
fn vendored_build(manifest: &Path) -> Headers {
    let vendor = manifest.join("../../vendor");
    let out = PathBuf::from(env::var("OUT_DIR").expect("set by cargo"));
    println!(
        "cargo:rerun-if-changed={}",
        vendor.join("tesseract").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        vendor.join("leptonica").display()
    );

    // Leptonica without any codec: pixels arrive through SetImage, so the
    // only link dependency left is libm. Installed (headers, static library,
    // LeptonicaConfig.cmake) so Tesseract's find_package can be pointed at it.
    let lept = common(cmake::Config::new(vendor.join("leptonica")))
        .out_dir(out.join("leptonica"))
        .define("BUILD_PROG", "OFF")
        .define("SW_BUILD", "OFF")
        .define("ENABLE_ZLIB", "OFF")
        .define("ENABLE_PNG", "OFF")
        .define("ENABLE_GIF", "OFF")
        .define("ENABLE_JPEG", "OFF")
        .define("ENABLE_TIFF", "OFF")
        .define("ENABLE_WEBP", "OFF")
        .define("ENABLE_OPENJPEG", "OFF")
        .build();

    // Tesseract: only the `libtesseract` target. The CLI executable would
    // otherwise be built too (and would need what we disable below), and an
    // install step would copy configs we pruned. Each flag's reason is in
    // spec §3.4 / D-4; the short version: no legacy engine and no ScrollView
    // (removes the exit() sites), no OpenMP (no hidden thread pool), no
    // -march=native (portable binaries; SIMD is dispatched at run time).
    let tess = common(cmake::Config::new(vendor.join("tesseract")))
        .out_dir(out.join("tesseract"))
        .define("BUILD_TRAINING_TOOLS", "OFF")
        .define("BUILD_TESTS", "OFF")
        .define("DISABLE_TIFF", "ON")
        .define("DISABLE_ARCHIVE", "ON")
        .define("DISABLE_CURL", "ON")
        .define("GRAPHICS_DISABLED", "ON")
        .define("DISABLED_LEGACY_ENGINE", "ON")
        .define("OPENMP_BUILD", "OFF")
        .define("ENABLE_LTO", "OFF")
        .define("ENABLE_NATIVE", "OFF")
        .define("INSTALL_CONFIGS", "OFF")
        .define("ENABLE_CCACHE", "OFF")
        .define("ENABLE_PRECOMPILED_HEADERS", "OFF")
        .define("Leptonica_DIR", lept.join("lib/cmake/leptonica"))
        .build_target("libtesseract")
        .build();
    let tess_build = tess.join("build");

    println!("cargo:rustc-link-search=native={}", tess_build.display());
    println!(
        "cargo:rustc-link-search=native={}",
        lept.join("lib").display()
    );
    // Directive order here is tesseract, leptonica; cc emits the shim's
    // archive afterwards, from compile_shim. That is the reverse of the
    // classic static-link order (the shim needs tesseract, tesseract needs
    // leptonica), and it still links because the default `bundle` modifier
    // merges all three archives into this crate's rlib, where the linker
    // resolves symbols across members regardless of order.
    println!("cargo:rustc-link-lib=static=tesseract");
    println!("cargo:rustc-link-lib=static=leptonica");

    Headers {
        tesseract: vec![vendor.join("tesseract/include"), tess_build.join("include")],
        leptonica: vec![lept.join("include/leptonica")],
    }
}

#[cfg(feature = "vendored")]
fn common(mut cfg: cmake::Config) -> cmake::Config {
    // Release always: an unoptimised Tesseract is an order of magnitude
    // slower, which would make every debug test run crawl.
    cfg.profile("Release")
        .define("BUILD_SHARED_LIBS", "OFF")
        // Position-independent: the archives end up inside the Python
        // extension module, a shared object.
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        // Deterministic layout (`lib`, never `lib64`), so Leptonica_DIR above
        // is right on every Linux distribution.
        .define("CMAKE_INSTALL_LIBDIR", "lib")
        // Keep both projects from discovering a system Leptonica (Tesseract
        // falls back to pkg-config when find_package fails) or OpenMP.
        .define("CMAKE_DISABLE_FIND_PACKAGE_PkgConfig", "ON")
        .define("CMAKE_DISABLE_FIND_PACKAGE_OpenMP", "ON");
    cfg
}

#[cfg(not(feature = "vendored"))]
fn vendored_build(_: &Path) -> Headers {
    unreachable!("main() selects vendored_build only with the `vendored` feature")
}

#[cfg(feature = "system-tesseract")]
fn system_libraries() -> Headers {
    // 1.79.0 is the first Leptonica with leptSetStderrHandler, which the
    // silence guarantee needs; 5.3 is the oldest Tesseract the shim is
    // written against.
    let lept = pkg_config::Config::new()
        .atleast_version("1.79.0")
        .probe("lept")
        .unwrap_or_else(|e| fail(&format!("system Leptonica >= 1.79.0 not found: {e}")));
    let tess = pkg_config::Config::new()
        .atleast_version("5.3")
        .probe("tesseract")
        .unwrap_or_else(|e| fail(&format!("system Tesseract >= 5.3 not found: {e}")));
    let mut leptonica = lept.include_paths.clone();
    // Some lept.pc files name the parent directory rather than .../leptonica.
    leptonica.extend(lept.include_paths.iter().map(|p| p.join("leptonica")));
    Headers {
        tesseract: tess.include_paths,
        leptonica,
    }
}

#[cfg(feature = "system-tesseract")]
fn fail(msg: &str) -> ! {
    eprintln!("error: pdf-extract-tesseract-sys: {msg}");
    std::process::exit(1);
}

#[cfg(not(feature = "system-tesseract"))]
fn system_libraries() -> Headers {
    unreachable!("main() selects system_libraries only with `system-tesseract`")
}

fn compile_shim(manifest: &Path, headers: &Headers) {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file(manifest.join("csrc/shim.cpp"))
        // The C++ runtime is linked once, last, by link_runtime(), so it
        // follows the two static archives that need it.
        .cpp_link_stdlib(None)
        .warnings(true)
        .extra_warnings(true)
        .warnings_into_errors(true);
    // Third-party headers go in as system includes so their own warnings do
    // not trip -Werror on our file. The compiler's own default directories
    // are skipped: re-adding /usr/include with -isystem reorders it before
    // libstdc++'s wrappers and breaks their #include_next <stdlib.h>.
    //
    // The shim also reads Tesseract's parameter registry (`params.h`, for
    // pxt_is_global_param), which no installation ships: it comes from the
    // vendored source tree under both linkages, included as
    // `<ccutil/params.h>` so none of the tree's other headers can shadow a
    // system one. Under `system-tesseract` this assumes the linked
    // library's `Param` and `ParamsVectors` layouts match the vendored
    // 5.5.3 header (`GlobalParams()` itself is an exported symbol); the
    // `ocr-system` CI job runs the refusal tests in tests/instance.rs
    // against the distribution's library, which is what checks it.
    let internal = manifest.join("../../vendor/tesseract/src");
    println!(
        "cargo:rerun-if-changed={}",
        internal.join("ccutil/params.h").display()
    );
    let mut seen = std::collections::BTreeSet::new();
    for dir in headers
        .tesseract
        .iter()
        .chain(&headers.leptonica)
        .chain(std::iter::once(&internal))
    {
        if dir == Path::new("/usr/include")
            || dir == Path::new("/usr/local/include")
            || !seen.insert(dir.clone())
        {
            continue;
        }
        build
            .flag("-isystem")
            .flag(dir.to_str().expect("UTF-8 include path"));
    }
    // The deliberate-throw entry point exists only where debug assertions
    // are on (every `cargo test` build), never in a release artefact.
    if env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some() {
        build.define("PXT_TEST_HOOKS", None);
    }
    build.compile("pxt_shim");
}

fn link_runtime() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("set by cargo");
    let cxx = if matches!(target_os.as_str(), "macos" | "ios" | "freebsd" | "openbsd") {
        "c++"
    } else {
        "stdc++"
    };
    println!("cargo:rustc-link-lib={cxx}");
    println!("cargo:rustc-link-lib=m");
}
