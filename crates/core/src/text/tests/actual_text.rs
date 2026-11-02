//! §4.9 `/ActualText` spans (T28, KP9, KP17).

use pdf_extract_testkit::{PdfBuilder, text_width};

use super::common::{raw_pdf, stream};
use super::{assert_close, extract, extract_with, texts};
use crate::Limits;
use crate::text::device::MAX_ACTUAL_TEXT_NESTING;
use crate::text::options::Unmapped;

fn page(content: &str) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(content);
    b.to_bytes()
}

#[test]
fn a_span_emits_its_text_at_its_first_glyph_and_covers_the_rest() {
    // Spec A.4: `(Hello)` over `H e l l o` → `Hello world`.
    let p = extract(page(
        "BT /Helv 11 Tf 100 700 Td /Span << /ActualText (Hello) >> BDC (H e l l o) Tj EMC \
         ( world) Tj ET",
    ));
    assert_eq!(texts(&p), ["Hello", " ", "w", "o", "r", "l", "d"]);
    let span = &p.chars[0];
    assert_eq!(span.origin, (100.0, 700.0));
    // The span's advance runs to the end of its last glyph, so the pen is
    // where ` world` starts.
    let covered = f64::from(text_width("H e l l o", 11.0).expect("WinAnsi"));
    assert_close(span.advance, covered);
    assert_close(p.chars[1].origin.0 - 100.0, covered);
}

#[test]
fn utf16_and_named_properties() {
    // `<FEFF00660069>` over one glyph → `fine`.
    let p = extract(page(
        "BT /Helv 11 Tf 100 700 Td /Span << /ActualText <FEFF00660069> >> BDC (X) Tj EMC \
         (ne) Tj ET",
    ));
    assert_eq!(texts(&p), ["fi", "n", "e"]);
    // A name operand resolves through the resources' /Properties → `ABC`.
    let named = raw_pdf(&[
        "<< /Font << /F 6 0 R >> /Properties << /P1 << /ActualText (ABC) >> >> >>".to_owned(),
        stream("", "BT /F 11 Tf 10 100 Td /Span /P1 BDC (xyz) Tj EMC ET"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ]);
    assert_eq!(texts(&extract(named)), ["ABC"]);
}

#[test]
fn an_empty_string_suppresses_the_span() {
    let p = extract(page(
        "BT /Helv 11 Tf 100 700 Td /Span << /ActualText () >> BDC (dropped) Tj EMC \
         (kept) Tj ET",
    ));
    assert_eq!(texts(&p).concat(), "kept");
}

#[test]
fn nested_spans_emit_inner_text_and_outer_text_once() {
    // Spec A.4 nested case; ours is `OUTERIN` (KP17 divergence, snapshot).
    let p = extract(page(
        "BT /Helv 11 Tf 100 700 Td /Span << /ActualText (OUTER) >> BDC (a) Tj \
         /Span << /ActualText (IN) >> BDC (b) Tj EMC (c) Tj EMC (d) Tj ET",
    ));
    assert_eq!(texts(&p), ["OUTER", "IN", "d"]);
    // An inner span seen first claims the glyphs; the outer span then has
    // nothing left to say.
    let p = extract(page(
        "BT /Helv 11 Tf 100 700 Td /Span << /ActualText (OUTER) >> BDC \
         /Span << /ActualText (IN) >> BDC (b) Tj EMC (c) Tj EMC ET",
    ));
    assert_eq!(texts(&p), ["IN"]);
}

#[test]
fn a_span_across_lines_and_plain_marked_content_inside() {
    // KP17 (ours, snapshot): one record at the first glyph.
    let p = extract(page(
        "BT /Helv 11 Tf 100 700 Td /Span << /ActualText (joined) >> BDC (ab) Tj \
         /P BMC 0 -20 Td (cd) Tj EMC EMC (e) Tj ET",
    ));
    assert_eq!(texts(&p), ["joined", "e"]);
    assert_eq!(p.chars[0].origin, (100.0, 700.0));
}

#[test]
fn spans_without_glyphs_and_stray_emc_are_harmless() {
    let p = extract(page(
        "EMC EMC BT /Helv 11 Tf 100 700 Td /Span << /ActualText (Ghost) >> BDC EMC \
         /Span << /Lang (en) >> BDC (x) Tj EMC EMC (y) Tj ET",
    ));
    assert_eq!(texts(&p), ["x", "y"]);
}

#[test]
fn span_records_are_filtered_and_counted_like_glyphs() {
    // A span whose glyphs lie off the page emits nothing; the text of a span
    // counts against the character limit by code points.
    let p = extract(page(
        "BT /Helv 11 Tf 100 900 Td /Span << /ActualText (Off) >> BDC (x) Tj EMC ET \
         BT /Helv 11 Tf 100 700 Td /Span << /ActualText (Long) >> BDC (x) Tj EMC ET",
    ));
    assert_eq!(texts(&p), ["Long"]);
    let limits = Limits {
        max_chars_per_page: 3,
        ..Limits::default()
    };
    let p = extract_with(
        page("BT /Helv 11 Tf 100 700 Td /Span << /ActualText (Long) >> BDC (x) Tj EMC (yz) Tj ET"),
        limits,
        Unmapped::Cid,
    )
    .expect("extracts");
    assert!(p.chars.is_empty(), "{:?}", texts(&p));
    assert_eq!(super::kinds(&p), [crate::WarningKind::CharLimit]);
}

#[test]
fn span_nesting_is_bounded() {
    // Spans beyond the bound are plain marked content: the innermost
    // tracked span speaks for the glyph.
    let depth = MAX_ACTUAL_TEXT_NESTING + 6;
    let mut content = String::from("BT /Helv 11 Tf 100 700 Td ");
    for i in 0..depth {
        content.push_str(&format!("/Span << /ActualText (S{i}) >> BDC "));
    }
    content.push_str("(x) Tj ");
    content.push_str(&"EMC ".repeat(depth));
    content.push_str("(y) Tj ET");
    let p = extract(page(&content));
    let innermost = format!("S{}", MAX_ACTUAL_TEXT_NESTING - 1);
    assert_eq!(texts(&p), [innermost.as_str(), "y"]);
}

/// A page whose resources name `/P0` with `/ActualText` of `text`.
fn named(text: &str, content: &str) -> Vec<u8> {
    raw_pdf(&[
        format!("<< /Font << /F 6 0 R >> /Properties << /P0 << /ActualText ({text}) >> >> >>"),
        stream("", content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ])
}

#[test]
fn actual_text_reads_are_budgeted_per_page() {
    // Four bytes per character of the limit: ten 40-byte spans use up a
    // 100-character page's budget, after which spans are plain marked
    // content and their glyphs speak for themselves.
    let mut content = String::from("BT /F 11 Tf 10 100 Td ");
    content.push_str(&"/Span /P0 BDC EMC ".repeat(10));
    content.push_str("/Span /P0 BDC (y) Tj EMC ET");
    let limits = Limits {
        max_chars_per_page: 100,
        ..Limits::default()
    };
    let p = extract_with(named(&"A".repeat(40), &content), limits, Unmapped::Cid).unwrap();
    assert_eq!(texts(&p), ["y"]);
}

#[test]
fn a_repeated_named_actual_text_is_read_until_the_budget_runs_out() {
    // Review finding: one large named string referenced by many BDCs was
    // decoded once per BDC. Now the reads stop once the page's budget is
    // spent (here 4 × 2,000,000 bytes, after 61 of the 500 reads); hayro's
    // own resolution of the property list per BDC is not ours to bound.
    let mut content = String::from("BT /F 11 Tf 10 100 Td ");
    content.push_str(&"/Span /P0 BDC EMC ".repeat(500));
    content.push_str("/Span /P0 BDC (z) Tj EMC ET");
    let p = extract(named(&"a".repeat(1 << 17), &content));
    assert_eq!(texts(&p), ["z"]);
}

#[test]
fn actual_text_decoding_follows_t28() {
    // §4.9 (6): PDFDocEncoding NUL, 0x9F and 0xAD end the string; 0x7F is
    // kept; UTF-16LE with its BOM is read; an unpaired surrogate becomes
    // U+FFFD; and the result is sanitised like any text (<FEFF0001>).
    for (string, want) in [
        (r"(a\000b)", "a"),
        (r"(a\237b)", "a"),
        (r"(a\255b)", "a"),
        (r"(a\177b)", "a\u{7F}b"),
        ("<FFFE6100>", "a"),
        ("<FEFFD800>", "\u{FFFD}"),
        ("<FEFF0001>", "\u{FFFD}"),
        (r"(a\nb)", "a\nb"),
    ] {
        let p = extract(page(&format!(
            "BT /Helv 11 Tf 100 700 Td /Span << /ActualText {string} >> BDC (x) Tj EMC ET"
        )));
        assert_eq!(texts(&p), [want], "{string}");
    }
}

/// A form that opens an `/ActualText` span and never closes it, then text
/// on the page after the form.
fn leaked_span() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_form_xobject(
            "Fm",
            [0.0, 0.0, 595.0, 842.0],
            "/Span << /ActualText (X) >> BDC BT /Helv 11 Tf 100 700 Td (form) Tj ET",
        )
        .raw_content("/Fm Do BT /Helv 11 Tf 100 600 Td (After) Tj ET");
    b.to_bytes()
}

#[test]
fn a_span_left_open_in_a_form_ends_with_the_form() {
    // §4.9 (6), H10b: the span closes at the end of the form.
    assert_eq!(texts(&extract(leaked_span())).concat(), "XAfter");
}

#[test]
fn a_4mb_named_property_referenced_2000_times_is_fast() {
    // §4.9 (6), H10c: the property list is resolved once per context, so
    // this extracts in under 2 s (about 4.6 s before H10 in release).
    let mut content = String::from("BT /F 11 Tf 10 100 Td ");
    content.push_str(&"/Span /P0 BDC EMC ".repeat(2000));
    content.push_str("(z) Tj ET");
    let bytes = named(&"\\200".repeat(1 << 20), &content);
    let start = std::time::Instant::now();
    let p = extract(bytes);
    assert_eq!(texts(&p), ["z"]);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}
