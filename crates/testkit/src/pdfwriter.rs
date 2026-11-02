//! Low-level PDF serialisation shared by the builder and the hostile generators.
//!
//! Two layers:
//!
//! - [`Objects`]: an in-memory table of object bodies keyed by object number.
//!   Numbers are allocated before bodies are known, so objects can refer to
//!   each other (a page to its parent, a form to itself) in any order.
//! - [`ObjWriter`]: a streaming writer over any `io::Write` that records the
//!   byte offset of every object and emits a classic cross-reference table
//!   with exact offsets. The 1M-page generator streams straight to a file
//!   through it; everything else goes through [`Objects::to_bytes`].
//!
//! Every file this crate writes has a correct xref. That matters: hayro and
//! MuPDF both silently repair a broken table, so a generator with stale
//! offsets would still "work" while exercising the repair path instead of the
//! normal one. Files meant to be broken break on purpose, elsewhere.
//!
//! Numbers and strings are formatted here so every generator agrees on the
//! byte encoding (the Python test kit must later reproduce these bytes).

use std::fmt::Write as _;
use std::io::{self, Write};

/// PDF header: version line plus a binary-marker comment (four bytes above
/// 127), which tells transfer tools the file is binary.
const HEADER: &[u8] = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n";

/// Streaming object writer with exact xref offsets.
pub(crate) struct ObjWriter<W: Write> {
    out: W,
    pos: u64,
    /// `offsets[n]` is the byte offset of object `n`; index 0 is the free-list
    /// head and never set.
    offsets: Vec<Option<u64>>,
}

impl<W: Write> ObjWriter<W> {
    /// Starts a file by writing the header.
    pub(crate) fn new(out: W) -> io::Result<Self> {
        Self::with_header(out, HEADER)
    }

    /// Starts a file with a caller-chosen header (corpus D's built files
    /// pin `%PDF-1.4`, spec 0002 §3.8).
    pub(crate) fn with_header(out: W, header: &[u8]) -> io::Result<Self> {
        let mut w = Self {
            out,
            pos: 0,
            offsets: vec![None],
        };
        w.raw(header)?;
        Ok(w)
    }

    fn raw(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    /// Writes object `id` (generation 0) with the given body, which is either a
    /// direct object or a dictionary followed by a `stream` section (see
    /// [`stream_body`]).
    pub(crate) fn object(&mut self, id: u32, body: &[u8]) -> io::Result<()> {
        let idx = id as usize;
        if self.offsets.len() <= idx {
            self.offsets.resize(idx + 1, None);
        }
        self.offsets[idx] = Some(self.pos);
        self.raw(format!("{id} 0 obj\n").as_bytes())?;
        self.raw(body)?;
        self.raw(b"\nendobj\n")
    }

    /// Writes the xref table and trailer and returns the underlying writer.
    ///
    /// Object numbers that were never written become free entries, so the
    /// table always covers `0..=max_id` in one subsection.
    pub(crate) fn finish(mut self, root: u32, extra_trailer: &str) -> io::Result<W> {
        let xref_at = self.pos;
        let size = self.offsets.len();
        let mut table = format!("xref\n0 {size}\n");
        for (n, off) in self.offsets.iter().enumerate() {
            match off {
                // Each entry is exactly 20 bytes including the two-byte EOL
                // (space + LF), as the format requires.
                Some(o) => {
                    let _ = writeln!(table, "{o:010} 00000 n ");
                }
                None if n == 0 => table.push_str("0000000000 65535 f \n"),
                None => table.push_str("0000000000 00000 f \n"),
            }
        }
        self.raw(table.as_bytes())?;
        let trailer = format!(
            "trailer\n<< /Size {size} /Root {root} 0 R{extra_trailer} >>\nstartxref\n{xref_at}\n%%EOF\n"
        );
        self.raw(trailer.as_bytes())?;
        Ok(self.out)
    }
}

/// In-memory object table: allocate numbers first, fill bodies later.
#[derive(Debug, Default)]
pub(crate) struct Objects {
    /// `bodies[i]` is object `i + 1`.
    bodies: Vec<Option<Vec<u8>>>,
}

impl Objects {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Reserves the next object number.
    pub(crate) fn alloc(&mut self) -> u32 {
        self.bodies.push(None);
        u32::try_from(self.bodies.len()).expect("fewer than 2^32 objects in a test file")
    }

    /// Sets the body of an allocated object.
    pub(crate) fn set(&mut self, id: u32, body: impl Into<Vec<u8>>) {
        self.bodies[id as usize - 1] = Some(body.into());
    }

    /// Allocates and sets in one step.
    pub(crate) fn add(&mut self, body: impl Into<Vec<u8>>) -> u32 {
        let id = self.alloc();
        self.set(id, body);
        id
    }

    /// Serialises the table. Objects are written in number order; an
    /// allocated object whose body was never set is left out (a free xref
    /// entry), which only a deliberately broken generator would do.
    pub(crate) fn to_bytes(&self, root: u32, extra_trailer: &str) -> Vec<u8> {
        self.to_bytes_with_header(root, extra_trailer, HEADER)
    }

    /// [`Objects::to_bytes`] with a caller-chosen header.
    pub(crate) fn to_bytes_with_header(
        &self,
        root: u32,
        extra_trailer: &str,
        header: &[u8],
    ) -> Vec<u8> {
        // Writing to a Vec cannot fail; the io::Result is an artefact of the
        // shared streaming writer.
        let mut w = ObjWriter::with_header(Vec::new(), header).expect("Vec write is infallible");
        for (i, body) in self.bodies.iter().enumerate() {
            if let Some(body) = body {
                let id = u32::try_from(i + 1).expect("object count checked at alloc");
                w.object(id, body).expect("Vec write is infallible");
            }
        }
        w.finish(root, extra_trailer)
            .expect("Vec write is infallible")
    }
}

/// Builds a stream object body: `<< entries /Length n >>` then the data.
/// `entries` is the inside of the dictionary without `/Length`.
pub(crate) fn stream_body(entries: &str, data: &[u8]) -> Vec<u8> {
    let sep = if entries.is_empty() { "" } else { " " };
    let mut out = format!("<< {entries}{sep}/Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// Formats a number the way every generator writes it: integers without a
/// decimal point, other values with the shortest decimal that round-trips
/// through `f32`. Rust's float `Display` never uses exponent notation, which
/// PDF does not allow.
///
/// # Panics
/// On NaN or infinity, which have no PDF spelling. Every public entry point
/// that takes user numbers either validates them (`insert_text`) or documents
/// this panic; a test kit that silently wrote `NaN` into a fixture would make
/// the reader under test look broken instead of the test.
pub(crate) fn num(v: f32) -> String {
    assert!(
        v.is_finite(),
        "non-finite number {v} cannot be written to a PDF"
    );
    if v == 0.0 {
        // Avoid "-0", which is legal but needlessly odd in content streams.
        return "0".to_owned();
    }
    if v.fract() == 0.0 && v.abs() < 1e9 {
        // Exact: the value is an integer well inside i64.
        #[allow(clippy::cast_possible_truncation)]
        return format!("{}", v as i64);
    }
    format!("{v}")
}

/// Space-separated numbers in brackets, e.g. a rectangle or matrix.
pub(crate) fn num_array(vals: &[f32]) -> String {
    let parts: Vec<String> = vals.iter().map(|&v| num(v)).collect();
    format!("[{}]", parts.join(" "))
}

/// Encodes a PDF name (with its leading slash). Bytes outside the regular
/// printable set, delimiters and `#` are written as `#xx`, so any Rust string
/// without NUL yields a valid name token.
///
/// # Panics
/// On NUL, which a PDF name cannot contain even escaped (test-authoring error).
pub(crate) fn name(s: &str) -> String {
    assert!(!s.contains('\0'), "PDF names cannot contain NUL: {s:?}");
    let mut out = String::with_capacity(s.len() + 1);
    out.push('/');
    for &b in s.as_bytes() {
        let regular = (b'!'..=b'~').contains(&b) && !b"()<>[]{}/%#".contains(&b);
        if regular {
            out.push(b as char);
        } else {
            let _ = write!(out, "#{b:02X}");
        }
    }
    out
}

/// Hex string `<...>` of raw bytes; immune to escaping mistakes.
pub(crate) fn hex_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2 + 2);
    out.push('<');
    for b in bytes {
        let _ = write!(out, "{b:02X}");
    }
    out.push('>');
    out
}

/// A PDF text string (used for `/Contents`, `/ActualText`-style values):
/// printable ASCII is written as a literal string with `\`, `(` and `)`
/// escaped; anything else as UTF-16BE with a byte-order mark, which every
/// reader must accept.
pub(crate) fn text_string(s: &str) -> String {
    if s.bytes().all(|b| (0x20..0x7F).contains(&b)) {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('(');
        for c in s.chars() {
            if matches!(c, '\\' | '(' | ')') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push(')');
        out
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in s.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        hex_string(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_format_without_exponent_or_negative_zero() {
        assert_eq!(num(742.0), "742");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(0.1), "0.1");
        assert_eq!(num(7.333), "7.333");
        assert_eq!(num(1e-7), "0.0000001");
        assert_eq!(num(-12.5), "-12.5");
        assert_eq!(num(3e9), "3000000000");
        assert_eq!(num_array(&[0.0, 0.0, 595.0, 842.0]), "[0 0 595 842]");
    }

    #[test]
    fn names_escape_delimiters_and_non_ascii() {
        assert_eq!(name("Helv"), "/Helv");
        assert_eq!(name("a b#(c)"), "/a#20b#23#28c#29");
        assert_eq!(name("é"), "/#C3#A9");
    }

    #[test]
    fn strings_escape_or_switch_to_utf16() {
        assert_eq!(hex_string(b"\x00A"), "<0041>");
        assert_eq!(text_string(r"a(b)\c"), r"(a\(b\)\\c)");
        assert_eq!(text_string("é"), "<FEFF00E9>");
    }

    #[test]
    fn xref_offsets_point_at_objects_and_free_entries_fill_gaps() {
        let mut objs = Objects::new();
        let a = objs.add("<< /Type /Catalog >>");
        let _gap = objs.alloc();
        let c = objs.add(stream_body("", b"BT ET"));
        let bytes = objs.to_bytes(a, "");
        // The header's binary marker is not UTF-8; blank it so byte offsets
        // and string offsets coincide.
        let text: String = bytes
            .iter()
            .map(|&b| if b < 0x80 { b as char } else { '?' })
            .collect();
        let xref_at: usize = text
            .rsplit("startxref\n")
            .next()
            .and_then(|t| t.lines().next())
            .and_then(|n| n.parse().ok())
            .expect("startxref");
        assert!(text[xref_at..].starts_with("xref\n0 4\n"));
        let entries: Vec<&str> = text[xref_at..].lines().skip(2).take(4).collect();
        assert_eq!(entries[0], "0000000000 65535 f ");
        assert_eq!(entries[2], "0000000000 00000 f ");
        for (id, entry) in [(a, entries[1]), (c, entries[3])] {
            let off: usize = entry[..10].parse().expect("offset");
            assert!(text[off..].starts_with(&format!("{id} 0 obj\n")));
        }
        assert!(text.contains("/Length 5 >>\nstream\nBT ET\nendstream"));
    }
}
