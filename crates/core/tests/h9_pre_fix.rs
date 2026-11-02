//! Regression pin for fork change H9 (spec §4.8b bullet 1): the concurrent
//! mis-offset lookup workload panics on hayro-syntax *without* H9 and passes
//! with it.
//!
//! The pre-H9 code no longer exists in this tree, so the test rebuilds it:
//! it copies `vendor/hayro-syntax` and reverse-applies the exported patch
//! series from the H9 patch onward (`vendor/patches/hayro/*-H9-*.patch` and
//! anything after it), which yields exactly the H1–H8 state the H9 commit
//! started from. A small harness crate is then built twice, against that
//! copy and against the real `vendor/hayro-syntax`, and run as a child
//! process on the same input: the same 16 threads × 100 `XRef::get` of the
//! mis-offset object as `h9_xref_repair.rs`. A child process is needed
//! because the failure is a panic (or, with a poisoned lock, a cascade of
//! them) inside hayro's lookup, which must not take this test binary down.
//!
//! The pre-H9 failure is a race, not a deterministic fault (the old code has
//! no hook to pause in), so the harness repeats the workload up to
//! [`ATTEMPTS`] times on fresh documents and stops at the first failure. The
//! post-H9 build must survive every attempt.
//!
//! Cost: each run builds hayro-syntax twice (unoptimised, no image
//! decoders), a few seconds. Everything is built in a fresh directory under
//! the OS temp dir that is deleted afterwards, not under `target/`: the
//! harness build is over 100 MB, and CI caches `target/`. The child `cargo`
//! runs `--offline` from a copy of the workspace `Cargo.lock`, so it needs
//! nothing the workspace build has not already fetched. Requires `git` (for
//! `git apply`), as `tools/vendor/check-patches.sh` does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use hayro_syntax::Pdf;
use hayro_syntax::object::ObjectIdentifier;
use pdf_extract_testkit::contract;

/// How many fresh documents the harness tries before declaring the build
/// race-free.
const ATTEMPTS: u32 = 50;

/// Upper bound on one harness run (not the build).
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// The harness program. It uses only APIs that exist both before and after
/// H9. Exit status 0: every attempt passed; 101: some thread panicked or a
/// lookup returned `None` (reported on stdout).
const HARNESS_MAIN: &str = r#"
use std::sync::{Arc, Barrier};
use std::thread;

use hayro_syntax::Pdf;
use hayro_syntax::object::{Dict, ObjectIdentifier};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("input file");
    let id = ObjectIdentifier::new(args[2].parse().expect("object number"), 0);
    let attempts: u32 = args[3].parse().expect("attempts");
    for attempt in 1..=attempts {
        let pdf = Arc::new(Pdf::new(bytes.clone()).expect("opens"));
        let start = Arc::new(Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (pdf, start) = (pdf.clone(), start.clone());
                thread::spawn(move || {
                    start.wait();
                    (0..100).all(|_| pdf.xref().get::<Dict<'_>>(id).is_some())
                })
            })
            .collect();
        let mut panicked = 0;
        let mut missing = 0;
        for h in handles {
            match h.join() {
                Ok(true) => {}
                Ok(false) => missing += 1,
                Err(_) => panicked += 1,
            }
        }
        if panicked + missing > 0 {
            println!("FAILED attempt {attempt}: {panicked} thread(s) panicked, {missing} saw None");
            std::process::exit(101);
        }
    }
    println!("PASSED all {attempts} attempts");
}
"#;

/// A fresh directory under the OS temp dir, removed (with its contents) on
/// drop, including when the test fails. Hand-rolled to avoid a
/// dev-dependency for one test.
struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        // `create_dir`, not `create_dir_all`: fails rather than reuse a
        // directory that already exists.
        fs::create_dir(&path).expect("create temp dir");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Writes `contents` to `path` unless it already holds exactly that, so an
/// unchanged file keeps its mtime and cargo does not rebuild it.
fn write_if_changed(path: &Path, contents: &[u8]) {
    if fs::read(path).ok().as_deref() == Some(contents) {
        return;
    }
    fs::create_dir_all(path.parent().expect("file has a parent")).expect("mkdir");
    fs::write(path, contents).expect("write");
}

/// Every file under `dir`, as paths relative to it.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for entry in fs::read_dir(dir.join(&rel)).expect("read_dir") {
            let entry = entry.expect("dir entry");
            let rel = rel.join(entry.file_name());
            if entry.file_type().expect("file type").is_dir() {
                stack.push(rel);
            } else {
                out.push(rel);
            }
        }
    }
    out
}

/// Makes `to` a copy of `from`: changed files rewritten, files that are gone
/// from `from` removed, identical files untouched.
fn sync_dir(from: &Path, to: &Path) {
    let wanted = files_under(from);
    for rel in &wanted {
        write_if_changed(&to.join(rel), &fs::read(from.join(rel)).expect("read"));
    }
    if to.exists() {
        for rel in files_under(to) {
            if !wanted.contains(&rel) {
                fs::remove_file(to.join(rel)).expect("remove stale file");
            }
        }
    }
}

/// Builds the pre-H9 hayro-syntax source tree under `work` and returns the
/// crate directory.
fn pre_h9_tree(work: &Path) -> PathBuf {
    let patches_dir = Path::new(REPO).join("vendor/patches/hayro");
    let mut patches: Vec<PathBuf> = fs::read_dir(&patches_dir)
        .expect("vendor/patches/hayro")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "patch"))
        .collect();
    patches.sort();
    let first_h9 = patches
        .iter()
        .position(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().contains("-H9-"))
        })
        .expect(
            "no H9 patch in vendor/patches/hayro: run tools/vendor/export-patches.sh \
             (this test reverse-applies it to rebuild the pre-H9 code)",
        );

    // Stage a fresh copy and reverse-apply the series from its newest patch
    // back to H9, then sync it into place (keeping mtimes where possible).
    let staging = work.join("staging");
    let _ = fs::remove_dir_all(&staging);
    sync_dir(
        &Path::new(REPO).join("vendor/hayro-syntax"),
        &staging.join("hayro-syntax"),
    );
    for patch in patches[first_h9..].iter().rev() {
        // Should the staging directory sit inside a git repository (a temp
        // dir under a checkout, say), `git apply` would find the repository,
        // read the patch paths relative to its root and skip every file,
        // successfully. The ceiling stops that search at `work`; the check
        // below catches it if it happens anyway.
        let status = Command::new("git")
            .current_dir(&staging)
            .env("GIT_CEILING_DIRECTORIES", work)
            .args([
                "apply",
                "-R",
                "--whitespace=nowarn",
                "--include=hayro-syntax/*",
            ])
            .arg(patch)
            .status()
            .expect("run git apply");
        assert!(status.success(), "reverse-applying {}", patch.display());
    }
    let xref = "hayro-syntax/src/xref.rs";
    assert_ne!(
        fs::read(staging.join(xref)).expect("staged xref.rs"),
        fs::read(Path::new(REPO).join("vendor").join(xref)).expect("vendored xref.rs"),
        "reverse-applying the H9 patch left xref.rs unchanged"
    );
    let tree = work.join("tree/hayro-syntax");
    sync_dir(&staging.join("hayro-syntax"), &tree);
    tree
}

/// Writes a harness crate named `name` depending on hayro-syntax at
/// `hayro`, builds it into the shared target directory and returns the
/// binary.
fn build_harness(work: &Path, name: &str, hayro: &Path) -> PathBuf {
    let dir = work.join(name);
    // `[workspace]` keeps the harness out of the enclosing repository's
    // workspace (the target directory lives inside it). Only `std`: the image
    // decoders are irrelevant to xref lookups and would lengthen the build.
    // The decoders are still optional dependencies, though, and cargo
    // resolves optional dependencies into the lockfile whether or not they
    // are enabled. The workspace takes the vendored codecs from `vendor/`
    // through `[patch.crates-io]`, so the copied lockfile names them with no
    // registry source; without the same patch here, an `--offline` resolve
    // looks them up in the crates.io index and fails on a machine that never
    // downloaded them (a fresh CI runner).
    let vendor = Path::new(REPO).join("vendor");
    let manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
         [dependencies]\nhayro-syntax = {{ path = {:?}, default-features = false, features = [\"std\"] }}\n\n\
         [patch.crates-io]\nhayro-jbig2 = {{ path = {:?} }}\nhayro-jpeg2000 = {{ path = {:?} }}\n\n\
         [workspace]\n",
        hayro.display().to_string(),
        vendor.join("hayro-jbig2").display().to_string(),
        vendor.join("hayro-jpeg2000").display().to_string(),
    );
    write_if_changed(&dir.join("Cargo.toml"), manifest.as_bytes());
    write_if_changed(&dir.join("src/main.rs"), HARNESS_MAIN.as_bytes());
    write_if_changed(
        &dir.join("Cargo.lock"),
        &fs::read(Path::new(REPO).join("Cargo.lock")).expect("workspace Cargo.lock"),
    );

    let target = work.join("target");
    let mut cmd = Command::new(env!("CARGO"));
    cmd.current_dir(&dir)
        .args(["build", "--offline", "--quiet"])
        .env("CARGO_TARGET_DIR", &target);
    // A coverage or custom-flags run of this test suite must not leak its
    // instrumentation into the harness build (or its profiles into the
    // report). cargo-llvm-cov also sets `RUSTC_WRAPPER` to itself, plus
    // variables of its own whose set changes between releases, hence the
    // prefix match.
    for var in [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTDOCFLAGS",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "LLVM_PROFILE_FILE",
        "CARGO_BUILD_TARGET_DIR",
        "CARGO_INCREMENTAL",
    ] {
        cmd.env_remove(var);
    }
    for (var, _) in std::env::vars_os() {
        let name = var.to_string_lossy();
        if name.starts_with("CARGO_LLVM_COV") || name.starts_with("__CARGO_LLVM_COV") {
            cmd.env_remove(&var);
        }
    }
    let out = cmd.output().expect("run cargo build for the harness");
    assert!(
        out.status.success(),
        "building {name} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    target.join("debug").join(name)
}

/// Runs a harness binary to completion (or the timeout) and returns its
/// output.
fn run_harness(bin: &Path, input: &Path, obj: i32) -> Output {
    let mut child = Command::new(bin)
        .arg(input)
        .arg(obj.to_string())
        .arg(ATTEMPTS.to_string())
        .env_remove("LLVM_PROFILE_FILE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn harness");
    let deadline = Instant::now() + RUN_TIMEOUT;
    while child.try_wait().expect("wait").is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("{} did not finish in {RUN_TIMEOUT:?}", bin.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().expect("harness output")
}

#[test]
fn concurrent_lookups_panic_without_h9_and_pass_with_it() {
    let temp = TempDir::new("pdf-extract-h9-pre-fix");
    let work = temp.0.as_path();

    let bytes = contract::misoffset_object(40, 6, 10);
    let pdf = Pdf::new(bytes.clone()).expect("opens");
    let id: ObjectIdentifier = pdf.pages()[6]
        .raw()
        .get_ref(b"Contents")
        .expect("content reference")
        .into();
    let input = work.join("misoffset.pdf");
    write_if_changed(&input, &bytes);
    let obj = id.obj_number;

    let pre_tree = pre_h9_tree(work);
    let pre = build_harness(work, "h9-harness-pre", &pre_tree);
    let post = build_harness(
        work,
        "h9-harness-post",
        &Path::new(REPO).join("vendor/hayro-syntax"),
    );

    let before = run_harness(&pre, &input, obj);
    let before_out = String::from_utf8_lossy(&before.stdout);
    let before_err = String::from_utf8_lossy(&before.stderr);
    eprintln!("pre-H9 harness: {:?}; {}", before.status, before_out.trim());
    assert!(
        !before.status.success(),
        "hayro without H9 survived {ATTEMPTS} concurrent attempts:\n{before_out}"
    );
    // A thread panicked inside hayro (or the process aborted): either way
    // the stderr names a panic, not some unrelated failure of the harness.
    assert!(
        before_err.contains("panicked"),
        "pre-H9 harness failed without a panic:\n{before_out}\n{before_err}"
    );

    let after = run_harness(&post, &input, obj);
    let after_out = String::from_utf8_lossy(&after.stdout);
    assert!(
        after.status.success(),
        "hayro with H9 failed:\n{after_out}\n{}",
        String::from_utf8_lossy(&after.stderr)
    );
    assert!(after_out.contains("PASSED"), "{after_out}");
}
