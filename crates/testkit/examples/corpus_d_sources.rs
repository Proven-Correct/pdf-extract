//! Writes corpus D's vector source pages and their ground truth into a
//! directory (`<name>.pdf`, `<name>.txt`), for `tools/oracle/mkcorpusd.py`
//! to rasterise with the oracle (run 0002 §3.8).
//!
//! Usage: `cargo run -p pdf-extract-testkit --example corpus_d_sources -- DIR`

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: corpus_d_sources DIR");
        return ExitCode::from(2);
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("cannot create {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    for s in pdf_extract_testkit::corpus_d::sources() {
        for (ext, bytes) in [("pdf", s.pdf.as_slice()), ("txt", s.truth.as_bytes())] {
            let path = dir.join(format!("{}.{ext}", s.name));
            if let Err(e) = std::fs::write(&path, bytes) {
                eprintln!("cannot write {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
