//! Unit tests of the close protocol (spec §4.8 "Close protocol") and of page
//! operations, which need the crate-internal operation hook
//! (`Page::with_loaded`): an extraction paused inside an operation while
//! `close()` runs, the weak-pointer check that `Loaded` is freed, and the
//! "text extracted" half of the encryption contract row (until `Page::text`
//! exists in unit 10, a minimal collecting device stands in for it).

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Weak};
use std::thread;
use std::time::Duration;

use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use kurbo::{Affine, BezPath, Rect};
use pdf_extract_testkit::PdfBuilder;

use super::*;

/// Records the Unicode of every glyph hayro draws.
#[derive(Default)]
struct Collect(String);

impl<'a> Device<'a> for Collect {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(
        &mut self,
        g: &Glyph<'a>,
        _: Affine,
        _: Affine,
        _: &Paint<'a>,
        _: &GlyphDrawMode,
    ) {
        match g.as_unicode() {
            Some(BfString::Char(c)) => self.0.push(c),
            Some(BfString::String(s)) => self.0.push_str(&s),
            None => self.0.push(char::REPLACEMENT_CHARACTER),
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The glyph text of page `index` of `loaded`, built the T17 way: the hayro
/// page exists only on this stack frame.
fn glyph_text(loaded: &Loaded, index: usize) -> String {
    let page = loaded
        .index
        .hayro_page(loaded.pdf.xref(), index)
        .expect("page builds");
    let cache = InterpreterCache::new();
    let mut ctx = Context::new(
        Affine::IDENTITY,
        Rect::new(-1e5, -1e5, 1e5, 1e5),
        &cache,
        loaded.pdf.xref(),
        InterpreterSettings::default(),
    );
    let mut out = Collect::default();
    interpret_page(&page, &mut ctx, &mut out);
    out.0
}

fn two_pages() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .raw_content("BT /Helv 11 Tf 72 720 Td (First) Tj ET");
    b.add_page_a4()
        .raw_content("BT /Helv 11 Tf 72 720 Td (Second) Tj ET");
    b.to_bytes()
}

fn doc() -> Document {
    Document::from_bytes(two_pages(), &OpenOptions::default()).expect("opens")
}

/// A weak pointer to the document's `Loaded`, for checking when it is freed.
fn weak_loaded(doc: &Document) -> Weak<Loaded> {
    Arc::downgrade(&doc.0.loaded().expect("open"))
}

#[test]
fn operations_see_the_right_page() {
    let doc = doc();
    let texts: Vec<String> = doc
        .pages()
        .map(|p| p.with_loaded(glyph_text).expect("open"))
        .collect();
    assert_eq!(texts, vec!["First", "Second"]);
}

/// Spec §4.8, encryption row: owner-password documents open and their text
/// is extracted (through hayro's decryption, transparently).
#[test]
fn owner_password_documents_extract_text() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/corpus-c");
    for alg in ["rc4_40", "rc4_128", "aes_128", "aes_256"] {
        let doc = Document::open(
            dir.join(format!("enc_{alg}_owner.pdf")),
            &OpenOptions::default(),
        )
        .expect("owner fixture opens");
        assert!(doc.is_encrypted());
        let text = doc
            .page(0)
            .expect("page 0")
            .with_loaded(glyph_text)
            .expect("open");
        assert_eq!(text, format!("Encrypted {alg} owner"), "{alg}");
    }
}

#[test]
fn close_then_use() {
    let doc = doc();
    let page = doc.page(1).expect("page");
    let warnings = doc.warnings();
    doc.close();
    assert!(doc.is_closed());
    assert_eq!(doc.page(0).expect_err("closed"), PageError::Closed);
    // Facts survive close.
    assert_eq!(doc.page_count(), 2);
    assert!(!doc.is_encrypted());
    assert_eq!(doc.warnings(), warnings);
    // Existing handles keep their index; operations on them fail.
    assert_eq!(page.index(), 1);
    assert_eq!(page.with_loaded(|_, i| i), Err(PageError::Closed));
    // pages() still yields handles, all closed.
    let handles: Vec<Page> = doc.pages().collect();
    assert_eq!(handles.len(), 2);
    assert!(handles.iter().all(|p| p.with_loaded(|_, _| ()).is_err()));
    // Idempotent; a clone sees the same state.
    doc.close();
    assert!(doc.clone().is_closed());
}

#[test]
fn close_frees_loaded_when_nothing_is_running() {
    let doc = doc();
    let page = doc.page(0).expect("page");
    let weak = weak_loaded(&doc);
    assert!(weak.upgrade().is_some());
    doc.close();
    // Page handles hold the shared facts, not the parsed document.
    assert!(weak.upgrade().is_none());
    drop(page);
}

#[test]
fn dropping_the_last_clone_frees_loaded() {
    let doc = doc();
    let clone = doc.clone();
    let page = doc.page(0).expect("page");
    let weak = weak_loaded(&doc);
    drop(doc);
    assert!(weak.upgrade().is_some(), "a clone is still alive");
    drop(clone);
    assert!(weak.upgrade().is_some(), "a page handle is still alive");
    drop(page);
    assert!(weak.upgrade().is_none());
}

/// An operation started before `close()` completes normally, holding the
/// parsed document alive until it ends; `close()` does not wait for it.
///
/// Every wait has a watchdog, so a regression that holds the state lock
/// across the operation (which would make `close()` block on the paused
/// worker, a deadlock) fails the test instead of hanging it.
#[test]
fn close_during_an_operation() {
    const WATCHDOG: Duration = Duration::from_secs(30);
    let doc = doc();
    let page = doc.page(0).expect("page");
    let weak = weak_loaded(&doc);
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = page.with_loaded(|loaded, index| {
            entered_tx.send(()).expect("main thread waits");
            // The test hook: pause inside the operation until told to go.
            go_rx
                .recv_timeout(WATCHDOG)
                .expect("main thread signals in time");
            glyph_text(loaded, index)
        });
        let _ = done_tx.send(result);
    });
    entered_rx
        .recv_timeout(WATCHDOG)
        .expect("worker entered the operation");

    // close() on its own thread so a blocking close trips the watchdog.
    let (closed_tx, closed_rx) = mpsc::channel();
    let closer = doc.clone();
    thread::spawn(move || {
        closer.close();
        let _ = closed_tx.send(());
    });
    closed_rx
        .recv_timeout(WATCHDOG)
        .expect("close() must not wait for an in-flight operation");
    assert!(doc.is_closed());
    assert_eq!(doc.page(0).expect_err("closed"), PageError::Closed);
    assert!(weak.upgrade().is_some(), "in-flight operation keeps Loaded");

    go_tx.send(()).expect("worker waits");
    let text = done_rx
        .recv_timeout(WATCHDOG)
        .expect("operation finishes")
        .expect("started before close");
    assert_eq!(text, "First");
    // The worker drops its Arc<Loaded> just before sending; allow for the
    // thread to finish unwinding its frame.
    let deadline = std::time::Instant::now() + WATCHDOG;
    while weak.upgrade().is_some() {
        assert!(std::time::Instant::now() < deadline, "Loaded never freed");
        thread::yield_now();
    }
}

/// A content stream whose xref offset is wrong opens without repair (open
/// never reads `/Contents`); the first operation that reads it repairs the
/// table lazily and still gets the right text. (The concurrent case is
/// tested at the hayro level by `tests/h9_xref_repair.rs` and through the
/// shared `Document` by unit 10, spec §4.10.)
#[test]
fn lazy_repair_during_an_operation() {
    let bytes = pdf_extract_testkit::contract::misoffset_object(10, 6, 10);
    let doc = Document::from_bytes(bytes, &OpenOptions::default()).expect("opens");
    assert!(doc.warnings().is_empty(), "{:?}", doc.warnings());
    let loaded = doc.0.loaded().expect("open");
    assert!(!loaded.pdf.xref_was_repaired());
    let text = doc
        .page(6)
        .expect("page")
        .with_loaded(glyph_text)
        .expect("open");
    assert_eq!(text, "Page 7");
    assert!(loaded.pdf.xref_was_repaired());
    for i in 0..10 {
        let t = doc
            .page(i)
            .expect("page")
            .with_loaded(glyph_text)
            .expect("open");
        assert_eq!(t, format!("Page {}", i + 1));
    }
}

#[test]
fn from_shared_does_not_copy() {
    let bytes = Arc::new(two_pages());
    let doc = Document::from_shared(bytes.clone(), &OpenOptions::default()).expect("opens");
    let loaded = doc.0.loaded().expect("open");
    assert!(Arc::ptr_eq(&loaded.source, &bytes));
    assert_eq!(doc.0.limits, Limits::default());
}

#[test]
fn pages_iterator_is_exact_and_fused() {
    let doc = doc();
    let mut it = doc.pages();
    assert_eq!(it.len(), 2);
    assert_eq!(it.size_hint(), (2, Some(2)));
    assert_eq!(it.next().map(|p| p.index()), Some(0));
    assert_eq!(it.len(), 1);
    assert_eq!(it.next().map(|p| p.index()), Some(1));
    assert!(it.next().is_none());
    assert!(it.next().is_none());
    assert_eq!(it.len(), 0);
    assert!(format!("{it:?}").contains("remaining: 0"));
}

/// One `Pages` shared by reference between threads hands out every index
/// exactly once (what the Python iterator relies on).
#[test]
fn shared_pages_iterator_hands_out_each_index_once() {
    let mut b = PdfBuilder::new();
    for _ in 0..500 {
        b.add_page(10.0, 10.0);
    }
    let doc = Document::from_bytes(b.to_bytes(), &OpenOptions::default()).expect("opens");
    let pages = doc.pages();
    let mut seen: Vec<usize> = thread::scope(|s| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                s.spawn(|| {
                    let mut got = Vec::new();
                    while let Some(p) = pages.next_shared() {
                        got.push(p.index());
                    }
                    got
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("no panic"))
            .collect()
    });
    seen.sort_unstable();
    assert_eq!(seen, (0..500).collect::<Vec<_>>());
}

#[test]
fn debug_output_is_informative() {
    let doc = doc();
    let s = format!("{doc:?}");
    assert!(
        s.contains("page_count: 2") && s.contains("closed: false"),
        "{s}"
    );
    let p = format!("{:?}", doc.page(1).expect("page"));
    assert!(p.contains("index: 1"), "{p}");
}

/// §4.8 close-during-use loop with real page operations: 1,000 rounds of
/// workers interpreting pages while another thread closes the document.
/// Every operation returns the right text or `Err(Closed)`; none panics.
#[test]
fn close_during_operations() {
    let bytes = Arc::new(two_pages());
    for round in 0..1000 {
        let doc = Document::from_shared(bytes.clone(), &OpenOptions::default()).expect("opens");
        let pages: Vec<Page> = doc.pages().collect();
        let start = std::sync::Barrier::new(4);
        thread::scope(|s| {
            for w in 0..3 {
                let (pages, start) = (&pages, &start);
                s.spawn(move || {
                    start.wait();
                    for k in 0..4 {
                        let page = &pages[(k + w) % 2];
                        match page.with_loaded(glyph_text) {
                            Ok(t) => assert_eq!(t, ["First", "Second"][page.index()]),
                            Err(e) => assert_eq!(e, PageError::Closed, "round {round}"),
                        }
                    }
                });
            }
            start.wait();
            for _ in 0..(round % 40) * 50 {
                std::hint::spin_loop();
            }
            doc.close();
        });
        assert!(pages.iter().all(|p| p.with_loaded(|_, _| ()).is_err()));
    }
}

/// T9/T17/T18: opening never builds hayro's page list, and neither does a
/// page operation; pages are rebuilt from the index on demand.
#[test]
fn open_and_operations_never_build_hayro_pages() {
    let bytes = pdf_extract_testkit::hostile::page_tree(
        pdf_extract_testkit::hostile::TreeShape::Flat(1000),
    );
    let doc = Document::from_bytes(bytes, &OpenOptions::default()).expect("opens");
    assert_eq!(doc.page_count(), 1000);
    let loaded = doc.0.loaded().expect("open");
    assert!(!loaded.pdf.pages_built());
    let text = doc
        .page(999)
        .expect("page")
        .with_loaded(glyph_text)
        .expect("open");
    assert_eq!(text, "Page 1000");
    assert!(!loaded.pdf.pages_built());
}
