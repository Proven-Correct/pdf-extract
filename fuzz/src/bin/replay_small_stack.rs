//! Small-stack replay of a fuzz corpus (design D7, §8.6): every input runs
//! through a document-level driver on a fresh thread with a 512 KiB stack,
//! the stack the library promises to work on. `tools/fuzz/run.sh` runs it
//! over each document target's accumulated corpus at the end of a session.
//!
//! Usage: `replay_small_stack <open|text|render|all> <file-or-dir>...`
//! (directories are read one level deep). Each input's path is printed before
//! it runs, so when a stack overflow aborts the process the last line names
//! the input. Exit status 0 when every input ended in a result or a typed
//! error, 1 when any panicked (each one is named), 2 on a usage error.
//!
//! Built as a plain release binary, not under libFuzzer and its sanitizer
//! instrumentation (which inflate stack frames), so the stack it measures is
//! close to a production build's. Inputs run strictly one after another: the
//! process spawns no other threads, so glibc's stack cache can only hand a
//! new thread a block left by an earlier 512 KiB thread, never a larger one
//! (the issue `crates/core/tests/common/stack.rs` has to measure around).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;

use pdf_extract_fuzz::{open_and_count, render_page0, text_sampled_pages};

const SMALL_STACK: usize = 512 * 1024;

type Driver = fn(Vec<u8>);

fn drivers(target: &str) -> Option<Vec<(&'static str, Driver)>> {
    let all: [(&'static str, Driver); 3] = [
        ("open", open_and_count),
        ("text", text_sampled_pages),
        ("render", render_page0),
    ];
    match target {
        "all" => Some(all.to_vec()),
        _ => all.iter().find(|(n, _)| *n == target).map(|d| vec![*d]),
    }
}

/// Files named directly, plus the files directly inside named directories,
/// in sorted order per directory.
fn collect(args: &[String]) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for arg in args {
        let path = Path::new(arg);
        if path.is_dir() {
            let mut files: Vec<PathBuf> = std::fs::read_dir(path)?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            files.sort();
            out.extend(files);
        } else {
            out.push(path.to_path_buf());
        }
    }
    Ok(out)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((target, paths)) = args.split_first() else {
        eprintln!("usage: replay_small_stack <open|text|render|all> <file-or-dir>...");
        return ExitCode::from(2);
    };
    let Some(drivers) = drivers(target) else {
        eprintln!("unknown target {target:?}: expected open, text, render or all");
        return ExitCode::from(2);
    };
    let inputs = match collect(paths) {
        Ok(inputs) => inputs,
        Err(e) => {
            eprintln!("cannot list inputs: {e}");
            return ExitCode::from(2);
        }
    };
    let mut failed = Vec::new();
    for path in &inputs {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("cannot read {}: {e}", path.display());
                return ExitCode::from(2);
            }
        };
        for &(name, drive) in &drivers {
            println!("{name} {}", path.display());
            let input = bytes.clone();
            let outcome = thread::Builder::new()
                .stack_size(SMALL_STACK)
                .spawn(move || drive(input))
                .expect("spawn small-stack thread")
                .join();
            if outcome.is_err() {
                failed.push(format!("{name} {}", path.display()));
            }
        }
    }
    println!(
        "replayed {} inputs x {} drivers on {} KiB stacks: {} panicked",
        inputs.len(),
        drivers.len(),
        SMALL_STACK / 1024,
        failed.len()
    );
    for f in &failed {
        println!("PANIC {f}");
    }
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
