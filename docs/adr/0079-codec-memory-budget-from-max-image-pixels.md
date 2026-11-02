---
id: 0079
title: Codec-internal memory is bounded by a per-decode byte budget derived from max_image_pixels
date: 2026-10-05
status: Accepted
summary: JBIG2 and JPEG 2000 decodes run under a cumulative byte budget of 2 and 64 bytes per max_image_pixels pixel, enforced inside the vendored codecs and refused as a distinct ImageMemoryLimit warning.
---

# 0079. Codec-internal memory is bounded by a per-decode byte budget derived from max_image_pixels

## Context

ADR 0062 (H13) bounded the output buffers at declared, header and reported dimensions, but left two stated residuals: JBIG2 region, symbol, pattern and halftone bitmaps, and JPEG 2000 per-tile records, whose sizes come from the encoded data. A small file could still make the decoder allocate gigabytes (for example a 16 GiB halftone gray-scale plane from 8 header bytes, or 65,535 clones of one symbol). Spec 0003 section 1.2 (A2) closes them. The crates involved had to be vendored to be patched (ADR 0080).

## Decision

One cumulative byte budget per decode call, derived from the existing `DecodeLimits.max_image_pixels` (no new `Limits` field): `JBIG2_BUDGET_BYTES_PER_PIXEL = 2` and `JPX_BUDGET_BYTES_PER_PIXEL = 64`, constants in hayro-syntax `filter/mod.rs`. The budget is enforced inside the vendored `hayro-jbig2` (H14) and `hayro-jpeg2000` (H15): every allocation sized by file-declared values is charged before it is made, including clones; the total is never refunded when a buffer drops. The codecs differ on reuse: JBIG2 (H14) charges a reinitialised bitmap in full even when its old buffer is large enough, and its charged `Vec`s may reserve up to 2x what was charged; JPEG 2000 (H15) charges reused buffers by capacity growth only, so its charges track the bytes allocated. An over-budget charge fails with the codec's `MemoryBudget` error, which hayro-syntax maps to `DecodeFailure::ImageMemoryLimit`, surfaced as `InterpreterWarning::ImageMemoryLimit` / `WarningKind::ImageMemoryLimit`, distinct from `ImagePixelLimit`. Every other codec error stays `ImageDecode`. CCITT and DCT are untouched.

Measured facts the multipliers rest on (spec 0003 revision log, Appendix A.5): the 64 multiplier leaves only a 1.5x margin for the costliest legitimate JPEG 2000 geometry (four components, 42 B/px on OpenJPEG output; gray 15, RGB 33, tiled RGB 17, RGB with 8 resolutions and 16x16 code-blocks 37), and more than four components are refused earlier (`TooManyChannels`); 1x1 code-blocks cost about 242 B/px, so such files are refused above roughly a quarter of the cap; JBIG2 refinement/aggregation dictionaries re-charge the reference list and Huffman table per aggregated symbol (about 44 B x N^2/2), so about 5,000 Huffman-coded aggregate symbols in one dictionary are refused at the default cap; a JPEG 2000 tile-count overflow surfaces as `ImageTooLarge` and hence `ImageDecode`, not `ImageMemoryLimit`. The default JPEG 2000 budget is 16.4 GB (15.3 GiB), not 16 GiB.

## Alternatives considered

- **A new `Limits` field for codec memory** - rejected; the existing knob is enough and callers who need more raise `max_image_pixels`.
- **Per-allocation caps (a fixed size per bitmap)** - do not bound the sum, and the 65,535-clone case passes each cap.
- **Refunding on drop** - needs ownership tracking through every codec path for little gain; cumulative is simple and conservative.
- **Reusing `ImagePixelLimit`** - rejected so that kind stays honest, as ADR 0062 did for `ImageDecodeFailure`.

## Consequences

The memory bound of spec 0002 section 2.2 now covers codec-internal allocations, under the small uncounted allowances stated in the spec. Some legitimate pathological files are refused (the cases above) and a caller can raise `max_image_pixels`. The budget is one-shot per decode; in JPEG 2000 a decoder context reused across decodes does not re-charge the capacity it already holds (hayro-syntax always decodes with a fresh context). The vendored codec diffs are patches 0023 onward and must be exported (ADR 0080).

## Addendum (2026-10-12)

Patch 0026 (H16, internal PR #75, internal issue #63) closes two crash shapes in hayro-jpeg2000 that the budget did not reach. Both now fail before any allocation with `TooManyChannels`, which hayro-syntax maps to `ImageDecode` rather than `ImageMemoryLimit`:

- a palette (`pclr`) JP2 whose codestream has more than one component, decoded with palette indices unresolved (as hayro-syntax always decodes);
- any image with more than 255 components.

The first shape overran a one-channel output buffer, and a 64x64 image panicked under any budget. In the second, the progression stores the component count as `u8`, so a count of 256 asserted and counts of 257–511 silently dropped components. The default budget did not stop the 256-component case either, contrary to what the run-0003 notes assumed.

`the_channel_planes_of_a_palette_image_are_charged` now exercises plane charging with a large 1-component palette image. Per ADR 35 no panic is caught; the remaining unaudited panic sites in both codecs, and a possible codec-boundary `catch_unwind`, are tracked in internal #76.

## Addendum (2026-10-19)

The "possible codec-boundary `catch_unwind`" tracked in internal #76 is now patch 0028 (H18), recorded in ADR 0086. A panic inside a call into any of the four image codecs (DCT, CCITT, JBIG2, JPEG 2000) is caught and becomes `DecodeFailure::ImageDecode`, so the image is skipped with `image: decode failed` instead of failing the page. A refusal by the memory budget is still `ImageMemoryLimit`, because the budget error is returned, not raised as a panic. Under `cfg(fuzzing)` the catch is compiled out, so the remaining unaudited panic sites are still found by fuzzing and fixed through bug-fix pass.

## Addendum (2026-10-19)

Patch 0033 (H23, internal PR #133, internal issue #116 item 2) adds a second JBIG2 allowance beside the byte budget, derived from the same `max_image_pixels` and carried in the same budget handle. The byte budget bounds what a decode allocates, not how often it draws what it allocated: a text region places up to 10,000 symbol instances and so does every aggregate symbol of a dictionary, so a few hundred bytes could draw a symbol of millions of pixels over and over, with no bound on CPU and out of reach of `page_time_budget` (ADR 0027, ADR 0088).

- **What is charged.** Each placed instance costs its symbol's `width x height`, unclipped, plus `INSTANCE_WORK_PIXELS = 64`, charged before the instance is drawn (`charge_work` in `decode/text.rs`, shared by text regions and aggregate symbols). Like the byte budget it is cumulative, never refunded, and counted from zero per `decode_with` call. Halftone region rendering is not charged.
- **The allowance.** `DecoderContext::work_budget = JBIG2_WORK_PIXELS_PER_PIXEL (16) x max_image_pixels` (hayro-syntax `filter/mod.rs`); `max_image_pixels: None` means no allowance, and a default `DecoderContext` has none. 16 is as many pixels as the 2 B/px byte budget can allocate as 1-bit bitmaps, so a decode may draw each pixel it could hold once. A text page draws each glyph about once, so this leaves a fourfold margin at the cap and far more for ordinary scans. The 64 is there because the per-placement overhead (about 30 ns, 64 pixels at the measured 0.45 ns) dominates small symbols, and an empty symbol would otherwise be free. Worst case at the 256M default cap: about 0.45 ns per charged pixel for the costliest shape (one-pixel-wide symbols), a few seconds of drawing.
- **Reporting.** `OverflowError::WorkBudget` is mapped with `MemoryBudget` to `DecodeFailure::ImageMemoryLimit`, so the image is skipped with `InterpreterWarning::ImageMemoryLimit` / `WarningKind::ImageMemoryLimit` and counted in `image_memory_limit_hits`. That warning kind therefore now also covers decode work, not only memory; raising `max_image_pixels` raises both. The work allowance is charged to the codec budget rather than given its own `Limits` field or warning kind: both refusals are the decoder outgrowing what the cap allows, and raising the cap admits both (hayro-syntax `decode_within_budget`). The warning's detail text still says "memory budget" only.
- **Known gap.** Halftone `render_patterns` (grid cells x pattern area) is not charged to the work allowance, and no other JBIG2 region type is.

## Addendum (2026-10-21)

Three residuals from the run-0003 review, corrected or recorded here (internal issues #66 A11 and A12, internal #65 C6). They qualify the claim above that allocations linear in the input are "bounded by `max_file_size`"; they do not change the decision.

- **A segment header can allocate well past the stream it came from (A11).** `hayro-jbig2`'s `segment.rs` calls `Vec::with_capacity(referred_to_count)` for the referred-to segment numbers before reading them. In the long form the count is a 29-bit field and the file need only carry its retention bytes (one byte per eight referred-to segments) to reach it, so the allocation is about 32 times those bytes, up to 2 GiB for one segment header (`2^29 - 1` four-byte entries). It is uncharged (the budget does not cover it) and it runs in `Image::new_embedded`, before `decode_with` installs the budget. It is transient. Its bound is the stream, but the stream here is filter output, so the bound is `max_stream_decoded_bytes` (256 MiB by default), not `max_file_size`. The allocation does not panic. If the allocator fails it aborts the process, and the codec guard of ADR 0086 cannot catch an abort. This is not clamped in the fork.
- **JBIG2's embedded MMR runs uncharged vectors in `hayro-ccitt` (A12).** A generic region coded with MMR calls `hayro_ccitt::decode` (not vendored, 0.3.0 in `Cargo.lock`) for a bitmap that was charged. The decoder keeps `ref_changes` and `coding_changes`, vectors that grow with the row width. JBIG2 bitmaps are at most 65,535 wide (`MAX_DIMENSION`), so each vector holds at most that many entries, about 1 to 2 MiB per region including growth slack. They are freed when the region finishes. CCITT as a PDF image filter is outside spec 0003 §2.1, but this use of it inside JBIG2 is in scope and spec 0003 does not name it.
- **The arithmetic-coded leg of refinement inside an aggregate is untested (internal #65 C6).** The Huffman-coded case, an aggregated symbol whose embedded text region refines an instance (`R_I = 1`), is covered by `refined_aggregate` and the differential charge tests added with it. The arithmetic-coded case needs the arithmetic text-region coding (the integer contexts including the refinement flag and deltas, and the symbol-ID coder IAID), and the test kit's `mq` module has only the Annex A.2 integer encoder and the generic template 0 and refinement template 1 region encoders, so it stays a residual. A plain immediate text region that refines an instance (outside aggregation) is likewise not covered.

Spec 0003 F6 and the §2.2 row for this boundary said the allocations linear in the input were bounded by `max_file_size`. Both now carry a dated note pointing here.
