//! Hand-assembled embedded JBIG2 streams (ITU-T T.88, Annex D.3) for the
//! decoder memory-budget tests (run 0003 H14, oracle §4.4).
//!
//! No JBIG2 encoder is in the dependency graph, and the shapes under test
//! are hostile ones no encoder would write, so the streams are assembled
//! here segment by segment. Most symbol dictionaries and every text region
//! are Huffman-coded (`SDHUFF` / `SBHUFF` = 1) with the standard tables of
//! Annex B, so their data is a plain bit string that `Bits` writes. Bitmaps
//! in those come from three sources that need no entropy coder: an
//! uncompressed height-class collective bitmap (the one-pixel base symbol),
//! refinement/aggregation (a symbol drawn by placing the base symbol, which
//! is how an 8192 × 8192 symbol costs a few bytes), and MMR rows that are all
//! white (one `V0` code, a single `1` bit, per row), or the hand-coded MMR
//! of `mmr_collective_bitmap`.
//!
//! The arithmetic-coded data is either real, written by the MQ encoder in
//! `mq.rs` (`arithmetic_symbols`, `refinement_data`), or arbitrary bytes that
//! decode to whatever they give (the halftone test's pattern dictionary,
//! `arithmetic_export_dictionary`'s export flags,
//! `arithmetic_new_symbols_dictionary`'s whole stream, and `refined_symbol`
//! when it is handed something other than `refinement_data`).
//!
//! The streams decode without error under no budget (the tests compare
//! their pixels with and without one), except the two shapes whose
//! unbudgeted decode would allocate gigabytes (the symbol export, 512 GiB;
//! the 40,000-square halftone grid, over 6.4 GB), which the tests never run
//! unbudgeted, and whatever `arithmetic_export_dictionary`,
//! `arithmetic_new_symbols_dictionary` and `refined_symbol` are handed that
//! does not decode (the internal #94 export flags and the internal #116 height classes are
//! refused by design).

use crate::mq;
use crate::pdfwriter::{Objects, stream_body};

/// Segment types used here (T.88 7.3).
mod kind {
    pub(super) const SYMBOL_DICTIONARY: u8 = 0;
    pub(super) const IMMEDIATE_TEXT_REGION: u8 = 6;
    pub(super) const PATTERN_DICTIONARY: u8 = 16;
    pub(super) const IMMEDIATE_HALFTONE_REGION: u8 = 22;
    pub(super) const INTERMEDIATE_GENERIC_REGION: u8 = 36;
    pub(super) const IMMEDIATE_GENERIC_REGION: u8 = 38;
    pub(super) const PAGE_INFORMATION: u8 = 48;
    pub(super) const END_OF_PAGE: u8 = 49;
}

/// One line of a standard Huffman table (Annex B): a prefix code and the
/// range of values it introduces, `low .. low + 2^range_bits`, whose offset
/// follows the prefix in `range_bits` bits.
#[derive(Clone, Copy)]
struct Line {
    prefix: &'static str,
    low: i64,
    range_bits: u32,
}

const fn line(prefix: &'static str, low: i64, range_bits: u32) -> Line {
    Line {
        prefix,
        low,
        range_bits,
    }
}

/// The non-negative lines of the tables the streams use (the negative and
/// lower-range lines are never needed). Codes as in T.88 Tables B.1, B.2,
/// B.4, B.6, B.8 and B.11.
const TABLE_B1: &[Line] = &[
    line("0", 0, 4),
    line("10", 16, 8),
    line("110", 272, 16),
    line("111", 65_808, 32),
];
const TABLE_B2: &[Line] = &[
    line("0", 0, 0),
    line("10", 1, 0),
    line("110", 2, 0),
    line("1110", 3, 3),
    line("11110", 11, 6),
    line("111110", 75, 32),
];
const TABLE_B2_OOB: &str = "111111";
const TABLE_B4: &[Line] = &[
    line("0", 1, 0),
    line("10", 2, 0),
    line("110", 3, 0),
    line("1110", 4, 3),
    line("11110", 12, 6),
    line("11111", 76, 32),
];
const TABLE_B6: &[Line] = &[
    line("00", 0, 7),
    line("010", 128, 7),
    line("011", 256, 8),
    line("1100", 512, 9),
    line("1101", 1024, 10),
];
const TABLE_B8: &[Line] = &[line("00", 0, 1), line("100", 4, 4)];
const TABLE_B8_OOB: &str = "01";
const TABLE_B11: &[Line] = &[
    line("0", 1, 0),
    line("10", 2, 1),
    line("1100", 4, 0),
    line("1101", 5, 1),
    line("11100", 7, 1),
    line("11101", 9, 2),
    line("111100", 13, 2),
    line("1111010", 17, 2),
    line("1111011", 21, 3),
    line("1111100", 29, 4),
    line("1111101", 45, 5),
    line("1111110", 77, 6),
    line("1111111", 141, 32),
];

/// Table B.15 (the refinement deltas and offsets of a Huffman text region),
/// its codes assigned from the prefix lengths as B.3 does, lowest line
/// first within a length. The lower-range line (below -24) is left out:
/// `Bits::huffman` writes an offset above `low`, not below it.
const TABLE_B15: &[Line] = &[
    line("1111100", -24, 4),
    line("111100", -8, 2),
    line("11100", -4, 1),
    line("1100", -2, 0),
    line("100", -1, 0),
    line("0", 0, 0),
    line("101", 1, 0),
    line("1101", 2, 0),
    line("11101", 3, 1),
    line("111101", 5, 2),
    line("1111101", 9, 4),
    line("1111111", 25, 32),
];

/// An MSB-first bit string, the order the decoder reads Huffman data in.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    len: usize,
}

impl Bits {
    fn bit(&mut self, one: bool) {
        if self.len.is_multiple_of(8) {
            self.bytes.push(0);
        }
        if one {
            let last = self.bytes.len() - 1;
            self.bytes[last] |= 0x80 >> (self.len % 8);
        }
        self.len += 1;
    }

    fn code(&mut self, code: &str) {
        for c in code.chars() {
            self.bit(c == '1');
        }
    }

    fn uint(&mut self, value: u64, bits: u32) {
        for i in (0..bits).rev() {
            self.bit(value >> i & 1 == 1);
        }
    }

    /// `value` in a standard table: the first line whose range holds it.
    fn huffman(&mut self, table: &[Line], value: i64) {
        let line = table
            .iter()
            .find(|l| value >= l.low && value - l.low < 1_i64 << l.range_bits)
            .unwrap_or_else(|| panic!("{value} is outside the table's encodable lines"));
        self.code(line.prefix);
        self.uint(
            u64::try_from(value - line.low).expect("offset is non-negative"),
            line.range_bits,
        );
    }

    /// Pads to a byte boundary with zero bits.
    fn align(&mut self) {
        self.len = self.bytes.len() * 8;
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// A segment header and its data (T.88 7.2): `number`, `kind`, the
/// referred-to segments, page association 1.
fn segment(number: u32, kind: u8, referred: &[u32], data: &[u8]) -> Vec<u8> {
    let mut out = number.to_be_bytes().to_vec();
    out.push(kind);
    let count = u32::try_from(referred.len()).expect("referred count fits 29 bits");
    if count <= 4 {
        out.push(u8::try_from(count << 5).expect("short form"));
    } else {
        // Long form: 0b111 and a 29-bit count, then one retention bit per
        // referred segment plus one for this segment, all zero.
        out.extend_from_slice(&(0xE000_0000 | count).to_be_bytes());
        out.resize(out.len() + (referred.len() + 1).div_ceil(8), 0);
    }
    for &r in referred {
        assert!(r < number, "a segment refers only to earlier segments");
        match number {
            0..=256 => out.push(u8::try_from(r).expect("one-byte reference")),
            257..=65_536 => {
                out.extend_from_slice(&u16::try_from(r).expect("two bytes").to_be_bytes())
            }
            _ => out.extend_from_slice(&r.to_be_bytes()),
        }
    }
    out.push(1);
    out.extend_from_slice(
        &u32::try_from(data.len())
            .expect("segment fits")
            .to_be_bytes(),
    );
    out.extend_from_slice(data);
    out
}

/// A page-information segment's data (7.4.8): `width × height`, white, not
/// striped.
fn page_information(width: u32, height: u32) -> Vec<u8> {
    let mut d = width.to_be_bytes().to_vec();
    d.extend_from_slice(&height.to_be_bytes());
    d.extend_from_slice(&[0; 8]); // resolutions unknown
    d.push(0); // flags: default pixel white
    d.extend_from_slice(&[0, 0]); // striping
    d
}

/// A region segment information field (7.4.1), combination operator OR.
fn region_info(width: u32, height: u32, x: u32, y: u32) -> Vec<u8> {
    let mut d = width.to_be_bytes().to_vec();
    d.extend_from_slice(&height.to_be_bytes());
    d.extend_from_slice(&x.to_be_bytes());
    d.extend_from_slice(&y.to_be_bytes());
    d.push(0);
    d
}

/// The fixed part of a Huffman symbol dictionary (7.4.2.1): `flags`, then
/// the exported and new symbol counts. Refinement template 1 needs no
/// adaptive-template bytes.
fn symbol_dictionary_header(flags: u16, exported: u32, new: u32) -> Vec<u8> {
    let mut d = flags.to_be_bytes().to_vec();
    d.extend_from_slice(&exported.to_be_bytes());
    d.extend_from_slice(&new.to_be_bytes());
    d
}

/// The `SDAT` bytes (7.4.2.1.2) of an arithmetic dictionary with generic
/// template 0: its nominal adaptive pixels (3, -1), (-3, -1), (2, -2),
/// (-2, -2), as signed bytes.
const NOMINAL_AT: [u8; 8] = [3, 0xFF, 0xFD, 0xFF, 2, 0xFE, 0xFE, 0xFE];

/// `SDHUFF` = 1, standard tables B.4 / B.2 / B.1 / B.1.
const SD_HUFFMAN: u16 = 0x0001;
/// `SDREFAGG` = 1 and `SDRTEMPLATE` = 1.
const SD_REFAGG: u16 = 0x0002 | 0x1000;

/// The export flags of a dictionary with `total` symbols that exports
/// exactly those in `exported` (an increasing range), as runs (6.5.10).
fn export_runs(bits: &mut Bits, total: u32, exported: std::ops::Range<u32>) {
    bits.huffman(TABLE_B1, i64::from(exported.start));
    bits.huffman(TABLE_B1, i64::from(exported.end - exported.start));
    if exported.end < total {
        bits.huffman(TABLE_B1, i64::from(total - exported.end));
    }
}

/// A dictionary of one new 1 × 1 symbol whose pixel is black, exported.
/// Its height-class collective bitmap is stored uncompressed.
fn base_dictionary() -> Vec<u8> {
    let mut bits = Bits::default();
    bits.huffman(TABLE_B4, 1); // height-class delta: height 1
    bits.huffman(TABLE_B2, 1); // width delta: width 1
    bits.code(TABLE_B2_OOB); // end of the height class
    bits.huffman(TABLE_B1, 0); // collective bitmap size 0: uncompressed
    bits.align();
    bits.uint(0x80, 8); // the one row: pixel (0, 0) black
    export_runs(&mut bits, 1, 0..1);
    let mut d = symbol_dictionary_header(SD_HUFFMAN, 1, 1);
    d.extend(bits.into_bytes());
    d
}

/// A dictionary of one new symbol, `rows` (`#` black, `.` white), exported,
/// with its collective bitmap stored uncompressed like the base symbol's:
/// each row padded to a byte (6.5.9).
fn uncompressed_dictionary(rows: &[&str]) -> Vec<u8> {
    let size = mq::Bitmap::new(rows);
    let mut bits = Bits::default();
    bits.huffman(TABLE_B4, i64::from(size.height)); // height-class delta
    bits.huffman(TABLE_B2, i64::from(size.width)); // width delta
    bits.code(TABLE_B2_OOB); // end of the height class
    bits.huffman(TABLE_B1, 0); // collective bitmap size 0: uncompressed
    bits.align();
    for y in 0..size.height {
        for x in 0..size.width {
            bits.bit(size.pixel(i64::from(x), i64::from(y)) == 1);
        }
        bits.align();
    }
    export_runs(&mut bits, 1, 0..1);
    let mut d = symbol_dictionary_header(SD_HUFFMAN, 1, 1);
    d.extend(bits.into_bytes());
    d
}

/// A dictionary that refers to the base dictionary (one input symbol) and
/// adds one `side × side` symbol by aggregation: the base symbol placed
/// twice at (0, 0), so the new symbol has one black pixel at its origin.
/// Only the new symbol is exported. The symbol bitmap is allocated at full
/// size but only its first row is written, so a large one costs address
/// space, not resident memory.
fn aggregate_dictionary(side: u32) -> Vec<u8> {
    aggregate_dictionary_sized(side, side)
}

/// [`aggregate_dictionary`] with a `width × height` new symbol.
fn aggregate_dictionary_sized(width: u32, height: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.huffman(TABLE_B4, i64::from(height)); // height-class delta
    bits.huffman(TABLE_B2, i64::from(width)); // width delta
    bits.huffman(TABLE_B1, 2); // aggregate of two instances (6.5.8.2.1)
    // The embedded text region (6.5.8.2.2, Table 17): strip T, then two
    // instances at S = 0, T = 0 of symbol 0 (a one-bit uniform code: two
    // symbols in all), no refinement, then the end of the strip.
    bits.huffman(TABLE_B11, 1); // initial STRIPT = -1
    bits.huffman(TABLE_B11, 1); // DT: STRIPT = 0
    bits.huffman(TABLE_B6, 0); // first S
    bits.code("0"); // symbol ID 0
    bits.code("0"); // R_I = 0
    bits.huffman(TABLE_B8, 0); // delta S
    bits.code("0");
    bits.code("0");
    bits.code(TABLE_B8_OOB);
    bits.code(TABLE_B2_OOB); // end of the height class
    export_runs(&mut bits, 2, 1..2);
    let mut d = symbol_dictionary_header(SD_HUFFMAN | SD_REFAGG, 1, 1);
    d.extend(bits.into_bytes());
    d
}

/// A dictionary with no new symbols that exports all `count` of its input
/// symbols: with `count` references to a one-symbol dictionary, `count`
/// copies of that symbol.
fn export_dictionary(count: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    export_runs(&mut bits, count, 0..count);
    let mut d = symbol_dictionary_header(SD_HUFFMAN, count, 0);
    d.extend(bits.into_bytes());
    d
}

/// An immediate Huffman text region (7.4.3) of `width × height` at the page
/// origin placing input symbol `symbol` of `symbols` once, with its top-left
/// corner at (`x`, `y`) on the page.
fn text_region(width: u32, height: u32, symbols: u32, symbol: u32, x: u32, y: u32) -> Vec<u8> {
    text_region_repeated(width, height, symbols, symbol, (x, y), 1)
}

/// [`text_region`] placing the symbol `instances` times, the first with its
/// top-left corner at `(x, y)` and each later one a row below the one before,
/// in a strip of its own (6.4.5: DT = 1, first S unchanged), so that they
/// stay on the region as long as it is tall enough.
fn text_region_repeated(
    width: u32,
    height: u32,
    symbols: u32,
    symbol: u32,
    (x, y): (u32, u32),
    instances: u32,
) -> Vec<u8> {
    let mut d = region_info(width, height, 0, 0);
    // SBHUFF = 1, REFCORNER = TOPLEFT; standard tables B.6 / B.8 / B.11.
    d.extend_from_slice(&0x0011_u16.to_be_bytes());
    d.extend_from_slice(&0_u16.to_be_bytes());
    d.extend_from_slice(&instances.to_be_bytes());
    // The symbol ID code lengths (7.4.3.1.7): every symbol gets the same
    // length, the run code for which is the only one in the run-code table,
    // with a one-bit prefix "0".
    let code_len = 32 - symbols.saturating_sub(1).leading_zeros().min(31);
    let mut bits = Bits::default();
    for runcode in 0..35 {
        bits.uint(u64::from(runcode == code_len), 4);
    }
    for _ in 0..symbols {
        bits.code("0");
    }
    bits.align();
    // The first instance (6.4.5): STRIPT = -1, DT = y + 1, first S = x.
    bits.huffman(TABLE_B11, 1);
    bits.huffman(TABLE_B11, i64::from(y) + 1);
    bits.huffman(TABLE_B6, i64::from(x));
    bits.uint(u64::from(symbol), code_len);
    bits.code(TABLE_B8_OOB);
    // Each later one: the next strip (DT = 1), first S + 0.
    for _ in 1..instances {
        bits.huffman(TABLE_B11, 1);
        bits.huffman(TABLE_B6, 0);
        bits.uint(u64::from(symbol), code_len);
        bits.code(TABLE_B8_OOB);
    }
    d.extend(bits.into_bytes());
    d
}

/// An intermediate MMR generic region (7.4.6) of `side × side`, all white:
/// one `V0` code (a `1` bit) per row.
fn white_mmr_region(side: u32) -> Vec<u8> {
    let mut d = region_info(side, side, 0, 0);
    d.push(0x01); // MMR
    let mut bits = Bits::default();
    for _ in 0..side {
        bits.bit(true);
    }
    d.extend(bits.into_bytes());
    d
}

/// Assembles segments in order after the page information, closing with an
/// end-of-page segment.
fn stream(page: (u32, u32), segments: Vec<(u8, Vec<u32>, Vec<u8>)>) -> Vec<u8> {
    let mut out = segment(
        0,
        kind::PAGE_INFORMATION,
        &[],
        &page_information(page.0, page.1),
    );
    let mut number = 1;
    for (kind, referred, data) in segments {
        out.extend(segment(number, kind, &referred, &data));
        number += 1;
    }
    out.extend(segment(number, kind::END_OF_PAGE, &[], &[]));
    out
}

/// The symbol-export shape (run 0003 Appendix A.5): a `side × side` symbol
/// exported `copies` times by one dictionary that names the symbol's
/// dictionary `copies` times, on a `page × page` page. When `place` is set, a
/// text region draws the last copy at (`page / 4`, `page / 2`) so the page
/// is not blank.
///
/// Segments: 1 the one-pixel base symbol, 2 the `side × side` aggregate, 3
/// the exporting dictionary, 4 the optional text region. With `side = 8192`
/// and `copies = 65_535` an unbudgeted decode copies 8 MiB 65,535 times
/// (512 GiB).
pub fn symbol_export(side: u32, copies: u32, page: u32, place: bool) -> Vec<u8> {
    let mut segments = vec![
        (kind::SYMBOL_DICTIONARY, vec![], base_dictionary()),
        (kind::SYMBOL_DICTIONARY, vec![1], aggregate_dictionary(side)),
        (
            kind::SYMBOL_DICTIONARY,
            vec![2; copies as usize],
            export_dictionary(copies),
        ),
    ];
    if place {
        segments.push((
            kind::IMMEDIATE_TEXT_REGION,
            vec![3],
            text_region(page, page, copies, copies - 1, page / 4, page / 2),
        ));
    }
    stream((page, page), segments)
}

/// A `width × height` symbol drawn `instances` times by one text region
/// (7.4.3) that covers the `page × page` page, from (0, 0) down, each a row
/// below the one before. The symbol is the
/// aggregate of [`symbol_export`] (the one-pixel base symbol placed twice at
/// its origin), so a large one costs a few bytes.
///
/// Segments: 1 the base symbol, 2 the aggregate, 3 the text region.
pub fn repeated_symbol(width: u32, height: u32, instances: u32, page: u32) -> Vec<u8> {
    stream(
        (page, page),
        vec![
            (kind::SYMBOL_DICTIONARY, vec![], base_dictionary()),
            (
                kind::SYMBOL_DICTIONARY,
                vec![1],
                aggregate_dictionary_sized(width, height),
            ),
            (
                kind::IMMEDIATE_TEXT_REGION,
                vec![2],
                text_region_repeated(page, page, 1, 0, (0, 0), instances),
            ),
        ],
    )
}

/// A text region that names the one-symbol base dictionary `refs` times
/// (so its symbol list has `refs` entries) and draws symbol `refs - 1` at
/// (3, 5), on a 64 × 64 page.
pub fn text_region_refs(refs: u32) -> Vec<u8> {
    stream(
        (64, 64),
        vec![
            (kind::SYMBOL_DICTIONARY, vec![], base_dictionary()),
            (
                kind::IMMEDIATE_TEXT_REGION,
                vec![1; refs as usize],
                text_region(64, 64, refs, refs - 1, 3, 5),
            ),
        ],
    )
}

/// A symbol dictionary that names the one-symbol base dictionary `refs`
/// times and neither adds nor exports a symbol, on a 64 × 64 page: its only
/// cost that grows with `refs` is its input-symbol list.
pub fn referring_dictionary(refs: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    export_runs(&mut bits, refs, 0..0);
    let mut dictionary = symbol_dictionary_header(SD_HUFFMAN, 0, 0);
    dictionary.extend(bits.into_bytes());
    stream(
        (64, 64),
        vec![
            (kind::SYMBOL_DICTIONARY, vec![], base_dictionary()),
            (kind::SYMBOL_DICTIONARY, vec![1; refs as usize], dictionary),
        ],
    )
}

/// An arithmetic-coded dictionary (`SDHUFF` = 0, generic template 0 with
/// its nominal adaptive pixels) that names the one-symbol base dictionary
/// once, adds no symbol and declares one export, on a 64 × 64 page. With no
/// new symbols its data holds only the export flags, so `data` is the `IAEX`
/// stream, whatever it decodes to: `ac 01` decodes to runs of 0 forever once
/// the two bytes are used up (internal #94).
pub fn arithmetic_export_dictionary(data: &[u8]) -> Vec<u8> {
    let mut dictionary = 0_u16.to_be_bytes().to_vec();
    dictionary.extend_from_slice(&NOMINAL_AT);
    dictionary.extend_from_slice(&1_u32.to_be_bytes()); // exported
    dictionary.extend_from_slice(&0_u32.to_be_bytes()); // new
    dictionary.extend_from_slice(data);
    stream(
        (64, 64),
        vec![
            (kind::SYMBOL_DICTIONARY, vec![], base_dictionary()),
            (kind::SYMBOL_DICTIONARY, vec![1], dictionary),
        ],
    )
}

/// An arithmetic-coded dictionary (`SDHUFF` = 0, `SDREFAGG` = 0, generic
/// template 0 with its nominal adaptive pixels) with no input symbols that
/// declares `num_new` new symbols, all exported, on a 64 × 64 page, and is
/// never drawn. `data` is its whole arithmetic stream (height classes,
/// symbol bitmaps, export flags), whatever it decodes to: with `num_new` 3,
/// `06 c4` decodes, once the two bytes are used up, to height classes that
/// hold no symbol forever (internal #116); with `num_new` 1, `a0 b6` decodes one
/// symbol and its export flags from the padding.
pub fn arithmetic_new_symbols_dictionary(num_new: u32, data: &[u8]) -> Vec<u8> {
    let mut dictionary = 0_u16.to_be_bytes().to_vec();
    dictionary.extend_from_slice(&NOMINAL_AT);
    dictionary.extend_from_slice(&num_new.to_be_bytes()); // exported
    dictionary.extend_from_slice(&num_new.to_be_bytes()); // new
    dictionary.extend_from_slice(data);
    stream(
        (64, 64),
        vec![(kind::SYMBOL_DICTIONARY, vec![], dictionary)],
    )
}

/// An arithmetic-coded symbol dictionary (`SDHUFF` = 0, `SDREFAGG` = 0,
/// generic template 0 with its nominal adaptive pixels), coded by the
/// test kit's MQ encoder (`mq`), of `symbols`, each given as rows of `#`
/// (black) and `.` (white); consecutive symbols of one height form a
/// height class. Every symbol is exported, and a text region draws symbol
/// `draw` with its top-left corner at `at` on a `page.0 × page.1` page.
pub fn arithmetic_symbols(
    symbols: &[&[&str]],
    draw: u32,
    at: (u32, u32),
    page: (u32, u32),
) -> Vec<u8> {
    let mut enc = mq::Encoder::new();
    let mut height_delta = mq::IntegerEncoder::new(); // IADH
    let mut width_delta = mq::IntegerEncoder::new(); // IADW
    let mut export = mq::IntegerEncoder::new(); // IAEX
    let mut generic = vec![mq::Context::default(); 1 << 16];
    let signed = |v: u32| i32::try_from(v).expect("small");
    let mut class_height = 0;
    let mut rest = symbols;
    while let Some(first) = rest.first() {
        let height = mq::Bitmap::new(first).height;
        let class_len = rest
            .iter()
            .take_while(|s| mq::Bitmap::new(s).height == height)
            .count();
        let (class, tail) = rest.split_at(class_len);
        height_delta.encode(&mut enc, Some(signed(height) - signed(class_height)));
        class_height = height;
        let mut width = 0;
        for rows in class {
            let bitmap = mq::Bitmap::new(rows);
            width_delta.encode(&mut enc, Some(signed(bitmap.width) - signed(width)));
            width = bitmap.width;
            mq::encode_generic_template0(&mut enc, &mut generic, &bitmap);
        }
        width_delta.encode(&mut enc, None); // end of the height class
        rest = tail;
    }
    let count = u32::try_from(symbols.len()).expect("small");
    // No input symbols: a run of 0 not exported, then all of them.
    export.encode(&mut enc, Some(0));
    export.encode(&mut enc, Some(signed(count)));

    let mut dictionary = 0_u16.to_be_bytes().to_vec();
    dictionary.extend_from_slice(&NOMINAL_AT);
    dictionary.extend_from_slice(&count.to_be_bytes()); // exported
    dictionary.extend_from_slice(&count.to_be_bytes()); // new
    dictionary.extend(enc.finish());
    stream(
        page,
        vec![
            (kind::SYMBOL_DICTIONARY, vec![], dictionary),
            (
                kind::IMMEDIATE_TEXT_REGION,
                vec![1],
                text_region(page.0, page.1, count, draw, at.0, at.1),
            ),
        ],
    )
}

/// A refinement block for [`refined_symbol`] or [`refined_from`]: `rows`
/// coded against `reference` (both `#` black, `.` white) by the test
/// kit's MQ encoder (`mq`) with refinement template 1, the reference's
/// (0, 0) at `offset` in `rows` (the RDX/RDY the stream declares).
/// `refined_symbol`'s reference is `["#"]` at (0, 0).
pub fn refinement_data(reference: &[&str], rows: &[&str], offset: (i32, i32)) -> Vec<u8> {
    let mut enc = mq::Encoder::new();
    let mut contexts = vec![mq::Context::default(); 1 << 10];
    mq::encode_refinement_template1(
        &mut enc,
        &mut contexts,
        &mq::Bitmap::new(rows),
        &mq::Bitmap::new(reference),
        (i64::from(offset.0), i64::from(offset.1)),
    );
    enc.finish()
}

/// Like [`text_region_refs`], but the region's data stops before its
/// instance count: the decode charges the `refs`-entry symbol list (while
/// collecting SBSYMS), then fails to parse the region. What it needs to get
/// that far is the symbol list's cost alone, without the symbol-ID code
/// table that a complete region adds per named symbol.
pub fn text_region_cut_short(refs: u32) -> Vec<u8> {
    let mut data = region_info(64, 64, 0, 0);
    data.extend_from_slice(&0x0011_u16.to_be_bytes()); // SBHUFF = 1, TOPLEFT
    data.extend_from_slice(&0_u16.to_be_bytes()); // standard tables
    stream(
        (64, 64),
        vec![
            (kind::SYMBOL_DICTIONARY, vec![], base_dictionary()),
            (kind::IMMEDIATE_TEXT_REGION, vec![1; refs as usize], data),
        ],
    )
}

/// A dictionary of one new `side × side` symbol made by single-instance
/// refinement (REFAGGNINST = 1, 6.5.8.2.2) of the one-pixel base symbol at
/// offset (0, 0), exported alone, then drawn at (3, 5) on a 64 × 64 page.
/// In Huffman mode the refinement bitmap is arithmetic-coded in a sized
/// block (template 1, no typical prediction); `refinement_data` is that
/// block, whatever it decodes to. [`refinement_data`] codes a chosen bitmap
/// for it.
pub fn refined_symbol(side: u32, refinement_data: &[u8]) -> Vec<u8> {
    refined_stream(base_dictionary(), (side, side), (0, 0), refinement_data)
}

/// Like [`refined_symbol`], but the reference is `reference` (rows of `#`
/// black, `.` white, stored uncompressed), placed with its (0, 0) at
/// `offset` (each -1, 0 or 1) in the new symbol `target`, which is coded by
/// [`refinement_data`]: unlike the one-pixel base symbol, a full reference
/// puts every reference pixel of the template to work.
pub fn refined_from(reference: &[&str], target: &[&str], offset: (i32, i32)) -> Vec<u8> {
    let size = mq::Bitmap::new(target);
    refined_stream(
        uncompressed_dictionary(reference),
        (size.width, size.height),
        offset,
        &refinement_data(reference, target, offset),
    )
}

/// The stream of [`refined_symbol`] with `reference_dictionary` (one
/// exported symbol) as the reference, at `(rdx, rdy)`, and a
/// `width × height` new symbol.
fn refined_stream(
    reference_dictionary: Vec<u8>,
    (width, height): (u32, u32),
    (rdx, rdy): (i32, i32),
    refinement_data: &[u8],
) -> Vec<u8> {
    // Table B.15's codes for -1, 0 and 1.
    let offset_code = |v: i32| match v {
        -1 => "100",
        0 => "0",
        1 => "101",
        _ => panic!("offset {v} is outside the encodable lines"),
    };
    let mut bits = Bits::default();
    bits.huffman(TABLE_B4, i64::from(height)); // height-class delta
    bits.huffman(TABLE_B2, i64::from(width)); // width delta
    bits.huffman(TABLE_B1, 1); // one instance: a refinement (6.5.8.2.1)
    bits.code("0"); // symbol ID 0 (one bit: two symbols in all)
    bits.code(offset_code(rdx)); // RDX (Table B.15)
    bits.code(offset_code(rdy)); // RDY
    bits.huffman(
        TABLE_B1,
        i64::try_from(refinement_data.len()).expect("small"),
    );
    bits.align();
    for &byte in refinement_data {
        bits.uint(u64::from(byte), 8);
    }
    bits.code(TABLE_B2_OOB); // end of the height class
    export_runs(&mut bits, 2, 1..2);
    let mut refined = symbol_dictionary_header(SD_HUFFMAN | SD_REFAGG, 1, 1);
    refined.extend(bits.into_bytes());
    stream(
        (64, 64),
        vec![
            (kind::SYMBOL_DICTIONARY, vec![], reference_dictionary),
            (kind::SYMBOL_DICTIONARY, vec![1], refined),
            (
                kind::IMMEDIATE_TEXT_REGION,
                vec![2],
                text_region(64, 64, 1, 0, 3, 5),
            ),
        ],
    )
}

/// Like [`repeated_symbol`]'s aggregate, a `size.0 × size.1` symbol built
/// from two instances of the base symbol at its origin, but the second
/// instance is refined (`R_I` = 1, 6.4.11) into `target`, coded by
/// [`refinement_data`]; the symbol is drawn once at (3, 5) on a 64 × 64
/// page. So the symbol is `target` with its (0, 0) black, clipped to
/// `size`. `target` is at most 50 pixels a side (its RDX and RDY stay
/// above Table B.15's lower range).
///
/// Segments: 1 the base symbol, 2 the aggregate, 3 the text region.
pub fn refined_aggregate(size: (u32, u32), target: &[&str]) -> Vec<u8> {
    let refined = mq::Bitmap::new(target);
    let rdw = i64::from(refined.width) - 1; // the reference is 1 × 1
    let rdh = i64::from(refined.height) - 1;
    let data = refinement_data(&["#"], target, (0, 0));
    let mut bits = Bits::default();
    bits.huffman(TABLE_B4, i64::from(size.1)); // height-class delta
    bits.huffman(TABLE_B2, i64::from(size.0)); // width delta
    bits.huffman(TABLE_B1, 2); // aggregate of two instances
    // The embedded text region, as in `aggregate_dictionary_sized`.
    bits.huffman(TABLE_B11, 1); // initial STRIPT = -1
    bits.huffman(TABLE_B11, 1); // DT: STRIPT = 0
    bits.huffman(TABLE_B6, 0); // first S
    bits.code("0"); // symbol ID 0
    bits.code("0"); // R_I = 0
    bits.huffman(TABLE_B8, 0); // delta S
    bits.code("0"); // symbol ID 0
    bits.code("1"); // R_I = 1
    bits.huffman(TABLE_B15, rdw);
    bits.huffman(TABLE_B15, rdh);
    // The reference sits at (RDW / 2 + RDX, RDH / 2 + RDY) in the refined
    // bitmap (6.4.11.1, rounding down): these offsets put it at (0, 0).
    bits.huffman(TABLE_B15, -rdw.div_euclid(2));
    bits.huffman(TABLE_B15, -rdh.div_euclid(2));
    bits.huffman(TABLE_B1, i64::try_from(data.len()).expect("small")); // RSIZE
    bits.align();
    for &byte in &data {
        bits.uint(u64::from(byte), 8);
    }
    bits.code(TABLE_B8_OOB); // end of the strip
    bits.code(TABLE_B2_OOB); // end of the height class
    export_runs(&mut bits, 2, 1..2);
    let mut dictionary = symbol_dictionary_header(SD_HUFFMAN | SD_REFAGG, 1, 1);
    dictionary.extend(bits.into_bytes());
    stream(
        (64, 64),
        vec![
            (kind::SYMBOL_DICTIONARY, vec![], base_dictionary()),
            (kind::SYMBOL_DICTIONARY, vec![1], dictionary),
            (
                kind::IMMEDIATE_TEXT_REGION,
                vec![2],
                text_region(64, 64, 1, 0, 3, 5),
            ),
        ],
    )
}

/// The refinement of [`refined_aggregate`] in a page-level text region
/// rather than one an aggregate symbol embeds: an immediate Huffman text
/// region with `SBREFINE` = 1 whose one instance refines the one-pixel base
/// symbol (`R_I` = 1, 6.4.11) into `target`, coded by [`refinement_data`]
/// with the reference at `target`'s (0, 0), and draws it with its top-left
/// corner at (3, 5) on a 64 × 64 page. The refined bitmap is drawn and
/// dropped, never stored as a symbol. `target` is at most 50 pixels a side,
/// as for `refined_aggregate`, and must fit the page from (3, 5).
///
/// Segments: 1 the base symbol, 2 the text region.
pub fn refined_text_region(target: &[&str]) -> Vec<u8> {
    let refined = mq::Bitmap::new(target);
    let rdw = i64::from(refined.width) - 1; // the reference is 1 × 1
    let rdh = i64::from(refined.height) - 1;
    let data = refinement_data(&["#"], target, (0, 0));
    let mut region = region_info(64, 64, 0, 0);
    // SBHUFF = 1, SBREFINE = 1, REFCORNER = TOPLEFT, SBRTEMPLATE = 1 (so no
    // refinement adaptive-template bytes follow).
    region.extend_from_slice(&0x8013_u16.to_be_bytes());
    // Standard tables B.6 / B.8 / B.11 for the placement; B.15 for RDW, RDH,
    // RDY and RDX (selector 1 each, bits 6-13); B.1 for RSIZE (bit 14 = 0).
    // These are the tables an embedded region uses (6.5.8.2.2, Table 17), so
    // the refinement codes match `refined_aggregate`'s.
    region.extend_from_slice(&0x1540_u16.to_be_bytes());
    region.extend_from_slice(&1_u32.to_be_bytes()); // one instance
    let mut bits = Bits::default();
    // The symbol ID code lengths, as in `text_region_repeated` with one
    // symbol: a one-bit code, length 1 in the run-code table.
    for runcode in 0..35 {
        bits.uint(u64::from(runcode == 1), 4);
    }
    bits.code("0");
    bits.align();
    bits.huffman(TABLE_B11, 1); // initial STRIPT = -1
    bits.huffman(TABLE_B11, 6); // DT: STRIPT = 5
    bits.huffman(TABLE_B6, 3); // first S = 3
    bits.code("0"); // symbol ID 0
    bits.code("1"); // R_I = 1
    bits.huffman(TABLE_B15, rdw);
    bits.huffman(TABLE_B15, rdh);
    // The reference sits at (RDW / 2 + RDX, RDH / 2 + RDY) in the refined
    // bitmap (6.4.11.1, rounding down): these offsets put it at (0, 0).
    bits.huffman(TABLE_B15, -rdw.div_euclid(2));
    bits.huffman(TABLE_B15, -rdh.div_euclid(2));
    bits.huffman(TABLE_B1, i64::try_from(data.len()).expect("small")); // RSIZE
    bits.align();
    for &byte in &data {
        bits.uint(u64::from(byte), 8);
    }
    bits.code(TABLE_B8_OOB); // end of the strip
    region.extend(bits.into_bytes());
    stream(
        (64, 64),
        vec![
            (kind::SYMBOL_DICTIONARY, vec![], base_dictionary()),
            (kind::IMMEDIATE_TEXT_REGION, vec![1], region),
        ],
    )
}

/// A Huffman dictionary of two symbols, 4 and 8 pixels wide and three high,
/// whose height-class collective bitmap is MMR-coded (6.5.9, `BMSIZE` > 0):
/// column 0 of each symbol is black and the rest white. The 12-pixel rows
/// are black at x = 0 and x = 4; the first row is coded in horizontal mode
/// against the all-white reference line and the others as vertical copies of
/// the row above. Both symbols are exported and the wider one is drawn at
/// (3, 5) on a 64 × 64 page.
pub fn mmr_collective_bitmap() -> Vec<u8> {
    // T.4 terminating codes: white 0 `00110101`, white 3 `1000`, black 1
    // `010`; horizontal mode `001`, vertical zero `1`.
    let mut mmr = Bits::default();
    mmr.code("001"); // a0a1 = 0 white, a1a2 = 1 black
    mmr.code("00110101");
    mmr.code("010");
    mmr.code("001"); // 3 white (x = 1..4), 1 black (x = 4)
    mmr.code("1000");
    mmr.code("010");
    mmr.code("1"); // the white tail, to the reference line's end
    for _ in 1..3 {
        mmr.code("11111"); // each of the row above's four edges, then the end
    }
    let mmr = mmr.into_bytes();
    let mut bits = Bits::default();
    bits.huffman(TABLE_B4, 3); // height-class delta: height 3
    bits.huffman(TABLE_B2, 4); // width delta: 4
    bits.huffman(TABLE_B2, 4); // width delta: 8
    bits.code(TABLE_B2_OOB); // end of the height class
    bits.huffman(TABLE_B1, i64::try_from(mmr.len()).expect("small"));
    bits.align();
    for &byte in &mmr {
        bits.uint(u64::from(byte), 8);
    }
    export_runs(&mut bits, 2, 0..2);
    let mut dictionary = symbol_dictionary_header(SD_HUFFMAN, 2, 2);
    dictionary.extend(bits.into_bytes());
    stream(
        (64, 64),
        vec![
            (kind::SYMBOL_DICTIONARY, vec![], dictionary),
            (
                kind::IMMEDIATE_TEXT_REGION,
                vec![1],
                text_region(64, 64, 2, 1, 3, 5),
            ),
        ],
    )
}

/// `count` intermediate generic regions of `side × side` (never drawn), on a
/// 64 × 64 page.
pub fn intermediate_regions(count: u32, side: u32) -> Vec<u8> {
    stream(
        (64, 64),
        (0..count)
            .map(|_| {
                (
                    kind::INTERMEDIATE_GENERIC_REGION,
                    vec![],
                    white_mmr_region(side),
                )
            })
            .collect(),
    )
}

/// A 64 × 64 page with an immediate generic region whose data is too short
/// to hold the region's own header: it parses (segment headers only) and
/// fails to decode.
pub fn truncated_region() -> Vec<u8> {
    stream(
        (64, 64),
        vec![(kind::IMMEDIATE_GENERIC_REGION, vec![], vec![0; 3])],
    )
}

/// A blank `width × height` page: nothing but the page bitmap.
pub fn blank_page(width: u32, height: u32) -> Vec<u8> {
    stream((width, height), vec![])
}

/// A halftone region whose gray-scale grid is `grid × grid` cells, over a
/// 64 × 64 region of a 64 × 64 page, with a two-pattern 1 × 1 dictionary.
/// The gray-scale image is four bytes per cell (6.4 GB for a 40,000 grid)
/// before its two bit planes; the data is a few arbitrary bytes, which a
/// budgeted decode never reaches.
pub fn halftone_grid(grid: u32) -> Vec<u8> {
    // Pattern dictionary (7.4.4): arithmetic template 0 (whatever the bytes
    // decode to), 1 × 1 patterns, GRAYMAX 1.
    let mut patterns = vec![0x00, 1, 1];
    patterns.extend_from_slice(&1_u32.to_be_bytes());
    patterns.extend_from_slice(&[0; 4]);
    // Halftone region (7.4.5): MMR, no skip, OR; the grid at the origin with
    // unit vectors.
    let mut halftone = region_info(64, 64, 0, 0);
    halftone.push(0x01);
    halftone.extend_from_slice(&grid.to_be_bytes());
    halftone.extend_from_slice(&grid.to_be_bytes());
    halftone.extend_from_slice(&0_i32.to_be_bytes());
    halftone.extend_from_slice(&0_i32.to_be_bytes());
    halftone.extend_from_slice(&256_u16.to_be_bytes());
    halftone.extend_from_slice(&0_u16.to_be_bytes());
    halftone.extend_from_slice(&[0xFF; 8]);
    stream(
        (64, 64),
        vec![
            (kind::PATTERN_DICTIONARY, vec![], patterns),
            (kind::IMMEDIATE_HALFTONE_REGION, vec![1], halftone),
        ],
    )
}

/// A one-page PDF (200 × 200 pt) whose content draws `jbig2` as a
/// `width × height` `/JBIG2Decode` image XObject over the whole page.
pub fn image_pdf(jbig2: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut objs = Objects::new();
    let image = objs.add(stream_body(
        &format!(
            "/Type /XObject /Subtype /Image /Width {width} /Height {height} \
             /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode"
        ),
        jbig2,
    ));
    let content = objs.add(stream_body("", b"q 200 0 0 200 0 0 cm /Im1 Do Q"));
    let catalog = objs.alloc();
    let root = objs.alloc();
    let page = objs.alloc();
    objs.set(
        page,
        format!(
            "<< /Type /Page /Parent {root} 0 R /MediaBox [0 0 200 200] \
             /Resources << /XObject << /Im1 {image} 0 R >> >> /Contents {content} 0 R >>"
        ),
    );
    objs.set(
        root,
        format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"),
    );
    objs.set(catalog, format!("<< /Type /Catalog /Pages {root} 0 R >>"));
    objs.to_bytes(catalog, "")
}

/// The hostile fixture of run 0003 §4.4: a 64 × 64 JBIG2 image XObject
/// whose 8192 × 8192 symbol is exported 65,535 times. Under any memory
/// budget the image is skipped with an `image: memory limit` warning and the
/// page renders blank; unbudgeted, decoding it would copy 512 GiB.
pub fn symbol_export_bomb_pdf() -> Vec<u8> {
    image_pdf(&symbol_export(8192, 65_535, 64, false), 64, 64)
}
