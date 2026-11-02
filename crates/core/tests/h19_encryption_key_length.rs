//! H19: an encryption `/Length` above 128 bits for the standard security
//! handler of revision 4 or earlier (internal #119).
//!
//! The file key of those revisions is (a prefix of) a 16-byte MD5 output, but
//! hayro-syntax sliced `hash[..Length / 8]`, so any `/Length` over 128 bits
//! panicked inside `Document::from_bytes`. The fork now caps the key at 16
//! bytes, as pdf.js does, so such a file opens and decrypts with that key.
//!
//! The fixtures are built here with an empty user password. `O`, `U` and the
//! RC4-encrypted content stream are precomputed (Algorithms 2, 4/5 and 1 of
//! ISO 32000-1 with the 16-byte key, independently of the code under test),
//! for `/ID` `10 11 ... 1f`, `/P -4`, `/O` bytes `41 42 ... 60`.

use pdf_extract_core::{Document, OpenOptions, TextOptions};

const O: &str = "4142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f60";
const ID: &str = "101112131415161718191a1b1c1d1e1f";

/// `U` and the encrypted content stream for revision 2 (no 50-round loop).
const U_R2: &str = "083f3961de67ab5c22a415b9db8c6fbfc709508ad70531d47a518ce36debf7af";
const CONTENT_R2: &str = "37c4c007e1f12aa105012f9181bebd72d8727ef99c522b27d0b192a8e77155421f044686";
/// The same for revisions 3 and 4 (`U` is 16 significant bytes, zero-padded).
const U_R34: &str = "b0a3526ad9a686c4b465d67bbce10cc200000000000000000000000000000000";
const CONTENT_R34: &str =
    "25d5d37586737635974c7744c342e601fc213a61b7b8a00628afe1e7d1892cf26baad548";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

/// A one-page encrypted PDF; `encrypt` is the body of the `/Encrypt`
/// dictionary (object 6) and `content` the bytes of the content stream.
fn encrypted_pdf(encrypt: &str, content: &[u8]) -> Vec<u8> {
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content);
    stream.extend_from_slice(b"\nendstream");
    let bodies: [Vec<u8>; 6] = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
          /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        stream,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        encrypt.as_bytes().to_vec(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
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
            "trailer\n<< /Size {} /Root 1 0 R /Encrypt 6 0 R /ID [<{ID}> <{ID}>] >>\n\
             startxref\n{xref_at}\n%%EOF\n",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A V2 (RC4) encryption dictionary.
fn rc4_dict(revision: u8, length: u16, u: &str) -> String {
    format!(
        "<< /Filter /Standard /V 2 /R {revision} /Length {length} /P -4 \
         /O <{O}> /U <{u}> >>"
    )
}

fn open(bytes: Vec<u8>) -> Document {
    Document::from_bytes(bytes, &OpenOptions::default()).expect("opens")
}

fn text_of(doc: &Document) -> String {
    doc.page(0)
        .expect("page")
        .text(&TextOptions::default())
        .expect("text")
        .trim_end()
        .to_owned()
}

#[test]
fn revision_3_lengths_above_128_bits_open_and_decrypt_with_a_16_byte_key() {
    // 136 is the first length over 16 bytes (the loop at the old :502); 256
    // is the length the fuzzer found.
    for length in [136, 256, 2040] {
        let doc = open(encrypted_pdf(
            &rc4_dict(3, length, U_R34),
            &unhex(CONTENT_R34),
        ));
        assert!(doc.is_encrypted(), "/Length {length}");
        assert_eq!(text_of(&doc), "Hello", "/Length {length}");
    }
}

#[test]
fn revision_2_length_above_128_bits_opens_and_decrypts_with_a_16_byte_key() {
    // Revision 2 skips the 50-round loop, so this reaches only the returned
    // key's slice (the old :506).
    let doc = open(encrypted_pdf(&rc4_dict(2, 256, U_R2), &unhex(CONTENT_R2)));
    assert_eq!(text_of(&doc), "Hello");
}

#[test]
fn a_length_of_exactly_128_bits_is_unchanged() {
    let doc = open(encrypted_pdf(&rc4_dict(3, 128, U_R34), &unhex(CONTENT_R34)));
    assert_eq!(text_of(&doc), "Hello");
}

#[test]
fn revision_4_and_revision_3_v5_dictionaries_with_oversized_lengths_open() {
    // V4 takes /Length from the dictionary and V5 forces 256, so both reach
    // the revision <= 4 key derivation when /R says so. The content stream is
    // left in the clear: the AES filters are not what is under test, only
    // that the open does not panic and authenticates the empty password.
    let cf = "/CF << /StdCF << /CFM /AESV2 /AuthEvent /DocOpen /Length 16 >> >> \
              /StmF /StdCF /StrF /StdCF";
    let v4 =
        format!("<< /Filter /Standard /V 4 /R 4 /Length 256 {cf} /P -4 /O <{O}> /U <{U_R34}> >>");
    let v5 = format!("<< /Filter /Standard /V 5 /R 3 {cf} /P -4 /O <{O}> /U <{U_R34}> >>");
    for (name, dict) in [("V4 R4", v4), ("V5 R3", v5)] {
        let doc = open(encrypted_pdf(&dict, b"BT ET"));
        assert!(doc.is_encrypted(), "{name}");
        assert_eq!(doc.page_count(), 1, "{name}");
    }
}
