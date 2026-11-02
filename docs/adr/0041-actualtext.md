---
id: 0041
title: Honour /ActualText with a simple first-glyph rule
date: 2026-10-01
status: Accepted
summary: The property list reaches the Device (H4) and /ActualText text is emitted at the span's first glyph; later glyphs advance the pen silently.
---

# 0041. Honour /ActualText with a simple first-glyph rule

## Context

Decision D-5 (compat point KP9). The oracle honours `/ActualText`, but its rule for distributing the text over the span's
glyphs is not reproducible black-box. hayro's `Device::begin_marked_content(tag, mcid)` carried no property dictionary.

## Decision

H4 passes the resolved property dictionary (`Option<&Dict<'_>>`) to `begin_marked_content`. The whole string
(PDFDocEncoding, or UTF-16 with either BOM, or UTF-8 with BOM) is emitted at the first glyph of the span with that glyph's
geometry; later glyphs emit nothing but advance the pen (the span record's advance grows); an empty string suppresses the
span; a nested span replaces its own glyphs. Spans nest at most 64 deep and a page decodes at most 4 x
`max_chars_per_page` bytes of `/ActualText`, after which spans are plain marked content.

## Alternatives considered

Recorded rationale: the simple rule matches all non-pathological probes. Mimicking the oracle's distribution rule was not
possible black-box.

## Consequences

Spans crossing line breaks and spans with gaps are expected divergences (KP17, `kp09_cross_line`, `kp09_gap`, `kp09_nested`). Unbalanced
`BDC` is closed at the end of each content stream (ADR 0032).
