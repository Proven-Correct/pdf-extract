//! Document-structure fixtures: page-tree shapes (KP10), the open-contract
//! inputs of spec §4.8 (Appendix C "contract"), and the hostile files small
//! enough to commit (Appendix C "hostile (committed)"; the large variants are
//! generated at test time).
//!
//! Hostile fixtures use the `hostile` gate (D-27, spec revision 10): open
//! outcome class, the exact warning-kind set and our own pinned text, never
//! the oracle's text.

use pdf_extract_testkit::hostile::{self, TreeShape};
use pdf_extract_testkit::{PdfBuilder, contract};

use super::{Fixture, HELV_DICT, assemble, helv, raw, stream};

fn kp10() -> Vec<Fixture> {
    let kp = &["KP10"];
    vec![
        Fixture::exact(
            "kp10_lying_count.pdf",
            kp,
            hostile::page_tree(TreeShape::LyingCount { claimed: 5 }),
        )
        .diverges(
            "KP10: the root's /Count says 5 for 100 leaves; the oracle trusts /Count, we \
             count every leaf (T9)",
        ),
        Fixture::exact(
            "kp10_dup_leaf.pdf",
            kp,
            hostile::page_tree(TreeShape::DuplicateLeaf),
        )
        .diverges(
            "KP10 (D-14): a leaf listed twice in /Kids; the oracle counts it twice, we \
             yield it once",
        ),
        Fixture::exact("kp10_cycle.pdf", kp, hostile::page_tree(TreeShape::Cycle)).diverges(
            "KP10 (D-14): the root lists itself in /Kids; the oracle reports 2 pages and \
             raises loading the second, we yield 1 page",
        ),
        Fixture::exact(
            "kp10_typeless_leaf.pdf",
            kp,
            hostile::page_tree(TreeShape::TypelessLeaf),
        ),
        Fixture::exact(
            "kp10_direct_kid.pdf",
            kp,
            hostile::page_tree(TreeShape::DirectDictKid),
        ),
        // A.5: an inherited 200 x 100 MediaBox and Resources two levels up;
        // the text 200 pt below the box is dropped.
        Fixture::exact(
            "kp10_inherited.pdf",
            kp,
            assemble(&[
                "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
                "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] \
                 /Resources << /Font << /Helv 6 0 R >> >> >>"
                    .to_owned(),
                "<< /Type /Pages /Parent 2 0 R /Kids [4 0 R] /Count 1 >>".to_owned(),
                "<< /Type /Page /Parent 3 0 R /Contents 5 0 R >>".to_owned(),
                stream(
                    "",
                    "BT /Helv 11 Tf 10 50 Td (Inherited) Tj ET \
                     BT /Helv 11 Tf 10 -150 Td (Below) Tj ET",
                ),
                HELV_DICT.to_owned(),
            ]),
        ),
    ]
}

/// The two-block page of spec A.1 (`Line one.`, `line two.`, a block 200 pt
/// lower), optionally rotated.
fn two_blocks(rotate: Option<i32>) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let page = b.add_page_a4();
    if let Some(r) = rotate {
        page.set_rotation(r);
    }
    for (y, t) in [
        (100.0, "Line one."),
        (114.0, "line two."),
        (314.0, "Block two."),
    ] {
        page.insert_text((100.0, y), t, helv(11.0))
            .expect("WinAnsi");
    }
    b.to_bytes()
}

fn contract_inputs() -> Vec<Fixture> {
    // Content-only detection (T3): valid documents whose names say otherwise.
    // Their bytes differ (one label each) because expectations and snapshots
    // are keyed by hash, while the oracle's outcome may depend on the name.
    let mut empty = PdfBuilder::new();
    empty.add_page_a4();
    vec![
        Fixture::exact("empty_page.pdf", &[], empty.to_bytes()),
        Fixture::exact(
            "zero_size_font.pdf",
            &[],
            raw("BT /Helv 0.001 Tf 100 700 Td (Zero) Tj ET"),
        ),
        Fixture::exact("rotate_90.pdf", &[], two_blocks(Some(90))),
        Fixture::exact("two_blocks.pdf", &[], two_blocks(None)),
        Fixture::exact(
            "header_offset_500.pdf",
            &[],
            contract::header_at_offset(500),
        ),
        Fixture::exact(
            "header_offset_1500.pdf",
            &[],
            contract::header_at_offset(1500),
        )
        .diverges(
            "Q6 (F2): the %PDF- header must lie within the first 1024 bytes, so a header \
             at offset 1500 is NotPdf; the oracle opens the file",
        ),
        Fixture::exact("broken_startxref.pdf", &[], contract::broken_startxref()),
        Fixture::exact("truncated_tail.pdf", &[], contract::truncated_tail(20)),
        Fixture::exact(
            "catalog_no_pages.pdf",
            &[],
            contract::catalog_without_pages(),
        )
        .diverges(
            "T5: a catalog without /Pages is Corrupt (hayro requires the reference); the \
             oracle opens it with 0 pages",
        ),
        // Run 0002 O36 (H11): a catalog whose /Pages is a direct dictionary.
        Fixture::exact("catalog_direct_pages.pdf", &[], contract::direct_pages()),
        Fixture::exact("empty_kids.pdf", &[], PdfBuilder::new().to_bytes()),
        Fixture::exact(
            "garbage_after_header.pdf",
            &[],
            contract::garbage_after_header(),
        ),
        Fixture::exact("plain_text.pdf", &[], contract::plain_text()),
        Fixture::exact(
            "valid.txt",
            &[],
            raw("BT /Helv 11 Tf 72 720 Td (NamedTxt) Tj ET"),
        ),
        Fixture::exact(
            "valid_noext",
            &[],
            raw("BT /Helv 11 Tf 72 720 Td (NoExtension) Tj ET"),
        ),
    ]
}

fn hostile_inputs() -> Vec<Fixture> {
    vec![
        // The page dictionary nests past the 256-level object cap: we index
        // no page (the oracle keeps it, with empty text); failing to read the
        // leaf also sends hayro into a lazy xref repair.
        Fixture::hostile(
            "deep_obj_20k.pdf",
            hostile::deep_nesting_in_page_dict(20_000),
            &[
                "ObjectDepth",
                "PageTreeBruteForce",
                "PageTreeNodeUnreadable",
                "XrefRepaired",
            ],
        ),
        Fixture::hostile(
            "deep_content_20k.pdf",
            hostile::deep_nesting_in_content(20_000),
            &["ObjectDepth"],
        ),
        // The unreadable resources object sends hayro into a lazy xref
        // repair, which the document reports (T8).
        Fixture::hostile(
            "deep_resources_20k.pdf",
            hostile::deep_nesting_in_resources(20_000),
            &["ObjectDepth", "XrefRepaired"],
        ),
        Fixture::hostile(
            "deep_type4_2k.pdf",
            hostile::deep_nesting_in_type4_function(2_000),
            &["ObjectDepth"],
        ),
        Fixture::hostile("deep_tree_200.pdf", hostile::deep_page_tree(200), &[]),
        Fixture::hostile("flate_bomb_8mib.pdf", hostile::flate_bomb(8 << 20), &[]),
        Fixture::hostile("many_ops_200k.pdf", hostile::operator_flood(200_000), &[]),
        Fixture::hostile("form_fanout_12.pdf", hostile::form_fanout(12), &[]),
        Fixture::hostile(
            "form_self_recursion.pdf",
            hostile::form_self_recursion(),
            &["FormXObjectCycle"],
        ),
    ]
}

pub fn fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();
    out.extend(kp10());
    out.extend(contract_inputs());
    out.extend(hostile_inputs());
    out
}
