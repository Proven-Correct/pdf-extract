//! Builds corpus D's twelve noise variants into a directory and prints one
//! `<sha256>  <path>  ok|DRIFT|unpinned` line per file against the
//! `[[built]]` pins in `tests/fixtures/manifest.toml` (run 0002 §3.8).
//!
//! The producer, not the check: it exits 0 whenever the files were written,
//! so `tools/oracle/mkcorpusd.py --pin-built <dir>` can pin new ones.
//! `tests/corpus_d_noise.rs` is the check.
//!
//! Usage: `cargo run -p pdf-extract-testkit --example corpus_d_noise -- DIR`

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pdf_extract_testkit::corpus_d;

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: corpus_d_noise DIR");
        return ExitCode::from(2);
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let manifest = match std::fs::read_to_string(fixtures.join("manifest.toml")) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("cannot read manifest.toml: {e}");
            return ExitCode::FAILURE;
        }
    };
    let table: toml::Table = match toml::from_str(&manifest) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("manifest.toml does not parse: {e}");
            return ExitCode::FAILURE;
        }
    };
    let pins: Vec<(String, String)> = table
        .get("built")
        .and_then(toml::Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    Some((
                        r.get("path")?.as_str()?.to_owned(),
                        r.get("sha256")?.as_str()?.to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let pins: Vec<(&str, &str)> = pins.iter().map(|(p, h)| (p.as_str(), h.as_str())).collect();
    match corpus_d::build_noise(&fixtures, &dir, &pins) {
        Ok(built) => {
            for b in built {
                let state = match &b.pinned {
                    None => "unpinned",
                    Some(p) if *p == b.sha256 => "ok",
                    Some(_) => "DRIFT",
                };
                println!("{}  {}  {state}", b.sha256, b.path.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
