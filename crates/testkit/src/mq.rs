//! The MQ arithmetic encoder of ITU-T T.88 Annex E (also JPEG 2000's) and
//! the context models JBIG2 drives it with, for the arithmetic-coded streams
//! of `jbig2.rs`.
//!
//! Only what the test streams need is here: the integer encoder of Annex A.2
//! (IADH, IADW, IAEX), generic-region template 0 with its nominal adaptive
//! pixels and no typical prediction (6.2.5, as a symbol dictionary codes its
//! bitmaps, 6.5.8.1), and refinement template 1 without typical prediction
//! (6.3.5, as a refinement/aggregate symbol codes its one refinement). The
//! context numbering follows the bit order of the T.88 templates (6.2.5.3,
//! 6.3.5.3), the order hayro-jbig2's context gatherers build, so a stream
//! written here decodes there to the pixels it was written from; the tests
//! check that round trip (`h14_jbig2_budget.rs`) rather than trusting this
//! transcription.

/// One line of the Qe table: the LPS probability estimate `Qe` and the next
/// state after an MPS or an LPS, and whether an LPS swaps the MPS sense.
struct Qe {
    qe: u32,
    nmps: u8,
    nlps: u8,
    switch: bool,
}

const fn qe(qe: u32, nmps: u8, nlps: u8, switch: bool) -> Qe {
    Qe {
        qe,
        nmps,
        nlps,
        switch,
    }
}

/// The probability estimation table of T.88 Annex E, the same table (in
/// the same order) as hayro-jbig2's decoder uses.
#[rustfmt::skip]
const QE_TABLE: [Qe; 47] = [
    qe(0x5601, 1, 1, true), qe(0x3401, 2, 6, false), qe(0x1801, 3, 9, false),
    qe(0x0AC1, 4, 12, false), qe(0x0521, 5, 29, false), qe(0x0221, 38, 33, false),
    qe(0x5601, 7, 6, true), qe(0x5401, 8, 14, false), qe(0x4801, 9, 14, false),
    qe(0x3801, 10, 14, false), qe(0x3001, 11, 17, false), qe(0x2401, 12, 18, false),
    qe(0x1C01, 13, 20, false), qe(0x1601, 29, 21, false), qe(0x5601, 15, 14, true),
    qe(0x5401, 16, 14, false), qe(0x5101, 17, 15, false), qe(0x4801, 18, 16, false),
    qe(0x3801, 19, 17, false), qe(0x3401, 20, 18, false), qe(0x3001, 21, 19, false),
    qe(0x2801, 22, 19, false), qe(0x2401, 23, 20, false), qe(0x2201, 24, 21, false),
    qe(0x1C01, 25, 22, false), qe(0x1801, 26, 23, false), qe(0x1601, 27, 24, false),
    qe(0x1401, 28, 25, false), qe(0x1201, 29, 26, false), qe(0x1101, 30, 27, false),
    qe(0x0AC1, 31, 28, false), qe(0x09C1, 32, 29, false), qe(0x08A1, 33, 30, false),
    qe(0x0521, 34, 31, false), qe(0x0441, 35, 32, false), qe(0x02A1, 36, 33, false),
    qe(0x0221, 37, 34, false), qe(0x0141, 38, 35, false), qe(0x0111, 39, 36, false),
    qe(0x0085, 40, 37, false), qe(0x0049, 41, 38, false), qe(0x0025, 42, 39, false),
    qe(0x0015, 43, 40, false), qe(0x0009, 44, 41, false), qe(0x0005, 45, 42, false),
    qe(0x0001, 45, 43, false), qe(0x5601, 46, 46, false),
];

/// A context's adaptive state: its row of the Qe table and its MPS.
#[derive(Clone, Copy, Default)]
pub(crate) struct Context {
    index: u8,
    mps: u8,
}

/// The encoder registers (`C`, `A`, `CT`) and the output. `out[0]` is
/// the byte before the stream that BYTEOUT's carry may reach (`BPST - 1` in
/// Annex E's terms); it is dropped when the stream is finished.
pub(crate) struct Encoder {
    c: u32,
    a: u32,
    ct: u32,
    out: Vec<u8>,
}

impl Encoder {
    /// INITENC.
    pub(crate) fn new() -> Self {
        Self {
            c: 0,
            a: 0x8000,
            ct: 12,
            out: vec![0],
        }
    }

    /// ENCODE: codes `bit` in context `cx`.
    pub(crate) fn encode(&mut self, cx: &mut Context, bit: u8) {
        let q = &QE_TABLE[usize::from(cx.index)];
        self.a -= q.qe;
        if bit == cx.mps {
            // CODEMPS.
            if self.a & 0x8000 == 0 {
                if self.a < q.qe {
                    self.a = q.qe;
                } else {
                    self.c += q.qe;
                }
                cx.index = q.nmps;
                self.renormalize();
            } else {
                self.c += q.qe;
            }
        } else {
            // CODELPS, with conditional exchange.
            if self.a < q.qe {
                self.c += q.qe;
            } else {
                self.a = q.qe;
            }
            if q.switch {
                cx.mps = 1 - cx.mps;
            }
            cx.index = q.nlps;
            self.renormalize();
        }
    }

    /// RENORME.
    fn renormalize(&mut self) {
        loop {
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.ct == 0 {
                self.byte_out();
            }
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    /// BYTEOUT with bit stuffing after 0xFF: a carry out of
    /// `C` is added to the last byte written, and a byte that becomes or
    /// follows 0xFF takes only seven bits so no marker can appear.
    fn byte_out(&mut self) {
        let last = self.out.len() - 1;
        if self.out[last] == 0xFF {
            self.emit_seven();
        } else if self.c < 0x0800_0000 {
            self.emit_eight();
        } else {
            self.out[last] += 1;
            if self.out[last] == 0xFF {
                self.c &= 0x07FF_FFFF;
                self.emit_seven();
            } else {
                self.emit_eight();
            }
        }
    }

    fn emit_seven(&mut self) {
        // Bits 20-27: seven data bits under the carry bit, the room a byte
        // after 0xFF leaves for a carry (the decoder's BYTEIN adds it back).
        self.out.push(u8::try_from(self.c >> 20).expect("one byte"));
        self.c &= 0x000F_FFFF;
        self.ct = 7;
    }

    fn emit_eight(&mut self) {
        // A carry (bit 27) has already gone into the previous byte.
        self.out
            .push(u8::try_from(self.c >> 19 & 0xFF).expect("masked"));
        self.c &= 0x0007_FFFF;
        self.ct = 8;
    }

    /// FLUSH (with SETBITS), then the 0xFF 0xAC terminating marker.
    /// Returns the coded bytes.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        let tempc = self.c + self.a;
        self.c |= 0xFFFF;
        if self.c >= tempc {
            self.c -= 0x8000;
        }
        self.c <<= self.ct;
        self.byte_out();
        self.c <<= self.ct;
        self.byte_out();
        if self.out.last() != Some(&0xFF) {
            self.out.push(0xFF);
        }
        self.out.push(0xAC);
        assert_eq!(self.out[0], 0, "a carry reached the byte before the stream");
        self.out.remove(0);
        self.out
    }
}

/// The integer encoder of A.2 (Figure A.1 inverted) for one of the `IAx`
/// procedures: 512 contexts, indexed by `PREV`.
pub(crate) struct IntegerEncoder {
    contexts: Vec<Context>,
}

impl IntegerEncoder {
    pub(crate) fn new() -> Self {
        Self {
            contexts: vec![Context::default(); 512],
        }
    }

    /// Codes `value`, or OOB for `None`.
    pub(crate) fn encode(&mut self, enc: &mut Encoder, value: Option<i32>) {
        // Figure A.1: each range's prefix bits after the sign, the number of
        // value bits and the range's low end.
        const RANGES: [(&[u8], u32, u32); 6] = [
            (&[0], 2, 0),
            (&[1, 0], 4, 4),
            (&[1, 1, 0], 6, 20),
            (&[1, 1, 1, 0], 8, 84),
            (&[1, 1, 1, 1, 0], 12, 340),
            (&[1, 1, 1, 1, 1], 32, 4436),
        ];
        // OOB is a negative zero.
        let (sign, magnitude) = match value {
            None => (1, 0),
            Some(v) => (u8::from(v < 0), v.unsigned_abs()),
        };
        let &(prefix, bits, low) = RANGES
            .iter()
            .rev()
            .find(|&&(_, _, low)| magnitude >= low)
            .expect("the first range starts at 0");
        let mut prev: u32 = 1;
        self.bit(enc, &mut prev, sign);
        for &b in prefix {
            self.bit(enc, &mut prev, b);
        }
        let offset = magnitude - low;
        for i in (0..bits).rev() {
            self.bit(enc, &mut prev, u8::from(offset >> i & 1 == 1));
        }
    }

    /// One bit in context `PREV` (low nine bits), then the A.2 step 3
    /// update: `PREV` keeps the last eight bits and a leading 1 once it has
    /// more than eight.
    fn bit(&mut self, enc: &mut Encoder, prev: &mut u32, bit: u8) {
        enc.encode(&mut self.contexts[(*prev & 0x1FF) as usize], bit);
        let shifted = (*prev << 1) | u32::from(bit);
        *prev = if *prev < 256 {
            shifted
        } else {
            (shifted & 511) | 256
        };
    }
}

/// A bilevel bitmap as rows of `#` (black) and `.` (white), each `width`
/// long; pixels outside it read as white, as the templates require.
pub(crate) struct Bitmap<'a> {
    pub(crate) width: u32,
    pub(crate) height: u32,
    rows: &'a [&'a str],
}

impl<'a> Bitmap<'a> {
    /// The bitmap of `rows`, which must all be the same length.
    pub(crate) fn new(rows: &'a [&'a str]) -> Self {
        let width = rows.first().map_or(0, |r| r.len());
        assert!(rows.iter().all(|r| r.len() == width), "ragged bitmap");
        Self {
            width: u32::try_from(width).expect("small"),
            height: u32::try_from(rows.len()).expect("small"),
            rows,
        }
    }

    /// The pixel at (`x`, `y`): 1 for a `#` in `rows`, 0 otherwise.
    pub(crate) fn pixel(&self, x: i64, y: i64) -> u8 {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return 0;
        };
        if x >= self.width || y >= self.height {
            return 0;
        }
        u8::from(self.rows[y as usize].as_bytes()[x as usize] == b'#')
    }
}

/// Generic-region template 0 with the nominal adaptive pixels A1 (3, -1),
/// A2 (-3, -1), A3 (2, -2), A4 (-2, -2), no TPGDON (6.2.5.3):
/// codes every pixel of `bitmap` in `contexts` (65,536 of them).
pub(crate) fn encode_generic_template0(
    enc: &mut Encoder,
    contexts: &mut [Context],
    bitmap: &Bitmap<'_>,
) {
    // (dx, dy) of context bits 15 down to 0.
    const TEMPLATE: [(i64, i64); 16] = [
        (-2, -2), // A4
        (-1, -2),
        (0, -2),
        (1, -2),
        (2, -2),  // A3
        (-3, -1), // A2
        (-2, -1),
        (-1, -1),
        (0, -1),
        (1, -1),
        (2, -1),
        (3, -1), // A1
        (-4, 0),
        (-3, 0),
        (-2, 0),
        (-1, 0),
    ];
    for y in 0..i64::from(bitmap.height) {
        for x in 0..i64::from(bitmap.width) {
            let cx = TEMPLATE.iter().fold(0_usize, |cx, &(dx, dy)| {
                cx << 1 | usize::from(bitmap.pixel(x + dx, y + dy))
            });
            enc.encode(&mut contexts[cx], bitmap.pixel(x, y));
        }
    }
}

/// Refinement template 1, no TPGRON (6.3.5.3): codes every pixel
/// of `bitmap` against `reference`, whose (0, 0) lies at (`dx`, `dy`) in
/// `bitmap` (GRREFERENCEDX/DY), in `contexts` (1,024 of them).
pub(crate) fn encode_refinement_template1(
    enc: &mut Encoder,
    contexts: &mut [Context],
    bitmap: &Bitmap<'_>,
    reference: &Bitmap<'_>,
    (dx, dy): (i64, i64),
) {
    // Context bits 9 down to 6 come from the bitmap being coded, 5 down to 0
    // from the reference, relative to the pixel's reference position.
    const CODED: [(i64, i64); 4] = [(-1, -1), (0, -1), (1, -1), (-1, 0)];
    const REFERENCE: [(i64, i64); 6] = [(0, -1), (-1, 0), (0, 0), (1, 0), (0, 1), (1, 1)];
    for y in 0..i64::from(bitmap.height) {
        for x in 0..i64::from(bitmap.width) {
            let (rx, ry) = (x - dx, y - dy);
            let cx = CODED
                .iter()
                .map(|&(cx, cy)| bitmap.pixel(x + cx, y + cy))
                .chain(
                    REFERENCE
                        .iter()
                        .map(|&(cx, cy)| reference.pixel(rx + cx, ry + cy)),
                )
                .fold(0_usize, |cx, p| cx << 1 | usize::from(p));
            enc.encode(&mut contexts[cx], bitmap.pixel(x, y));
        }
    }
}
