---
id: 0088
title: Data-terminated JBIG2 loops get forward-progress guards, because page_time_budget cannot interrupt a decode
date: 2026-10-19
status: Accepted
summary: The two JBIG2 symbol-dictionary loops that end only on what the arithmetic data decodes to (export flags, H17; height classes, H22) refuse once the data is used up and the loop stops advancing, with a threshold of two empty runs for the export flags and more than 64 empty height classes for the height-class loop.
---

# 0088. Data-terminated JBIG2 loops get forward-progress guards, because page_time_budget cannot interrupt a decode

## Context

ADR 0027 bounds time with a deadline that hayro checks every 4,096 operators, and records that one operator (a 256 MiB stream decode, a font parse) cannot be interrupted. An image codec call is such an operator. ADR 0079 bounds the memory a codec may allocate, but a loop that allocates nothing is not charged, so a decoder loop that does not terminate spins at full CPU past both `page_time_budget` and the memory budget.

Fuzzing found two such loops in the vendored `hayro-jbig2` symbol-dictionary decoder (6.5). Both end only on what the MQ-coded data decodes to. The arithmetic decoder pads its input with 0xFF and stops advancing at a marker, so once the data is used up it is a finite state machine that reads nothing, and some streams then cycle forever:

- the export-flag loop (6.5.10) runs until the decoded run lengths reach the dictionary's symbol count, and only a run of at least one symbol moves it (internal #94; a 99-byte stream);
- the height-class loop runs until the dictionary has its declared new symbols, and only a symbol moves it, so a cycle of height classes that hold no symbol (IADH, then IADW = OOB) never ends (internal #116; the two-byte stream `06 c4` with three new symbols).

## Decision

Each loop refuses, with its own `SymbolError`, once the arithmetic decoder is exhausted (`ArithmeticDecoder::is_exhausted`: past its input or at a marker) and the loop has stopped advancing for longer than a threshold. The thresholds differ because the two loops differ:

- **Export flags, H17 (patch 0027, `ExportStalled`):** two 0-length runs in a row decoded after the data is used up. Every such cycle decodes them, and no encoder writes them, since a 0-length run is needed only first. Refusing any 0-length run after the data would break valid streams: a short stream's runs all come from the padding, its leading 0 among them.
- **Height classes, H22 (patch 0032, `HeightClassStalled`, internal issue #116):** more than `MAX_STALLED_HEIGHT_CLASSES = 64` height classes in a row that decode no symbol after the data is used up. Two in a row, as for the export flags, is too tight: the two-byte dictionary `a0 b6` with one new symbol decodes three empty height classes from its padding before its symbol, and must keep decoding (`a_short_stream_whose_padding_decodes_empty_height_classes_still_decodes`). 64 is far above anything seen and still ends a spin at once. The count is a heuristic, not derived from the standard; no encoder writes an empty height class.

The Huffman path needs neither guard: its reader fails at the end of the data. The JPEG 2000 copy of the arithmetic decoder has no `is_exhausted` caller, since its loops over coding passes and bit planes are counted from the packet headers.

## Alternatives considered

- **Refuse at the first empty run or height class after the data** - breaks short streams that decode correctly today, as above; this is the reason the thresholds are not 1.
- **Reuse the export-flag threshold (two in a row) for height classes** - refuses `a0 b6`, as above.
- Other approaches (making `page_time_budget` interrupt a codec call, a fixed iteration cap, a watchdog) - rationale for rejecting them was not recorded in the project sources at the time of the fixes.

## Consequences

The remaining guarantee is narrower than "decoders terminate". Only these two JBIG2 loops are guarded. ADR 0079's addendum of the same date adds H23, a work allowance that bounds the time JBIG2 spends placing symbol instances; every other decoder loop (CCITT, DCT, RGBA conversion, the halftone region) is bounded only by its own termination or by the allocations the memory budget charges. A further data-terminated loop found by fuzzing gets the same treatment through the bug fix pass. The 64 threshold in H22 is a judgement call: a pathological but legitimate stream with more than 64 consecutive empty classes after its data would be refused as `ImageDecode`, and none is known.
