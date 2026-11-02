//! Getting the bytes and deciding whether they are a PDF (spec T1-T3, §5.1,
//! §5.2 of the design spec).
//!
//! The whole document is read into one `Arc<Vec<u8>>` (D-15: hayro's
//! `PdfData` needs a sized `T` behind the `Arc`, so `Arc<[u8]>` does not
//! work). Reading up front, rather than memory-mapping, means the file
//! descriptor is closed before open returns and a file truncated or rewritten
//! by another process later cannot fault the reader (mmap is deferred, D-20).
//!
//! Detection is content-only: the name and extension are never consulted, so
//! a PDF named `.txt` opens and a text file named `.pdf` does not.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::error::OpenError;

/// How far into the input the `%PDF-` header may be (F2; Q6 kept 1024 even
/// though the oracle accepts a header at 1500).
pub(crate) const HEADER_WINDOW: usize = 1024;

/// Reads the file at `path` whole, refusing it before reading when it is not
/// a regular file or its metadata says it is larger than `max_size`.
///
/// Non-regular paths (directory, FIFO, socket, device) are refused from
/// `stat` on the path, *before* opening: opening a FIFO for reading blocks
/// until a writer appears, and a device can be read forever. Symlinks are
/// followed, so a link to a regular file opens. The check is repeated with
/// `fstat` on the open descriptor, which also supplies the size, so the size
/// describes the file actually read. A path swapped between the two checks
/// is a documented residual (spec §2.4, TOCTOU at open).
///
/// A file that grows after the check is still capped: the read takes at
/// most `max_size + 1` bytes and a file found larger at that point is
/// refused the same way.
pub(crate) fn read_file(path: &Path, max_size: u64) -> Result<Vec<u8>, OpenError> {
    let not_found = |e: io::Error| match e.kind() {
        io::ErrorKind::NotFound => OpenError::NotFound(path.to_path_buf()),
        _ => OpenError::Io(e),
    };
    require_regular(&std::fs::metadata(path).map_err(not_found)?)?;
    let file = File::open(path).map_err(not_found)?;
    let meta = file.metadata()?;
    require_regular(&meta)?;
    let size = meta.len();
    check_size(size, max_size)?;
    // The metadata size is a capacity hint only; a lying or growing file is
    // handled by the `take` below. Saturate rather than fail on 32-bit
    // targets, where the size check above already bounds it far lower.
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(usize::MAX));
    file.take(max_size.saturating_add(1))
        .read_to_end(&mut bytes)?;
    // `file` was moved into `take` and is dropped here, closing the
    // descriptor before parsing starts (T1).
    check_size(bytes.len() as u64, max_size)?;
    Ok(bytes)
}

/// `Io(InvalidInput, "not a regular file")` unless `meta` is a regular file.
fn require_regular(meta: &std::fs::Metadata) -> Result<(), OpenError> {
    if meta.is_file() {
        return Ok(());
    }
    Err(OpenError::Io(io::Error::new(
        io::ErrorKind::InvalidInput,
        "not a regular file",
    )))
}

/// `TooLarge` when `size` exceeds `max_size`.
pub(crate) fn check_size(size: u64, max_size: u64) -> Result<(), OpenError> {
    if size > max_size {
        return Err(OpenError::TooLarge {
            size,
            limit: max_size,
        });
    }
    Ok(())
}

/// Content-only file-type detection (T3): empty input is `Empty`; input with
/// no complete `%PDF-` tag inside its first [`HEADER_WINDOW`] bytes is
/// `NotPdf`; anything else goes to the parser.
///
/// "Inside" means the whole five-byte tag: a header starting at byte 1020
/// spills past the window and is not accepted.
pub(crate) fn detect(bytes: &[u8]) -> Result<(), OpenError> {
    if bytes.is_empty() {
        return Err(OpenError::Empty);
    }
    let window = &bytes[..bytes.len().min(HEADER_WINDOW)];
    if memchr::memmem::find(window, b"%PDF-").is_none() {
        return Err(OpenError::NotPdf);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_window_boundary() {
        let at = |offset: usize| {
            let mut v = vec![b'x'; offset];
            v.extend_from_slice(b"%PDF-1.7\n");
            v
        };
        assert!(detect(&at(0)).is_ok());
        assert!(detect(&at(HEADER_WINDOW - 5)).is_ok());
        assert!(matches!(
            detect(&at(HEADER_WINDOW - 4)),
            Err(OpenError::NotPdf)
        ));
        assert!(matches!(detect(&at(1500)), Err(OpenError::NotPdf)));
        assert!(matches!(detect(b""), Err(OpenError::Empty)));
        // A short input that is only a header is still handed to the parser.
        assert!(detect(b"%PDF-").is_ok());
    }

    #[test]
    fn size_check() {
        assert!(check_size(5, 5).is_ok());
        assert!(matches!(
            check_size(6, 5),
            Err(OpenError::TooLarge { size: 6, limit: 5 })
        ));
    }

    #[test]
    fn a_directory_is_not_a_regular_file() {
        let dir = std::env::temp_dir();
        match read_file(&dir, u64::MAX) {
            Err(OpenError::Io(e)) => {
                assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
                assert_eq!(e.to_string(), "not a regular file");
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }
}
