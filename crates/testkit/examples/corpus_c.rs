//! Writes the generated corpus-C fixtures (spec §4.13, Appendix C) into
//! `tests/fixtures/corpus-c/` and their entries into the generated section of
//! `tests/fixtures/manifest.toml`, from the catalogue in
//! `tests/common/corpus/`.
//!
//! Usage: `cargo run -p pdf-extract-testkit --example corpus_c [-- FIXTURES_DIR]`
//! (default: the repository's `tests/fixtures`).
//!
//! Deterministic: the same source gives the same bytes, hashes and manifest
//! text, which `tests/corpus_c.rs` checks against the committed copies.
//! Files listed in the previous generated section but no longer in the
//! catalogue are deleted, so a renamed recipe leaves nothing behind. The
//! encrypted fixtures and the rest of the manifest are not touched. After
//! running this, regenerate the oracle expectations
//! (`tools/oracle/gen.py`, which needs PyMuPDF and is never run on CI) and,
//! for divergence and hostile fixtures, the snapshots (`PARITY_BLESS=1 cargo test -p
//! pdf-extract-core --test parity`).

#[path = "../tests/common/corpus/mod.rs"]
mod corpus;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `corpus-c/<file>` paths named in the generated section of `manifest`.
fn generated_paths(manifest: &str) -> BTreeSet<String> {
    let Some(start) = manifest.find(corpus::MANIFEST_BEGIN) else {
        return BTreeSet::new();
    };
    let end = manifest[start..]
        .find(corpus::MANIFEST_END)
        .map_or(manifest.len(), |e| start + e);
    manifest[start..end]
        .lines()
        .filter_map(|l| l.strip_prefix("path = \"corpus-c/"))
        .filter_map(|l| l.strip_suffix('"'))
        .map(str::to_owned)
        .collect()
}

/// `manifest` with its generated section replaced by `section` (appended
/// when there is none yet).
fn splice(manifest: &str, section: &str) -> String {
    match manifest.find(corpus::MANIFEST_BEGIN) {
        Some(start) => {
            let end = manifest[start..]
                .find(corpus::MANIFEST_END)
                .map(|e| start + e + corpus::MANIFEST_END.len())
                .expect("generated section has an end marker");
            let rest = manifest[end..].trim_start_matches('\n');
            let mut out = format!("{}{section}", &manifest[..start]);
            if !rest.is_empty() {
                out.push('\n');
                out.push_str(rest);
            }
            out
        }
        None => format!("{}\n\n{section}", manifest.trim_end_matches('\n')),
    }
}

fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("corpus-c");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let manifest_path = root.join("manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;

    let fixtures = corpus::catalogue();
    let names: BTreeSet<&str> = fixtures.iter().map(|f| f.file.as_str()).collect();
    if names.len() != fixtures.len() {
        return Err("duplicate file names in the catalogue".into());
    }
    for stale in generated_paths(&manifest)
        .iter()
        .filter(|p| !names.contains(p.as_str()))
    {
        let path = dir.join(stale);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("removed {stale}");
        }
    }

    let mut section = format!("{}\n", corpus::MANIFEST_BEGIN);
    for f in &fixtures {
        let path = dir.join(&f.file);
        std::fs::write(&path, &f.bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        section.push('\n');
        section.push_str(&corpus::manifest_block(f, &sha256_hex(&f.bytes)));
    }
    section.push('\n');
    section.push_str(corpus::MANIFEST_END);
    section.push('\n');
    std::fs::write(&manifest_path, splice(&manifest, &section))
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    println!("wrote {} fixtures to {}", fixtures.len(), dir.display());
    Ok(())
}

fn main() -> ExitCode {
    let root = std::env::args_os().nth(1).map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures"),
        PathBuf::from,
    );
    match run(&root) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("corpus_c: {e}");
            ExitCode::FAILURE
        }
    }
}
