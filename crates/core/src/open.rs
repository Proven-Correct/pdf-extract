//! The open pipeline (spec T1-T8, T14-T15, §3.3 signal sampling, D-31).
//!
//! Turns validated bytes into a [`Loaded`] document plus the facts that
//! outlive it (page count, encryption flag, open-time warnings). The steps,
//! in order, each with the error it can produce:
//!
//! 1. size against `max_file_size` (`TooLarge`) and content detection
//!    (`Empty`, `NotPdf`);
//! 2. hayro's `Pdf::new_with_options` with the decode limits (`Corrupt`,
//!    `PasswordRequired`, `UnsupportedEncryption`);
//! 3. `max_objects` against the parsed xref (`Limit { Objects }`);
//! 4. the page-tree walk, and the brute-force scan when it found nothing
//!    (`TooManyPages`);
//! 5. hayro's side-channel signals turned into warnings.
//!
//! The encryption flag is hayro's `Pdf::is_encrypted()` (fork change H9,
//! D-35): whether the parse installed a decryptor. It is decided by the
//! parsed `/Encrypt` entry, so `#xx`-escaped names (`/Encr#79pt`) count.
//!
//! **Signals (D-12, spec §3.3).** hayro reports three things out of band:
//! the object-nesting cap through a *thread-local* hit counter, the decode
//! cap through the document's `decode_stats()`, and xref repair through
//! `xref_was_repaired()`. The counter is read before step 2 and after step
//! 4; that is only valid because everything between runs synchronously on
//! the calling thread, which this module must keep true (no work may be
//! handed to another thread inside the window). Repair is lazy in hayro (the
//! first failed lookup can trigger it), so it is read after step 2 *and*
//! again after the walk.

use std::sync::Arc;

use hayro_syntax::{DecodeLimits, DecryptionError, LoadPdfError, Pdf, object_depth_limit_hits};

use crate::document::{Loaded, OpenOptions};
use crate::error::OpenError;
use crate::index::{self, WalkLimits};
use crate::limits::LimitKind;
use crate::source;
use crate::warning::{Warning, WarningKind, WarningSink};

/// What an open produced: the parsed document and the facts that stay
/// readable after `close()`.
pub(crate) struct Opened {
    pub(crate) loaded: Loaded,
    pub(crate) page_count: usize,
    pub(crate) is_encrypted: bool,
    pub(crate) warnings: Vec<Warning>,
}

/// Opens `bytes` with `opts`; see the module docs for the steps.
pub(crate) fn open(bytes: Arc<Vec<u8>>, opts: &OpenOptions) -> Result<Opened, OpenError> {
    let limits = &opts.limits;
    source::check_size(bytes.len() as u64, limits.max_file_size)?;
    source::detect(&bytes)?;

    let decode_limits = decode_limits(limits);
    let mut sink = WarningSink::new(None);

    // --- signal window: synchronous, this thread only (module docs) ---
    let depth_hits_before = object_depth_limit_hits();
    let pdf = Pdf::new_with_options(bytes.clone(), "", decode_limits).map_err(map_load_error)?;
    note_repair(&pdf, &mut sink);
    let is_encrypted = pdf.is_encrypted();

    check_objects(&pdf, limits.max_objects)?;

    let walk_limits = WalkLimits {
        max_pages: opts.max_pages,
        max_depth: limits.max_page_tree_depth,
    };
    let outcome = index::walk(&pdf.xref(), pdf.pages_root(), walk_limits, &mut sink)?;
    let index = if outcome.index.is_empty() && !outcome.depth_skipped {
        index::brute_force(&pdf, opts.max_pages, &mut sink)?
    } else {
        outcome.index
    };

    note_repair(&pdf, &mut sink);
    // A lazy repair during the walk rebuilds the xref from a scan of the
    // file, which can find far more objects than the table listed. (A
    // repair during a later page operation can still grow it; that cost is
    // bounded by the file size, which `max_file_size` bounds.)
    check_objects(&pdf, limits.max_objects)?;
    let depth_hits = object_depth_limit_hits().saturating_sub(depth_hits_before);
    // --- end of signal window ---
    sink.push_n(WarningKind::ObjectDepth, depth_hits, || {
        "nesting limit: an over-deep object was treated as missing".into()
    });
    let over_limit = pdf.decode_stats().streams_over_limit;
    sink.push_n(WarningKind::StreamDecodeLimit, over_limit, || {
        format!(
            "decoded size limit: a stream over {} bytes was treated as absent",
            limits.max_stream_decoded_bytes
        )
    });

    let page_count = index.len();
    Ok(Opened {
        loaded: Loaded {
            source: bytes,
            pdf,
            index,
        },
        page_count,
        is_encrypted,
        warnings: sink.finish(),
    })
}

/// The hayro decode limits a document is opened with. The image pixel cap
/// (H13) lives here rather than in the interpreter settings because hayro
/// checks it where an image's declared dimensions and the document's limits
/// meet, before any filter runs; only rendering decodes images, but every
/// document carries the cap so a renderer cannot forget it.
pub(crate) fn decode_limits(limits: &crate::limits::Limits) -> DecodeLimits {
    DecodeLimits {
        max_stream_decoded_bytes: limits.max_stream_decoded_bytes,
        max_image_pixels: Some(limits.max_image_pixels),
    }
}

/// T7: `Limit { Objects }` when the xref has more than `max` entries.
fn check_objects(pdf: &Pdf, max: usize) -> Result<(), OpenError> {
    let objects = pdf.len();
    if objects > max {
        return Err(OpenError::Limit {
            kind: LimitKind::Objects,
            value: objects as u64,
        });
    }
    Ok(())
}

/// Records `XrefRepaired` once if hayro has rebuilt the xref by now.
fn note_repair(pdf: &Pdf, sink: &mut WarningSink) {
    if pdf.xref_was_repaired() && !sink.has(WarningKind::XrefRepaired) {
        sink.push(WarningKind::XrefRepaired, || "repaired".into());
    }
}

/// T4: hayro's load errors to ours.
fn map_load_error(e: LoadPdfError) -> OpenError {
    match e {
        LoadPdfError::Invalid => OpenError::Corrupt(
            "the cross-reference table, trailer or catalog could not be read".into(),
        ),
        LoadPdfError::Decryption(DecryptionError::PasswordProtected) => OpenError::PasswordRequired,
        LoadPdfError::Decryption(DecryptionError::MissingIDEntry) => {
            OpenError::UnsupportedEncryption("the trailer has no /ID entry".into())
        }
        LoadPdfError::Decryption(DecryptionError::InvalidEncryption) => {
            OpenError::UnsupportedEncryption("the encryption dictionary is invalid".into())
        }
        LoadPdfError::Decryption(DecryptionError::UnsupportedAlgorithm) => {
            OpenError::UnsupportedEncryption("the encryption algorithm is not supported".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_errors_map_per_t4() {
        assert!(matches!(
            map_load_error(LoadPdfError::Invalid),
            OpenError::Corrupt(_)
        ));
        assert!(matches!(
            map_load_error(LoadPdfError::Decryption(DecryptionError::PasswordProtected)),
            OpenError::PasswordRequired
        ));
        for e in [
            DecryptionError::MissingIDEntry,
            DecryptionError::InvalidEncryption,
            DecryptionError::UnsupportedAlgorithm,
        ] {
            assert!(matches!(
                map_load_error(LoadPdfError::Decryption(e)),
                OpenError::UnsupportedEncryption(_)
            ));
        }
    }

    fn open(bytes: Vec<u8>) -> crate::Document {
        crate::Document::from_bytes(bytes, &crate::OpenOptions::default()).expect("opens")
    }

    #[test]
    fn the_encryption_flag_sees_through_name_escapes() {
        assert!(!open(pdf_extract_testkit::contract::header_at_offset(0)).is_encrypted());
        // Mentioning /Encrypt somewhere harmless does not make a file
        // encrypted.
        let mut b = pdf_extract_testkit::PdfBuilder::new();
        b.add_page_a4().raw_content("% /Encrypt in a comment");
        assert!(!open(b.to_bytes()).is_encrypted());
        // An encrypted file whose trailer spells the key with an escape: a
        // byte search for `/Encrypt` would miss it, the parsed trailer does
        // not.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/corpus-c/enc_aes_128_owner.pdf"
        );
        let mut bytes = std::fs::read(path).expect("fixture");
        let mut n = 0;
        while let Some(at) = memchr::memmem::find(&bytes, b"/Encrypt") {
            bytes.splice(at..at + 8, b"/Encr#79pt".iter().copied());
            n += 1;
        }
        assert!(n > 0);
        assert!(memchr::memmem::find(&bytes, b"/Encrypt").is_none());
        assert!(open(bytes).is_encrypted());
    }

    #[test]
    fn the_decode_limits_carry_both_caps() {
        // Spec §4.4 (core, before the renderer exists): what `open` passes to
        // `Pdf::new_with_options` carries the image pixel cap.
        let limits = crate::Limits {
            max_image_pixels: 1_000_000,
            max_stream_decoded_bytes: 4096,
            ..crate::Limits::default()
        };
        assert_eq!(
            decode_limits(&limits),
            DecodeLimits {
                max_stream_decoded_bytes: 4096,
                max_image_pixels: Some(1_000_000),
            }
        );
        assert_eq!(
            decode_limits(&crate::Limits::default()).max_image_pixels,
            Some(256_000_000)
        );
    }
}
