//! A lazy xref repair after open that changes what the page index points to
//! (internal issue #56 B20 and B21).
//!
//! The page index is built at open and keeps object identifiers, not the
//! objects: every page operation resolves its leaf, and the inline root's
//! inherited attributes, again through the xref. A repair triggered later by
//! some unrelated lookup rebuilds the xref map from a scan of the file, and
//! the scan keeps the last definition of each object number, so an object
//! redefined after `%%EOF` (without a new xref section) is invisible at open
//! and authoritative after the repair. These tests pin, as characterization,
//! what each path does then:
//!
//! - B20: a leaf that no longer resolves to a dictionary gives an empty page
//!   with a `PageTreeNodeUnreadable` warning (`text/extract.rs`); the render
//!   twin is pinned in `pdf-extract-render`'s `render.rs`.
//! - B21: a replaced catalog takes the inline page-tree root with it. hayro's
//!   `pages_root()` then hands back an empty dictionary, and core's index
//!   silently drops what the leaves inherit from that root (`/MediaBox`,
//!   `/Resources`): no warning, and the page reads with defaults.
//!
//! In both cases a page's result depends on whether some earlier operation
//! happened to repair the table, which T39 says it should not. These tests
//! pin current behaviour; they do not endorse it.
//!
//! The input is `testkit::contract::redefined_after_open`; the repair is
//! forced by reading the first page's content stream, whose xref offset is
//! wrong.

use hayro_syntax::object::{Dict, ObjectIdentifier, Stream};
use hayro_syntax::{PagesRoot, Pdf};
use pdf_extract_core::{Document, OpenOptions, TextOptions, TextPage, Warning, WarningKind};
use pdf_extract_testkit::contract::{self, REDEFINED_BEYOND_TEXT, REDEFINED_TEXT, Redefined};

/// The advance of `S`, the first glyph of the second page, in the fixture's
/// 12 pt Courier and in the Helvetica hayro substitutes when `/F1` is not in
/// the resources.
const COURIER_S: f32 = 7.2;
const HELVETICA_S: f32 = 8.004;

/// The first page's content stream, whose xref entry is off by 10 bytes:
/// the first lookup of it repairs the table.
fn first_content() -> ObjectIdentifier {
    ObjectIdentifier::new(5, 0)
}

/// The second page's leaf, which `Redefined::SecondPage` replaces.
fn second_leaf() -> ObjectIdentifier {
    ObjectIdentifier::new(4, 0)
}

fn open(what: Redefined) -> Document {
    let doc = Document::from_bytes(
        contract::redefined_after_open(what),
        &OpenOptions::default(),
    )
    .expect("opens");
    // Opening reads neither the tail nor any content stream.
    assert_eq!(doc.page_count(), 2);
    assert!(doc.warnings().is_empty(), "{:?}", doc.warnings());
    doc
}

fn read(doc: &Document, page: usize) -> TextPage {
    doc.page(page)
        .expect("page")
        .text_page(&TextOptions::default())
        .expect("a page, not an error")
}

fn text_of(page: &TextPage) -> String {
    page.to_text(TextOptions::default().block_separator)
}

fn first_advance(page: &TextPage) -> f32 {
    page.blocks()[0].lines()[0].chars()[0].advance()
}

fn repaired(doc: &Document) -> bool {
    doc.warnings()
        .iter()
        .any(|w| w.kind == WarningKind::XrefRepaired)
}

/// The fixture's premise, at the hayro level: before the repair the xref
/// names the original objects; after it, the last definition in the file.
#[test]
fn the_repair_scan_keeps_the_last_definition() {
    let pdf = Pdf::new(contract::redefined_after_open(Redefined::SecondPage)).expect("opens");
    assert!(pdf.xref().get::<Dict<'_>>(second_leaf()).is_some());
    assert!(!pdf.xref_was_repaired());
    assert!(pdf.xref().get::<Stream<'_>>(first_content()).is_some());
    assert!(pdf.xref_was_repaired());
    assert!(pdf.xref().get::<Dict<'_>>(second_leaf()).is_none());
    assert_eq!(pdf.xref().get::<i32>(second_leaf()), Some(0));
}

/// B20, text path: once the repair has replaced the second leaf, that page
/// is empty with one `PageTreeNodeUnreadable` warning (page-level, not an
/// error). Read before the repair, the same page is normal.
#[test]
fn a_leaf_replaced_by_a_repair_after_open_is_an_empty_page_with_a_warning() {
    let doc = open(Redefined::SecondPage);
    // Both pages show only their first string: the inherited box clips the
    // second.
    assert_eq!(text_of(&read(&doc, 1)), format!("{}\n", REDEFINED_TEXT[1]));
    assert!(!repaired(&doc));
    assert_eq!(text_of(&read(&doc, 0)), format!("{}\n", REDEFINED_TEXT[0]));
    assert!(repaired(&doc));

    let page = read(&doc, 1);
    assert!(page.blocks().is_empty());
    assert_eq!(
        page.warnings(),
        [Warning {
            page: Some(1),
            kind: WarningKind::PageTreeNodeUnreadable,
            detail: "page dictionary no longer resolves; page is empty".into(),
            count: 1,
        }]
    );
}

/// B21, hayro: `pages_root()` re-reads an inline root from the catalog on
/// every call; once the repair has replaced the catalog, it hands back an
/// empty dictionary (no `/Kids`) instead.
#[test]
fn pages_root_is_an_empty_inline_dictionary_once_the_catalog_is_replaced() {
    let pdf = Pdf::new(contract::redefined_after_open(Redefined::Catalog)).expect("opens");
    let PagesRoot::Inline(root) = pdf.pages_root() else {
        panic!("the fixture's root is inline");
    };
    assert!(root.contains_key(b"Kids") && root.contains_key(b"MediaBox"));

    assert!(pdf.xref().get::<Stream<'_>>(first_content()).is_some());
    assert!(pdf.xref_was_repaired());
    let PagesRoot::Inline(root) = pdf.pages_root() else {
        panic!("the trailer read at open still says inline");
    };
    assert!(root.is_empty(), "{root:?}");
}

/// B21, core: a page resolved after the repair has replaced the catalog
/// loses everything it inherits from the inline root, silently. The page
/// box falls back to the default, so the text the inherited `/MediaBox`
/// clipped appears; `/F1` is no longer in any resource dictionary, so hayro
/// draws it in its Helvetica substitute (same ASCII text, different
/// advances). No read carries a warning. Read before the repair, the same
/// page has both attributes.
#[test]
fn a_catalog_replaced_by_a_repair_after_open_drops_the_inline_roots_attributes() {
    let doc = open(Redefined::Catalog);
    let before = read(&doc, 1);
    assert!(!repaired(&doc));
    assert_eq!(text_of(&before), format!("{}\n", REDEFINED_TEXT[1]));
    assert!((first_advance(&before) - COURIER_S).abs() < 1e-3);

    let first = read(&doc, 0);
    assert!(repaired(&doc));
    assert_eq!(text_of(&first), format!("{}\n", REDEFINED_TEXT[0]));

    let after = read(&doc, 1);
    assert_eq!(
        text_of(&after),
        format!("{}\n{REDEFINED_BEYOND_TEXT}\n", REDEFINED_TEXT[1])
    );
    assert!((first_advance(&after) - HELVETICA_S).abs() < 1e-3);

    for (i, page) in [before, first, after].iter().enumerate() {
        assert!(
            page.warnings().is_empty(),
            "read {i}: {:?}",
            page.warnings()
        );
    }
}
