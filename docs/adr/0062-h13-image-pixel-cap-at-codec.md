---
id: 0062
title: max_image_pixels is enforced at declared, codec-header and reported dimensions (H13)
date: 2026-10-03
status: Accepted
summary: H13 enforces max_image_pixels in hayro-syntax's DecodeLimits at the declared, codec-header and reported dimensions, with a distinct failure variant.
---

# 0062. max_image_pixels is enforced at declared, codec-header and reported dimensions (H13)

## Context

Spec 0002 D-8, revised in revision (3). Unit 04 found the declared-size check bounds only DCT: CCITT, JBIG2 and JPEG 2000 size their output from the encoded data (Appendix B.4), so a small declared image could still decode into a huge buffer.

## Decision

`max_image_pixels` is enforced in hayro-syntax's `DecodeLimits`: on the declared dimensions before any filter runs, on each codec's header dimensions inside its filter module before it allocates, and on the reported dimensions before conversion, with a distinct `DecodeFailure`/`InterpreterWarning` variant.

## Alternatives considered

- **Declared-size check only** (the signed-off form) - bounds only DCT.

## Consequences

The memory bound in spec 0002 section 2.2 is true for the output buffers. Not bounded, and stated as residuals: JBIG2 region, symbol, pattern and halftone bitmaps, and JPEG 2000 per-tile records. A follow-on fork unit for a JBIG2 internal budget was offered to the user.

## Addendum (2026-10-06)

The two residuals named above (JBIG2 internal bitmaps, JPEG 2000 per-tile records) and the offered follow-on unit were taken up in run 0003: see ADR 0079, which bounds codec-internal memory with a per-decode byte budget derived from `max_image_pixels`. This record's decision stands.

## Addendum (2026-10-21)

Three residuals of the H13 checks that still hold after H14 to H24 (internal issue #56 B22), recorded so nobody reads the three-point enforcement as stronger than it is.

- **The DCT header check never fires.** `maybe_patch_jpeg_dimensions` rewrites the SOF marker down to the declared dimensions before the decoder reads the header, so the header size it then checks is never above the declared size, which was checked first. The check stays so that every codec follows the same rule. `jpeg_frames_are_patched_down_to_the_declared_size` (`crates/core/tests/h13_image_pixels.rs`) pins the patching.
- **A Group 4 image with `/Rows 0` and a large body is now refused.** The check bounds the rows as `4 x data.len()` when `/Rows` is 0, which is exact for Group 3 and only errs high for Group 4 (which decodes nothing without `/Rows`). A Group 4 stream whose body makes `Columns x 4 x data.len()` exceed the cap used to give an empty image and is now an `ImagePixelLimit` refusal. That change is neither recorded in the original decision nor covered by a test; the Group 3 case is (`ccitt_without_rows_is_bounded_by_its_data`).
- **A refused JBIG2 globals stream is swallowed, yet the counter still moves.** `jbig2.rs` reads `/JBIG2Globals` with `.and_then(|g| g.decoded().ok())`, so a globals stream whose decode fails, including one refused by `check_image_pixels`, is treated as absent (the image then decodes without those globals). `check_image_pixels` increments `image_pixel_limit_hits` when it refuses, wherever the refusal is later swallowed, so core and render report an `ImagePixelLimit` warning for an image that may still have been drawn.
