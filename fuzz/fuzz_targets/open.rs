//! Fuzz target 1 (design spec §13.6): open a document from raw bytes and read
//! its page count, under the fuzzing limits. Seeds: corpus B and C.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| pdf_extract_fuzz::open_and_count(data.to_vec()));
