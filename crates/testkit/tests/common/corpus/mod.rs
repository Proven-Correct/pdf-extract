//! The corpus-C catalogue (spec §4.13, Appendix C): every generated fixture
//! of `tests/fixtures/corpus-c/` with the recipe that builds it, the compat
//! points it pins and the parity gate it is held to.
//!
//! One list, two users, like `samples.rs`: `examples/corpus_c.rs` writes the
//! files and their `manifest.toml` entries, and `tests/corpus_c.rs` rebuilds
//! every file and checks it is byte-identical to the committed copy and that
//! the manifest still says what this catalogue says. So the committed bytes
//! are always reproducible from source, and the gate of a fixture is changed
//! here, in review, never by hand-editing the manifest.
//!
//! The oracle never runs here. `tools/oracle/gen.py` reads the committed
//! files from disk and writes `tests/fixtures/expectations/`; the gates are
//! checked by `crates/core/tests/parity.rs`. The encrypted fixtures are not
//! in this catalogue: the test kit never encrypts, and `mkfixtures.py enc`
//! owns them.
//!
//! Gates follow the spec's KP register (§1.8), residuals (§2.5) and
//! Appendix C: `exact` unless the spec documents the difference, in which
//! case `divergence` with the reason. A recipe whose oracle output turns out
//! to differ from ours is not relabelled here; that is a finding for the
//! spec owner.

#![allow(dead_code)] // the example and the test use different parts

mod content;
mod fonts;
mod layout;
mod structure;

use std::fmt::Write as _;

use pdf_extract_testkit::{PdfBuilder, TextStyle, text_width};

/// How `parity.rs` holds a fixture to the oracle (manifest `gate`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Our open outcome and every page's text equal the oracle's.
    Exact,
    /// We differ from the oracle on purpose; the reason cites the spec.
    /// Our output is pinned by a snapshot and must still differ.
    Divergence(&'static str),
    /// A hostile input (D-27, spec revision 10): gated on the open outcome
    /// class, the warning-kind set and our own pinned text (snapshot), never
    /// on the oracle's text, which depends on where the hostility sits.
    Hostile,
}

/// One corpus-C file.
#[derive(Clone, Debug)]
pub struct Fixture {
    /// File name under `corpus-c/` (also the recipe's name).
    pub file: String,
    /// The file's bytes.
    pub bytes: Vec<u8>,
    /// Compat points it pins (`KP1`, ...); empty for contract and hostile
    /// inputs.
    pub compat_points: Vec<&'static str>,
    /// The parity gate.
    pub gate: Gate,
    /// For hostile inputs (D-27): the exact set of warning kinds (document
    /// and pages, `Debug` names, sorted) our extraction must report.
    pub warnings: Option<Vec<&'static str>>,
}

impl Fixture {
    /// An `exact` fixture.
    pub fn exact(file: impl Into<String>, compat_points: &[&'static str], bytes: Vec<u8>) -> Self {
        Self {
            file: file.into(),
            bytes,
            compat_points: compat_points.to_vec(),
            gate: Gate::Exact,
            warnings: None,
        }
    }

    /// Turns the fixture into a documented divergence.
    #[must_use]
    pub fn diverges(mut self, reason: &'static str) -> Self {
        self.gate = Gate::Divergence(reason);
        self
    }

    /// A hostile fixture that must report exactly `kinds` (sorted here).
    pub fn hostile(file: impl Into<String>, bytes: Vec<u8>, kinds: &[&'static str]) -> Self {
        Self {
            gate: Gate::Hostile,
            ..Self::exact(file, &[], bytes)
        }
        .warns(kinds)
    }

    /// Records the warning kinds a fixture must produce.
    #[must_use]
    pub fn warns(mut self, kinds: &[&'static str]) -> Self {
        let mut kinds = kinds.to_vec();
        kinds.sort_unstable();
        self.warnings = Some(kinds);
        self
    }

    /// The manifest `generator` value.
    pub fn generator(&self) -> String {
        format!("testkit::corpus_c::{}", self.file)
    }
}

/// The whole catalogue, in a stable order (layout, content, fonts,
/// structure). File names are unique.
pub fn catalogue() -> Vec<Fixture> {
    let mut out = Vec::new();
    out.extend(layout::fixtures());
    out.extend(content::fixtures());
    out.extend(fonts::fixtures());
    out.extend(structure::fixtures());
    out
}

/// The `[[fixture]]` table of one entry, exactly as the manifest carries it.
/// `sha256` is the hex digest of the bytes (the caller computes it, so this
/// file needs no hashing dependency).
pub fn manifest_block(f: &Fixture, sha256: &str) -> String {
    let points: Vec<String> = f.compat_points.iter().map(|p| format!("\"{p}\"")).collect();
    let mut s = format!(
        "[[fixture]]\npath = \"corpus-c/{}\"\nsha256 = \"{sha256}\"\ngenerator = \"{}\"\n\
         compat_points = [{}]\ngate = ",
        f.file,
        f.generator(),
        points.join(", "),
    );
    match f.gate {
        Gate::Exact => s.push_str("\"exact\"\n"),
        Gate::Divergence(reason) => {
            let _ = write!(
                s,
                "\"divergence\"\ndivergence = \"{}\"\nsnapshot = \"snapshots/{sha256}.txt\"\n",
                toml_escape(reason)
            );
        }
        Gate::Hostile => {
            let _ = writeln!(s, "\"hostile\"\nsnapshot = \"snapshots/{sha256}.txt\"");
        }
    }
    if let Some(kinds) = &f.warnings {
        let kinds: Vec<String> = kinds.iter().map(|k| format!("\"{k}\"")).collect();
        let _ = writeln!(s, "warnings = [{}]", kinds.join(", "));
    }
    s
}

/// Escapes a TOML basic string (the reasons are plain text, but a quote or
/// backslash must not break the manifest).
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Markers around the generated section of `manifest.toml`.
pub const MANIFEST_BEGIN: &str = "# corpus_c:begin (generated by `cargo run -p pdf-extract-testkit --example corpus_c`; do not edit)";
/// See [`MANIFEST_BEGIN`].
pub const MANIFEST_END: &str = "# corpus_c:end";

// ---------------------------------------------------------------------------
// Shared recipe helpers

/// Helvetica at `size`, the builder's default font.
pub fn helv(size: f32) -> TextStyle {
    TextStyle::size(size)
}

/// The AFM width of a WinAnsi run.
pub fn width(text: &str, size: f32) -> f32 {
    text_width(text, size).expect("WinAnsi text")
}

/// One page `width` x 842 pt with Helvetica runs `(x, y, text, size)`, y
/// measured down from the top as `insert_text` does.
pub fn runs_on(page_width: f32, rs: &[(f32, f32, &str, f32)]) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let page = b.add_page(page_width, 842.0);
    for &(x, y, t, size) in rs {
        page.insert_text((x, y), t, helv(size))
            .expect("WinAnsi text");
    }
    b.to_bytes()
}

/// As [`runs_on`] on an A4 page.
pub fn runs(rs: &[(f32, f32, &str, f32)]) -> Vec<u8> {
    runs_on(595.0, rs)
}

/// One A4 page with raw content.
pub fn raw(content: &str) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(content);
    b.to_bytes()
}

/// A classic-xref PDF whose object `i + 1` is `bodies[i]`, with object 1 as
/// the catalog. Offsets are exact, so readers open it without repair. For
/// shapes the builder does not offer (inherited attributes, `/Contents`
/// arrays, named property lists, OCMDs).
pub fn assemble(bodies: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(bodies.len());
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1);
    for offset in offsets {
        let _ = writeln!(tail, "{offset:010} 00000 n ");
    }
    let _ = write!(
        tail,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        bodies.len() + 1,
        out.len()
    );
    out.extend_from_slice(tail.as_bytes());
    out
}

/// A stream object body with an exact `/Length`.
pub fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

/// The standard WinAnsi Helvetica font dictionary.
pub const HELV_DICT: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

/// File-name spelling of a number: `1`, `0.5`, `8.75` (Rust's shortest
/// round-trip form, which never uses an exponent for these values).
pub fn n(v: f32) -> String {
    format!("{v}")
}
