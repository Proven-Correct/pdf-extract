//! Corpus D source pages (run 0002 §3.8, Appendix C): the vector text pages
//! that `tools/oracle/mkcorpusd.py` rasterises with the oracle's renderer,
//! degrades and re-wraps as image-only PDFs.
//!
//! Each [`Source`] is one builder page plus its ground truth, the exact text
//! the builder was given, in reading order (lines joined by `\n`). The
//! layouts live here, next to the builder, so the ground truth and the
//! drawing come from the same data and cannot drift apart; everything
//! raster-side (resolution, skew, blur, JPEG) is the oracle script's, except
//! the noise variants, which are built here at test time from the committed
//! clean files (`noise`, spec revision 5).
//!
//! Text is ASCII Helvetica/WinAnsi only: OCR error rates are measured
//! against this text, and characters outside ASCII would make the metric
//! depend on how the model spells typographic punctuation.

use std::fmt::Write as _;

mod noise;

pub use noise::{
    Built, NOISE_SEED, ScanPage, add_noise, build_noise, default_build_dir, noise_field,
    noise_names, noise_variant, noise_variant_page, pure_noise_page, scan_pdf,
};

use crate::afm::{encode_win_ansi, text_width};
use crate::builder::{PdfBuilder, TextStyle};
use crate::pdfwriter::{hex_string, num};

/// One corpus-D source page.
#[derive(Clone, Debug)]
pub struct Source {
    /// Recipe name (`clean_body_11pt`, ...); the corpus file names derive
    /// from it.
    pub name: &'static str,
    /// The one-page vector PDF.
    pub pdf: Vec<u8>,
    /// Ground truth: the text drawn, in reading order, `\n` after each line.
    pub truth: String,
}

/// A public-domain passage (the opening of the United States Declaration
/// of Independence, 1776), the body text of most recipes. Fixed here so
/// the ground truth never depends on anything outside the repository.
const PASSAGE: &str = "When in the Course of human events, it becomes necessary for one \
people to dissolve the political bands which have connected them with another, and to \
assume among the powers of the earth, the separate and equal station to which the Laws of \
Nature and of Nature's God entitle them, a decent respect to the opinions of mankind \
requires that they should declare the causes which impel them to the separation. We hold \
these truths to be self-evident, that all men are created equal, that they are endowed by \
their Creator with certain unalienable Rights, that among these are Life, Liberty and the \
pursuit of Happiness. That to secure these rights, Governments are instituted among Men, \
deriving their just powers from the consent of the governed, That whenever any Form of \
Government becomes destructive of these ends, it is the Right of the People to alter or to \
abolish it, and to institute new Government, laying its foundation on such principles and \
organizing its powers in such form, as to them shall seem most likely to effect their \
Safety and Happiness. Prudence, indeed, will dictate that Governments long established \
should not be changed for light and transient causes; and accordingly all experience hath \
shewn, that mankind are more disposed to suffer, while evils are sufferable, than to right \
themselves by abolishing the forms to which they are accustomed. But when a long train of \
abuses and usurpations, pursuing invariably the same Object evinces a design to reduce them \
under absolute Despotism, it is their right, it is their duty, to throw off such \
Government, and to provide new Guards for their future security. Such has been the patient \
sufferance of these Colonies; and such is now the necessity which constrains them to alter \
their former Systems of Government. The history of the present King of Great Britain is a \
history of repeated injuries and usurpations, all having in direct object the establishment \
of an absolute Tyranny over these States. To prove this, let Facts be submitted to a candid \
world. He has refused his Assent to Laws, the most wholesome and necessary for the public \
good. He has forbidden his Governors to pass Laws of immediate and pressing importance, \
unless suspended in their operation till his Assent should be obtained; and when so \
suspended, he has utterly neglected to attend to them. He has refused to pass other Laws \
for the accommodation of large districts of people, unless those people would relinquish \
the right of Representation in the Legislature, a right inestimable to them and formidable \
to tyrants only. He has called together legislative bodies at places unusual, \
uncomfortable, and distant from the depository of their public Records, for the sole \
purpose of fatiguing them into compliance with his measures. He has dissolved \
Representative Houses repeatedly, for opposing with manly firmness his invasions on the \
rights of the people. He has refused for a long time, after such dissolutions, to cause \
others to be elected; whereby the Legislative powers, incapable of Annihilation, have \
returned to the People at large for their exercise; the State remaining in the mean time \
exposed to all the dangers of invasion from without, and convulsions within. He has \
endeavoured to prevent the population of these States; for that purpose obstructing the \
Laws for Naturalization of Foreigners; refusing to pass others to encourage their \
migrations hither, and raising the conditions of new Appropriations of Lands. He has \
obstructed the Administration of Justice, by refusing his Assent to Laws for establishing \
Judiciary powers. He has made Judges dependent on his Will alone, for the tenure of their \
offices, and the amount and payment of their salaries. He has erected a multitude of New \
Offices, and sent hither swarms of Officers to harass our people, and eat out their \
substance. He has kept among us, in times of peace, Standing Armies without the Consent of \
our legislatures. He has affected to render the Military independent of and superior to \
the Civil power. He has combined with others to subject us to a jurisdiction foreign to \
our constitution, and unacknowledged by our laws; giving his Assent to their Acts of \
pretended Legislation.";

/// Words of [`PASSAGE`] starting at word `skip`, cycling if a layout
/// needs more.
fn words(skip: usize) -> impl Iterator<Item = &'static str> {
    let all: Vec<&'static str> = PASSAGE.split_whitespace().collect();
    let n = all.len();
    (skip..).map(move |i| all[i % n])
}

/// Greedy word wrap: `count` lines no wider than `width` points at `size`,
/// taking words from `src`.
fn wrap(
    src: &mut impl Iterator<Item = &'static str>,
    size: f32,
    width: f32,
    count: usize,
) -> Vec<String> {
    let mut lines = Vec::with_capacity(count);
    let mut pending: Option<&str> = None;
    while lines.len() < count {
        let mut line = String::new();
        loop {
            let w = pending
                .take()
                .or_else(|| src.next())
                .expect("endless words");
            let candidate = if line.is_empty() {
                w.to_owned()
            } else {
                format!("{line} {w}")
            };
            if line.is_empty() || width_of(&candidate, size) <= width {
                line = candidate;
            } else {
                pending = Some(w);
                break;
            }
        }
        lines.push(line);
    }
    lines
}

fn width_of(text: &str, size: f32) -> f32 {
    text_width(text, size).expect("corpus text is WinAnsi")
}

/// Draws `lines` from the top-left `(x, y)` with `leading` points between
/// baselines.
fn draw(page: &mut crate::PageBuilder, lines: &[String], x: f32, y: f32, size: f32, leading: f32) {
    for (i, line) in lines.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let baseline = y + leading * i as f32;
        page.insert_text((x, baseline), line, TextStyle::size(size))
            .expect("corpus text is WinAnsi");
    }
}

fn truth(lines: &[String]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(l);
        out.push('\n');
    }
    out
}

const A4: (f32, f32) = (595.0, 842.0);
const LETTER: (f32, f32) = (612.0, 792.0);
const MARGIN: f32 = 72.0;

/// An A4 page of 40 wrapped lines at `size` points.
fn body(name: &'static str, size: f32) -> Source {
    let lines = wrap(&mut words(0), size, A4.0 - 2.0 * MARGIN, 40);
    let mut b = PdfBuilder::new();
    draw(
        b.add_page(A4.0, A4.1),
        &lines,
        MARGIN,
        MARGIN,
        size,
        size * 1.35,
    );
    Source {
        name,
        pdf: b.to_bytes(),
        truth: truth(&lines),
    }
}

/// Letter, two 11 pt columns of 40 lines; the truth reads the left column
/// first, as Tesseract's automatic layout analysis should.
fn two_column() -> Source {
    let (size, gutter) = (11.0, 24.0);
    let col = (LETTER.0 - 2.0 * MARGIN - gutter) / 2.0;
    let mut src = words(120);
    let left = wrap(&mut src, size, col, 40);
    let right = wrap(&mut src, size, col, 40);
    let mut b = PdfBuilder::new();
    let page = b.add_page(LETTER.0, LETTER.1);
    draw(page, &left, MARGIN, 60.0, size, 16.5);
    draw(page, &right, MARGIN + col + gutter, 60.0, size, 16.5);
    let mut t = truth(&left);
    t.push_str(&truth(&right));
    Source {
        name: "clean_two_column",
        pdf: b.to_bytes(),
        truth: t,
    }
}

/// A4: an 18 pt heading, 11 pt body and an 8 pt footnote.
fn heading_footnote() -> Source {
    let width = A4.0 - 2.0 * MARGIN;
    let heading = vec!["Of the Causes Which Impel the Separation".to_owned()];
    let mut src = words(260);
    let body = wrap(&mut src, 11.0, width, 28);
    let note = wrap(&mut src, 8.0, width, 3);
    let mut b = PdfBuilder::new();
    let page = b.add_page(A4.0, A4.1);
    draw(page, &heading, MARGIN, 90.0, 18.0, 24.0);
    draw(page, &body, MARGIN, 130.0, 11.0, 15.0);
    draw(page, &note, MARGIN, 760.0, 8.0, 10.5);
    let mut t = truth(&heading);
    t.push_str(&truth(&body));
    t.push_str(&truth(&note));
    Source {
        name: "clean_heading_footnote",
        pdf: b.to_bytes(),
        truth: t,
    }
}

/// A4: 20 numbered items, 11 pt, one line each.
fn numbered_list() -> Source {
    let mut src = words(400);
    let items: Vec<String> = (1..=20)
        .map(|i| {
            let text: Vec<&str> = src.by_ref().take(6 + i % 5).collect();
            format!("{i}. {}.", text.join(" ").trim_end_matches([',', ';', '.']))
        })
        .collect();
    let mut b = PdfBuilder::new();
    draw(b.add_page(A4.0, A4.1), &items, MARGIN, MARGIN, 11.0, 22.0);
    Source {
        name: "clean_numbered_list",
        pdf: b.to_bytes(),
        truth: truth(&items),
    }
}

/// Thirty reporter citations (`Name v. Name, 123 F.3d 456, 461 (9th Cir.
/// 2001).`): dense digits, periods and abbreviations, the strings OCR at
/// 72 dpi garbles. A fixed linear congruential sequence picks the parts.
fn citation_lines() -> Vec<String> {
    const PARTIES: [&str; 12] = [
        "Anderson",
        "Baker",
        "Carter",
        "Dawson",
        "Ellison",
        "Fletcher",
        "Garrison",
        "Holloway",
        "Ingram",
        "Jefferson",
        "Kendall",
        "Lombard",
    ];
    const REPORTERS: [&str; 6] = ["F.3d", "F.2d", "F. Supp. 2d", "U.S.", "S. Ct.", "F.4th"];
    const COURTS: [&str; 6] = [
        "9th Cir.",
        "2d Cir.",
        "D. Mass.",
        "5th Cir.",
        "N.D. Cal.",
        "7th Cir.",
    ];
    let mut state: u32 = 20_261_003;
    let mut next = |n: u32| {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (state >> 16) % n
    };
    (0..30)
        .map(|_| {
            let a = PARTIES[next(12) as usize];
            let b = PARTIES[next(12) as usize];
            let reporter = REPORTERS[next(6) as usize];
            let (vol, page) = (100 + next(899), 1 + next(1200));
            let pin = page + 1 + next(40);
            let year = 1975 + next(50);
            if reporter == "U.S." || reporter == "S. Ct." {
                format!("{a} v. {b}, {vol} {reporter} {page}, {pin} ({year}).")
            } else {
                let court = COURTS[next(6) as usize];
                format!("{a} v. {b}, {vol} {reporter} {page}, {pin} ({court} {year}).")
            }
        })
        .collect()
}

fn citations() -> Source {
    let lines = citation_lines();
    let mut b = PdfBuilder::new();
    draw(b.add_page(A4.0, A4.1), &lines, MARGIN, MARGIN, 11.0, 20.0);
    Source {
        name: "clean_citations",
        pdf: b.to_bytes(),
        truth: truth(&lines),
    }
}

fn one_line() -> Source {
    let lines = vec!["This page is intentionally left blank except for this one line.".to_owned()];
    let mut b = PdfBuilder::new();
    draw(b.add_page(A4.0, A4.1), &lines, MARGIN, 200.0, 11.0, 15.0);
    Source {
        name: "clean_one_line",
        pdf: b.to_bytes(),
        truth: truth(&lines),
    }
}

/// A4 portrait MediaBox with `/Rotate 90`: the page displays as landscape,
/// and the text is drawn turned a quarter turn counter-clockwise in user
/// space so that it reads upright once the rotation is applied. A
/// renderer that ignored `/Rotate` would hand Tesseract sideways text.
fn rotate90() -> Source {
    let (size, leading) = (11.0, 15.0);
    // The displayed page is 842 x 595; a displayed point (X, Y), Y down,
    // is user-space (x, y) = (Y, X) under a clockwise quarter turn.
    let lines = wrap(&mut words(560), size, A4.1 - 2.0 * MARGIN, 25);
    let mut b = PdfBuilder::new();
    let page = b.add_page(A4.0, A4.1);
    page.set_rotation(90);
    for (i, line) in lines.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let display_y = MARGIN + leading * i as f32;
        let bytes = encode_win_ansi(line).expect("WinAnsi");
        let mut ops = String::new();
        let _ = write!(
            ops,
            "q BT /Helv {} Tf 0 1 -1 0 {} {} Tm {} Tj ET Q",
            num(size),
            num(display_y),
            num(MARGIN),
            hex_string(&bytes)
        );
        page.raw_content(&ops);
    }
    Source {
        name: "clean_rotate90",
        pdf: b.to_bytes(),
        truth: truth(&lines),
    }
}

/// A4 MediaBox with a Letter-height CropBox `[0 25 595 817]` (Letter's
/// 612 pt width does not fit inside A4's 595, so the box keeps A4's width)
/// and a margin note above and below it that lies outside the CropBox:
/// visible on the MediaBox, absent from the ground truth.
fn cropbox() -> Source {
    let lines = wrap(&mut words(700), 11.0, A4.0 - 2.0 * MARGIN, 38);
    let mut b = PdfBuilder::new();
    let page = b.add_page(A4.0, A4.1);
    page.set_crop_box([0.0, 25.0, A4.0, 817.0]);
    draw(page, &lines, MARGIN, 60.0, 11.0, 19.0);
    for y in [16.0, 836.0] {
        page.insert_text(
            (MARGIN, y),
            "MARGIN NOTE OUTSIDE THE CROP BOX",
            TextStyle::size(9.0),
        )
        .expect("WinAnsi");
    }
    Source {
        name: "clean_cropbox",
        pdf: b.to_bytes(),
        truth: truth(&lines),
    }
}

/// Thirty characters on an A4 page: below `extract`'s default `min_chars`
/// of 50 whether read from the text layer or recognised.
fn sparse() -> Source {
    let lines = vec!["Exhibit 12: Notice of Filings.".to_owned()];
    let mut b = PdfBuilder::new();
    draw(b.add_page(A4.0, A4.1), &lines, MARGIN, 120.0, 11.0, 15.0);
    Source {
        name: "sparse_below_min_chars",
        pdf: b.to_bytes(),
        truth: truth(&lines),
    }
}

/// Every source page, in catalogue order.
pub fn sources() -> Vec<Source> {
    vec![
        body("clean_body_09pt", 9.0),
        body("clean_body_10pt", 10.0),
        body("clean_body_11pt", 11.0),
        body("clean_body_12pt", 12.0),
        two_column(),
        heading_footnote(),
        numbered_list(),
        citations(),
        one_line(),
        rotate90(),
        cropbox(),
        sparse(),
    ]
}
