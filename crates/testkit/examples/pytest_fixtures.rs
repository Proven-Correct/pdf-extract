//! Writes the generated PDFs the Python test suite (`tests/python`) runs on,
//! so that the Python contract, hostile and no-output tests use exactly the
//! generators the Rust tests use (one source of truth for fixture bytes).
//!
//! Usage: `cargo run -p pdf-extract-testkit --example pytest_fixtures -- DIR`
//!
//! Output: `DIR/<name>.pdf` for every named fixture below, `DIR/samples/`
//! with every builder and hostile sample of `tests/common/samples.rs` (the
//! catalogue the oracle `--verify` run also uses), and `DIR/index.tsv` with
//! one `relative path<TAB>group` line per file. `tests/python/conftest.py`
//! runs this once per test session; the Python test kit
//! (`pdf_extract.testing`) covers only the builder, not these generators.
//!
//! `DIR/ocr_noise.pdf` (group `ocr`) is built from the committed corpus-D
//! file `sparse_below_min_chars.pdf` plus the noise field.
//!
//! `DIR/golden/` holds the golden side of the Python test kit's byte-for-byte
//! check (spec §4.12): every document and width of the shared call script
//! `tests/data/builder_golden.tsv`, built here with the Rust kit, as
//! `golden/<doc>.pdf` and `golden/widths.tsv` (`fontsize<TAB>f32 bits as
//! 8 hex digits<TAB>text`). `tests/python/test_testing.py` replays the same
//! script through `pdf_extract.testing` and compares.

#[path = "../tests/common/samples.rs"]
mod samples;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pdf_extract_testkit::corpus_d::{self, ScanPage};
use pdf_extract_testkit::hostile::{self, TreeShape};
use pdf_extract_testkit::{PdfBuilder, TextStyle, contract, text_width};
use pdf_extract_testkit::{jbig2, jpx};

/// The call script shared with `tests/python/test_testing.py`.
const GOLDEN_SCRIPT: &str = include_str!("../tests/data/builder_golden.tsv");

/// A script number: parsed as `f64` and narrowed, exactly as the Python
/// binding narrows a float (PyO3's `f32` conversion is `f64 as f32`).
#[allow(clippy::cast_possible_truncation)]
fn script_num(field: &str, line: usize) -> Result<f32, String> {
    field
        .parse::<f64>()
        .map(|v| v as f32)
        .map_err(|e| format!("builder_golden.tsv:{line}: bad number {field:?}: {e}"))
}

/// The golden script's output: `(name, bytes)` per document, and the
/// `widths.tsv` text.
struct Golden {
    docs: Vec<(String, Vec<u8>)>,
    widths: String,
}

/// Runs the golden script.
fn golden() -> Result<Golden, String> {
    let mut docs: Vec<(String, PdfBuilder)> = Vec::new();
    let mut widths = String::new();
    for (i, raw) in GOLDEN_SCRIPT.lines().enumerate() {
        let line = i + 1;
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = raw.splitn(6, '\t').collect();
        let doc = || format!("builder_golden.tsv:{line}: no `doc` line yet");
        match fields.as_slice() {
            ["doc", name] => docs.push(((*name).to_owned(), PdfBuilder::new())),
            ["page"] => {
                docs.last_mut().ok_or_else(doc)?.1.add_page_a4();
            }
            ["page", w, h] => {
                let (w, h) = (script_num(w, line)?, script_num(h, line)?);
                docs.last_mut().ok_or_else(doc)?.1.add_page(w, h);
            }
            ["text", x, y, size, mode, rest @ ..] => {
                let style = TextStyle {
                    fontsize: script_num(size, line)?,
                    render_mode: mode
                        .parse()
                        .map_err(|e| format!("builder_golden.tsv:{line}: bad mode: {e}"))?,
                    ..TextStyle::default()
                };
                let point = (script_num(x, line)?, script_num(y, line)?);
                // An empty text may lose its trailing tab to an editor.
                let text = rest.first().copied().unwrap_or("");
                let b = &mut docs.last_mut().ok_or_else(doc)?.1;
                let index = b.page_count().checked_sub(1).ok_or_else(|| {
                    format!("builder_golden.tsv:{line}: `text` before any `page`")
                })?;
                b.page_mut(index)
                    .expect("index < page_count")
                    .insert_text(point, text, style)
                    .map_err(|e| format!("builder_golden.tsv:{line}: {e}"))?;
            }
            ["width", size, rest @ ..] => {
                let text = rest.first().copied().unwrap_or("");
                let w = text_width(text, script_num(size, line)?)
                    .map_err(|e| format!("builder_golden.tsv:{line}: {e}"))?;
                let _ = writeln!(widths, "{size}\t{:08x}\t{text}", w.to_bits());
            }
            _ => return Err(format!("builder_golden.tsv:{line}: cannot parse {raw:?}")),
        }
    }
    let docs = docs.into_iter().map(|(n, b)| (n, b.to_bytes())).collect();
    Ok(Golden { docs, widths })
}

/// Fixtures the Python tests address by name, grouped for `index.tsv`.
fn named() -> Vec<(String, &'static str, Vec<u8>)> {
    let mut out: Vec<(String, &'static str, Vec<u8>)> = Vec::new();
    let mut add = |name: &str, group: &'static str, bytes: Vec<u8>| {
        out.push((name.to_owned(), group, bytes));
    };

    // Open contract (spec §4.8).
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .raw_content("BT /Helv 11 Tf 72 720 Td (One) Tj ET");
    b.add_page_a4()
        .raw_content("BT /Helv 11 Tf 72 720 Td (Two) Tj ET");
    add("valid", "contract", b.to_bytes());
    add(
        "garbage_after_header",
        "contract",
        contract::garbage_after_header(),
    );
    add("plain_text", "contract", contract::plain_text());
    add(
        "catalog_without_pages",
        "contract",
        contract::catalog_without_pages(),
    );
    add("broken_startxref", "contract", contract::broken_startxref());
    for cut in [8, 20, 60, 120, 200] {
        add(
            &format!("truncated_tail_{cut}"),
            "contract",
            contract::truncated_tail(cut),
        );
    }
    for offset in [500, 1019, 1020, 1500] {
        add(
            &format!("header_at_{offset}"),
            "contract",
            contract::header_at_offset(offset),
        );
    }
    add(
        "misoffset_40_6_10",
        "contract",
        contract::misoffset_object(40, 6, 10),
    );
    add("empty_kids", "contract", PdfBuilder::new().to_bytes());

    // Page-tree shapes.
    let shapes = [
        ("tree_flat_2", TreeShape::Flat(2)),
        ("tree_flat_3", TreeShape::Flat(3)),
        ("tree_flat_10", TreeShape::Flat(10)),
        ("tree_lying_count_5", TreeShape::LyingCount { claimed: 5 }),
        ("tree_duplicate_leaf", TreeShape::DuplicateLeaf),
        ("tree_cycle", TreeShape::Cycle),
        ("tree_typeless_leaf", TreeShape::TypelessLeaf),
        ("tree_direct_dict_kid", TreeShape::DirectDictKid),
        (
            "tree_fanout_100x2",
            TreeShape::Fanout {
                leaves_per_node: 100,
                levels: 2,
            },
        ),
    ];
    for (name, shape) in shapes {
        add(name, "tree", hostile::page_tree(shape));
    }
    for depth in [0, 10, 11, 200, 3000] {
        add(
            &format!("deep_page_tree_{depth}"),
            "tree",
            hostile::deep_page_tree(depth),
        );
    }

    // Hostile content (spec §4.10's core-level rows, repeated in Python).
    add(
        "deep_nesting_in_page_dict_200000",
        "hostile",
        hostile::deep_nesting_in_page_dict(200_000),
    );
    add(
        "deep_nesting_in_content_200000",
        "hostile",
        hostile::deep_nesting_in_content(200_000),
    );
    add(
        "deep_nesting_in_resources_200000",
        "hostile",
        hostile::deep_nesting_in_resources(200_000),
    );
    add(
        "deep_nesting_in_type4_function_200000",
        "hostile",
        hostile::deep_nesting_in_type4_function(200_000),
    );
    add("flate_bomb_50mib", "hostile", hostile::flate_bomb(50 << 20));
    add(
        "operator_flood_400000",
        "hostile",
        hostile::operator_flood(400_000),
    );
    add(
        "form_self_recursion",
        "hostile",
        hostile::form_self_recursion(),
    );
    add("nested_forms_49", "hostile", hostile::nested_forms(49));
    add("nested_forms_60", "hostile", hostile::nested_forms(60));
    // Run 0003 H14: a JBIG2 image whose decoder would copy 512 GiB unless its
    // memory budget refuses it.
    add(
        "jbig2_symbol_export",
        "hostile",
        jbig2::symbol_export_bomb_pdf(),
    );
    // Run 0003 H15: a JPEG 2000 image with 3.6 billion tile records unless
    // its memory budget refuses it.
    add("jpx_tile_bomb", "hostile", jpx::tile_bomb_pdf());
    // Opening this takes long enough (~100 ms+) for the GIL test to see a
    // held GIL; written straight to disk by the generator.
    add("big_tree_300000", "big", Vec::new());
    // ~25M operators: only ever opened with a short `page_time_budget`.
    add("form_fanout_22", "slow", hostile::form_fanout(22));

    // Text shape (design spec §13.5).
    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    for (y, text) in [
        (100.0, "Line one."),
        (113.0, "line two."),
        (300.0, "Block two."),
    ] {
        p.insert_text((72.0, y), text, TextStyle::default())
            .expect("ASCII");
    }
    add("two_blocks", "text", b.to_bytes());
    let mut b = PdfBuilder::new();
    b.add_page_a4();
    add("empty_page", "text", b.to_bytes());
    let mut b = PdfBuilder::new();
    for i in 0..40 {
        b.add_page_a4()
            .insert_text(
                (72.0, 72.0),
                &format!("Page {}", i + 1),
                TextStyle::default(),
            )
            .expect("ASCII");
    }
    add("forty_pages", "text", b.to_bytes());

    // The OCR noise pre-check (run 0003 §4.2): page 0 a small white scan,
    // page 1 the pure noise field (skipped at 300 dpi), page 2 corpus D's
    // sparse page under noise (recognised at 150 dpi).
    add("ocr_noise", "ocr", ocr_noise());

    out
}

/// The three-page `ocr_noise` fixture.
fn ocr_noise() -> Vec<u8> {
    let white = ScanPage {
        media_box: [0.0, 0.0, 72.0, 72.0],
        crop_box: None,
        rotate: 0,
        width: 1,
        height: 1,
        pixels: vec![255],
    };
    let clean = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/corpus-d/sparse_below_min_chars.pdf");
    let clean = std::fs::read(&clean).unwrap_or_else(|e| panic!("{}: {e}", clean.display()));
    let sparse = corpus_d::noise_variant_page(&clean).expect("committed scan");
    corpus_d::scan_pdf(&[white, corpus_d::pure_noise_page(), sparse])
}

fn write(
    dir: &Path,
    rel: &str,
    bytes: &[u8],
    index: &mut String,
    group: &str,
) -> Result<(), String> {
    std::fs::write(dir.join(rel), bytes).map_err(|e| format!("cannot write {rel}: {e}"))?;
    let _ = writeln!(index, "{rel}\t{group}");
    Ok(())
}

fn run(dir: &Path) -> Result<(), String> {
    let sample_dir = dir.join("samples");
    std::fs::create_dir_all(&sample_dir)
        .map_err(|e| format!("cannot create {}: {e}", sample_dir.display()))?;
    let mut index = String::new();
    for (name, group, bytes) in named() {
        let rel = format!("{name}.pdf");
        if name == "big_tree_300000" {
            hostile::big_tree_to_path(300_000, &dir.join(&rel))
                .map_err(|e| format!("cannot write {rel}: {e}"))?;
            let _ = writeln!(index, "{rel}\t{group}");
        } else {
            write(dir, &rel, &bytes, &mut index, group)?;
        }
    }
    let golden_dir = dir.join("golden");
    std::fs::create_dir_all(&golden_dir)
        .map_err(|e| format!("cannot create {}: {e}", golden_dir.display()))?;
    let Golden { docs, widths } = golden()?;
    for (name, bytes) in docs {
        write(
            dir,
            &format!("golden/{name}.pdf"),
            &bytes,
            &mut index,
            "golden",
        )?;
    }
    std::fs::write(golden_dir.join("widths.tsv"), widths)
        .map_err(|e| format!("cannot write golden/widths.tsv: {e}"))?;
    for s in samples::builder_samples()
        .into_iter()
        .chain(samples::hostile_samples())
    {
        let group = format!("sample-{}", s.kind);
        write(
            dir,
            &format!("samples/{}.pdf", s.name),
            &s.bytes,
            &mut index,
            &group,
        )?;
    }
    // Written last: its presence marks a complete directory.
    std::fs::write(dir.join("index.tsv"), index).map_err(|e| format!("cannot write index.tsv: {e}"))
}

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: pytest_fixtures DIR");
        return ExitCode::from(2);
    };
    match run(&dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
