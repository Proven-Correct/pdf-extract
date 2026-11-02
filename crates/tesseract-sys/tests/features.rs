// The two link features are mutually exclusive and one is required (spec
// 0002 §3.4, §4.6). build.rs enforces it; this test proves it by running
// `cargo check` on scratch packages that depend on this crate with neither
// feature and with both, and asserting build.rs's message. The scratch
// packages live under CARGO_TARGET_TMPDIR with their own `[workspace]`
// table (so they are not mistaken for members of this workspace) and share
// one target directory across runs. They get a copy of the workspace's
// Cargo.lock, after a `cargo fetch --locked` of the whole workspace (a
// no-op once everything is cached; a default build alone never downloads
// `pkg-config`), so `cargo check --offline` resolves the same versions.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const MESSAGE: &str = "enable exactly one of the features `vendored` (the default) and \
                       `system-tesseract`";

fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fetch_once() {
    static FETCHED: OnceLock<()> = OnceLock::new();
    FETCHED.get_or_init(|| {
        let status = Command::new(cargo())
            .current_dir(workspace_root())
            .args(["fetch", "--locked", "--quiet"])
            .status()
            .expect("run cargo fetch");
        assert!(status.success(), "cargo fetch failed");
    });
}

fn scratch(name: &str, dependency: &str) -> PathBuf {
    fetch_once();
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("tesseract-sys-features");
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).expect("scratch dir");
    let sys = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
         publish = false\n\n[workspace]\n\n[dependencies]\n\
         pdf-extract-tesseract-sys = {{ path = {path:?}, {dependency} }}\n",
        path = sys.to_str().expect("UTF-8 path"),
    );
    std::fs::write(dir.join("Cargo.toml"), manifest).expect("manifest");
    std::fs::write(dir.join("src/lib.rs"), "").expect("lib.rs");
    std::fs::copy(workspace_root().join("Cargo.lock"), dir.join("Cargo.lock")).expect("lock");
    dir
}

fn check_fails_with_message(dir: &Path) {
    let out = Command::new(cargo())
        .current_dir(dir)
        .args(["check", "--offline", "--quiet"])
        // A sanitizer or coverage run of this suite must not leak its
        // flags into the scratch build: `-Zsanitizer` without `--target`
        // reaches proc-macros, which refuse it, and the build then fails
        // for a reason other than the one under test.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env(
            "CARGO_TARGET_DIR",
            Path::new(env!("CARGO_TARGET_TMPDIR")).join("tesseract-sys-features/target"),
        )
        .output()
        .expect("run cargo check");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "cargo check succeeded:\n{stderr}");
    assert!(stderr.contains(MESSAGE), "unexpected failure:\n{stderr}");
}

#[test]
fn neither_feature_is_refused() {
    check_fails_with_message(&scratch("neither", "default-features = false"));
}

#[test]
fn both_features_are_refused() {
    check_fails_with_message(&scratch("both", "features = [\"system-tesseract\"]"));
}
