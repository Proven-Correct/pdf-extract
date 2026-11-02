//! Helvetica metrics and the WinAnsi encoding, so fixtures can place text runs
//! exactly (gaps of 0.25 pt matter for the line-assembly sweeps).
//!
//! Widths come from Adobe's Core14 `Helvetica.afm` (shipped unmodified in
//! `data/`, with Adobe's redistribution notice in `data/README-AFM.txt`),
//! parsed once at first use. The WinAnsi table below is transcribed from the
//! PDF specification (ISO 32000-1, Annex D.2), not from any other
//! implementation. Kerning pairs in the AFM are deliberately ignored: a PDF
//! `Tj` does not kern, so the pen advances by the plain widths.
//!
//! "WinAnsi printable" here means the codes that have a glyph in
//! WinAnsiEncoding: 0x20-0x7E and 0x80-0xFF minus the five undefined codes
//! (0x81, 0x8D, 0x8F, 0x90, 0x9D). Control characters, 0x7F and anything
//! beyond are rejected rather than guessed at.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::BuildError;

const HELVETICA_AFM: &str = include_str!("../data/Helvetica.afm");

/// Glyph names for codes 0x20..=0x7E, in code order.
#[rustfmt::skip]
const ASCII_NAMES: [&str; 95] = [
    "space", "exclam", "quotedbl", "numbersign", "dollar", "percent", "ampersand",
    // 0x27 and 0x60 are quotesingle/grave in WinAnsi (StandardEncoding has the
    // curly quoteright/quoteleft there).
    "quotesingle", "parenleft", "parenright", "asterisk", "plus", "comma", "hyphen", "period",
    "slash", "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    "colon", "semicolon", "less", "equal", "greater", "question", "at", "A", "B", "C", "D", "E",
    "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X",
    "Y", "Z", "bracketleft", "backslash", "bracketright", "asciicircum", "underscore", "grave",
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z", "braceleft", "bar", "braceright", "asciitilde",
];

/// The 0x80..=0x9F block, where WinAnsi departs from Latin-1:
/// `(code, Unicode, glyph name)`. Codes absent here are undefined.
const HIGH_BLOCK: [(u8, char, &str); 27] = [
    (0x80, '\u{20AC}', "Euro"),
    (0x82, '\u{201A}', "quotesinglbase"),
    (0x83, '\u{0192}', "florin"),
    (0x84, '\u{201E}', "quotedblbase"),
    (0x85, '\u{2026}', "ellipsis"),
    (0x86, '\u{2020}', "dagger"),
    (0x87, '\u{2021}', "daggerdbl"),
    (0x88, '\u{02C6}', "circumflex"),
    (0x89, '\u{2030}', "perthousand"),
    (0x8A, '\u{0160}', "Scaron"),
    (0x8B, '\u{2039}', "guilsinglleft"),
    (0x8C, '\u{0152}', "OE"),
    (0x8E, '\u{017D}', "Zcaron"),
    (0x91, '\u{2018}', "quoteleft"),
    (0x92, '\u{2019}', "quoteright"),
    (0x93, '\u{201C}', "quotedblleft"),
    (0x94, '\u{201D}', "quotedblright"),
    (0x95, '\u{2022}', "bullet"),
    (0x96, '\u{2013}', "endash"),
    (0x97, '\u{2014}', "emdash"),
    (0x98, '\u{02DC}', "tilde"),
    (0x99, '\u{2122}', "trademark"),
    (0x9A, '\u{0161}', "scaron"),
    (0x9B, '\u{203A}', "guilsinglright"),
    (0x9C, '\u{0153}', "oe"),
    (0x9E, '\u{017E}', "zcaron"),
    (0x9F, '\u{0178}', "Ydieresis"),
];

/// Glyph names for codes 0xA0..=0xFF, whose Unicode values equal the code
/// (Latin-1). 0xA0 (no-break space) and 0xAD (soft hyphen) reuse the `space`
/// and `hyphen` glyphs, as the PDF specification's table notes.
#[rustfmt::skip]
const LATIN1_NAMES: [&str; 96] = [
    "space", "exclamdown", "cent", "sterling", "currency", "yen", "brokenbar", "section",
    "dieresis", "copyright", "ordfeminine", "guillemotleft", "logicalnot", "hyphen", "registered",
    "macron", "degree", "plusminus", "twosuperior", "threesuperior", "acute", "mu", "paragraph",
    "periodcentered", "cedilla", "onesuperior", "ordmasculine", "guillemotright", "onequarter",
    "onehalf", "threequarters", "questiondown", "Agrave", "Aacute", "Acircumflex", "Atilde",
    "Adieresis", "Aring", "AE", "Ccedilla", "Egrave", "Eacute", "Ecircumflex", "Edieresis",
    "Igrave", "Iacute", "Icircumflex", "Idieresis", "Eth", "Ntilde", "Ograve", "Oacute",
    "Ocircumflex", "Otilde", "Odieresis", "multiply", "Oslash", "Ugrave", "Uacute", "Ucircumflex",
    "Udieresis", "Yacute", "Thorn", "germandbls", "agrave", "aacute", "acircumflex", "atilde",
    "adieresis", "aring", "ae", "ccedilla", "egrave", "eacute", "ecircumflex", "edieresis",
    "igrave", "iacute", "icircumflex", "idieresis", "eth", "ntilde", "ograve", "oacute",
    "ocircumflex", "otilde", "odieresis", "divide", "oslash", "ugrave", "uacute", "ucircumflex",
    "udieresis", "yacute", "thorn", "ydieresis",
];

/// One WinAnsi code with its character and Helvetica advance (1/1000 em).
#[derive(Clone, Copy, Debug)]
struct Entry {
    code: u8,
    width: u16,
}

struct Tables {
    by_char: HashMap<char, Entry>,
    /// Characters in code order, for enumeration.
    chars: Vec<char>,
    ascender: f32,
    descender: f32,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(build_tables)
}

/// Parses the AFM (only `C ... ; WX w ; N name ; ...` lines and the
/// Ascender/Descender header) and joins it with the WinAnsi table. A WinAnsi
/// glyph missing from the AFM is a packaging bug, hence the panic: the AFM is
/// a fixed, checked-in file and `every_win_ansi_glyph_has_a_width` covers it.
fn build_tables() -> Tables {
    let mut widths: HashMap<&str, u16> = HashMap::new();
    let mut ascender = None;
    let mut descender = None;
    for line in HELVETICA_AFM.lines() {
        if let Some(rest) = line.strip_prefix("Ascender ") {
            ascender = rest.trim().parse::<f32>().ok();
        } else if let Some(rest) = line.strip_prefix("Descender ") {
            descender = rest.trim().parse::<f32>().ok();
        } else if line.starts_with("C ") {
            let mut wx = None;
            let mut glyph = None;
            for field in line.split(';').map(str::trim) {
                if let Some(v) = field.strip_prefix("WX ") {
                    wx = v.trim().parse::<u16>().ok();
                } else if let Some(v) = field.strip_prefix("N ") {
                    glyph = Some(v.trim());
                }
            }
            if let (Some(w), Some(n)) = (wx, glyph) {
                widths.insert(n, w);
            }
        }
    }

    let mut entries: Vec<(char, u8, &str)> = Vec::with_capacity(218);
    for (i, n) in ASCII_NAMES.iter().enumerate() {
        let code = 0x20 + u8::try_from(i).expect("95 entries");
        entries.push((char::from(code), code, n));
    }
    for &(code, c, n) in &HIGH_BLOCK {
        entries.push((c, code, n));
    }
    for (i, n) in LATIN1_NAMES.iter().enumerate() {
        let code = 0xA0 + u8::try_from(i).expect("96 entries");
        entries.push((char::from(code), code, n));
    }

    let mut by_char = HashMap::with_capacity(entries.len());
    let mut chars = Vec::with_capacity(entries.len());
    for (c, code, glyph) in entries {
        let width = *widths
            .get(glyph)
            .unwrap_or_else(|| panic!("Helvetica.afm has no width for WinAnsi glyph {glyph}"));
        by_char.insert(c, Entry { code, width });
        chars.push(c);
    }
    Tables {
        by_char,
        chars,
        ascender: ascender.expect("Helvetica.afm has an Ascender line") / 1000.0,
        descender: descender.expect("Helvetica.afm has a Descender line") / 1000.0,
    }
}

fn lookup(text: &str) -> Result<Vec<Entry>, BuildError> {
    let t = tables();
    text.chars()
        .map(|ch| {
            t.by_char
                .get(&ch)
                .copied()
                .ok_or(BuildError::NotWinAnsi { ch })
        })
        .collect()
}

/// Encodes `text` as WinAnsi bytes, or fails on the first character that has
/// no WinAnsi glyph.
pub fn encode_win_ansi(text: &str) -> Result<Vec<u8>, BuildError> {
    Ok(lookup(text)?.into_iter().map(|e| e.code).collect())
}

/// Advance width in points of `text` set in standard Helvetica at `fontsize`
/// with WinAnsiEncoding: the sum of the AFM widths, no kerning.
///
/// # Errors
/// [`BuildError::NotWinAnsi`] if a character has no WinAnsi glyph;
/// [`BuildError::NonFinite`] if `fontsize` is NaN or infinite.
pub fn text_width(text: &str, fontsize: f32) -> Result<f32, BuildError> {
    if !fontsize.is_finite() {
        return Err(BuildError::NonFinite);
    }
    let units: u64 = lookup(text)?.iter().map(|e| u64::from(e.width)).sum();
    // Sum in integers and scale once, so long strings do not accumulate
    // rounding error. The final narrowing to f32 is the API's precision.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let width = (units as f64 * f64::from(fontsize) / 1000.0) as f32;
    Ok(width)
}

/// Every character the builder accepts, in WinAnsi code order (218 of them).
pub fn win_ansi_chars() -> impl Iterator<Item = char> {
    tables().chars.iter().copied()
}

/// Helvetica's AFM `Ascender` and `Descender` in em units: `(0.718, -0.207)`.
pub fn helvetica_ascent_descent() -> (f32, f32) {
    let t = tables();
    (t.ascender, t.descender)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_win_ansi_glyph_has_a_width() {
        // build_tables panics otherwise; also pins the size of the set.
        assert_eq!(win_ansi_chars().count(), 95 + 27 + 96);
    }

    #[test]
    fn known_widths() {
        // Helvetica AFM: A 667, l 222, p 556, h 556, a 556.
        let w = text_width("Alpha", 1000.0).expect("ASCII");
        assert!((w - (667.0 + 222.0 + 556.0 + 556.0 + 556.0)).abs() < 1e-3);
        assert!((text_width("Alpha", 11.0).expect("ASCII") - 28.127).abs() < 1e-4);
        assert_eq!(text_width("", 11.0).expect("empty"), 0.0);
    }

    #[test]
    fn encoding_departs_from_latin1_only_in_the_high_block() {
        assert_eq!(
            encode_win_ansi("A\u{20AC}\u{e9}").expect("WinAnsi"),
            [0x41, 0x80, 0xE9]
        );
        assert_eq!(
            encode_win_ansi("\u{a0}\u{ad}").expect("WinAnsi"),
            [0xA0, 0xAD]
        );
        for bad in ['\u{81}', '\u{7f}', '\t', '\n', '\u{0}', '日', '\u{2603}'] {
            assert_eq!(
                encode_win_ansi(&bad.to_string()),
                Err(BuildError::NotWinAnsi { ch: bad })
            );
        }
    }

    #[test]
    fn ascent_descent_from_afm() {
        let (a, d) = helvetica_ascent_descent();
        assert!((a - 0.718).abs() < 1e-6 && (d + 0.207).abs() < 1e-6);
    }
}
