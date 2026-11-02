//! Unit tests of the helper's loop, driven in memory: frames in, frames
//! and an exit status out. The protocol checks need no Tesseract; the
//! start-up and recognition paths use the test model, found as the
//! integration tests find it.

use std::path::PathBuf;

use super::*;
use crate::config::{EngineMode, PageSegMode};
use crate::tessdata::Candidates;

fn tessdata() -> PathBuf {
    Candidates::from_env()
        .ordered()
        .into_iter()
        .find(|d| d.join("eng.traineddata").is_file())
        .expect("no eng.traineddata: set TESSDATA_PREFIX")
}

fn hello_in(dir: &Path, languages: &str) -> wire::Hello {
    wire::Hello {
        proto_version: wire::PROTO_VERSION,
        oem: EngineMode::LstmOnly,
        psm: PageSegMode::Auto,
        languages: languages.into(),
        tessdata: dir.to_str().expect("UTF-8 test path").into(),
        debug_null: "/dev/null".into(),
    }
}

fn hello() -> Message {
    Message::Hello(hello_in(&tessdata(), "eng"))
}

fn recognize(width: u32, height: u32, pixels: Vec<u8>, deadline_ms: u32) -> Message {
    Message::Recognize(
        wire::RecognizeHeader {
            width,
            height,
            dpi: 300,
            psm: PageSegMode::Auto,
            deadline_ms,
        },
        pixels,
    )
}

fn white(n: u32) -> Message {
    recognize(n, n, vec![255; (n * n) as usize], 60_000)
}

/// Runs the loop over `input` frames (plus `tail` raw bytes); returns the
/// exit status and the frames written.
fn run(input: &[Message], tail: &[u8], fault: Option<Fault>) -> (u8, Vec<Message>) {
    let mut bytes: Vec<u8> = input.iter().flat_map(wire::encode).collect();
    bytes.extend_from_slice(tail);
    let mut out = Vec::new();
    let status = serve(&mut bytes.as_slice(), &mut out, fault, Some(vec![0, 1, 2]));
    let mut replies = Vec::new();
    let mut r = out.as_slice();
    while !r.is_empty() {
        replies.push(wire::read_frame(&mut r).expect("helper wrote a valid frame"));
    }
    (status, replies)
}

#[test]
fn faults_parse_only_known_values() {
    assert_eq!(Fault::parse("abort-on-recognize"), Some(Fault::Abort));
    assert_eq!(Fault::parse("exit-on-recognize"), Some(Fault::Exit));
    assert_eq!(Fault::parse("hang-on-recognize"), Some(Fault::Hang));
    assert_eq!(Fault::parse("garbage-on-recognize"), Some(Fault::Garbage));
    assert_eq!(Fault::parse("report-fds"), Some(Fault::ReportFds));
    assert_eq!(Fault::parse("report-env"), Some(Fault::ReportEnv));
    assert_eq!(
        Fault::parse("slow-on-recognize:250"),
        Some(Fault::Slow(Duration::from_millis(250)))
    );
    assert_eq!(Fault::parse("slow-on-recognize:x"), None);
    assert_eq!(Fault::parse("abort"), None);
    // This test process has neither variable set.
    assert_eq!(Fault::from_env(), None);
}

#[test]
fn the_open_descriptors_include_the_standard_three() {
    let fds = open_fds();
    assert!(fds.starts_with(&[0, 1, 2]), "{fds:?}");
    assert_eq!(join([1, 2]), "1,2");
}

#[test]
fn protocol_violations_before_hello() {
    assert_eq!(run(&[], b"", None), (0, vec![]));
    assert_eq!(run(&[Message::Shutdown], b"", None), (0, vec![]));
    assert_eq!(run(&[], b"nonsense", None).0, EXIT_PROTOCOL);
    assert_eq!(run(&[white(1)], b"", None).0, EXIT_PROTOCOL);
    let Message::Hello(mut h) = hello() else {
        unreachable!()
    };
    h.proto_version = 2;
    let (status, replies) = run(&[Message::Hello(h)], b"", None);
    assert_eq!(status, EXIT_PROTOCOL);
    assert!(
        matches!(&replies[..], [Message::HelloErr(r, d)] if *r == reason::ENGINE_NOT_BUILT && d.contains("protocol version 2")),
        "{replies:?}"
    );
}

#[test]
fn start_up_failures_are_reported_then_exit_1() {
    let dir = tessdata();
    for (langs, want) in [
        ("a/b", reason::INIT_FAILED),
        // Empty members (spec 0003 F19), refused before any file is read.
        ("+", reason::INIT_FAILED),
        ("eng+", reason::INIT_FAILED),
        ("+eng", reason::INIT_FAILED),
        ("eng++fra", reason::INIT_FAILED),
        ("zzz_none", reason::LANGUAGE_DATA_MISSING),
    ] {
        let (status, replies) = run(&[Message::Hello(hello_in(&dir, langs))], b"", None);
        assert_eq!(status, EXIT_INIT, "{langs}");
        assert!(
            matches!(&replies[..], [Message::HelloErr(r, _)] if *r == want),
            "{langs}: {replies:?}"
        );
    }
    let empty = std::env::temp_dir().join(format!("pxo-helper-{}-empty", std::process::id()));
    std::fs::create_dir_all(&empty).expect("mkdir");
    std::fs::write(empty.join("eng.traineddata"), b"").expect("write");
    let (status, replies) = run(&[Message::Hello(hello_in(&empty, "eng"))], b"", None);
    assert_eq!(status, EXIT_INIT);
    assert!(
        matches!(&replies[..], [Message::HelloErr(r, _)] if *r == reason::INIT_FAILED),
        "{replies:?}"
    );
}

#[test]
fn a_session_answers_hello_recognises_and_shuts_down() {
    let (status, replies) = run(
        &[
            hello(),
            white(8),
            recognize(8, 8, vec![255; 64], 0),
            Message::Shutdown,
        ],
        b"",
        None,
    );
    assert_eq!(status, 0);
    match &replies[..] {
        [
            Message::HelloOk(ok),
            Message::Result(out),
            Message::PageErr(k, _),
        ] => {
            assert_eq!(ok.library_version, pdf_extract_core::VERSION);
            assert_eq!(ok.tesseract_version, sys::version());
            assert_eq!(ok.model_sha256.len(), 1);
            assert!(out.blocks.is_empty(), "a white raster has no text");
            assert_eq!(*k, page_err::TIMEOUT, "a spent deadline");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn malformed_requests_end_the_helper_with_status_2() {
    for bad in [
        recognize(0, 1, vec![], 1000),
        recognize(2, 2, vec![0; 3], 1000),
        Message::HelloOk(HelloOk {
            tesseract_version: String::new(),
            library_version: String::new(),
            model_sha256: Vec::new(),
        }),
    ] {
        let (status, replies) = run(&[hello(), bad.clone()], b"", None);
        assert_eq!(status, EXIT_PROTOCOL, "{bad:?}");
        assert_eq!(replies.len(), 1, "only HelloOk");
    }
    let (status, _) = run(&[hello()], b"PXO1junk", None);
    assert_eq!(status, EXIT_PROTOCOL);
    // End of input after Hello: the parent has gone.
    assert_eq!(run(&[hello()], b"", None).0, 0);
}

#[test]
fn in_process_faults_answer_the_first_request_only() {
    let (_, replies) = run(&[hello(), white(1), white(1)], b"", Some(Fault::ReportFds));
    assert!(
        matches!(&replies[1], Message::PageErr(_, d) if d == "fds=0,1,2"),
        "{replies:?}"
    );
    assert!(matches!(&replies[2], Message::Result(_)), "{replies:?}");
    let (_, replies) = run(&[hello(), white(1)], b"", Some(Fault::ReportEnv));
    assert!(
        matches!(&replies[1], Message::PageErr(_, d) if d.starts_with("env=")),
        "{replies:?}"
    );
    let (_, replies) = run(
        &[hello(), white(1)],
        b"",
        Some(Fault::Slow(Duration::from_millis(1))),
    );
    assert!(matches!(&replies[1], Message::Result(_)), "{replies:?}");
}

#[test]
fn the_garbage_fault_writes_an_oversized_frame_header() {
    let input: Vec<u8> = [hello(), white(1)].iter().flat_map(wire::encode).collect();
    let mut out = Vec::new();
    let status = serve(&mut input.as_slice(), &mut out, Some(Fault::Garbage), None);
    assert_eq!(status, 0);
    let mut r = out.as_slice();
    assert!(matches!(wire::read_frame(&mut r), Ok(Message::HelloOk(_))));
    assert!(matches!(
        wire::read_frame(&mut r),
        Err(wire::WireError::TooLarge(n)) if n == wire::MAX_FRAME + 1
    ));
}

/// A writer that accepts `budget` bytes, then fails like a closed pipe.
struct Closing(usize);

impl Write for Closing {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.0 == 0 {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let n = buf.len().min(self.0);
        self.0 -= n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_parent_that_goes_away_ends_the_helper_quietly() {
    let input: Vec<u8> = [hello(), white(1)].iter().flat_map(wire::encode).collect();
    // Cannot even answer Hello.
    assert_eq!(serve(&mut input.as_slice(), &mut Closing(0), None, None), 0);
    // Answers Hello, cannot answer the page.
    let hello_ok_len = wire::encode(&Message::HelloOk(HelloOk {
        tesseract_version: sys::version(),
        library_version: pdf_extract_core::VERSION.into(),
        model_sha256: vec![("eng".into(), [0; 32])],
    }))
    .len();
    assert_eq!(
        serve(
            &mut input.as_slice(),
            &mut Closing(hello_ok_len),
            None,
            None
        ),
        0
    );
}
