---
id: 0018
title: Cap object nesting in hayro-syntax, hayro-interpret's Type 4 parser and hayro-postscript
date: 2026-10-01
status: Accepted
summary: Object and PostScript nesting is capped at 256 levels (change H2a); an over-deep read fails instead of overflowing the stack.
---

# 0018. Cap object nesting in hayro-syntax, hayro-interpret's Type 4 parser and hayro-postscript

## Context

Decision D-3. Without a cap R1 (no input may crash the process) is unachievable: a verified stack overflow occurs at
200,000-deep nesting in three places (`deep_obj`, `deep_content`, `deep_resources`), on 8 MiB stacks as well as
512 KiB ones. Upstream `Array::read/skip`, `Dict::read/skip` and `Object::read` recurse with no limit
(`hayro-postscript/src/object.rs` carries a TODO about `<<<<` overflow).

## Decision

H2a adds a depth counter to hayro-syntax's `Reader`, incremented in array and dictionary read/skip, with
`MAX_OBJECT_DEPTH = 256`; beyond it the read returns `None`, so the containing object is treated as missing. The
recursive Type 4 procedure parser in hayro-interpret (`function/type4.rs`) gets the same cap. hayro-postscript's
`object.rs::read` caps `<<`/`[` nesting at 256 and returns the existing `Error::LimitCheck`. Content operands use the
same `Object::read`, so `TJ` arrays are covered.

## Alternatives considered

No alternative is recorded in the spec.

## Consequences

An over-deep content operand ends the content stream (`UntypedIter::next` returns `None`), so the rest of that stream
is lost; this is pinned by tests. For PostScript the error class for runs of more than 256 `<<`, `>>` or `[` changes
from `SyntaxError` to `LimitCheck`; both end the CMap parse the same way. The signal route is ADR 0019.
