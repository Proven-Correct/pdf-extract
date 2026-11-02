//! Fuzz target 2 (design spec §13.6): extract the text page of every page of
//! a document opened from raw bytes, under the fuzzing limits. Seeds: corpus
//! B and C.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| pdf_extract_fuzz::text_sampled_pages(data.to_vec()));
