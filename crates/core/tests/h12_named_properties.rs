//! Tests of fork change H12: named properties through shared and inherited
//! resources (spec 0002 §3.2, O37, oracle §4.3; Appendix A.2).
//!
//! Two causes made a form leak text under `/OC /P1 BDC` when `P1` is an OFF
//! optional-content group named only in the page's `/Properties`:
//!
//! 1. hayro-syntax's cycle guard refuses a reference to any object already on
//!    the lookup's parent chain, so a form reached through the page's
//!    resources object (5) could not resolve its own `/Resources 5 0 R`; the
//!    form got empty resources. H12 resolves a referenced `/Resources` with a
//!    fresh lookup (`Resources::resolve_dict`).
//! 2. the `BDC` handler read only the innermost `/Properties`, without the
//!    parent walk every other resource getter performs. H12 adds
//!    `Resources::get_property` / `get_property_ref` and uses the walk.
//!
//! Fonts and nested forms never showed cause 1 because their getters walk the
//! parent chain, which reaches the same object 5. The `precedence_*` tests
//! isolate cause 1: there the parent chain holds a *different* entry for the
//! same name, so only the correctly resolved `/Resources` picks the right one.

mod common;

use common::{
    Collector, Run, assemble_pdf, interpret, on_small_stack, run_on_small_stack, run_page, stream,
};
use hayro_syntax::Pdf;
use hayro_syntax::object::dict::keys::NAME;
use hayro_syntax::object::{self, Dict, Name};
use hayro_syntax::page::Resources;

const HELV: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
const CATALOG_P1_OFF: &str = "<< /Type /Catalog /Pages 2 0 R \
     /OCProperties << /OCGs [9 0 R] /D << /OFF [9 0 R] >> >> >>";
const PAGE_CONTENT: &str = "BT /F 11 Tf 10 150 Td (PAGEVIS) Tj ET \
     /OC /P1 BDC BT /F 11 Tf 10 130 Td (HIDDENPAGE) Tj ET EMC /Fx Do";
const FORM_CONTENT: &str = "BT /F 11 Tf 10 100 Td (FORMVIS) Tj ET \
     /OC /P1 BDC BT /F 11 Tf 10 80 Td (HIDDENFORM) Tj ET EMC";

/// One Appendix A.2 shape.
struct Shape {
    /// The page's `/Resources` value.
    page_resources: &'static str,
    /// Object 5's (the shared resources object's) `/Properties` value.
    shared_properties: &'static str,
    /// The form's `/Resources` entry, including the key, or empty.
    form_resources: &'static str,
    /// Extra form dictionary entries (an `/OC` on the whole form).
    form_extra: &'static str,
    /// The form's content stream.
    form_content: &'static str,
}

const SHARED: Shape = Shape {
    page_resources: "5 0 R",
    shared_properties: "<< /P1 9 0 R >>",
    form_resources: "/Resources 5 0 R",
    form_extra: "",
    form_content: FORM_CONTENT,
};

/// Objects: 1 catalog (P1 = OCG 9, OFF), 2 pages, 3 page, 4 content, 5 the
/// shared resources object, 6 the form `/Fx`, 7 Helvetica, 8 a full copy of
/// 5, 9 the OCG, 10 an OCMD over 9, 11 an indirect `/Properties` dictionary,
/// 12 a resources dictionary holding only `/Properties`.
fn q8(shape: &Shape) -> Vec<u8> {
    let shared = format!(
        "<< /Font << /F 7 0 R >> /XObject << /Fx 6 0 R >> /Properties {} >>",
        shape.shared_properties
    );
    assemble_pdf(&[
        CATALOG_P1_OFF.to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources {} \
             /Contents 4 0 R >>",
            shape.page_resources
        ),
        stream("", PAGE_CONTENT),
        shared,
        stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 200 200] {} {}",
                shape.form_resources, shape.form_extra
            ),
            shape.form_content,
        ),
        HELV.to_owned(),
        "<< /Font << /F 7 0 R >> /Properties << /P1 9 0 R >> >>".to_owned(),
        "<< /Type /OCG /Name (P1) >>".to_owned(),
        "<< /Type /OCMD /OCGs [9 0 R] /P /AnyOn >>".to_owned(),
        "<< /P1 9 0 R >>".to_owned(),
        "<< /Properties << /P1 9 0 R >> >>".to_owned(),
    ])
}

fn text_of(bytes: Vec<u8>) -> String {
    let pdf = Pdf::new(bytes).expect("fixture opens");
    let page = &pdf.pages()[0];
    let out: Collector = interpret(&pdf, page);
    out.text
}

// ---------------------------------------------------------------------------
// The leaking shapes (Appendix A.2): hidden after H12.

#[test]
fn shared_resources_hide_the_form_s_hidden_content() {
    assert_eq!(text_of(q8(&SHARED)), "PAGEVISFORMVIS");
}

#[test]
fn shared_resources_with_indirect_properties_hide_it() {
    let shape = Shape {
        shared_properties: "11 0 R",
        ..SHARED
    };
    assert_eq!(text_of(q8(&shape)), "PAGEVISFORMVIS");
}

#[test]
fn shared_resources_with_an_ocmd_hide_it() {
    let shape = Shape {
        shared_properties: "<< /P1 10 0 R >>",
        ..SHARED
    };
    assert_eq!(text_of(q8(&shape)), "PAGEVISFORMVIS");
}

#[test]
fn a_form_without_resources_inherits_the_page_s_properties() {
    let shape = Shape {
        form_resources: "",
        ..SHARED
    };
    assert_eq!(text_of(q8(&shape)), "PAGEVISFORMVIS");
}

#[test]
fn a_form_with_direct_resources_without_properties_inherits_them() {
    let shape = Shape {
        form_resources: "/Resources << /Font << /F 7 0 R >> >>",
        ..SHARED
    };
    assert_eq!(text_of(q8(&shape)), "PAGEVISFORMVIS");
}

// ---------------------------------------------------------------------------
// The controls, which never leaked.

#[test]
fn controls_are_unchanged() {
    for (what, shape) in [
        (
            "direct resources with properties",
            Shape {
                form_resources: "/Resources << /Font << /F 7 0 R >> /Properties << /P1 9 0 R >> >>",
                ..SHARED
            },
        ),
        (
            "indirect full copy",
            Shape {
                form_resources: "/Resources 8 0 R",
                ..SHARED
            },
        ),
        (
            "indirect properties-only",
            Shape {
                form_resources: "/Resources 12 0 R",
                ..SHARED
            },
        ),
        (
            "shared, page resources direct",
            Shape {
                page_resources: "<< /Font << /F 7 0 R >> /XObject << /Fx 6 0 R >> \
                                 /Properties << /P1 9 0 R >> >>",
                ..SHARED
            },
        ),
    ] {
        assert_eq!(text_of(q8(&shape)), "PAGEVISFORMVIS", "{what}");
    }
}

#[test]
fn a_hidden_form_is_still_hidden_whole() {
    let shape = Shape {
        form_extra: "/OC 9 0 R",
        ..SHARED
    };
    assert_eq!(text_of(q8(&shape)), "PAGEVIS");
}

// ---------------------------------------------------------------------------
// Cause 1 alone: the form's `/Resources` must be the object it names, not
// whatever the parent chain happens to hold under the same name.

/// Page resources 5 name `P1` = OCG 9 (OFF) and draw form 6, whose direct
/// resources name `P1` = OCG 10 (ON) and hold `{inner_key}`; whatever 6 draws
/// names `/Resources 5 0 R` (an ancestor of its lookup) and marks its content
/// with `/OC /P1`. The right answer uses 5's `P1`, which is OFF.
fn precedence(inner_key: &str, inner_content: &str, extra: &[String]) -> Vec<u8> {
    let mut bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R \
         /OCProperties << /OCGs [9 0 R 10 0 R] /D << /OFF [9 0 R] >> >> >>"
            .to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 5 0 R \
         /Contents 4 0 R >>"
            .to_owned(),
        stream("", "BT /F 11 Tf 10 150 Td (PAGEVIS) Tj ET /Fa Do"),
        "<< /Font << /F 7 0 R >> /XObject << /Fa 6 0 R >> /Properties << /P1 9 0 R >> >>"
            .to_owned(),
        stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 200 200] \
                 /Resources << {inner_key} /Properties << /P1 10 0 R >> >>"
            ),
            inner_content,
        ),
        HELV.to_owned(),
        // 8: free for the caller's first extra object.
        String::new(),
        "<< /Type /OCG /Name (Off) >>".to_owned(),
        "<< /Type /OCG /Name (On) >>".to_owned(),
    ];
    // Objects 8 and 11 onwards come from the caller.
    let mut extra = extra.iter();
    bodies[7] = extra.next().cloned().unwrap_or_else(|| "null".to_owned());
    bodies.extend(extra.cloned());
    assemble_pdf(&bodies)
}

#[test]
fn precedence_a_nested_form_uses_the_resources_it_names() {
    let bytes = precedence(
        "/XObject << /Fb 8 0 R >>",
        "/Fb Do",
        &[stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200] /Resources 5 0 R",
            "BT /F 11 Tf 10 100 Td (FORMVIS) Tj ET \
             /OC /P1 BDC BT /F 11 Tf 10 80 Td (HIDDENFORM) Tj ET EMC",
        )],
    );
    assert_eq!(text_of(bytes), "PAGEVISFORMVIS");
}

/// A Type 3 font (object 8) whose one glyph `a` (procedure object 11) shows
/// `/OC /P1`-marked paths; `font_resources` and `proc_resources` are the
/// `/Resources` entries of the font and of the glyph procedure.
fn type3_objects(font_resources: &str, proc_resources: &str) -> Vec<String> {
    vec![
        format!(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] \
             /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 11 0 R >> \
             /Encoding << /Type /Encoding /Differences [97 /a] >> \
             /FirstChar 97 /LastChar 97 /Widths [1000] {font_resources} >>"
        ),
        stream(
            proc_resources,
            "1000 0 d0 0 0 m 10 0 l 10 10 l f \
             /OC /P1 BDC 0 0 m 20 0 l 20 20 l f EMC",
        ),
    ]
}

fn paths_with_nested(bytes: Vec<u8>) -> usize {
    on_small_stack(move || {
        let pdf = Pdf::new(bytes).expect("fixture opens");
        run_page(
            &pdf,
            &Run {
                run_nested: true,
                ..Run::default()
            },
        )
        .paths
    })
}

const T3_FORM: (&str, &str) = ("/Font << /T3 8 0 R >>", "BT /T3 11 Tf 10 100 Td (a) Tj ET");

#[test]
fn precedence_a_type3_font_uses_the_resources_it_names() {
    let bytes = precedence(T3_FORM.0, T3_FORM.1, &type3_objects("/Resources 5 0 R", ""));
    // The unmarked path only: the marked one is under OCG 9, which is OFF.
    assert_eq!(paths_with_nested(bytes), 1);
}

#[test]
fn precedence_a_type3_glyph_procedure_uses_the_resources_it_names() {
    let bytes = precedence(T3_FORM.0, T3_FORM.1, &type3_objects("", "/Resources 5 0 R"));
    assert_eq!(paths_with_nested(bytes), 1);
}

#[test]
fn precedence_a_tiling_pattern_uses_the_resources_it_names() {
    let bytes = precedence(
        "/Pattern << /Pat 8 0 R >>",
        "/Pattern cs /Pat scn 0 0 200 200 re f",
        &[stream(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 \
             /BBox [0 0 20 20] /XStep 20 /YStep 20 /Resources 5 0 R",
            "0 0 m 10 0 l 10 10 l f /OC /P1 BDC 0 0 m 20 0 l 20 20 l f EMC",
        )],
    );
    // The page-filling rectangle, then the pattern cell's unmarked path.
    assert_eq!(paths_with_nested(bytes), 2);
}

// The same sites with a *direct* `/Resources`: the dictionary itself is used
// (no fresh lookup is needed), and its `P1` (OFF) wins over the inherited one.

/// Direct resources naming `P1` = OCG 9, which is OFF.
const DIRECT_OFF: &str = "/Resources << /Properties << /P1 9 0 R >> >>";

#[test]
fn a_type3_font_with_direct_resources_uses_them() {
    let bytes = precedence(T3_FORM.0, T3_FORM.1, &type3_objects(DIRECT_OFF, ""));
    assert_eq!(paths_with_nested(bytes), 1);
}

#[test]
fn a_type3_glyph_procedure_with_direct_resources_uses_them() {
    let bytes = precedence(T3_FORM.0, T3_FORM.1, &type3_objects("", DIRECT_OFF));
    assert_eq!(paths_with_nested(bytes), 1);
}

#[test]
fn a_glyph_procedure_with_non_dictionary_resources_inherits() {
    // `/Resources 42` is no dictionary: the procedure gets an empty level
    // and inherits the form's `P1` (ON), as before H12.
    let bytes = precedence(T3_FORM.0, T3_FORM.1, &type3_objects("", "/Resources 42"));
    assert_eq!(paths_with_nested(bytes), 2);
}

#[test]
fn a_tiling_pattern_with_direct_resources_uses_them() {
    let bytes = precedence(
        "/Pattern << /Pat 8 0 R >>",
        "/Pattern cs /Pat scn 0 0 200 200 re f",
        &[stream(
            &format!(
                "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 \
                 /BBox [0 0 20 20] /XStep 20 /YStep 20 {DIRECT_OFF}"
            ),
            "0 0 m 10 0 l 10 10 l f /OC /P1 BDC 0 0 m 20 0 l 20 20 l f EMC",
        )],
    );
    // The page-filling rectangle, then the pattern cell's unmarked path.
    assert_eq!(paths_with_nested(bytes), 2);
}

#[test]
fn a_name_found_at_no_level_is_shown() {
    // The form has no `/Resources`: its `P1` comes from the page and is
    // hidden; `/Nowhere` is in no `/Properties` at any level, so it marks
    // nothing and its content is shown.
    let shape = Shape {
        form_resources: "",
        form_content: "BT /F 11 Tf 10 100 Td (FORMVIS) Tj ET \
             /OC /P1 BDC BT /F 11 Tf 10 80 Td (HIDDENFORM) Tj ET EMC \
             /OC /Nowhere BDC BT /F 11 Tf 10 60 Td (NOWHERE) Tj ET EMC",
        ..SHARED
    };
    assert_eq!(text_of(q8(&shape)), "PAGEVISFORMVISNOWHERE");
}

#[test]
fn precedence_controls_with_the_on_group_draw_both_paths() {
    // The same shapes with `/Resources` left out: the parent chain, where the
    // form's `P1` (ON) is innermost, is then the right answer.
    let bytes = precedence(T3_FORM.0, T3_FORM.1, &type3_objects("", ""));
    assert_eq!(paths_with_nested(bytes), 2);
    let bytes = precedence(
        "/Pattern << /Pat 8 0 R >>",
        "/Pattern cs /Pat scn 0 0 200 200 re f",
        &[stream(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 \
             /BBox [0 0 20 20] /XStep 20 /YStep 20",
            "0 0 m 10 0 l 10 10 l f /OC /P1 BDC 0 0 m 20 0 l 20 20 l f EMC",
        )],
    );
    assert_eq!(paths_with_nested(bytes), 3);
}

// ---------------------------------------------------------------------------
// `Resources::get_property` and `get_property_ref`.

fn name(n: &str) -> Name<'_> {
    Name::new(n.as_bytes()).expect("valid name")
}

fn ocg_name(d: &Dict<'_>) -> String {
    let n = d.get::<object::String<'_>>(NAME).expect("OCG /Name");
    String::from_utf8_lossy(n.as_bytes()).into_owned()
}

#[test]
fn get_property_walks_to_the_page_s_entry() {
    let pdf = Pdf::new(q8(&SHARED)).expect("opens");
    let page = &pdf.pages()[0];
    let page_res = page.resources();
    let form = page_res.get_x_object(&name("Fx")).expect("form");
    // The form's own resources are object 5 again; with no `/Resources`
    // at all the entry can only come from the parent.
    for own in [Resources::resolve_dict(form.dict()), Dict::default()] {
        let res = Resources::from_parent(own, page_res.clone());
        let p1 = res.get_property(&name("P1")).expect("P1 found");
        assert_eq!(ocg_name(&p1), "P1");
        assert_eq!(
            res.get_property_ref(&name("P1")).map(|r| r.obj_number),
            Some(9)
        );
        assert!(res.get_property(&name("Nowhere")).is_none());
        assert!(res.get_property_ref(&name("Nowhere")).is_none());
    }
}

#[test]
fn get_property_prefers_the_innermost_level() {
    let pdf = Pdf::new(precedence("", "", &[])).expect("opens");
    let page = &pdf.pages()[0];
    let page_res = page.resources();
    let form = page_res.get_x_object(&name("Fa")).expect("form");
    let res = Resources::from_parent(Resources::resolve_dict(form.dict()), page_res.clone());
    assert_eq!(ocg_name(&res.get_property(&name("P1")).expect("P1")), "On");
    assert_eq!(
        res.get_property_ref(&name("P1")).map(|r| r.obj_number),
        Some(10)
    );
    // The page alone still sees its own entry.
    assert_eq!(
        ocg_name(&page_res.get_property(&name("P1")).expect("P1")),
        "Off"
    );
    assert!(page_res.get_property(&name("Nowhere")).is_none());
}

// ---------------------------------------------------------------------------
// Genuine cycles. The fresh lookup is used for the one `/Resources` entry
// only; hayro-syntax's guard still refuses an object that contains itself,
// and the interpreter's form-cycle check and nesting cap bound what is drawn.
// Every case runs on a 512 KiB stack.

#[test]
fn a_resources_dictionary_containing_itself_resolves_to_nothing() {
    // Object 5's `/Properties` is object 5: refused by the guard, so the
    // name resolves nowhere and the marked text is not hidden.
    let bytes = assemble_pdf(&[
        CATALOG_P1_OFF.to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 5 0 R \
         /Contents 4 0 R >>"
            .to_owned(),
        stream("", "/OC /P1 BDC BT /F 11 Tf 10 150 Td (Shown) Tj ET EMC"),
        "<< /Font << /F 6 0 R >> /Properties 5 0 R >>".to_owned(),
        HELV.to_owned(),
    ]);
    let out = run_on_small_stack(bytes, Run::default());
    assert_eq!(out.text, "Shown");
}

#[test]
fn a_form_naming_itself_as_resources_is_drawn_once() {
    // Form 6's `/Resources` is form 6, a stream, so its own resources are
    // empty (as before H12); `/Fx` is found in the page's resources, and the
    // form-cycle check stops the second draw.
    let bytes = assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 5 0 R \
         /Contents 4 0 R >>"
            .to_owned(),
        stream("", "/Fx Do"),
        "<< /Font << /F 7 0 R >> /XObject << /Fx 6 0 R >> >>".to_owned(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200] /Resources 6 0 R \
             /Font << /F 7 0 R >> /XObject << /Fx 6 0 R >>",
            "BT /F 11 Tf 10 100 Td (Self) Tj ET /Fx Do",
        ),
        HELV.to_owned(),
    ]);
    let out = run_on_small_stack(bytes, Run::default());
    assert_eq!(out.text, "Self");
    assert_eq!(out.stats.form_cycle_hits, 1);
}

#[test]
fn two_forms_naming_each_other_as_resources_get_empty_resources() {
    // Form 6's `/Resources` is form 7 and vice versa. A stream is not a
    // dictionary, so each form's own resources are empty (as before H12);
    // `/N` is then found nowhere and the page's `/A` is not reached either.
    let form = |text: &str, own: u32, other: u32| {
        stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [-1000 -1000 1000 1000] \
                 /Resources {other} 0 R /XObject << /N {own} 0 R >>"
            ),
            &format!("BT /F 11 Tf 0 0 Td ({text}) Tj ET q 1 0 0 1 0 -12 cm /N Do Q"),
        )
    };
    let bytes = assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 5 0 R \
         /Contents 4 0 R >>"
            .to_owned(),
        stream("", "q 1 0 0 1 20 150 cm /A Do Q"),
        "<< /Font << /F 8 0 R >> /XObject << /A 6 0 R >> >>".to_owned(),
        form("A", 6, 7),
        form("B", 7, 6),
        HELV.to_owned(),
    ]);
    let out = run_on_small_stack(bytes, Run::default());
    assert_eq!(out.lines(), ["A"]);
    assert_eq!(out.stats.form_cycle_hits, 0);
}

#[test]
fn two_resources_objects_naming_each_other_terminate() {
    // Form 6's resources (object 9) name form 7 and `/Properties` object
    // 10; form 7's resources (object 10) name form 6 and `/Properties` 9.
    // Every lookup is fresh, so nothing is refused, and the form loop is
    // stopped by the form-cycle check.
    let form = |text: &str, res: u32| {
        stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [-1000 -1000 1000 1000] /Resources {res} 0 R"
            ),
            &format!("BT /F 11 Tf 0 0 Td ({text}) Tj ET q 1 0 0 1 0 -12 cm /N Do Q"),
        )
    };
    let bytes = assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 5 0 R \
         /Contents 4 0 R >>"
            .to_owned(),
        stream("", "q 1 0 0 1 20 150 cm /A Do Q"),
        "<< /Font << /F 8 0 R >> /XObject << /A 6 0 R >> >>".to_owned(),
        form("A", 9),
        form("B", 10),
        HELV.to_owned(),
        "<< /XObject << /N 7 0 R >> /Properties 10 0 R >>".to_owned(),
        "<< /XObject << /N 6 0 R >> /Properties 9 0 R >>".to_owned(),
    ]);
    let out = run_on_small_stack(bytes, Run::default());
    assert_eq!(out.lines(), ["A", "B"]);
    assert_eq!(out.stats.form_cycle_hits, 1);
}

#[test]
fn a_type3_font_naming_itself_as_resources_stops_at_the_nesting_cap() {
    // The glyph procedure shows `a` in the same font, found through the
    // font's own `/Resources`, which is the font: renderer-style nesting is
    // bounded by the 50-level cap, not by the stack.
    let bytes = assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 5 0 R \
         /Contents 4 0 R >>"
            .to_owned(),
        stream("", "BT /T3 11 Tf 10 100 Td (a) Tj ET"),
        "<< /Font << /T3 6 0 R >> >>".to_owned(),
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] \
         /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 7 0 R >> \
         /Encoding << /Type /Encoding /Differences [97 /a] >> \
         /FirstChar 97 /LastChar 97 /Widths [1000] \
         /Resources 6 0 R /Font << /T3 6 0 R >> >>"
            .to_owned(),
        stream("", "1000 0 d0 BT /T3 1000 Tf (a) Tj ET"),
    ]);
    let out = run_on_small_stack(
        bytes,
        Run {
            run_nested: true,
            ..Run::default()
        },
    );
    assert!(out.stats.nesting_cap_hits >= 1, "{:?}", out.stats);
}
