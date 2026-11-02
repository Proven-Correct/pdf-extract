//! H20: a lone `+` is a number, and reads as 0 like a lone `-` (internal #120).
//!
//! hayro-syntax's `Number::skip` (used to validate an array before iterating
//! it) accepts a sign followed by white space as a zero, as pdf.js does, but
//! `Number::read` returned `None` for a lone `+` (only `-` and `.` were
//! recorded as a sign or a dot). `ArrayIter::next` then unwrapped that
//! `None`, so `[+ ]` validated and panicked on the first `next`, and the page
//! tree walk reached it through `/Kids [3 0 R + R]`. The fork now reads a
//! lone `+` as 0, and the iterator ends rather than unwrapping a failed
//! re-read.

use hayro_syntax::object::{Array, FromBytes, Number};
use pdf_extract_core::{Document, OpenOptions};

/// A one-page document whose root `/Kids` is `kids`.
fn pdf_with_kids(kids: &str) -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids {kids} /Count 1 >>"),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_string(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes(),
    );
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn a_lone_plus_reads_as_zero_like_a_lone_minus() {
    for text in ["+", "+ ", "-", "- ", "+ 5"] {
        let n = Number::from_bytes(text.as_bytes());
        assert_eq!(n.map(|n| n.as_i64()), Some(0), "{text:?}");
    }
}

#[test]
fn iterating_an_array_holding_a_lone_plus_does_not_panic() {
    for text in ["[+ ]", "[ + ]", "[1 + 2]", "[-  ]"] {
        let array = Array::from_bytes(text.as_bytes()).expect("validates");
        // Whatever the iterator yields, it must not unwrap a failed re-read.
        let _ = array.iter::<Number>().count();
        let _ = array.raw_iter().count();
    }
}

#[test]
fn opening_a_document_with_a_lone_plus_among_the_kids_does_not_panic() {
    for kids in ["[3 0 R + ]", "[+ 3 0 R]", "[80 + R]", "[3 + R]"] {
        let doc = Document::from_bytes(pdf_with_kids(kids), &OpenOptions::default());
        // Open may accept or refuse the tree; only a panic is a failure.
        let _ = doc.map(|d| d.page_count());
    }
    // The valid reference before the stray `+` still counts.
    let doc = Document::from_bytes(pdf_with_kids("[3 0 R + ]"), &OpenOptions::default());
    assert_eq!(doc.expect("opens").page_count(), 1);
}
