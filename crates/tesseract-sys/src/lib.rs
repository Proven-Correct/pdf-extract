//! Tesseract behind an exception-safe shim (spec 0002 §3.4).
//!
//! This crate owns the native half of OCR: it builds (feature `vendored`) or
//! links (feature `system-tesseract`) Tesseract and Leptonica, compiles the
//! C++ shim in `csrc/shim.cpp`, and exposes one safe type, [`Instance`], over
//! it. Higher layers (`pdf-extract-ocr`) pool instances, render pages and
//! map [`Error`] onto the public OCR errors; nothing here knows about PDFs.
//!
//! Invariants this crate keeps:
//! - Rust calls only the shim, never Tesseract's C or C++ API, so no C++
//!   exception can unwind into Rust (D-5).
//! - Process-wide state is touched exactly once, before the first `Init`:
//!   Tesseract's `debug_file` goes to the null device and Leptonica's stderr
//!   handler becomes a no-op, so nothing reaches fds 1 and 2 (O14, O40).
//! - An [`Instance`] is `Send` but not `Sync`: one thread at a time per
//!   Tesseract handle; separate handles are independent.
//!
//! The crate also hosts [`pipe`], the few `poll(2)`/`fcntl(2)` calls that
//! `pdf-extract-ocr`'s process isolation needs (spec O32): that crate is
//! `forbid(unsafe_code)`, and this is the workspace's FFI crate.

mod ffi;
#[cfg(unix)]
pub mod pipe;

use std::any::Any;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Whether the linked Tesseract has the legacy (pre-LSTM) engine. The
/// vendored build is configured without it (D-4); a distribution build
/// usually has it, so `system-tesseract` lets `Init` decide.
pub const LEGACY_ENGINE_BUILT: bool = !cfg!(feature = "vendored");

/// The known tessdata install locations, in the order resolution tries them
/// (spec 0002 O9 item 3; D-3 appended the MacPorts directory).
///
/// Lives here, below `pdf-extract-ocr`, so that this crate's integration
/// tests (which cannot depend on the ocr crate) and the ocr crate's
/// resolver share one list instead of two copies kept in step by a comment.
/// Not part of the crate's supported surface.
#[doc(hidden)]
pub const TESSDATA_KNOWN_DIRS: [&str; 7] = [
    "/usr/share/tesseract-ocr/5/tessdata",
    "/usr/share/tesseract-ocr/4.00/tessdata",
    "/usr/share/tesseract/tessdata",
    "/usr/share/tessdata",
    "/usr/local/share/tessdata",
    "/opt/homebrew/share/tessdata",
    "/opt/local/share/tessdata",
];

/// How the native libraries were obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linkage {
    /// Built from `vendor/` as static archives.
    Vendored,
    /// The distribution's libraries, found by pkg-config.
    System,
}

/// The linkage of this build.
pub const LINKAGE: Linkage = if cfg!(feature = "vendored") {
    Linkage::Vendored
} else {
    Linkage::System
};

/// What went wrong, as far as a caller needs to branch on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// `Init` failed (missing or unreadable model, bad datapath), or the
    /// requested engine mode is not built.
    InitFailed,
    /// An argument the shim or this crate rejected (bad dimensions, a NUL in
    /// a string, a language list outside `[A-Za-z0-9_]+(\+[A-Za-z0-9_]+)*`,
    /// a refused variable name, no image set).
    InvalidArgument,
    /// `Recognize` failed for a reason other than the deadline.
    Recognition,
    /// `Recognize` stopped because the deadline passed.
    Timeout,
    /// A C++ exception was caught by the shim; the message is its `what()`.
    Exception,
}

/// An error from the shim, with the shim's message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("tesseract: {message}")]
pub struct Error {
    /// The classification.
    pub kind: ErrorKind,
    /// The shim's return code, or `None` when this crate refused the call
    /// before reaching the shim.
    pub code: Option<i32>,
    /// Human-readable detail (the shim's error buffer, or ours).
    pub message: String,
}

impl Error {
    fn refused(kind: ErrorKind, message: impl Into<String>) -> Self {
        Error {
            kind,
            code: None,
            message: message.into(),
        }
    }

    /// Converts a non-zero shim code and its error buffer.
    fn from_code(code: c_int, buf: &[u8; ffi::ERRBUF_LEN]) -> Self {
        let kind = match code {
            ffi::PXT_ERR_INIT => ErrorKind::InitFailed,
            ffi::PXT_ERR_EXCEPTION => ErrorKind::Exception,
            ffi::PXT_ERR_TIMEOUT => ErrorKind::Timeout,
            ffi::PXT_ERR_ARG => ErrorKind::InvalidArgument,
            ffi::PXT_ERR_RECOGNIZE | ffi::PXT_ERR_CALLBACK => ErrorKind::Recognition,
            // A code this crate does not know: the shim and ffi.rs disagree.
            _ => ErrorKind::Recognition,
        };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let mut message = String::from_utf8_lossy(&buf[..end]).into_owned();
        if message.is_empty() {
            message = format!("shim error code {code}");
        }
        Error {
            kind,
            code: Some(code),
            message,
        }
    }
}

/// Runs one fallible shim call with a fresh error buffer.
fn call(f: impl FnOnce(*mut c_char, usize) -> c_int) -> Result<(), Error> {
    let mut buf = [0u8; ffi::ERRBUF_LEN];
    let code = f(buf.as_mut_ptr().cast(), buf.len());
    if code == ffi::PXT_OK {
        Ok(())
    } else {
        Err(Error::from_code(code, &buf))
    }
}

/// Tesseract's engine modes, numbered as Tesseract numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineMode {
    /// The legacy engine only (absent from the vendored build).
    TesseractOnly = 0,
    /// The LSTM engine only.
    LstmOnly = 1,
    /// Legacy and LSTM combined (absent from the vendored build).
    TesseractLstmCombined = 2,
    /// Whatever the model supports; LSTM in a build without the legacy engine.
    Default = 3,
}

/// Tesseract's page segmentation modes, numbered as Tesseract numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PageSegMode {
    OsdOnly = 0,
    AutoOsd = 1,
    AutoOnly = 2,
    Auto = 3,
    SingleColumn = 4,
    SingleBlockVertText = 5,
    SingleBlock = 6,
    SingleLine = 7,
    SingleWord = 8,
    CircleWord = 9,
    SingleChar = 10,
    SparseText = 11,
    SparseTextOsd = 12,
    RawLine = 13,
}

/// One recognised word, in image pixels (origin top left).
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    /// Tesseract's UTF-8 text (invalid UTF-8, never seen in practice, is
    /// replaced with U+FFFD).
    pub text: String,
    /// `[left, top, right, bottom]`, clamped at 0; all zero when Tesseract
    /// had no box for the word.
    pub bbox: [u32; 4],
    /// Tesseract's word confidence, 0–100.
    pub confidence: f32,
}

/// A text line: its words in reading order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    /// The words.
    pub words: Vec<Word>,
}

/// A layout block: its lines in reading order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    /// The lines.
    pub lines: Vec<Line>,
}

/// The version string of the linked Tesseract (e.g. `"5.5.3"`).
pub fn version() -> String {
    // SAFETY: pxt_version returns TessBaseAPI::Version(), a pointer to a
    // static NUL-terminated string that lives for the whole process.
    unsafe { CStr::from_ptr(ffi::pxt_version()) }
        .to_string_lossy()
        .into_owned()
}

/// The platform's null device, where Tesseract's debug output goes.
const NULL_DEVICE: &CStr = if cfg!(windows) { c"NUL" } else { c"/dev/null" };

/// The outcome of the one process-wide initialisation.
///
/// **Forking.** The `OnceLock` is held while the initialisation runs; a
/// `fork()` while another thread is inside it leaves the lock held forever
/// in the child, which then deadlocks on its first `Instance::new`. Build
/// an engine before forking; see the `Forking` section of the
/// `pdf-extract-ocr` crate docs. No `atfork` handler is installed.
static GLOBAL_INIT: OnceLock<Result<(), Error>> = OnceLock::new();

/// How many times this process ran `pxt_global_init` (0 or 1). Process
/// isolation's tests read it to prove the parent never loads Tesseract
/// (spec 0002 §4.9).
static GLOBAL_INIT_CALLS: AtomicUsize = AtomicUsize::new(0);

/// Sets the process-wide silence once, before any `Init` (O14), sending
/// Tesseract's debug output to `debug_null`. A failure is remembered and
/// returned to every later caller: without it Tesseract would print to
/// stderr, so no instance may be created.
///
/// Only the first call (this one, or the implicit one inside
/// [`Instance::new`], which uses the platform null device) takes effect;
/// later calls return the stored outcome whatever path they pass. The OCR
/// helper process calls this with the path its parent sent (spec §3.5)
/// before creating its instance.
///
/// # Errors
/// [`ErrorKind::InvalidArgument`] for a path containing NUL (not stored:
/// nothing was initialised), or the stored error of the first call.
pub fn init_process(debug_null: &Path) -> Result<(), Error> {
    let path = cstring("debug_file path", path_bytes(debug_null))?;
    init_with(&path)
}

fn init_with(debug_null: &CStr) -> Result<(), Error> {
    GLOBAL_INIT
        .get_or_init(|| {
            GLOBAL_INIT_CALLS.fetch_add(1, Ordering::Relaxed);
            // SAFETY: debug_null is a NUL-terminated string that outlives the
            // call; err/len describe call()'s live buffer.
            call(|err, len| unsafe { ffi::pxt_global_init(debug_null.as_ptr(), err, len) })
        })
        .clone()
}

fn global_init() -> Result<(), Error> {
    init_with(NULL_DEVICE)
}

/// See [`GLOBAL_INIT_CALLS`] (test observability).
#[doc(hidden)]
pub fn global_init_calls() -> usize {
    GLOBAL_INIT_CALLS.load(Ordering::Relaxed)
}

fn cstring(what: &str, bytes: Vec<u8>) -> Result<CString, Error> {
    CString::new(bytes)
        .map_err(|_| Error::refused(ErrorKind::InvalidArgument, format!("{what} contains NUL")))
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

/// Per-instance parameters that are refused although they are not
/// process-global (spec 0003 F18). The process-global set is Tesseract's own
/// answer ([`is_global_param`]), not a list here.
///
/// Scope: `with_variables` refuses every process-global param and every
/// per-instance param that (a) opens a file named by or derived from the
/// variable, (b) selects a training, dump or box mode, or (c) changes which
/// models Init loads (e.g. `tessedit_ocr_engine_mode`,
/// `tessedit_init_config_only`, `tessedit_load_sublangs`); the list is
/// re-audited at every Tesseract upgrade (last: 5.5.3). Params outside these
/// classes are residuals, not defects. "Models" means the engines and the
/// set of traineddata languages; dictionary toggles (`load_*_dawg`,
/// `lstm_use_matrix`) only shrink what one model loads and stay allowed.
///
/// Known residuals, left allowed on purpose (ADR 0082, addendum 2026-10-21):
/// `interactive_display_mode` (opens a ScrollView connection in builds
/// without `GRAPHICS_DISABLED`; the vendored build skips recognition and
/// returns an empty page instead), the legacy adaptation parameters such as
/// `classify_enable_learning` (they change results, not files or models),
/// and the legacy-build `oem` argument that re-overrides
/// `tessedit_ocr_engine_mode` (moot while that name is refused).
const REFUSED_INSTANCE_PARAMS: [&str; 20] = [
    // (a) Files: written (`tessedit_write_params_to_file` at Init,
    // `output_ambig_words_file` during recognition) or read from any path
    // (`user_*_file`) or from `<datapath>/<lang>.<suffix>` (`user_*_suffix`).
    "tessedit_write_params_to_file",
    "output_ambig_words_file",
    "user_words_file",
    "user_patterns_file",
    "user_words_suffix",
    "user_patterns_suffix",
    // (b) Dump modes, each writing into the working directory or next to
    // the input/output name: `<output>.processed.tif`, `<input>_debug.pdf`
    // on Clear, `vhlinefinding.pdf`, `<input>.a`, `<input>.doc`
    // (`save_doc_words`; Tesseract aborts when it cannot open that file).
    "tessedit_write_images",
    "tessedit_dump_pageseg_images",
    "textord_tabfind_show_vlines",
    "classify_save_adapted_templates",
    "save_doc_words",
    // (b) Training and box modes: Recognize reads `<input>.box` files and/or
    // writes `.lstmf`, `.tr` or training text.
    "tessedit_train_line_recognizer",
    "tessedit_train_from_boxes",
    "tessedit_ambigs_training",
    "tessedit_resegment_from_boxes",
    "tessedit_resegment_from_line_boxes",
    "tessedit_make_boxes_from_boxes",
    // (c) Model selection. These act after the checks in `with_variables`
    // and so undo them: an engine mode the vendored build lacks (it fixes
    // the mode before applying variables; Init then "succeeds" with no
    // engine), no model at all (Recognize then crashes), or sub-languages
    // joined into `<datapath>/<lang>.traineddata` without
    // `valid_language_list`.
    "tessedit_ocr_engine_mode",
    "tessedit_init_config_only",
    "tessedit_load_sublangs",
];

/// Whether `languages` is Tesseract's `+`-joined form with no empty member:
/// `[A-Za-z0-9_]+(\+[A-Za-z0-9_]+)*`. Tesseract itself skips a leading or
/// doubled `+` and substitutes `eng` for an empty code, so `"+"` would
/// silently load `eng` and `"eng+"` would load it twice. The character set
/// also keeps a path separator or `..` from reaching the filesystem through
/// `<datapath>/<lang>.traineddata`.
fn valid_language_list(languages: &str) -> bool {
    languages.split('+').all(|code| {
        !code.is_empty() && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    })
}

/// Asks Tesseract whether `name` is registered as a process-global
/// parameter (shim `pxt_is_global_param`).
fn is_global_param(name: &CStr) -> Result<bool, Error> {
    let mut buf = [0u8; ffi::ERRBUF_LEN];
    let (err, len) = (buf.as_mut_ptr().cast(), buf.len());
    // SAFETY: `name` is NUL-terminated and outlives the call; err/len
    // describe `buf`, live here, which the shim writes at most len bytes of.
    let code = unsafe { ffi::pxt_is_global_param(name.as_ptr(), err, len) };
    match code {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::from_code(code, &buf)),
    }
}

/// Refuses a variable `Init` must not be given (spec 0003 F18): any
/// process-global parameter, which would change every instance in the
/// process (`debug_file` would undo the silence of O14), and the
/// per-instance parameters of [`REFUSED_INSTANCE_PARAMS`].
fn check_variable(name: &str, cname: &CStr) -> Result<(), Error> {
    if REFUSED_INSTANCE_PARAMS.contains(&name) {
        return Err(Error::refused(
            ErrorKind::InvalidArgument,
            format!(
                "variable {name:?} makes Tesseract open files, enter a training, dump or box \
                 mode, or change which models it loads, and is refused"
            ),
        ));
    }
    if is_global_param(cname)? {
        return Err(Error::refused(
            ErrorKind::InvalidArgument,
            format!(
                "variable {name:?} is a process-global Tesseract parameter and cannot be set \
                 per instance"
            ),
        ));
    }
    Ok(())
}

/// One Tesseract handle with its models loaded.
///
/// Created by [`Instance::new`]; reused across pages by calling
/// [`Instance::clear`] between them; freed on drop.
#[derive(Debug)]
pub struct Instance {
    raw: NonNull<ffi::PxtHandle>,
    // Raw pointers already make the type !Send and !Sync; Send is restored
    // below, and this marker documents that Sync is withheld on purpose.
    _not_sync: PhantomData<std::cell::Cell<()>>,
}

// SAFETY: a TessBaseAPI has no thread affinity; it may be used from any
// thread as long as one thread at a time uses it, which `&mut self` on every
// method and the missing Sync impl guarantee. Instances share only the
// mutex-protected dawg cache and process-wide parameters we never change
// after global_init.
unsafe impl Send for Instance {}

impl Instance {
    /// Creates a handle and loads `languages` (Tesseract's `+`-joined form,
    /// e.g. `"eng"` or `"eng+deu"`) from the tessdata directory `datapath`
    /// (the directory holding `<lang>.traineddata`).
    pub fn new(datapath: &Path, languages: &str, oem: EngineMode) -> Result<Self, Error> {
        Self::with_variables(datapath, languages, oem, &[])
    }

    /// As [`Instance::new`], passing per-instance Tesseract variables to
    /// `Init` (never through `SetVariable`, never through config files).
    ///
    /// Every name is checked first: a process-global parameter (one
    /// Tesseract registers in `GlobalParams()`, such as `debug_file`,
    /// `log_level` or `dotproduct`) and a per-instance parameter that opens
    /// files, selects a training, dump or box mode, or changes which models
    /// `Init` loads (the list and its scope: `REFUSED_INSTANCE_PARAMS`)
    /// are refused with [`ErrorKind::InvalidArgument`] naming the variable,
    /// and nothing is initialised. Other per-instance parameters, debug
    /// toggles included, are passed through. A model's own `lang.config` is
    /// applied by Tesseract without this check.
    pub fn with_variables(
        datapath: &Path,
        languages: &str,
        oem: EngineMode,
        variables: &[(&str, &str)],
    ) -> Result<Self, Error> {
        if !LEGACY_ENGINE_BUILT
            && matches!(
                oem,
                EngineMode::TesseractOnly | EngineMode::TesseractLstmCombined
            )
        {
            return Err(Error::refused(
                ErrorKind::InitFailed,
                "legacy engine not built",
            ));
        }
        if !valid_language_list(languages) {
            return Err(Error::refused(
                ErrorKind::InvalidArgument,
                format!("invalid language list {languages:?}"),
            ));
        }
        let datapath = cstring("tessdata path", path_bytes(datapath))?;
        let langs = cstring("language list", languages.as_bytes().to_vec())?;
        let mut names = Vec::with_capacity(variables.len());
        let mut values = Vec::with_capacity(variables.len());
        for (name, value) in variables {
            let cname = cstring("variable name", name.as_bytes().to_vec())?;
            check_variable(name, &cname)?;
            names.push(cname);
            values.push(cstring("variable value", value.as_bytes().to_vec())?);
        }
        let name_ptrs: Vec<*const c_char> = names.iter().map(|s| s.as_ptr()).collect();
        let value_ptrs: Vec<*const c_char> = values.iter().map(|s| s.as_ptr()).collect();

        global_init()?;
        // SAFETY: pxt_create takes no arguments and returns an owned handle
        // or null (allocation failure, caught in the shim).
        let raw = NonNull::new(unsafe { ffi::pxt_create() }).ok_or_else(|| {
            Error::refused(
                ErrorKind::Exception,
                "could not allocate a Tesseract handle",
            )
        })?;
        // Owned from here on: Drop frees the handle on every error path.
        let instance = Instance {
            raw,
            _not_sync: PhantomData,
        };
        // SAFETY: the handle is live and exclusively ours; every pointer is
        // a NUL-terminated CString (or an array of them, of length n) that
        // outlives the call; err/len describe call()'s buffer.
        call(|err, len| unsafe {
            ffi::pxt_init(
                instance.raw.as_ptr(),
                datapath.as_ptr(),
                langs.as_ptr(),
                oem as c_int,
                name_ptrs.as_ptr(),
                value_ptrs.as_ptr(),
                name_ptrs.len(),
                err,
                len,
            )
        })?;
        Ok(instance)
    }

    /// Hands an 8-bit gray image to Tesseract (copied; `data` may be freed
    /// afterwards). `data` holds `height` rows of `bytes_per_line` bytes,
    /// of which the first `width` are pixels.
    pub fn set_image(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        bytes_per_line: u32,
    ) -> Result<(), Error> {
        let bad = |what: &str| Error::refused(ErrorKind::InvalidArgument, what.to_owned());
        let w = c_int::try_from(width).map_err(|_| bad("image width over i32"))?;
        let h = c_int::try_from(height).map_err(|_| bad("image height over i32"))?;
        let bpl = c_int::try_from(bytes_per_line).map_err(|_| bad("row stride over i32"))?;
        if w == 0 || h == 0 || bytes_per_line < width {
            return Err(bad("empty image or row stride shorter than the width"));
        }
        let expected = u64::from(height) * u64::from(bytes_per_line);
        if data.len() as u64 != expected {
            return Err(bad("image buffer length is not height x bytes_per_line"));
        }
        // SAFETY: the handle is live and exclusively borrowed; `data` holds
        // exactly height x bytes_per_line bytes (checked above), which is
        // all the shim reads; err/len describe call()'s buffer.
        call(|err, len| unsafe {
            ffi::pxt_set_image(self.raw.as_ptr(), data.as_ptr(), w, h, 1, bpl, err, len)
        })
    }

    /// Tells Tesseract the image's resolution in pixels per inch. Requires
    /// an image. A value Tesseract finds implausible (0, say) is replaced by
    /// Tesseract with its own estimate at recognition time.
    pub fn set_source_resolution(&mut self, ppi: u32) -> Result<(), Error> {
        let ppi = c_int::try_from(ppi).unwrap_or(c_int::MAX);
        // SAFETY: live, exclusively borrowed handle; err/len from call().
        call(|err, len| unsafe { ffi::pxt_set_resolution(self.raw.as_ptr(), ppi, err, len) })
    }

    /// Sets the page segmentation mode for the next recognition.
    pub fn set_page_seg_mode(&mut self, psm: PageSegMode) -> Result<(), Error> {
        // SAFETY: live, exclusively borrowed handle; err/len from call().
        call(|err, len| unsafe { ffi::pxt_set_psm(self.raw.as_ptr(), psm as c_int, err, len) })
    }

    /// Recognises the current image, giving up once `timeout` has elapsed
    /// (Tesseract polls the deadline between words, so the return can be
    /// up to one unit of work late). A zero timeout is already expired.
    pub fn recognize(&mut self, timeout: Duration) -> Result<(), Error> {
        if timeout.is_zero() {
            return Err(Error::refused(
                ErrorKind::Timeout,
                "the deadline has already passed",
            ));
        }
        // At least 1 ms: the shim treats a non-positive deadline as an error.
        let ms = c_int::try_from(timeout.as_millis().max(1)).unwrap_or(c_int::MAX);
        // SAFETY: live, exclusively borrowed handle; err/len from call().
        call(|err, len| unsafe { ffi::pxt_recognize(self.raw.as_ptr(), ms, err, len) })
    }

    /// The last recognition's words as blocks of lines, in Tesseract's
    /// reading order. Empty when nothing was recognised (or nothing ran).
    pub fn blocks(&mut self) -> Result<Vec<Block>, Error> {
        self.collect(Collector::default())
    }

    /// Test hook (debug builds only): [`Instance::blocks`] with a callback
    /// that panics on the first word, proving the panic crosses the shim
    /// without unwinding through C++ and reaches the caller with its
    /// payload. The public API takes no callback, so a test cannot supply
    /// a panicking one itself.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn test_blocks_panicking(&mut self) -> Result<Vec<Block>, Error> {
        self.collect(Collector {
            panic_on_word: true,
            ..Collector::default()
        })
    }

    fn collect(&mut self, mut collector: Collector) -> Result<Vec<Block>, Error> {
        let user: *mut Collector = &mut collector;
        // SAFETY: live, exclusively borrowed handle; `on_word` matches the
        // shim's callback type and `user` points at `collector`, which
        // outlives the call and is accessed only through this pointer
        // while the shim runs; err/len from call().
        let result = call(|err, len| unsafe {
            ffi::pxt_collect(self.raw.as_ptr(), on_word, user.cast(), err, len)
        });
        if let Some(payload) = collector.panic {
            // Rust callers receive panics as panics (0001 D-16).
            resume_unwind(payload);
        }
        result.map(|()| collector.blocks)
    }

    /// Drops the image and results, keeping the models loaded.
    pub fn clear(&mut self) {
        // SAFETY: live, exclusively borrowed handle.
        unsafe { ffi::pxt_clear(self.raw.as_ptr()) }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the handle is live and owned by this Instance, which is
        // being dropped, so nothing can use it afterwards; pxt_end before
        // pxt_delete is the order the shim documents.
        unsafe {
            ffi::pxt_end(self.raw.as_ptr());
            ffi::pxt_delete(self.raw.as_ptr());
        }
    }
}

#[derive(Default)]
struct Collector {
    blocks: Vec<Block>,
    panic: Option<Box<dyn Any + Send>>,
    /// Set only by [`Instance::test_blocks_panicking`].
    #[cfg(debug_assertions)]
    panic_on_word: bool,
}

impl Collector {
    fn push(&mut self, flags: c_int, word: Word) {
        if flags & ffi::PXT_BLOCK_START != 0 || self.blocks.is_empty() {
            self.blocks.push(Block::default());
        }
        let block = self.blocks.last_mut().expect("pushed above when empty");
        if flags & ffi::PXT_LINE_START != 0 || block.lines.is_empty() {
            block.lines.push(Line::default());
        }
        let line = block.lines.last_mut().expect("pushed above when empty");
        line.words.push(word);
    }
}

#[cfg(debug_assertions)]
thread_local! {
    /// How many times [`on_word`] has run on this thread. Per thread, not
    /// global, because the shim calls back on the thread that called
    /// `pxt_collect` and tests run in parallel.
    static WORD_CALLBACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test hook (debug builds only): how many times the word callback has run
/// on the calling thread, so a test can tell that a walk stopped where the
/// callback asked it to (return 1) rather than carrying on and discarding
/// the rest.
#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn test_word_callbacks() -> usize {
    WORD_CALLBACKS.with(std::cell::Cell::get)
}

/// The shim's per-word callback. Never unwinds: a panic is caught, stored
/// and re-raised by [`Instance::blocks`] after the shim has returned.
unsafe extern "C" fn on_word(
    user: *mut c_void,
    flags: c_int,
    text: *const c_char,
    len: usize,
    left: c_int,
    top: c_int,
    right: c_int,
    bottom: c_int,
    confidence: f32,
) -> c_int {
    // SAFETY: `user` is the `&mut Collector` passed by Instance::blocks,
    // live and unaliased for the duration of pxt_collect.
    let collector = unsafe { &mut *user.cast::<Collector>() };
    #[cfg(debug_assertions)]
    WORD_CALLBACKS.with(|n| n.set(n.get() + 1));
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        #[cfg(debug_assertions)]
        if collector.panic_on_word {
            panic!("on_word: deliberate panic");
        }
        // SAFETY: the shim passes Tesseract's NUL-terminated word text and
        // its strlen; the buffer lives until this callback returns.
        let bytes = unsafe { std::slice::from_raw_parts(text.cast::<u8>(), len) };
        let clamp = |v: c_int| u32::try_from(v).unwrap_or(0);
        collector.push(
            flags,
            Word {
                text: String::from_utf8_lossy(bytes).into_owned(),
                bbox: [clamp(left), clamp(top), clamp(right), clamp(bottom)],
                confidence,
            },
        );
    }));
    match outcome {
        Ok(()) => 0,
        Err(payload) => {
            collector.panic = Some(payload);
            1
        }
    }
}

/// Test hook (debug builds only): makes the shim throw a
/// `std::runtime_error` inside its guard, proving exceptions become
/// [`ErrorKind::Exception`] with the exception's message.
#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn test_throw() -> Result<(), Error> {
    // SAFETY: err/len describe call()'s live buffer; no other state.
    call(|err, len| unsafe { ffi::pxt_test_throw(err, len) })
}
