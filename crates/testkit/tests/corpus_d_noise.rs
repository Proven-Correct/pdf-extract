//! Corpus D's test-time noise variants hash to their pins (run 0002 §3.8,
//! §4.12): all twelve are built into `corpus_d::default_build_dir()` and
//! each SHA-256 must equal its `manifest.toml` `[[built]]` row, and a second
//! build reuses the files untouched. Runs in the `rust` job; needs no
//! tessdata. A failure here means the noise model, the assembly or a clean
//! file's pixels changed: re-pin (README "Corpus D") or find the drift.

use std::path::{Path, PathBuf};

use pdf_extract_testkit::corpus_d;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn pins() -> Vec<(String, String)> {
    let text = std::fs::read_to_string(fixtures_dir().join("manifest.toml")).expect("manifest");
    let table: toml::Table = toml::from_str(&text).expect("manifest parses");
    table
        .get("built")
        .and_then(toml::Value::as_array)
        .expect("[[built]] rows")
        .iter()
        .map(|r| {
            (
                r["path"].as_str().expect("path").to_owned(),
                r["sha256"].as_str().expect("sha256").to_owned(),
            )
        })
        .collect()
}

#[test]
fn the_noise_variants_hash_to_their_pins_and_are_reused() {
    let pins = pins();
    let pins: Vec<(&str, &str)> = pins.iter().map(|(p, h)| (p.as_str(), h.as_str())).collect();
    let out = corpus_d::default_build_dir();
    let first = corpus_d::build_noise(&fixtures_dir(), &out, &pins).expect("builds");
    assert_eq!(first.len(), 12);
    let drift: Vec<String> = first
        .iter()
        .filter(|b| b.pinned.as_deref() != Some(b.sha256.as_str()))
        .map(|b| format!("{}: built {}, pinned {:?}", b.name, b.sha256, b.pinned))
        .collect();
    assert!(
        drift.is_empty(),
        "noise variants differ from their [[built]] pins:\n{}\nRe-pin: cargo run -p \
         pdf-extract-testkit --example corpus_d_noise -- DIR; cd tools/oracle && uv run python \
         mkcorpusd.py --pin-built DIR; gen.py --ocr --built DIR; mkcorpusd.py --verify --built DIR",
        drift.join("\n")
    );
    let mtimes: Vec<_> = first
        .iter()
        .map(|b| {
            std::fs::metadata(&b.path)
                .and_then(|m| m.modified())
                .expect("mtime")
        })
        .collect();
    let second = corpus_d::build_noise(&fixtures_dir(), &out, &pins).expect("reuses");
    for ((a, b), t) in first.iter().zip(&second).zip(&mtimes) {
        assert_eq!(a, b);
        let now = std::fs::metadata(&b.path)
            .and_then(|m| m.modified())
            .expect("mtime");
        assert_eq!(&now, t, "{}: rebuilt instead of reused", b.name);
    }
}

#[test]
fn a_file_that_does_not_match_its_pin_is_rebuilt_and_reported() {
    let out = std::env::temp_dir().join(format!("pxt-noise-drift-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).expect("mkdir");
    let name = "clean_one_line_noise.pdf";
    std::fs::write(out.join(name), b"stale").expect("stale file");
    let built = corpus_d::build_noise(&fixtures_dir(), &out, &[(name, "0".repeat(64).as_str())])
        .expect("never fails on a mismatch");
    let b = built
        .iter()
        .find(|b| b.path.ends_with(name))
        .expect("built");
    assert_eq!(b.pinned.as_deref(), Some("0".repeat(64).as_str()));
    assert_ne!(b.sha256, "0".repeat(64));
    assert_ne!(std::fs::read(&b.path).expect("read"), b"stale");
    assert!(built.iter().filter(|b| b.pinned.is_none()).count() == 11);
    // No temporary files left behind.
    let names: Vec<_> = std::fs::read_dir(&out)
        .expect("ls")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(names.len(), 12, "{names:?}");
    let _ = std::fs::remove_dir_all(&out);
}
