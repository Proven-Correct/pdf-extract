---
id: 0072
title: CI and OCR expectations use a pinned tessdata_fast model
date: 2026-10-03
status: Accepted
summary: CI and committed OCR expectations use tessdata_fast eng.traineddata pinned by blob sha and SHA-256, recorded in every expectation.
---

# 0072. CI and OCR expectations use a pinned tessdata_fast model

## Context

Spec 0002 D-17. OCR numbers are meaningless without the model id, and tessdata_fast is what Debian ships.

## Decision

Pinned by git blob sha `bbef4675053b5b468cdb477053e28b1c698ba08e`, SHA-256 `7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2`; every expectation carries `tessdata_sha256`, which equals that SHA-256.

## Alternatives considered

No alternative recorded.

## Consequences

quality.rs fails when an expectation's hash differs from the model in use. Unit 13 notes that current Debian tesseract-ocr-eng is tessdata_fast (stated unverified at the time).
