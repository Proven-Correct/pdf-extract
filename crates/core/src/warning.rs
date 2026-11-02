//! Non-fatal findings (spec §3.3 `warning.rs`, §2.2).
//!
//! A malformed part of a document that the library worked around is reported
//! as a [`Warning`] rather than an error: open-time warnings hang off the
//! [`Document`](crate::Document), per-page ones off the page's output (units
//! 09-10). The [`WarningKind`] is the stable, testable part; the detail text
//! is for humans and may change.
//!
//! Warnings are collected through [`WarningSink`], which keeps one warning per
//! kind. A hostile page tree can repeat the same defect once per `/Kids`
//! entry (millions of times in a file of a few megabytes); one warning per
//! occurrence would make the warning list an amplification vector, so
//! repeats are folded into [`Warning::count`] on the first occurrence (whose
//! detail is kept).

use std::fmt;

/// One thing the library had to work around.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// The page it concerns, or `None` for document-level (open-time) ones.
    pub page: Option<usize>,
    /// What kind of problem it was.
    pub kind: WarningKind,
    /// Human-readable detail of the first occurrence (object numbers,
    /// counts). Not a stable format.
    pub detail: String,
    /// How many times this kind occurred (at least 1); repeats are
    /// coalesced into one warning. Saturates at `u32::MAX`.
    pub count: u32,
}

/// The kinds of [`Warning`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WarningKind {
    /// The cross-reference table was broken and rebuilt by scanning the file.
    XrefRepaired,
    /// A page-tree node was reached a second time (a cycle or a duplicated
    /// `/Kids` entry) and skipped.
    PageTreeRevisited,
    /// A page-tree node was deeper than `Limits::max_page_tree_depth`; its
    /// subtree was skipped.
    PageTreeDepth,
    /// The page tree yielded no pages, so the file was scanned for page
    /// objects instead.
    PageTreeBruteForce,
    /// A page-tree node (or the root) could not be read as a dictionary.
    PageTreeNodeUnreadable,
    /// An object nested deeper than the parser's fixed cap was dropped.
    ObjectDepth,
    /// A stream would have decoded to more than
    /// `Limits::max_stream_decoded_bytes` and was treated as absent.
    StreamDecodeLimit,
    /// A stream failed to decode.
    StreamDecodeFailed,
    /// The content stream had syntax errors; operators were skipped.
    ContentDamaged,
    /// A font could not be loaded and was replaced by a standard font; its
    /// text is kept, but glyph shapes and widths may differ from the
    /// document's.
    FontLoadFailed,
    /// `Limits::max_chars_per_page` was reached; the page is partial.
    CharLimit,
    /// Nested content (forms, Type 3 glyphs, patterns, soft masks) went past
    /// the interpreter's nesting cap and was skipped.
    InterpreterNesting,
    /// A Form XObject drew itself (directly or indirectly); the repeat was
    /// skipped.
    FormXObjectCycle,
    /// A font of a kind the library does not support.
    UnsupportedFont,
    /// An image could not be decoded.
    ImageDecodeFailure,
    /// An image's dimensions, as declared, as its codec read them from its
    /// header or parameters, or as the codec reported them after decoding,
    /// exceed `Limits::max_image_pixels`; it was skipped (rendering only).
    ImagePixelLimit,
    /// An image's codec would have used more memory than the budget
    /// `Limits::max_image_pixels` implies (JBIG2: 2 bytes per pixel of the
    /// cap; JPEG 2000: 64), or a JBIG2 decode would have placed symbol
    /// instances worth more than 16 pixels per pixel of the cap; the image
    /// was skipped and the page otherwise rendered (rendering only).
    ImageMemoryLimit,
    /// A page's text pass ran out of a per-page budget inside
    /// `Document::extract`, which kept going with `""` for that page (run
    /// 0002 O23, Q6). Document-level, but names its page: it displays as
    /// `page N: <the PageError message>` rather than `category: detail`.
    PageLimit,
    /// OCR's noise pre-check found the rendered raster dominated by
    /// speckle and recognised it at half the resolution instead (run 0003
    /// F13, F14).
    OcrNoiseDownsampled,
    /// OCR's noise pre-check found the rendered raster dominated by
    /// speckle at both the rendered and half the resolution, and skipped
    /// recognition: the page's OCR text is empty (run 0003 F13, F14).
    OcrNoiseSkipped,
}

impl WarningKind {
    /// The category word shown before the colon in a warning's text, e.g.
    /// `page tree` in `page tree: node revisited`. Several kinds share one
    /// category; the detail tells them apart.
    pub fn category(self) -> &'static str {
        match self {
            Self::XrefRepaired => "xref",
            Self::PageTreeRevisited
            | Self::PageTreeDepth
            | Self::PageTreeBruteForce
            | Self::PageTreeNodeUnreadable => "page tree",
            Self::ObjectDepth => "object",
            Self::StreamDecodeLimit | Self::StreamDecodeFailed => "stream",
            Self::ContentDamaged => "content",
            Self::FontLoadFailed | Self::UnsupportedFont => "font",
            Self::CharLimit => "text",
            Self::InterpreterNesting => "interpreter",
            Self::FormXObjectCycle => "form",
            Self::ImageDecodeFailure | Self::ImagePixelLimit | Self::ImageMemoryLimit => "image",
            Self::PageLimit => "page",
            Self::OcrNoiseDownsampled | Self::OcrNoiseSkipped => "ocr",
        }
    }
}

impl fmt::Display for Warning {
    /// `"<category>: <detail>"`, e.g. `xref: repaired`, followed by
    /// `" (×N)"` when the warning occurred `N > 1` times. This is the string
    /// the Python binding exposes. The one exception is
    /// [`WarningKind::PageLimit`], which is `"page N: <detail>"` (N the
    /// 0-based index, as in `PageResult::index`): run 0002 O23 asks for the
    /// page number in the text, and a static category cannot carry it.
    ///
    /// The per-page OCR notes `Document::extract` copies into its
    /// document-level list (run 0003 F14) carry the whole page-prefixed text
    /// in their detail (`page N: ocr: ...`), so they display that detail as
    /// is; the same kinds on the page's own `TextPage` display as
    /// `ocr: <detail>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.kind, self.page) {
            (WarningKind::PageLimit, Some(page)) => write!(f, "page {page}: {}", self.detail)?,
            (WarningKind::OcrNoiseDownsampled | WarningKind::OcrNoiseSkipped, Some(page))
                if is_page_note(&self.detail, page) =>
            {
                f.write_str(&self.detail)?;
            }
            _ => write!(f, "{}: {}", self.kind.category(), self.detail)?,
        }
        if self.count > 1 {
            write!(f, " (×{})", self.count)?;
        }
        Ok(())
    }
}

/// Whether `detail` starts with `page <page>: `, the prefix of a per-page
/// note in `Extraction::warnings`.
fn is_page_note(detail: &str, page: usize) -> bool {
    detail
        .strip_prefix("page ")
        .and_then(|rest| rest.strip_prefix(page.to_string().as_str()))
        .is_some_and(|rest| rest.starts_with(": "))
}

/// Collects warnings, one per kind, in order of first occurrence.
#[derive(Debug, Default)]
pub(crate) struct WarningSink {
    page: Option<usize>,
    // (kind, first detail, occurrences). A Vec, not a map: there are at most
    // nineteen kinds and the order of first occurrence is part of the output.
    entries: Vec<(WarningKind, String, u64)>,
}

impl WarningSink {
    /// A sink for warnings about `page` (`None`: document level).
    pub(crate) fn new(page: Option<usize>) -> Self {
        Self {
            page,
            entries: Vec::new(),
        }
    }

    /// Records one occurrence. `detail` is only evaluated for the first
    /// occurrence of a kind, so callers in hot loops pay nothing for repeats.
    pub(crate) fn push(&mut self, kind: WarningKind, detail: impl FnOnce() -> String) {
        self.push_n(kind, 1, detail);
    }

    /// Records `n` occurrences at once (for counters sampled from hayro).
    pub(crate) fn push_n(&mut self, kind: WarningKind, n: u64, detail: impl FnOnce() -> String) {
        if n == 0 {
            return;
        }
        match self.entries.iter_mut().find(|(k, _, _)| *k == kind) {
            Some((_, _, count)) => *count += n,
            None => self.entries.push((kind, detail(), n)),
        }
    }

    /// Whether a warning of `kind` has been recorded.
    pub(crate) fn has(&self, kind: WarningKind) -> bool {
        self.entries.iter().any(|(k, _, _)| *k == kind)
    }

    /// The collected warnings, one per kind, with their counts.
    pub(crate) fn finish(self) -> Vec<Warning> {
        let page = self.page;
        self.entries
            .into_iter()
            .map(|(kind, detail, count)| Warning {
                page,
                kind,
                detail,
                count: u32::try_from(count).unwrap_or(u32::MAX),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_category_colon_detail() {
        let w = Warning {
            page: None,
            kind: WarningKind::XrefRepaired,
            detail: "repaired".into(),
            count: 1,
        };
        assert_eq!(w.to_string(), "xref: repaired");
        let many = Warning { count: 999, ..w };
        assert_eq!(many.to_string(), "xref: repaired (×999)");
    }

    #[test]
    fn page_limit_displays_its_page() {
        let w = Warning {
            page: Some(3),
            kind: WarningKind::PageLimit,
            detail: "limit ops_per_page exceeded".into(),
            count: 1,
        };
        assert_eq!(w.to_string(), "page 3: limit ops_per_page exceeded");
        // Without a page it falls back to the category form.
        let orphan = Warning { page: None, ..w };
        assert_eq!(orphan.to_string(), "page: limit ops_per_page exceeded");
    }

    #[test]
    fn every_kind_has_a_category() {
        use WarningKind::*;
        for k in [
            XrefRepaired,
            PageTreeRevisited,
            PageTreeDepth,
            PageTreeBruteForce,
            PageTreeNodeUnreadable,
            ObjectDepth,
            StreamDecodeLimit,
            StreamDecodeFailed,
            ContentDamaged,
            FontLoadFailed,
            CharLimit,
            InterpreterNesting,
            FormXObjectCycle,
            UnsupportedFont,
            ImageDecodeFailure,
            ImagePixelLimit,
            ImageMemoryLimit,
            PageLimit,
            OcrNoiseDownsampled,
            OcrNoiseSkipped,
        ] {
            assert!(!k.category().is_empty());
        }
        assert_eq!(ImagePixelLimit.category(), "image");
        assert_eq!(ImageMemoryLimit.category(), "image");
        assert_eq!(OcrNoiseDownsampled.category(), "ocr");
        assert_eq!(OcrNoiseSkipped.category(), "ocr");
    }

    #[test]
    fn ocr_noise_notes_display_by_shape() {
        // On the page's own TextPage: category form.
        let on_page = Warning {
            page: Some(3),
            kind: WarningKind::OcrNoiseSkipped,
            detail: "noise-dominated raster skipped (no convincing foreground at 300 or 150 dpi)"
                .into(),
            count: 1,
        };
        assert_eq!(
            on_page.to_string(),
            "ocr: noise-dominated raster skipped (no convincing foreground at 300 or 150 dpi)"
        );
        // In Extraction::warnings: the detail already names the page.
        let note = Warning {
            detail: format!("page 3: {on_page}"),
            ..on_page.clone()
        };
        assert_eq!(
            note.to_string(),
            "page 3: ocr: noise-dominated raster skipped (no convincing foreground at 300 or \
             150 dpi)"
        );
        // A prefix naming another page, or a near miss, keeps the category.
        for detail in ["page 30: x", "page 4: x", "page 3x", "page 3", "pages 3: x"] {
            let w = Warning {
                detail: detail.into(),
                kind: WarningKind::OcrNoiseDownsampled,
                ..on_page.clone()
            };
            assert_eq!(w.to_string(), format!("ocr: {detail}"));
        }
        let orphan = Warning {
            page: None,
            detail: "page 3: x".into(),
            ..on_page
        };
        assert_eq!(orphan.to_string(), "ocr: page 3: x");
    }

    #[test]
    fn sink_folds_repeats_and_keeps_first_occurrence_order() {
        let mut s = WarningSink::new(Some(3));
        s.push(WarningKind::PageTreeRevisited, || {
            "node revisited (5 0 R)".into()
        });
        s.push(WarningKind::XrefRepaired, || "repaired".into());
        s.push(WarningKind::PageTreeRevisited, || unreachable!("lazy"));
        s.push_n(WarningKind::ObjectDepth, 0, || unreachable!("n == 0"));
        s.push_n(WarningKind::ObjectDepth, 2, || "nesting limit".into());
        assert!(s.has(WarningKind::XrefRepaired));
        assert!(!s.has(WarningKind::CharLimit));
        let w = s.finish();
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].kind, WarningKind::PageTreeRevisited);
        assert_eq!(w[0].detail, "node revisited (5 0 R)");
        assert_eq!(w[0].count, 2);
        assert_eq!(w[1].count, 1);
        assert_eq!(w[0].page, Some(3));
        assert_eq!(w[1].detail, "repaired");
        assert_eq!(w[2].to_string(), "object: nesting limit (×2)");
        let mut s = WarningSink::new(None);
        s.push_n(WarningKind::CharLimit, u64::MAX, || "x".into());
        assert_eq!(s.finish()[0].count, u32::MAX);
    }
}
