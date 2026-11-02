//! The helper protocol's frames (spec 0002 §3.5), compiled into both the
//! library (parent side) and the helper binary (child side).
//!
//! Every message is a 12-byte header — `b"PXO1"`, a kind byte, three zero
//! bytes, the payload length as `u32` little-endian — followed by the
//! payload. Payloads are fixed-layout little-endian fields and
//! length-prefixed UTF-8 strings; no serialisation library is involved, so
//! the whole format is this file.
//!
//! Decoding treats its input as hostile in both directions (§2.2): a
//! declared length over [`MAX_FRAME`] is refused before anything is
//! allocated, the payload is read incrementally (a lying length costs only
//! the bytes actually sent), every block/line/word count is capped at
//! [`MAX_COUNT`], strings must be UTF-8, enum bytes must be in range, and a
//! payload must be consumed exactly. Each refusal is a [`WireError`]; the
//! parent turns it into a page error and kills the helper, the helper exits
//! with status 2.

use std::fmt;
use std::io::{self, Read, Write};

use crate::backend::{OcrBlock, OcrLine, OcrOutput, OcrWord};
use crate::config::{EngineMode, PageSegMode};

/// The first four bytes of every frame.
pub const MAGIC: [u8; 4] = *b"PXO1";
/// Bytes before the payload.
pub const HEADER_LEN: usize = 12;
/// The largest payload either side accepts (64 MiB).
pub const MAX_FRAME: u32 = 64 * 1024 * 1024;
/// The largest block, line, word or model count in one payload (2²⁰).
pub const MAX_COUNT: u32 = 1 << 20;
/// The protocol version a `Hello` carries.
pub const PROTO_VERSION: u32 = 1;
/// Bytes of a `Recognize` payload before the pixels.
pub const RECOGNIZE_FIXED: usize = 17;
/// The largest raster, in bytes, one `Recognize` frame carries. The raster
/// is 8-bit gray, one byte per pixel, so this is also its pixel limit.
pub const MAX_RASTER_BYTES: usize = MAX_FRAME as usize - RECOGNIZE_FIXED;

/// Frame kinds (the header's fifth byte).
pub mod kind {
    /// Parent → child: start up.
    pub const HELLO: u8 = 0x01;
    /// Parent → child: recognise one raster.
    pub const RECOGNIZE: u8 = 0x02;
    /// Parent → child: exit cleanly.
    pub const SHUTDOWN: u8 = 0x03;
    /// Child → parent: started.
    pub const HELLO_OK: u8 = 0x81;
    /// Child → parent: could not start.
    pub const HELLO_ERR: u8 = 0x82;
    /// Child → parent: the recognised words.
    pub const RESULT: u8 = 0x83;
    /// Child → parent: the page failed.
    pub const PAGE_ERR: u8 = 0x84;
}

/// `HelloErr.reason` values, mirroring core's `UnavailableReason`.
pub mod reason {
    /// No model directory.
    pub const NO_TESSDATA: u8 = 0;
    /// A language's model is missing or unreadable.
    pub const LANGUAGE_DATA_MISSING: u8 = 1;
    /// Tesseract refused to start.
    pub const INIT_FAILED: u8 = 2;
    /// Sent only by the helper's protocol-version check: the library and its
    /// helper come from different builds. The client relies on that when it
    /// appends its own version to the detail (`client.rs`), so a new sender
    /// of this value must carry its own version or change that code.
    pub const ENGINE_NOT_BUILT: u8 = 3;
}

/// `PageErr.kind` values.
pub mod page_err {
    /// Recognition failed.
    pub const RECOGNITION: u8 = 0;
    /// The deadline passed inside the helper.
    pub const TIMEOUT: u8 = 1;
}

/// The parent's start-up request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// [`PROTO_VERSION`] of the sender.
    pub proto_version: u32,
    /// The engine mode.
    pub oem: EngineMode,
    /// The default page segmentation mode (used for the warm-up).
    pub psm: PageSegMode,
    /// Tesseract's `+`-joined language list.
    pub languages: String,
    /// The resolved model directory.
    pub tessdata: String,
    /// The null device Tesseract's `debug_file` is pointed at.
    pub debug_null: String,
}

/// The fixed part of a `Recognize` payload; the pixels follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecognizeHeader {
    /// Raster width in pixels.
    pub width: u32,
    /// Raster height in pixels.
    pub height: u32,
    /// The raster's resolution.
    pub dpi: u32,
    /// The page segmentation mode.
    pub psm: PageSegMode,
    /// Milliseconds left of the page budget when the request was written.
    pub deadline_ms: u32,
}

/// The child's start-up reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelloOk {
    /// Tesseract's version string.
    pub tesseract_version: String,
    /// The helper build's `pdf_extract_core::VERSION`; the parent refuses a
    /// helper from another build (spec §3.5, "version-locked").
    pub library_version: String,
    /// SHA-256 of each language's model file.
    pub model_sha256: Vec<(String, [u8; 32])>,
}

/// One protocol message.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// `0x01`.
    Hello(Hello),
    /// `0x02`, with `width × height` pixels.
    Recognize(RecognizeHeader, Vec<u8>),
    /// `0x03`.
    Shutdown,
    /// `0x81`.
    HelloOk(HelloOk),
    /// `0x82`: one of [`reason`] and a detail.
    HelloErr(u8, String),
    /// `0x83`.
    Result(OcrOutput),
    /// `0x84`: one of [`page_err`] and a detail.
    PageErr(u8, String),
}

/// Why a frame could not be read or decoded.
#[derive(Debug)]
pub enum WireError {
    /// The stream ended cleanly, before any byte of a header.
    Closed,
    /// The stream ended inside a frame.
    Truncated,
    /// The first four bytes were not [`MAGIC`].
    BadMagic,
    /// A reserved header byte was not zero.
    BadHeader,
    /// A declared payload length over [`MAX_FRAME`].
    TooLarge(u32),
    /// A kind byte this protocol does not define.
    UnknownKind(u8),
    /// A string that is not UTF-8.
    BadUtf8,
    /// A count over [`MAX_COUNT`].
    CountTooLarge(u32),
    /// A field whose value is out of range (named).
    BadValue(&'static str),
    /// Bytes left over after the payload's last field.
    Trailing,
    /// The transport failed (including a deadline, as `TimedOut`).
    Io(io::Error),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("pipe closed"),
            Self::Truncated => f.write_str("truncated frame"),
            Self::BadMagic => f.write_str("bad magic in frame header"),
            Self::BadHeader => f.write_str("reserved bytes set in frame header"),
            Self::TooLarge(n) => write!(f, "frame too large ({n} bytes)"),
            Self::UnknownKind(k) => write!(f, "unknown frame kind 0x{k:02x}"),
            Self::BadUtf8 => f.write_str("invalid UTF-8 in frame"),
            Self::CountTooLarge(n) => write!(f, "count {n} over the limit in frame"),
            Self::BadValue(what) => write!(f, "bad {what} in frame"),
            Self::Trailing => f.write_str("trailing bytes in frame"),
            Self::Io(e) => write!(f, "pipe error: {e}"),
        }
    }
}

impl std::error::Error for WireError {}

const PSM_ALL: [PageSegMode; 14] = [
    PageSegMode::OsdOnly,
    PageSegMode::AutoOsd,
    PageSegMode::AutoOnly,
    PageSegMode::Auto,
    PageSegMode::SingleColumn,
    PageSegMode::SingleBlockVertText,
    PageSegMode::SingleBlock,
    PageSegMode::SingleLine,
    PageSegMode::SingleWord,
    PageSegMode::CircleWord,
    PageSegMode::SingleChar,
    PageSegMode::SparseText,
    PageSegMode::SparseTextOsd,
    PageSegMode::RawLine,
];

const OEM_ALL: [EngineMode; 4] = [
    EngineMode::TesseractOnly,
    EngineMode::LstmOnly,
    EngineMode::TesseractLstmCombined,
    EngineMode::Default,
];

/// Tesseract's number for a mode (the wire value).
fn psm_byte(psm: PageSegMode) -> u8 {
    psm.to_sys() as u8
}

fn oem_byte(oem: EngineMode) -> u8 {
    oem.to_sys() as u8
}

fn psm_from(b: u8) -> Result<PageSegMode, WireError> {
    PSM_ALL
        .into_iter()
        .find(|p| psm_byte(*p) == b)
        .ok_or(WireError::BadValue("page segmentation mode"))
}

fn oem_from(b: u8) -> Result<EngineMode, WireError> {
    OEM_ALL
        .into_iter()
        .find(|o| oem_byte(*o) == b)
        .ok_or(WireError::BadValue("engine mode"))
}

/// Appends little-endian fields and strings to a payload.
#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// A length or count. Encoding is only ever asked for values the
    /// decoder would accept, which are far below `u32::MAX`; saturating
    /// keeps the encoder total, and the peer then refuses the frame.
    fn len(&mut self, n: usize) {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }
    fn str(&mut self, s: &str) {
        self.len(s.len());
        self.0.extend_from_slice(s.as_bytes());
    }
}

/// The header for a payload of `len` bytes.
fn header(kind: u8, len: usize) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..4].copy_from_slice(&MAGIC);
    h[4] = kind;
    h[8..].copy_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_le_bytes());
    h
}

/// The header and fixed fields of a `Recognize` frame carrying
/// `pixel_len` pixel bytes, which the caller writes next (so an 8.7 MB
/// raster is not copied into a frame buffer).
pub fn recognize_prefix(h: &RecognizeHeader, pixel_len: usize) -> Vec<u8> {
    let mut out = Out::default();
    out.0.extend_from_slice(&header(
        kind::RECOGNIZE,
        RECOGNIZE_FIXED.saturating_add(pixel_len),
    ));
    out.u32(h.width);
    out.u32(h.height);
    out.u32(h.dpi);
    out.u8(psm_byte(h.psm));
    out.u32(h.deadline_ms);
    out.0
}

/// Encodes a whole frame.
pub fn encode(msg: &Message) -> Vec<u8> {
    let mut p = Out::default();
    let k = match msg {
        Message::Hello(h) => {
            p.u32(h.proto_version);
            p.u8(oem_byte(h.oem));
            p.u8(psm_byte(h.psm));
            p.str(&h.languages);
            p.str(&h.tessdata);
            p.str(&h.debug_null);
            kind::HELLO
        }
        Message::Recognize(h, pixels) => {
            let mut frame = recognize_prefix(h, pixels.len());
            frame.extend_from_slice(pixels);
            return frame;
        }
        Message::Shutdown => kind::SHUTDOWN,
        Message::HelloOk(ok) => {
            p.str(&ok.tesseract_version);
            p.str(&ok.library_version);
            p.len(ok.model_sha256.len());
            for (lang, digest) in &ok.model_sha256 {
                p.str(lang);
                p.0.extend_from_slice(digest);
            }
            kind::HELLO_OK
        }
        Message::HelloErr(reason, detail) => {
            p.u8(*reason);
            p.str(detail);
            kind::HELLO_ERR
        }
        Message::Result(out) => {
            p.len(out.blocks.len());
            for block in &out.blocks {
                p.len(block.lines.len());
                for line in &block.lines {
                    p.len(line.words.len());
                    for word in &line.words {
                        for v in word.bbox_px {
                            p.u32(v);
                        }
                        p.u32(word.confidence.to_bits());
                        p.str(&word.text);
                    }
                }
            }
            kind::RESULT
        }
        Message::PageErr(k, detail) => {
            p.u8(*k);
            p.str(detail);
            kind::PAGE_ERR
        }
    };
    let mut frame = header(k, p.0.len()).to_vec();
    frame.extend_from_slice(&p.0);
    frame
}

/// Writes one frame.
///
/// # Errors
/// The writer's error.
pub fn write_frame(w: &mut impl Write, msg: &Message) -> io::Result<()> {
    w.write_all(&encode(msg))?;
    w.flush()
}

/// Reads up to `buf.len()` bytes; returns how many (fewer only at EOF).
fn fill(r: &mut impl Read, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(WireError::Io(e)),
        }
    }
    Ok(got)
}

/// Reads and decodes one frame.
///
/// # Errors
/// [`WireError::Closed`] at a clean end of stream; any other variant for a
/// malformed or incomplete frame or a transport failure.
pub fn read_frame(r: &mut impl Read) -> Result<Message, WireError> {
    let mut h = [0u8; HEADER_LEN];
    match fill(r, &mut h)? {
        0 => return Err(WireError::Closed),
        HEADER_LEN => {}
        _ => return Err(WireError::Truncated),
    }
    if h[..4] != MAGIC {
        return Err(WireError::BadMagic);
    }
    if h[5..8] != [0, 0, 0] {
        return Err(WireError::BadHeader);
    }
    let len = u32::from_le_bytes([h[8], h[9], h[10], h[11]]);
    if len > MAX_FRAME {
        return Err(WireError::TooLarge(len));
    }
    let k = h[4];
    if !matches!(
        k,
        kind::HELLO
            | kind::RECOGNIZE
            | kind::SHUTDOWN
            | kind::HELLO_OK
            | kind::HELLO_ERR
            | kind::RESULT
            | kind::PAGE_ERR
    ) {
        return Err(WireError::UnknownKind(k));
    }
    // Incremental: memory follows the bytes actually received, not the
    // declared length.
    let mut payload = Vec::new();
    let got = r
        .take(u64::from(len))
        .read_to_end(&mut payload)
        .map_err(WireError::Io)?;
    if got != len as usize {
        return Err(WireError::Truncated);
    }
    if k == kind::RECOGNIZE {
        // The pixels are the rest of the payload: split off, not copied
        // (a 300 dpi A4 raster is 8.7 MB).
        let h = recognize_header(&mut In { buf: &payload })?;
        let pixels = payload.split_off(RECOGNIZE_FIXED);
        return Ok(Message::Recognize(h, pixels));
    }
    decode(k, &payload)
}

/// The fixed fields of a `Recognize` payload.
fn recognize_header(i: &mut In<'_>) -> Result<RecognizeHeader, WireError> {
    Ok(RecognizeHeader {
        width: i.u32()?,
        height: i.u32()?,
        dpi: i.u32()?,
        psm: psm_from(i.u8()?)?,
        deadline_ms: i.u32()?,
    })
}

/// Reads fields from a payload.
struct In<'a> {
    buf: &'a [u8],
}

impl<'a> In<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if self.buf.len() < n {
            return Err(WireError::Truncated);
        }
        let (head, rest) = self.buf.split_at(n);
        self.buf = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.bytes(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn count(&mut self) -> Result<usize, WireError> {
        let n = self.u32()?;
        if n > MAX_COUNT {
            return Err(WireError::CountTooLarge(n));
        }
        Ok(n as usize)
    }
    fn str(&mut self) -> Result<String, WireError> {
        let n = self.u32()? as usize;
        let b = self.bytes(n)?;
        String::from_utf8(b.to_vec()).map_err(|_| WireError::BadUtf8)
    }
    /// A `Vec` for `n` items of at least `min` bytes each, sized by what
    /// the payload can still hold rather than by the (untrusted) count.
    fn vec<T>(&self, n: usize, min: usize) -> Vec<T> {
        Vec::with_capacity(n.min(self.buf.len() / min.max(1)))
    }
    fn end(&self) -> Result<(), WireError> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(WireError::Trailing)
        }
    }
}

/// Bytes of the smallest encoded word: box, confidence, empty text.
const MIN_WORD: usize = 4 * 4 + 4 + 4;

/// Decodes the payload of a frame of kind `k`.
///
/// # Errors
/// As [`read_frame`], except `Closed` and `Io`.
pub fn decode(k: u8, payload: &[u8]) -> Result<Message, WireError> {
    let mut i = In { buf: payload };
    let msg = match k {
        kind::HELLO => Message::Hello(Hello {
            proto_version: i.u32()?,
            oem: oem_from(i.u8()?)?,
            psm: psm_from(i.u8()?)?,
            languages: i.str()?,
            tessdata: i.str()?,
            debug_null: i.str()?,
        }),
        kind::RECOGNIZE => {
            let h = recognize_header(&mut i)?;
            let pixels = i.bytes(i.buf.len())?.to_vec();
            Message::Recognize(h, pixels)
        }
        kind::SHUTDOWN => Message::Shutdown,
        kind::HELLO_OK => {
            let tesseract_version = i.str()?;
            let library_version = i.str()?;
            let n = i.count()?;
            let mut model_sha256 = i.vec(n, 4 + 32);
            for _ in 0..n {
                let lang = i.str()?;
                let mut digest = [0u8; 32];
                digest.copy_from_slice(i.bytes(32)?);
                model_sha256.push((lang, digest));
            }
            Message::HelloOk(HelloOk {
                tesseract_version,
                library_version,
                model_sha256,
            })
        }
        kind::HELLO_ERR => {
            let r = i.u8()?;
            if r > reason::ENGINE_NOT_BUILT {
                return Err(WireError::BadValue("start-up failure reason"));
            }
            Message::HelloErr(r, i.str()?)
        }
        kind::RESULT => {
            let nb = i.count()?;
            let mut blocks = i.vec(nb, 4);
            for _ in 0..nb {
                let nl = i.count()?;
                let mut lines = i.vec(nl, 4);
                for _ in 0..nl {
                    let nw = i.count()?;
                    let mut words = i.vec(nw, MIN_WORD);
                    for _ in 0..nw {
                        let bbox_px = [i.u32()?, i.u32()?, i.u32()?, i.u32()?];
                        let confidence = f32::from_bits(i.u32()?);
                        let text = i.str()?;
                        words.push(OcrWord {
                            text,
                            bbox_px,
                            confidence,
                        });
                    }
                    lines.push(OcrLine { words });
                }
                blocks.push(OcrBlock { lines });
            }
            Message::Result(OcrOutput { blocks })
        }
        kind::PAGE_ERR => {
            let k = i.u8()?;
            if k > page_err::TIMEOUT {
                return Err(WireError::BadValue("page error kind"));
            }
            Message::PageErr(k, i.str()?)
        }
        other => return Err(WireError::UnknownKind(other)),
    };
    i.end()?;
    Ok(msg)
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
