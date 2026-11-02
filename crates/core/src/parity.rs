//! Parity metrics between a reference extraction and ours (design spec
//! §13.2; `migration-parity` spec).
//!
//! Before an application moves from another extractor (typically PyMuPDF),
//! both are run on the same pages and scored for agreement, without
//! storing either text. [`compare_page`] reduces one page pair to numbers
//! and booleans; [`ParityAccumulator`] folds any number of them (and merges
//! with others) into the aggregates the §13.2 gates are defined on. The
//! project's parity tests (`tests/parity/gate_b.rs`) are built on it: gate B
//! over the committed corpus, and gate A over a private corpus, which is how
//! a consumer measures its own documents offline
//! (`tools/oracle/run_private_corpus.sh`; `docs/migrating-from-pymupdf.md`).
//! The module is Rust-only; there is no Python binding.
//!
//! Definitions, all on texts after Python's `str.strip()` (the metrics are
//! defined on stripped text, and consumers compare Python strings):
//!
//! - **routing**: a page "is text" when it has at least
//!   [`ROUTING_MIN_CHARS`] characters (the usual OCR decision); the two
//!   sides agree when they decide alike;
//! - **similarity**: `1 − levenshtein(a, b) / max(chars(a), chars(b))`,
//!   counted in Unicode scalar values; 1.0 when the texts are equal
//!   (including both empty);
//! - **exact**: the stripped texts are equal;
//! - **line delta**: candidate lines minus reference lines, lines being the
//!   `'\n'`-separated parts of a non-empty stripped text.
//!
//! Percentiles use the nearest-rank rule (`p`-th = the `⌈p·n/100⌉`-th
//! smallest sample), so the median of an even count is the lower middle:
//! never more lenient than interpolation. The accumulator keeps
//! similarities in a histogram of [`SIMILARITY_BUCKETS`] buckets (floor of
//! `similarity · 10⁵`), not as samples, so its memory is bounded however
//! long it runs; a reported percentile is its bucket's lower edge. Every
//! gate threshold is a multiple of 10⁻⁵, so a verdict is exactly the one the
//! raw samples would give, and a reported value is at most 10⁻⁵ below the
//! true one.
//!
//! Cost: Levenshtein is O(n·m) time (O(min) memory) in the page lengths;
//! a pair of 20,000-character pages takes on the order of a second, which
//! is why scoring is an offline step and never runs beside a production
//! extraction.

use std::collections::BTreeMap;

/// A page "is text" for the routing metric when its stripped text has at
/// least this many characters (design spec §13.2).
pub const ROUTING_MIN_CHARS: usize = 50;

/// Resolution of the accumulator's similarity histogram: buckets
/// `0..=SIMILARITY_SCALE`, the last holding exact 1.0.
const SIMILARITY_SCALE: u32 = 100_000;

/// Number of similarity buckets (10⁻⁵ resolution, 1.0 included).
pub const SIMILARITY_BUCKETS: usize = SIMILARITY_SCALE as usize + 1;

/// Python's `str.isspace()`: Unicode `White_Space` plus U+001C–U+001F,
/// which Python counts as whitespace (bidi classes B and S) and Rust does
/// not.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()` with no arguments, which the metrics are defined
/// on.
#[must_use]
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(py_space)
}

/// `1 − levenshtein / max(len)` in characters of two (stripped) texts; 1.0
/// when they are equal.
#[must_use]
pub fn similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let longest = a.chars().count().max(b.chars().count());
    #[allow(clippy::cast_precision_loss)] // page lengths, far below 2^52
    let sim = 1.0 - strsim::levenshtein(a, b) as f64 / longest as f64;
    sim
}

/// Lines of an already stripped text: `'\n'`-separated, 0 when empty.
fn line_count(stripped: &str) -> usize {
    if stripped.is_empty() {
        0
    } else {
        stripped.split('\n').count()
    }
}

/// One page compared: numbers and booleans only, never text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageComparison {
    /// Both sides make the same routing decision ([`ROUTING_MIN_CHARS`]).
    pub routing_agrees: bool,
    /// `1 − normalised Levenshtein distance`, in `[0, 1]`.
    pub similarity: f64,
    /// The stripped texts are equal.
    pub exact: bool,
    /// Candidate lines minus reference lines.
    pub line_delta: i64,
    /// Characters in the stripped reference text.
    pub reference_chars: usize,
    /// Characters in the stripped candidate text.
    pub candidate_chars: usize,
}

/// Compares one page's reference text (for example PyMuPDF's) with the
/// candidate (ours). Both are stripped first, as Python's `str.strip()`
/// would.
#[must_use]
pub fn compare_page(reference: &str, candidate: &str) -> PageComparison {
    let (r, c) = (py_strip(reference), py_strip(candidate));
    let (reference_chars, candidate_chars) = (r.chars().count(), c.chars().count());
    #[allow(clippy::cast_possible_wrap)] // line counts are far below i64::MAX
    let line_delta = line_count(c) as i64 - line_count(r) as i64;
    PageComparison {
        routing_agrees: (reference_chars >= ROUTING_MIN_CHARS)
            == (candidate_chars >= ROUTING_MIN_CHARS),
        similarity: similarity(r, c),
        exact: r == c,
        line_delta,
        reference_chars,
        candidate_chars,
    }
}

/// Gate thresholds over many pages. Each is a minimum; a report passes a
/// gate when its value is at least the threshold.
///
/// Similarity thresholds (`median`, `low_min`) must be multiples of 10⁻⁵,
/// the histogram's resolution, for a verdict to equal the one the raw
/// samples would give; a finer threshold is judged against a bucket floor,
/// so it can fail a sample that is just above it. The §13.2 values and
/// the corpus-B ones (0.98, 0.90) all are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gates {
    /// Minimum share of pages whose routing decision agrees.
    pub routing: f64,
    /// Minimum median similarity.
    pub median: f64,
    /// The low percentile checked, in `(0, 100]`.
    pub low_percentile: f64,
    /// Minimum similarity at [`Gates::low_percentile`].
    pub low_min: f64,
}

impl Gates {
    /// Design spec §13.2: routing agreement ≥ 99.9 %, median similarity
    /// ≥ 0.999, 1st-percentile similarity ≥ 0.98.
    pub const SPEC: Self = Self {
        routing: 0.999,
        median: 0.999,
        low_percentile: 1.0,
        low_min: 0.98,
    };
}

impl Default for Gates {
    fn default() -> Self {
        Self::SPEC
    }
}

/// One gate's outcome in a [`ParityReport`].
#[derive(Clone, Debug, PartialEq)]
pub struct GateVerdict {
    /// `"routing"`, `"median"` or `"low_percentile"`.
    pub name: &'static str,
    /// The measured value.
    pub value: f64,
    /// The threshold it is held to.
    pub threshold: f64,
    /// `value >= threshold`.
    pub passed: bool,
}

/// The aggregates over every accumulated page. All rates and similarities
/// are `None` when no page was accumulated.
#[derive(Clone, Debug, PartialEq)]
pub struct ParityReport {
    /// Pages accumulated.
    pub pages: u64,
    /// Share of pages whose routing decision agrees.
    pub routing_rate: Option<f64>,
    /// Median similarity (nearest rank, bucket lower edge).
    pub median: Option<f64>,
    /// Similarity at the gates' low percentile.
    pub low: Option<f64>,
    /// The low percentile reported in `low`.
    pub low_percentile: f64,
    /// Smallest similarity seen (exact).
    pub min: Option<f64>,
    /// Mean similarity (exact).
    pub mean: Option<f64>,
    /// Share of pages that match exactly after stripping.
    pub exact_rate: Option<f64>,
    /// Mean of `|line_delta|`.
    pub line_delta_mean_abs: Option<f64>,
    /// Largest `|line_delta|`.
    pub line_delta_max_abs: u64,
    /// Pages with `|line_delta|` 0, 1, 2 and more than 2.
    pub line_delta_histogram: [u64; 4],
    /// The gate verdicts: routing, median, low percentile. Empty when no
    /// page was accumulated, so nothing is reported as passed.
    pub verdicts: Vec<GateVerdict>,
}

impl ParityReport {
    /// Whether at least one page was scored and every gate passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        !self.verdicts.is_empty() && self.verdicts.iter().all(|v| v.passed)
    }
}

/// Folds [`PageComparison`]s into the §13.2 aggregates in bounded memory.
/// Accumulators merge, so partial results from several processes or
/// periods combine into the report one accumulator fed every page would
/// give.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParityAccumulator {
    pages: u64,
    routing_agree: u64,
    exact: u64,
    similarity_sum: f64,
    similarity_min: Option<f64>,
    line_delta_abs_sum: u64,
    line_delta_max_abs: u64,
    line_delta_histogram: [u64; 4],
    /// Similarity bucket (`floor(similarity · 10⁵)`) to page count; sparse,
    /// at most [`SIMILARITY_BUCKETS`] entries.
    similarity_histogram: BTreeMap<u32, u64>,
}

/// The histogram bucket of a similarity in `[0, 1]`.
fn bucket(similarity: f64) -> u32 {
    let clamped = similarity.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // in 0..=SCALE
    let b = (clamped * f64::from(SIMILARITY_SCALE)).floor() as u32;
    b.min(SIMILARITY_SCALE)
}

impl ParityAccumulator {
    /// An empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pages accumulated so far.
    #[must_use]
    pub fn pages(&self) -> u64 {
        self.pages
    }

    /// Adds one page's comparison. Comparisons come from [`compare_page`],
    /// whose similarity is always in `[0, 1]`; a hand-built value outside
    /// that range is clamped into the histogram, and a NaN poisons `mean`.
    pub fn add(&mut self, c: &PageComparison) {
        self.pages += 1;
        self.routing_agree += u64::from(c.routing_agrees);
        self.exact += u64::from(c.exact);
        self.similarity_sum += c.similarity;
        self.similarity_min = Some(
            self.similarity_min
                .map_or(c.similarity, |m| m.min(c.similarity)),
        );
        let abs = c.line_delta.unsigned_abs();
        self.line_delta_abs_sum += abs;
        self.line_delta_max_abs = self.line_delta_max_abs.max(abs);
        #[allow(clippy::cast_possible_truncation)] // min(…, 3)
        let slot = abs.min(3) as usize;
        self.line_delta_histogram[slot] += 1;
        *self
            .similarity_histogram
            .entry(bucket(c.similarity))
            .or_insert(0) += 1;
    }

    /// Adds every page of `other`.
    pub fn merge(&mut self, other: &Self) {
        self.pages += other.pages;
        self.routing_agree += other.routing_agree;
        self.exact += other.exact;
        self.similarity_sum += other.similarity_sum;
        self.similarity_min = match (self.similarity_min, other.similarity_min) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.line_delta_abs_sum += other.line_delta_abs_sum;
        self.line_delta_max_abs = self.line_delta_max_abs.max(other.line_delta_max_abs);
        for (mine, theirs) in self
            .line_delta_histogram
            .iter_mut()
            .zip(other.line_delta_histogram)
        {
            *mine += theirs;
        }
        for (b, n) in &other.similarity_histogram {
            *self.similarity_histogram.entry(*b).or_insert(0) += n;
        }
    }

    /// Nearest-rank percentile `p` in `(0, 100]`: the lower edge of the
    /// bucket holding the `⌈p·n/100⌉`-th smallest sample.
    fn percentile(&self, p: f64) -> Option<f64> {
        if self.pages == 0 {
            return None;
        }
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )] // page counts far below 2^52; the rank is in 1..=n
        let rank = ((p / 100.0) * self.pages as f64).ceil().max(1.0) as u64;
        let rank = rank.min(self.pages);
        // The histogram holds every accumulated page, so some bucket reaches
        // the rank.
        self.similarity_histogram
            .iter()
            .scan(0, |seen, (b, n)| {
                *seen += n;
                Some((*seen, *b))
            })
            .find(|(seen, _)| *seen >= rank)
            .map(|(_, b)| f64::from(b) / f64::from(SIMILARITY_SCALE))
    }

    /// The aggregates and the verdicts against `gates`.
    #[must_use]
    pub fn report(&self, gates: &Gates) -> ParityReport {
        #[allow(clippy::cast_precision_loss)] // page counts, far below 2^52
        let rate = |k: u64| (self.pages > 0).then(|| k as f64 / self.pages as f64);
        #[allow(clippy::cast_precision_loss)]
        let mean = (self.pages > 0).then(|| self.similarity_sum / self.pages as f64);
        let routing_rate = rate(self.routing_agree);
        let median = self.percentile(50.0);
        let low = self.percentile(gates.low_percentile);
        let verdicts = match (routing_rate, median, low) {
            (Some(r), Some(m), Some(l)) => [
                ("routing", r, gates.routing),
                ("median", m, gates.median),
                ("low_percentile", l, gates.low_min),
            ]
            .into_iter()
            .map(|(name, value, threshold)| GateVerdict {
                name,
                value,
                threshold,
                passed: value >= threshold,
            })
            .collect(),
            _ => Vec::new(),
        };
        ParityReport {
            pages: self.pages,
            routing_rate,
            median,
            low,
            low_percentile: gates.low_percentile,
            min: self.similarity_min,
            mean,
            exact_rate: rate(self.exact),
            line_delta_mean_abs: rate(self.line_delta_abs_sum),
            line_delta_max_abs: self.line_delta_max_abs,
            line_delta_histogram: self.line_delta_histogram,
            verdicts,
        }
    }
}

#[cfg(test)]
#[path = "parity_tests.rs"]
mod tests;
