//! Tests of fork change H10e: every font that is replaced by a substitute
//! because it cannot be loaded bumps the thread-local `font_substitutions()`
//! (spec §3.2, T37, oracle §4.9b).
//!
//! hayro keeps the text of such fonts (it draws them with a standard font),
//! as the reference does, but upstream gave no sign that it had substituted
//! one, so a caller could not warn that the glyph shapes and metrics are not
//! the document's.

mod common;

use common::{interpret, raw_pdf, stream};
use hayro_interpret::font_substitutions;
use hayro_syntax::Pdf;

/// Shows `shown` with the font resource `/F` = `font_entry`, plus extra
/// objects numbered from 6.
fn page(font_entry: &str, shown: &str, extra: &[String]) -> Vec<u8> {
    let mut objects = vec![
        format!("<< /Font << /F {font_entry} >> >>"),
        stream("", &format!("BT /F 11 Tf 10 100 Td {shown} Tj ET")),
    ];
    objects.extend_from_slice(extra);
    raw_pdf(&objects)
}

/// A simple font of `subtype` whose descriptor's `file_key` program is
/// garbage.
fn corrupt_simple(subtype: &str, file_key: &str) -> Vec<u8> {
    page(
        "6 0 R",
        "(Broken)",
        &[
            format!(
                "<< /Type /Font /Subtype /{subtype} /BaseFont /Broken /FirstChar 32 \
                 /LastChar 126 /Widths [{}] /FontDescriptor 7 0 R >>",
                "500 ".repeat(95)
            ),
            format!("<< /Type /FontDescriptor /FontName /Broken /Flags 32 /{file_key} 8 0 R >>"),
            stream("/Length1 10 /Length2 0 /Length3 0", "not a font program"),
        ],
    )
}

/// A Type0 Identity-H font whose CIDFontType2 descendant's `/FontFile2` is
/// garbage.
fn corrupt_cid() -> Vec<u8> {
    page(
        "6 0 R",
        "<002400250026>",
        &[
            "<< /Type /Font /Subtype /Type0 /BaseFont /Broken /Encoding /Identity-H \
             /DescendantFonts [9 0 R] >>"
                .to_owned(),
            "<< /Type /FontDescriptor /FontName /Broken /Flags 32 /FontFile2 8 0 R >>".to_owned(),
            stream("/Length1 10", "not a font program"),
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Broken /CIDSystemInfo \
             << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
             /FontDescriptor 7 0 R /CIDToGIDMap /Identity >>"
                .to_owned(),
        ],
    )
}

/// How far the counter moved while page 0 of `bytes` was interpreted, and
/// how many glyphs the device received.
fn run(bytes: Vec<u8>) -> (u64, usize) {
    let pdf = Pdf::new(bytes).expect("opens");
    let page = &pdf.pages()[0];
    let before = font_substitutions();
    let out = interpret(&pdf, page);
    (font_substitutions() - before, out.text.chars().count())
}

#[test]
fn each_broken_font_is_counted_once_and_keeps_its_glyphs() {
    let cases: Vec<(&str, Vec<u8>, usize)> = vec![
        (
            "corrupt FontFile2",
            corrupt_simple("TrueType", "FontFile2"),
            6,
        ),
        ("corrupt FontFile", corrupt_simple("Type1", "FontFile"), 6),
        ("corrupt FontFile3", corrupt_simple("Type1", "FontFile3"), 6),
        ("corrupt CID FontFile2", corrupt_cid(), 3),
        ("dangling reference", page("99 0 R", "(Dangling)", &[]), 8),
        ("non-dictionary entry", page("42", "(NotADict)", &[]), 8),
        (
            "unknown subtype",
            page(
                "<< /Type /Font /Subtype /Type42 /BaseFont /Helvetica >>",
                "(BadSubtype)",
                &[],
            ),
            10,
        ),
        (
            "missing subtype",
            page("<< /Type /Font /BaseFont /Helvetica >>", "(NoSubtype)", &[]),
            9,
        ),
        (
            "Type0 without DescendantFonts",
            page(
                "<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding /Identity-H >>",
                "<00240025>",
                &[],
            ),
            4,
        ),
    ];
    for (case, bytes, glyphs) in cases {
        assert_eq!(run(bytes), (1, glyphs), "{case}");
    }
}

#[test]
fn standard_and_missing_fonts_are_not_counted() {
    for (case, bytes) in [
        (
            "standard 14",
            page(
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
                "(Fine)",
                &[],
            ),
        ),
        (
            "non-embedded TrueType",
            page(
                "<< /Type /Font /Subtype /TrueType /BaseFont /Arial >>",
                "(Fine)",
                &[],
            ),
        ),
        (
            "missing resource name",
            raw_pdf(&[
                "<< >>".to_owned(),
                stream("", "BT /Nope 11 Tf 10 100 Td (Fine) Tj ET"),
            ]),
        ),
    ] {
        assert_eq!(run(bytes), (0, 4), "{case}");
    }
}

#[test]
fn a_broken_font_selected_twice_is_counted_once() {
    // Fonts are cached per interpreter cache: the failure is counted where
    // the font is loaded.
    let bytes = page(
        "6 0 R",
        "(Broken) Tj /F 9 Tf (Br)",
        &[
            format!(
                "<< /Type /Font /Subtype /TrueType /BaseFont /Broken /FirstChar 32 \
                 /LastChar 126 /Widths [{}] /FontDescriptor 7 0 R >>",
                "500 ".repeat(95)
            ),
            "<< /Type /FontDescriptor /FontName /Broken /Flags 32 /FontFile2 8 0 R >>".to_owned(),
            stream("/Length1 10", "not a font program"),
        ],
    );
    assert_eq!(run(bytes), (1, 8));
}

#[test]
fn the_counter_is_per_thread() {
    let before = font_substitutions();
    std::thread::spawn(|| run(corrupt_cid())).join().unwrap();
    assert_eq!(font_substitutions(), before);
}
