//! Replay of the committed fuzz regression set (design D7; spec "Fixed
//! findings leave a committed reproducer", "The fuzz corpus is replayed on
//! 512 KiB thread stacks").
//!
//! Every file under `fuzz/regressions/{open,text,cmap,unicode}/` goes through
//! the same `fuzz_api` driver and limits as its libFuzzer target, on a
//! measured 512 KiB stack, on stable and without libFuzzer, so a fixed
//! finding stays fixed in the per-push suite. The `render` set is replayed by
//! `pdf-extract-render`'s copy of this test, the crate that owns that entry
//! point.
//!
//! A panic fails the test with the offending file named; a stack overflow
//! aborts the test process, which fails it too.

mod common;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Once;

use common::try_on_small_stack;
use pdf_extract_core::fuzz_api;

thread_local! {
    /// Panics raised on this thread, caught or not (see [`count_panics`]).
    static PANICS: Cell<usize> = const { Cell::new(0) };
}

/// Installs, once, a panic hook that counts every panic on the panicking
/// thread before the previous hook runs. A panic the library catches itself
/// (the codec boundary, patch 0028, which only `--cfg fuzzing` compiles out)
/// still runs the hook, so the replay sees it although the driver returns
/// normally: a reproducer must end without any panic, caught or not.
fn count_panics() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            PANICS.with(|p| p.set(p.get() + 1));
            previous(info);
        }));
    });
}

/// The committed regression set of `target`.
fn regression_dir(target: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/regressions")
        .join(target)
}

/// The reproducers in `dir`, sorted; dotfiles and the README are not inputs.
/// A missing directory is an empty set (git keeps no empty directories).
fn inputs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .map(|e| e.expect("directory entry reads").path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            !name.starts_with('.') && !name.starts_with("README")
        })
        .collect();
    files.sort();
    files
}

/// Runs `drive` over every input in `dir` on a 512 KiB stack; returns how
/// many ran.
fn replay(dir: &Path, drive: fn(Vec<u8>)) -> usize {
    count_panics();
    let files = inputs(dir);
    for path in &files {
        let bytes = std::fs::read(path).expect("reproducer reads");
        let outcome = try_on_small_stack(move || {
            drive(bytes);
            PANICS.with(Cell::get)
        });
        match outcome {
            Ok(0) => {}
            Ok(_) => panic!(
                "{} panicked on replay (caught inside the library)",
                path.display()
            ),
            Err(_) => panic!("{} panicked on replay", path.display()),
        }
    }
    files.len()
}

fn cmap(bytes: Vec<u8>) {
    fuzz_api::parse_cmap(&bytes);
}

fn unicode(bytes: Vec<u8>) {
    fuzz_api::unicode_fallbacks(&bytes);
}

/// A target's name and driver.
type Target = (&'static str, fn(Vec<u8>));

/// The targets this crate replays, with their drivers.
const TARGETS: [Target; 4] = [
    ("open", fuzz_api::open_and_count),
    ("text", fuzz_api::text_sampled_pages),
    ("cmap", cmap),
    ("unicode", unicode),
];

#[test]
fn committed_regressions_replay_without_panic() {
    assert!(
        regression_dir("").join("README.md").is_file(),
        "fuzz/regressions/README.md is missing: the replay would pass vacuously on a moved set"
    );
    for (target, drive) in TARGETS {
        let n = replay(&regression_dir(target), drive);
        eprintln!("fuzz regressions: {target}: {n} replayed");
    }
}

#[test]
fn replay_runs_every_input_of_a_directory() {
    // The committed set may be empty, so prove the machinery on a scratch
    // set with one input per target, a README and a dotfile to skip.
    let dir = std::env::temp_dir().join(format!("pxe-fuzz-replay-{}", std::process::id()));
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/corpus-c/enc_aes_128_owner.pdf");
    for (target, drive) in TARGETS {
        let sub = dir.join(target);
        std::fs::create_dir_all(&sub).expect("scratch dir");
        std::fs::copy(&fixture, sub.join("sample")).expect("sample copies");
        std::fs::write(sub.join("README.md"), "not an input").expect("readme");
        std::fs::write(sub.join(".gitkeep"), "").expect("dotfile");
        assert_eq!(replay(&sub, drive), 1, "{target}");
    }
    assert!(inputs(&dir.join("absent")).is_empty());

    // A panic the driver catches itself, as the codec boundary does outside
    // `cfg(fuzzing)`, still fails the replay; so does one that escapes.
    fn caught(_: Vec<u8>) {
        let _ = std::panic::catch_unwind(|| panic!("planted, caught"));
    }
    fn escaping(_: Vec<u8>) {
        panic!("planted, escaping");
    }
    for drive in [caught as fn(Vec<u8>), escaping] {
        let failed = std::panic::catch_unwind(|| replay(&dir.join("open"), drive));
        assert!(failed.is_err(), "a planted panic passed the replay");
    }
    std::fs::remove_dir_all(&dir).expect("scratch dir removed");
}
