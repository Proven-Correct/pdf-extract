//! Tests for fork change H11, a direct `/Pages` dictionary (run 0002 O36,
//! §4.2).
//!
//! A catalog may write its page-tree root inline instead of referencing it.
//! hayro used to reject such a file while resolving the trailer; H11 records
//! the root's shape (`PagesRoot::Ref` or `PagesRoot::Inline`) and core's
//! index walk starts from the inline dictionary. These tests pin both layers
//! and check that every file with an indirect root is read exactly as before.

use std::path::Path;

use hayro_syntax::object::{Dict, ObjectIdentifier};
use hayro_syntax::{LoadPdfError, PagesRoot, Pdf};
use pdf_extract_core::{Document, OpenError, OpenOptions, TextOptions, WarningKind};
use pdf_extract_testkit::contract;

/// Assembles a classic-xref PDF whose object `i + 1` is `objects[i]`, with
/// object 1 as the catalog. Offsets are exact, so it opens without repair.
fn assemble(objects: &[&str]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// `bytes` with its `startxref` offset zeroed in place, so hayro must
/// rebuild the table by scanning (the `RootRef`/fallback path of
/// `XRef::new` rather than the trailer-dictionary one).
fn broken_startxref(mut bytes: Vec<u8>) -> Vec<u8> {
    let tag = b"startxref\n";
    let at = bytes
        .windows(tag.len())
        .rposition(|w| w == tag)
        .expect("startxref present")
        + tag.len();
    let digits = bytes[at..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    bytes[at..at + digits].fill(b'0');
    bytes
}

const FONT: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

fn content(text: &str) -> String {
    let data = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
    format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// `text` at y = 600 and `Outside` at y = 750.
fn two_strings(text: &str) -> String {
    let data =
        format!("BT /F1 12 Tf 72 600 Td ({text}) Tj ET BT /F1 12 Tf 72 750 Td (Outside) Tj ET");
    format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len())
}

fn text_of(doc: &Document, page: usize) -> String {
    doc.page(page)
        .expect("page")
        .text(&TextOptions::default())
        .expect("text")
}

/// §4.2 hayro level: the fixture opens, its root is inline, one page.
#[test]
fn hayro_opens_an_inline_root() {
    let pdf = Pdf::new(contract::direct_pages()).expect("opens");
    let root = pdf.pages_root();
    assert!(root.id().is_none());
    let PagesRoot::Inline(dict) = root else {
        panic!("expected an inline root, got {root:?}");
    };
    assert_eq!(dict.get::<i32>(b"Count".as_slice()), Some(1));
    assert_eq!(pdf.pages().len(), 1);
}

/// The repair path of `XRef::new` (no usable trailer offset) classifies the
/// root the same way.
#[test]
fn hayro_opens_an_inline_root_after_repair() {
    let pdf = Pdf::new(broken_startxref(contract::direct_pages())).expect("opens");
    assert!(pdf.xref_was_repaired());
    assert!(matches!(pdf.pages_root(), PagesRoot::Inline(_)));
    assert_eq!(pdf.pages().len(), 1);
    let doc = Document::from_bytes(
        broken_startxref(contract::direct_pages()),
        &OpenOptions::default(),
    )
    .expect("core opens");
    assert_eq!(doc.page_count(), 1);
    assert_eq!(text_of(&doc, 0), "Hello\n");
}

/// A catalog with no `/Pages` key, or one holding neither a reference nor a
/// dictionary, is still `Invalid` (0001 T5 unchanged).
#[test]
fn missing_or_malformed_pages_is_still_invalid() {
    for catalog in [
        "<< /Type /Catalog >>",
        "<< /Type /Catalog /Pages 42 >>",
        "<< /Type /Catalog /Pages [3 0 R] >>",
        "<< /Type /Catalog /Pages /Pages >>",
    ] {
        let bytes = assemble(&[catalog, "<< /Producer (x) >>"]);
        assert!(
            matches!(Pdf::new(bytes), Err(LoadPdfError::Invalid)),
            "{catalog}"
        );
    }
    assert!(matches!(
        Pdf::new(contract::catalog_without_pages()),
        Err(LoadPdfError::Invalid)
    ));
}

/// The id of the catalog's `/Pages` reference, read without `pages_root()`.
fn catalog_pages_ref(pdf: &Pdf) -> Option<ObjectIdentifier> {
    let catalog: Dict<'_> = pdf.xref().get(pdf.xref().root_id())?;
    catalog.get_ref(b"Pages".as_slice()).map(Into::into)
}

/// §4.2 regression: on every other corpus-C file that opens, the root is
/// `PagesRoot::Ref` with the identifier the catalog names.
#[test]
fn corpus_c_roots_are_unchanged() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/corpus-c");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("corpus-c") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let Ok(pdf) = Pdf::new(std::fs::read(&path).expect("read")) else {
            continue;
        };
        if name == "catalog_direct_pages.pdf" {
            assert!(matches!(pdf.pages_root(), PagesRoot::Inline(_)));
            continue;
        }
        let want = catalog_pages_ref(&pdf).unwrap_or_else(|| panic!("{name}: no /Pages ref"));
        match pdf.pages_root() {
            PagesRoot::Ref(id) => assert_eq!(id, want, "{name}"),
            PagesRoot::Inline(_) => panic!("{name}: unexpectedly inline"),
        }
        checked += 1;
    }
    assert!(checked > 200, "only {checked} corpus-C files opened");
}

/// §4.2 core: one page, the oracle's text, no warning; `max_pages = 0`
/// still refuses it.
#[test]
fn core_reads_an_inline_root() {
    let doc =
        Document::from_bytes(contract::direct_pages(), &OpenOptions::default()).expect("opens");
    assert_eq!(doc.page_count(), 1);
    assert_eq!(text_of(&doc, 0), "Hello\n");
    assert!(doc.warnings().is_empty(), "{:?}", doc.warnings());

    let opts = OpenOptions {
        max_pages: Some(0),
        ..OpenOptions::default()
    };
    assert!(matches!(
        Document::from_bytes(contract::direct_pages(), &opts),
        Err(OpenError::TooManyPages {
            limit: 0,
            at_least: 1
        })
    ));
}

/// An inline root that carries inheritable attributes and has both an
/// indirect and a direct kid: both pages inherit its `/Resources` and
/// `/MediaBox` (the root is re-read from the catalog when a page is built),
/// and the direct kid is found again by position.
#[test]
fn inline_root_attributes_are_inherited_by_indirect_and_direct_kids() {
    let catalog = "<< /Type /Catalog /Pages << /Type /Pages /MediaBox [0 0 300 700] \
         /Resources << /Font << /F1 2 0 R >> >> \
         /Kids [3 0 R << /Type /Page /Contents 5 0 R >>] /Count 2 >> >>";
    let bytes = assemble(&[
        catalog,
        FONT,
        "<< /Type /Page /Contents 4 0 R >>",
        &two_strings("First"),
        &two_strings("Second"),
    ]);
    let pdf = Pdf::new(bytes.clone()).expect("opens");
    assert!(matches!(pdf.pages_root(), PagesRoot::Inline(_)));
    assert_eq!(pdf.pages().len(), 2);

    let doc = Document::from_bytes(bytes, &OpenOptions::default()).expect("opens");
    assert_eq!(doc.page_count(), 2);
    assert!(doc.warnings().is_empty(), "{:?}", doc.warnings());
    // The font is only in the root's /Resources, and the second string of
    // each page lies above the root's 300 x 700 MediaBox and is dropped.
    assert_eq!(text_of(&doc, 0), "First\n");
    assert_eq!(text_of(&doc, 1), "Second\n");
}

/// An inline root without `/Kids` yields no tree pages; the walk warns and
/// the brute-force scan finds the page object instead (D-31, T15).
#[test]
fn inline_root_without_kids_falls_back_to_brute_force() {
    let bytes = assemble(&[
        "<< /Type /Catalog /Pages << /Type /Pages /Count 1 >> >>",
        FONT,
        "<< /Type /Page /Resources << /Font << /F1 2 0 R >> >> /Contents 4 0 R >>",
        &content("Found"),
    ]);
    let doc = Document::from_bytes(bytes, &OpenOptions::default()).expect("opens");
    assert_eq!(doc.page_count(), 1);
    let kinds: Vec<WarningKind> = doc.warnings().iter().map(|w| w.kind).collect();
    assert_eq!(
        kinds,
        vec![
            WarningKind::PageTreeNodeUnreadable,
            WarningKind::PageTreeBruteForce
        ]
    );
    let detail = &doc.warnings()[0].detail;
    assert!(detail.contains("inline /Pages root"), "{detail}");
    assert_eq!(text_of(&doc, 0), "Found\n");
}

/// A kid that is a reference to object `0 0 R` (the stand-in identifier core
/// records for the inline root) is an ordinary, unresolvable reference: it
/// is neither confused with the root nor deduplicated against it.
#[test]
fn zero_reference_kid_is_not_the_inline_root() {
    let bytes = assemble(&[
        "<< /Type /Catalog /Pages << /Type /Pages /Kids [0 0 R 3 0 R] /Count 2 >> >>",
        FONT,
        "<< /Type /Page /Resources << /Font << /F1 2 0 R >> >> /Contents 4 0 R >>",
        &content("Only"),
    ]);
    let doc = Document::from_bytes(bytes, &OpenOptions::default()).expect("opens");
    assert_eq!(doc.page_count(), 1);
    assert_eq!(text_of(&doc, 0), "Only\n");
    let kinds: Vec<WarningKind> = doc.warnings().iter().map(|w| w.kind).collect();
    assert!(
        kinds
            .iter()
            .all(|k| *k == WarningKind::PageTreeNodeUnreadable || *k == WarningKind::XrefRepaired),
        "{kinds:?}"
    );
}
