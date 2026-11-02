//! Unit tests of the rendering hook (run 0002 §4.1, `with_hayro_page`
//! bullet): the page the closure receives is the one the text path builds.
//! Inside the crate because the text path's builder (`PageIndex::hayro_page`)
//! is internal; `tests/render_support.rs` checks the same pages against
//! hayro's own eager page list from outside.

use std::path::PathBuf;

use crate::{Document, OpenOptions, PageError};

fn corpus_c() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/corpus-c");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("corpus-c exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
        .collect();
    files.sort();
    files
}

/// `[x0, y0, x1, y1]` of an optional media box, comparable with `==`.
fn mb(page: Option<&hayro_syntax::page::Page<'_>>) -> Option<[f64; 4]> {
    page.map(|p| {
        let r = p.media_box();
        [r.x0, r.y0, r.x1, r.y1]
    })
}

#[test]
fn the_hook_sees_the_page_the_text_path_builds() {
    let mut pages = 0;
    for path in corpus_c() {
        let Ok(doc) = Document::open(&path, &OpenOptions::default()) else {
            continue;
        };
        for page in doc.pages() {
            let via_hook = page
                .with_hayro_page(|input| {
                    assert_eq!(input.index, page.index());
                    assert_eq!(*input.limits, *page.limits());
                    assert!(format!("{input:?}").contains("RenderInput"));
                    mb(input.page)
                })
                .expect("open");
            let via_text_path = page
                .with_loaded(|loaded, i| mb(loaded.index.hayro_page(loaded.pdf.xref(), i).as_ref()))
                .expect("open");
            assert_eq!(
                via_hook,
                via_text_path,
                "{} page {}",
                path.display(),
                page.index()
            );
            assert!(
                via_hook.is_some(),
                "{} page {}",
                path.display(),
                page.index()
            );
            pages += 1;
        }
    }
    // Most of corpus C opens; a near-empty run would pass vacuously.
    assert!(pages >= 250, "only {pages} pages compared");
}

#[test]
fn the_hook_fails_closed_after_close() {
    let path = corpus_c().into_iter().next().expect("a fixture");
    let doc = Document::open(&path, &OpenOptions::default()).expect("opens");
    let page = doc.page(0).expect("page 0");
    doc.close();
    assert_eq!(
        page.with_hayro_page(|_| unreachable!("closed"))
            .expect_err("closed"),
        PageError::Closed
    );
}
