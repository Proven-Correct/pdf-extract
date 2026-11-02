// Behaviour of `Instance` against the real Tesseract (spec 0002 §4.6): Init
// success and failure, recognition of a known image, the deadline mapping,
// the exception path, argument checks and the result tree. Every shim entry
// point is reached from here or from the silence/concurrency tests
// (tests/shim_symbols.rs checks that).

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Gray, hello, noise, tessdata, tiled_hello, white};
#[cfg(debug_assertions)]
use pdf_extract_tesseract_sys::test_word_callbacks;
use pdf_extract_tesseract_sys::{
    Block, EngineMode, ErrorKind, Instance, LINKAGE, Linkage, PageSegMode, version,
};

static_assertions::assert_impl_all!(Instance: Send);
static_assertions::assert_not_impl_any!(Instance: Sync);

const MINUTE: Duration = Duration::from_secs(60);

fn eng() -> Instance {
    Instance::new(&tessdata(), "eng", EngineMode::LstmOnly).expect("Init eng")
}

fn load(inst: &mut Instance, img: &Gray) {
    inst.set_image(&img.data, img.width, img.height, img.width)
        .expect("set_image");
}

fn recognise(inst: &mut Instance, img: &Gray, ppi: u32) -> Vec<Block> {
    load(inst, img);
    inst.set_source_resolution(ppi).expect("resolution");
    inst.set_page_seg_mode(PageSegMode::Auto).expect("psm");
    inst.recognize(MINUTE).expect("recognize");
    let blocks = inst.blocks().expect("blocks");
    inst.clear();
    blocks
}

fn words(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .flat_map(|b| &b.lines)
        .flat_map(|l| &l.words)
        .map(|w| w.text.clone())
        .collect()
}

#[test]
fn version_is_the_linked_release() {
    let v = version();
    match LINKAGE {
        Linkage::Vendored => assert!(v.starts_with("5.5.3"), "{v}"),
        // Ubuntu 24.04 ships 5.3.4; the floor build.rs enforces is 5.3.
        Linkage::System => assert!(v.starts_with('5'), "{v}"),
    }
}

#[test]
fn init_with_the_test_model_succeeds() {
    drop(eng());
}

#[test]
fn init_with_variables_succeeds() {
    let inst = Instance::with_variables(
        &tessdata(),
        "eng",
        EngineMode::Default,
        &[("tessedit_do_invert", "0")],
    );
    assert!(inst.is_ok(), "{inst:?}");
}

/// Spec 0003 F18: process-global parameters (Tesseract's own answer) and
/// the per-instance parameters in the refused classes (files, training /
/// dump / box modes, model selection) are refused before anything is
/// initialised, naming the variable. The directory does not exist, so a
/// name that got through would fail differently (InitFailed from the shim).
#[test]
fn refused_class_variables_are_refused_before_init() {
    let missing = Path::new("/nonexistent/pdf-extract-tessdata");
    for (name, value) in [
        ("debug_file", ""),
        ("debug_file", "/tmp/pdf-extract-debug"),
        ("log_level", "0"),
        ("dotproduct", "native"),
        ("tessedit_write_images", "1"),
        ("tessedit_write_params_to_file", "x"),
        // Files named by or derived from the value (internal #65 A2).
        ("output_ambig_words_file", "/tmp/pdf-extract-ambigs"),
        ("user_words_file", "/etc/passwd"),
        ("user_patterns_file", "/etc/passwd"),
        ("user_words_suffix", "user-words"),
        ("user_patterns_suffix", "user-patterns"),
        // Training, dump and box modes (internal #65 A1).
        ("tessedit_dump_pageseg_images", "1"),
        ("textord_tabfind_show_vlines", "1"),
        ("classify_save_adapted_templates", "1"),
        // Appends to `<imagefile>.doc`; Tesseract aborts if it cannot open it.
        ("save_doc_words", "1"),
        ("tessedit_train_line_recognizer", "1"),
        ("tessedit_train_from_boxes", "1"),
        ("tessedit_ambigs_training", "1"),
        ("tessedit_resegment_from_boxes", "1"),
        ("tessedit_resegment_from_line_boxes", "1"),
        ("tessedit_make_boxes_from_boxes", "1"),
        // Which models Init loads (internal #65 A2, A3).
        ("tessedit_load_sublangs", "../../x"),
        ("tessedit_init_config_only", "1"),
        ("tessedit_ocr_engine_mode", "0"),
    ] {
        let err = Instance::with_variables(missing, "eng", EngineMode::LstmOnly, &[(name, value)])
            .expect_err(name);
        assert_eq!(err.kind, ErrorKind::InvalidArgument, "{name}: {err}");
        assert_eq!(err.code, None, "{name}: refused in Rust");
        assert!(err.message.contains(name), "{name}: {err}");
    }
    // A refused name anywhere in the list refuses the whole call.
    let err = Instance::with_variables(
        &tessdata(),
        "eng",
        EngineMode::LstmOnly,
        &[("tessedit_do_invert", "0"), ("debug_file", "")],
    )
    .expect_err("second variable");
    assert!(err.message.contains("process-global"), "{err}");
    // A per-instance debug toggle is passed through (0002 §3.4).
    Instance::with_variables(
        &tessdata(),
        "eng",
        EngineMode::LstmOnly,
        &[("tessedit_debug_quality_metrics", "0")],
    )
    .expect("per-instance debug variable");
}

#[test]
fn init_with_a_missing_directory_names_it() {
    let missing = Path::new("/nonexistent/pdf-extract-tessdata");
    let err = Instance::new(missing, "eng", EngineMode::LstmOnly).expect_err("must fail");
    assert_eq!(err.kind, ErrorKind::InitFailed);
    assert!(matches!(err.code, Some(c) if c != 0), "{err:?}");
    assert!(
        err.message.contains("/nonexistent/pdf-extract-tessdata"),
        "{err}"
    );
    assert!(err.to_string().contains("eng"), "{err}");
}

#[test]
fn init_with_a_missing_language_fails() {
    let err =
        Instance::new(&tessdata(), "zz_no_such_lang", EngineMode::LstmOnly).expect_err("must fail");
    assert_eq!(err.kind, ErrorKind::InitFailed);
}

#[cfg(feature = "vendored")]
#[test]
fn legacy_modes_are_refused_before_init() {
    for oem in [EngineMode::TesseractOnly, EngineMode::TesseractLstmCombined] {
        // A directory that would make Init fail differently proves the
        // refusal happens first.
        let err = Instance::new(Path::new("/nonexistent"), "eng", oem).expect_err("refused");
        assert_eq!(err.kind, ErrorKind::InitFailed);
        assert_eq!(err.code, None, "refused in Rust, before the shim");
        assert_eq!(err.message, "legacy engine not built");
    }
}

#[test]
fn bad_language_names_never_reach_tesseract() {
    // The last four: empty members, which Tesseract would skip or replace
    // with `eng` (spec 0003 F19).
    for langs in [
        "", "../eng", "eng/x", "e ng", "eng\0", "+", "eng+", "+eng", "eng++fra",
    ] {
        let err = Instance::new(&tessdata(), langs, EngineMode::LstmOnly).expect_err(langs);
        assert_eq!(err.kind, ErrorKind::InvalidArgument, "{langs:?}");
        assert_eq!(err.code, None);
    }
}

#[test]
fn nul_bytes_in_path_or_variables_are_refused() {
    let err = Instance::new(Path::new("/tmp/a\0b"), "eng", EngineMode::LstmOnly)
        .expect_err("NUL in path");
    assert_eq!(err.kind, ErrorKind::InvalidArgument);
    for vars in [[("a\0", "1")], [("a", "1\0")]] {
        let err = Instance::with_variables(&tessdata(), "eng", EngineMode::LstmOnly, &vars)
            .expect_err("NUL in variable");
        assert_eq!(err.kind, ErrorKind::InvalidArgument);
    }
}

/// The MacPorts `tesseract-eng` file is the combined (legacy + LSTM) 23.5 MB
/// model; a build without the legacy engine must still load its LSTM part.
/// Ignored rather than skipped when the file is absent (CI has only
/// `tessdata_fast`): a test that returns early would report a pass.
#[test]
#[ignore = "needs the MacPorts combined eng.traineddata in /opt/local/share/tessdata; run with --include-ignored"]
fn combined_model_loads_lstm_only() {
    let dir = Path::new("/opt/local/share/tessdata");
    assert!(
        dir.join("eng.traineddata").is_file(),
        "no MacPorts eng.traineddata in {}",
        dir.display()
    );
    let mut inst = Instance::new(dir, "eng", EngineMode::LstmOnly).expect("Init combined");
    assert_eq!(words(&recognise(&mut inst, &hello(), 300)), ["Hello"]);
}

#[test]
fn recognises_hello() {
    let img = hello();
    let mut inst = eng();
    let blocks = recognise(&mut inst, &img, 300);
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    assert_eq!(blocks[0].lines.len(), 1, "{blocks:?}");
    let line_words = &blocks[0].lines[0].words;
    assert_eq!(line_words.len(), 1, "{blocks:?}");
    let w = &line_words[0];
    assert_eq!(w.text, "Hello");
    let [l, t, r, b] = w.bbox;
    assert!(l < r && r <= img.width && t < b && b <= img.height, "{w:?}");
    assert!(w.confidence > 50.0, "{w:?}");
}

#[test]
fn an_instance_is_reusable_after_clear() {
    let mut inst = eng();
    let first = recognise(&mut inst, &hello(), 300);
    let second = recognise(&mut inst, &hello(), 300);
    assert_eq!(first, second);
    // After clear there is no image: recognising again is an argument error.
    let err = inst.recognize(MINUTE).expect_err("no image");
    assert_eq!(err.kind, ErrorKind::InvalidArgument);
    assert!(inst.blocks().expect("no results").is_empty());
}

#[test]
fn a_white_pixel_has_no_words() {
    let mut inst = eng();
    load(&mut inst, &white(1, 1));
    inst.set_source_resolution(300).expect("resolution");
    // pxt_recognize returned 0: an empty page is a success.
    inst.recognize(MINUTE).expect("empty page recognises");
    assert!(inst.blocks().expect("blocks").is_empty());
}

#[test]
fn several_lines_and_blocks_form_a_tree() {
    let mut inst = eng();
    let blocks = recognise(&mut inst, &tiled_hello(2, 3), 300);
    let lines: usize = blocks.iter().map(|b| b.lines.len()).sum();
    assert!(lines >= 3, "{blocks:?}");
    let all = words(&blocks);
    assert_eq!(all.len(), 6, "{all:?}");
    assert!(all.iter().all(|w| w == "Hello"), "{all:?}");
    assert!(
        blocks
            .iter()
            .all(|b| b.lines.iter().all(|l| !l.words.is_empty()))
    );
}

#[test]
fn every_page_seg_mode_is_accepted() {
    let mut inst = eng();
    load(&mut inst, &hello());
    for psm in [
        PageSegMode::OsdOnly,
        PageSegMode::AutoOsd,
        PageSegMode::AutoOnly,
        PageSegMode::Auto,
        PageSegMode::SingleColumn,
        PageSegMode::SingleBlockVertText,
        PageSegMode::SingleBlock,
        PageSegMode::SingleLine,
        PageSegMode::SingleWord,
        PageSegMode::CircleWord,
        PageSegMode::SingleChar,
        PageSegMode::SparseText,
        PageSegMode::SparseTextOsd,
        PageSegMode::RawLine,
    ] {
        inst.set_page_seg_mode(psm).expect("psm");
    }
    inst.set_source_resolution(300).expect("resolution");
    inst.set_page_seg_mode(PageSegMode::SingleLine)
        .expect("psm");
    inst.recognize(MINUTE).expect("single line");
    assert_eq!(words(&inst.blocks().expect("blocks")), ["Hello"]);
}

#[test]
fn image_arguments_are_checked() {
    let mut inst = eng();
    let bad = |r: Result<(), pdf_extract_tesseract_sys::Error>| {
        assert_eq!(r.expect_err("bad image").kind, ErrorKind::InvalidArgument)
    };
    bad(inst.set_image(&[], 0, 0, 0));
    bad(inst.set_image(&[0; 4], 2, 2, 1)); // stride shorter than width
    bad(inst.set_image(&[0; 3], 2, 2, 2)); // buffer too short
    bad(inst.set_image(&[0; 5], 2, 2, 2)); // buffer too long
    bad(inst.set_image(&[0; 4], u32::MAX, 1, u32::MAX));
    // No image yet: resolution has nothing to apply to.
    bad(inst.set_source_resolution(300));
    // A padded stride is honoured: the padding bytes are not pixels.
    let img = hello();
    let stride = img.width + 3;
    let mut padded = Vec::new();
    for row in img.data.chunks(img.width as usize) {
        padded.extend_from_slice(row);
        padded.extend_from_slice(&[0, 0, 0]);
    }
    inst.set_image(&padded, img.width, img.height, stride)
        .expect("padded");
    inst.set_source_resolution(300).expect("resolution");
    inst.recognize(MINUTE).expect("recognize");
    assert_eq!(words(&inst.blocks().expect("blocks")), ["Hello"]);
}

#[test]
fn an_image_leptonica_refuses_is_an_error_not_a_crash() {
    // 46341^2 >= 2^31 bytes: Leptonica's pixCreate would return null, which
    // Tesseract's own SetImage(bytes) writes through; the shim refuses the
    // size before calling it. The buffer is 2 GiB of address space, but
    // vec! of zeros is lazily committed by the OS and never touched.
    let side = 46_341u32;
    let data = vec![0u8; (side as usize) * (side as usize)];
    let mut inst = eng();
    let err = inst
        .set_image(&data, side, side, side)
        .expect_err("too big");
    assert_eq!(err.kind, ErrorKind::InvalidArgument);
    assert!(err.message.contains("Leptonica"), "{err}");
}

#[test]
fn zero_timeout_is_already_expired() {
    let mut inst = eng();
    load(&mut inst, &hello());
    let err = inst.recognize(Duration::ZERO).expect_err("expired");
    assert_eq!(err.kind, ErrorKind::Timeout);
    assert_eq!(err.code, None);
}

/// D-11: a -1 from Recognize with our clock past the deadline is a timeout.
/// A run that finishes inside 1 ms says nothing, so the image grows until
/// one run takes longer than the deadline; that run must be a timeout.
#[test]
fn deadline_maps_to_timeout() {
    let mut inst = eng();
    for (i, side) in [2000u32, 3000, 4000].into_iter().enumerate() {
        for img in [
            noise(side, side, 0x5eed + i as u64),
            tiled_hello(side / 750, side / 300),
        ] {
            load(&mut inst, &img);
            inst.set_source_resolution(300).expect("resolution");
            let start = Instant::now();
            let result = inst.recognize(Duration::from_millis(1));
            let elapsed = start.elapsed();
            inst.clear();
            if elapsed <= Duration::from_millis(1) {
                continue; // inconclusive: finished inside the deadline
            }
            eprintln!("side {side}: {elapsed:?}");
            let err = result.expect_err("exceeded the deadline");
            assert_eq!(err.kind, ErrorKind::Timeout, "{err}");
            assert_eq!(err.code, Some(4));
            assert!(err.message.contains("deadline"), "{err}");
            return;
        }
    }
    panic!("no image took longer than 1 ms to recognise");
}

#[cfg(debug_assertions)]
#[test]
fn a_cxx_exception_becomes_an_error() {
    let err = pdf_extract_tesseract_sys::test_throw().expect_err("thrown");
    assert_eq!(err.kind, ErrorKind::Exception);
    assert_eq!(err.code, Some(2));
    assert_eq!(err.message, "pxt_test_throw: deliberate exception");
    assert_eq!(
        err.to_string(),
        "tesseract: pxt_test_throw: deliberate exception"
    );
}

/// 0001 D-16: a panic in the word callback is caught before it can unwind into
/// the shim, the walk stops, and `blocks` re-raises it with its payload.
#[cfg(debug_assertions)]
#[test]
fn a_callback_panic_reaches_the_caller() {
    // Six words, so a walk that went on after the panic would show as more
    // than one callback.
    let mut inst = eng();
    load(&mut inst, &tiled_hello(2, 3));
    inst.set_source_resolution(300).expect("resolution");
    inst.set_page_seg_mode(PageSegMode::Auto).expect("psm");
    inst.recognize(MINUTE).expect("recognize");
    let before = test_word_callbacks();
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        inst.test_blocks_panicking()
    }))
    .expect_err("the callback's panic propagates");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"on_word: deliberate panic")
    );
    assert_eq!(
        test_word_callbacks() - before,
        1,
        "the walk went on after the callback panicked"
    );
    // The handle survived: the same results collect normally afterwards, and
    // a walk that is not stopped makes one callback per word.
    let before = test_word_callbacks();
    assert_eq!(words(&inst.blocks().expect("blocks")), ["Hello"; 6]);
    assert_eq!(test_word_callbacks() - before, 6);
}
