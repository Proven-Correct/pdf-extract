//! Smoke test for the vendored hayro wiring (spec §4.2).
//!
//! Proves that `pdf-extract-core` links against the in-tree fork under
//! `vendor/` and that the fork can parse a minimal document end to end. The
//! PDF is a hand-written byte literal rather than test-kit output so this test
//! depends on nothing but hayro itself (the test kit arrives in unit 03).

/// A one-page PDF: catalog, page tree, one empty page.
///
/// The cross-reference table holds real byte offsets. hayro silently rebuilds
/// a broken xref by scanning for objects, so a literal with stale offsets would
/// still "open"; `literal_xref_is_exact` pins the offsets so that this test
/// exercises the normal xref path, not the repair path. Each xref entry is
/// exactly 20 bytes, including the trailing space before the newline.
const ONE_PAGE_PDF: &[u8] = concat!(
    "%PDF-1.7\n",
    "1 0 obj\n",
    "<< /Type /Catalog /Pages 2 0 R >>\n",
    "endobj\n",
    "2 0 obj\n",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>\n",
    "endobj\n",
    "3 0 obj\n",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\n",
    "endobj\n",
    "xref\n",
    "0 4\n",
    "0000000000 65535 f \n",
    "0000000009 00000 n \n",
    "0000000058 00000 n \n",
    "0000000115 00000 n \n",
    "trailer\n",
    "<< /Size 4 /Root 1 0 R >>\n",
    "startxref\n",
    "186\n",
    "%%EOF\n",
)
.as_bytes();

/// Returns the offset of the first occurrence of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap_or_else(|| panic!("{:?} not found", String::from_utf8_lossy(needle)))
}

#[test]
fn literal_xref_is_exact() {
    let text = std::str::from_utf8(ONE_PAGE_PDF).expect("literal is ASCII");
    let xref_at = find(ONE_PAGE_PDF, b"xref\n0 4\n");
    let startxref: usize = text
        .split("startxref\n")
        .nth(1)
        .and_then(|rest| rest.lines().next())
        .and_then(|n| n.parse().ok())
        .expect("startxref value");
    assert_eq!(startxref, xref_at, "startxref must point at the xref table");

    let entries = &text[xref_at + "xref\n0 4\n".len()..];
    for (num, entry) in entries.lines().take(4).enumerate().skip(1) {
        assert_eq!(
            entry.len(),
            19,
            "xref entry {num} must be 20 bytes with its newline"
        );
        let offset: usize = entry[..10].parse().expect("numeric offset");
        let header = format!("{num} 0 obj\n");
        assert!(
            ONE_PAGE_PDF[offset..].starts_with(header.as_bytes()),
            "xref entry {num} points at offset {offset}, not at object {num}",
        );
    }
}

#[test]
fn vendored_hayro_opens_one_page_pdf() {
    let pdf = hayro_syntax::Pdf::new(ONE_PAGE_PDF.to_vec()).expect("hayro opens the literal");
    assert_eq!(pdf.pages().len(), 1);
}
