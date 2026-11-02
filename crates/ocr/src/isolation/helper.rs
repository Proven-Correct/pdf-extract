//! The helper process (spec 0002 §1.5, §3.5): the whole of
//! `pdf-extract-ocr-helper`'s `main`.
//!
//! The helper reads frames from stdin and writes frames to stdout; its
//! stderr is the null device. It expects one `Hello`, sets Tesseract's
//! process-wide silence to the path the parent sent, creates one instance,
//! runs the same up-front validation as the in-process engine (checksums,
//! `Init`, a warm-up recognition; O7, O29) and answers `HelloOk` or
//! `HelloErr`. It then serves `Recognize` requests one at a time until
//! `Shutdown` (exit 0, after the instance is ended) or end of input (exit
//! 0: the parent has gone).
//!
//! The parent is trusted, but its frames are still checked (§2.2): a
//! malformed frame, an unexpected kind, or a raster whose size does not
//! match its payload ends the helper with status 2, which the parent
//! reports as a failed page. A protocol version mismatch is answered with
//! `HelloErr` before exiting 2, so the parent can say why.
//!
//! **Fault hooks (O31).** For tests only, `PDF_EXTRACT_OCR_HELPER_FAULT`
//! makes the first `Recognize` misbehave — and only when
//! `PDF_EXTRACT_OCR_HELPER_FAULT_ARMED=1` is set too, so a stray value in a
//! production environment does nothing. The library never reads these
//! variables; it only passes them through to the helpers it spawns.

use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use pdf_extract_core::OcrPageError;
use pdf_extract_render::GrayImage;
use pdf_extract_tesseract_sys as sys;

use super::wire::{self, HelloOk, Message, page_err, reason};
use crate::backend::{OcrRequest, Recognizer};
use crate::tessdata;
use crate::tesseract::{TesseractInstance, sha256_file, warm_up};

/// The fault to inject (O31).
pub const FAULT_ENV: &str = "PDF_EXTRACT_OCR_HELPER_FAULT";
/// Must be `1` for [`FAULT_ENV`] to be read.
pub const FAULT_ARMED_ENV: &str = "PDF_EXTRACT_OCR_HELPER_FAULT_ARMED";

/// Exit status for a malformed frame or protocol violation.
const EXIT_PROTOCOL: u8 = 2;
/// Exit status when the engine could not start (after `HelloErr`).
const EXIT_INIT: u8 = 1;
/// Exit status of the `exit-on-recognize` fault.
const EXIT_FAULT: i32 = 3;

/// Budget of the start-up warm-up recognition, which `Hello` does not
/// carry; the parent's own wait for `HelloOk` is not bounded by it.
pub(crate) const WARMUP_BUDGET: Duration = Duration::from_secs(60);

/// What the first `Recognize` does instead of (or before) recognising.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Fault {
    /// `abort-on-recognize`: `abort()`, as an `ASSERT_HOST` would.
    Abort,
    /// `exit-on-recognize`: exit with status 3.
    Exit,
    /// `hang-on-recognize`: never answer.
    Hang,
    /// `garbage-on-recognize`: send a frame with an invalid length.
    Garbage,
    /// `slow-on-recognize:<ms>`: sleep, then recognise normally.
    Slow(Duration),
    /// `report-fds`: answer with the descriptors open at start-up.
    ReportFds,
    /// `report-env`: answer with the names of the environment variables.
    ReportEnv,
}

impl Fault {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "abort-on-recognize" => Self::Abort,
            "exit-on-recognize" => Self::Exit,
            "hang-on-recognize" => Self::Hang,
            "garbage-on-recognize" => Self::Garbage,
            "report-fds" => Self::ReportFds,
            "report-env" => Self::ReportEnv,
            other => {
                let ms = other.strip_prefix("slow-on-recognize:")?.parse().ok()?;
                Self::Slow(Duration::from_millis(ms))
            }
        })
    }

    /// The armed fault, if any.
    fn from_env() -> Option<Self> {
        if std::env::var_os(FAULT_ARMED_ENV).is_some_and(|v| v == "1") {
            std::env::var(FAULT_ENV).ok().and_then(|v| Self::parse(&v))
        } else {
            None
        }
    }
}

/// The descriptors open in this process, ascending. Read through
/// `/dev/fd`, whose listing includes the directory handle itself; that
/// one is closed once the listing ends, so only numbers still open
/// afterwards are kept.
fn open_fds() -> Vec<u32> {
    let mut fds: Vec<u32> = std::fs::read_dir("/dev/fd")
        .map(|dir| {
            dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    fds.retain(|n| std::fs::metadata(format!("/dev/fd/{n}")).is_ok());
    fds.sort_unstable();
    fds
}

fn join<T: ToString>(items: impl IntoIterator<Item = T>) -> String {
    items
        .into_iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Runs the helper; returns its exit status.
#[must_use]
pub fn main() -> u8 {
    let fault = Fault::from_env();
    // Before anything else opens a descriptor.
    let fds_at_start = (fault == Some(Fault::ReportFds)).then(open_fds);
    let mut input = io::stdin().lock();
    let mut output = BufWriter::new(io::stdout().lock());
    serve(&mut input, &mut output, fault, fds_at_start)
}

/// Writes one frame; `false` when the parent can no longer be reached.
fn send(out: &mut impl Write, msg: &Message) -> bool {
    wire::write_frame(out, msg).is_ok()
}

fn serve(
    input: &mut impl io::Read,
    out: &mut impl Write,
    mut fault: Option<Fault>,
    fds_at_start: Option<Vec<u32>>,
) -> u8 {
    let hello = match wire::read_frame(input) {
        Ok(Message::Hello(h)) => h,
        Ok(Message::Shutdown) | Err(wire::WireError::Closed) => return 0,
        Ok(_) | Err(_) => return EXIT_PROTOCOL,
    };
    if hello.proto_version != wire::PROTO_VERSION {
        let detail = format!(
            "protocol version {} requested; this helper (pdf-extract {}) speaks {}",
            hello.proto_version,
            pdf_extract_core::VERSION,
            wire::PROTO_VERSION
        );
        // EngineNotBuilt in the parent: the library and its helper come
        // from different builds (§3.5, "fails fast with EngineNotBuilt
        // naming both versions").
        send(out, &Message::HelloErr(reason::ENGINE_NOT_BUILT, detail));
        return EXIT_PROTOCOL;
    }
    let mut instance = match start(&hello) {
        Ok((instance, ok)) => {
            if !send(out, &Message::HelloOk(ok)) {
                return 0;
            }
            instance
        }
        Err((r, detail)) => {
            send(out, &Message::HelloErr(r, detail));
            return EXIT_INIT;
        }
    };
    loop {
        let (h, pixels) = match wire::read_frame(input) {
            Ok(Message::Recognize(h, pixels)) => (h, pixels),
            // Dropping the instance ends it (Tesseract's End()).
            Ok(Message::Shutdown) | Err(wire::WireError::Closed) => return 0,
            Ok(_) | Err(_) => return EXIT_PROTOCOL,
        };
        let deadline = crate::engine::deadline_after(
            Instant::now(),
            Duration::from_millis(u64::from(h.deadline_ms)),
        );
        let expected = u64::from(h.width) * u64::from(h.height);
        if h.width == 0 || h.height == 0 || expected != pixels.len() as u64 {
            return EXIT_PROTOCOL;
        }
        let reply = match fault.take() {
            Some(Fault::Abort) => std::process::abort(),
            Some(Fault::Exit) => std::process::exit(EXIT_FAULT),
            Some(Fault::Hang) => loop {
                std::thread::sleep(Duration::from_secs(3600));
            },
            Some(Fault::Garbage) => {
                let mut bad = wire::encode(&Message::Shutdown);
                bad[8..12].copy_from_slice(&(wire::MAX_FRAME + 1).to_le_bytes());
                let _ = out.write_all(&bad).and_then(|()| out.flush());
                continue;
            }
            Some(Fault::ReportFds) => Message::PageErr(
                page_err::RECOGNITION,
                format!("fds={}", join(fds_at_start.iter().flatten())),
            ),
            Some(Fault::ReportEnv) => {
                let mut names: Vec<String> = std::env::vars_os()
                    .map(|(k, _)| k.to_string_lossy().into_owned())
                    .collect();
                names.sort();
                Message::PageErr(page_err::RECOGNITION, format!("env={}", join(names)))
            }
            Some(Fault::Slow(d)) => {
                std::thread::sleep(d);
                recognize(&mut instance, h, pixels, deadline)
            }
            None => recognize(&mut instance, h, pixels, deadline),
        };
        if !send(out, &reply) {
            return 0;
        }
    }
}

/// Creates and validates the instance `hello` describes (O7, O29).
fn start(hello: &wire::Hello) -> Result<(TesseractInstance, HelloOk), (u8, String)> {
    sys::init_process(Path::new(&hello.debug_null))
        .map_err(|e| (reason::INIT_FAILED, e.message))?;
    let dir = Path::new(&hello.tessdata);
    let mut model_sha256 = Vec::new();
    for lang in hello.languages.split('+') {
        // The parent validated the names (O8); checked again because they
        // become a path here.
        if !crate::config::valid_language(lang) {
            return Err((
                reason::INIT_FAILED,
                format!("invalid language name {lang:?}"),
            ));
        }
        let path = tessdata::model_path(dir, lang);
        let digest = sha256_file(&path).map_err(|e| {
            (
                reason::LANGUAGE_DATA_MISSING,
                format!("cannot read {}: {e}", path.display()),
            )
        })?;
        model_sha256.push((lang.to_owned(), digest));
    }
    let mut instance = sys::Instance::new(dir, &hello.languages, hello.oem.to_sys())
        .map(TesseractInstance)
        .map_err(|e| (reason::INIT_FAILED, e.message))?;
    warm_up(&mut instance, hello.psm, WARMUP_BUDGET).map_err(|d| (reason::INIT_FAILED, d))?;
    Ok((
        instance,
        HelloOk {
            tesseract_version: sys::version(),
            library_version: pdf_extract_core::VERSION.to_owned(),
            model_sha256,
        },
    ))
}

/// Recognises one raster and clears the instance for the next.
fn recognize(
    instance: &mut TesseractInstance,
    h: wire::RecognizeHeader,
    pixels: Vec<u8>,
    deadline: Instant,
) -> Message {
    let img = GrayImage {
        width: h.width,
        height: h.height,
        data: pixels,
    };
    let req = OcrRequest {
        dpi: h.dpi,
        deadline,
        psm: h.psm,
    };
    let reply = match instance.recognize(&img, &req) {
        Ok(out) => Message::Result(out),
        Err(OcrPageError::Timeout(_)) => {
            Message::PageErr(page_err::TIMEOUT, "deadline passed".into())
        }
        Err(OcrPageError::Recognition(m)) => Message::PageErr(page_err::RECOGNITION, m),
        Err(other) => Message::PageErr(page_err::RECOGNITION, other.to_string()),
    };
    instance.clear();
    reply
}

#[cfg(test)]
#[path = "helper_tests.rs"]
mod tests;
