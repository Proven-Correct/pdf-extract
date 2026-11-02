//! Font dictionaries the builder can attach to a page: standard-14 Type 1
//! fonts with a chosen encoding, Type 3 fonts with glyph procedures, and
//! non-embedded Type 0 (CID) fonts.
//!
//! Nothing here embeds a font program. That is the point: the compat fixtures
//! probe how a reader maps codes to Unicode and positions glyphs when it must
//! fall back to its own metrics and tables (standard-14 substitutes, the
//! CID orderings' UCS2 maps, the standard Macintosh glyph order). Embedded
//! fonts come from imported corpus files instead.
//!
//! The types are plain data with public fields: a test states exactly the
//! dictionary it wants, and serialisation writes those entries and nothing
//! more, so a missing `/FontDescriptor` or `/ToUnicode` really is missing.

use std::fmt::Write as _;

use crate::pdfwriter::{Objects, hex_string, name, num, num_array, stream_body};

/// A font resource. See the module docs for what each kind is for.
#[derive(Clone, Debug, PartialEq)]
pub enum FontDict {
    /// `/Subtype /Type1`, normally one of the standard 14 names.
    Type1(Type1Font),
    /// `/Subtype /Type3` with content-stream glyph procedures.
    Type3(Type3Font),
    /// `/Subtype /Type0` with one non-embedded CIDFont descendant.
    Type0(Type0Font),
}

impl FontDict {
    /// Standard Helvetica with WinAnsiEncoding: the font `insert_text` uses.
    pub fn helvetica() -> Self {
        Self::standard14("Helvetica", SimpleEncoding::Named(BaseEncoding::WinAnsi))
    }

    /// A standard-14 Type 1 font with the given encoding, no ToUnicode.
    pub fn standard14(base_font: &str, encoding: SimpleEncoding) -> Self {
        Self::Type1(Type1Font {
            base_font: base_font.to_owned(),
            encoding,
            to_unicode: None,
        })
    }
}

/// A Type 1 font dictionary without an embedded program.
#[derive(Clone, Debug, PartialEq)]
pub struct Type1Font {
    /// `/BaseFont`, e.g. `Helvetica`, `Symbol`.
    pub base_font: String,
    /// `/Encoding`.
    pub encoding: SimpleEncoding,
    /// Optional `/ToUnicode` CMap program (see [`to_unicode_cmap`]).
    pub to_unicode: Option<String>,
}

/// The `/Encoding` entry of a simple font.
#[derive(Clone, Debug, PartialEq)]
pub enum SimpleEncoding {
    /// No `/Encoding` entry: the font's built-in encoding (StandardEncoding
    /// for the Latin standard-14 fonts, the font's own for Symbol/Dingbats).
    Builtin,
    /// `/Encoding /<name>`.
    Named(BaseEncoding),
    /// An encoding dictionary with `/Differences`; `base` becomes
    /// `/BaseEncoding` when present. Each pair is `(code, glyph name)`.
    Differences {
        /// Optional `/BaseEncoding`.
        base: Option<BaseEncoding>,
        /// `(code, glyph name)` pairs, written in the given order.
        differences: Vec<(u8, String)>,
    },
}

/// The predefined simple-font encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseEncoding {
    /// `WinAnsiEncoding`.
    WinAnsi,
    /// `MacRomanEncoding`.
    MacRoman,
    /// `MacExpertEncoding`.
    MacExpert,
}

impl BaseEncoding {
    fn pdf_name(self) -> &'static str {
        match self {
            Self::WinAnsi => "/WinAnsiEncoding",
            Self::MacRoman => "/MacRomanEncoding",
            Self::MacExpert => "/MacExpertEncoding",
        }
    }
}

/// A Type 3 font. Glyph procedures are content streams in glyph space and
/// must begin with `d0` or `d1`.
#[derive(Clone, Debug, PartialEq)]
pub struct Type3Font {
    /// `/FontMatrix`; `[0.001 0 0 0.001 0 0]` makes glyph space 1000 units/em.
    pub font_matrix: [f32; 6],
    /// `/FontBBox` in glyph space.
    pub font_bbox: [f32; 4],
    /// The glyphs; codes must be distinct, names may repeat (two codes
    /// sharing one procedure, which must then be identical). Violations panic
    /// at serialisation.
    pub glyphs: Vec<Type3Glyph>,
    /// Optional `/ToUnicode` CMap program.
    pub to_unicode: Option<String>,
}

/// One Type 3 glyph: code, name (in `/Differences` and `/CharProcs`), width
/// in glyph space (`/Widths`) and procedure.
#[derive(Clone, Debug, PartialEq)]
pub struct Type3Glyph {
    /// Character code (one byte).
    pub code: u8,
    /// Glyph name.
    pub name: String,
    /// Advance in glyph-space units.
    pub width: f32,
    /// Glyph procedure (content-stream operators).
    pub procedure: String,
}

/// A Type 0 font with one CIDFont descendant, no embedded program.
#[derive(Clone, Debug, PartialEq)]
pub struct Type0Font {
    /// `/BaseFont` of both the Type 0 font and the CIDFont.
    pub base_font: String,
    /// Predefined CMap name for `/Encoding`, e.g. `Identity-H`, `90ms-RKSJ-H`.
    pub encoding: String,
    /// The descendant's subtype.
    pub cid_subtype: CidFontSubtype,
    /// `/CIDSystemInfo /Registry`, normally `Adobe`.
    pub registry: String,
    /// `/CIDSystemInfo /Ordering`, e.g. `Identity`, `Japan1`.
    pub ordering: String,
    /// `/CIDSystemInfo /Supplement`.
    pub supplement: i32,
    /// `/DW`, written only when set.
    pub default_width: Option<f32>,
    /// Body of the `/W` array (without brackets), e.g. `"34 [600 600] 3 3 250"`.
    pub widths: Option<String>,
    /// Optional `/FontDescriptor` of the CIDFont.
    pub descriptor: Option<FontDescriptor>,
    /// Optional `/ToUnicode` CMap program on the Type 0 font.
    pub to_unicode: Option<String>,
}

impl Type0Font {
    /// Identity-H, `Adobe-Identity-0`, CIDFontType2, nothing optional.
    pub fn identity_h(base_font: &str) -> Self {
        Self {
            base_font: base_font.to_owned(),
            encoding: "Identity-H".to_owned(),
            cid_subtype: CidFontSubtype::Type2,
            registry: "Adobe".to_owned(),
            ordering: "Identity".to_owned(),
            supplement: 0,
            default_width: None,
            widths: None,
            descriptor: None,
            to_unicode: None,
        }
    }
}

/// `CIDFontType0` (CFF-based) or `CIDFontType2` (TrueType-based).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CidFontSubtype {
    /// `/CIDFontType0`.
    Type0,
    /// `/CIDFontType2`.
    Type2,
}

/// A font descriptor without a font file. Metrics are in 1/1000 em.
#[derive(Clone, Debug, PartialEq)]
pub struct FontDescriptor {
    /// `/Flags` (bit 6, value 32, = nonsymbolic).
    pub flags: u32,
    /// `/Ascent`.
    pub ascent: f32,
    /// `/Descent` (negative).
    pub descent: f32,
    /// `/CapHeight`.
    pub cap_height: f32,
    /// `/ItalicAngle`.
    pub italic_angle: f32,
    /// `/StemV`.
    pub stem_v: f32,
    /// `/FontBBox`.
    pub font_bbox: [f32; 4],
}

impl Default for FontDescriptor {
    fn default() -> Self {
        Self {
            flags: 32,
            ascent: 900.0,
            descent: -200.0,
            cap_height: 700.0,
            italic_angle: 0.0,
            stem_v: 80.0,
            font_bbox: [0.0, -200.0, 1000.0, 900.0],
        }
    }
}

/// Builds a ToUnicode CMap program mapping each code to a string, with
/// `code_bytes`-byte codes (1 for simple fonts, 2 for Identity CID fonts).
///
/// # Panics
/// If `code_bytes` is not 1 or 2, or a code does not fit in `code_bytes`
/// bytes (test-authoring errors).
pub fn to_unicode_cmap(code_bytes: usize, mappings: &[(u32, &str)]) -> String {
    assert!(matches!(code_bytes, 1 | 2), "code_bytes must be 1 or 2");
    let max = if code_bytes == 1 { 0xFF } else { 0xFFFF };
    if let Some((code, _)) = mappings.iter().find(|(c, _)| *c > max) {
        panic!("code {code:#X} does not fit in {code_bytes} byte(s)");
    }
    let digits = code_bytes * 2;
    let (lo, hi) = if code_bytes == 1 {
        ("00", "FF")
    } else {
        ("0000", "FFFF")
    };
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n",
    );
    let _ = writeln!(s, "1 begincodespacerange\n<{lo}> <{hi}>\nendcodespacerange");
    // bfchar blocks hold at most 100 entries each.
    for chunk in mappings.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (code, text) in chunk {
            let mut utf16 = Vec::new();
            for unit in text.encode_utf16() {
                utf16.extend_from_slice(&unit.to_be_bytes());
            }
            let _ = writeln!(s, "<{code:0digits$X}> {}", hex_string(&utf16));
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}

fn add_to_unicode(objs: &mut Objects, to_unicode: Option<&String>, dict: &mut String) {
    if let Some(cmap) = to_unicode {
        let id = objs.add(stream_body("", cmap.as_bytes()));
        let _ = write!(dict, " /ToUnicode {id} 0 R");
    }
}

fn differences_array(differences: &[(u8, String)]) -> String {
    let mut s = String::from("[");
    for (i, (code, glyph)) in differences.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{code} {}", name(glyph));
    }
    s.push(']');
    s
}

/// Writes the font's objects and returns the font dictionary's object number.
pub(crate) fn write_font(objs: &mut Objects, font: &FontDict) -> u32 {
    match font {
        FontDict::Type1(f) => {
            let mut d = format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont {}",
                name(&f.base_font)
            );
            match &f.encoding {
                SimpleEncoding::Builtin => {}
                SimpleEncoding::Named(b) => {
                    let _ = write!(d, " /Encoding {}", b.pdf_name());
                }
                SimpleEncoding::Differences { base, differences } => {
                    d.push_str(" /Encoding << /Type /Encoding");
                    if let Some(b) = base {
                        let _ = write!(d, " /BaseEncoding {}", b.pdf_name());
                    }
                    let _ = write!(d, " /Differences {} >>", differences_array(differences));
                }
            }
            add_to_unicode(objs, f.to_unicode.as_ref(), &mut d);
            d.push_str(" >>");
            objs.add(d)
        }
        FontDict::Type3(f) => write_type3(objs, f),
        FontDict::Type0(f) => write_type0(objs, f),
    }
}

fn write_type3(objs: &mut Objects, f: &Type3Font) -> u32 {
    let first = f.glyphs.iter().map(|g| g.code).min().unwrap_or(0);
    let last = f.glyphs.iter().map(|g| g.code).max().unwrap_or(0);
    let mut widths = vec![0.0f32; usize::from(last - first) + 1];
    let mut procs = String::new();
    // One procedure per name and one glyph per code: anything else would be
    // silently dropped from the dictionary, so it is rejected loudly.
    let mut seen: Vec<(&str, &str)> = Vec::new();
    let mut codes = [false; 256];
    for g in &f.glyphs {
        assert!(
            !codes[usize::from(g.code)],
            "Type 3 code {} given twice",
            g.code
        );
        codes[usize::from(g.code)] = true;
        widths[usize::from(g.code - first)] = g.width;
        if let Some((_, proc_)) = seen.iter().find(|(n, _)| *n == g.name) {
            assert_eq!(
                *proc_, g.procedure,
                "Type 3 glyph {} has two procedures",
                g.name
            );
        } else {
            seen.push((&g.name, &g.procedure));
            let id = objs.add(stream_body("", g.procedure.as_bytes()));
            let _ = write!(procs, " {} {id} 0 R", name(&g.name));
        }
    }
    let diffs: Vec<(u8, String)> = f.glyphs.iter().map(|g| (g.code, g.name.clone())).collect();
    let mut d = format!(
        "<< /Type /Font /Subtype /Type3 /FontMatrix {} /FontBBox {} /FirstChar {first} \
         /LastChar {last} /Widths {} /Encoding << /Type /Encoding /Differences {} >> \
         /CharProcs <<{procs} >> /Resources << >>",
        num_array(&f.font_matrix),
        num_array(&f.font_bbox),
        num_array(&widths),
        differences_array(&diffs),
    );
    add_to_unicode(objs, f.to_unicode.as_ref(), &mut d);
    d.push_str(" >>");
    objs.add(d)
}

fn write_type0(objs: &mut Objects, f: &Type0Font) -> u32 {
    let subtype = match f.cid_subtype {
        CidFontSubtype::Type0 => "/CIDFontType0",
        CidFontSubtype::Type2 => "/CIDFontType2",
    };
    let mut cid = format!(
        "<< /Type /Font /Subtype {subtype} /BaseFont {} /CIDSystemInfo << /Registry {} \
         /Ordering {} /Supplement {} >>",
        name(&f.base_font),
        crate::pdfwriter::text_string(&f.registry),
        crate::pdfwriter::text_string(&f.ordering),
        f.supplement,
    );
    if let Some(dw) = f.default_width {
        let _ = write!(cid, " /DW {}", num(dw));
    }
    if let Some(w) = &f.widths {
        let _ = write!(cid, " /W [{w}]");
    }
    if let Some(fd) = &f.descriptor {
        let id = objs.add(format!(
            "<< /Type /FontDescriptor /FontName {} /Flags {} /FontBBox {} /ItalicAngle {} \
             /Ascent {} /Descent {} /CapHeight {} /StemV {} >>",
            name(&f.base_font),
            fd.flags,
            num_array(&fd.font_bbox),
            num(fd.italic_angle),
            num(fd.ascent),
            num(fd.descent),
            num(fd.cap_height),
            num(fd.stem_v),
        ));
        let _ = write!(cid, " /FontDescriptor {id} 0 R");
    }
    cid.push_str(" >>");
    let cid_id = objs.add(cid);
    let mut d = format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont {} /Encoding {} /DescendantFonts [{cid_id} 0 R]",
        name(&f.base_font),
        name(&f.encoding),
    );
    add_to_unicode(objs, f.to_unicode.as_ref(), &mut d);
    d.push_str(" >>");
    objs.add(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(font: &FontDict) -> String {
        let mut objs = Objects::new();
        let id = write_font(&mut objs, font);
        let bytes = objs.to_bytes(id, "");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn type1_encodings() {
        assert!(body(&FontDict::helvetica()).contains(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
        ));
        assert!(
            !body(&FontDict::standard14("Symbol", SimpleEncoding::Builtin)).contains("/Encoding")
        );
        let diff = FontDict::standard14(
            "Helvetica",
            SimpleEncoding::Differences {
                base: Some(BaseEncoding::MacRoman),
                differences: vec![(65, "zzunknown".into()), (66, "uni2603".into())],
            },
        );
        assert!(body(&diff).contains(
            "/Encoding << /Type /Encoding /BaseEncoding /MacRomanEncoding \
             /Differences [65 /zzunknown 66 /uni2603] >>"
        ));
    }

    #[test]
    fn type3_widths_cover_code_range_and_procs_are_shared_by_name() {
        let glyph = |code, n: &str| Type3Glyph {
            code,
            name: n.into(),
            width: 500.0,
            procedure: "500 0 d0 0 0 400 400 re f".into(),
        };
        let f = FontDict::Type3(Type3Font {
            font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
            font_bbox: [0.0, 0.0, 500.0, 500.0],
            glyphs: vec![glyph(65, "g65"), glyph(67, "sq"), glyph(68, "sq")],
            to_unicode: Some(to_unicode_cmap(1, &[(65, "A")])),
        });
        let s = body(&f);
        assert!(s.contains("/FirstChar 65 /LastChar 68 /Widths [500 0 500 500]"));
        assert!(s.contains("/Differences [65 /g65 67 /sq 68 /sq]"));
        assert_eq!(
            s.matches("d0 0 0 400 400 re f").count(),
            2,
            "one proc per name"
        );
        assert!(s.contains("<41> <0041>"));
    }

    #[test]
    fn type0_writes_only_requested_entries() {
        let mut f = Type0Font::identity_h("Arial");
        assert!(!body(&FontDict::Type0(f.clone())).contains("FontDescriptor"));
        f.descriptor = Some(FontDescriptor::default());
        f.default_width = Some(1000.0);
        f.widths = Some("36 [722 722]".into());
        let s = body(&FontDict::Type0(f));
        assert!(s.contains("/Subtype /CIDFontType2 /BaseFont /Arial"));
        assert!(s.contains("/Ordering (Identity) /Supplement 0 >> /DW 1000 /W [36 [722 722]]"));
        assert!(s.contains("/Ascent 900 /Descent -200"));
        assert!(s.contains("/Encoding /Identity-H /DescendantFonts ["));
    }

    #[test]
    fn to_unicode_cmap_two_byte_codes_and_chunks() {
        let many: Vec<(u32, &str)> = (0..150).map(|i| (i, "x")).collect();
        let s = to_unicode_cmap(2, &many);
        assert!(s.contains("<0000> <FFFF>"));
        assert!(s.contains("100 beginbfchar") && s.contains("50 beginbfchar"));
        assert!(s.contains("<0095> <0078>"));
    }
}
