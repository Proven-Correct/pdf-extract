---
id: 0081
title: A Rust noise pre-check replicating Tesseract's Otsu guards recognition against noise-dominated rasters
date: 2026-10-05
status: Accepted
summary: Before Tesseract runs, a pure-Rust check replicates its global Otsu split and tests the isolation of its foreground side; speckle is downsampled once and re-checked, else skipped, default on with a noise_precheck opt-out.
---

# 0081. A Rust noise pre-check replicating Tesseract's Otsu guards recognition against noise-dominated rasters

## Context

Tesseract binarises the whole raster with one global Otsu threshold. On a page with little real ink the threshold splits the noise instead: `sparse_below_min_chars_noise` at 300 dpi becomes 21.7 % foreground, about 1.09 million components, 16-25 s of layout analysis and garbage output (CER 0.93); a pure-noise blank page costs 46 s. The same page at 150 dpi reads exactly in 22 ms. Spec 0003 section 1.3 (A4). The first rule ("class `None` implies speckle") skipped real text (an inverted banner, a figure) and let an inverted noise page stall 27 s; user-approved revision Q10 (2026-10-05) replaced it.

## Decision

`precheck::classify` in `pdf-extract-ocr` (pure Rust, `forbid(unsafe_code)`) replicates Tesseract's `OtsuStats` (same maximisation, same class verdicts) and takes Tesseract's own foreground side (pixels `<= t` when the dark share is under half, else `> t`). For every class except `Empty`, when that side's share `p >= 0.01` and its isolation fraction is `>= 0.5 x (1 - p)^4`, the raster is `Speckle`; otherwise `Clear`. The class never decides by itself. On `Speckle` the raster is box-filtered 2x2 once and re-checked: `Clear` is recognised at half the dpi (coordinates mapped with the exact factor 2), still `Speckle` is skipped with an empty OCR page. Warnings `OcrNoiseDownsampled` / `OcrNoiseSkipped`, also surfaced as `page N: ocr: ...` notes in `Extraction.warnings`. It runs in `OcrEngine::ocr` before the backend (so in the parent under process isolation), only when `noise_precheck` is true (default on; Python `OcrEngine(noise_precheck=False)`) and the rendered dpi is at least 144. The constants (0.01, 0.5, the floor, one downsample step) are fixed, not configurable.

## Alternatives considered

- **Leptonica in the pre-check** - would load native code into the parent under `Isolation::Process` and put the check behind, not in front of, the backend.
- **Connected-component analysis** - 0.3-1 s per 300-dpi page; the isolation fraction decides the same way in one pass.
- **A "dark pixels" rule** - misfires on gray text with no dark pixel (reads perfectly).
- **The class-based first rule** - skipped real text and stalled on inverted noise (above).
- **Configurable thresholds** - widens the surface for no measured need.

## Consequences

This narrows spec 0002 section 2.1's "no pre-processing" non-goal (only rasters classified `Speckle` are touched) and amends O2/O18/O19 (recognition now has two warning kinds). Margins measured on corpus D at 72/150/300 dpi: at least 2x for noise rasters, pages with content at isolation 0.016 or below (corpus-D text pages 0.000-0.013); corpus D is unchanged. Residuals: correlated noise and photo pages (spec section 2.3). Cost under 50 ms per A4 raster at 300 dpi, reported, never gated. Opt-out restores run-0002 behaviour including the stall.
