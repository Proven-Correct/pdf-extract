//! Tests for fork change H1, lazy pages (spec §3.2 H1 row, §4.5).
//!
//! H1 stops `Pdf::new` from walking the page tree and adds the pieces a caller
//! needs to walk it itself: `pages_root()`, `Page::from_dict` with
//! `InheritedAttributes`, plus `pages_built()` and `xref_was_repaired()`.
//! These tests pin that the eager `pages()` path is unchanged and that a page
//! built with `from_dict` is indistinguishable from the eager one.
//!
//! The memory bullet of §4.5 needs a counting global allocator and so lives
//! in its own test binary, `h1_memory.rs`.

// Only the measuring small-stack runner, not the rest of `common`.
#[path = "common/stack.rs"]
mod stack;

use hayro_syntax::object::{Array, Dict, MaybeRef, Name, ObjectIdentifier, Rect};
use hayro_syntax::page::{InheritedAttributes, Page};
use hayro_syntax::{LoadPdfError, Pdf};
use pdf_extract_testkit::PdfBuilder;
use pdf_extract_testkit::hostile::{self, TreeShape};

/// Assembles a classic-xref PDF from object bodies; object `i + 1` is
/// `objects[i]` and the catalog is object `root`. Offsets are exact, so the
/// file opens without repair. Used for inputs the test kit has no generator
/// for (spec §4.5 assigns contract generators to unit 08).
fn assemble(objects: &[&str], root: usize) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root {root} 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// The identifier of `pdf`'s page-tree root. Every file in this module writes
/// `/Pages` as a reference; the inline form (H11) is tested in
/// `h11_direct_pages.rs`.
fn root_id(pdf: &Pdf) -> ObjectIdentifier {
    pdf.pages_root().id().expect("/Pages is a reference")
}

/// One leaf found by [`walk`]: its dictionary and what it inherits.
struct Leaf<'a> {
    dict: Dict<'a>,
    inherited: InheritedAttributes<'a>,
}

/// A pending node of [`walk`]: an interior node with its ancestors' ids (its
/// own included, for the cycle check), or a leaf.
enum Pending<'a> {
    Interior(Dict<'a>, InheritedAttributes<'a>, Vec<ObjectIdentifier>),
    Leaf(Leaf<'a>),
}

/// Walks the page tree from `pdf.pages_root()` the way a caller of H1 would
/// (iteratively, one `XRef::get` per node) and returns every leaf in document
/// order with its inherited attributes.
///
/// To serve as an oracle for `from_dict`, it reproduces the eager walk's
/// decisions rather than core's (unit 08): a kid with `/Type /Pages` is
/// interior and everything else is a leaf, and a kid reference that names one
/// of its own ancestors is dropped, as hayro's reference-cycle check does.
fn walk(pdf: &Pdf) -> Vec<Leaf<'_>> {
    let xref = pdf.xref();
    let root: Dict<'_> = xref.get(root_id(pdf)).expect("page tree root resolves");
    let mut leaves = Vec::new();
    let mut stack = vec![Pending::Interior(
        root,
        InheritedAttributes::default(),
        vec![root_id(pdf)],
    )];
    while let Some(pending) = stack.pop() {
        let (node, mut inherited, ancestors) = match pending {
            Pending::Leaf(leaf) => {
                leaves.push(leaf);
                continue;
            }
            Pending::Interior(node, inherited, ancestors) => (node, inherited, ancestors),
        };
        if let Some(r) = node.get::<Rect>(b"MediaBox") {
            inherited.media_box = Some(r);
        }
        if let Some(r) = node.get::<Rect>(b"CropBox") {
            inherited.crop_box = Some(r);
        }
        if let Some(r) = node.get::<i32>(b"Rotate") {
            inherited.rotate = Some(r);
        }
        if let Some(res) = node.get::<Dict<'_>>(b"Resources") {
            inherited.resources.push(res);
        }
        let Some(kids) = node.get::<Array<'_>>(b"Kids") else {
            continue;
        };
        let mut children = Vec::new();
        for kid in kids.raw_iter() {
            let (dict, id) = match kid {
                MaybeRef::Ref(r) => {
                    let id = ObjectIdentifier::from(r);
                    if ancestors.contains(&id) {
                        continue;
                    }
                    match xref.get::<Dict<'_>>(id) {
                        Some(d) => (d, Some(id)),
                        None => continue,
                    }
                }
                MaybeRef::NotRef(obj) => match obj.into_dict() {
                    Some(d) => (d, None),
                    None => continue,
                },
            };
            let is_pages = dict.get::<Name<'_>>(b"Type").as_deref() == Some(&b"Pages"[..]);
            children.push(if is_pages {
                let mut a = ancestors.clone();
                a.extend(id);
                Pending::Interior(dict, inherited.clone(), a)
            } else {
                Pending::Leaf(Leaf {
                    dict,
                    inherited: inherited.clone(),
                })
            });
        }
        // Reversed, so that popping the stack visits kids in document order.
        stack.extend(children.into_iter().rev());
    }
    leaves
}

fn rect(r: Rect) -> [f64; 4] {
    [r.x0, r.y0, r.x1, r.y1]
}

/// Asserts `a` and `b` agree on everything §4.5 names, and on each of `fonts`.
fn assert_same_page(label: &str, a: &Page<'_>, b: &Page<'_>, fonts: &[&[u8]]) {
    assert_eq!(
        rect(a.media_box()),
        rect(b.media_box()),
        "{label}: media box"
    );
    assert_eq!(rect(a.crop_box()), rect(b.crop_box()), "{label}: crop box");
    assert_eq!(
        format!("{:?}", a.rotation()),
        format!("{:?}", b.rotation()),
        "{label}: rotation"
    );
    assert_eq!(a.raw(), b.raw(), "{label}: page dictionary");
    for name in fonts {
        let n = Name::new(name).expect("valid name");
        let (fa, fb) = (a.resources().get_font(&n), b.resources().get_font(&n));
        assert_eq!(fa, fb, "{label}: font {}", String::from_utf8_lossy(name));
        assert_eq!(
            fa.and_then(|d| d.obj_id()),
            fb.and_then(|d| d.obj_id()),
            "{label}: font object id"
        );
    }
}

/// Checks every page `from_dict` builds from `walk` against the eager
/// `pages()`, in order, and returns the page count.
fn check_equivalence(label: &str, bytes: Vec<u8>, fonts: &[&[u8]]) -> usize {
    let pdf = Pdf::new(bytes).unwrap_or_else(|e| panic!("{label}: {e:?}"));
    let leaves = walk(&pdf);
    assert!(!pdf.pages_built(), "{label}: walking must not build pages");
    let eager = pdf.pages();
    assert_eq!(leaves.len(), eager.len(), "{label}: page count");
    for (i, (leaf, page)) in leaves.iter().zip(eager.iter()).enumerate() {
        let lazy = Page::from_dict(&leaf.dict, &leaf.inherited);
        assert_same_page(&format!("{label} page {i}"), &lazy, page, fonts);
    }
    eager.len()
}

/// §4.5 bullet 2 over every unit-03 `TreeShape` variant, at several sizes.
/// These trees carry the inherited `MediaBox` and `Resources` on the root.
#[test]
fn from_dict_matches_eager_pages_for_every_tree_shape() {
    let shapes = [
        (TreeShape::Flat(0), 0),
        (TreeShape::Flat(1), 1),
        (TreeShape::Flat(17), 17),
        (
            TreeShape::Fanout {
                leaves_per_node: 1,
                levels: 1,
            },
            1,
        ),
        (
            TreeShape::Fanout {
                leaves_per_node: 3,
                levels: 4,
            },
            81,
        ),
        (
            TreeShape::Fanout {
                leaves_per_node: 10,
                levels: 2,
            },
            100,
        ),
        (TreeShape::Cycle, 1),
        (TreeShape::DuplicateLeaf, 2),
        (
            TreeShape::LyingCount { claimed: 0 },
            hostile::LYING_COUNT_LEAVES,
        ),
        (
            TreeShape::LyingCount { claimed: 1_000_000 },
            hostile::LYING_COUNT_LEAVES,
        ),
        (TreeShape::TypelessLeaf, 2),
        (TreeShape::DirectDictKid, 2),
    ];
    for (shape, pages) in shapes {
        let n = check_equivalence(
            &format!("{shape:?}"),
            hostile::page_tree(shape),
            &[b"Helv", b"Nope"],
        );
        assert_eq!(n, pages, "{shape:?}");
    }
    let n = check_equivalence(
        "deep_page_tree(40)",
        hostile::deep_page_tree(40),
        &[b"Helv"],
    );
    assert_eq!(n, 1);
}

/// A tree that exercises every inherited attribute and resource shadowing.
/// Objects: 1 catalog, 2 root, 3 middle, 4–7 leaves, 8–12 fonts.
///
/// - root: MediaBox, CropBox, Rotate 90, fonts F1→8, F2→9
/// - middle: MediaBox and Rotate -90 override, fonts F1→10 (shadows 8)
/// - leaf 4 (under middle): own CropBox, own font F3→11
/// - leaf 5 (under middle): own Rotate 180 and Resources shadowing F2→12
/// - leaf 6 (under root): nothing of its own
/// - leaf 7 (under root): own MediaBox, its Resources an indirect empty dict
fn inheritance_tree() -> Vec<u8> {
    let font = |base: &str| format!("<< /Type /Font /Subtype /Type1 /BaseFont /{base} >>");
    let fonts: Vec<String> = [
        "Helvetica",
        "Courier",
        "Times-Roman",
        "Symbol",
        "ZapfDingbats",
    ]
    .iter()
    .map(|b| font(b))
    .collect();
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 6 0 R 7 0 R] /Count 4 /MediaBox [0 0 600 800] \
         /CropBox [10 10 590 790] /Rotate 90 /Resources << /Font << /F1 8 0 R /F2 9 0 R >> >> >>"
            .into(),
        "<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R] /Count 2 /MediaBox [0 0 300 400] \
         /Rotate -90 /Resources << /Font << /F1 10 0 R >> >> >>"
            .into(),
        "<< /Type /Page /Parent 3 0 R /CropBox [5 5 100 100] /Resources << /Font << /F3 11 0 R >> >> >>"
            .into(),
        "<< /Type /Page /Parent 3 0 R /Rotate 180 /Resources << /Font << /F2 12 0 R >> >> >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 50 60] /Resources 13 0 R >>".into(),
    ]
    .into_iter()
    .chain(fonts)
    .chain(std::iter::once("<< >>".to_owned()))
    .collect();
    let refs: Vec<&str> = objs.iter().map(String::as_str).collect();
    assemble(&refs, 1)
}

/// §4.5 bullet 2 with `CropBox`/`Rotate` inheritance and resource shadowing at
/// several levels, which the unit-03 shapes do not cover; also pins the
/// expected values so the test does not merely compare two wrong answers.
#[test]
fn from_dict_matches_eager_pages_with_overrides_at_every_level() {
    let fonts: &[&[u8]] = &[b"F1", b"F2", b"F3", b"F4"];
    assert_eq!(
        check_equivalence("inheritance", inheritance_tree(), fonts),
        4
    );

    let pdf = Pdf::new(inheritance_tree()).expect("opens");
    let leaves = walk(&pdf);
    let pages: Vec<Page<'_>> = leaves
        .iter()
        .map(|l| Page::from_dict(&l.dict, &l.inherited))
        .collect();
    let font_id = |p: &Page<'_>, name: &[u8]| {
        p.resources()
            .get_font(&Name::new(name).expect("valid name"))
            .and_then(|d| d.obj_id())
            .map(|id| id.obj_number)
    };
    let expect = [
        // (media box, crop box, rotation, F1, F2, F3)
        (
            [0.0, 0.0, 300.0, 400.0],
            [5.0, 5.0, 100.0, 100.0],
            "FlippedHorizontal",
            Some(10),
            Some(9),
            Some(11),
        ),
        (
            [0.0, 0.0, 300.0, 400.0],
            [10.0, 10.0, 590.0, 790.0],
            "Flipped",
            Some(10),
            Some(12),
            None,
        ),
        (
            [0.0, 0.0, 600.0, 800.0],
            [10.0, 10.0, 590.0, 790.0],
            "Horizontal",
            Some(8),
            Some(9),
            None,
        ),
        (
            [0.0, 0.0, 50.0, 60.0],
            [10.0, 10.0, 590.0, 790.0],
            "Horizontal",
            Some(8),
            Some(9),
            None,
        ),
    ];
    for (i, (p, (mb, cb, rot, f1, f2, f3))) in pages.iter().zip(expect).enumerate() {
        assert_eq!(rect(p.media_box()), mb, "page {i} media box");
        assert_eq!(rect(p.crop_box()), cb, "page {i} crop box");
        assert_eq!(format!("{:?}", p.rotation()), rot, "page {i} rotation");
        assert_eq!(font_id(p, b"F1"), f1, "page {i} F1");
        assert_eq!(font_id(p, b"F2"), f2, "page {i} F2");
        assert_eq!(font_id(p, b"F3"), f3, "page {i} F3");
        assert_eq!(font_id(p, b"F4"), None, "page {i} F4");
    }
}

/// With nothing inherited, `from_dict` applies hayro's defaults (A4, crop box
/// = media box, no rotation), as the eager walk does for a bare leaf.
#[test]
fn from_dict_without_inheritance_uses_defaults() {
    let pdf = Pdf::new(hostile::page_tree(TreeShape::Flat(1))).expect("opens");
    let leaf = &walk(&pdf)[0];
    let page = Page::from_dict(&leaf.dict, &InheritedAttributes::default());
    let a4 = rect(hayro_syntax::page::A4);
    assert_eq!(rect(page.media_box()), a4);
    assert_eq!(rect(page.crop_box()), a4);
    assert_eq!(format!("{:?}", page.rotation()), "None");
    assert!(
        page.resources()
            .get_font(&Name::new(b"Helv").expect("name"))
            .is_none()
    );
}

/// §4.5 bullet 1: opening builds no pages; the first `pages()` builds all.
#[test]
fn new_is_lazy_and_pages_builds_on_first_call() {
    let pdf = Pdf::new(hostile::page_tree(TreeShape::Flat(100_000))).expect("opens");
    assert!(!pdf.pages_built());
    // Reading the root does not build pages either.
    let _root: Dict<'_> = pdf.xref().get(root_id(&pdf)).expect("root");
    assert!(!pdf.pages_built());
    assert_eq!(pdf.pages().len(), 100_000);
    assert!(pdf.pages_built());
    // The second call returns the cached list.
    assert!(std::ptr::eq(pdf.pages(), pdf.pages()));
}

/// The id of the catalog's `/Pages` entry, read independently of H1.
fn catalog_pages_ref(pdf: &Pdf) -> ObjectIdentifier {
    let catalog: Dict<'_> = pdf.xref().get(pdf.xref().root_id()).expect("catalog");
    catalog.get_ref(b"Pages").expect("/Pages reference").into()
}

/// §4.5 bullet 3, first half.
#[test]
fn pages_root_is_the_catalogs_pages_reference() {
    for bytes in [
        hostile::page_tree(TreeShape::Flat(3)),
        hostile::deep_page_tree(4),
        inheritance_tree(),
        two_page_builder_pdf(),
    ] {
        let pdf = Pdf::new(bytes).expect("opens");
        assert_eq!(root_id(&pdf), catalog_pages_ref(&pdf));
    }
    // A known layout: the inheritance tree's root is object 2.
    let pdf = Pdf::new(inheritance_tree()).expect("opens");
    assert_eq!(root_id(&pdf), ObjectIdentifier::new(2, 0));
}

/// A builder PDF with its `startxref` offset digits overwritten with zeros in
/// place (same length, so nothing else moves). Offset 0 is the header, not an
/// xref section, so hayro must rebuild the table by scanning.
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
    assert!(digits > 0, "startxref offset present");
    bytes[at..at + digits].fill(b'0');
    bytes
}

fn two_page_builder_pdf() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .raw_content("BT /Helv 11 Tf 72 720 Td (One) Tj ET");
    b.add_page(300.0, 200.0)
        .raw_content("BT /Helv 11 Tf 10 10 Td (Two) Tj ET");
    b.to_bytes()
}

/// §4.5 bullet 3, second half.
#[test]
fn xref_was_repaired_only_for_a_broken_startxref() {
    let intact = Pdf::new(two_page_builder_pdf()).expect("intact opens");
    assert!(!intact.xref_was_repaired());
    assert_eq!(intact.pages().len(), 2);
    // Reading every object of an intact file never triggers a repair.
    assert!(!intact.xref_was_repaired());

    let broken = Pdf::new(broken_startxref(two_page_builder_pdf())).expect("broken opens");
    assert!(broken.xref_was_repaired());
    assert_eq!(root_id(&broken), catalog_pages_ref(&broken));
    assert_eq!(broken.pages().len(), 2);

    for shape in [TreeShape::Flat(5), TreeShape::DuplicateLeaf] {
        let pdf = Pdf::new(hostile::page_tree(shape)).expect("opens");
        assert!(!pdf.xref_was_repaired(), "{shape:?}");
    }
}

/// §4.5 bullet 5: a dangling `/Pages` reference no longer fails `Pdf::new`
/// (the tree is not read at open); with no page objects to find by brute
/// force, `pages()` is empty. A catalog with no `/Pages` key still fails.
#[test]
fn dangling_pages_opens_empty_and_missing_pages_key_fails() {
    // Object 9 does not exist.
    let dangling = assemble(
        &["<< /Type /Catalog /Pages 9 0 R >>", "<< /Producer (x) >>"],
        1,
    );
    let pdf = Pdf::new(dangling).expect("dangling /Pages opens");
    assert_eq!(root_id(&pdf), ObjectIdentifier::new(9, 0));
    assert!(pdf.xref().get::<Dict<'_>>(root_id(&pdf)).is_none());
    assert_eq!(pdf.pages().len(), 0);
    assert!(pdf.pages_built());

    let missing = assemble(&["<< /Type /Catalog >>"], 1);
    assert!(matches!(Pdf::new(missing), Err(LoadPdfError::Invalid)));
}

/// A dangling `/Pages` reference with page objects in the file: the eager
/// path's brute-force fallback still finds them (only pages with `/Contents`,
/// as before H1 moved that check out of `Page::new`).
#[test]
fn dangling_pages_falls_back_to_brute_force() {
    let bytes = assemble(
        &[
            "<< /Type /Catalog /Pages 9 0 R >>",
            "<< /Type /Page /Contents 4 0 R >>",
            "<< /Type /Page >>",
            "<< /Length 0 >>\nstream\n\nendstream",
        ],
        1,
    );
    let pdf = Pdf::new(bytes).expect("opens");
    assert_eq!(pdf.pages().len(), 1);
}

/// Appendix B item 3: before H1 a 3,000-deep tree overflowed a 512 KiB stack
/// inside `Pdf::new`, because the eager walk recursed. Opening and walking it
/// iteratively now fits; `pages()` (still recursive, for the renderer) is not
/// called.
#[test]
fn deep_tree_opens_on_a_small_stack() {
    let bytes = hostile::deep_page_tree(3_000);
    let leaves = stack::on_small_stack(move || {
        let pdf = Pdf::new(bytes).expect("opens");
        let n = walk(&pdf).len();
        assert!(!pdf.pages_built());
        n
    });
    assert_eq!(leaves, 1);
}

/// §4.5 bullet 3 (revision 4): a `/Pages` root that resolves to a dictionary
/// without `/Kids`, or to something that is not a dictionary, is no longer
/// read at open, so `Pdf::new` succeeds; with no page objects for the
/// brute-force fallback to find, `pages()` is empty.
#[test]
fn pages_root_without_kids_or_not_a_dictionary_opens_empty() {
    for (label, root) in [
        ("no /Kids", "<< /Type /Pages /Count 0 >>"),
        ("integer", "42"),
        ("array", "[1 0 R]"),
        ("name", "/Pages"),
    ] {
        let bytes = assemble(&["<< /Type /Catalog /Pages 2 0 R >>", root], 1);
        let pdf = Pdf::new(bytes).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        assert_eq!(root_id(&pdf), ObjectIdentifier::new(2, 0), "{label}");
        assert!(!pdf.pages_built(), "{label}");
        assert_eq!(pdf.pages().len(), 0, "{label}");
        assert!(pdf.pages_built(), "{label}");
    }
}

/// Points the xref entry of object `obj` at byte `offset`, in place: every
/// classic-xref entry is 20 bytes and the offset is its first 10 digits, so
/// nothing else in the file moves. Assumes a single `0 N` subsection, as
/// [`assemble`] writes.
fn set_xref_offset(mut bytes: Vec<u8>, obj: usize, offset: usize) -> Vec<u8> {
    // The leading newline keeps this from matching the tail of `startxref`.
    let tag = b"\nxref\n";
    let section = bytes
        .windows(tag.len())
        .rposition(|w| w == tag)
        .expect("xref table present")
        + tag.len();
    let first_entry = section
        + bytes[section..]
            .iter()
            .position(|&b| b == b'\n')
            .expect("subsection header")
        + 1;
    let at = first_entry + 20 * obj;
    bytes[at..at + 10].copy_from_slice(format!("{offset:010}").as_bytes());
    bytes
}

/// §4.5 bullet 4 (revision 4): repair is lazy. A file whose table is intact
/// except for one object's offset opens without repair; the first lookup of
/// that object finds another object at the recorded offset, rebuilds the
/// table by scanning, and from then on `xref_was_repaired()` is true.
#[test]
fn xref_was_repaired_flips_on_first_lookup_of_a_mis_offset_object() {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Marker (lazy) >>",
    ];
    let intact = assemble(&objects, 1);
    let catalog_at = intact
        .windows(b"1 0 obj".len())
        .position(|w| w == b"1 0 obj")
        .expect("catalog present");
    // Object 4's entry now names the catalog's offset: a valid object, but
    // with the wrong id, which is what sends `XRef::get` into repair.
    let bytes = set_xref_offset(intact, 4, catalog_at);
    let target = ObjectIdentifier::new(4, 0);

    let pdf = Pdf::new(bytes).expect("opens");
    assert!(
        !pdf.xref_was_repaired(),
        "nothing mis-offset is read at open"
    );
    // Walking the tree reads objects 2 and 3 only, whose entries are right.
    assert_eq!(pdf.pages().len(), 1);
    assert!(
        !pdf.xref_was_repaired(),
        "intact objects do not trigger repair"
    );

    let marker = pdf
        .xref()
        .get::<Dict<'_>>(target)
        .expect("object 4 found after repair");
    assert_eq!(
        marker
            .get::<hayro_syntax::object::String<'_>>(b"Marker".as_slice())
            .map(|s| s.as_bytes().to_vec()),
        Some(b"lazy".to_vec())
    );
    assert!(
        pdf.xref_was_repaired(),
        "the first lookup repaired the table"
    );
    // The rebuilt table serves the objects that were right all along.
    assert!(pdf.xref().get::<Dict<'_>>(root_id(&pdf)).is_some());
}
