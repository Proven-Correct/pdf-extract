//! Output-shape invariants of spec §4.10 (and internal survey §13.5) as property tests over
//! randomly generated builder pages:
//!
//! - the text is empty or ends with `'\n'`;
//! - under `BlockSeparator::Newline` it contains no `"\n\n"` unless a
//!   mapping step emitted a line feed (here: only the Type 3 code 10, which
//!   is unmapped and emitted raw);
//! - it is exactly the lines' texts, each followed by `'\n'`, and no line is
//!   empty;
//! - `text()` equals `text_page().to_text()` for both separators;
//! - `BlankLine` output equals the `Newline` output with exactly one `'\n'`
//!   inserted at each interior block boundary (spec §4.10, revision 7);
//! - the output is identical across 8 threads × 50 repetitions.
//!
//! The generator is a seeded SplitMix64 (no `proptest`: it is not in the
//! dependency graph, and a fixed-seed generator is enough to make the cases
//! reproducible). Set `PDF_EXTRACT_SHAPE_SEED` to explore other seeds; every
//! failure message carries the seed and the case number, and
//! `PDF_EXTRACT_SHAPE_CASES` changes the number of cases.

use std::sync::Barrier;
use std::thread;

use pdf_extract_core::{BlockSeparator, Document, OpenOptions, TextOptions, Unmapped};
use pdf_extract_testkit::{Annotation, FontDict, PdfBuilder, TextStyle, Type3Font, Type3Glyph};

/// Default seed; any fixed value works.
const SEED: u64 = 0x5EED_0010_7E47_5A9E;
const CASES: usize = 400;

/// SplitMix64: tiny, well-distributed, and fully determined by its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// Uniform in `[lo, hi)`, rounded to 1/100 so the content stream holds
    /// exactly the value used.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        let t = (self.below(10_000) as f32) / 10_000.0;
        ((lo + (hi - lo) * t) * 100.0).round() / 100.0
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A Type 3 font whose codes 1, 9 and 10 are unmapped (emitted raw as
/// U+FFFD, TAB and LF) and code 65 is `A`.
fn type3() -> FontDict {
    let glyph = |code: u8, name: &str| Type3Glyph {
        code,
        name: name.to_owned(),
        width: 500.0,
        procedure: "500 0 d0 0 0 100 100 re f".to_owned(),
    };
    FontDict::Type3(Type3Font {
        font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        font_bbox: [0.0, 0.0, 500.0, 500.0],
        glyphs: vec![
            glyph(1, "g1"),
            glyph(9, "g9"),
            glyph(10, "g10"),
            glyph(65, "A"),
        ],
        to_unicode: None,
    })
}

const WORDS: &[&str] = &[
    "Alpha",
    "beta",
    "Gamma",
    "  ",
    " lead",
    "trail ",
    "x",
    "W",
    "a b c",
    "1.5",
    "caf\u{E9}",
    "A\u{A0}B",
    "\u{2014}",
    "...",
    "Hello   World",
];

/// One random page and whether it can emit a line feed from a mapping.
fn random_page(rng: &mut Rng) -> (Vec<u8>, bool) {
    let mut b = PdfBuilder::new();
    let (w, h) = if rng.chance(20) {
        (rng.range(50.0, 400.0), rng.range(50.0, 400.0))
    } else {
        (595.0, 842.0)
    };
    let page = b.add_page(w, h);
    if rng.chance(20) {
        page.set_rotation(90 * rng.below(4) as i32);
    }
    let mut may_emit_lf = false;
    let mut forms = 0;
    for _ in 0..rng.below(14) {
        match rng.below(10) {
            // Plain runs, the common case.
            0..=4 => {
                let style = TextStyle {
                    fontsize: if rng.chance(5) {
                        0.0
                    } else {
                        rng.range(1.0, 40.0)
                    },
                    render_mode: rng.below(8) as u8,
                    ..TextStyle::default()
                };
                let at = (rng.range(-20.0, w + 20.0), rng.range(-20.0, h + 20.0));
                page.insert_text(at, rng.pick(WORDS), style)
                    .expect("WinAnsi");
            }
            // A run with spacing operators and a rotated text matrix.
            5 | 6 => {
                let angle = f32::from(rng.below(360) as u16).to_radians();
                let (s, c) = angle.sin_cos();
                page.raw_content(&format!(
                    "BT /Helv {} Tf {} Tc {} Tw {} Tz {c:.4} {s:.4} {:.4} {c:.4} {} {} Tm ({}) Tj ET",
                    rng.range(4.0, 30.0),
                    rng.range(-2.0, 10.0),
                    rng.range(-5.0, 30.0),
                    rng.range(20.0, 150.0),
                    -s,
                    rng.range(0.0, w),
                    rng.range(0.0, h),
                    rng.pick(&["Rot", "a b", "spaced out", "Q"]),
                ));
            }
            // A TJ array with adjustments.
            7 => {
                page.raw_content(&format!(
                    "BT /Helv 11 Tf {} {} Td [(A) {} (B) {} (C)] TJ ET",
                    rng.range(0.0, w),
                    rng.range(0.0, h),
                    rng.range(-3000.0, 3000.0),
                    rng.range(-3000.0, 3000.0),
                ));
            }
            // Text inside a Form XObject (block boundary markers).
            8 => {
                let name = format!("Fm{forms}");
                forms += 1;
                page.add_form_xobject(
                    &name,
                    [0.0, 0.0, w, h],
                    &format!(
                        "BT /Helv 11 Tf {} {} Td (In form) Tj ET",
                        rng.range(0.0, w),
                        rng.range(0.0, h)
                    ),
                )
                .raw_content(&format!("q /{name} Do Q"));
            }
            // Type 3 codes that are unmapped, including line feed.
            _ => {
                let codes = ["41", "4109", "0A41", "41010A41"];
                let pick = rng.pick(&codes);
                may_emit_lf |= pick.contains("0A");
                page.add_font("T3", type3()).raw_content(&format!(
                    "BT /T3 {} Tf {} {} Td <{pick}> Tj ET",
                    rng.range(5.0, 30.0),
                    rng.range(0.0, w),
                    rng.range(0.0, h)
                ));
            }
        }
    }
    if rng.chance(15) {
        page.add_annotation(
            Annotation::new("FreeText", [10.0, 10.0, 200.0, 40.0])
                .with_appearance("BT /Helv 9 Tf 2 5 Td (Annotated) Tj ET"),
        );
    }
    (b.to_bytes(), may_emit_lf)
}

/// Text of page 0 under both separators, plus what the shape checks need.
struct Observed {
    newline: String,
    blank: String,
    /// Line texts, grouped by block.
    blocks: Vec<Vec<String>>,
}

/// Newline output with exactly one '\n' inserted at each interior block
/// boundary: what `BlankLine` must give (spec §4.10, revision 7).
fn with_blank_lines(blocks: &[Vec<String>]) -> String {
    blocks
        .iter()
        .map(|b| b.iter().map(|l| format!("{l}\n")).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn observe(bytes: &[u8], unmapped: Unmapped) -> Observed {
    let doc = Document::from_bytes(bytes.to_vec(), &OpenOptions::default()).expect("opens");
    let page = doc.page(0).expect("page 0");
    let opts = |sep| TextOptions {
        unmapped,
        block_separator: sep,
        ..TextOptions::default()
    };
    let tp = page
        .text_page(&opts(BlockSeparator::Newline))
        .expect("extracts");
    let newline = page.text(&opts(BlockSeparator::Newline)).expect("extracts");
    let blank = page
        .text(&opts(BlockSeparator::BlankLine))
        .expect("extracts");
    assert_eq!(
        newline,
        tp.to_text(BlockSeparator::Newline),
        "text() vs to_text()"
    );
    assert_eq!(
        blank,
        tp.to_text(BlockSeparator::BlankLine),
        "text() vs to_text()"
    );
    Observed {
        newline,
        blank,
        blocks: tp
            .blocks()
            .iter()
            .map(|b| b.lines().iter().map(|l| l.text()).collect())
            .collect(),
    }
}

fn check_shape(o: &Observed, may_emit_lf: bool, ctx: &str) {
    let t = &o.newline;
    assert!(t.is_empty() || t.ends_with('\n'), "{ctx}: {t:?}");
    if !may_emit_lf {
        assert!(!t.contains("\n\n"), "{ctx}: {t:?}");
    }
    assert!(o.blocks.iter().all(|b| !b.is_empty()), "{ctx}: empty block");
    assert!(
        o.blocks.iter().flatten().all(|l| !l.is_empty()),
        "{ctx}: empty line"
    );
    let rebuilt: String = o
        .blocks
        .iter()
        .flatten()
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(*t, rebuilt, "{ctx}");
    // BlankLine == Newline plus exactly one '\n' per interior block
    // boundary (so a single-block page is identical).
    assert_eq!(o.blank, with_blank_lines(&o.blocks), "{ctx}");
    if o.blocks.len() <= 1 {
        assert_eq!(o.blank, *t, "{ctx}");
    }
    if t.is_empty() {
        assert!(o.blocks.is_empty(), "{ctx}");
    }
}

#[test]
fn shape_invariants_hold_on_random_pages() {
    let seed = env_or("PDF_EXTRACT_SHAPE_SEED", SEED);
    let cases = env_or("PDF_EXTRACT_SHAPE_CASES", CASES);
    let mut rng = Rng(seed);
    let mut non_empty = 0;
    for case in 0..cases {
        let (bytes, may_emit_lf) = random_page(&mut rng);
        let unmapped = [Unmapped::Cid, Unmapped::Replacement, Unmapped::Skip][case % 3];
        let ctx = format!("seed {seed:#x}, case {case}, {unmapped:?}");
        let o = observe(&bytes, unmapped);
        check_shape(&o, may_emit_lf, &ctx);
        non_empty += usize::from(!o.newline.is_empty());
    }
    // The generator must actually produce text most of the time, or the
    // checks above are vacuous.
    assert!(
        non_empty * 2 > cases,
        "only {non_empty} of {cases} pages had text"
    );
}

#[test]
fn separator_adds_only_newlines_between_blocks() {
    // Spec §4.10 (revision 7): `BlankLine` output is the `Newline` output
    // with exactly one '\n' inserted at each interior block boundary, so
    // `len(text.strip())` differs by the number of boundaries.
    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    for (i, y) in [100.0, 114.0, 400.0, 414.0, 700.0].into_iter().enumerate() {
        p.insert_text((100.0, y), &format!("Line {i}"), TextStyle::default())
            .expect("text");
    }
    let o = observe(&b.to_bytes(), Unmapped::Cid);
    assert_eq!(o.blocks.len(), 3);
    assert_eq!(o.newline, "Line 0\nLine 1\nLine 2\nLine 3\nLine 4\n");
    assert_eq!(o.blank, "Line 0\nLine 1\n\nLine 2\nLine 3\n\nLine 4\n");
    assert_eq!(o.blank, with_blank_lines(&o.blocks));
    assert_eq!(o.blank.trim().len(), o.newline.trim().len() + 2);
}

#[test]
fn output_is_identical_across_threads_and_repetitions() {
    let seed = env_or("PDF_EXTRACT_SHAPE_SEED", SEED);
    let mut rng = Rng(seed ^ 0xC0FF_EE00);
    let pages: Vec<Vec<u8>> = (0..16).map(|_| random_page(&mut rng).0).collect();
    let docs: Vec<Document> = pages
        .iter()
        .map(|p| Document::from_bytes(p.clone(), &OpenOptions::default()).expect("opens"))
        .collect();
    let run = |docs: &[Document]| -> Vec<String> {
        docs.iter()
            .map(|d| {
                d.page(0)
                    .expect("page")
                    .text(&TextOptions::default())
                    .expect("text")
            })
            .collect()
    };
    let expected = run(&docs);
    let start = Barrier::new(8);
    thread::scope(|s| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                s.spawn(|| {
                    start.wait();
                    (0..50).map(|_| run(&docs)).collect::<Vec<_>>()
                })
            })
            .collect();
        for w in workers {
            for (rep, got) in w.join().expect("no panic").into_iter().enumerate() {
                assert_eq!(got, expected, "seed {seed:#x}, repetition {rep}");
            }
        }
    });
}
