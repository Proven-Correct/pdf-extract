//! Fuzz target 4 (design spec §13.6): parse raw bytes as a CMap, resolving
//! `usecmap` against the predefined CMaps. Input in hayro-cmap's embedded
//! binary form (starting `bcmap`) is dropped, as the interpreter refuses it
//! from a document (H24, internal #121), so hostile input reaches only the text parser.
//! Seeds: corpus B and C's CMap streams (tools/fuzz/seed-cmaps.py), and the
//! fuzz crate's `synthetic_cmaps` binary, hand-written text CMaps covering
//! the grammar those streams miss, each checked to parse before it is written.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| pdf_extract_fuzz::parse_cmap(data));
