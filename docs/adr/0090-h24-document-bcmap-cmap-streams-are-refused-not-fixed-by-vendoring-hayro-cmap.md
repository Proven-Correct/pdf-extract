---
id: 0090
title: A document's CMap stream in hayro-cmap's binary format is refused (H24), not fixed by vendoring hayro-cmap
date: 2026-10-21
status: Accepted
summary: Patch 0034 (H24) makes the vendored hayro-interpret refuse a /ToUnicode or Type 0 /Encoding stream that starts with the bytes "bcmap" (hayro-cmap's private binary format) as an unparseable CMap, instead of vendoring hayro-cmap to fix its overflowing range decoder.
---

# 0090. A document's CMap stream in hayro-cmap's binary format is refused (H24), not fixed by vendoring hayro-cmap

## Context

Internal issue #121 (found by fuzzing). `hayro-cmap` 0.1.0 treats any CMap data that starts with `bcmap` as the binary format its embedded predefined CMaps are stored in. That parser trusts its input: it rebuilds each range as the previous end plus 1 plus a Huffman-decoded delta (`bcmap/mod.rs`, six unchecked sites in `parse_cid_segment` and `parse_bf_segment`), so hostile deltas overflow. Builds with overflow checks (test, fuzz) panic. The workspace's release profile does not enable them, so there the range wraps and the map is wrong. The data reached the parser straight from the document: a `/ToUnicode` stream (`font/mod.rs`, `read_to_unicode`) and a Type 0 `/Encoding` stream (`font/cid.rs`, `read_encoding`) are both passed to `CMap::parse`. ADR 0086's codec boundary covers image codecs only, so nothing caught it.

`hayro-cmap` is not vendored (ADR 0080: vendor only what we patch). When this was decided, crates.io had only `hayro-cmap` 0.1.0, so no released fix existed to bump to, and the upstream sources still had every unchecked site.

## Decision

Patch 0034 (H24) adds `is_binary_cmap(data)` (`data.starts_with(b"bcmap")`) to the vendored `hayro-interpret` and applies it at the two places a document's CMap bytes are parsed. A stream in the binary format is treated exactly like a CMap that fails to parse: a `/ToUnicode` map is absent, and a Type 0 font whose `/Encoding` stream is refused fails to load.

Only data that comes from the document is checked. Predefined CMaps named by the document, the UCS2 fallback and `usecmap` inside a text CMap still resolve through `hayro_cmap::load_embedded` and still use the binary parser, which therefore sees only the trusted bundle. The `cmap` fuzz driver (`fuzz_api::parse_cmap`) drops `bcmap` input the same way, so the target fuzzes what the interpreter can reach, and the reproducer is committed to `fuzz/regressions/cmap/`. `crates/core/tests/h24_binary_cmap_gate.rs` pins the gate, including that the bundled CMaps still parse.

The same patch carries comment-only corrections unrelated to CMaps (the H13 pixel-cap doc comments, `resolve_palette_indices`, an import order).

## Alternatives considered

- **Vendor `hayro-cmap` and fix its range decoder** - fixes the cause and would let a hostile bcmap stream be parsed safely. It costs a sixth vendored crate (about 392 KB including the compressed CMap bundle) and a third pristine tag. `export-patches.sh` would have to move from two ranges to N, with tests in `test-check-patches.sh`, which is already near its size tripwire. It also needs new `check-patches.sh` and PROVENANCE rows, `[patch.crates-io]` entries in the root manifest, `fuzz/Cargo.toml` and the h9 harness, changes to `gen-notices.sh` and `license.rs` (both treat `hayro-cmap` as a registry crate), a regenerated `THIRD_PARTY_NOTICES`, and an addendum or successor to ADR 0080. That is worth paying only if a document's binary-format stream had to be accepted, and nothing does.
- **Wait for an upstream fix and bump** - none exists to bump to (see Context). An upstream report is independent of this decision and does not replace the gate.

## Consequences

- A document can no longer reach the binary parser. The overflow is closed on the production path with no new vendored crate.
- Behaviour moves toward the oracle: the binary format is private to hayro-cmap, and no other PDF reader is known to parse it from a document. The cost is that a document carrying such a stream loses its ToUnicode map, or (as `/Encoding`) its font, where hayro-cmap alone would have decoded it. A PDF carrying one was never valid input for any other reader.
- The binary parser is now reachable only through the bundle (predefined names and `usecmap`). A flaw in the bundle's data is out of scope, since it is trusted.
- The embedded-CMap seeds for the `cmap` fuzz target (`fuzz/src/bin/embedded_cmaps.rs`, `tools/fuzz/seed-cmaps.py`) are all `bcmap` data, so they are now dead on their own; only a mutation that loses the prefix reaches the text parser. The target no longer fuzzes the binary format from hostile input (`openspec/specs/fuzzing`).
- The gate is a prefix test, so it must be revisited if `hayro-cmap` changes its magic bytes or an upstream release makes the decoder safe. In debug builds the reproducer test asserts that the unguarded parser still overflows on the reproducer, and fails when it stops, as the prompt to reconsider.
- No committed test passes a text CMap through the success path of the new gate as a Type 0 `/Encoding` stream.
