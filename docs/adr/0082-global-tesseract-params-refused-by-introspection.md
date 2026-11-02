---
id: 0082
title: Process-global Tesseract parameters are refused by asking Tesseract, not by a list
date: 2026-10-05
status: Accepted
summary: Instance::with_variables refuses any name found in Tesseract's GlobalParams() (via a shim query) plus the two file-writing instance parameters.
---

# 0082. Process-global Tesseract parameters are refused by asking Tesseract, not by a list

## Context

Spec 0003 F18 (B4). A variable passed to `Instance::with_variables` that is process-global (for example `debug_file` set to `""`) changes every instance's behaviour, not just that instance's. The function is reachable today only from the sys crate's own tests (`OcrConfig` and Python expose no variables), but the set of global parameters belongs to Tesseract and changes with versions.

## Decision

`Instance::with_variables` returns `ErrorKind::InvalidArgument` naming the variable for any name that resolves in Tesseract's `GlobalParams()`, answered by a new shim entry `pxt_is_global_param(name)` that searches the four typed global vectors. Two per-instance parameters that write files, `tessedit_write_params_to_file` and `tessedit_write_images`, are refused from a Rust-side constant. `("debug_file", "")` is the pinned negative case; `("tessedit_do_invert", "0")` still works.

## Alternatives considered

- **A denylist of known global names** - drifts from the vendored Tesseract on every upgrade and silently admits new globals.
- **Do nothing because nothing exposes variables** - leaves a trap for the first caller that does.

## Consequences

The refused set tracks the vendored Tesseract automatically. The two file-writing names are the only hand-kept list and must be revisited at Tesseract upgrades. The shim symbol is referenced by a sys test as `shim_symbols.rs` requires.

## Addendum (2026-10-14)

Internal issue #65 (A1–A3) found per-instance parameters outside the two-name list that still write files (`tessedit_dump_pageseg_images`, `tessedit_train_line_recognizer`), read arbitrary paths (`user_words_file`, `user_patterns_file`), bypass the language-list check (`tessedit_load_sublangs`), or leave Init without a usable engine (`tessedit_init_config_only`, `tessedit_ocr_engine_mode`). The decision is to extend the hand-kept denylist, not to switch to an allowlist, and to keep the `GlobalParams()` introspection unchanged. The list (renamed `REFUSED_INSTANCE_PARAMS` in `crates/tesseract-sys/src/lib.rs`) now has a documented scope: `with_variables` refuses every process-global param and every per-instance param that (a) opens a file named by or derived from the variable, (b) selects a training, dump or box mode, or (c) changes which models Init loads (e.g. `tessedit_ocr_engine_mode`, `tessedit_init_config_only`, `tessedit_load_sublangs`); the list is re-audited at every Tesseract upgrade (last audited against 5.5.3: 19 names). Params outside these classes are residuals, not defects; dictionary toggles (`load_*_dawg`, `lstm_use_matrix`) are judged not to change which models load.

## Addendum (2026-10-21)

Internal issue #98. `save_doc_words` appends to `<imagefile>.doc` and Tesseract aborts the process when that file cannot be opened; it is a dump mode (class b) and joins `REFUSED_INSTANCE_PARAMS` (now 20 names, still audited against 5.5.3). Dictionary parameters are confirmed not to be models: the 2026-10-14 judgement stands. Three parameters stay allowed as documented residuals, recorded in the `REFUSED_INSTANCE_PARAMS` doc comment: `interactive_display_mode` (opens a ScrollView connection in builds without `GRAPHICS_DISABLED`; the vendored build skips recognition and returns an empty page), the legacy adaptation parameters such as `classify_enable_learning` (they change results, not files or models), and the legacy-build `oem` argument that re-overrides `tessedit_ocr_engine_mode` (moot while that name is refused). A new refusal class for connections or recognition bypass would be needed to cover the first; none exists today.
