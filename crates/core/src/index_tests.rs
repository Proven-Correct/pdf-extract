//! Unit tests of the page-tree walk and page rebuilding (`index.rs`): the
//! resolution-count bounds of T14/T18 through a counting `NodeSource`, and
//! equivalence of every rebuilt page with hayro's eager `pages()` (T13, T17).

use std::cell::Cell;

use hayro_syntax::Pdf;
use hayro_syntax::object::{Dict, Name, ObjectIdentifier, Rect};
use hayro_syntax::page::Page as HayroPage;
use pdf_extract_testkit::hostile::{self, TreeShape};

use super::*;
use crate::warning::{WarningKind, WarningSink};

/// Counts every dictionary the walk resolves (the "counting xref wrapper"
/// of spec T18 / §4.8).
struct Counting<'a> {
    xref: &'a XRef,
    calls: Cell<usize>,
}

impl<'a> NodeSource<'a> for Counting<'a> {
    fn dict(&self, id: ObjectIdentifier) -> Option<Dict<'a>> {
        self.calls.set(self.calls.get() + 1);
        self.xref.get::<Dict<'a>>(id)
    }
}

const NO_LIMITS: WalkLimits = WalkLimits {
    max_pages: None,
    max_depth: 256,
};

/// Walks `pdf` through a counter; returns the outcome, the resolution count
/// and the warning kinds.
fn counted_walk(
    pdf: &Pdf,
    limits: WalkLimits,
) -> (Result<WalkOutcome, OpenError>, usize, Vec<WarningKind>) {
    let src = Counting {
        xref: pdf.xref(),
        calls: Cell::new(0),
    };
    let mut sink = WarningSink::new(None);
    let out = walk(&src, pdf.pages_root(), limits, &mut sink);
    let kinds = sink.finish().into_iter().map(|w| w.kind).collect();
    (out, src.calls.get(), kinds)
}

fn open(bytes: Vec<u8>) -> Pdf {
    Pdf::new(bytes).expect("fixture opens")
}

/// §4.8: `Fanout{100,2}` with `max_pages = 10` stops with
/// `TooManyPages{10, 11}` after at most 10 + 3 resolutions (root, first
/// interior node, eleven leaves).
#[test]
fn max_pages_bounds_resolutions() {
    let pdf = open(hostile::page_tree(TreeShape::Fanout {
        leaves_per_node: 100,
        levels: 2,
    }));
    let limits = WalkLimits {
        max_pages: Some(10),
        max_depth: 256,
    };
    let (out, calls, _) = counted_walk(&pdf, limits);
    assert!(matches!(
        out,
        Err(OpenError::TooManyPages {
            limit: 10,
            at_least: 11
        })
    ));
    assert!(calls <= 10 + 3, "{calls} resolutions");
    assert!(!pdf.pages_built());
}

/// T18: a full walk resolves exactly `leaves + interior nodes` dictionaries,
/// whatever the shape; duplicates and cycles cost nothing extra.
#[test]
fn a_full_walk_resolves_each_node_once() {
    for (shape, leaves, interior) in [
        (TreeShape::Flat(50), 50, 1),
        (
            TreeShape::Fanout {
                leaves_per_node: 5,
                levels: 3,
            },
            125,
            1 + 5 + 25,
        ),
        (TreeShape::DuplicateLeaf, 1, 1),
        (TreeShape::Cycle, 1, 1),
        (TreeShape::TypelessLeaf, 2, 1),
        // The direct kid is not resolved: it is already in hand.
        (TreeShape::DirectDictKid, 2, 1),
    ] {
        let pdf = open(hostile::page_tree(shape));
        let (out, calls, _) = counted_walk(&pdf, NO_LIMITS);
        let out = out.expect("walk succeeds");
        assert_eq!(out.index.len(), leaves, "{shape:?}");
        let resolved_leaves = if shape == TreeShape::DirectDictKid {
            1
        } else {
            leaves
        };
        assert_eq!(calls, resolved_leaves + interior, "{shape:?}");
        assert_eq!(out.index.inherited.len(), interior, "{shape:?}");
    }
}

/// The depth cap costs no resolution below it and records that it cut.
#[test]
fn depth_cap_stops_resolving() {
    let pdf = open(hostile::deep_page_tree(3000));
    let (out, calls, kinds) = counted_walk(&pdf, NO_LIMITS);
    let out = out.expect("walk succeeds");
    assert!(out.index.is_empty());
    assert!(out.depth_skipped);
    // Depths 0..=256 are resolved, depth 257 is refused unresolved.
    assert_eq!(calls, 257);
    assert_eq!(kinds, vec![WarningKind::PageTreeDepth]);
}

fn rect(r: Rect) -> [f64; 4] {
    [r.x0, r.y0, r.x1, r.y1]
}

/// Asserts the rebuilt page equals the eager one on everything `Page`
/// derives from inheritance: boxes, rotation, dictionary, and each font.
fn assert_same(label: &str, ours: &HayroPage<'_>, eager: &HayroPage<'_>, fonts: &[&[u8]]) {
    assert_eq!(rect(ours.media_box()), rect(eager.media_box()), "{label}");
    assert_eq!(rect(ours.crop_box()), rect(eager.crop_box()), "{label}");
    assert_eq!(
        format!("{:?}", ours.rotation()),
        format!("{:?}", eager.rotation()),
        "{label}"
    );
    assert_eq!(ours.raw(), eager.raw(), "{label}: dictionary");
    for font in fonts {
        let n = Name::new(font).expect("valid name");
        let (a, b) = (
            ours.resources().get_font(&n),
            eager.resources().get_font(&n),
        );
        assert_eq!(
            a.and_then(|d| d.obj_id()),
            b.and_then(|d| d.obj_id()),
            "{label}: font {}",
            String::from_utf8_lossy(font)
        );
    }
}

/// Rebuilds every page of `bytes` from the index and compares it with
/// hayro's eager walk, page by page.
fn check_equivalence(label: &str, bytes: Vec<u8>, fonts: &[&[u8]]) -> usize {
    let pdf = open(bytes);
    let mut sink = WarningSink::new(None);
    let index = walk(&pdf.xref(), pdf.pages_root(), NO_LIMITS, &mut sink)
        .expect("walk")
        .index;
    let ours: Vec<HayroPage<'_>> = (0..index.len())
        .map(|i| index.hayro_page(pdf.xref(), i).expect("leaf resolves"))
        .collect();
    assert!(index.hayro_page(pdf.xref(), index.len()).is_none());
    let eager = pdf.pages();
    assert_eq!(ours.len(), eager.len(), "{label}: count");
    for (i, (a, b)) in ours.iter().zip(eager.iter()).enumerate() {
        assert_same(&format!("{label} page {i}"), a, b, fonts);
    }
    ours.len()
}

/// T13/T17 over the unit-03 shapes whose eager page list has no duplicates
/// (hayro keeps duplicate leaves; we do not, KP10).
#[test]
fn rebuilt_pages_match_eager_pages_for_tree_shapes() {
    for shape in [
        TreeShape::Flat(7),
        TreeShape::Fanout {
            leaves_per_node: 3,
            levels: 3,
        },
        TreeShape::LyingCount { claimed: 5 },
        TreeShape::TypelessLeaf,
        TreeShape::DirectDictKid,
        TreeShape::Cycle,
    ] {
        check_equivalence(
            &format!("{shape:?}"),
            hostile::page_tree(shape),
            &[b"Helv", b"Nope"],
        );
    }
    assert_eq!(
        check_equivalence("deep 40", hostile::deep_page_tree(40), &[b"Helv"]),
        1
    );
}

/// A classic-xref file from object bodies (object `i + 1` is `bodies[i]`,
/// catalog object 1), written in number order.
fn assemble(bodies: &[&str]) -> Vec<u8> {
    let order: Vec<usize> = (0..bodies.len()).collect();
    assemble_in_order(bodies, &order)
}

/// Like [`assemble`], but writes the objects to the file in `order` (indices
/// into `bodies`), so file order and object-number order can differ.
fn assemble_in_order(bodies: &[&str], order: &[usize]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0; bodies.len()];
    for &i in order {
        offsets[i] = out.len();
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, bodies[i]).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes(),
    );
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// Every inheritable key overridden at every level, plus a malformed
/// nearest `MediaBox` (the outer valid one must win, as in the eager walk)
/// and a node whose only contribution is `Resources`.
#[test]
fn rebuilt_pages_match_eager_pages_with_overrides() {
    let bytes = assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        // 2: root
        "<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 3 /MediaBox [0 0 500 600] \
         /CropBox [10 10 400 500] /Rotate 90 /Resources << /Font << /F1 8 0 R /F2 9 0 R >> >> >>",
        // 3: middle, malformed MediaBox, own Rotate and F1
        "<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R] /Count 2 /MediaBox [1 2] \
         /Rotate -90 /Resources << /Font << /F1 10 0 R >> >> >>",
        // 4: own CropBox and F3
        "<< /Type /Page /Parent 3 0 R /CropBox [0 0 100 100] /Resources << /Font << /F3 11 0 R >> >> >>",
        // 5: own Rotate, shadows F2
        "<< /Type /Page /Parent 3 0 R /Rotate 180 /Resources << /Font << /F2 12 0 R >> >> >>",
        // 6: middle with resources only
        "<< /Type /Pages /Parent 2 0 R /Kids [7 0 R] /Count 1 /Resources << /Font << /F3 12 0 R >> >> >>",
        // 7: nothing of its own
        "<< /Type /Page /Parent 6 0 R >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >>",
    ]);
    let n = check_equivalence("overrides", bytes, &[b"F1", b"F2", b"F3"]);
    assert_eq!(n, 3);
}

/// A node with `/Kids` that is not an array, and a `/Kids` entry that is
/// neither a reference nor a dictionary, are reported and skipped; a root
/// without `/Kids` yields nothing (D-31) rather than being taken for a page.
#[test]
fn malformed_kids_are_reported() {
    let bytes = assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 42 4 0 R] /Count 2 >>",
        "<< /Type /Pages /Kids 7 >>",
        "<< /Type /Page >>",
    ]);
    let pdf = open(bytes);
    let (out, _, kinds) = counted_walk(&pdf, NO_LIMITS);
    assert_eq!(out.expect("walk").index.len(), 1);
    assert_eq!(kinds, vec![WarningKind::PageTreeNodeUnreadable]);

    let pdf = open(assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Page /MediaBox [0 0 10 10] >>",
    ]));
    let (out, calls, kinds) = counted_walk(&pdf, NO_LIMITS);
    assert!(out.expect("walk").index.is_empty());
    assert_eq!(calls, 1);
    assert_eq!(kinds, vec![WarningKind::PageTreeNodeUnreadable]);
}

/// T15: the brute-force scan returns `/Type /Page` dictionaries in
/// object-number order (not file order), respects `max_pages`, and the
/// pages it finds can be built.
#[test]
fn brute_force_is_ordered_and_bounded() {
    // The catalog's /Pages dangles; the file holds the pages in the order
    // 5, 4, 2 (object numbers), with an empty /Pages node in between.
    let bytes = assemble_in_order(
        &[
            "<< /Type /Catalog /Pages 9 0 R >>",
            "<< /Type /Page /MediaBox [0 0 20 20] >>",
            "<< /Type /Pages /Kids [] >>",
            "<< /Type /Page /MediaBox [0 0 40 40] >>",
            "<< /Type /Page >>",
        ],
        &[0, 4, 3, 2, 1],
    );
    let pdf = open(bytes);
    let mut sink = WarningSink::new(None);
    let index = brute_force(&pdf, None, &mut sink).expect("scan");
    assert_eq!(index.len(), 3);
    let ids: Vec<i32> = index.leaves.iter().map(|l| l.obj.obj_number).collect();
    assert_eq!(ids, vec![2, 4, 5]);
    let page = index.hayro_page(pdf.xref(), 1).expect("builds");
    assert_eq!(rect(page.media_box()), [0.0, 0.0, 40.0, 40.0]);
    let w = sink.finish();
    assert_eq!(w[0].kind, WarningKind::PageTreeBruteForce);
    assert!(w[0].detail.contains("found 3"), "{}", w[0].detail);

    let mut sink = WarningSink::new(None);
    assert!(matches!(
        brute_force(&pdf, Some(2), &mut sink),
        Err(OpenError::TooManyPages {
            limit: 2,
            at_least: 3
        })
    ));
    assert!(brute_force(&pdf, Some(3), &mut sink).is_ok());
}

/// A direct-dictionary kid is found again by position when its page is
/// built, and a stale position yields `None` rather than a wrong page.
#[test]
fn direct_kid_is_rebuilt_by_position() {
    let pdf = open(hostile::page_tree(TreeShape::DirectDictKid));
    let mut sink = WarningSink::new(None);
    let index = walk(&pdf.xref(), pdf.pages_root(), NO_LIMITS, &mut sink)
        .expect("walk")
        .index;
    let leaf = index.leaves[1];
    assert_ne!(leaf.kid, NOT_DIRECT);
    assert!(index.hayro_page(pdf.xref(), 1).is_some());

    let mut stale = PageIndex {
        leaves: vec![PageRef { kid: 99, ..leaf }],
        inherited: index.inherited.clone(),
    };
    assert!(stale.hayro_page(pdf.xref(), 0).is_none());
    // A position that names a reference, not a direct dictionary.
    stale.leaves[0].kid = 0;
    assert!(stale.hayro_page(pdf.xref(), 0).is_none());
}

/// Direct-dictionary kids obey the depth cap like indirect ones.
#[test]
fn direct_kids_obey_the_depth_cap() {
    let pdf = open(hostile::page_tree(TreeShape::DirectDictKid));
    let limits = WalkLimits {
        max_pages: None,
        max_depth: 0,
    };
    let (out, _, kinds) = counted_walk(&pdf, limits);
    let out = out.expect("walk");
    assert!(out.index.is_empty());
    assert!(out.depth_skipped);
    assert_eq!(kinds, vec![WarningKind::PageTreeDepth]);
}

/// Direct kids beyond MAX_DIRECT_KID_POSITION are skipped (bounding the
/// cost of rebuilding their pages); those at or below it are kept.
#[test]
fn direct_kids_beyond_the_position_cap_are_skipped() {
    let kids = "<< /MediaBox [0 0 10 10] >> ".repeat(MAX_DIRECT_KID_POSITION + 3);
    let root = format!("<< /Type /Pages /Kids [{kids}] >>");
    let pdf = open(assemble(&["<< /Type /Catalog /Pages 2 0 R >>", &root]));
    let (out, calls, kinds) = counted_walk(&pdf, NO_LIMITS);
    let out = out.expect("walk");
    assert_eq!(out.index.len(), MAX_DIRECT_KID_POSITION + 1);
    assert_eq!(calls, 1);
    assert_eq!(kinds, vec![WarningKind::PageTreeNodeUnreadable]);
    let last = out
        .index
        .hayro_page(pdf.xref(), MAX_DIRECT_KID_POSITION)
        .expect("last kept page builds");
    assert_eq!(rect(last.media_box()), [0.0, 0.0, 10.0, 10.0]);
}
