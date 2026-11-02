//! H2a: the object-nesting cap in the vendored hayro fork (spec §3.2, §4.4).
//!
//! hayro parses arrays, dictionaries and Type 4 (PostScript calculator)
//! procedures recursively. Before H2a, 200,000 levels of nesting overflowed
//! even an 8 MiB stack and aborted the process (spec Appendix A.7). With H2a
//! anything nested deeper than `MAX_OBJECT_DEPTH` (256) fails to parse like
//! any other malformed object, and each refusal bumps a thread-local counter
//! that the library samples around an operation to raise a warning.
//!
//! Every hostile fixture here runs on a 512 KiB thread stack. What the
//! library makes of these outcomes (`ObjectDepth` warnings, page counts,
//! `Page::text`) is tested in units 08-10.

mod common;

use std::process::Command;

use common::{SMALL_STACK, interpret, on_small_stack};
use hayro_interpret::Function;
use hayro_postscript::Scanner;
use hayro_syntax::object::{Array, Dict, FromBytes, Object};
use hayro_syntax::{MAX_OBJECT_DEPTH, Pdf, object_depth_limit_hits};
use pdf_extract_testkit::hostile;
use smallvec::SmallVec;

/// The nesting the spec's hostile fixtures use: far beyond any stack.
const HOSTILE_DEPTH: usize = 200_000;

/// Runs `f` and returns its result with the number of nesting-cap refusals
/// recorded on this thread meanwhile.
fn counting_hits<T>(f: impl FnOnce() -> T) -> (T, u64) {
    let before = object_depth_limit_hits();
    let result = f();
    (result, object_depth_limit_hits() - before)
}

#[test]
fn cap_is_256() {
    assert_eq!(MAX_OBJECT_DEPTH, 256);
}

#[test]
fn over_deep_page_dict_drops_the_page() {
    let (pages, hits) = on_small_stack(|| {
        counting_hits(|| {
            let pdf = Pdf::new(hostile::deep_nesting_in_page_dict(HOSTILE_DEPTH))
                .expect("the file itself is well formed");
            // hayro's eager page walk cannot read the leaf, so skips it.
            pdf.pages().len()
        })
    });
    assert_eq!(pages, 0);
    assert!(hits >= 1, "no nesting-cap hit recorded");
}

#[test]
fn over_deep_content_operand_ends_the_content_stream() {
    let (text, hits) = on_small_stack(|| {
        counting_hits(|| {
            let pdf = Pdf::new(hostile::deep_nesting_in_content(HOSTILE_DEPTH)).expect("opens");
            let page = &pdf.pages()[0];
            interpret(&pdf, page).text
        })
    });
    // `(Before) Tj <deep> TJ (After) Tj`: the deep operand fails to parse,
    // the content iterator stops there, and `After` is lost (as in the
    // oracle). Pinned exactly so a change in hayro's resynchronisation is
    // noticed.
    assert_eq!(text, "Before");
    assert!(hits >= 1, "no nesting-cap hit recorded");
}

#[test]
fn over_deep_resources_fall_back_to_no_resources() {
    let ((resources_empty, text), hits) = on_small_stack(|| {
        counting_hits(|| {
            let pdf = Pdf::new(hostile::deep_nesting_in_resources(HOSTILE_DEPTH)).expect("opens");
            let page = &pdf.pages()[0];
            let res = page.resources();
            let all_empty = [
                &res.fonts,
                &res.ext_g_states,
                &res.color_spaces,
                &res.x_objects,
                &res.patterns,
                &res.shadings,
                &res.properties,
            ]
            .iter()
            .all(|d| d.is_empty());
            (all_empty, interpret(&pdf, page).text)
        })
    });
    assert!(
        resources_empty,
        "the unreadable resources dictionary must read as empty"
    );
    // `/Helv` cannot be resolved; hayro falls back to Helvetica.
    assert_eq!(text, "Res");
    assert!(hits >= 1, "no nesting-cap hit recorded");
}

#[test]
fn over_deep_type4_function_is_rejected() {
    let ((function_parsed, collected), hits) = on_small_stack(|| {
        counting_hits(|| {
            let pdf =
                Pdf::new(hostile::deep_nesting_in_type4_function(HOSTILE_DEPTH)).expect("opens");
            let page = &pdf.pages()[0];
            let cs = page
                .resources()
                .color_spaces
                .get::<Array<'_>>("CS0")
                .expect("CS0 is readable: only the function program is deep");
            let tint = cs
                .iter::<Object<'_>>()
                .nth(3)
                .expect("Separation has a tint transform");
            let parsed = Function::new(&tint).is_some();
            (parsed, interpret(&pdf, page))
        })
    });
    assert!(!function_parsed, "Function::new must refuse the program");
    assert_eq!(collected.text, "Fn");
    // Without its tint transform the Separation colour space cannot be
    // built; hayro then falls back to DeviceGray (`ColorSpace::device_gray()`
    // in its `cs` handler) and still fills the rectangle. Pinned: the fill is
    // not skipped.
    assert_eq!(collected.paths, 1);
    assert!(hits >= 1, "type4.rs must report the refusal");
}

/// `<< /K << /K ... 1 ... >> >>`: `depth` dictionaries, one inside the other.
fn nested_dicts(depth: usize) -> String {
    let mut s = "<< /K ".repeat(depth);
    s.push('1');
    s.push_str(&" >>".repeat(depth));
    s
}

/// `[[...[1]...]]`: `depth` arrays, one inside the other.
fn nested_arrays(depth: usize) -> String {
    format!("{}1{}", "[".repeat(depth), "]".repeat(depth))
}

/// `[<< /K [<< ... >>] >>]`: arrays and dictionaries alternating, `depth`
/// containers in all (an odd depth ends on an array).
fn nested_mixed(depth: usize) -> String {
    let (mut open, mut close) = (String::new(), String::new());
    for level in 0..depth {
        if level % 2 == 0 {
            open.push('[');
            close.insert(0, ']');
        } else {
            open.push_str("<< /K ");
            close.insert_str(0, " >>");
        }
    }
    format!("{open}1{close}")
}

#[test]
fn nesting_boundary_is_256_levels() {
    on_small_stack(|| {
        for (shape, make) in [
            ("dicts", nested_dicts as fn(usize) -> String),
            ("arrays", nested_arrays),
            ("mixed", nested_mixed),
        ] {
            for depth in [1, 255, 256] {
                let (parsed, hits) =
                    counting_hits(|| Object::from_bytes(make(depth).as_bytes()).is_some());
                assert!(parsed, "{shape} nested {depth} deep must parse");
                assert_eq!(hits, 0, "{shape} at {depth} must not count a hit");
            }
            let (parsed, hits) =
                counting_hits(|| Object::from_bytes(make(257).as_bytes()).is_some());
            assert!(!parsed, "{shape} nested 257 deep must fail");
            assert!(hits >= 1, "{shape} at 257 must count a hit");
        }
        // The typed entry points go through the same check.
        assert!(Dict::from_bytes(nested_dicts(256).as_bytes()).is_some());
        assert!(Dict::from_bytes(nested_dicts(257).as_bytes()).is_none());
        assert!(Array::from_bytes(nested_arrays(256).as_bytes()).is_some());
        assert!(Array::from_bytes(nested_arrays(257).as_bytes()).is_none());
    });
}

/// A `<<` where a key should be is read as a whole object (hayro's leniency
/// for garbage between entries), the heaviest recursion in the parser. With
/// the dev profile's `opt-level` for hayro-syntax (workspace Cargo.toml), the
/// 256 accepted levels fit the small stack in debug builds as in release.
#[test]
fn dictionaries_in_key_position_are_capped() {
    on_small_stack(|| {
        for depth in [256, 257, HOSTILE_DEPTH] {
            let src = format!("{}1{}", "<<".repeat(depth), ">>".repeat(depth));
            let (parsed, hits) = counting_hits(|| Object::from_bytes(src.as_bytes()).is_some());
            // hayro skips the garbage, so up to the cap this parses (as
            // nested empty-keyed dictionaries); beyond it, the cap refuses.
            assert_eq!(parsed, depth <= 256, "depth {depth}");
            assert_eq!(hits >= 1, depth > 256, "depth {depth}: {hits} hits");
        }
    });
    on_small_stack(|| {
        // The same run where a file's only object should be, with no xref:
        // hayro's repair scan probes it as a dictionary.
        let mut file = b"%PDF-1.7\n1 0 obj\n".to_vec();
        file.extend_from_slice("<<".repeat(HOSTILE_DEPTH).as_bytes());
        let (_, hits) = counting_hits(|| Pdf::new(file).is_ok());
        assert!(hits >= 1);
    });
}

#[test]
fn a_256_deep_object_still_reads_on_a_small_stack() {
    // The cap only helps if the deepest accepted object fits: read, then walk
    // all the way down through the lazily parsed levels.
    on_small_stack(|| {
        let text = nested_mixed(256);
        let mut obj = Object::from_bytes(text.as_bytes()).expect("parses");
        let mut levels = 0;
        loop {
            obj = match obj {
                Object::Array(a) => a.iter::<Object<'_>>().next().expect("one item"),
                Object::Dict(d) => d.get::<Object<'_>>("K").expect("one entry"),
                Object::Number(_) => break,
                other => panic!("unexpected {other:?}"),
            };
            levels += 1;
        }
        assert_eq!(levels, 256);
    });
}

#[test]
fn the_hit_counter_is_per_thread() {
    // A fresh thread starts at zero, whatever other threads have done.
    let fresh = std::thread::spawn(object_depth_limit_hits)
        .join()
        .expect("thread");
    assert_eq!(fresh, 0);

    let before_here = object_depth_limit_hits();
    let seen_there = on_small_stack(|| {
        let before = object_depth_limit_hits();
        assert!(Object::from_bytes(nested_arrays(300).as_bytes()).is_none());
        object_depth_limit_hits() - before
    });
    assert!(seen_there >= 1, "the spawned thread records its own hit");
    assert_eq!(
        object_depth_limit_hits(),
        before_here,
        "hits on another thread must not show up here"
    );
}

/// A Type 4 function stream (a direct object; no xref needed). `/Domain []`
/// and no `/Range` is how hayro's own type4.rs tests evaluate a bare program:
/// no inputs, no clamping, the whole operand stack is the output.
fn type4_function(program: &str) -> Option<Function> {
    let src = format!(
        "<< /FunctionType 4 /Domain [] /Length {} >>\nstream\n{program}\nendstream",
        program.len()
    );
    let obj = Object::from_bytes(src.as_bytes()).expect("stream object parses");
    Function::new(&obj)
}

fn eval(f: &Function) -> Vec<f32> {
    f.eval(SmallVec::new()).expect("evaluates").to_vec()
}

/// `{ true { true { ... 7 ... } if } if }`: procedures nested `depth` levels
/// (the outer braces count as one) that leave a single 7.
fn nested_procedure(depth: usize) -> String {
    let depth = depth.max(1);
    format!(
        "{{{} 7{} }}",
        " true {".repeat(depth - 1),
        " } if".repeat(depth - 1)
    )
}

#[test]
fn type4_procedure_nesting_is_capped() {
    // Parsing is what a hostile file reaches, so it runs on the small stack.
    let accepted = on_small_stack(|| {
        let mut accepted = Vec::new();
        for depth in [1, 200, 256] {
            let (f, hits) = counting_hits(|| type4_function(&nested_procedure(depth)));
            assert!(f.is_some(), "{depth} nested procedures must parse");
            assert_eq!(hits, 0, "depth {depth}");
            accepted.push((depth, f));
        }
        for depth in [257, 300, HOSTILE_DEPTH] {
            let (f, hits) = counting_hits(|| type4_function(&nested_procedure(depth)));
            assert!(f.is_none(), "{depth} nested procedures must be refused");
            assert!(hits >= 1, "type4.rs must count the refusal at {depth}");
        }
        accepted
    });
    // Evaluation recurses through the nested `if`s too, and also has to fit
    // the small stack (the dev profile optimises hayro-interpret).
    on_small_stack(move || {
        for (depth, f) in accepted {
            assert_eq!(eval(&f.expect("checked above")), [7.0], "depth {depth}");
        }
    });
}

#[test]
fn type4_evaluation_is_unchanged() {
    // A sample of hayro-interpret's own type4.rs `op_impl` cases (program,
    // expected operand stack), including the procedure operators whose
    // parser H2a changes.
    let cases: &[(&str, &[f32])] = &[
        ("{ 3 2 div }", &[1.5]),
        ("{ 5 3 mul }", &[15.0]),
        ("{ -5 2 idiv }", &[-2.0]),
        ("{ 7 3 bitshift }", &[56.0]),
        ("{ 4.2 6 lt }", &[1.0]),
        ("{ 12 3 xor }", &[15.0]),
        ("{ true { 1.0 } if }", &[1.0]),
        ("{ false { 1.0 } if }", &[]),
        ("{ true { 1.0 } { 2.0 } ifelse }", &[1.0]),
        ("{ false { 1.0 } { 2.0 } ifelse }", &[2.0]),
        ("{ 1 2 3 2 copy }", &[1.0, 2.0, 3.0, 2.0, 3.0]),
        ("{ 1 2 3 4 3 index }", &[1.0, 2.0, 3.0, 4.0, 1.0]),
        ("{ 1 2 3 3 -1 roll }", &[2.0, 3.0, 1.0]),
    ];
    for &(program, want) in cases {
        let f = type4_function(program).unwrap_or_else(|| panic!("{program} parses"));
        assert_eq!(eval(&f), want, "{program}");
    }
}

#[test]
fn postscript_scanner_nesting_is_capped() {
    on_small_stack(|| {
        let dict_delims = |n: usize| format!("{}42", "<<".repeat(n));
        for n in [200, 256] {
            let src = dict_delims(n);
            assert!(
                matches!(
                    Scanner::new(src.as_bytes()).parse_object(),
                    Ok(hayro_postscript::Object::Number(_))
                ),
                "{n} nested << must parse"
            );
        }
        for n in [257, 300, HOSTILE_DEPTH] {
            let src = dict_delims(n);
            assert_eq!(
                Scanner::new(src.as_bytes()).parse_object(),
                Err(hayro_postscript::Error::LimitCheck),
                "{n} nested <<"
            );
        }
        // `>>` is skipped the same way.
        let closers = format!("{}42", ">>".repeat(300));
        assert_eq!(
            Scanner::new(closers.as_bytes()).parse_object(),
            Err(hayro_postscript::Error::LimitCheck)
        );

        // Arrays are skipped iteratively but capped at the same depth.
        let arrays = |n: usize| format!("{}1{}", "[".repeat(n), "]".repeat(n));
        assert!(Scanner::new(arrays(256).as_bytes()).parse_array().is_ok());
        assert_eq!(
            Scanner::new(arrays(257).as_bytes()).parse_array(),
            Err(hayro_postscript::Error::LimitCheck)
        );
    });
}

/// Environment variable that turns [`child_extract_fixture`] from a no-op
/// into the real workload; set only by [`deep_fixtures_do_not_abort`].
const CHILD_ENV: &str = "PDF_EXTRACT_H2A_CHILD_FIXTURE";

fn fixture(name: &str) -> Vec<u8> {
    match name {
        "page_dict" => hostile::deep_nesting_in_page_dict(HOSTILE_DEPTH),
        "content" => hostile::deep_nesting_in_content(HOSTILE_DEPTH),
        "resources" => hostile::deep_nesting_in_resources(HOSTILE_DEPTH),
        "type4_function" => hostile::deep_nesting_in_type4_function(HOSTILE_DEPTH),
        other => panic!("unknown fixture {other}"),
    }
}

/// The child half of [`deep_fixtures_do_not_abort`]: opens the fixture named
/// by [`CHILD_ENV`] and interprets every page on a 512 KiB stack. Without the
/// variable (a normal test run) it does nothing; the per-fixture tests above
/// cover the outcomes in-process.
#[test]
fn child_extract_fixture() {
    let Ok(name) = std::env::var(CHILD_ENV) else {
        return;
    };
    let bytes = fixture(&name);
    on_small_stack(move || {
        if let Ok(pdf) = Pdf::new(bytes) {
            for page in pdf.pages().iter() {
                let _ = interpret(&pdf, page);
            }
        }
    });
}

/// Regression for the pre-H2 stack overflow (spec §4.4, Appendix A.7):
/// each 200,000-deep fixture runs in a child process, so an overflow shows
/// up as a failed exit status here instead of killing the test binary.
#[test]
fn deep_fixtures_do_not_abort() {
    let exe = std::env::current_exe().expect("test binary path");
    for name in ["page_dict", "content", "resources", "type4_function"] {
        let out = Command::new(&exe)
            .args([
                "--exact",
                "child_extract_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, name)
            .output()
            .expect("run child test process");
        assert!(
            out.status.success(),
            "{name}: child exited with {:?} on a {SMALL_STACK}-byte stack\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        // Guard against a vacuous pass: the filter must have run the child.
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("1 passed"),
            "{name}: child did not run the fixture test\nstdout:\n{stdout}"
        );
    }
}
