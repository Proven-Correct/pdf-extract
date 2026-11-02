//! §4.9 geometry: origins, direction, size and advance in page space.

use hayro_interpret::TransformExt;
use kurbo::{Point, Vec2};
use pdf_extract_testkit::{
    CidFontSubtype, FontDescriptor, FontDict, PdfBuilder, TextStyle, Type0Font, text_width,
};

use super::{assert_close, assert_near, extract, texts};

fn ab_at_100_100(rotation: Option<i32>) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let page = b.add_page_a4();
    if let Some(r) = rotation {
        page.set_rotation(r);
    }
    page.insert_text((100.0, 100.0), "AB", TextStyle::size(11.0))
        .expect("WinAnsi");
    b.to_bytes()
}

#[test]
fn insert_text_gives_afm_origins_and_advances() {
    let p = extract(ab_at_100_100(None));
    assert_eq!(texts(&p), ["A", "B"]);
    let a_width = f64::from(text_width("A", 11.0).expect("WinAnsi"));
    assert_close(a_width as f32, 0.667 * 11.0);
    let (a, b) = (&p.chars[0], &p.chars[1]);
    assert_near(a.origin, (100.0, 742.0));
    assert_near(b.origin, (100.0 + a_width, 742.0));
    for c in [a, b] {
        assert_close(c.size, 11.0);
        assert_near(c.dir, (1.0, 0.0));
        assert!(!c.invisible);
    }
    assert_close(a.advance, a_width);
    assert_close(
        b.advance,
        f64::from(text_width("B", 11.0).expect("WinAnsi")),
    );
}

#[test]
fn a_rotated_page_follows_its_initial_transform() {
    let bytes = ab_at_100_100(Some(90));
    let pdf = hayro_syntax::Pdf::new(bytes.clone()).expect("opens");
    let t = pdf.pages()[0].initial_transform(false).to_kurbo();
    let p = extract(bytes);
    assert_eq!(texts(&p), ["A", "B"]);
    let a_width = f64::from(text_width("A", 11.0).expect("WinAnsi"));
    let a = t * Point::new(100.0, 742.0);
    let b = t * Point::new(100.0 + a_width, 742.0);
    let dir = (t * Point::new(1.0, 0.0) - t * Point::ORIGIN).normalize();
    assert_near(p.chars[0].origin, (a.x, a.y));
    assert_near(p.chars[1].origin, (b.x, b.y));
    assert_near(p.chars[0].dir, (dir.x, dir.y));
    assert!(
        dir.y.abs() > 0.99,
        "a quarter turn: the baseline runs vertically"
    );
    assert_close(p.chars[0].size, 11.0);
    assert_close(p.chars[0].advance, a_width);
}

#[test]
fn a_quarter_turn_text_matrix_runs_up_the_page() {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .raw_content("BT /Helv 11 Tf 0 1 -1 0 300 400 Tm (AB) Tj ET");
    let p = extract(b.to_bytes());
    let a_width = f64::from(text_width("A", 11.0).expect("WinAnsi"));
    assert_near(p.chars[0].dir, (0.0, 1.0));
    assert_near(p.chars[0].origin, (300.0, 400.0));
    assert_near(p.chars[1].origin, (300.0, 400.0 + a_width));
    assert_close(p.chars[0].size, 11.0);
    assert_close(p.chars[0].advance, a_width);
}

#[test]
fn scaling_rise_and_spacing() {
    // Tz 50 halves advances but not the size; Ts lifts the origin; Tc moves
    // the next origin, not the advance. (Text state outlives `ET`, hence
    // the resets.)
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(
        "BT /Helv 10 Tf 50 Tz 100 100 Td (A) Tj ET \
         BT /Helv 10 Tf 100 Tz 5 Ts 100 200 Td (A) Tj ET \
         BT /Helv 10 Tf 0 Ts 3 Tc 100 300 Td (AB) Tj ET",
    );
    let p = extract(b.to_bytes());
    let a10 = 6.67;
    assert_close(p.chars[0].size, 10.0);
    assert_close(p.chars[0].advance, a10 / 2.0);
    assert_near(p.chars[1].origin, (100.0, 205.0));
    assert_close(p.chars[1].size, 10.0);
    assert_close(p.chars[2].advance, a10);
    assert_near(p.chars[3].origin, (100.0 + a10 + 3.0, 300.0));
}

#[test]
fn degenerate_sizes_keep_the_glyph() {
    // T25: a zero-size glyph is kept when it has Unicode. With `Tz 0` the x
    // axis vanishes but the size is still the font size, and the direction
    // comes from the y axis.
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .raw_content("BT /Helv 0 Tf 100 100 Td (Z) Tj ET BT /Helv 11 Tf 0 Tz 100 200 Td (W) Tj ET");
    let p = extract(b.to_bytes());
    assert_eq!(texts(&p), ["Z", "W"]);
    assert_close(p.chars[0].size, 0.0);
    assert_near(p.chars[0].dir, (1.0, 0.0));
    assert_near(p.chars[0].origin, (100.0, 100.0));
    assert_close(p.chars[1].size, 11.0);
    assert_near(p.chars[1].dir, (1.0, 0.0));
    assert_close(p.chars[1].advance, 0.0);
}

#[test]
fn vertical_writing_keeps_the_text_matrix_direction() {
    // T29/KP12: `dir` is the x axis whatever the writing mode, so vertical
    // glyphs have no advance along it (each will form its own line).
    let vertical = Type0Font {
        encoding: "Identity-V".to_owned(),
        ordering: "Japan1".to_owned(),
        supplement: 6,
        cid_subtype: CidFontSubtype::Type0,
        descriptor: Some(FontDescriptor::default()),
        ..Type0Font::identity_h("MS-Mincho")
    };
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font("V", FontDict::Type0(vertical))
        .raw_content("BT /V 10 Tf 300 600 Td <002200230024> Tj ET");
    let p = extract(b.to_bytes());
    assert_eq!(texts(&p), ["A", "B", "C"]);
    for c in &p.chars {
        assert_near(c.dir, (1.0, 0.0));
        assert_close(c.advance, 0.0);
        assert_close(c.size, 10.0);
    }
    // Each glyph sits below the previous one.
    let down = Vec2::new(
        f64::from(p.chars[1].origin.0 - p.chars[0].origin.0),
        f64::from(p.chars[1].origin.1 - p.chars[0].origin.1),
    );
    assert!(down.y < -5.0 && down.x.abs() < 1e-3, "{down:?}");
}

#[test]
fn geometry_beyond_f32_is_dropped() {
    // Review finding: a CTM product finite in f64 but not in f32 would put
    // `inf` into a record.
    let big = format!("{}.0", "1".repeat(38));
    let cm = format!("{big} 0 0 {big} 0 0 cm ").repeat(9);
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(&format!(
        "q {cm} BT /Helv 11 Tf (A) Tj ET Q BT /Helv 11 Tf 100 100 Td (B) Tj ET"
    ));
    let p = extract(b.to_bytes());
    assert_eq!(texts(&p), ["B"]);
}
