//! Licensing of our own code is GPL-3.0-or-later and nothing else (spec 4.12b,
//! D-40): `LICENSE` is the canonical GPL-3.0 text, every workspace member and
//! the Python package declare `GPL-3.0-or-later`, vendored crates keep their
//! upstream license, and no tracked file still calls our own code MIT.
//!
//! THIRD_PARTY_NOTICES is written by `tools/license/gen-notices.sh` (the `deny`
//! CI job fails if it is stale); the notices tests here check, by exact text,
//! that it carries what the wheel must ship: each vendored hayro crate's
//! licence, every notice of data that hayro embeds with `include_bytes!`, and
//! the licences of the vendored C/C++ libraries (Tesseract, Leptonica).
//!
//! The negative half (a GPL/LGPL/AGPL *dependency* still fails
//! `cargo deny check licenses`) needs the cargo-deny binary, so it lives in
//! `tools/license/check-deny-rejects-copyleft.sh` and the `deny` CI job.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use toml::Table;

/// SHA-256 of https://www.gnu.org/licenses/gpl-3.0.txt (35149 bytes), fetched
/// 2026-10-02. `LICENSE` must stay byte-identical to it.
const GPL3_SHA256: &str = "3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986";
const SPDX: &str = "GPL-3.0-or-later";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn toml_of(rel: &str) -> Table {
    read(rel).parse().unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// Does this line assert that OUR project is MIT-licensed? Deliberately
/// narrow: third-party mentions ("Apache-2.0 OR MIT", "tesseract-rs (MIT)")
/// and the spec's history of the relicensing must pass. A line is flagged if it
/// contains
/// - `license = "MIT"` (a manifest field), or
/// - "project's MIT" / "project\u{2019}s MIT", or
/// - "MIT license" / "MIT-licensed" together with "our ", "project",
///   "this crate" or "pdf-extract" on the same line.
fn claims_own_license_is_mit(line: &str) -> bool {
    let lower = line.to_lowercase();
    if line.contains("license = \"MIT\"")
        || line.contains("project's MIT")
        || line.contains("project\u{2019}s MIT")
    {
        return true;
    }
    let mit_license = lower.contains("mit license") || lower.contains("mit-licensed");
    mit_license
        && ["our ", "project", "this crate", "pdf-extract"]
            .iter()
            .any(|k| lower.contains(k))
}

/// Tracked text files that are ours: not vendored, not the GPL text, not this
/// test (which spells the patterns out), not the lockfile, not the superseded
/// MIT record.
fn own_text_files() -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-files"])
        .current_dir(root())
        .output()
        .expect("run git ls-files");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|f| !f.starts_with("vendor/") && *f != "LICENSE")
        .filter(|f| *f != "crates/testkit/tests/license.rs")
        // ADR 0008 is the superseded record of the MIT decision: it states
        // the claim on purpose, as history (superseded by ADR 0009).
        .filter(|f| *f != "docs/adr/0008-license-mit.md")
        .filter(|f| {
            [".md", ".toml", ".py", ".rs", ".txt", ".sh", ".yml"]
                .iter()
                .any(|e| f.ends_with(e))
        })
        .map(String::from)
        .collect()
}

/// `cargo metadata` (format 1, offline) run at the repository root.
fn cargo_metadata(extra: &[&str]) -> serde_json::Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--offline"])
        .args(extra)
        .current_dir(root())
        .output()
        .expect("run cargo metadata");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("metadata JSON")
}

#[test]
fn license_file_is_canonical_gpl3() {
    let bytes = std::fs::read(root().join("LICENSE")).expect("LICENSE");
    let hex: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        hex, GPL3_SHA256,
        "LICENSE differs from the canonical gpl-3.0.txt"
    );
}

#[test]
fn every_workspace_member_is_gpl_and_unpublished() {
    let meta = cargo_metadata(&["--no-deps"]);
    let packages = meta["packages"].as_array().expect("packages");
    // core, ocr, python, render, tesseract-sys, testkit; the vendored crates
    // are excluded from the workspace, so `--no-deps` never lists them.
    assert_eq!(
        packages.len(),
        6,
        "unexpected workspace members: {packages:?}"
    );
    for p in packages {
        let name = p["name"].as_str().unwrap();
        assert_eq!(p["license"].as_str(), Some(SPDX), "{name}");
        // `[licenses.private] ignore = true` only exempts unpublished crates.
        assert_eq!(
            p["publish"],
            serde_json::json!([]),
            "{name} must be publish = false"
        );
    }
}

#[test]
fn vendored_crates_keep_their_upstream_license() {
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(root().join("vendor")).expect("vendor/") {
        let dir = entry.unwrap().path();
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest).unwrap();
        let table: Table = text.parse().unwrap();
        let license = table["package"]["license"]
            .as_str()
            .expect("vendored license");
        assert!(
            !license.contains("GPL"),
            "{}: {license}",
            manifest.display()
        );
        assert!(
            license.contains("MIT") || license.contains("Apache"),
            "{license}"
        );
        seen.push(table["package"]["name"].as_str().unwrap().to_string());
    }
    assert_every_vendored_hayro_crate(seen);
}

/// `seen`, the crates a test found under vendor/, is exactly the set
/// vendor/PROVENANCE.md imports (its `| hayro-` rows, the manifest import.sh
/// reads), which is the three original crates plus the two image codecs that
/// joined them at vendor/hayro-pristine-2.
fn assert_every_vendored_hayro_crate(mut seen: Vec<String>) {
    seen.sort();
    let mut manifest: Vec<String> = read("vendor/PROVENANCE.md")
        .lines()
        .filter_map(|l| l.strip_prefix("| hayro-"))
        .map(|rest| format!("hayro-{}", rest.split(' ').next().unwrap()))
        .collect();
    manifest.sort();
    assert_eq!(
        seen, manifest,
        "vendored hayro crates vs PROVENANCE.md rows"
    );
    assert_eq!(
        seen,
        [
            "hayro-interpret",
            "hayro-jbig2",
            "hayro-jpeg2000",
            "hayro-postscript",
            "hayro-syntax",
        ]
    );
}

#[test]
fn python_package_declares_gpl() {
    let py = toml_of("pyproject.toml");
    let project = py["project"].as_table().unwrap();
    assert_eq!(project["license"].as_str(), Some(SPDX));
    let classifiers: Vec<&str> = project["classifiers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    // PEP 639: the SPDX expression is the licence declaration; a "License ::"
    // classifier beside it is deprecated and could contradict it.
    assert!(
        !classifiers.iter().any(|c| c.starts_with("License ::")),
        "{classifiers:?}"
    );
    // The wheel embeds Adobe's AFM and AGL table; their notices must ship.
    let files: Vec<&str> = project["license-files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    for f in [
        "LICENSE",
        "THIRD_PARTY_NOTICES",
        "crates/testkit/data/README-AFM.txt",
        // The Rust project's notice for the statically linked std (Part 5).
        RUST_STD_COPYRIGHT,
    ] {
        assert!(files.contains(&f), "license-files lacks {f}");
        assert!(root().join(f).is_file(), "{f} missing");
    }
}

#[test]
fn claim_detector_flags_own_mit_claims_only() {
    for bad in [
        "binaries; our own work, under the project's MIT license.",
        "license = \"MIT\"",
        "pdf-extract is released under the MIT license",
    ] {
        assert!(claims_own_license_is_mit(bad), "{bad}");
    }
    for ok in [
        "`Apache-2.0 OR MIT`; see vendor/PROVENANCE.md",
        "We deliberately do not use tesseract-rs (MIT).",
        "move from MIT (Q1) to GPL-3.0-or-later",
        "(MIT OR Apache-2.0), which transcribes Apple's reference",
    ] {
        assert!(!claims_own_license_is_mit(ok), "{ok}");
    }
}

#[test]
fn no_mit_claim_for_our_own_code() {
    let files = own_text_files();
    assert!(
        files.len() > 20,
        "git ls-files looked wrong: {}",
        files.len()
    );
    for f in files {
        // Some tracked paths may be absent from a sparse checkout; skip them.
        let Ok(text) = std::fs::read_to_string(root().join(&f)) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            assert!(!claims_own_license_is_mit(line), "{f}:{}: {line}", i + 1);
        }
    }
}

/// The rule `tools/license/about.hbs` puts before each crate-licence section.
const SECTION_RULE: &str =
    "\n----------------------------------------------------------------------\n";

/// gen-notices.sh's statement for LAB.icc, which carries no licence of its own
/// (the user's decision: it ships under hayro-interpret's licence).
const LAB_ICC_STATEMENT: &str = "\
LAB.icc was generated with LCMS2 and states no licence of its own. It is
distributed under hayro-interpret's licence, Apache-2.0 OR MIT (Part 1).
";

fn assert_verbatim(notices: &str, label: &str, text: &str) {
    assert!(!text.trim().is_empty(), "{label} is empty");
    assert!(
        notices.contains(text),
        "THIRD_PARTY_NOTICES lacks {label} verbatim; rerun tools/license/gen-notices.sh"
    );
}

/// Every crate-licence section whose licence text is exactly `text`, from its
/// rule up to the text: the heading and the "Used by" list.
fn sections_with_text<'a>(notices: &'a str, text: &str) -> Vec<&'a str> {
    notices
        .match_indices(text)
        .filter_map(|(i, _)| notices[..i].rfind(SECTION_RULE).map(|r| &notices[r..i]))
        .collect()
}

/// Source directory of the single locked `name` in the extension's graph:
/// `vendor/<name>` for a vendored crate, else its registry copy. Filtered to
/// the host platform: offline, an unfiltered `cargo metadata` would need the
/// sources of Windows-only crates a Unix build never fetches.
fn locked_crate_dir(name: &str) -> PathBuf {
    static META: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    let meta = META.get_or_init(|| {
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
        let out = Command::new(rustc)
            .arg("-vV")
            .output()
            .expect("run rustc -vV");
        let info = String::from_utf8(out.stdout).unwrap();
        let host = info
            .lines()
            .find_map(|l| l.strip_prefix("host: "))
            .expect("rustc -vV host line")
            .to_string();
        cargo_metadata(&[
            "--locked",
            "--manifest-path",
            "crates/python/Cargo.toml",
            "--filter-platform",
            &host,
        ])
    });
    let found: Vec<_> = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["name"] == name)
        .collect();
    assert_eq!(found.len(), 1, "expected one locked {name}");
    Path::new(found[0]["manifest_path"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_path_buf()
}

/// about.toml must accept exactly deny.toml's licences, in the same order:
/// the order decides which licence of a dual-licensed crate ships.
#[test]
fn about_accepts_exactly_the_deny_allow_list() {
    let strings = |v: &toml::Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    let deny = toml_of("deny.toml");
    let about = toml_of("about.toml");
    assert_eq!(
        strings(&about["accepted"]),
        strings(&deny["licenses"]["allow"])
    );
    assert_eq!(about["accepted"][0].as_str(), Some("MIT"));
}

#[test]
fn notices_carry_each_vendored_hayro_crate_license() {
    let notices = read("THIRD_PARTY_NOTICES");
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(root().join("vendor")).expect("vendor/") {
        let dir = entry.unwrap().path();
        let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        let table: Table = manifest.parse().unwrap();
        let name = table["package"]["name"].as_str().unwrap();
        let version = table["package"]["version"].as_str().unwrap();
        // about.toml accepts MIT first, so a dual-licensed crate ships MIT,
        // whose file carries the copyright line.
        let mit = std::fs::read_to_string(dir.join("LICENSE-MIT"))
            .unwrap_or_else(|e| panic!("{name}: LICENSE-MIT: {e}"));
        assert_verbatim(&notices, &format!("{name}'s LICENSE-MIT"), &mit);
        let entry = "\nUsed by:\n";
        let line = format!("\n    - {name} {version}\n");
        assert!(
            sections_with_text(&notices, &mit)
                .iter()
                .any(|s| s.contains(entry) && s.contains(&line)),
            "no MIT section with its LICENSE-MIT text lists {name} {version}"
        );
        seen.push(name.to_string());
    }
    assert_every_vendored_hayro_crate(seen);
    // Our own crates are GPL (LICENSE), never a third-party section.
    assert!(!notices.contains("\n    - pdf-extract"), "own crate listed");
}

#[test]
fn notices_carry_each_embedded_asset_notice() {
    let notices = read("THIRD_PARTY_NOTICES");
    let assets = "vendor/hayro-interpret/assets";
    // Foxit .pfb standard fonts (PDFium BSD-3), the provenance README with the
    // Foxit attribution, and the CC0 terms of the CMYK profile.
    for f in ["LICENSE_FOXIT", "README.md", "CGATS_LICENSE.txt"] {
        let rel = format!("{assets}/{f}");
        assert_verbatim(&notices, &rel, &read(&rel));
    }
    // Every embedded font program is named, so a new one cannot slip past.
    let mut fonts = 0;
    for entry in std::fs::read_dir(root().join(assets)).unwrap() {
        let file = entry.unwrap().file_name().into_string().unwrap();
        if file.ends_with(".pfb") {
            assert!(notices.contains(&format!("\n    {file}\n")), "{file}");
            fonts += 1;
        }
    }
    assert_eq!(fonts, 14, "hayro-interpret embeds the 14 standard fonts");
    assert_verbatim(&notices, "the LAB.icc statement", LAB_ICC_STATEMENT);
    // Other crates that embed data: hayro-jpeg2000's CC0 ICC profiles (README
    // naming them, and the CC0 text) and hayro-cmap's Adobe CMap terms for
    // cmaps.brotli. Read from the locked copies: hayro-jpeg2000's is the
    // vendored one, hayro-cmap's the registry's.
    assert!(
        locked_crate_dir("hayro-jpeg2000").ends_with("vendor/hayro-jpeg2000"),
        "hayro-jpeg2000 must resolve onto vendor/ ([patch.crates-io])"
    );
    for (krate, file) in [
        ("hayro-jpeg2000", "assets/README.md"),
        ("hayro-jpeg2000", "assets/LICENSE.txt"),
        ("hayro-cmap", "assets/LICENSE.txt"),
    ] {
        let text = std::fs::read_to_string(locked_crate_dir(krate).join(file))
            .unwrap_or_else(|e| panic!("{krate} {file}: {e}"));
        assert_verbatim(&notices, &format!("{krate}'s {file}"), &text);
    }
    let mut profiles = 0;
    for entry in std::fs::read_dir(locked_crate_dir("hayro-jpeg2000").join("assets")).unwrap() {
        let file = entry.unwrap().file_name().into_string().unwrap();
        if file.ends_with(".icc") {
            assert!(notices.contains(&format!("\n    {file}\n")), "{file}");
            profiles += 1;
        }
    }
    assert_eq!(profiles, 3, "hayro-jpeg2000 embeds three ICC profiles");
    // The Adobe Glyph List notice for crates/core/src/text/agl.rs.
    assert_verbatim(
        &notices,
        "tools/license/agl-notice.txt",
        &read("tools/license/agl-notice.txt"),
    );
}

/// The version column of `source`'s row in vendor/PROVENANCE.md's C/C++ table
/// (`| tesseract | 5.5.3 | <url> | ...`).
fn c_source_version(source: &str) -> String {
    let provenance = read("vendor/PROVENANCE.md");
    let prefix = format!("| {source} | ");
    let rows: Vec<&str> = provenance
        .lines()
        .filter_map(|l| l.strip_prefix(&prefix))
        .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    assert_eq!(rows.len(), 1, "expected one {source} row in PROVENANCE.md");
    rows[0].split(" | ").next().unwrap().to_string()
}

/// Part 4 (spec §3.7): each vendored C/C++ library's licence file verbatim,
/// directly after its "Licence, <file> verbatim:" line, in a section whose
/// heading names the version PROVENANCE.md pins. Tesseract is Apache-2.0,
/// Leptonica BSD-2-Clause; both carry their copyright lines in these files.
#[test]
fn notices_carry_vendored_c_library_licenses() {
    let notices = read("THIRD_PARTY_NOTICES");
    let part = notices
        .find("\nPart 4. Vendored C/C++ libraries\n")
        .expect("THIRD_PARTY_NOTICES lacks Part 4; rerun tools/license/gen-notices.sh");
    for (source, title, file) in [
        ("tesseract", "Tesseract", "LICENSE"),
        ("leptonica", "Leptonica", "leptonica-license.txt"),
    ] {
        let rel = format!("vendor/{source}/{file}");
        let text = read(&rel);
        assert_verbatim(&notices, &rel, &text);
        let heading = format!("\n{title} {} (vendor/{source})\n", c_source_version(source));
        let at = notices[part..]
            .find(&heading)
            .unwrap_or_else(|| panic!("Part 4 lacks the heading {heading:?}"));
        let body = format!("Licence, {rel} verbatim:\n\n{text}");
        let found = notices[part + at..]
            .find(&body)
            .unwrap_or_else(|| panic!("{rel} is not verbatim under its Part 4 heading"));
        // The licence belongs to this section: no other section heading of
        // Part 4 sits between the heading and the text.
        assert!(
            !notices[part + at + heading.len()..part + at + found].contains(" (vendor/"),
            "{rel} is not in its own section"
        );
    }
}

const RUST_STD_COPYRIGHT: &str = "tools/license/rust-std-COPYRIGHT.html";

/// The `channel` pinned by rust-toolchain.toml (a release version).
fn pinned_rust() -> String {
    toml_of("rust-toolchain.toml")["toolchain"]["channel"]
        .as_str()
        .expect("rust-toolchain.toml pins a channel")
        .to_string()
}

/// Part 5: the Rust standard library of the pinned toolchain (licensing spec
/// "Standard library acknowledged", "System runtimes stated, not shipped").
/// Written by tools/license/rust_std.py from the toolchain's rust-src; the
/// Python test of that module compares this part with a fresh generation.
/// Here: the heading names the pin, so a toolchain bump without regenerating
/// fails; every crates.io row has its licence section; the in-tree crates
/// point at the committed copy of the Rust project's own notice.
#[test]
fn notices_carry_the_rust_standard_library() {
    let notices = read("THIRD_PARTY_NOTICES");
    let heading = format!("\nPart 5. Rust standard library (Rust {})\n", pinned_rust());
    let part = notices.find(&heading).unwrap_or_else(|| {
        panic!("THIRD_PARTY_NOTICES lacks {heading:?}; rerun tools/license/gen-notices.sh")
    });
    let part5 = &notices[part..];
    let rows: Vec<(&str, &str, &str)> = part5
        .lines()
        .filter_map(|l| l.strip_prefix("    "))
        .filter_map(|l| {
            let mut cols = l.split_whitespace();
            Some((cols.next()?, cols.next()?, cols.next()?))
        })
        .filter(|(_, _, origin)| *origin == "crates.io" || *origin == "rust-lang/rust")
        .collect();
    for krate in [
        "std",
        "core",
        "alloc",
        "compiler_builtins",
        "libc",
        "hashbrown",
    ] {
        assert!(
            rows.iter().any(|(n, ..)| *n == krate),
            "Part 5 does not list {krate}"
        );
    }
    for (name, version, origin) in &rows {
        if *origin == "crates.io" {
            // Whatever file name rust_std.py picked (LICENSE-MIT, COPYING, ...).
            let section = format!("\n{name}-{version}/");
            assert!(
                part5.contains(&section),
                "Part 5 lacks the licence of {name} {version}"
            );
        }
    }
    assert!(part5.contains("rust-std-COPYRIGHT.html"));
    for runtime in [
        "libstdc++",
        "libc++",
        "GCC Runtime Library Exception",
        "not shipped",
    ] {
        assert!(part5.contains(runtime), "Part 5 does not state {runtime:?}");
    }
    let copyright = read(RUST_STD_COPYRIGHT);
    assert!(
        copyright.contains("Copyright notices for The Rust Standard Library"),
        "{RUST_STD_COPYRIGHT} is not the toolchain's COPYRIGHT-library.html"
    );
}

/// Unfilled tokens of the SPDX / upstream licence templates. cargo-about falls
/// back to such a template when a crate ships no file for the licence it
/// picked; the result has no real copyright line, so it must be clarified in
/// about.toml instead of shipped.
const TEMPLATE_PLACEHOLDERS: &[&str] = &[
    "<year>",
    "<owner>",
    "<copyright holders>",
    "<name of author>",
    "<organization>",
    "[yyyy]",
    "[name of copyright owner]",
];

/// The Apache-2.0 appendix is itself instructions for applying the licence,
/// and its sample notice is this exact line with the bracketed tokens left in
/// on purpose ("replaced with your own identifying information"). An Apache
/// text reproduced verbatim may carry it, so it is not an unfilled template.
const APACHE_APPENDIX_SAMPLE: &str = "Copyright [yyyy] [name of copyright owner]";
const APACHE_APPENDIX_HEADING: &str = "APPENDIX: How to apply the Apache License to your work.";

/// `line:token` for each unfilled placeholder in `text`.
fn unfilled_placeholders(text: &str) -> Vec<String> {
    let mut in_apache_appendix = false;
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim() == SECTION_RULE.trim() {
            in_apache_appendix = false;
        }
        if line.trim() == APACHE_APPENDIX_HEADING {
            in_apache_appendix = true;
        }
        if in_apache_appendix && line.trim() == APACHE_APPENDIX_SAMPLE {
            continue;
        }
        let lower = line.to_lowercase();
        for t in TEMPLATE_PLACEHOLDERS {
            if lower.contains(t) {
                hits.push(format!("{}:{t}: {line}", i + 1));
            }
        }
    }
    hits
}

#[test]
fn placeholder_detector_flags_templates_only() {
    for bad in [
        "Copyright (c) <year> <owner>. ",
        "Copyright (c) <year> <copyright holders>",
        "Copyright [yyyy] [name of copyright owner]",
    ] {
        assert!(!unfilled_placeholders(bad).is_empty(), "{bad}");
    }
    let rule = SECTION_RULE.trim();
    for ok in [
        "Copyright (c) [2021] [Marvin Countryman]".to_string(),
        "Copyright (c) 2016 Dropbox, Inc.".to_string(),
        format!("{rule}\n   {APACHE_APPENDIX_HEADING}\n\n   {APACHE_APPENDIX_SAMPLE}\n"),
    ] {
        assert_eq!(unfilled_placeholders(&ok), Vec::<String>::new(), "{ok}");
    }
    // The appendix exemption ends at the next section.
    let leaked = format!("   {APACHE_APPENDIX_HEADING}\n{rule}\n{APACHE_APPENDIX_SAMPLE}\n");
    assert_eq!(unfilled_placeholders(&leaked).len(), 2, "{leaked}");
}

#[test]
fn notices_have_no_unfilled_template_placeholders() {
    let hits = unfilled_placeholders(&read("THIRD_PARTY_NOTICES"));
    assert!(
        hits.is_empty(),
        "THIRD_PARTY_NOTICES reproduces a licence template; clarify the crate in about.toml:\n{}",
        hits.join("\n")
    );
}
