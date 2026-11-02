//! Concurrency tests of spec §4.8 and §4.10 (T38-T41, C1-C2): one shared
//! `Document` used from many threads gives exactly the sequential results,
//! for the handle API (`page_count`, `page()`, `pages()`, `warnings()`,
//! `close()`) and for text extraction byte for byte, including while hayro
//! repairs a broken xref lazily under the threads' feet (C2 under repair,
//! needs fork change H9).
//!
//! Every test is deterministic: threads start together at a barrier, but the
//! assertions never depend on which thread wins a race, only on every
//! thread's output equalling the sequential output. The close-during-use loop
//! that pauses inside a page operation needs the crate-internal operation
//! hook and lives in `src/document_tests.rs`.

use std::sync::Barrier;
use std::thread;

use pdf_extract_core::{Document, OpenOptions, PageError, TextOptions, WarningKind};
use pdf_extract_testkit::hostile::{self, TreeShape};
use pdf_extract_testkit::{PdfBuilder, TextStyle, contract};

fn forty_pages() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    for i in 0..40 {
        b.add_page_a4()
            .raw_content(&format!("BT /Helv 11 Tf 72 720 Td (Page {i}) Tj ET"));
    }
    b.to_bytes()
}

/// A 40-page document with more on each page than a label: several lines,
/// two blocks, a form, and a page-specific amount of text, so pages differ
/// and a mix-up between threads would show.
fn forty_text_pages() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    for i in 0..40 {
        let p = b.add_page_a4();
        p.insert_text(
            (72.0, 72.0),
            &format!("Page {i} heading"),
            TextStyle::size(16.0),
        )
        .expect("text");
        for line in 0..(i % 7 + 1) {
            let y = 100.0 + 14.0 * line as f32;
            p.insert_text(
                (72.0, y),
                &format!("Line {line} of page {i}, words  apart"),
                TextStyle::default(),
            )
            .expect("text");
        }
        p.add_form_xobject(
            "Fm",
            [0.0, 0.0, 595.0, 842.0],
            &format!("BT /Helv 9 Tf 72 100 Td (Footer {i}) Tj ET"),
        )
        .raw_content("q /Fm Do Q");
    }
    b.to_bytes()
}

/// Text of every page of `doc`, in order.
fn all_text(doc: &Document) -> Vec<String> {
    doc.pages()
        .map(|p| p.text(&TextOptions::default()).expect("page extracts"))
        .collect()
}

/// Text and warnings of page `k % page_count`, for `calls` values of `k`
/// from `first`. Warnings are compared too (T39): in particular a lazy xref
/// repair must never show up in whichever page happened to trigger it.
fn texts_from(doc: &Document, first: usize, calls: usize) -> Vec<(usize, String)> {
    let n = doc.page_count();
    (first..first + calls)
        .map(|k| {
            let i = k % n;
            let tp = doc
                .page(i)
                .expect("in range")
                .text_page(&TextOptions::default())
                .expect("extracts");
            // The repair belongs to the document (T8): never in a page.
            assert!(
                !tp.warnings()
                    .iter()
                    .any(|w| w.kind == WarningKind::XrefRepaired),
                "page {i}: {:?}",
                tp.warnings()
            );
            assert!(tp.warnings().is_empty(), "page {i}: {:?}", tp.warnings());
            (i, tp.to_text(Default::default()))
        })
        .collect()
}

/// What one thread observes of a document.
fn observe(doc: &Document, calls: usize) -> Vec<(usize, usize)> {
    (0..calls)
        .map(|k| {
            let i = k % 41; // includes one out-of-range index
            let got = match doc.page(i) {
                Ok(p) => p.index(),
                Err(PageError::IndexOutOfRange { index, count }) => index + count * 1000,
                Err(e) => panic!("unexpected {e}"),
            };
            (doc.page_count(), got)
        })
        .collect()
}

/// 16 threads × 200 calls on one shared 40-page document equal the
/// sequential results.
#[test]
fn shared_document_matches_sequential() {
    let doc = Document::from_bytes(forty_pages(), &OpenOptions::default()).expect("opens");
    let expected = observe(&doc, 200);
    let start = Barrier::new(16);
    thread::scope(|s| {
        let workers: Vec<_> = (0..16)
            .map(|_| {
                s.spawn(|| {
                    start.wait();
                    observe(&doc, 200)
                })
            })
            .collect();
        for w in workers {
            assert_eq!(w.join().expect("no panic"), expected);
        }
    });
}

/// 8 threads opening separate documents at once get what one thread gets.
#[test]
fn parallel_opens_match_sequential() {
    let inputs: Vec<Vec<u8>> = vec![
        forty_pages(),
        hostile::page_tree(TreeShape::Flat(300)),
        hostile::page_tree(TreeShape::Fanout {
            leaves_per_node: 6,
            levels: 3,
        }),
        hostile::page_tree(TreeShape::Cycle),
        hostile::page_tree(TreeShape::DuplicateLeaf),
        hostile::page_tree(TreeShape::LyingCount { claimed: 3 }),
        pdf_extract_testkit::contract::broken_startxref(),
        hostile::deep_page_tree(300),
    ];
    let summary = |bytes: Vec<u8>| {
        let doc = Document::from_bytes(bytes, &OpenOptions::default()).expect("opens");
        let warnings: Vec<String> = doc.warnings().iter().map(ToString::to_string).collect();
        (doc.page_count(), warnings)
    };
    let expected: Vec<_> = inputs.iter().cloned().map(summary).collect();
    let start = Barrier::new(inputs.len());
    let got: Vec<_> = thread::scope(|s| {
        let workers: Vec<_> = inputs
            .iter()
            .cloned()
            .map(|bytes| {
                let start = &start;
                s.spawn(move || {
                    start.wait();
                    summary(bytes)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().expect("no panic"))
            .collect()
    });
    assert_eq!(got, expected);
}

/// 1,000 rounds: workers hammer `page()` and `pages()` while another thread
/// closes the document at a varying moment. Every call returns `Ok` or
/// `Err(Closed)`; facts stay readable; nothing panics.
#[test]
fn close_during_use() {
    let bytes = forty_pages();
    for round in 0..1000 {
        let doc = Document::from_bytes(bytes.clone(), &OpenOptions::default()).expect("opens");
        let start = Barrier::new(5);
        thread::scope(|s| {
            for w in 0..4 {
                let (doc, start) = (&doc, &start);
                s.spawn(move || {
                    start.wait();
                    for k in 0..50 {
                        match doc.page((k + w) % 40) {
                            Ok(p) => assert_eq!(p.index(), (k + w) % 40),
                            Err(PageError::Closed) => assert!(doc.is_closed()),
                            Err(e) => panic!("round {round}: unexpected {e}"),
                        }
                        assert_eq!(doc.page_count(), 40);
                        assert_eq!(doc.pages().len(), 40);
                    }
                });
            }
            start.wait();
            // Close after a round-dependent amount of spinning so the close
            // lands at different points of the workers' loops.
            for _ in 0..(round % 50) * 20 {
                std::hint::spin_loop();
            }
            doc.close();
        });
        assert!(doc.is_closed());
        assert_eq!(doc.page(0).expect_err("closed"), PageError::Closed);
    }
}

/// Spec §4.8/§4.10: 16 threads × 200 `text()` calls on one shared 40-page
/// document equal the sequential output byte for byte. Each thread starts at
/// a different page so that different pages run at the same time too.
#[test]
fn shared_document_text_matches_sequential() {
    let doc = Document::from_bytes(forty_text_pages(), &OpenOptions::default()).expect("opens");
    let sequential = all_text(&doc);
    assert!(
        sequential.iter().all(|t| t.starts_with("Page ")),
        "{sequential:?}"
    );
    let start = Barrier::new(16);
    thread::scope(|s| {
        let workers: Vec<_> = (0..16)
            .map(|t| {
                let (doc, start) = (&doc, &start);
                s.spawn(move || {
                    start.wait();
                    texts_from(doc, t * 3, 200)
                })
            })
            .collect();
        for w in workers {
            for (i, text) in w.join().expect("no panic") {
                assert_eq!(text, sequential[i], "page {i}");
            }
        }
    });
}

/// 16 threads, each on its own document with the same bytes, against one
/// sequential pass (no shared state at all beyond the process).
#[test]
fn separate_documents_text_matches_sequential() {
    let bytes = forty_text_pages();
    let sequential =
        all_text(&Document::from_bytes(bytes.clone(), &OpenOptions::default()).expect("opens"));
    let start = Barrier::new(16);
    thread::scope(|s| {
        for _ in 0..16 {
            let (bytes, start, sequential) = (&bytes, &start, &sequential);
            s.spawn(move || {
                let doc =
                    Document::from_bytes(bytes.clone(), &OpenOptions::default()).expect("opens");
                start.wait();
                for _ in 0..5 {
                    assert_eq!(&all_text(&doc), sequential);
                }
            });
        }
    });
}

/// Index of the page whose content stream's xref offset is wrong, 0-based
/// ("page 7" of spec §4.10).
const BROKEN_PAGE: usize = 6;

fn misoffset() -> Vec<u8> {
    contract::misoffset_object(40, BROKEN_PAGE, 10)
}

fn xref_repaired_count(doc: &Document) -> usize {
    doc.warnings()
        .iter()
        .filter(|w| w.kind == WarningKind::XrefRepaired)
        .count()
}

/// The sequential reference for the mis-offset document: extracted from a
/// separate instance, and equal to the intact file's text (the repair finds
/// the object).
fn misoffset_sequential() -> Vec<String> {
    let doc = Document::from_bytes(misoffset(), &OpenOptions::default()).expect("opens");
    let repaired = all_text(&doc);
    let intact = all_text(
        &Document::from_bytes(
            contract::misoffset_object(40, BROKEN_PAGE, 0),
            &OpenOptions::default(),
        )
        .expect("opens"),
    );
    assert_eq!(repaired, intact);
    assert_eq!(repaired[BROKEN_PAGE], format!("Page {}\n", BROKEN_PAGE + 1));
    repaired
}

/// One round: 16 threads each extract every page of `doc` (each starting at
/// a different page, so the broken one is reached at different moments)
/// and must all see `sequential`.
fn race_all_pages(doc: &Document, sequential: &[String]) {
    let start = Barrier::new(16);
    thread::scope(|s| {
        let workers: Vec<_> = (0..16)
            .map(|t| {
                let (doc, start) = (doc, &start);
                s.spawn(move || {
                    start.wait();
                    texts_from(doc, t, doc.page_count())
                })
            })
            .collect();
        for w in workers {
            for (i, text) in w.join().expect("no panic") {
                assert_eq!(text, sequential[i], "page {i}");
            }
        }
    });
}

/// Spec §4.10 (C2 under lazy repair, needs H9): a 40-page document whose
/// xref offset for page 7's content stream is wrong by 10 bytes; 16 threads
/// each extract all pages concurrently, 50 rounds. No panic, every thread's
/// output equals the sequential output, and the document carries exactly one
/// `XrefRepaired` warning after the first round (none before: the repair is
/// lazy and opening never touches content streams).
///
/// No page's `TextPage` carries the repair (`texts_from` checks every
/// page). Per spec revision 7 this test asserts correctness and the absence
/// of panics only: the pre-H9 abort is pinned at the hayro level by
/// `h9_pre_fix.rs` (unit 08b), which rebuilds hayro without H9 in a child
/// process; core itself cannot be built against pre-H9 hayro, so there is no
/// core-level child-process variant.
#[test]
fn shared_document_under_lazy_xref_repair() {
    let sequential = misoffset_sequential();
    let doc = Document::from_bytes(misoffset(), &OpenOptions::default()).expect("opens");
    assert_eq!(xref_repaired_count(&doc), 0, "{:?}", doc.warnings());
    for round in 0..50 {
        race_all_pages(&doc, &sequential);
        // Exactly one from round 1 on, whichever thread triggered it.
        assert_eq!(
            xref_repaired_count(&doc),
            1,
            "round {round}: {:?}",
            doc.warnings()
        );
    }
    doc.close();
    assert_eq!(xref_repaired_count(&doc), 1, "still reported after close");
}

/// The repair itself raced many times: a fresh document per round, so every
/// round's threads hit the broken object before anyone has repaired it.
#[test]
fn fresh_documents_racing_the_repair() {
    let sequential = misoffset_sequential();
    for round in 0..20 {
        let doc = Document::from_bytes(misoffset(), &OpenOptions::default()).expect("opens");
        race_all_pages(&doc, &sequential);
        assert_eq!(xref_repaired_count(&doc), 1, "round {round}");
    }
}

/// Close during text extraction: every call returns text equal to the
/// sequential text, or `Err(Closed)`; nothing panics (spec §4.8, T40).
#[test]
fn close_during_text_extraction() {
    let bytes = forty_text_pages();
    let sequential =
        all_text(&Document::from_bytes(bytes.clone(), &OpenOptions::default()).expect("opens"));
    for round in 0..200 {
        let doc = Document::from_bytes(bytes.clone(), &OpenOptions::default()).expect("opens");
        let pages: Vec<_> = doc.pages().collect();
        let start = Barrier::new(5);
        thread::scope(|s| {
            for w in 0..4 {
                let (pages, start, sequential) = (&pages, &start, &sequential);
                s.spawn(move || {
                    start.wait();
                    for k in 0..10 {
                        let i = (k * 7 + w) % 40;
                        match pages[i].text(&TextOptions::default()) {
                            Ok(t) => assert_eq!(&t, &sequential[i], "round {round}"),
                            Err(PageError::Closed) => {}
                            Err(e) => panic!("round {round}: unexpected {e}"),
                        }
                    }
                });
            }
            start.wait();
            for _ in 0..(round % 20) * 500 {
                std::hint::spin_loop();
            }
            doc.close();
        });
        assert_eq!(
            pages[0].text(&TextOptions::default()).expect_err("closed"),
            PageError::Closed
        );
    }
}
