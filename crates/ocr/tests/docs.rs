//! Source checks that the fork-hazard documentation (run 0003 F1, F4) stays
//! attached to every lock it names. Syntactic by design, like
//! `pdf-extract-tesseract-sys`'s `shim_symbols.rs`: it reads the source
//! files, because rustdoc output does not cover private items and a lock's
//! doc comment is exactly where a later reader looks.

use std::path::PathBuf;

fn workspace_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The contiguous `///` block directly above the first line containing
/// `anchor` (attributes between the block and the item are skipped).
fn doc_above(src: &str, anchor: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.contains(anchor))
        .unwrap_or_else(|| panic!("`{anchor}` not found"));
    let mut doc = Vec::new();
    for line in lines[..at].iter().rev() {
        let t = line.trim_start();
        if t.starts_with("///") {
            doc.push(t);
        } else if t.starts_with("#[") && doc.is_empty() {
            continue;
        } else {
            break;
        }
    }
    doc.join("\n")
}

/// Every `//!` line of the file.
fn inner_doc(src: &str) -> String {
    src.lines()
        .filter(|l| l.trim_start().starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_forking_note(what: &str, doc: &str) {
    assert!(
        doc.to_lowercase().contains("fork"),
        "{what}: doc comment does not mention fork:\n{doc}"
    );
    assert!(
        doc.contains("crate#forking") || doc.contains("`Forking` section"),
        "{what}: doc comment does not link the crate-level Forking section:\n{doc}"
    );
}

#[test]
fn the_crate_docs_have_a_forking_section_with_the_three_rules() {
    let doc = inner_doc(&workspace_file("ocr/src/lib.rs"));
    assert!(doc.contains("# Forking"), "crate docs lack `# Forking`");
    for needle in [
        "Build every engine before forking",
        "Do not fork while another thread",
        "posix_spawn",
        "GLOBAL_INIT",
        "OVERRIDE",
        "LOG",
        "TEST_ENV",
    ] {
        assert!(doc.contains(needle), "Forking section lacks `{needle}`");
    }
}

#[test]
fn every_lock_of_the_hazard_statement_links_the_forking_section() {
    let engine = workspace_file("ocr/src/engine.rs");
    assert_forking_note(
        "default_ocr_engine",
        &doc_above(&engine, "pub fn default_ocr_engine"),
    );
    assert_forking_note("OcrEngine", &doc_above(&engine, "pub struct OcrEngine"));
    assert_forking_note(
        "pool module",
        &inner_doc(&workspace_file("ocr/src/pool.rs")),
    );
    let client = workspace_file("ocr/src/isolation/client.rs");
    assert_forking_note("LOG", &doc_above(&client, "static LOG:"));
    assert_forking_note("TEST_ENV", &doc_above(&client, "static TEST_ENV:"));
    assert_forking_note(
        "tessdata OVERRIDE",
        &doc_above(&workspace_file("ocr/src/tessdata.rs"), "static OVERRIDE:"),
    );
    assert_forking_note(
        "GLOBAL_INIT",
        &doc_above(
            &workspace_file("tesseract-sys/src/lib.rs"),
            "static GLOBAL_INIT:",
        ),
    );
}

#[test]
fn default_ocr_engine_documents_what_a_panic_leaves_behind() {
    let doc = doc_above(
        &workspace_file("ocr/src/engine.rs"),
        "pub fn default_ocr_engine",
    );
    assert!(
        doc.contains("panic") && doc.contains("empty"),
        "F4: the doc must say a panic propagates and leaves the cell empty:\n{doc}"
    );
}
