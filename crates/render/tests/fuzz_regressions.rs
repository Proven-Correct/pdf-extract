//! Replay of the committed `render` fuzz regression set (design D7; spec
//! "Fixed findings leave a committed reproducer"). The other targets' sets are
//! replayed by `pdf-extract-core`'s `tests/fuzz_regressions.rs`; this crate
//! owns the render entry point.
//!
//! [`render_page0`] must stay the same as `render_page0` in `fuzz/src/lib.rs`,
//! the driver of the `render` libFuzzer target: same open options, same page,
//! same render options. It is repeated rather than shared because core, which
//! holds the other drivers, cannot depend on this crate, and the `fuzz/`
//! crate is a nightly-only workspace this suite cannot build.

mod common;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Once;

use common::stack::try_on_small_stack;
use pdf_extract_core::Document;
use pdf_extract_core::fuzz_api::{FUZZ_MAX_RENDER_PIXELS, FUZZ_RENDER_DPI, fuzz_open_options};
use pdf_extract_render::{RenderOptions, render_gray};

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

/// The `render` target's driver: page 0 at 36 dpi under the fuzzing limits.
fn render_page0(bytes: Vec<u8>) {
    let Ok(doc) = Document::from_bytes(bytes, &fuzz_open_options()) else {
        return;
    };
    let Ok(page) = doc.page(0) else {
        return;
    };
    let opts = RenderOptions {
        dpi: FUZZ_RENDER_DPI,
        max_render_pixels: FUZZ_MAX_RENDER_PIXELS,
        render_annotations: true,
    };
    let _ = render_gray(&page, &opts);
}

/// The reproducers in `dir`, sorted; dotfiles and the README are not inputs.
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

/// Renders every input in `dir` on a 512 KiB stack; returns how many ran.
fn replay(dir: &Path) -> usize {
    count_panics();
    let files = inputs(dir);
    for path in &files {
        let bytes = std::fs::read(path).expect("reproducer reads");
        let outcome = try_on_small_stack(move || {
            render_page0(bytes);
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

#[test]
fn committed_render_regressions_replay_without_panic() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/regressions");
    assert!(
        root.join("README.md").is_file(),
        "fuzz/regressions/README.md is missing: the replay would pass vacuously on a moved set"
    );
    let n = replay(&root.join("render"));
    eprintln!("fuzz regressions: render: {n} replayed");
}

#[test]
fn replay_renders_every_input_of_a_directory() {
    // The committed set may be empty, so prove the machinery on a scratch
    // set: a real page, a non-PDF, a README and a dotfile to skip.
    let dir = std::env::temp_dir().join(format!("pxe-fuzz-render-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let fixture = common::fixtures_dir().join("corpus-c/enc_aes_128_owner.pdf");
    std::fs::copy(fixture, dir.join("page")).expect("sample copies");
    std::fs::write(dir.join("garbage"), b"%PDF-1.7 truncated").expect("garbage");
    std::fs::write(dir.join("README.md"), "not an input").expect("readme");
    std::fs::write(dir.join(".gitkeep"), "").expect("dotfile");
    assert_eq!(replay(&dir), 2);
    assert!(inputs(&dir.join("absent")).is_empty());
    std::fs::remove_dir_all(&dir).expect("scratch dir removed");
}
