//! §4.9 character limit, interpreter budget, Type 3 procedures, and the
//! per-page warnings sampled from hayro's signals.

use std::time::Duration;

use pdf_extract_testkit::{FontDict, PdfBuilder, Type3Font, Type3Glyph, hostile, jbig2, jpx};

use super::{Unmapped, extract, extract_with, joined, kinds, on_small_stack, texts};
use crate::{Document, LimitKind, Limits, OpenOptions, PageError, WarningKind};

fn page(content: &str) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(content);
    b.to_bytes()
}

fn limits(f: impl FnOnce(&mut Limits)) -> Limits {
    let mut l = Limits::default();
    f(&mut l);
    l
}

#[test]
fn the_character_limit_stops_the_device() {
    let bytes = page("BT /Helv 11 Tf 100 700 Td (ABCDEFGH) Tj ET");
    let p = extract_with(
        bytes.clone(),
        limits(|l| l.max_chars_per_page = 5),
        Unmapped::Cid,
    )
    .expect("a full page is a warning, not an error");
    assert_eq!(joined(&p), "ABCDE");
    assert_eq!(kinds(&p), [WarningKind::CharLimit]);
    assert_eq!(p.warnings[0].page, Some(0));
    assert_eq!(
        p.warnings[0].count, 1,
        "one warning however many glyphs were cut"
    );
    assert!(
        p.warnings[0]
            .to_string()
            .starts_with("text: character limit")
    );
    // Exactly at the limit is not over it.
    let p = extract_with(bytes, limits(|l| l.max_chars_per_page = 8), Unmapped::Cid).unwrap();
    assert_eq!(p.chars.len(), 8);
    assert!(p.warnings.is_empty());
}

#[test]
fn the_operator_budget_is_a_page_error() {
    let err = extract_with(
        hostile::operator_flood(20_000),
        limits(|l| l.max_ops_per_page = 1000),
        Unmapped::Cid,
    )
    .unwrap_err();
    assert_eq!(
        err,
        PageError::Limit {
            kind: LimitKind::OpsPerPage,
            value: 1000
        }
    );
}

#[test]
fn the_time_budget_is_a_page_error() {
    let err = extract_with(
        hostile::operator_flood(50_000_000),
        limits(|l| l.page_time_budget = Duration::from_millis(50)),
        Unmapped::Cid,
    )
    .unwrap_err();
    assert_eq!(
        err,
        PageError::Limit {
            kind: LimitKind::PageTimeBudget,
            value: 50
        }
    );
    // A budget too large for `Instant` means no deadline.
    let p = extract_with(
        hostile::operator_flood(1000),
        limits(|l| l.page_time_budget = Duration::MAX),
        Unmapped::Cid,
    )
    .expect("no deadline");
    assert_eq!(joined(&p), "StartEnd");
}

#[test]
fn type3_procedures_are_never_run() {
    // D-33: a Type 3 glyph whose procedure shows the same glyph again. A
    // device that ran procedures would recurse to the nesting cap; this one
    // records the shown glyph once and never nests.
    let font = FontDict::Type3(Type3Font {
        font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        font_bbox: [0.0, 0.0, 1000.0, 1000.0],
        glyphs: vec![Type3Glyph {
            code: b'a',
            name: "a".to_owned(),
            width: 1000.0,
            procedure: "1000 0 d0 0 0 1 1 re f BT /T3 1 Tf (a) Tj ET".to_owned(),
        }],
        to_unicode: None,
    });
    let mut doc = PdfBuilder::new();
    doc.add_page_a4()
        .add_font("T3", font)
        .raw_content("BT /T3 10 Tf 100 700 Td (aa) Tj ET");
    let bytes = doc.to_bytes();
    let p = on_small_stack(move || extract(bytes));
    assert_eq!(texts(&p), ["a", "a"]);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

#[test]
fn interpreter_signals_become_page_warnings() {
    // Each detail is compared whole with the one `details` defines, which the
    // renderer also uses: a reworded literal in `extract.rs` would otherwise
    // pass here and in render's own check, and the two paths would drift.
    use super::super::details::{
        FORM_CYCLE_DETAIL, INTERPRETER_NESTING_DETAIL, OBJECT_DEPTH_DETAIL,
        stream_decode_limit_detail,
    };
    let p = extract(hostile::form_self_recursion());
    assert_eq!(joined(&p), "Rec");
    assert_eq!(kinds(&p), [WarningKind::FormXObjectCycle]);
    assert_eq!(p.warnings[0].detail, FORM_CYCLE_DETAIL);

    let p = on_small_stack(|| extract(hostile::nested_forms(60)));
    assert_eq!(kinds(&p), [WarningKind::InterpreterNesting]);
    assert!(joined(&p).ends_with("L50"));
    assert_eq!(p.warnings[0].detail, INTERPRETER_NESTING_DETAIL);

    let p = on_small_stack(|| extract(hostile::deep_nesting_in_content(200_000)));
    assert_eq!(joined(&p), "Before");
    assert_eq!(kinds(&p), [WarningKind::ObjectDepth]);
    assert_eq!(p.warnings[0].detail, OBJECT_DEPTH_DETAIL);

    let decode_limits = limits(|l| l.max_stream_decoded_bytes = 16 << 20);
    let p = extract_with(
        hostile::flate_bomb(50 << 20),
        decode_limits.clone(),
        Unmapped::Cid,
    )
    .expect("extracts");
    assert!(p.chars.is_empty());
    assert_eq!(kinds(&p), [WarningKind::StreamDecodeLimit]);
    assert_eq!(p.warnings[0].page, Some(0));
    assert_eq!(
        p.warnings[0].detail,
        stream_decode_limit_detail(&decode_limits)
    );
}

#[test]
fn the_nesting_detail_states_the_interpreters_cap() {
    // The cap is hayro's `MAX_NESTED_INTERPRETATION_DEPTH`, crate-private in
    // the vendored interpreter, so the detail cannot be built from it; this
    // ties the number the detail states to where nesting actually stops.
    let detail = super::super::details::INTERPRETER_NESTING_DETAIL;
    let words: Vec<&str> = detail.split_whitespace().collect();
    let cap: u32 = words
        .windows(2)
        .find(|w| w[1] == "levels")
        .and_then(|w| w[0].parse().ok())
        .unwrap_or_else(|| panic!("no level count in {detail:?}"));
    // A form at the cap still runs; the one past it is skipped and warned.
    let p = on_small_stack(move || extract(hostile::nested_forms(cap + 1)));
    assert_eq!(kinds(&p), [WarningKind::InterpreterNesting]);
    assert!(joined(&p).ends_with(&format!("L{cap}")), "{}", joined(&p));
    let p = on_small_stack(move || extract(hostile::nested_forms(cap)));
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert!(joined(&p).ends_with(&format!("L{cap}")), "{}", joined(&p));
}

#[test]
fn interpreter_warning_sink_counts_per_kind() {
    use hayro_interpret::InterpreterWarning;
    use std::sync::atomic::Ordering;
    let counts = super::super::extract::tests_support::sink_counts();
    counts.note(InterpreterWarning::UnsupportedFont);
    counts.note(InterpreterWarning::ImageDecodeFailure);
    counts.note(InterpreterWarning::ImageDecodeFailure);
    counts.note(InterpreterWarning::ImagePixelLimit);
    counts.note(InterpreterWarning::ImageMemoryLimit);
    counts.note(InterpreterWarning::ImageMemoryLimit);
    assert_eq!(counts.unsupported_font.load(Ordering::Relaxed), 1);
    assert_eq!(counts.image_decode_failure.load(Ordering::Relaxed), 2);
    assert_eq!(counts.image_pixel_limit.load(Ordering::Relaxed), 1);
    assert_eq!(counts.image_memory_limit.load(Ordering::Relaxed), 2);
}

#[test]
fn the_memory_limit_detail_names_the_limit_to_raise() {
    let detail = super::super::details::IMAGE_MEMORY_LIMIT_DETAIL;
    assert!(detail.starts_with("memory limit: "), "{detail}");
    assert!(detail.contains("max_image_pixels"), "{detail}");
}

#[test]
fn the_pixel_limit_detail_names_every_refusal_point() {
    // H13 refuses at the declared size, the codec header and the decoded
    // size; the text must not claim only the first.
    let detail = super::super::details::image_pixel_limit_detail(&Limits::default());
    for point in ["declared", "codec", "decoded"] {
        assert!(detail.contains(point), "{detail}");
    }
    assert!(!detail.contains("declaring"), "{detail}");
}

#[test]
fn extract_holds_no_copy_of_a_shared_detail() {
    // The renderer takes these from `details` too (its own test checks
    // that); a literal copy back in `extract.rs` would let the two paths
    // drift apart again. This scan catches only an identical copy: a
    // reworded one is caught by comparing emitted details with the shared
    // ones, which `interpreter_signals_become_page_warnings` (form cycle,
    // interpreter nesting, object depth, stream decode limit),
    // `fonts_that_fail_to_load_keep_their_text_and_are_reported` and
    // `content::damaged_content_keeps_the_text_after_the_garbage` do. Four
    // kinds cannot be raised through core text extraction and so are
    // covered by this scan alone: `UnsupportedFont` (the vendored
    // interpreter has no producer of it) and the three image kinds (the
    // text device never decodes images).
    use super::super::details::{
        CONTENT_DAMAGED_DETAIL, FONT_SUBSTITUTED_DETAIL, FORM_CYCLE_DETAIL,
        IMAGE_DECODE_FAILURE_DETAIL, IMAGE_MEMORY_LIMIT_DETAIL, INTERPRETER_NESTING_DETAIL,
        OBJECT_DEPTH_DETAIL, UNSUPPORTED_FONT_DETAIL,
    };
    let src = include_str!("../extract.rs");
    for detail in [
        CONTENT_DAMAGED_DETAIL,
        FONT_SUBSTITUTED_DETAIL,
        FORM_CYCLE_DETAIL,
        IMAGE_DECODE_FAILURE_DETAIL,
        IMAGE_MEMORY_LIMIT_DETAIL,
        INTERPRETER_NESTING_DETAIL,
        OBJECT_DEPTH_DETAIL,
        UNSUPPORTED_FONT_DETAIL,
    ] {
        assert!(!src.contains(detail), "{detail}");
    }
    for prefix in ["decoded size limit:", "pixel limit:"] {
        assert!(!src.contains(prefix), "{prefix}");
    }
}

#[test]
fn the_pixel_limit_detail_names_the_cap() {
    let detail = super::super::details::image_pixel_limit_detail(&limits(|l| {
        l.max_image_pixels = 1_000_000;
    }));
    assert!(detail.starts_with("pixel limit: "), "{detail}");
    assert!(detail.contains("1000000"), "{detail}");
}

#[test]
fn the_text_path_never_decodes_images() {
    // An inline image far over the cap: the text device asks for no pixels,
    // so nothing is refused and nothing warned.
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .raw_content("q 10 0 0 10 0 0 cm BI /W 20000 /H 20000 /BPC 8 /CS /G ID \x00 EI Q");
    let p = extract_with(
        b.to_bytes(),
        limits(|l| l.max_image_pixels = 1),
        Unmapped::Cid,
    )
    .expect("extracts");
    assert!(kinds(&p).is_empty(), "{:?}", p.warnings);
    // Nor is a JBIG2 image whose decoder would exceed its memory budget
    // (run 0003 H14): rendered, it is an `image: memory limit` warning.
    let p = extract_with(
        jbig2::symbol_export_bomb_pdf(),
        limits(|l| l.max_image_pixels = 1_000_000),
        Unmapped::Cid,
    )
    .expect("extracts");
    assert!(kinds(&p).is_empty(), "{:?}", p.warnings);
    // Nor a JPEG 2000 image whose tile records alone exceed its budget
    // (H15).
    let p = extract_with(
        jpx::tile_bomb_pdf(),
        limits(|l| l.max_image_pixels = 1_000_000),
        Unmapped::Cid,
    )
    .expect("extracts");
    assert!(kinds(&p).is_empty(), "{:?}", p.warnings);
}

#[test]
fn closed_documents_refuse_and_threads_agree() {
    let doc = Document::from_bytes(
        page("BT /Helv 11 Tf 100 700 Td (Same on every thread) Tj ET"),
        &OpenOptions::default(),
    )
    .expect("opens");
    let page0 = doc.page(0).expect("page 0");
    let expected = page0.extract_chars(Unmapped::Cid).expect("extracts");
    let results: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| s.spawn(|| page0.extract_chars(Unmapped::Cid).expect("extracts")))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("no panic"))
            .collect()
    });
    assert!(results.iter().all(|r| *r == expected));
    doc.close();
    assert_eq!(page0.extract_chars(Unmapped::Cid), Err(PageError::Closed));
}

/// Eight glyphs, then a flood of 5,000 `q Q` pairs.
fn cap_then_flood() -> Vec<u8> {
    page(&format!(
        "BT /Helv 11 Tf 100 700 Td (ABCDEFGH) Tj ET {}",
        "q Q ".repeat(5000)
    ))
}

#[test]
fn a_budget_exhausted_before_the_cap_is_an_error() {
    // D-38: the operator budget runs out while the page is still below the
    // character limit.
    let err = extract_with(
        cap_then_flood(),
        limits(|l| {
            l.max_ops_per_page = 1000;
            l.max_chars_per_page = 100;
        }),
        Unmapped::Cid,
    )
    .unwrap_err();
    assert_eq!(
        err,
        PageError::Limit {
            kind: LimitKind::OpsPerPage,
            value: 1000
        }
    );
}

#[test]
fn the_cap_cancels_interpretation_and_returns_the_page() {
    // §4.9 (6), D-38, H10g: reaching the cap raises the cancel flag
    // (`BudgetExhaustion::Cancelled`), interpretation stops within
    // `check_interval` operators, and the partial page is returned instead
    // of the later operator-budget error.
    let p = extract_with(
        cap_then_flood(),
        limits(|l| {
            l.max_ops_per_page = 1000;
            l.max_chars_per_page = 5;
        }),
        Unmapped::Cid,
    )
    .expect("the partial page");
    assert_eq!(joined(&p), "ABCDE");
    assert_eq!(kinds(&p), [WarningKind::CharLimit]);
    // hayro stopped right after the `Tj` that reached the cap (`BT`, `Tf`,
    // `Td`, `Tj`, and the refused `ET`), not 10,000 operators later.
    let stats = super::super::extract::tests_support::last_stats().expect("interpreted");
    assert_eq!(
        stats.exhausted,
        Some(hayro_interpret::BudgetExhaustion::Cancelled)
    );
    assert_eq!(stats.ops, 5);
}

/// Shows `shown` with the font resource `/F` = `font_entry`, plus extra
/// objects numbered from 6.
fn font_page(font_entry: &str, shown: &str, extra: &[String]) -> Vec<u8> {
    let mut objects = vec![
        format!("<< /Font << /F {font_entry} >> >>"),
        super::common::stream("", &format!("BT /F 11 Tf 10 100 Td {shown} Tj ET")),
    ];
    objects.extend_from_slice(extra);
    super::common::raw_pdf(&objects)
}

/// A font of `subtype` showing `text`, whose descriptor's `file_key`
/// program is garbage.
fn corrupt_program(subtype: &str, file_key: &str, text: &str) -> Vec<u8> {
    font_page(
        "6 0 R",
        &format!("({text})"),
        &[
            format!(
                "<< /Type /Font /Subtype /{subtype} /BaseFont /Broken /FirstChar 32 \
                 /LastChar 126 /Widths [{}] /FontDescriptor 7 0 R >>",
                "500 ".repeat(95)
            ),
            format!("<< /Type /FontDescriptor /FontName /Broken /Flags 32 /{file_key} 8 0 R >>"),
            super::common::stream("/Length1 10 /Length2 0 /Length3 0", "not a font program"),
        ],
    )
}

#[test]
fn fonts_that_fail_to_load_keep_their_text_and_are_reported() {
    // §4.9 (6b), T37, H10e: hayro substitutes a standard font, the text is
    // kept as the reference keeps it, and one `FontLoadFailed` warning
    // counts the substitutions.
    let cid = font_page(
        "6 0 R",
        "<002400250026>",
        &[
            "<< /Type /Font /Subtype /Type0 /BaseFont /Broken /Encoding /Identity-H \
             /DescendantFonts [9 0 R] >>"
                .to_owned(),
            "<< /Type /FontDescriptor /FontName /Broken /Flags 32 /FontFile2 8 0 R >>".to_owned(),
            super::common::stream("/Length1 10", "not a font program"),
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Broken /CIDSystemInfo \
             << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
             /FontDescriptor 7 0 R /CIDToGIDMap /Identity >>"
                .to_owned(),
        ],
    );
    let cases = [
        (
            corrupt_program("TrueType", "FontFile2", "BrokenTT"),
            "BrokenTT",
        ),
        (corrupt_program("Type1", "FontFile", "BrokenT1"), "BrokenT1"),
        (
            corrupt_program("Type1", "FontFile3", "BrokenCFF"),
            "BrokenCFF",
        ),
        (cid, "ABC"),
        (font_page("99 0 R", "(Dangling)", &[]), "Dangling"),
        (font_page("42", "(NotADict)", &[]), "NotADict"),
        (
            font_page(
                "<< /Type /Font /Subtype /Type42 /BaseFont /Helvetica >>",
                "(BadSubtype)",
                &[],
            ),
            "BadSubtype",
        ),
        (
            font_page("<< /Type /Font /BaseFont /Helvetica >>", "(NoSubtype)", &[]),
            "NoSubtype",
        ),
        (
            // Helvetica reads the two-byte codes as four one-byte codes:
            // NUL (sanitised), `$`, NUL, `%`.
            font_page(
                "<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding /Identity-H >>",
                "<00240025>",
                &[],
            ),
            "\u{FFFD}$\u{FFFD}%",
        ),
    ];
    for (bytes, text) in cases {
        let p = extract(bytes);
        assert_eq!(joined(&p), text);
        assert_eq!(kinds(&p), [WarningKind::FontLoadFailed], "{text}");
        assert_eq!(p.warnings[0].count, 1, "{text}");
        assert!(
            p.warnings[0].to_string().starts_with("font: substituted"),
            "{}",
            p.warnings[0]
        );
        // The shared detail the renderer also raises, not a copy of it.
        assert_eq!(
            p.warnings[0].detail,
            super::super::details::FONT_SUBSTITUTED_DETAIL,
            "{text}"
        );
    }
}

#[test]
fn each_substitution_is_counted() {
    // Two broken fonts on one page: one warning, count 2.
    let bytes = font_page(
        "6 0 R /G 99 0 R",
        "(One) Tj /G 11 Tf (Two)",
        &[
            format!(
                "<< /Type /Font /Subtype /TrueType /BaseFont /Broken /FirstChar 32 \
                 /LastChar 126 /Widths [{}] /FontDescriptor 7 0 R >>",
                "500 ".repeat(95)
            ),
            "<< /Type /FontDescriptor /FontName /Broken /Flags 32 /FontFile2 8 0 R >>".to_owned(),
            super::common::stream("/Length1 10", "not a font program"),
        ],
    );
    let p = extract(bytes);
    assert_eq!(joined(&p), "OneTwo");
    assert_eq!(kinds(&p), [WarningKind::FontLoadFailed]);
    assert_eq!(p.warnings[0].count, 2);
    assert!(p.warnings[0].to_string().ends_with("(×2)"));
}

#[test]
fn a_missing_font_resource_is_not_a_failure() {
    // hayro's deliberate Helvetica fallback for a name not in `/Font`.
    let bytes = super::common::raw_pdf(&[
        "<< >>".to_owned(),
        super::common::stream("", "BT /Nope 11 Tf 10 100 Td (Fine) Tj ET"),
    ]);
    let p = extract(bytes);
    assert_eq!(joined(&p), "Fine");
    assert!(p.warnings.is_empty());
}

#[test]
fn decode_limit_warnings_belong_to_their_page() {
    // §4.9 (6), H10f: two threads work on different pages of one document;
    // only the page whose content is over the decode limit is warned.
    //
    // To make the misattribution certain rather than lucky, page 1 is long
    // (a 400,000-operator flood after its text) and the other thread keeps
    // re-extracting the bomb page, one over-limit decode each, for as long
    // as page 1 runs. Before H10 the counter is per document, so page 1's
    // before/after delta always includes those decodes and it is charged a
    // `StreamDecodeLimit` it did not cause.
    use std::io::Write as _;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    let mut bomb = Vec::new();
    {
        let mut z = flate2::write::ZlibEncoder::new(&mut bomb, flate2::Compression::best());
        z.write_all(&vec![b' '; 4 << 20]).unwrap();
        z.finish().unwrap();
    }
    let mut page0 = format!(
        "<< /Length {} /Filter /FlateDecode >>\nstream\n",
        bomb.len()
    )
    .into_bytes();
    page0.extend_from_slice(&bomb);
    page0.extend_from_slice(b"\nendstream");
    let page1 = format!(
        "BT /F 11 Tf 100 700 Td (Fine) Tj ET {}",
        "q Q ".repeat(200_000)
    );
    // Page 0: the flate bomb; page 1: text and the flood. Built by hand so
    // both pages live in one document.
    let bytes = super::common::assemble_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
           /Resources << /Font << /F 7 0 R >> >> /Contents 6 0 R >>"
            .to_vec(),
        page0,
        super::common::stream("", &page1).into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ]);
    let doc = Document::from_bytes(
        bytes,
        &OpenOptions {
            limits: limits(|l| l.max_stream_decoded_bytes = 1 << 20),
            ..OpenOptions::default()
        },
    )
    .expect("opens");

    let done = AtomicBool::new(false);
    let bombs = AtomicU32::new(0);
    let p1 = std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::Acquire) {
                let p0 = doc.page(0).unwrap().extract_chars(Unmapped::Cid).unwrap();
                assert_eq!(kinds(&p0), [WarningKind::StreamDecodeLimit]);
                bombs.fetch_add(1, Ordering::Relaxed);
            }
        });
        // Let the bomb loop get going before page 1 starts.
        while bombs.load(Ordering::Relaxed) == 0 {
            std::thread::yield_now();
        }
        let p1 = doc.page(1).unwrap().extract_chars(Unmapped::Cid).unwrap();
        done.store(true, Ordering::Release);
        p1
    });
    assert!(
        bombs.load(Ordering::Relaxed) > 1,
        "the bomb page ran alongside"
    );
    assert_eq!(joined(&p1), "Fine");
    assert!(p1.warnings.is_empty(), "{:?}", p1.warnings);
}
