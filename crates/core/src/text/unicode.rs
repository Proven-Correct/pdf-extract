//! From glyph to Unicode (spec T26, T27; design spec §6.2).
//!
//! The chain, first hit wins:
//!
//! 1. the font's ToUnicode CMap, with the reference's rules for control
//!    characters (spec T26, revision 10b): an entry that is a **single**
//!    code point in U+0008-U+000D reads as U+0020; a single code point that
//!    is any other control (U+0000-U+001F, U+007F-U+009F) is ignored, and the
//!    chain goes on as if the map had no entry for that code; an entry of
//!    several code points is taken verbatim and only sanitised (below);
//! 2. simple fonts (Type 1, TrueType, Type 3): the glyph name from the
//!    encoding and `/Differences`, through the glyph list (`agl`) or a
//!    `uniXXXX` / `uXXXX[XX]` name, with the KP15 normalisation below;
//! 3. CID fonts of a known character collection: the collection's UCS2 CMap
//!    (hayro, fork change H6a);
//! 4. Identity CID fonts with an embedded TrueType/OpenType program: the
//!    program's `cmap`, reversed (hayro, H6b);
//!    4b. non-embedded Identity CID fonts: the CID as a glyph index in the
//!    standard Macintosh glyph order, then the glyph list (KP16);
//! 5. otherwise the glyph is unmapped and [`Unmapped`] decides.
//!
//! Whatever the source, emitted text is sanitised (D-10): NUL, C0 controls
//! other than tab, line feed and carriage return, and U+FFFE/U+FFFF become
//! U+FFFD; C1 controls and line feeds are kept, as the reference emits them.
//!
//! **Who does what.** For simple fonts, step 1 is hayro's
//! `Glyph::to_unicode()` (fork change H10h), the ToUnicode map's own answer
//! with no fallback, and core owns step 2 from H5's raw `glyph_name()` and
//! its own glyph list (`agl`, D-39), so hayro's glyph-name table never
//! decides, with or without a ToUnicode map. For composite fonts hayro's
//! `as_unicode()` covers steps 1, 3 and 4 (none of which uses glyph names).
//! Core adds the KP15 normalisation, step 4b and the unmapped policy.
//!
//! **KP15 is per encoding position, not per name.** The reference reads
//! WinAnsi 0xA0 (`nbspace`) as a space, WinAnsi 0xAD (`sfthyphen`) as a
//! hyphen and MacRoman 0xCA (`nbspace`) as a space (spec A.4). Re-probed
//! (2026-10-01): the same names elsewhere keep U+00A0 / U+00AD: from
//! `/Differences` (even one restating 0xA0 as `nbspace`), from a Type 3
//! font, or from an explicit ToUnicode entry. hayro does not say whether a
//! name came from the base encoding or from `/Differences`, so the rule here
//! is the closest one it allows: a Type 1 or TrueType font, no ToUnicode
//! map, and exactly those three (code, name) pairs. A `/Differences` entry
//! that restates one of them is the one probed case where this reads a space
//! and the reference does not.
//!
//! The same file decodes PDF text strings (`/ActualText`, spec T28):
//! UTF-16BE with a byte-order mark, UTF-8 with one (PDF 2.0), otherwise
//! PDFDocEncoding.

use hayro_interpret::font::{FontKind, Glyph};
use hayro_interpret::hayro_cmap::{BfString, CidFamily};

use super::agl::AGL;
use super::mac_glyphs::MAC_GLYPH_ORDER;
use super::options::Unmapped;
use super::record::CharText;

/// What the Unicode chain made of one glyph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Lookup {
    /// Steps 1-4b found text.
    Mapped(CharText),
    /// Nothing found; the raw code to emit under [`Unmapped::Cid`]: the
    /// character code for simple fonts, the CID for composite fonts (KP22).
    Unmapped(u32),
}

/// Runs the chain for one glyph. Mapped text is sanitised (D-10).
pub(super) fn lookup(glyph: &Glyph<'_>) -> Lookup {
    let found = match glyph.font_kind() {
        FontKind::Type0 => composite(glyph),
        FontKind::Type1 | FontKind::TrueType | FontKind::Type3 => simple(glyph),
    };
    match found {
        Lookup::Mapped(text) => Lookup::Mapped(sanitised(text)),
        unmapped @ Lookup::Unmapped(_) => unmapped,
    }
}

/// Steps 1 and 2 for simple fonts (module docs).
fn simple(glyph: &Glyph<'_>) -> Lookup {
    let name = glyph.glyph_name();
    if glyph.has_to_unicode() {
        // Step 1: the map's own entry, as written (an entry for U+0000 is
        // kept, then sanitised). A code the map lacks goes on to step 2 with
        // core's table, never hayro's (`as_unicode()` would use its own).
        // Control-valued single code points follow T26 (10b): a space, or
        // no entry at all.
        if let Some(found) = glyph.to_unicode()
            && let Entry::Text(text) = to_unicode_entry(&found)
        {
            return Lookup::Mapped(text);
        }
    } else if glyph.font_kind() != FontKind::Type3
        && let Some(c) = name.and_then(|n| kp15_position(glyph.char_code(), n))
    {
        // KP15 only applies without a ToUnicode map (an explicit entry for
        // these codes is honoured as written) and never to Type 3 fonts,
        // whose names always come from `/Differences` (re-probed).
        return Lookup::Mapped(CharText::from_char(c));
    }
    // Step 2 on the raw name, with core's table: hayro's never decides. A
    // glyph without a name is unmapped (T26 has no further step for simple
    // fonts).
    match name.and_then(name_to_unicode) {
        Some(c) => Lookup::Mapped(CharText::from_char(c)),
        None => Lookup::Unmapped(glyph.char_code()),
    }
}

/// Steps 1, 3, 4 (all inside hayro) and 4b for composite fonts.
fn composite(glyph: &Glyph<'_>) -> Lookup {
    // Step 1 first through the map's own entry, so the control rules of
    // T26 (10b) apply. An ignored entry must not reach `as_unicode()`,
    // which would return the same entry; hayro offers no way to ask for
    // steps 3 and 4 alone, so an ignored entry is unmapped. It never takes
    // step 4b either: that step is for fonts without a ToUnicode map, and
    // the reference emits the raw CID here (`tounicode_ctl_t0_*`: CID 91
    // reads `[`, not the Macintosh-order `x`).
    let mut ignored = false;
    if glyph.has_to_unicode()
        && let Some(found) = glyph.to_unicode()
    {
        match to_unicode_entry(&found) {
            Entry::Text(text) => return Lookup::Mapped(text),
            Entry::Ignored => ignored = true,
        }
    }
    if !ignored && let Some(found) = glyph.as_unicode() {
        return Lookup::Mapped(bf_text(&found));
    }
    let cid = glyph.cid();
    // Step 4b applies only where nothing better can exist: no program of the
    // font's own (whose glyph order would be authoritative), no ToUnicode map
    // (which is authoritative even where it is silent), and no character
    // collection with a UCS2 map. The reference treats CIDFontType0 and
    // CIDFontType2 alike here (re-probed), so the subtype, which hayro does
    // not expose, is not needed.
    let identity = glyph
        .character_collection()
        .is_none_or(|cc| cc.family == CidFamily::AdobeIdentity);
    if !glyph.is_embedded()
        && !glyph.has_to_unicode()
        && identity
        && let Some(c) = cid
            .and_then(|cid| MAC_GLYPH_ORDER.get(usize::try_from(cid).ok()?))
            .and_then(|name| agl_lookup(name))
    {
        return Lookup::Mapped(CharText::from_char(c));
    }
    Lookup::Unmapped(cid.unwrap_or_else(|| glyph.char_code()))
}

/// The text of an unmapped glyph under `policy`, or `None` to emit nothing.
pub(crate) fn unmapped_text(code: u32, policy: Unmapped) -> Option<char> {
    match policy {
        Unmapped::Skip => None,
        Unmapped::Replacement => Some(char::REPLACEMENT_CHARACTER),
        Unmapped::Cid => Some(
            char::from_u32(code)
                .filter(|&c| storable(c))
                .unwrap_or(char::REPLACEMENT_CHARACTER),
        ),
    }
}

/// Whether a code point may be emitted as is (T27, D-10): everything except
/// NUL, C0 controls other than tab, line feed and carriage return, and the
/// noncharacters U+FFFE and U+FFFF. Surrogates and values above U+10FFFF
/// cannot occur in a `char`: every decoder here turns them into U+FFFD.
fn storable(c: char) -> bool {
    !matches!(c, '\0'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}')
}

/// D-10 for text from any source: [`storable`] or U+FFFD.
pub(crate) fn sanitise(s: &str) -> std::borrow::Cow<'_, str> {
    if s.chars().all(storable) {
        std::borrow::Cow::Borrowed(s)
    } else {
        std::borrow::Cow::Owned(
            s.chars()
                .map(|c| {
                    if storable(c) {
                        c
                    } else {
                        char::REPLACEMENT_CHARACTER
                    }
                })
                .collect(),
        )
    }
}

fn sanitised(text: CharText) -> CharText {
    match sanitise(text.as_str()) {
        std::borrow::Cow::Borrowed(_) => text,
        std::borrow::Cow::Owned(clean) => CharText::new(&clean),
    }
}

/// What T26 step 1 makes of one ToUnicode entry.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Entry {
    /// Use this text (still to be sanitised, D-10).
    Text(CharText),
    /// Treat the code as absent from the map.
    Ignored,
}

/// T26 step 1 (revision 10b), probed against the reference: a single code
/// point in U+0008-U+000D is a space; any other single control code point
/// (`char::is_control`: U+0000-U+001F, U+007F-U+009F) is ignored; anything
/// else, and every entry of several code points, is taken as written.
fn to_unicode_entry(found: &BfString) -> Entry {
    let single = match found {
        BfString::Char(c) => Some(*c),
        BfString::String(s) => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        }
    };
    match single {
        Some('\u{8}'..='\u{D}') => Entry::Text(CharText::from_char(' ')),
        Some(c) if c.is_control() => Entry::Ignored,
        _ => Entry::Text(bf_text(found)),
    }
}

fn bf_text(found: &BfString) -> CharText {
    match found {
        BfString::Char(c) => CharText::from_char(*c),
        BfString::String(s) => CharText::new(s),
    }
}

/// Step 2 on a glyph name: the glyph list, then `uniXXXX` / `uXXXX` names.
pub(crate) fn name_to_unicode(name: &str) -> Option<char> {
    agl_lookup(name).or_else(|| unicode_from_name(name))
}

/// KP15: the encoding positions the reference reads as plain characters
/// (module docs): WinAnsi 0xA0 and MacRoman 0xCA `nbspace`, WinAnsi 0xAD
/// `sfthyphen`.
pub(crate) fn kp15_position(code: u32, name: &str) -> Option<char> {
    match (code, name) {
        (0xA0 | 0xCA, "nbspace") => Some(' '),
        (0xAD, "sfthyphen") => Some('-'),
        _ => None,
    }
}

/// The Adobe Glyph List entry for `name` (single code points only; see
/// `agl`).
pub(crate) fn agl_lookup(name: &str) -> Option<char> {
    AGL.binary_search_by(|(n, _)| n.cmp(&name))
        .ok()
        .map(|i| AGL[i].1)
}

/// `uniXXXX` or `uXXXX`-style names, parsed as hayro parses them for the
/// fonts it maps itself (any hex digits after the prefix, one code point), so
/// a name maps the same way whichever font kind shows it.
pub(crate) fn unicode_from_name(name: &str) -> Option<char> {
    let hex = name
        .strip_prefix("uni")
        .or_else(|| name.strip_prefix('u'))?;
    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
}

/// Decodes a PDF text string (ISO 32000-2 §7.9.2.2) as the reference does
/// (probed 2026-10-01): UTF-16BE after `FE FF`, UTF-16LE after `FF FE`,
/// UTF-8 after `EF BB BF`, PDFDocEncoding otherwise. A NUL ends the string
/// in every form, and so does a byte PDFDocEncoding leaves undefined (0x9F,
/// 0xAD); 0x7F is kept. Unpaired surrogates become U+FFFD (the reference
/// produces unrelated code points for them).
///
/// At most `max_chars` code points are decoded: the text device never needs
/// more than its remaining character budget, and this keeps a hostile
/// multi-megabyte string from being decoded in full each time it is used.
pub(crate) fn decode_text_string(bytes: &[u8], max_chars: usize) -> String {
    let until_nul = |c: &char| *c != '\0';
    if let Some(utf16) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        decode_utf16(utf16, u16::from_be_bytes, max_chars)
    } else if let Some(utf16) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        decode_utf16(utf16, u16::from_le_bytes, max_chars)
    } else if let Some(utf8) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(utf8)
            .chars()
            .take_while(until_nul)
            .take(max_chars)
            .collect()
    } else {
        bytes
            .iter()
            .map_while(|&b| pdf_doc_char(b))
            .take(max_chars)
            .collect()
    }
}

fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16, max_chars: usize) -> String {
    // A trailing odd byte is ignored, as the reference ignores it.
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| unit(pair))
        .take_while(|&u| u != 0);
    char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .take(max_chars)
        .collect()
}

/// PDFDocEncoding (ISO 32000-2, Annex D.2): Latin-1 except for 0x18-0x1F
/// (spacing accents) and 0x80-0xA0 (typographic characters, the euro sign).
/// `None` for the bytes that end a string (see [`decode_text_string`]).
fn pdf_doc_char(b: u8) -> Option<char> {
    const ACCENTS: [char; 8] = [
        '\u{02D8}', '\u{02C7}', '\u{02C6}', '\u{02D9}', '\u{02DD}', '\u{02DB}', '\u{02DA}',
        '\u{02DC}',
    ];
    const HIGH: [char; 31] = [
        '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{0192}',
        '\u{2044}', '\u{2039}', '\u{203A}', '\u{2212}', '\u{2030}', '\u{201E}', '\u{201C}',
        '\u{201D}', '\u{2018}', '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}',
        '\u{0141}', '\u{0152}', '\u{0160}', '\u{0178}', '\u{017D}', '\u{0131}', '\u{0142}',
        '\u{0153}', '\u{0161}', '\u{017E}',
    ];
    match b {
        0x00 | 0x9F | 0xAD => None,
        0x18..=0x1F => Some(ACCENTS[usize::from(b - 0x18)]),
        0x80..=0x9E => Some(HIGH[usize::from(b - 0x80)]),
        0xA0 => Some('\u{20AC}'),
        _ => Some(char::from(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glyph_list_is_sorted_and_has_the_names_the_fixtures_use() {
        assert!(
            AGL.windows(2).all(|w| w[0].0 < w[1].0),
            "sorted, no duplicates"
        );
        assert_eq!(AGL.len(), 4200);
        assert_eq!(agl_lookup("A"), Some('A'));
        assert_eq!(agl_lookup("alpha"), Some('\u{03B1}'));
        assert_eq!(agl_lookup("nbspace"), Some('\u{00A0}'));
        assert_eq!(agl_lookup("zzunknown"), None);
        assert_eq!(agl_lookup("whitesquare"), Some('\u{25A1}'));
        // TeX names are not in the AGL (D-39; a §2.5 residual).
        assert_eq!(agl_lookup("square"), None);
        // Several code points in the AGL: the reference leaves the name
        // unmapped, and so does the table.
        assert_eq!(agl_lookup("dalethatafpatah"), None);
        assert_eq!(agl_lookup("parenleftbig"), None);
    }

    #[test]
    fn glyph_names_map_per_step_two() {
        // Away from the KP15 positions the names keep their list values.
        assert_eq!(name_to_unicode("nbspace"), Some('\u{A0}'));
        assert_eq!(name_to_unicode("sfthyphen"), Some('\u{AD}'));
        assert_eq!(kp15_position(0xA0, "nbspace"), Some(' '));
        assert_eq!(kp15_position(0xCA, "nbspace"), Some(' '));
        assert_eq!(kp15_position(0xAD, "sfthyphen"), Some('-'));
        assert_eq!(kp15_position(0xAD, "nbspace"), None);
        assert_eq!(kp15_position(0x01, "nbspace"), None);
        assert_eq!(name_to_unicode("uni2603"), Some('\u{2603}'));
        assert_eq!(name_to_unicode("u1F600"), Some('\u{1F600}'));
        assert_eq!(
            name_to_unicode("udieresis"),
            Some('\u{00FC}'),
            "list before prefix"
        );
        assert_eq!(name_to_unicode("uniD800"), None, "surrogate");
        assert_eq!(name_to_unicode("u"), Some('u'), "a glyph-list name");
        assert_eq!(name_to_unicode("uZZ"), None);
        assert_eq!(name_to_unicode("g65"), None);
        assert_eq!(name_to_unicode("g128"), None);
    }

    #[test]
    fn the_mac_order_has_258_names_in_apple_order() {
        assert_eq!(MAC_GLYPH_ORDER.len(), 258);
        assert_eq!(
            &MAC_GLYPH_ORDER[..4],
            [".notdef", ".null", "nonmarkingreturn", "space"]
        );
        // KP16: CIDs 36, 37, 38, 3, 68 read as `ABC a`.
        let kp16: String = [36, 37, 38, 3, 68]
            .iter()
            .filter_map(|&i| agl_lookup(MAC_GLYPH_ORDER[i]))
            .collect();
        assert_eq!(kp16, "ABC a");
        assert_eq!(MAC_GLYPH_ORDER[257], "dcroat");
        // The three specials have no Unicode (the reference emits their raw
        // CIDs).
        assert!(MAC_GLYPH_ORDER[..3].iter().all(|n| agl_lookup(n).is_none()));
    }

    #[test]
    fn tounicode_control_rules_per_the_probe_table() {
        // Spec T26 / Appendix A.4 (revision 10b).
        let text = |s: &str| Entry::Text(CharText::new(s));
        for c in ['\u{8}', '\t', '\n', '\u{B}', '\u{C}', '\r'] {
            assert_eq!(to_unicode_entry(&BfString::Char(c)), text(" "), "{c:?}");
            assert_eq!(
                to_unicode_entry(&BfString::String(c.to_string())),
                text(" "),
                "{c:?} as a one-character string"
            );
        }
        for c in [
            '\0', '\u{1}', '\u{7}', '\u{E}', '\u{1F}', '\u{7F}', '\u{80}', '\u{85}', '\u{9F}',
        ] {
            assert_eq!(
                to_unicode_entry(&BfString::Char(c)),
                Entry::Ignored,
                "{c:?}"
            );
        }
        for c in ['\u{A0}', '\u{2028}', ' ', 'x', '\u{FEFF}'] {
            assert_eq!(to_unicode_entry(&BfString::Char(c)), text(&c.to_string()));
        }
        // Several code points: verbatim (sanitising comes later).
        for s in ["A\t", "\tA", "A\u{85}", "A\0", "A\nB", "A\u{A0}", "ffi"] {
            assert_eq!(
                to_unicode_entry(&BfString::String(s.to_owned())),
                text(s),
                "{s:?}"
            );
        }
        assert_eq!(to_unicode_entry(&BfString::String(String::new())), text(""));
    }

    #[test]
    fn sanitising_applies_to_any_text() {
        assert_eq!(sanitise("plain\t\n\r\u{85}"), "plain\t\n\r\u{85}");
        assert!(matches!(sanitise("ok"), std::borrow::Cow::Borrowed(_)));
        assert_eq!(
            sanitise("a\0b\u{1}c\u{FFFE}\u{FFFF}"),
            "a\u{FFFD}b\u{FFFD}c\u{FFFD}\u{FFFD}"
        );
        assert_eq!(sanitised(CharText::new("x\0")).as_str(), "x\u{FFFD}");
    }

    #[test]
    fn unmapped_policy() {
        let cid = |code| unmapped_text(code, Unmapped::Cid);
        assert_eq!(cid(65), Some('A'));
        assert_eq!(cid(200), Some('\u{C8}'));
        assert_eq!(cid(0x80), Some('\u{80}'), "C1 kept");
        for kept in [9, 10, 13, 32] {
            assert_eq!(cid(kept), char::from_u32(kept));
        }
        for bad in [
            0, 1, 8, 11, 12, 14, 31, 0xD800, 0xDFFF, 0xFFFE, 0xFFFF, 0x11_0000,
        ] {
            assert_eq!(cid(bad), Some('\u{FFFD}'), "{bad:#X}");
        }
        assert_eq!(cid(0x1F600), Some('\u{1F600}'));
        assert_eq!(unmapped_text(65, Unmapped::Replacement), Some('\u{FFFD}'));
        assert_eq!(unmapped_text(65, Unmapped::Skip), None);
    }

    #[test]
    fn text_strings_decode_per_their_marker() {
        // Each expectation was checked against the reference (module docs).
        assert_eq!(decode_text_string(b"Hello", 100), "Hello");
        assert_eq!(
            decode_text_string(&[0xFE, 0xFF, 0, 0x66, 0, 0x69], 100),
            "fi"
        );
        assert_eq!(
            decode_text_string(&[0xFF, 0xFE, 0x41, 0, 0x42, 0], 100),
            "AB"
        );
        // A surrogate pair, then a lone high surrogate, then an odd byte.
        let utf16 = [
            0xFE, 0xFF, 0xD8, 0x3D, 0xDE, 0x00, 0xD8, 0x00, 0x00, 0x41, 0x42,
        ];
        assert_eq!(decode_text_string(&utf16, 100), "\u{1F600}\u{FFFD}A");
        assert_eq!(
            decode_text_string(&[0xFE, 0xFF, 0, 0x41, 0, 0, 0, 0x42], 100),
            "A"
        );
        assert_eq!(
            decode_text_string(b"\xEF\xBB\xBFk\xC3\xA9\0z", 100),
            "k\u{e9}"
        );
        assert_eq!(
            decode_text_string(&[0x18, 0x1F, 0x80, 0x93, 0x9E, 0xA0, 0xE9, 0x7F, 0x01], 100),
            "\u{2D8}\u{2DC}\u{2022}\u{FB01}\u{17E}\u{20AC}\u{E9}\u{7F}\u{1}"
        );
        for end in [0x00, 0x9F, 0xAD] {
            assert_eq!(decode_text_string(&[0x41, end, 0x42], 100), "A", "{end:#X}");
        }
        assert_eq!(decode_text_string(b"abcdef", 3), "abc");
        assert_eq!(decode_text_string(&[0xFE, 0xFF, 0, 0x61, 0, 0x62], 1), "a");
        assert_eq!(decode_text_string(b"", 3), "");
    }
}
