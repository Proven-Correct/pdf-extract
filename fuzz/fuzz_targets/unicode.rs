//! Fuzz target 5 (design spec §13.6): the font-to-Unicode fallbacks (glyph
//! names, codes, PDF text strings). The first input byte selects the
//! fallback; see `pdf_extract_core::fuzz_api::unicode_fallbacks`. Seeds:
//! fuzz/seeds/unicode/.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| pdf_extract_fuzz::unicode_fallbacks(data));
