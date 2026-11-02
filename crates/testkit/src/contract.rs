//! Inputs for the open-contract tests (spec §3.4 `contract`, §4.8, Appendix C
//! "contract" row): files that are damaged, mislabelled or not PDFs at all,
//! each exercising one branch of how a reader classifies what it was given.
//!
//! Unlike [`crate::hostile`], these are not resource attacks: they are the
//! everyday ways a file reaching a PDF library is wrong. Every generator
//! starts from the same small, valid document (one A4 page showing
//! [`BASE_TEXT`]) and breaks exactly one thing, so a test that sees a file
//! open (or fail) knows which defect it was reacting to. Output is
//! deterministic, like the rest of the kit.

use crate::PdfBuilder;
use crate::pdfwriter::{Objects, stream_body};

/// The text shown by the valid document every generator starts from.
pub const BASE_TEXT: &str = "Contract";

/// A valid one-page A4 document showing [`BASE_TEXT`] in Helvetica.
fn base() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .raw_content(&format!("BT /Helv 11 Tf 72 720 Td ({BASE_TEXT}) Tj ET"));
    b.to_bytes()
}

/// `%PDF-1.4` followed by bytes that contain no PDF syntax at all: the header
/// passes content detection, so the parser itself must reject the file
/// (oracle: `FileDataError`; spec §4.8: `Corrupt`).
pub fn garbage_after_header() -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    // Printable, deterministic, and free of `obj`, `xref`, `trailer` and
    // `startxref`, so no repair scan can find anything to rebuild from.
    for i in 0..64u32 {
        out.extend_from_slice(format!("garbage line {i}: lorem ipsum dolor\n").as_bytes());
    }
    out
}

/// A valid document preceded by `offset` bytes of junk, so its `%PDF-`
/// header starts at byte `offset`. The junk is printable text that never
/// contains `%PDF-`. Xref offsets in the document are relative to its own
/// start, so a reader must either honour the header offset or repair.
pub fn header_at_offset(offset: usize) -> Vec<u8> {
    const LINE: &[u8] = b"junk before the header\n";
    let mut out = Vec::with_capacity(offset + 2048);
    while out.len() < offset {
        let take = LINE.len().min(offset - out.len());
        out.extend_from_slice(&LINE[..take]);
    }
    out.extend_from_slice(&base());
    out
}

/// The valid document with its `startxref` offset overwritten in place by
/// zeros (same length, so nothing else moves). Offset 0 is the header, not an
/// xref section, so a reader must rebuild the table by scanning.
pub fn broken_startxref() -> Vec<u8> {
    let mut bytes = base();
    let tag = b"startxref\n";
    let at = bytes
        .windows(tag.len())
        .rposition(|w| w == tag)
        .expect("the builder always writes startxref")
        + tag.len();
    let digits = bytes[at..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    bytes[at..at + digits].fill(b'0');
    bytes
}

/// The valid document minus its last `cut` bytes (saturating at an empty
/// file). Small cuts remove `%%EOF` and the `startxref` offset; larger ones
/// eat into the trailer and the xref table, leaving only the objects for a
/// repair scan.
pub fn truncated_tail(cut: usize) -> Vec<u8> {
    let mut bytes = base();
    bytes.truncate(bytes.len().saturating_sub(cut));
    bytes
}

/// Plain text with no `%PDF-` anywhere: what a reader sees when a text file
/// is named `something.pdf`.
pub fn plain_text() -> Vec<u8> {
    b"This is a plain text file.\nIt is not a PDF, whatever its name says.\n".to_vec()
}

/// A well-formed file (exact xref) whose catalog has no `/Pages` entry.
/// hayro rejects it while resolving the trailer; the oracle opens it with
/// zero pages (spec T5, a documented divergence).
pub fn catalog_without_pages() -> Vec<u8> {
    let mut objs = Objects::new();
    let catalog = objs.add("<< /Type /Catalog >>");
    objs.add("<< /Producer (pdf-extract-testkit) >>");
    objs.to_bytes(catalog, "")
}

/// The text shown by [`direct_pages`].
pub const DIRECT_PAGES_TEXT: &str = "Hello";

/// A well-formed file (exact xref) whose catalog writes its page-tree root
/// inline: `/Pages << /Type /Pages /Kids [3 0 R] /Count 1 >>` instead of a
/// reference. One A4 page shows [`DIRECT_PAGES_TEXT`] in Helvetica 12 pt.
/// The oracle opens it with one page and no warning (run 0002 Appendix A.1,
/// O36); hayro needs fork change H11 to accept it.
///
/// Objects: 1 catalog, 2 font, 3 page (no `/Parent`: its parent has no
/// identifier), 4 content stream.
pub fn direct_pages() -> Vec<u8> {
    let mut objs = Objects::new();
    let catalog = objs.add("<< /Type /Catalog /Pages << /Type /Pages /Kids [3 0 R] /Count 1 >> >>");
    objs.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    objs.add(
        "<< /Type /Page /MediaBox [0 0 595 842] /Resources << /Font << /F1 2 0 R >> >> \
         /Contents 4 0 R >>",
    );
    objs.add(stream_body(
        "",
        format!("BT /F1 12 Tf 72 720 Td ({DIRECT_PAGES_TEXT}) Tj ET").as_bytes(),
    ));
    objs.to_bytes(catalog, "")
}

/// A `pages`-page document (page `i` shows `Page <i+1>`) whose xref entry
/// for page `page`'s content stream (0-based page index) is off by `delta`
/// bytes. Nothing else is touched, so the file opens without repair and
/// indexes normally (the page tree never reads `/Contents`); the first
/// lookup of that content stream finds no object at the recorded offset and
/// sends the reader into a lazy xref repair. Drives the lazy-repair tests
/// (spec §3.4, §4.8b, §4.10).
///
/// # Panics
/// If `page >= pages`, or if the shifted offset would be negative or wider
/// than the ten digits of an xref entry (test-authoring errors).
pub fn misoffset_object(pages: usize, page: usize, delta: i64) -> Vec<u8> {
    assert!(page < pages, "page {page} out of range for {pages} pages");
    let mut b = PdfBuilder::new();
    for i in 0..pages {
        b.add_page_a4()
            .raw_content(&format!("BT /Helv 11 Tf 72 720 Td (Page {}) Tj ET", i + 1));
    }
    let mut bytes = b.to_bytes();
    let content = content_object_of_page(&bytes, page);
    let entry = xref_entry_at(&bytes, content);
    let offset: i64 = std::str::from_utf8(&bytes[entry..entry + 10])
        .expect("xref digits are ASCII")
        .parse()
        .expect("xref offset is a number");
    let shifted = offset + delta;
    assert!(
        (0..10_000_000_000).contains(&shifted),
        "offset {shifted} does not fit an xref entry"
    );
    bytes[entry..entry + 10].copy_from_slice(format!("{shifted:010}").as_bytes());
    bytes
}

/// Object number of the content stream of the `page`-th page dictionary in
/// file order (the builder writes pages in document order).
fn content_object_of_page(bytes: &[u8], page: usize) -> usize {
    let text = String::from_utf8_lossy(bytes);
    let dict = text
        .match_indices("/Type /Page ")
        .nth(page)
        .map(|(at, _)| &text[at..])
        .expect("page dictionary present");
    let after = &dict[dict.find("/Contents ").expect("page has /Contents") + 10..];
    after
        .split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .expect("/Contents is a reference")
}

/// Byte position of the ten offset digits of object `obj`'s entry in the
/// builder's single-subsection classic xref table (20 bytes per entry).
fn xref_entry_at(bytes: &[u8], obj: usize) -> usize {
    let tag = b"\nxref\n";
    let section = bytes
        .windows(tag.len())
        .rposition(|w| w == tag)
        .expect("xref table present")
        + tag.len();
    let first = section
        + bytes[section..]
            .iter()
            .position(|&b| b == b'\n')
            .expect("subsection header")
        + 1;
    first + 20 * obj
}

/// The text shown by page `i` of [`redefined_after_open`].
pub const REDEFINED_TEXT: [&str; 2] = ["First", "Second"];

/// The `/MediaBox` that [`redefined_after_open`]'s inline page-tree root
/// passes down to both leaves (neither leaf has one of its own).
pub const REDEFINED_MEDIA_BOX: [f32; 4] = [0.0, 0.0, 300.0, 400.0];

/// Text every page of [`redefined_after_open`] also shows at y = 600: above
/// [`REDEFINED_MEDIA_BOX`] but inside a default A4 or Letter box, so it is
/// clipped exactly while the inherited box still applies.
pub const REDEFINED_BEYOND_TEXT: &str = "Beyond";

/// Which object [`redefined_after_open`] redefines after `%%EOF`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redefined {
    /// The second page's leaf becomes the integer `0`, so its identifier
    /// no longer resolves to a dictionary.
    SecondPage,
    /// The catalog becomes `<< /Type /Catalog >>`, so the inline page-tree
    /// root (and everything the leaves inherit from it) no longer resolves.
    Catalog,
}

/// A two-page document whose meaning changes under a lazy xref repair after
/// it was opened: the one input that reaches the "page dictionary no longer
/// resolves" branches of the text and render paths (internal issue #56 B20) and the
/// `pages_root()` empty-dictionary fallback (B21).
///
/// The catalog writes its page-tree root inline, with `/MediaBox`
/// [`REDEFINED_MEDIA_BOX`] and a `/Resources` font `/F1`; the two leaves are
/// referenced and carry only `/Contents` (page `i` shows
/// `REDEFINED_TEXT[i]`). `/F1` is Courier rather than the kit's usual
/// Helvetica because hayro substitutes Helvetica for a font name missing
/// from the resources: the glyph advances then tell whether the inherited
/// `/Resources` still applied. The xref entry of the first page's content
/// stream is off by 10 bytes, as in [`misoffset_object`], and a tail of bare
/// object definitions after `%%EOF` (an incremental update whose xref
/// section was lost) redefines `what`. Opening reads neither the tail (the xref still names
/// the original objects) nor any content stream, so the file opens cleanly
/// with two pages. The first lookup of the first page's content finds no
/// object at the recorded offset and repairs the table by scanning the whole
/// file; the scan keeps the last definition of each object number, so from
/// then on `what` resolves to the tail's version. Only the xref map is
/// rebuilt: the trailer (and so the catalog's identifier) is the one read at
/// open.
///
/// Objects: 1 catalog (with the inline root), 2 font, 3 and 4 the page
/// leaves, 5 and 6 their content streams.
pub fn redefined_after_open(what: Redefined) -> Vec<u8> {
    let [x0, y0, x1, y1] = REDEFINED_MEDIA_BOX;
    let mut objs = Objects::new();
    let catalog = objs.add(format!(
        "<< /Type /Catalog /Pages << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 \
         /MediaBox [{x0} {y0} {x1} {y1}] /Resources << /Font << /F1 2 0 R >> >> >> >>"
    ));
    objs.add("<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>");
    objs.add("<< /Type /Page /Contents 5 0 R >>");
    objs.add("<< /Type /Page /Contents 6 0 R >>");
    for text in REDEFINED_TEXT {
        objs.add(stream_body(
            "",
            format!(
                "BT /F1 12 Tf 20 300 Td ({text}) Tj ET \
                 BT /F1 12 Tf 20 600 Td ({REDEFINED_BEYOND_TEXT}) Tj ET"
            )
            .as_bytes(),
        ));
    }
    let mut bytes = objs.to_bytes(catalog, "");

    // Shift the first content stream's entry (object 5), as
    // `misoffset_object` does; the digits keep their width, so nothing moves.
    let entry = xref_entry_at(&bytes, 5);
    let offset: u64 = std::str::from_utf8(&bytes[entry..entry + 10])
        .expect("xref digits are ASCII")
        .parse()
        .expect("xref offset is a number");
    bytes[entry..entry + 10].copy_from_slice(format!("{:010}", offset + 10).as_bytes());

    bytes.extend_from_slice(match what {
        Redefined::SecondPage => b"4 0 obj\n0\nendobj\n".as_slice(),
        Redefined::Catalog => b"1 0 obj\n<< /Type /Catalog >>\nendobj\n".as_slice(),
    });
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn header_offset_is_exact_and_junk_has_no_header() {
        for offset in [0, 1, 500, 1500] {
            let bytes = header_at_offset(offset);
            assert_eq!(find(&bytes, b"%PDF-"), Some(offset), "offset {offset}");
            assert_eq!(&bytes[offset..], base().as_slice());
        }
    }

    #[test]
    fn garbage_has_a_header_and_nothing_else() {
        let bytes = garbage_after_header();
        assert!(bytes.starts_with(b"%PDF-1.4\n"));
        for word in [&b"obj"[..], b"xref", b"trailer", b"<<"] {
            assert!(find(&bytes, word).is_none(), "{word:?}");
        }
    }

    #[test]
    fn broken_startxref_keeps_length_and_zeroes_the_offset() {
        let good = base();
        let bad = broken_startxref();
        assert_eq!(good.len(), bad.len());
        let tail = String::from_utf8_lossy(&bad[bad.len() - 30..]).into_owned();
        assert!(tail.contains("startxref\n0"), "{tail:?}");
        assert_ne!(good, bad);
    }

    #[test]
    fn truncation_saturates() {
        let n = base().len();
        assert_eq!(truncated_tail(10).len(), n - 10);
        assert!(truncated_tail(n + 5).is_empty());
    }

    #[test]
    fn misoffset_moves_exactly_one_entry() {
        let good = misoffset_object(3, 1, 0);
        let bad = misoffset_object(3, 1, 10);
        assert_eq!(good.len(), bad.len());
        let diff: Vec<usize> = (0..good.len()).filter(|&i| good[i] != bad[i]).collect();
        assert!(!diff.is_empty() && diff.len() <= 10, "{diff:?}");
        let obj = content_object_of_page(&good, 1);
        let at = xref_entry_at(&good, obj);
        assert!(diff.iter().all(|&i| (at..at + 10).contains(&i)));
        // The unshifted entry points at the content object's header.
        let off: usize = std::str::from_utf8(&good[at..at + 10])
            .expect("ascii")
            .parse()
            .expect("number");
        assert!(good[off..].starts_with(format!("{obj} 0 obj").as_bytes()));
        assert!(find(&good[off..], b"(Page 2)").is_some());
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn misoffset_page_must_exist() {
        let _ = misoffset_object(2, 2, 10);
    }

    #[test]
    #[should_panic(expected = "does not fit")]
    fn misoffset_cannot_go_negative() {
        let _ = misoffset_object(1, 0, -1_000_000);
    }

    #[test]
    fn redefined_after_open_differs_only_in_its_tail() {
        let leaf = redefined_after_open(Redefined::SecondPage);
        let catalog = redefined_after_open(Redefined::Catalog);
        let eof = b"%%EOF\n";
        let body = find(&leaf, eof).expect("%%EOF") + eof.len();
        assert_eq!(leaf[..body], catalog[..body]);
        assert_eq!(&leaf[body..], b"4 0 obj\n0\nendobj\n");
        assert_eq!(&catalog[body..], b"1 0 obj\n<< /Type /Catalog >>\nendobj\n");
        assert!(find(&leaf, b"/Pages << /Type /Pages /Kids [3 0 R 4 0 R]").is_some());
        // Object 5's entry points 10 bytes past its header.
        let at = xref_entry_at(&leaf, 5);
        let off: usize = std::str::from_utf8(&leaf[at..at + 10])
            .expect("ascii")
            .parse()
            .expect("number");
        assert!(leaf[off - 10..].starts_with(b"5 0 obj"));
    }

    #[test]
    fn plain_text_and_catalog_shapes() {
        assert!(find(&plain_text(), b"%PDF-").is_none());
        let cat = catalog_without_pages();
        assert!(cat.starts_with(b"%PDF-"));
        assert!(find(&cat, b"/Pages").is_none());
        let direct = direct_pages();
        assert!(find(&direct, b"/Pages << /Type /Pages /Kids [3 0 R]").is_some());
        assert!(find(&direct, b"/Pages 2 0 R").is_none());
        assert!(find(&direct, b"(Hello) Tj").is_some());
    }
}
