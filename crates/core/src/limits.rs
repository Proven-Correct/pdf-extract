//! Resource limits (spec §1.4, §3.3 `limits.rs`).
//!
//! Every bound the library places on hostile input lives in [`Limits`], so a
//! caller tunes them in one place and the Python binding mirrors one struct.
//! Two bounds are deliberately not here: the object-nesting cap is the fixed
//! constant `hayro_syntax::MAX_OBJECT_DEPTH` (D-12: the parser has no access
//! to per-document settings), and `max_pages` is an [`OpenOptions`] field
//! because it is a per-call request rather than a safety policy.
//!
//! [`OpenOptions`]: crate::OpenOptions

use std::fmt;
use std::time::Duration;

/// Bounds on the work and memory one document may cost.
///
/// The defaults admit every real document the project has seen while keeping
/// a hostile one from exhausting a process; see spec §1.4 for where each is
/// enforced. Fields are public: none depends on another, so there is no
/// invariant for a constructor to protect.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Largest input accepted, in bytes. Checked against file metadata
    /// before anything is read. Default 512 MiB.
    pub max_file_size: u64,
    /// Deepest page-tree node visited (the root is depth 0). Deeper subtrees
    /// are skipped with a [`WarningKind::PageTreeDepth`](crate::WarningKind)
    /// warning. Default 256.
    pub max_page_tree_depth: u32,
    /// Most cross-reference entries a document may have. Checked after the
    /// xref is parsed, so it bounds downstream work, not the parse.
    /// Default 10,000,000.
    pub max_objects: usize,
    /// Most bytes one stream may decode to; a stream over it is treated as
    /// absent. Default 256 MiB.
    pub max_stream_decoded_bytes: u64,
    /// Most content-stream operators interpreted per page. Default 10,000,000.
    pub max_ops_per_page: u64,
    /// Wall-clock budget for interpreting one page. Default 30 s.
    pub page_time_budget: Duration,
    /// Most characters extracted per page. Default 2,000,000.
    pub max_chars_per_page: usize,
    /// Largest image, by `width × height`, that rendering will decode; a
    /// larger one is skipped with a
    /// [`WarningKind::ImagePixelLimit`](crate::WarningKind) warning. The
    /// size is checked as declared (`/Width × /Height`, before any filter),
    /// as each image codec reads it from its header before it allocates, and
    /// as the codec reports it after decoding, so the cap bounds the
    /// allocation, not only the output. It also sets the memory budgets of
    /// the JBIG2 and JPEG 2000 decoders, whose working sets the image size
    /// does not bound: 2 and 64 bytes per pixel of this cap (512 MB and
    /// 16.4 GB at the default), counted over one image's decode, and the
    /// JBIG2 decoder's allowance for placing symbol instances, 16 pixels per
    /// pixel of the cap; an image whose decoder would exceed either is
    /// skipped with a [`WarningKind::ImageMemoryLimit`](crate::WarningKind)
    /// warning.
    /// The text path never decodes images, so this matters only to rendering
    /// (OCR). Default 256,000,000.
    pub max_image_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_size: 512 * 1024 * 1024,
            max_page_tree_depth: 256,
            max_objects: 10_000_000,
            max_stream_decoded_bytes: 256 * 1024 * 1024,
            max_ops_per_page: 10_000_000,
            page_time_budget: Duration::from_secs(30),
            max_chars_per_page: 2_000_000,
            max_image_pixels: 256_000_000,
        }
    }
}

/// Which limit an error or warning refers to.
///
/// `Display` gives the snake-case name the Python binding reports as
/// `LimitExceededError.limit_name` (spec T47b), so the two never drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LimitKind {
    /// [`Limits::max_file_size`].
    FileSize,
    /// `OpenOptions::max_pages`.
    Pages,
    /// [`Limits::max_page_tree_depth`].
    PageTreeDepth,
    /// [`Limits::max_objects`].
    Objects,
    /// [`Limits::max_stream_decoded_bytes`].
    StreamDecodedBytes,
    /// [`Limits::max_ops_per_page`].
    OpsPerPage,
    /// [`Limits::page_time_budget`].
    PageTimeBudget,
    /// [`Limits::max_chars_per_page`].
    CharsPerPage,
    /// [`Limits::max_image_pixels`].
    ImagePixels,
    /// The OCR engine's `max_render_pixels` (configured on the engine, not
    /// in [`Limits`]: it bounds the raster OCR allocates, not the document).
    RenderPixels,
    /// The OCR engine's `page_timeout`, which covers rendering and
    /// recognition of one page.
    OcrPageTimeout,
}

impl LimitKind {
    /// The snake-case name of the limit (`"objects"`, `"ops_per_page"`, ...).
    pub fn name(self) -> &'static str {
        match self {
            Self::FileSize => "file_size",
            Self::Pages => "pages",
            Self::PageTreeDepth => "page_tree_depth",
            Self::Objects => "objects",
            Self::StreamDecodedBytes => "stream_decoded_bytes",
            Self::OpsPerPage => "ops_per_page",
            Self::PageTimeBudget => "page_time_budget",
            Self::CharsPerPage => "chars_per_page",
            Self::ImagePixels => "image_pixels",
            Self::RenderPixels => "render_pixels",
            Self::OcrPageTimeout => "ocr_page_timeout",
        }
    }
}

impl fmt::Display for LimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec_table() {
        let l = Limits::default();
        assert_eq!(l.max_file_size, 512 << 20);
        assert_eq!(l.max_page_tree_depth, 256);
        assert_eq!(l.max_objects, 10_000_000);
        assert_eq!(l.max_stream_decoded_bytes, 256 << 20);
        assert_eq!(l.max_ops_per_page, 10_000_000);
        assert_eq!(l.page_time_budget, Duration::from_secs(30));
        assert_eq!(l.max_chars_per_page, 2_000_000);
        assert_eq!(l.max_image_pixels, 256_000_000);
    }

    #[test]
    fn kind_names_are_distinct_snake_case() {
        let all = [
            LimitKind::FileSize,
            LimitKind::Pages,
            LimitKind::PageTreeDepth,
            LimitKind::Objects,
            LimitKind::StreamDecodedBytes,
            LimitKind::OpsPerPage,
            LimitKind::PageTimeBudget,
            LimitKind::CharsPerPage,
            LimitKind::ImagePixels,
            LimitKind::RenderPixels,
            LimitKind::OcrPageTimeout,
        ];
        let names: std::collections::HashSet<String> =
            all.iter().map(ToString::to_string).collect();
        assert_eq!(names.len(), all.len());
        for n in &names {
            assert!(n.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{n}");
        }
        assert_eq!(LimitKind::OpsPerPage.to_string(), "ops_per_page");
        assert_eq!(LimitKind::ImagePixels.name(), "image_pixels");
        assert_eq!(LimitKind::RenderPixels.name(), "render_pixels");
        assert_eq!(LimitKind::OcrPageTimeout.name(), "ocr_page_timeout");
    }
}
