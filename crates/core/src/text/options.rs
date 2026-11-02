//! Text extraction options (spec §3.3 `text/options.rs`; T27, T29-T33).
//!
//! [`TextOptions`] bundles what a caller can choose per extraction: the
//! policy for glyphs without Unicode ([`Unmapped`], applied by the text
//! device), how blocks are separated in the serialised text
//! ([`BlockSeparator`]) and the line assembler's thresholds
//! ([`LayoutParams`]). The defaults reproduce the reference extractor's
//! `'text'` output (the "compat" profile).

/// What to emit for a glyph that has no Unicode mapping (spec T27, design
/// spec §6.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Unmapped {
    /// Emit the raw character code (the CID for composite fonts) as a code
    /// point, as the reference extractor does (KP6). Codes that cannot be
    /// stored safely become U+FFFD instead: NUL, C0 controls other than tab,
    /// line feed and carriage return, surrogates, U+FFFE, U+FFFF and values
    /// above U+10FFFF (a deliberate difference, D-10). C1 controls are kept.
    #[default]
    Cid,
    /// Emit U+FFFD for every unmapped glyph.
    Replacement,
    /// Emit nothing. The glyph's position is still the content's, so the
    /// space it occupied shows up as a gap to the next glyph (which may
    /// become a space or a line break).
    Skip,
}

/// How blocks are separated in the serialised text (design spec §6.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlockSeparator {
    /// Every line ends in `'\n'` and nothing marks a block boundary: the
    /// reference extractor's `'text'` shape (compat default).
    #[default]
    Newline,
    /// As `Newline`, plus one empty line between blocks.
    BlankLine,
}

/// Thresholds of the line assembler (spec T29-T32, D-9), each a multiple of
/// the font size of the glyph being placed (its *em*), except
/// `angle_tolerance_deg`.
///
/// The defaults are the compat values bracketed by the reference probes
/// (spec Appendix A.1), each strictly inside its bracket so that no probe
/// sits on a threshold; every comparison is a strict `>`. Values that are negative or NaN are not rejected;
/// they simply make the corresponding test always or never fire.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutParams {
    /// A gap along the baseline wider than this (strictly: `gap > space_gap
    /// · em`) inserts a space unless the incoming glyph is U+0020 or the
    /// line already ends in U+0020, U+1680, U+3000 or U+FEFF (KP1, KP24).
    /// Default 0.149.
    ///
    /// The reference probes bracket the threshold at [0.1477, 0.15) em under
    /// the strict comparison: a 3.25 pt gap at 22 pt (0.1477 em) joins, and a
    /// 3 pt gap before a 20 pt glyph (exactly 0.15 em) gets a space. The
    /// comparison is strict because a gap of zero must never become a space
    /// (`em` can be 0 for a zero-size font). 0.15 itself would put that
    /// second probe exactly on the threshold, where the f32 page positions
    /// decide the result; 0.149 sits strictly inside the bracket, 0.001 em
    /// from both probes (spec revision 7).
    pub space_gap: f32,
    /// Movement across the baseline wider than this (strictly: `perp >
    /// perp_break · em`) starts a new line (KP2). Default 0.809, inside the
    /// probed bracket [0.8000, 0.8182): an 11 pt glyph raised 8.8 pt joins,
    /// one raised 9 pt breaks.
    pub perp_break: f32,
    /// A forward gap along the baseline wider than this (strictly: `along >
    /// gap_break · em`) starts a new line (KP2). Default 0.798, inside the
    /// probed bracket [0.7955, 0.8000): a 17.5 pt gap at 22 pt stays on the
    /// line, an 8.8 pt gap at 11 pt and a 16 pt gap before a 20 pt glyph
    /// break. The along and across brackets abut at 0.8 em, so the two
    /// directions need separate thresholds (spec revision 7b).
    pub gap_break: f32,
    /// A backward movement along the baseline wider than this (strictly:
    /// `-along > back_break · em`) starts a new line; less joins without a
    /// space (KP3). Default 0.795, inside the probed bracket
    /// [0.7727, 0.8182): moving back 8.5 pt at 11 pt joins, 9 pt breaks.
    pub back_break: f32,
    /// A new line further than this from the previous one (across the
    /// baseline) also starts a new block (T32). Default 1.5.
    pub block_gap: f32,
    /// A new line starting further left of its block than this starts a new
    /// block (T32). Default 2.0.
    pub block_indent: f32,
    /// Baseline directions further apart than this (degrees) start a new
    /// line and a new block. Default 2.0.
    pub angle_tolerance_deg: f32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            space_gap: 0.149,
            perp_break: 0.809,
            gap_break: 0.798,
            back_break: 0.795,
            block_gap: 1.5,
            block_indent: 2.0,
            angle_tolerance_deg: 2.0,
        }
    }
}

/// Options for [`Page::text_page`](crate::Page::text_page) and
/// [`Page::text`](crate::Page::text). The default is the compat profile.
///
/// There is deliberately no optional-content switch: hidden optional
/// content is always excluded, as the reference does (KP11, D-28).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextOptions {
    /// Policy for glyphs without a Unicode mapping.
    pub unmapped: Unmapped,
    /// Block separator used by [`Page::text`](crate::Page::text).
    pub block_separator: BlockSeparator,
    /// Line-assembly thresholds.
    pub layout: LayoutParams,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_compat_profile() {
        let o = TextOptions::default();
        assert_eq!(o.unmapped, Unmapped::Cid);
        assert_eq!(o.block_separator, BlockSeparator::Newline);
        assert_eq!(
            o.layout,
            LayoutParams {
                space_gap: 0.149,
                perp_break: 0.809,
                gap_break: 0.798,
                back_break: 0.795,
                block_gap: 1.5,
                block_indent: 2.0,
                angle_tolerance_deg: 2.0,
            }
        );
    }
}
