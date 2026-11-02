//! Writes every generated sample (builder and hostile) into a directory, with
//! an `index.tsv` of `file<TAB>kind<TAB>oracle expectation`, for
//! `tools/oracle/mkfixtures.py --verify` to run through the oracle.
//!
//! Usage: `cargo run -p pdf-extract-testkit --example dump_fixtures -- DIR`

#[path = "../tests/common/samples.rs"]
mod samples;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: dump_fixtures DIR");
        return ExitCode::from(2);
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("cannot create {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    let mut index = String::new();
    for s in samples::builder_samples()
        .into_iter()
        .chain(samples::hostile_samples())
    {
        let file = format!("{}.pdf", s.name);
        if let Err(e) = std::fs::write(dir.join(&file), &s.bytes) {
            eprintln!("cannot write {file}: {e}");
            return ExitCode::FAILURE;
        }
        let _ = writeln!(index, "{file}\t{}\t{}", s.kind, s.oracle.token());
    }
    if let Err(e) = std::fs::write(dir.join("index.tsv"), index) {
        eprintln!("cannot write index.tsv: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
