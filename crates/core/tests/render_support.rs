//! The rendering hook from outside the crate (run 0002 §4.1,
//! `with_hayro_page` bullet). Built only with `--features render-support`;
//! without it the hook must not exist, which the `compile_fail` doctest on
//! `Page` checks.
//!
//! For every corpus-C fixture whose page count agrees with hayro's own eager
//! page-tree walk, each page's media box seen through the hook equals the one
//! of hayro's page at the same position: an independent check that the hook
//! hands out the right page, not just the same code path's result.

#![cfg(feature = "render-support")]

use std::path::PathBuf;

use pdf_extract_core::{Document, OpenOptions};

fn mb(r: hayro_syntax::object::Rect) -> [f64; 4] {
    [r.x0, r.y0, r.x1, r.y1]
}

#[test]
fn media_boxes_match_hayros_eager_walk() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/corpus-c");
    let mut compared = 0;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("corpus-c")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
        .collect();
    entries.sort();
    for path in entries {
        let bytes = std::fs::read(&path).expect("read");
        let Ok(doc) = Document::from_bytes(bytes.clone(), &OpenOptions::default()) else {
            continue;
        };
        let Ok(eager) = hayro_syntax::Pdf::new(bytes) else {
            continue;
        };
        let eager_pages = eager.pages();
        if eager_pages.len() != doc.page_count() {
            continue;
        }
        for (page, want) in doc.pages().zip(eager_pages.iter()) {
            let got = page
                .with_hayro_page(|input| input.page.map(|p| mb(p.media_box())))
                .expect("open");
            assert_eq!(
                got,
                Some(mb(want.media_box())),
                "{} page {}",
                path.display(),
                page.index()
            );
            compared += 1;
        }
    }
    assert!(compared >= 250, "only {compared} pages compared");
}
