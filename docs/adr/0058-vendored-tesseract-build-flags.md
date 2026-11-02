---
id: 0058
title: Vendored Tesseract build disables the legacy engine, graphics, OpenMP and native tuning
date: 2026-10-03
status: Accepted
summary: The vendored build sets DISABLED_LEGACY_ENGINE, GRAPHICS_DISABLED, OPENMP_BUILD=OFF and ENABLE_NATIVE=OFF with no codecs, so legacy engine modes fail fast.
---

# 0058. Vendored Tesseract build disables the legacy engine, graphics, OpenMP and native tuning

## Context

Spec 0002 D-4. Tesseract calls exit() on model-load paths and in ScrollView; OpenMP would break the concurrency rule C5; wheels must be portable.

## Decision

Build with `DISABLED_LEGACY_ENGINE=ON`, `GRAPHICS_DISABLED=ON`, `OPENMP_BUILD=OFF`, `ENABLE_NATIVE=OFF`, no TIFF/archive/curl, and Leptonica without codecs. `EngineMode::TesseractOnly` and `TesseractLstmCombined` consequently fail fast with `InitFailed` on the vendored build.

## Alternatives considered

No alternative recorded.

## Consequences

Every exit() on a model-load path and ScrollView's exit(0) are gone; C5 holds; wheels stay portable through runtime SIMD dispatch; the attack surface shrinks to SetImage plus model parsing. Legacy and combined models cannot be used with the vendored build.
