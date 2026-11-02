//! Fuzz target 3 (design spec §13.6): render page 0 of a document opened from
//! raw bytes at 36 dpi, under the fuzzing limits. The only target that
//! decodes images, so the only one that reaches the image codecs; the
//! `cfg(fuzzing)` build compiles their panic boundary out, so a codec panic
//! is a crash here. Seeds: corpus B and C.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| pdf_extract_fuzz::render_page0(data.to_vec()));
