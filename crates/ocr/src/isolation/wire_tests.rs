//! Tests of the frame format (spec 0002 §4.9, first item): every message
//! round-trips through encode → decode, including randomly generated
//! results, and each malformation decodes to its documented error.

use super::*;

/// splitmix64: a seeded, dependency-free generator, so a failing case can
/// be replayed from the printed seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn u32(&mut self) -> u32 {
        u32::try_from(self.next() >> 32).expect("32 bits")
    }
    /// Text mixing ASCII, multi-byte code points and the empty string.
    fn text(&mut self) -> String {
        const POOL: [char; 8] = ['a', 'Z', '7', ' ', 'é', 'ß', '中', '😀'];
        let n = self.below(12);
        (0..n)
            .map(|_| POOL[usize::try_from(self.below(8)).expect("small")])
            .collect()
    }
}

fn random_output(rng: &mut Rng) -> OcrOutput {
    let blocks = (0..rng.below(4))
        .map(|_| OcrBlock {
            lines: (0..rng.below(4))
                .map(|_| OcrLine {
                    words: (0..rng.below(6))
                        .map(|_| OcrWord {
                            text: rng.text(),
                            bbox_px: [rng.u32(), rng.u32(), rng.u32(), rng.u32()],
                            #[allow(clippy::cast_precision_loss)]
                            confidence: rng.below(10_001) as f32 / 100.0,
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    OcrOutput { blocks }
}

fn roundtrip(msg: &Message) -> Message {
    let bytes = encode(msg);
    let back = read_frame(&mut bytes.as_slice()).expect("decodes");
    assert_eq!(&back, msg, "round trip of {msg:?}");
    back
}

fn hello() -> Hello {
    Hello {
        proto_version: PROTO_VERSION,
        oem: EngineMode::LstmOnly,
        psm: PageSegMode::Auto,
        languages: "eng+deu".into(),
        tessdata: "/opt/local/share/tessdata".into(),
        debug_null: "/dev/null".into(),
    }
}

#[test]
fn random_results_round_trip() {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| u64::from(d.subsec_nanos()));
    println!("wire property seed: {seed}");
    let mut rng = Rng(seed);
    for _ in 0..500 {
        let out = random_output(&mut rng);
        roundtrip(&Message::Result(out));
        let (w, h) = (1 + rng.below(16), 1 + rng.below(16));
        let pixels: Vec<u8> = (0..w * h).map(|_| rng.next() as u8).collect();
        roundtrip(&Message::Recognize(
            RecognizeHeader {
                width: u32::try_from(w).expect("small"),
                height: u32::try_from(h).expect("small"),
                dpi: rng.u32(),
                psm: PSM_ALL[usize::try_from(rng.below(14)).expect("small")],
                deadline_ms: rng.u32(),
            },
            pixels,
        ));
        roundtrip(&Message::PageErr(
            u8::try_from(rng.below(2)).expect("small"),
            rng.text(),
        ));
    }
}

#[test]
fn every_fixed_message_round_trips() {
    for oem in OEM_ALL {
        for psm in PSM_ALL {
            roundtrip(&Message::Hello(Hello {
                oem,
                psm,
                ..hello()
            }));
        }
    }
    roundtrip(&Message::Shutdown);
    roundtrip(&Message::HelloOk(HelloOk {
        tesseract_version: "5.5.3".into(),
        library_version: "0.1.0".into(),
        model_sha256: vec![("eng".into(), [7; 32]), ("deu".into(), [0xff; 32])],
    }));
    for r in 0..=reason::ENGINE_NOT_BUILT {
        roundtrip(&Message::HelloErr(
            r,
            "Failed loading language 'eng'".into(),
        ));
    }
    roundtrip(&Message::Result(OcrOutput::default()));
}

#[test]
fn a_recognize_prefix_plus_pixels_is_the_encoded_frame() {
    let h = RecognizeHeader {
        width: 2,
        height: 1,
        dpi: 300,
        psm: PageSegMode::SingleLine,
        deadline_ms: 1234,
    };
    let mut frame = recognize_prefix(&h, 2);
    assert_eq!(frame.len(), HEADER_LEN + RECOGNIZE_FIXED);
    frame.extend_from_slice(&[0, 255]);
    assert_eq!(frame, encode(&Message::Recognize(h, vec![0, 255])));
}

fn decode_err(bytes: &[u8]) -> WireError {
    read_frame(&mut &bytes[..]).expect_err("malformed")
}

fn with_header(k: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = header(k, payload.len()).to_vec();
    f.extend_from_slice(payload);
    f
}

#[test]
fn bad_magic_is_refused() {
    let mut f = encode(&Message::Shutdown);
    f[0] = b'X';
    assert!(matches!(decode_err(&f), WireError::BadMagic));
    assert_eq!(WireError::BadMagic.to_string(), "bad magic in frame header");
}

#[test]
fn a_length_of_max_frame_plus_one_is_refused_before_reading() {
    let mut f = header(kind::RESULT, 0).to_vec();
    f[8..].copy_from_slice(&(MAX_FRAME + 1).to_le_bytes());
    let err = decode_err(&f);
    assert!(matches!(err, WireError::TooLarge(n) if n == MAX_FRAME + 1));
    assert!(err.to_string().starts_with("frame too large"));
    // Exactly MAX_FRAME is a size the reader accepts (and then finds short).
    f[8..].copy_from_slice(&MAX_FRAME.to_le_bytes());
    assert!(matches!(decode_err(&f), WireError::Truncated));
}

#[test]
fn truncation_anywhere_is_refused() {
    let full = encode(&Message::HelloOk(HelloOk {
        tesseract_version: "5.5.3".into(),
        library_version: "0.1.0".into(),
        model_sha256: vec![("eng".into(), [1; 32])],
    }));
    assert!(matches!(decode_err(&[]), WireError::Closed));
    assert_eq!(WireError::Closed.to_string(), "pipe closed");
    for cut in 1..full.len() {
        let err = decode_err(&full[..cut]);
        assert!(matches!(err, WireError::Truncated), "cut at {cut}: {err}");
    }
    // A header whose length overstates the fields that follow.
    let mut short = full.clone();
    let declared = u32::try_from(full.len() - HEADER_LEN + 4).expect("small");
    short[8..12].copy_from_slice(&declared.to_le_bytes());
    short.extend_from_slice(&[0; 4]);
    assert!(matches!(decode_err(&short), WireError::Trailing));
    // A Recognize payload shorter than its fixed fields.
    let f = with_header(kind::RECOGNIZE, &[1, 0, 0, 0, 1]);
    assert!(matches!(decode_err(&f), WireError::Truncated));
    assert!(matches!(
        decode(kind::RECOGNIZE, &[1]),
        Err(WireError::Truncated)
    ));
    // A payload whose own fields overrun it.
    let f = with_header(kind::PAGE_ERR, &[0, 9, 0, 0, 0, b'x']);
    assert!(matches!(decode_err(&f), WireError::Truncated));
}

#[test]
fn a_count_of_two_to_the_twenty_plus_one_is_refused() {
    for depth in 0..3 {
        let mut p = Vec::new();
        for _ in 0..depth {
            p.extend_from_slice(&1u32.to_le_bytes());
        }
        p.extend_from_slice(&(MAX_COUNT + 1).to_le_bytes());
        let err = decode_err(&with_header(kind::RESULT, &p));
        assert!(
            matches!(err, WireError::CountTooLarge(n) if n == MAX_COUNT + 1),
            "depth {depth}: {err}"
        );
        assert!(err.to_string().contains("frame"));
    }
    let mut p = Vec::new();
    for s in ["5", "0"] {
        p.extend_from_slice(&1u32.to_le_bytes());
        p.extend_from_slice(s.as_bytes());
    }
    p.extend_from_slice(&(MAX_COUNT + 1).to_le_bytes());
    assert!(matches!(
        decode_err(&with_header(kind::HELLO_OK, &p)),
        WireError::CountTooLarge(_)
    ));
    // At the cap the count is accepted, and the missing items are a
    // truncation (no allocation of 2^20 words up front).
    let p = MAX_COUNT.to_le_bytes();
    assert!(matches!(
        decode_err(&with_header(kind::RESULT, &p)),
        WireError::Truncated
    ));
}

#[test]
fn invalid_utf8_is_refused() {
    let p = [0u8, 2, 0, 0, 0, 0xc3, 0x28];
    let err = decode_err(&with_header(kind::PAGE_ERR, &p));
    assert!(matches!(err, WireError::BadUtf8));
    assert_eq!(err.to_string(), "invalid UTF-8 in frame");
}

#[test]
fn header_and_value_checks() {
    let mut f = encode(&Message::Shutdown);
    f[6] = 1;
    assert!(matches!(decode_err(&f), WireError::BadHeader));
    f = encode(&Message::Shutdown);
    f[4] = 0x7f;
    let err = decode_err(&f);
    assert!(matches!(err, WireError::UnknownKind(0x7f)));
    assert_eq!(err.to_string(), "unknown frame kind 0x7f");
    assert!(matches!(
        decode(0x7f, &[]),
        Err(WireError::UnknownKind(0x7f))
    ));
    // Out-of-range enum bytes.
    let mut hello_bytes = encode(&Message::Hello(hello()));
    hello_bytes[HEADER_LEN + 4] = 9;
    assert!(matches!(
        decode_err(&hello_bytes),
        WireError::BadValue("engine mode")
    ));
    hello_bytes = encode(&Message::Hello(hello()));
    hello_bytes[HEADER_LEN + 5] = 99;
    let err = decode_err(&hello_bytes);
    assert_eq!(err.to_string(), "bad page segmentation mode in frame");
    let err = decode_err(&with_header(kind::PAGE_ERR, &[2, 0, 0, 0, 0]));
    assert!(matches!(err, WireError::BadValue("page error kind")));
    let err = decode_err(&with_header(kind::HELLO_ERR, &[4, 0, 0, 0, 0]));
    assert!(matches!(err, WireError::BadValue(_)));
    // A Shutdown with a payload has trailing bytes.
    let err = decode_err(&with_header(kind::SHUTDOWN, &[0]));
    assert_eq!(err.to_string(), "trailing bytes in frame");
}

/// A reader that fails, to show transport errors are kept as such.
struct Failing(io::ErrorKind);

impl Read for Failing {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(self.0.into())
    }
}

/// A reader that is interrupted once, then yields its bytes.
struct Interrupted(bool, Vec<u8>);

impl Read for Interrupted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.0 {
            self.0 = true;
            return Err(io::ErrorKind::Interrupted.into());
        }
        let n = buf.len().min(self.1.len());
        buf[..n].copy_from_slice(&self.1[..n]);
        self.1.drain(..n);
        Ok(n)
    }
}

#[test]
fn transport_errors_and_interruptions() {
    let err = read_frame(&mut Failing(io::ErrorKind::TimedOut)).expect_err("fails");
    assert!(matches!(&err, WireError::Io(e) if e.kind() == io::ErrorKind::TimedOut));
    assert!(err.to_string().starts_with("pipe error"));
    let msg = read_frame(&mut Interrupted(false, encode(&Message::Shutdown))).expect("ok");
    assert_eq!(msg, Message::Shutdown);
    // An error while reading the payload.
    let mut chained = encode(&Message::PageErr(0, "x".into()));
    chained.truncate(HEADER_LEN);
    let mut r = chained.as_slice().chain(Failing(io::ErrorKind::BrokenPipe));
    assert!(matches!(read_frame(&mut r), Err(WireError::Io(_))));
    let mut sink = Vec::new();
    write_frame(&mut sink, &Message::Shutdown).expect("write");
    assert_eq!(sink, encode(&Message::Shutdown));
}
