//! Finding the helper binary (spec 0002 O28, D-16).
//!
//! Three places, in order, and nothing else — in particular never `PATH`,
//! so the binary that runs is one the deployment put there on purpose:
//!
//! 1. `$PDF_EXTRACT_OCR_HELPER`, which must be an absolute path (a relative
//!    one is refused, not resolved against the working directory);
//! 2. `OcrConfig::helper_dir` (the Python package directory, where the
//!    wheel installs the helper beside the extension module);
//! 3. the directory of the current executable, for Rust programs that ship
//!    the helper beside themselves.
//!
//! The first that applies is final: a configured location that lacks the
//! helper is an error naming it, not a fall-through to the next one. Every
//! source must yield an absolute path — a relative `helper_dir` is refused
//! like a relative variable — so the program spawned always contains a
//! `/` and `exec` never searches its default path. The
//! candidate must be an executable regular file (symbolic links are
//! followed). The check is made once, at engine construction (fail fast);
//! the binary vanishing afterwards surfaces when a helper is respawned.

use std::ffi::OsString;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The helper's file name.
pub const HELPER_NAME: &str = "pdf-extract-ocr-helper";
/// The variable naming the helper explicitly.
pub const HELPER_ENV: &str = "PDF_EXTRACT_OCR_HELPER";

/// Locates the helper for an engine configured with `helper_dir`, reading
/// the process environment and executable path.
///
/// # Errors
/// Why no usable helper was found (the caller reports `EngineNotBuilt`).
pub(crate) fn locate(helper_dir: Option<&Path>) -> Result<PathBuf, String> {
    locate_with(
        std::env::var_os(HELPER_ENV),
        helper_dir,
        std::env::current_exe(),
    )
}

/// [`locate`] over explicit inputs.
pub(crate) fn locate_with(
    env: Option<OsString>,
    helper_dir: Option<&Path>,
    exe: io::Result<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(value) = env.filter(|v| !v.is_empty()) {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(format!(
                "{HELPER_ENV} must be an absolute path, not {}",
                path.display()
            ));
        }
        return checked(path, HELPER_ENV);
    }
    if let Some(dir) = helper_dir {
        // Refused, not resolved: a relative directory (including the empty
        // one, which would leave a bare file name) would be checked against
        // the working directory but run through exec's own search, and
        // would re-resolve against whatever the working directory is when
        // a helper is respawned.
        if !dir.is_absolute() {
            return Err(format!(
                "helper_dir must be an absolute path, not {:?}",
                dir.display().to_string()
            ));
        }
        return checked(dir.join(HELPER_NAME), "helper_dir");
    }
    let exe = exe.map_err(|e| format!("cannot find the current executable: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", exe.display()))?;
    checked(dir.join(HELPER_NAME), "the executable's directory")
}

/// `path` if it is an absolute path to an executable regular file; else
/// why not. Absolute is the invariant every source must meet: the spawned
/// program name then contains a `/`, so `exec` runs exactly the file
/// checked here and never searches (O28, D-16).
fn checked(path: PathBuf, from: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!(
            "OCR helper path {} (from {from}) is not absolute",
            path.display()
        ));
    }
    let meta = std::fs::metadata(&path).map_err(|e| {
        format!(
            "OCR helper not found at {} (from {from}): {e}",
            path.display()
        )
    })?;
    if !meta.is_file() {
        return Err(format!(
            "OCR helper at {} (from {from}) is not a regular file",
            path.display()
        ));
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(format!(
            "OCR helper at {} (from {from}) is not executable",
            path.display()
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pxo-discovery-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn file(dir: &Path, mode: u32) -> PathBuf {
        let p = dir.join(HELPER_NAME);
        std::fs::write(&p, b"#!/bin/sh\n").expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).expect("chmod");
        p
    }

    fn no_exe() -> io::Result<PathBuf> {
        Err(io::Error::other("no exe"))
    }

    #[test]
    fn an_absolute_variable_wins_over_helper_dir() {
        let a = scratch("env-a");
        let b = scratch("env-b");
        let want = file(&a, 0o755);
        file(&b, 0o755);
        let got = locate_with(Some(want.clone().into()), Some(&b), no_exe());
        assert_eq!(got, Ok(want));
    }

    #[test]
    fn a_relative_variable_is_refused_not_searched() {
        let dir = scratch("rel");
        file(&dir, 0o755);
        let err =
            locate_with(Some("relative/path".into()), Some(&dir), no_exe()).expect_err("relative");
        assert!(err.contains("absolute"), "{err}");
        let err =
            locate_with(Some(HELPER_NAME.into()), Some(&dir), no_exe()).expect_err("bare name");
        assert!(err.contains("absolute"), "{err}");
    }

    #[test]
    fn an_empty_or_relative_helper_dir_is_refused_not_searched() {
        // A helper in the working directory must not be found through a
        // relative helper_dir (it would be checked here but exec'd through
        // the default search path).
        let dir = scratch("rel-dir");
        let exe = file(&dir, 0o755);
        for rel in ["", ".", "rel-dir", "./sub/dir"] {
            let err = locate_with(None, Some(Path::new(rel)), Ok(exe.clone()))
                .expect_err("relative helper_dir");
            assert!(
                err.contains("helper_dir must be an absolute path"),
                "{rel:?}: {err}"
            );
        }
        // An executable path without a directory cannot slip through either.
        let err = locate_with(None, None, Ok(PathBuf::from("prog"))).expect_err("bare exe");
        assert!(
            err.contains("not absolute") || err.contains("no parent"),
            "{err}"
        );
        let err = checked(PathBuf::from(HELPER_NAME), "test").expect_err("bare name");
        assert!(err.contains("not absolute"), "{err}");
    }

    #[test]
    fn non_executable_missing_and_directory_candidates_are_refused() {
        let dir = scratch("modes");
        let p = file(&dir, 0o644);
        let err = locate_with(Some(p.into()), None, no_exe()).expect_err("0644");
        assert!(err.contains("not executable"), "{err}");
        let err =
            locate_with(Some(dir.join("absent").into()), None, no_exe()).expect_err("missing");
        assert!(err.contains("not found"), "{err}");
        let sub = dir.join("sub");
        std::fs::create_dir_all(sub.join(HELPER_NAME)).expect("mkdir");
        let err = locate_with(None, Some(&sub), no_exe()).expect_err("dir");
        assert!(err.contains("not a regular file"), "{err}");
    }

    #[test]
    fn helper_dir_is_final_and_the_executable_directory_is_last() {
        let dir = scratch("exe");
        let want = file(&dir, 0o700);
        let empty = scratch("exe-empty");
        // An empty variable counts as unset.
        assert_eq!(
            locate_with(Some(OsString::new()), None, Ok(dir.join("prog"))),
            Ok(want.clone())
        );
        assert_eq!(locate_with(None, Some(&dir), no_exe()), Ok(want));
        // helper_dir without the helper does not fall through to the exe.
        let err = locate_with(None, Some(&empty), Ok(dir.join("prog"))).expect_err("final");
        assert!(err.contains("helper_dir"), "{err}");
        let err = locate_with(None, None, no_exe()).expect_err("no exe");
        assert!(err.contains("current executable"), "{err}");
        let err = locate_with(None, None, Ok(PathBuf::from("/"))).expect_err("root");
        assert!(err.contains("no parent"), "{err}");
    }
}
