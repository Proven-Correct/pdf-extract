//! The catalogue of generated sample files, shared by the hayro acceptance
//! test (`tests/generators_hayro.rs`) and the oracle dump
//! (`examples/dump_fixtures.rs`, read by `tools/oracle/mkfixtures.py
//! --verify`). One list means the two checks can never drift apart: every
//! sample hayro is tested on is also run through the oracle.
//!
//! Builder samples exercise every `PdfBuilder`/`PageBuilder` feature at least
//! once and must open everywhere. Hostile samples are the "committed" sizes of
//! spec Appendix C plus one file per `TreeShape`; their expected outcomes are
//! recorded per sample with the evidence for each.

#![allow(dead_code)] // each including crate uses a different subset

use pdf_extract_testkit::hostile::{self, TreeShape};
use pdf_extract_testkit::{
    Annotation, Appearance, AppearanceStream, BaseEncoding, CidFontSubtype, FontDescriptor,
    FontDict, PdfBuilder, SimpleEncoding, TextStyle, Type0Font, Type3Font, Type3Glyph,
    win_ansi_chars,
};

/// What the vendored hayro's `Pdf::new` does with a sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hayro {
    /// Opens with this many pages (`pdf.pages().len()`).
    Pages(usize),
}

/// What the oracle does with a sample (checked by `mkfixtures.py --verify`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oracle {
    /// Opens, `page_count` equals this, and `get_text` succeeds on every page.
    Pages(usize),
    /// Opens with this `page_count`, but loading some page raises (a
    /// documented oracle behaviour, not a generator defect).
    PagesLoadRaises(usize),
}

impl Oracle {
    /// The token written to the dump's `index.tsv`.
    pub fn token(self) -> String {
        match self {
            Self::Pages(n) => format!("pages={n}"),
            Self::PagesLoadRaises(n) => format!("pages={n};load-raises"),
        }
    }
}

pub struct Sample {
    pub name: &'static str,
    pub kind: &'static str,
    pub bytes: Vec<u8>,
    pub hayro: Hayro,
    pub oracle: Oracle,
}

fn sample(name: &'static str, kind: &'static str, bytes: Vec<u8>, pages: usize) -> Sample {
    Sample {
        name,
        kind,
        bytes,
        hayro: Hayro::Pages(pages),
        oracle: Oracle::Pages(pages),
    }
}

impl Sample {
    /// Overrides the oracle expectation where it differs from hayro's.
    fn oracle(mut self, oracle: Oracle) -> Self {
        self.oracle = oracle;
        self
    }
}

const HELV11: TextStyle = TextStyle {
    fontsize: 11.0,
    render_mode: 0,
    font: "Helv",
};

/// Every builder feature at least once. All must open in hayro and the
/// oracle with the stated page count.
pub fn builder_samples() -> Vec<Sample> {
    let mut out = Vec::new();

    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .insert_text((72.0, 72.0), "Hello world", HELV11)
        .expect("ASCII");
    out.push(sample("builder_hello", "builder", b.to_bytes(), 1));

    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    for (i, size) in [7.0f32, 11.0, 22.0, 0.001].into_iter().enumerate() {
        let y = 100.0 + 40.0 * i as f32;
        p.insert_text((72.0, y), "Size", TextStyle::size(size))
            .expect("ASCII");
    }
    for mode in 0..=7u8 {
        let style = TextStyle {
            render_mode: mode,
            ..HELV11
        };
        p.insert_text(
            (72.0, 300.0 + 20.0 * f32::from(mode)),
            &format!("Mode{mode}"),
            style,
        )
        .expect("ASCII");
    }
    out.push(sample("builder_sizes_modes", "builder", b.to_bytes(), 1));

    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    let all: Vec<char> = win_ansi_chars().collect();
    for (i, chunk) in all.chunks(30).enumerate() {
        let line: String = chunk.iter().collect();
        p.insert_text((40.0, 60.0 + 20.0 * i as f32), &line, HELV11)
            .expect("WinAnsi");
    }
    out.push(sample("builder_winansi_all", "builder", b.to_bytes(), 1));

    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .set_crop_box([100.0, 100.0, 400.0, 700.0])
        .set_rotation(90)
        .insert_text((72.0, 792.0), "OutsideCrop", HELV11)
        .expect("ASCII");
    out.push(sample("builder_crop_rotate", "builder", b.to_bytes(), 1));

    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(
        "BT /Helv 11 Tf 72 700 Td 2 Tc (Hello) Tj 0 Tc 0 -20 Td 20 Tw (a b c) Tj 0 Tw \
         0 -20 Td [(A) -300 (B) -2000 (C)] TJ 50 Tz 0 -20 Td (Alpha) Tj 100 Tz ET",
    );
    out.push(sample("builder_raw_content", "builder", b.to_bytes(), 1));

    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font(
            "FDiff",
            FontDict::standard14(
                "Helvetica",
                SimpleEncoding::Differences {
                    base: None,
                    differences: vec![
                        (65, "zzunknown".into()),
                        (66, "uni2603".into()),
                        (128, "g128".into()),
                    ],
                },
            ),
        )
        .add_font(
            "FMac",
            FontDict::standard14("Helvetica", SimpleEncoding::Named(BaseEncoding::MacRoman)),
        )
        .add_font(
            "FStd",
            FontDict::standard14("Helvetica", SimpleEncoding::Builtin),
        )
        .add_font(
            "FSym",
            FontDict::standard14("Symbol", SimpleEncoding::Builtin),
        )
        .raw_content(
            "BT /FDiff 11 Tf 72 700 Td <414280> Tj ET \
             BT /FMac 11 Tf 72 680 Td <61CA62> Tj ET \
             BT /FStd 11 Tf 72 660 Td <61A062DC63> Tj ET \
             BT /FSym 11 Tf 72 640 Td (abg) Tj ET",
        );
    out.push(sample(
        "builder_type1_encodings",
        "builder",
        b.to_bytes(),
        1,
    ));

    let square = "500 0 d0 50 0 400 600 re f";
    let glyph = |code: u8, name: &str| Type3Glyph {
        code,
        name: name.into(),
        width: 600.0,
        procedure: square.into(),
    };
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font(
            "T3",
            FontDict::Type3(Type3Font {
                font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
                font_bbox: [0.0, 0.0, 600.0, 600.0],
                glyphs: vec![
                    glyph(0, "g0"),
                    glyph(1, "g1"),
                    glyph(9, "g9"),
                    glyph(10, "g10"),
                    glyph(32, "space"),
                    glyph(65, "g65"),
                    glyph(66, "square"),
                    glyph(200, "g200"),
                ],
                to_unicode: None,
            }),
        )
        .raw_content("BT /T3 20 Tf 72 700 Td <4142000109200AC8> Tj ET");
    out.push(sample("builder_type3", "builder", b.to_bytes(), 1));

    let mut japan1 = Type0Font::identity_h("MS-Mincho");
    japan1.encoding = "90ms-RKSJ-H".into();
    japan1.ordering = "Japan1".into();
    japan1.supplement = 2;
    japan1.cid_subtype = CidFontSubtype::Type0;
    // Without a FontDescriptor the oracle fails to load a CIDFont ("missing
    // font descriptor") and falls back to raw bytes; with one it maps through
    // the ordering's UCS2 CMap as spec Appendix A.4 records.
    japan1.descriptor = Some(FontDescriptor::default());
    let mut vertical = japan1.clone();
    vertical.encoding = "Identity-V".into();
    let mut arial = Type0Font::identity_h("Arial");
    arial.descriptor = Some(FontDescriptor::default());
    arial.default_width = Some(1000.0);
    arial.widths = Some("3 [278] 36 [667 667 722] 68 [556]".into());
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font("CIdH", FontDict::Type0(arial))
        .add_font("CJp", FontDict::Type0(japan1))
        .add_font("CJv", FontDict::Type0(vertical))
        .raw_content(
            "BT /CIdH 11 Tf 72 700 Td <00240025002600030044> Tj ET \
             BT /CJp 11 Tf 72 680 Td <93FA967B8CEA4142> Tj ET \
             BT /CJv 11 Tf 300 600 Td <002200230024> Tj ET",
        );
    out.push(sample("builder_type0", "builder", b.to_bytes(), 1));

    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    let ocg = p.add_ocg("HiddenForm", false);
    p.add_form_xobject(
        "Fm0",
        [0.0, 0.0, 595.0, 842.0],
        "BT /Helv 11 Tf 72 680 Td (InForm) Tj ET",
    )
    .add_form_xobject(
        "Clip",
        [0.0, 0.0, 50.0, 50.0],
        "BT /Helv 11 Tf 100 600 Td (OutsideBBox) Tj ET BT /Helv 11 Tf 5 20 Td (InBBox) Tj ET",
    )
    .add_form_xobject(
        "Oc",
        [0.0, 0.0, 595.0, 842.0],
        "BT /Helv 11 Tf 72 500 Td (FormOCG) Tj ET",
    )
    .set_form_ocg("Oc", &ocg)
    .raw_content(
        "BT /Helv 11 Tf 72 700 Td (Before) Tj ET /Fm0 Do BT /Helv 11 Tf 72 660 Td (After) Tj ET \
         /Clip Do /Oc Do",
    );
    out.push(sample("builder_forms", "builder", b.to_bytes(), 1));

    let ap = "BT /Helv 11 Tf 2 5 Td (AnnotText) Tj ET";
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .insert_text((72.0, 72.0), "Body", HELV11)
        .expect("ASCII");
    let p = b.add_page_a4();
    p.insert_text((72.0, 72.0), "Body", HELV11).expect("ASCII");
    p.add_annotation(Annotation::new("FreeText", [72.0, 600.0, 272.0, 620.0]).with_appearance(ap))
        .add_annotation(
            Annotation::new("FreeText", [72.0, 570.0, 272.0, 590.0])
                .with_flags(Annotation::FLAG_HIDDEN)
                .with_appearance(ap),
        )
        .add_annotation(
            Annotation::new("FreeText", [72.0, 540.0, 272.0, 560.0])
                .with_flags(Annotation::FLAG_NO_VIEW)
                .with_appearance(ap),
        )
        .add_annotation(Annotation::new("Popup", [72.0, 510.0, 272.0, 530.0]).with_appearance(ap))
        .add_annotation(Annotation::new("Link", [72.0, 480.0, 272.0, 500.0]).with_appearance(ap))
        .add_annotation(
            Annotation::new("Widget", [72.0, 450.0, 272.0, 470.0]).with_appearance_spec(
                Appearance::States {
                    states: vec![
                        ("On".into(), AppearanceStream::new(ap)),
                        ("Off".into(), AppearanceStream::new("")),
                    ],
                    current: "On".into(),
                },
            ),
        )
        .add_annotation(
            Annotation::new("Stamp", [72.0, 400.0, 172.0, 440.0]).with_appearance_spec(
                Appearance::Stream(AppearanceStream {
                    content: "BT /Helv 11 Tf 2 5 Td (Scaled) Tj ET".into(),
                    bbox: Some([0.0, 0.0, 200.0, 80.0]),
                    matrix: Some([0.5, 0.0, 0.0, 0.5, 0.0, 0.0]),
                }),
            ),
        )
        .add_annotation(
            Annotation::new("FreeText", [72.0, 360.0, 272.0, 380.0])
                .with_contents("NoAppearance", "/Helv 11 Tf 0 g"),
        );
    out.push(sample("builder_annotations", "builder", b.to_bytes(), 2));

    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    let on = p.add_ocg("VisibleOCG", true);
    let off = p.add_ocg("HiddenOCG", false);
    p.raw_content(&format!(
        "/OC /{} BDC BT /Helv 11 Tf 72 700 Td (VisibleOCG) Tj ET EMC \
         /OC /{} BDC BT /Helv 11 Tf 72 680 Td (HiddenOCG) Tj ET EMC \
         /Span << /ActualText (Hello) >> BDC BT /Helv 11 Tf 72 660 Td (H e l l o) Tj ET EMC \
         /Span << /ActualText <FEFF00660069> >> BDC BT /Helv 11 Tf 72 640 Td <AD> Tj ET EMC",
        on.name(),
        off.name()
    ));
    out.push(sample("builder_ocg_actualtext", "builder", b.to_bytes(), 1));

    let mut b = PdfBuilder::new();
    for (i, (w, h)) in [(595.0, 842.0), (200.0, 100.0), (842.0, 595.0)]
        .into_iter()
        .enumerate()
    {
        b.add_page(w, h)
            .insert_text((10.0, 50.0), &format!("Page {}", i + 1), HELV11)
            .expect("ASCII");
    }
    out.push(sample("builder_multi_page", "builder", b.to_bytes(), 3));

    let mut b = PdfBuilder::new();
    b.add_page_a4();
    out.push(sample("builder_empty_page", "builder", b.to_bytes(), 1));

    out.push(sample(
        "builder_no_pages",
        "builder",
        PdfBuilder::new().to_bytes(),
        0,
    ));
    out
}

/// Appendix C "hostile (committed)" sizes plus every `TreeShape`.
pub fn hostile_samples() -> Vec<Sample> {
    let mut out = vec![
        // H2a: the page dictionary nests deeper than hayro's object-depth cap,
        // so it cannot be read and the walk drops the page. The oracle keeps
        // the page (text `''`, spec Appendix A.5).
        sample(
            "deep_obj_20k",
            "hostile",
            hostile::deep_nesting_in_page_dict(20_000),
            0,
        )
        .oracle(Oracle::Pages(1)),
        sample(
            "deep_content_20k",
            "hostile",
            hostile::deep_nesting_in_content(20_000),
            1,
        ),
        sample(
            "deep_resources_20k",
            "hostile",
            hostile::deep_nesting_in_resources(20_000),
            1,
        ),
        sample(
            "deep_type4_2k",
            "hostile",
            hostile::deep_nesting_in_type4_function(2_000),
            1,
        ),
        sample("deep_tree_200", "hostile", hostile::deep_page_tree(200), 1),
        sample(
            "flate_bomb_8mib",
            "hostile",
            hostile::flate_bomb(8 << 20),
            1,
        ),
        sample(
            "many_ops_200k",
            "hostile",
            hostile::operator_flood(200_000),
            1,
        ),
        sample("form_fanout_12", "hostile", hostile::form_fanout(12), 1),
        sample(
            "form_self_recursion",
            "hostile",
            hostile::form_self_recursion(),
            1,
        ),
        sample("nested_forms_60", "hostile", hostile::nested_forms(60), 1),
        sample(
            "tree_flat_10",
            "hostile",
            hostile::page_tree(TreeShape::Flat(10)),
            10,
        ),
        sample(
            "tree_fanout_10x2",
            "hostile",
            hostile::page_tree(TreeShape::Fanout {
                leaves_per_node: 10,
                levels: 2,
            }),
            100,
        ),
        sample(
            "tree_dup_leaf",
            "hostile",
            hostile::page_tree(TreeShape::DuplicateLeaf),
            2,
        ),
        sample(
            "tree_lying_count_5",
            "hostile",
            hostile::page_tree(TreeShape::LyingCount { claimed: 5 }),
            100,
        )
        // The oracle trusts the root's /Count ("too many kids in page tree").
        .oracle(Oracle::Pages(5)),
        sample(
            "tree_typeless_leaf",
            "hostile",
            hostile::page_tree(TreeShape::TypelessLeaf),
            2,
        ),
        sample(
            "tree_direct_kid",
            "hostile",
            hostile::page_tree(TreeShape::DirectDictKid),
            2,
        ),
        // hayro's indirect-object cycle check stops the self-reference (spec
        // A.7: one page); the oracle reports /Count 2 and raises "cycle in page
        // tree" when page 2 is loaded (spec A.5).
        sample(
            "tree_cycle",
            "hostile",
            hostile::page_tree(TreeShape::Cycle),
            1,
        )
        .oracle(Oracle::PagesLoadRaises(2)),
    ];
    let mut big = std::env::temp_dir();
    // Unique per call, not just per process: tests run on parallel threads.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    big.push(format!(
        "pdf-extract-testkit-big-{}-{seq}.pdf",
        std::process::id()
    ));
    hostile::big_tree_to_path(1_000, &big).expect("temp file");
    let bytes = std::fs::read(&big).expect("just written");
    let _ = std::fs::remove_file(&big);
    out.push(sample("big_tree_1000", "hostile", bytes, 1_000));
    out
}
