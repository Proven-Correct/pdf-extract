---
id: 0001
title: Build the core in pure Rust on the hayro crates
date: 2026-09-25
status: Accepted
summary: The core is pure Rust on hayro-syntax, hayro-interpret and hayro, all Apache-2.0 OR MIT.
---

# 0001. Build the core in pure Rust on the hayro crates

## Context

The library replaces the PyMuPDF functionality its first consumer uses (open, page count, text layer, later OCR). The
reason for replacing PyMuPDF is its AGPL license, so requirement L1 bans AGPL, GPL and LGPL code from the
shipped dependency graph, including transitively. Requirement C1 needs one `Document` usable from many threads.
Source: design spec D1 and section 3.1.

## Decision

The core is pure Rust and is built on the hayro crates (`hayro-syntax`, `hayro-interpret`, `hayro`, plus the
`-jbig2`, `-jpeg2000`, `-ccitt` and `-cmap` leaf crates). They provide the parser, xref repair, RC4/AES
decryption, font handling, a rasteriser and JBIG2/JPEG 2000/CCITT decoders, all permissively licensed. The
selection was made on conditions: the required changes are carried as a fork (ADR 0002).

## Alternatives considered

Design spec section 3.1 records each option and its verdict:

- **MuPDF / PyMuPDF (AGPL-3.0)** excluded: it is what is being replaced.
- **Poppler, Xpdf (GPL)** excluded by L1.
- **jbig2dec (AGPL-3.0)** excluded by L1; a permissive JBIG2 decoder is needed because scanned documents often use JBIG2.
- **PDFium via `pdfium-render`** rejected: PDFium is not thread-safe even across documents, and `pdfium-render`
  serialises every call behind one global mutex, which fails C1; it also needs a prebuilt C++ binary per platform.
- **`lopdf`** rejected as the foundation: loads the whole document eagerly, limited repair, no content
  interpreter, font handling or rasteriser.
- **Writing our own parser** rejected: robust handling of real-world broken PDFs takes years.

## Consequences

Easier: no copyleft code to avoid, a CPU rasteriser and permissive image decoders for the OCR run, an interpreter
crate that forbids `unsafe`. Harder: the 0.x crates break APIs between releases, so versions are pinned exactly;
`hayro_interpret::Context` is not `Send` (its font cache is an `Rc<RefCell<..>>`), so contexts are created per
operation; hayro as released does not meet the robustness requirements (ADR 0002). A dependency probe
found 85 third-party crates, all MIT, Apache-2.0, BSD-3, Zlib, Unicode-3.0 or Unlicense (or OR-combinations).
