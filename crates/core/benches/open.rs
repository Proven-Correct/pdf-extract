//! P1 benchmark (spec T18, §4.8 "Benchmarks"): wall-clock of
//! `Document::open` + `page_count` on 100k- and 1M-page files, and of
//! `max_pages = 10` rejecting the 1M file, plus each case's peak RSS.
//!
//! REPORT-ONLY. The CI `bench` job runs this with `continue-on-error` and
//! compares against `benches/baseline.json` with annotations; nothing here
//! gates. The correctness side of P1 (no pages built at open, bounded node
//! resolutions, bounded xref memory) is asserted by tests instead.
//!
//! Peak RSS is a per-process high-water mark, so each case is measured in a
//! fresh child process (this same binary, re-run with
//! `PDF_EXTRACT_BENCH_CHILD` set) that opens the file once and reports
//! `getrusage`'s `ru_maxrss`. The parent writes the results as JSON to
//! `$PDF_EXTRACT_BENCH_RSS_OUT` (default `bench-rss.json` in the working
//! directory), where `tools/bench/report.py` picks them up.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use criterion::Criterion;
use pdf_extract_core::{Document, OpenError, OpenOptions};
use pdf_extract_testkit::hostile::big_tree_to_path;

const CHILD_ENV: &str = "PDF_EXTRACT_BENCH_CHILD";

/// One benchmarked case: a file and the `max_pages` to open it with.
struct Case {
    name: &'static str,
    leaves: usize,
    max_pages: Option<usize>,
}

const CASES: [Case; 3] = [
    Case {
        name: "open_page_count/100k",
        leaves: 100_000,
        max_pages: None,
    },
    Case {
        name: "open_page_count/1m",
        leaves: 1_000_000,
        max_pages: None,
    },
    Case {
        name: "open_max_pages_10/1m",
        leaves: 1_000_000,
        max_pages: Some(10),
    },
];

/// Opens `path` as the case says and returns the page count (or the
/// `max_pages` error's lower bound), so the work cannot be optimised away.
fn run(path: &Path, max_pages: Option<usize>) -> usize {
    let opts = OpenOptions {
        max_pages,
        ..OpenOptions::default()
    };
    match Document::open(path, &opts) {
        Ok(doc) => doc.page_count(),
        Err(OpenError::TooManyPages { at_least, .. }) if max_pages.is_some() => at_least,
        Err(e) => panic!("{}: {e}", path.display()),
    }
}

/// Peak resident set size of this process, in bytes.
fn peak_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` writes a complete `rusage` into the pointer it is
    // given when it returns 0; the pointer is to a live, properly aligned,
    // zero-initialised `rusage` that outlives the call. On failure the
    // zeroed value is never read.
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc != 0 {
        return 0;
    }
    // SAFETY: rc == 0, so the kernel filled the struct; zeroed is a valid
    // `rusage` in any case (all fields are integers).
    let maxrss = unsafe { usage.assume_init() }.ru_maxrss;
    let maxrss = u64::try_from(maxrss).unwrap_or(0);
    // Linux reports KiB, macOS bytes.
    if cfg!(target_os = "macos") {
        maxrss
    } else {
        maxrss * 1024
    }
}

/// The generated input files, in a per-run temp directory removed on drop.
struct Inputs {
    dir: PathBuf,
}

impl Inputs {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("pdf-extract-bench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create bench dir");
        for leaves in [100_000, 1_000_000] {
            let path = Self::path_in(&dir, leaves);
            big_tree_to_path(leaves, &path).expect("generate bench input");
        }
        Self { dir }
    }

    fn path_in(dir: &Path, leaves: usize) -> PathBuf {
        dir.join(format!("tree_{leaves}.pdf"))
    }

    fn path(&self, leaves: usize) -> PathBuf {
        Self::path_in(&self.dir, leaves)
    }
}

impl Drop for Inputs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn bench_open(c: &mut Criterion, inputs: &Inputs) {
    let mut group = c.benchmark_group("p1");
    // A 1M-page open takes on the order of a second; ten samples keep the
    // whole bench within a few minutes on a CI runner.
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    for case in &CASES {
        let path = inputs.path(case.leaves);
        group.bench_function(case.name, |b| b.iter(|| run(&path, case.max_pages)));
    }
    group.finish();
}

/// Measures each case's peak RSS in a child process and writes the JSON.
fn report_rss(inputs: &Inputs) {
    let exe = std::env::current_exe().expect("own path");
    let mut entries = Vec::new();
    for case in &CASES {
        let path = inputs.path(case.leaves);
        let arg = format!(
            "{}|{}",
            path.display(),
            case.max_pages.map_or_else(String::new, |n| n.to_string())
        );
        let out = Command::new(&exe)
            .env(CHILD_ENV, arg)
            .output()
            .expect("run child");
        let rss = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let file_bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
        println!(
            "peak_rss {}: {rss} bytes (file {file_bytes} bytes)",
            case.name
        );
        entries.push(format!(
            "    \"{}\": {{ \"peak_rss_bytes\": {}, \"file_bytes\": {file_bytes} }}",
            case.name,
            rss.parse::<u64>().unwrap_or(0)
        ));
    }
    let json = format!("{{\n{}\n}}\n", entries.join(",\n"));
    let target = std::env::var_os("PDF_EXTRACT_BENCH_RSS_OUT")
        .map_or_else(|| PathBuf::from("bench-rss.json"), PathBuf::from);
    std::fs::write(&target, json).expect("write RSS report");
}

/// Child mode: open once, print peak RSS in bytes.
fn child(arg: &str) {
    let (path, max) = arg.split_once('|').expect("path|max_pages");
    let max_pages = (!max.is_empty()).then(|| max.parse().expect("max_pages"));
    let n = run(Path::new(path), max_pages);
    assert!(n > 0);
    println!("{}", peak_rss_bytes());
}

fn main() {
    if let Ok(arg) = std::env::var(CHILD_ENV) {
        child(&arg);
        return;
    }
    let inputs = Inputs::new();
    let mut c = Criterion::default().configure_from_args();
    bench_open(&mut c, &inputs);
    c.final_summary();
    report_rss(&inputs);
}
