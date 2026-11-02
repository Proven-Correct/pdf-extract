//! `OcrEngine`, `OcrInfo` and `default_ocr_engine` (run 0002 §3.6, O8, O12,
//! O13, O21, O28), plus the test-only hooks `_scripted_engine`,
//! `_scripted_stats` and `_set_tessdata_candidates` (§4.11).
//!
//! **Arguments.** Out-of-range values are argument errors, so they raise
//! `ValueError` here before any engine is built (O8: `ValueError` from
//! Python, `InitFailed` from Rust). What only the host can answer — no model
//! directory, a missing language, Tesseract refusing to start, a legacy
//! engine mode on a build without the legacy engine, a missing helper —
//! is `OcrUnavailableError` from the engine itself.
//!
//! **GIL.** Construction loads the model (tens to hundreds of milliseconds)
//! and runs under `py.detach` (O21), as does the first `default_ocr_engine()`
//! call; the getters are constant time and stay attached.
//!
//! **Isolation default (internal #53, ADR 0092).** Python engines run Tesseract in
//! helper processes unless the caller passes `isolation="in_process"`, and
//! the engine behind `default_ocr_engine()` and `Page.ocr()` without an
//! engine is this binding's own process-isolated one, not the Rust crate's
//! (which stays in-process: a Rust binary has no package directory to find
//! the helper in). A missing helper is `OcrUnavailableError(reason=
//! "engine_not_built")`; there is deliberately no fallback to in-process,
//! which would turn a broken install into an uncontained one.
//!
//! **Helper discovery (O28, D-16).** With `isolation="process"` the engine is
//! told to look for `pdf-extract-ocr-helper` in the directory of this
//! extension module (`Path(_native.__file__).resolve().parent`), which is
//! where `tools/build/stage-helper.sh` and the wheel put it. Nothing on
//! `PATH` is searched.
//!
//! **Fork.** An engine survives `os.fork()`: the child rebuilds its pool on
//! first use (O11, O30). One race is not defended: a fork while another
//! thread is inside the first `default_ocr_engine()` call, or holds the
//! engine's pool lock, leaves that lock held forever in the child. Programs
//! that fork from a threaded process should build their engines before
//! forking, or use `multiprocessing` with the "spawn" or "forkserver" start
//! method (the crate docs of `pdf-extract-ocr` carry the full statement).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use pdf_extract_core::{OcrUnavailable, UnavailableReason};
use pdf_extract_ocr as ocr;
use pdf_extract_testkit::ocr::{ScriptedBackend, Step, output};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use crate::errors::ocr_unavailable;
use crate::guard::guarded;

/// The `isolation` literals, in [`ocr::Isolation`] order.
fn isolation(value: &str) -> PyResult<ocr::Isolation> {
    match value {
        "in_process" => Ok(ocr::Isolation::InProcess),
        "process" => Ok(ocr::Isolation::Process),
        other => Err(PyValueError::new_err(format!(
            "isolation must be 'in_process' or 'process', not {other:?}"
        ))),
    }
}

fn isolation_name(value: ocr::Isolation) -> &'static str {
    match value {
        ocr::Isolation::InProcess => "in_process",
        ocr::Isolation::Process => "process",
    }
}

/// The `psm` literals, with the enum values they name.
const PSM: [(&str, ocr::PageSegMode); 14] = [
    ("osd_only", ocr::PageSegMode::OsdOnly),
    ("auto_osd", ocr::PageSegMode::AutoOsd),
    ("auto_only", ocr::PageSegMode::AutoOnly),
    ("auto", ocr::PageSegMode::Auto),
    ("single_column", ocr::PageSegMode::SingleColumn),
    (
        "single_block_vert_text",
        ocr::PageSegMode::SingleBlockVertText,
    ),
    ("single_block", ocr::PageSegMode::SingleBlock),
    ("single_line", ocr::PageSegMode::SingleLine),
    ("single_word", ocr::PageSegMode::SingleWord),
    ("circle_word", ocr::PageSegMode::CircleWord),
    ("single_char", ocr::PageSegMode::SingleChar),
    ("sparse_text", ocr::PageSegMode::SparseText),
    ("sparse_text_osd", ocr::PageSegMode::SparseTextOsd),
    ("raw_line", ocr::PageSegMode::RawLine),
];

/// The `oem` literals, with the enum values they name.
const OEM: [(&str, ocr::EngineMode); 4] = [
    ("tesseract_only", ocr::EngineMode::TesseractOnly),
    ("lstm_only", ocr::EngineMode::LstmOnly),
    (
        "tesseract_lstm_combined",
        ocr::EngineMode::TesseractLstmCombined,
    ),
    ("default", ocr::EngineMode::Default),
];

/// Looks `value` up in a literal table, or raises `ValueError` listing the
/// accepted spellings.
fn literal<T: Copy>(name: &str, table: &[(&'static str, T)], value: &str) -> PyResult<T> {
    table
        .iter()
        .find(|(k, _)| *k == value)
        .map(|(_, v)| *v)
        .ok_or_else(|| {
            let accepted: Vec<String> = table.iter().map(|(k, _)| format!("'{k}'")).collect();
            PyValueError::new_err(format!(
                "{name} must be one of {}, not {value:?}",
                accepted.join(", ")
            ))
        })
}

/// The literal naming `value` in `table` (every enum value is listed).
fn literal_name<T: Copy + PartialEq>(table: &[(&'static str, T)], value: T) -> &'static str {
    table
        .iter()
        .find(|(_, v)| *v == value)
        .map_or("?", |(k, _)| *k)
}

/// `languages` as the caller may pass it: `"eng"`, `"eng+deu"` or a
/// sequence of names.
#[derive(FromPyObject)]
pub(crate) enum Languages {
    One(String),
    Many(Vec<String>),
}

impl Languages {
    /// The individual names, checked like Rust's `OcrConfig::validate`
    /// (`[A-Za-z0-9_]+`, at least one) but as an argument error.
    fn names(self) -> PyResult<Vec<String>> {
        let names: Vec<String> = match self {
            Self::One(joined) => joined.split('+').map(str::to_owned).collect(),
            Self::Many(names) => names,
        };
        if names.is_empty() {
            return Err(PyValueError::new_err(
                "languages must name at least one language",
            ));
        }
        if let Some(bad) = names
            .iter()
            .find(|n| n.is_empty() || !n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        {
            return Err(PyValueError::new_err(format!(
                "invalid language name {bad:?} (allowed: A-Z, a-z, 0-9, _)"
            )));
        }
        Ok(names)
    }
}

/// `value` as a `dpi`: an integer in `MIN_DPI..=MAX_DPI`.
pub(crate) fn dpi(value: i128) -> PyResult<u32> {
    u32::try_from(value)
        .ok()
        .filter(|d| (ocr::MIN_DPI..=ocr::MAX_DPI).contains(d))
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "dpi must be between {} and {}, got {value}",
                ocr::MIN_DPI,
                ocr::MAX_DPI
            ))
        })
}

/// `value` as a positive integer of type `T`.
fn positive<T: TryFrom<i128>>(name: &str, value: i128) -> PyResult<T> {
    if value < 1 {
        return Err(PyValueError::new_err(format!(
            "{name} must be at least 1, got {value}"
        )));
    }
    T::try_from(value)
        .map_err(|_| PyValueError::new_err(format!("{name} is too large, got {value}")))
}

/// `value` as `max_render_pixels`: positive, and within one helper frame
/// under process isolation. Checked like Rust's `OcrConfig::validate` but as
/// an argument error (O8).
fn render_pixels(value: i128, isolation: ocr::Isolation) -> PyResult<u64> {
    let n: u64 = positive("max_render_pixels", value)?;
    if isolation == ocr::Isolation::Process && n > ocr::MAX_PROCESS_RENDER_PIXELS {
        return Err(PyValueError::new_err(format!(
            "max_render_pixels must be at most {} with isolation='process' \
             (a raster must fit one helper frame), got {value}; pass \
             isolation='in_process' to allow a larger value",
            ocr::MAX_PROCESS_RENDER_PIXELS
        )));
    }
    Ok(n)
}

/// `value` seconds as a non-zero `Duration`.
fn timeout(value: f64) -> PyResult<Duration> {
    Duration::try_from_secs_f64(value)
        .ok()
        .filter(|d| !d.is_zero())
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "page_timeout must be a finite number of seconds greater than zero, got {value}"
            ))
        })
}

/// The directory of this extension module, resolved (O28: the helper is
/// installed beside it). `None` if the module has no usable `__file__`
/// (an embedded build); discovery then falls back to the executable's
/// directory, and reports a missing helper as `engine_not_built`.
fn package_dir(py: Python<'_>) -> Option<PathBuf> {
    let file: PathBuf = py
        .import("pdf_extract._native")
        .and_then(|m| m.getattr("__file__"))
        .and_then(|f| f.extract())
        .ok()?;
    // The module file itself is resolved, symlinks included, so a linked
    // `_native` finds the helper beside its target (where the wheel and
    // stage-helper.sh put it, and where `Path(__file__).resolve().parent`
    // looks). Absolute and symlink-free also means a respawn after a `chdir`
    // finds the same file; discovery refuses a relative directory.
    let resolved = std::fs::canonicalize(&file)
        .or_else(|_| std::path::absolute(&file))
        .ok()?;
    resolved.parent().map(Path::to_path_buf)
}

/// The helper's file name, beside the extension module (ADR 0076).
const HELPER_NAME: &str = "pdf-extract-ocr-helper";

/// `err` with a hint for the commonest cause of a missing helper in a
/// development or source install: the build did not stage it. Added only
/// when the package directory (absolute, as discovery requires) really
/// lacks the file and no `PDF_EXTRACT_OCR_HELPER` override chose another
/// one; an empty override counts as unset, as it does for discovery.
fn stage_hint(mut err: OcrUnavailable, helper_dir: Option<&Path>) -> OcrUnavailable {
    let overridden = std::env::var_os("PDF_EXTRACT_OCR_HELPER").is_some_and(|v| !v.is_empty());
    if err.reason == UnavailableReason::EngineNotBuilt
        && !overridden
        && helper_dir.is_some_and(|dir| dir.is_absolute() && !dir.join(HELPER_NAME).is_file())
    {
        err.detail.push_str(
            " (a source build must run tools/build/stage-helper.sh before maturin \
             develop/build; or pass isolation='in_process')",
        );
    }
    err
}

/// Builds an engine for `cfg`, without the GIL; a missing helper in the
/// package directory gets [`stage_hint`].
fn build_engine(py: Python<'_>, cfg: ocr::OcrConfig) -> Result<ocr::OcrEngine, OcrUnavailable> {
    let dir = cfg.helper_dir.clone();
    py.detach(|| ocr::OcrEngine::tesseract(cfg))
        .map_err(|e| stage_hint(e, dir.as_deref()))
}

/// An OCR engine: a configuration, the Tesseract models and a pool of
/// instances. Build once and share between threads; see the stub for the
/// arguments.
///
/// With `isolation="process"` (the default), each pool slot is a helper
/// process, so a crash inside Tesseract fails one page instead of ending
/// the interpreter. `isolation="in_process"` runs Tesseract in this process
/// (no helper; an abort inside Tesseract ends the interpreter) and is the
/// only mode that allows `max_render_pixels` above 66977776.
///
/// Forking. An engine built before ``os.fork()`` is reused by the child,
/// which rebuilds its pool on first use. A fork while another thread is
/// inside OCR, or is building an engine (the first ``default_ocr_engine()``
/// call included), leaves a lock held forever in the child and deadlocks
/// it. The same applies to ``multiprocessing`` with the ``fork`` start
/// method. Safe patterns: use the ``spawn`` or ``forkserver`` start method
/// (CPython 3.14 made ``forkserver`` the default on POSIX, per its
/// release notes "Changed in version 3.14: On POSIX platforms the default
/// start method was changed from *fork* to *forkserver* to retain the
/// performance but avoid common multithreaded process incompatibilities";
/// earlier versions default to ``fork`` on Linux), or fork before the
/// first OCR call from a process with no other thread using OCR, or build
/// every engine, the default one included, before forking.
#[pyclass(module = "pdf_extract", name = "OcrEngine", frozen)]
pub(crate) struct OcrEngine {
    pub(crate) inner: ocr::OcrEngine,
    /// The script behind a `_scripted_engine` (tests only).
    scripted: Option<ScriptedBackend>,
}

impl OcrEngine {
    pub(crate) fn wrap(inner: ocr::OcrEngine) -> Self {
        Self {
            inner,
            scripted: None,
        }
    }
}

#[pymethods]
impl OcrEngine {
    // Literal defaults where PyO3 can show them in `__text_signature__`;
    // `languages` has none it can render, so its default shows as `...`.
    #[new]
    #[pyo3(signature = (
        *,
        languages = Languages::One(String::from("eng")),
        tessdata = None,
        dpi = 300,
        max_instances = None,
        page_timeout = 60.0,
        max_render_pixels = 40_000_000,
        psm = "auto",
        oem = "lstm_only",
        isolation = "process",
        noise_precheck = true,
    ))]
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors the keyword-only Python signature"
    )]
    fn new(
        py: Python<'_>,
        languages: Languages,
        tessdata: Option<PathBuf>,
        dpi: i128,
        max_instances: Option<i128>,
        page_timeout: f64,
        max_render_pixels: i128,
        psm: &str,
        oem: &str,
        isolation: &str,
        noise_precheck: bool,
    ) -> PyResult<Self> {
        guarded(|| {
            let defaults = ocr::OcrConfig::default();
            let isolation = self::isolation(isolation)?;
            let cfg = ocr::OcrConfig {
                tessdata,
                languages: languages.names()?,
                dpi: self::dpi(dpi)?,
                max_instances: max_instances
                    .map(|n| positive("max_instances", n))
                    .transpose()?
                    .unwrap_or(defaults.max_instances),
                page_timeout: timeout(page_timeout)?,
                psm: literal("psm", &PSM, psm)?,
                oem: literal("oem", &OEM, oem)?,
                max_render_pixels: render_pixels(max_render_pixels, isolation)?,
                isolation,
                helper_dir: match isolation {
                    ocr::Isolation::Process => package_dir(py),
                    ocr::Isolation::InProcess => None,
                },
                noise_precheck,
            };
            build_engine(py, cfg)
                .map(Self::wrap)
                .map_err(|e| ocr_unavailable(py, e))
        })
    }

    /// The model directory in use ("" for a test engine without one).
    #[getter]
    fn tessdata(&self) -> PyResult<OsString> {
        // `OsString`, not `PathBuf`: PyO3 turns a path into `pathlib.Path`,
        // and the stub promises `str`.
        guarded(|| {
            Ok(self
                .inner
                .tessdata()
                .map(|p| p.as_os_str().to_owned())
                .unwrap_or_default())
        })
    }

    /// The languages recognised, in order.
    #[getter]
    fn languages<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        guarded(|| PyTuple::new(py, &self.inner.config().languages))
    }

    /// The default resolution pages are rendered at.
    #[getter]
    fn dpi(&self) -> PyResult<u32> {
        guarded(|| Ok(self.inner.config().dpi))
    }

    /// The most Tesseract instances (concurrent recognitions) the engine keeps.
    #[getter]
    fn max_instances(&self) -> PyResult<usize> {
        guarded(|| Ok(self.inner.config().max_instances))
    }

    /// The budget for one page, rendering and recognition, in seconds.
    #[getter]
    fn page_timeout(&self) -> PyResult<f64> {
        guarded(|| Ok(self.inner.config().page_timeout.as_secs_f64()))
    }

    /// The most pixels a page raster may hold.
    #[getter]
    fn max_render_pixels(&self) -> PyResult<u64> {
        guarded(|| Ok(self.inner.config().max_render_pixels))
    }

    /// The page segmentation mode.
    #[getter]
    fn psm(&self) -> PyResult<&'static str> {
        guarded(|| Ok(literal_name(&PSM, self.inner.config().psm)))
    }

    /// The engine mode.
    #[getter]
    fn oem(&self) -> PyResult<&'static str> {
        guarded(|| Ok(literal_name(&OEM, self.inner.config().oem)))
    }

    /// `"in_process"` or `"process"`.
    #[getter]
    fn isolation(&self) -> PyResult<&'static str> {
        guarded(|| Ok(isolation_name(self.inner.config().isolation)))
    }

    /// Whether the noise pre-check runs before recognition.
    #[getter]
    fn noise_precheck(&self) -> PyResult<bool> {
        guarded(|| Ok(self.inner.config().noise_precheck))
    }

    /// What the engine runs: Tesseract's version, the models and their
    /// SHA-256 digests, the linkage and the isolation.
    fn info(&self) -> PyResult<OcrInfo> {
        guarded(|| {
            Ok(OcrInfo {
                inner: self.inner.info(),
            })
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        guarded(|| {
            let cfg = self.inner.config();
            Ok(format!(
                "<pdf_extract.OcrEngine languages={:?} dpi={} max_instances={} isolation='{}'>",
                cfg.languages.join("+"),
                cfg.dpi,
                cfg.max_instances,
                isolation_name(cfg.isolation)
            ))
        })
    }
}

/// What an `OcrEngine` runs (`OcrEngine.info()`).
#[pyclass(module = "pdf_extract", name = "OcrInfo", frozen)]
pub(crate) struct OcrInfo {
    inner: ocr::BackendInfo,
}

#[pymethods]
impl OcrInfo {
    /// The backend: `"tesseract"`.
    #[getter]
    fn name(&self) -> PyResult<&'static str> {
        guarded(|| Ok(self.inner.name))
    }

    /// Tesseract's version.
    #[getter]
    fn version(&self) -> PyResult<String> {
        guarded(|| Ok(self.inner.version.clone()))
    }

    /// The languages loaded.
    #[getter]
    fn languages<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        guarded(|| PyTuple::new(py, &self.inner.languages))
    }

    /// The model directory, if the backend has one.
    #[getter]
    fn tessdata(&self) -> PyResult<Option<OsString>> {
        guarded(|| Ok(self.inner.tessdata.clone().map(PathBuf::into_os_string)))
    }

    /// Language → SHA-256 of its `.traineddata`, as lowercase hex.
    #[getter]
    fn model_sha256<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        guarded(|| {
            let dict = PyDict::new(py);
            for (lang, digest) in &self.inner.model_sha256 {
                let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
                dict.set_item(lang, hex)?;
            }
            Ok(dict)
        })
    }

    /// `"vendored"` or `"system"`.
    #[getter]
    fn linkage(&self) -> PyResult<&'static str> {
        guarded(|| {
            Ok(match self.inner.linkage {
                ocr::Linkage::Vendored => "vendored",
                ocr::Linkage::System => "system",
            })
        })
    }

    /// `"in_process"` or `"process"`.
    #[getter]
    fn isolation(&self) -> PyResult<&'static str> {
        guarded(|| Ok(isolation_name(self.inner.isolation)))
    }

    fn __repr__(&self) -> PyResult<String> {
        guarded(|| {
            Ok(format!(
                "<pdf_extract.OcrInfo name='{}' version='{}' languages={:?} isolation='{}'>",
                self.inner.name,
                self.inner.version,
                self.inner.languages.join("+"),
                isolation_name(self.inner.isolation)
            ))
        })
    }
}

/// The engine `Page.ocr()` uses when given none, and `default_ocr_engine()`
/// returns: this binding's own (internal #53), process-isolated, with the helper
/// looked up in the package directory. Rust's `default_ocr_engine()` stays
/// in-process and is not used from Python.
///
/// Built once per process on first use, from the defaults and
/// `TESSDATA_PREFIX`; a failure (`Err`) is stored and raised again on every
/// later call, as in Rust (O12). A panic during construction propagates
/// out of `get_or_init` and leaves the cell empty, so the next call builds
/// again (ADR 35). The cell is one more lock a `fork()` can strand while
/// another thread is inside the first construction; the fork rules are in
/// the `Forking` section of the `pdf-extract-ocr` crate docs.
pub(crate) fn default_engine(py: Python<'_>) -> PyResult<ocr::OcrEngine> {
    static DEFAULT: OnceLock<Result<ocr::OcrEngine, OcrUnavailable>> = OnceLock::new();
    // Once built, no package-directory lookup (an import and a
    // canonicalize) on every `Page.ocr()`.
    let built = match DEFAULT.get() {
        Some(built) => built.clone(),
        None => {
            // Needs the interpreter, so it is resolved before detaching.
            let cfg = ocr::OcrConfig {
                isolation: ocr::Isolation::Process,
                helper_dir: package_dir(py),
                ..ocr::OcrConfig::default()
            };
            let dir = cfg.helper_dir.clone();
            py.detach(|| {
                DEFAULT
                    .get_or_init(|| {
                        ocr::OcrEngine::tesseract(cfg).map_err(|e| stage_hint(e, dir.as_deref()))
                    })
                    .clone()
            })
        }
    };
    built.map_err(|e| ocr_unavailable(py, e))
}

/// The process-wide engine, built on first use from the defaults
/// (process isolation, with the helper beside this package) and
/// `TESSDATA_PREFIX`; a failure is stored and raised again on every call
/// (`OcrUnavailableError`).
///
/// Forking: after it was built, ``os.fork()`` is safe (the child reuses the
/// engine and rebuilds the pool); a fork while another thread is building
/// it, or is inside OCR, deadlocks the child. That includes
/// ``multiprocessing`` with the ``fork`` start method: call this once
/// before starting workers, or use the ``spawn`` or ``forkserver`` start
/// method (see ``OcrEngine``).
#[pyfunction]
pub(crate) fn default_ocr_engine(py: Python<'_>) -> PyResult<OcrEngine> {
    guarded(|| default_engine(py).map(OcrEngine::wrap))
}

/// One `_scripted_engine` step, from its tuple form.
fn step(value: &Bound<'_, PyAny>) -> PyResult<Step> {
    let bad = || {
        PyTypeError::new_err(format!(
            "a scripted step is ('text', str), ('empty',), ('recognition', str), \
             ('timeout',), ('panic', str) or ('sleep', seconds, step), not {value}"
        ))
    };
    let tuple = value.cast::<PyTuple>().map_err(|_| bad())?;
    let kind: String = tuple.get_item(0).map_err(|_| bad())?.extract()?;
    let arg = |i: usize| tuple.get_item(i).map_err(|_| bad());
    Ok(match kind.as_str() {
        "text" => {
            let text: String = arg(1)?.extract()?;
            // Synthetic word boxes in raster pixels: 30 px per line, 100 px
            // per word. Only the text matters to the tests.
            let lines: Vec<Vec<(&str, [u32; 4])>> = text
                .lines()
                .zip(0u32..)
                .map(|(line, y)| {
                    line.split_whitespace()
                        .zip(0u32..)
                        .map(|(word, x)| {
                            let (l, t) = (10 + 100 * x, 10 + 30 * y);
                            (word, [l, t, l + 90, t + 20])
                        })
                        .collect()
                })
                .collect();
            let refs: Vec<&[(&str, [u32; 4])]> = lines.iter().map(Vec::as_slice).collect();
            Step::Output(output(&refs))
        }
        "empty" => Step::empty(),
        "recognition" => Step::recognition(&arg(1)?.extract::<String>()?),
        "timeout" => Step::timeout(),
        "panic" => Step::Panic(arg(1)?.extract()?),
        "sleep" => {
            let secs: f64 = arg(1)?.extract()?;
            let d = Duration::try_from_secs_f64(secs)
                .map_err(|_| PyValueError::new_err(format!("bad sleep duration {secs}")))?;
            Step::Sleep(d, Box::new(step(&arg(2)?)?))
        }
        _ => return Err(bad()),
    })
}

/// Test hook (run 0002 §4.11): an engine whose recognitions follow a script
/// instead of running Tesseract. Pages are still rendered for real; each
/// recognition takes the next of `steps`, then `default` (an empty result
/// when `None`) once they run out. Recognitions go through the production
/// pool, bounded by `max_instances`; the first `fail_creates` pool instances
/// fail to start (`OcrUnavailableError`). The engine's noise pre-check runs
/// as in a real engine unless `noise_precheck` is false.
#[pyfunction]
#[pyo3(signature = (
    steps = Vec::new(),
    *,
    default = None,
    dpi = 300,
    page_timeout = 60.0,
    max_instances = 1,
    fail_creates = 0,
    noise_precheck = true,
))]
#[allow(
    clippy::needless_pass_by_value,
    reason = "PyO3 extracts arguments by value"
)]
pub(crate) fn _scripted_engine(
    steps: Vec<Bound<'_, PyAny>>,
    default: Option<Bound<'_, PyAny>>,
    dpi: i128,
    page_timeout: f64,
    max_instances: i128,
    fail_creates: usize,
    noise_precheck: bool,
) -> PyResult<OcrEngine> {
    guarded(|| {
        let backend = match &default {
            Some(d) => ScriptedBackend::with_default(step(d)?),
            None => ScriptedBackend::new(),
        };
        for s in &steps {
            backend.push(step(s)?);
        }
        backend.fail_next_creates(fail_creates);
        let cfg = ocr::OcrConfig {
            dpi: self::dpi(dpi)?,
            page_timeout: timeout(page_timeout)?,
            max_instances: positive("max_instances", max_instances)?,
            noise_precheck,
            ..ocr::OcrConfig::default()
        };
        let inner = ocr::OcrEngine::with_pool(Arc::new(backend.clone()), cfg);
        Ok(OcrEngine {
            inner,
            scripted: Some(backend),
        })
    })
}

/// Test hook: the counters of a `_scripted_engine` (`calls`,
/// `max_in_flight`, `creates`, `clears`, `drops`).
#[pyfunction]
pub(crate) fn _scripted_stats<'py>(
    py: Python<'py>,
    engine: PyRef<'_, OcrEngine>,
) -> PyResult<Bound<'py, PyDict>> {
    guarded(|| {
        let Some(b) = &engine.scripted else {
            return Err(PyValueError::new_err("not a scripted engine"));
        };
        let dict = PyDict::new(py);
        dict.set_item("calls", b.calls())?;
        dict.set_item("max_in_flight", b.max_in_flight())?;
        dict.set_item("creates", b.creates())?;
        dict.set_item("clears", b.clears())?;
        dict.set_item("drops", b.drops())?;
        Ok(dict)
    })
}

/// Test hook: replaces the list of known install locations searched for
/// models by engines built afterwards (the default engine included, if it
/// has not been built yet); `None` restores the real list.
/// `TESSDATA_PREFIX` is still honoured: its value is read now, from the
/// environment, as an engine would read it.
#[pyfunction]
pub(crate) fn _set_tessdata_candidates(known: Option<Vec<PathBuf>>) -> PyResult<()> {
    guarded(|| {
        ocr::set_candidates_override(known.map(|known| ocr::Candidates {
            known,
            ..ocr::Candidates::from_env()
        }));
        Ok(())
    })
}
