//! Finding the model directory (spec 0002 O9, §7.3).
//!
//! Resolution is a fixed, ordered list of candidates checked with `stat`
//! only — no subprocess, no shell, no `PATH` search: the explicit
//! `OcrConfig::tessdata`, then `$TESSDATA_PREFIX` in both of its common
//! forms (`$P` and `$P/tessdata`), then the known install locations. The
//! first directory holding `<first language>.traineddata` is chosen; the
//! other languages must then be in *that* directory, so a missing one
//! names the chosen directory rather than silently falling through to a
//! later candidate that may hold a different model version.
//!
//! The environment and the known list are captured in [`Candidates`] so
//! tests (and the Python binding's `_set_tessdata_candidates`) can inject
//! them: [`set_candidates_override`] replaces what [`Candidates::current`]
//! returns for the default engine. Every resolution bumps a counter
//! ([`resolutions`]) so a test can prove that a stored failure is not
//! resolved again.

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{PoisonError, RwLock};

use pdf_extract_core::{OcrUnavailable, UnavailableReason};
use pdf_extract_tesseract_sys as sys;

/// The known install locations, in order (O9 item 3; D-3 appended the
/// MacPorts directory). Defined in `pdf-extract-tesseract-sys` so that crate's
/// own tests use the same list.
pub const KNOWN_DIRS: [&str; 7] = sys::TESSDATA_KNOWN_DIRS;

/// The environment-dependent inputs of resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidates {
    /// The value of `TESSDATA_PREFIX`, if set and non-empty.
    pub prefix: Option<PathBuf>,
    /// The known directories, in order.
    pub known: Vec<PathBuf>,
}

/// Test-only replacement for the process's own candidates. A `fork()` while
/// another thread holds this lock leaves it locked in the child; see the
/// [crate-level `Forking` section](crate#forking).
static OVERRIDE: RwLock<Option<Candidates>> = RwLock::new(None);
static RESOLUTIONS: AtomicUsize = AtomicUsize::new(0);

impl Candidates {
    /// The process's own: `TESSDATA_PREFIX` and [`KNOWN_DIRS`].
    pub fn from_env() -> Self {
        Self {
            prefix: prefix_from(std::env::var_os("TESSDATA_PREFIX")),
            known: KNOWN_DIRS.iter().map(PathBuf::from).collect(),
        }
    }

    /// The override if one is set, else [`Candidates::from_env`].
    pub fn current() -> Self {
        OVERRIDE
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_else(Self::from_env)
    }

    /// The ordered directories the engine would try with no explicit
    /// `tessdata` option. Test support: the test suites look for the model
    /// with the library's own order, so a change to it cannot leave them
    /// resolving a different model than the engine does.
    #[doc(hidden)]
    pub fn ordered(&self) -> Vec<PathBuf> {
        self.list(None)
    }

    /// The ordered candidate list for an explicit directory `explicit`.
    fn list(&self, explicit: Option<&Path>) -> Vec<PathBuf> {
        let mut out = Vec::with_capacity(3 + self.known.len());
        out.extend(explicit.map(Path::to_path_buf));
        if let Some(p) = &self.prefix {
            out.push(p.clone());
            out.push(p.join("tessdata"));
        }
        out.extend(self.known.iter().cloned());
        out
    }
}

/// An empty `TESSDATA_PREFIX` is treated as unset, as Tesseract does.
fn prefix_from(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Replaces the candidates [`Candidates::current`] returns (`None` restores
/// the environment). For tests and the Python binding's test hook; it
/// affects engines built afterwards, never an existing one.
#[doc(hidden)]
pub fn set_candidates_override(candidates: Option<Candidates>) {
    *OVERRIDE.write().unwrap_or_else(PoisonError::into_inner) = candidates;
}

/// How many resolutions this process has run (test observability for O12).
#[doc(hidden)]
pub fn resolutions() -> usize {
    RESOLUTIONS.load(Ordering::Relaxed)
}

/// A resolved model directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// The chosen directory.
    pub dir: PathBuf,
    /// The candidates tried before it (and rejected), in order.
    pub searched: Vec<PathBuf>,
}

/// The model file of `lang` in `dir`.
pub(crate) fn model_path(dir: &Path, lang: &str) -> PathBuf {
    dir.join(format!("{lang}.traineddata"))
}

/// Whether `path` is a regular file (following symlinks).
fn is_regular_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// The `NoTessdata` detail (spec 0003 F22): the tested prefix, then what
/// was tried, so the hint never tells a caller to set something they
/// already set. "The tessdata option" reads correctly from Rust
/// (`OcrConfig.tessdata`) and Python (`OcrEngine(tessdata=...)`) alike.
fn no_tessdata_detail(first: &str, explicit: Option<&Path>, prefix: Option<&Path>) -> String {
    let mut detail = format!("no directory holds {first}.traineddata");
    if let Some(p) = explicit {
        detail.push_str(&format!("; the tessdata setting {} lacks it", p.display()));
    }
    if let Some(p) = prefix {
        let p = p.display();
        detail.push_str(&format!("; TESSDATA_PREFIX={p} and {p}/tessdata lack it"));
    }
    if explicit.is_none() && prefix.is_none() {
        detail.push_str("; set the tessdata option or TESSDATA_PREFIX");
    }
    detail
}

/// Runs O9 for `languages` (already validated: non-empty, each a plain
/// name). `searched` holds the candidates that were tried and did not hold
/// the first language; on success the chosen directory is
/// [`Resolved::dir`], and on failure every candidate is listed.
pub(crate) fn resolve(
    explicit: Option<&Path>,
    candidates: &Candidates,
    languages: &[String],
) -> Result<Resolved, OcrUnavailable> {
    RESOLUTIONS.fetch_add(1, Ordering::Relaxed);
    let first = languages.first().map_or("eng", String::as_str).to_owned();
    let mut searched = Vec::new();
    let mut chosen = None;
    for dir in candidates.list(explicit) {
        if dir.is_dir() && is_regular_file(&model_path(&dir, &first)) {
            chosen = Some(dir);
            break;
        }
        searched.push(dir);
    }
    let unavailable = |reason, tessdata: Option<PathBuf>, searched, detail| OcrUnavailable {
        reason,
        tessdata,
        languages: languages.to_vec(),
        searched,
        detail,
    };
    let Some(dir) = chosen else {
        return Err(unavailable(
            UnavailableReason::NoTessdata,
            None,
            searched,
            no_tessdata_detail(&first, explicit, candidates.prefix.as_deref()),
        ));
    };
    // O7 (2): every model must be a readable regular file in the chosen
    // directory. Opening (not only `stat`) catches a permission problem
    // here, with the file named, rather than as Tesseract's vaguer message.
    for lang in languages {
        let path = model_path(&dir, lang);
        if !is_regular_file(&path) || File::open(&path).is_err() {
            return Err(unavailable(
                UnavailableReason::LanguageDataMissing,
                Some(dir.clone()),
                searched,
                format!(
                    "{} is missing or unreadable in the chosen directory {}",
                    path.file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                    dir.display()
                ),
            ));
        }
    }
    Ok(Resolved { dir, searched })
}

#[cfg(test)]
#[path = "tessdata_tests.rs"]
mod tests;
